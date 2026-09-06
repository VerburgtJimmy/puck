//! Packagist / composer-repo metadata loader with VCR-first modes for default Packagist.
//!
//! - **Replay**: filesystem only for default Packagist (`packagist/p2/`); miss is an error.
//!   Custom `type: composer` repos are **not** VCR'd (skipped in Replay — see docs).
//! - **Live**: Packagist filesystem first when a VCR root is set, else HTTP; miss → HTTP.
//!   Custom composer repos always use HTTP (`packages.json` → `metadata-url`).
//! - **Record**: like Live for Packagist, writing under the VCR root; custom repos live-only.
//!
//! Lookup order: listed composer repos (first wins), then default Packagist if enabled.

use crate::composer_repo::{
    canonicalize_metadata_url, default_packagist_url, metadata_url_for_package,
    metadata_url_template, packages_json_url, parse_repositories, PackagesJsonCache,
    RepositoryConfig,
};
use crate::packagist::{load_p2_metadata, p2_path};
use crate::replay::{ReplayError, ReplayMode};
use crate::{Error, Result};
use puck_dist::{AuthHeader, AuthStore};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://repo.packagist.org";

/// Loads package metadata bytes for `vendor/package` names from composer repos + Packagist.
pub struct P2Loader {
    mode: ReplayMode,
    /// Registry root containing `packagist/p2/` (optional in pure Live).
    vcr_root: Option<PathBuf>,
    /// Default Packagist base URL (HTTP fallback when packagist enabled).
    base_url: String,
    auth: AuthStore,
    cache: RefCell<HashMap<String, Vec<u8>>>,
    /// Optional override for HTTP (unit tests); receives full URL.
    http_get: Option<Box<dyn Fn(&str) -> Result<Vec<u8>>>>,
    /// User-listed composer repository base URLs (order = precedence).
    composer_urls: Vec<String>,
    packagist_enabled: bool,
    packages_json_cache: RefCell<PackagesJsonCache>,
}

impl std::fmt::Debug for P2Loader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("P2Loader")
            .field("mode", &self.mode)
            .field("vcr_root", &self.vcr_root)
            .field("base_url", &self.base_url)
            .field("auth", &self.auth)
            .field("cache_len", &self.cache.borrow().len())
            .field("http_override", &self.http_get.is_some())
            .field("composer_urls", &self.composer_urls)
            .field("packagist_enabled", &self.packagist_enabled)
            .finish()
    }
}

impl P2Loader {
    /// Build from optional registry root + project dir (for `auth.json`).
    ///
    /// Mode selection:
    /// - No registry → [`ReplayMode::Live`]
    /// - Registry set → [`ReplayMode::Replay`], unless `PUCK_REGISTRY_MODE` is
    ///   `live` / `record` / `replay`
    pub fn from_env_and_registry(
        registry_root: Option<PathBuf>,
        project_dir: Option<&Path>,
    ) -> Result<Self> {
        let mode = match &registry_root {
            None => ReplayMode::Live,
            Some(_) => match std::env::var("PUCK_REGISTRY_MODE") {
                Ok(raw) => parse_registry_mode(&raw)?,
                Err(_) => ReplayMode::Replay,
            },
        };
        if matches!(mode, ReplayMode::Record) && registry_root.is_none() {
            return Err(Error::Message(
                "PUCK_REGISTRY_MODE=record requires --registry / $PUCK_REGISTRY".into(),
            ));
        }
        let auth = AuthStore::load_merged(
            puck_dist::composer_home_dir().as_deref(),
            project_dir,
            std::env::var("COMPOSER_AUTH").ok().as_deref(),
        )
        .map_err(|e| Error::Message(e.to_string()))?;
        Ok(Self {
            mode,
            vcr_root: registry_root,
            base_url: DEFAULT_BASE_URL.to_owned(),
            auth,
            cache: RefCell::new(HashMap::new()),
            http_get: None,
            composer_urls: Vec::new(),
            packagist_enabled: true,
            packages_json_cache: RefCell::new(PackagesJsonCache::new()),
        })
    }

    /// Construct a loader with explicit mode (tests / tooling).
    pub fn new(
        mode: ReplayMode,
        vcr_root: Option<PathBuf>,
        project_dir: Option<&Path>,
    ) -> Result<Self> {
        if matches!(mode, ReplayMode::Record) && vcr_root.is_none() {
            return Err(Error::Message(
                "Record mode requires a registry root to write p2 snapshots".into(),
            ));
        }
        let auth = AuthStore::load_merged(
            puck_dist::composer_home_dir().as_deref(),
            project_dir,
            std::env::var("COMPOSER_AUTH").ok().as_deref(),
        )
        .map_err(|e| Error::Message(e.to_string()))?;
        Ok(Self {
            mode,
            vcr_root,
            base_url: DEFAULT_BASE_URL.to_owned(),
            auth,
            cache: RefCell::new(HashMap::new()),
            http_get: None,
            composer_urls: Vec::new(),
            packagist_enabled: true,
            packages_json_cache: RefCell::new(PackagesJsonCache::new()),
        })
    }

    /// Override Packagist base URL (e.g. mock server in tests).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Inject HTTP GET for tests (full URL → body).
    pub fn with_http_get(mut self, f: impl Fn(&str) -> Result<Vec<u8>> + 'static) -> Self {
        self.http_get = Some(Box::new(f));
        self
    }

    /// Apply parsed `repositories` (composer URLs + packagist enable flag).
    pub fn with_repository_config(mut self, config: RepositoryConfig) -> Self {
        self.composer_urls = config.composer_urls;
        self.packagist_enabled = config.packagist_enabled;
        self
    }

    /// Parse `repositories` from a root composer.json value and apply.
    pub fn with_composer_json(self, root: &Value) -> Self {
        self.with_repository_config(parse_repositories(root))
    }

    pub fn mode(&self) -> ReplayMode {
        self.mode
    }

    pub fn vcr_root(&self) -> Option<&Path> {
        self.vcr_root.as_deref()
    }

    pub fn packagist_enabled(&self) -> bool {
        self.packagist_enabled
    }

    pub fn composer_urls(&self) -> &[String] {
        &self.composer_urls
    }

    /// Fetch metadata bytes for `vendor/package` (lowercased cache key).
    pub fn get(&self, package: &str) -> Result<Vec<u8>> {
        let package = package.to_ascii_lowercase();
        if let Some(hit) = self.cache.borrow().get(&package).cloned() {
            return Ok(hit);
        }
        let bytes = self.load_uncached(&package)?;
        self.cache.borrow_mut().insert(package, bytes.clone());
        Ok(bytes)
    }

    fn load_uncached(&self, package: &str) -> Result<Vec<u8>> {
        // 1) Custom composer repos (order = precedence). Not covered by Packagist VCR.
        for repo_base in &self.composer_urls {
            match self.load_from_composer_repo(repo_base, package) {
                Ok(bytes) => return Ok(bytes),
                Err(Error::Replay(ReplayError::Miss(_))) => continue,
                Err(err) => return Err(err),
            }
        }

        // 2) Default Packagist
        if self.packagist_enabled {
            return self.load_from_packagist(package);
        }

        Err(Error::Replay(ReplayError::Miss(package.to_owned())))
    }

    fn load_from_composer_repo(&self, repo_base: &str, package: &str) -> Result<Vec<u8>> {
        if self.mode == ReplayMode::Replay {
            // VCR fixtures only cover default Packagist `packagist/p2/`.
            return Err(Error::Replay(ReplayError::Miss(package.to_owned())));
        }

        let template = self.ensure_metadata_template(repo_base)?;
        let Some(template) = template else {
            // No Composer 2 metadata-url (V1-only repo) — treat as miss for this source.
            return Err(Error::Replay(ReplayError::Miss(package.to_owned())));
        };
        let url = metadata_url_for_package(
            &canonicalize_metadata_url(repo_base, &template),
            package,
        );
        self.http_get_bytes(&url, package)
    }

    fn ensure_metadata_template(&self, repo_base: &str) -> Result<Option<String>> {
        if let Some(cached) = self.packages_json_cache.borrow().get(repo_base).cloned() {
            return Ok(cached);
        }
        let index_url = packages_json_url(repo_base);
        let bytes = self.http_get_bytes(&index_url, repo_base)?;
        let doc: Value = serde_json::from_slice(&bytes).map_err(|e| {
            Error::Message(format!("invalid packages.json from {repo_base}: {e}"))
        })?;
        let template = metadata_url_template(&doc);
        self.packages_json_cache
            .borrow_mut()
            .insert(repo_base.to_owned(), template.clone());
        Ok(template)
    }

    fn load_from_packagist(&self, package: &str) -> Result<Vec<u8>> {
        if let Some(root) = &self.vcr_root {
            match load_p2_metadata(root, package) {
                Ok(bytes) => return Ok(bytes),
                Err(ReplayError::Miss(_)) => {
                    if self.mode == ReplayMode::Replay {
                        return Err(Error::Replay(ReplayError::Miss(package.to_owned())));
                    }
                }
                Err(err) => return Err(Error::Replay(err)),
            }
        } else if self.mode == ReplayMode::Replay {
            return Err(Error::Message(
                "Replay mode requires a registry root with packagist/p2/".into(),
            ));
        }

        let bytes = self.fetch_packagist_http(package)?;
        if self.mode == ReplayMode::Record {
            if let Some(root) = &self.vcr_root {
                self.write_recording(root, package, &bytes)?;
            }
        }
        Ok(bytes)
    }

    fn fetch_packagist_http(&self, package: &str) -> Result<Vec<u8>> {
        let base = if self.base_url.is_empty() {
            default_packagist_url()
        } else {
            self.base_url.trim_end_matches('/')
        };
        let url = format!("{base}/p2/{package}.json");
        self.http_get_bytes(&url, package)
    }

    fn http_get_bytes(&self, url: &str, miss_key: &str) -> Result<Vec<u8>> {
        if let Some(f) = &self.http_get {
            return match f(url) {
                Ok(bytes) => Ok(bytes),
                Err(Error::Replay(ReplayError::Miss(_))) => {
                    Err(Error::Replay(ReplayError::Miss(miss_key.to_owned())))
                }
                Err(err) => Err(err),
            };
        }
        http_get(&url, &self.auth, miss_key)
    }

    fn write_recording(&self, root: &Path, package: &str, bytes: &[u8]) -> Result<()> {
        let path = p2_path(root, package);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| {
                Error::Replay(ReplayError::Io {
                    path: parent.display().to_string(),
                    source,
                })
            })?;
        }
        std::fs::write(&path, bytes).map_err(|source| {
            Error::Replay(ReplayError::Io {
                path: path.display().to_string(),
                source,
            })
        })?;
        Ok(())
    }
}

fn parse_registry_mode(raw: &str) -> Result<ReplayMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "live" => Ok(ReplayMode::Live),
        "record" => Ok(ReplayMode::Record),
        "replay" => Ok(ReplayMode::Replay),
        other => Err(Error::Message(format!(
            "invalid PUCK_REGISTRY_MODE={other:?}; expected live|record|replay"
        ))),
    }
}

fn http_get(url: &str, auth: &AuthStore, miss_key: &str) -> Result<Vec<u8>> {
    // `reqwest::blocking` must not run on a Tokio worker (CLI uses `block_on`).
    let url = url.to_owned();
    let auth = auth.clone();
    let miss_key = miss_key.to_owned();
    std::thread::spawn(move || http_get_sync(&url, &auth, &miss_key))
        .join()
        .unwrap_or_else(|_| Err(Error::Message("metadata HTTP worker panicked".into())))
}

fn http_get_sync(url: &str, auth: &AuthStore, miss_key: &str) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("puck/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Message(format!("http client: {e}")))?;

    let mut request = client.get(url);
    if let Some(header) = auth.authorization_for_url(url) {
        request = match header {
            AuthHeader::Basic { username, password } => {
                request.basic_auth(username, Some(password))
            }
            AuthHeader::Bearer(token) => request.bearer_auth(token),
            AuthHeader::GithubToken(token) => {
                request.header(reqwest::header::AUTHORIZATION, format!("token {token}"))
            }
        };
    }

    let response = request
        .send()
        .map_err(|e| Error::Message(format!("GET {url}: {e}")))?;
    let status = response.status();
    if status.as_u16() == 404 {
        return Err(Error::Replay(ReplayError::Miss(miss_key.to_owned())));
    }
    if !status.is_success() {
        return Err(Error::Message(format!("GET {url}: HTTP {status}")));
    }
    response
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|e| Error::Message(format!("GET {url}: {e}")))
}

/// Map loader errors for pool building: [`ReplayError::Miss`] → `Ok(None)`.
pub fn load_p2_optional(loader: &P2Loader, package: &str) -> Result<Option<Vec<u8>>> {
    match loader.get(package) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(Error::Replay(ReplayError::Miss(_))) => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn replay_miss_errors() {
        let dir = tempfile::tempdir().unwrap();
        let loader = P2Loader::new(ReplayMode::Replay, Some(dir.path().to_path_buf()), None)
            .unwrap();
        let err = loader.get("no/such").unwrap_err();
        assert!(
            matches!(err, Error::Replay(ReplayError::Miss(ref p)) if p == "no/such"),
            "{err:?}"
        );
    }

    #[test]
    fn record_writes_vcr_file() {
        let dir = tempfile::tempdir().unwrap();
        let body = br#"{"packages":{"acme/demo":[]}}"#.to_vec();
        let body_clone = body.clone();
        let loader = P2Loader::new(ReplayMode::Record, Some(dir.path().to_path_buf()), None)
            .unwrap()
            .with_http_get(move |_url| Ok(body_clone.clone()));
        let got = loader.get("acme/demo").unwrap();
        assert_eq!(got, body);
        let path = p2_path(dir.path(), "acme/demo");
        assert_eq!(std::fs::read(&path).unwrap(), body);
    }

    #[test]
    fn live_uses_http_when_vcr_missing() {
        let dir = tempfile::tempdir().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();
        let loader = P2Loader::new(ReplayMode::Live, Some(dir.path().to_path_buf()), None)
            .unwrap()
            .with_http_get(move |url| {
                hits2.fetch_add(1, Ordering::SeqCst);
                assert!(url.ends_with("/p2/acme/live.json"));
                Ok(br#"{"packages":{}}"#.to_vec())
            });
        let _ = loader.get("acme/live").unwrap();
        let _ = loader.get("acme/live").unwrap(); // cache
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn live_prefers_filesystem_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let p2 = dir.path().join("packagist/p2");
        std::fs::create_dir_all(&p2).unwrap();
        let path = p2.join("acme$fs.json");
        std::fs::write(&path, br#"{"packages":{"acme/fs":[]}}"#).unwrap();
        let loader = P2Loader::new(ReplayMode::Live, Some(dir.path().to_path_buf()), None)
            .unwrap()
            .with_http_get(|_| Err(Error::Message("should not hit network".into())));
        let bytes = loader.get("acme/fs").unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("acme/fs"));
    }

    #[test]
    fn composer_repo_packages_json_then_metadata() {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();
        let loader = P2Loader::new(ReplayMode::Live, None, None)
            .unwrap()
            .with_repository_config(RepositoryConfig {
                composer_urls: vec!["https://satis.example.com".into()],
                packagist_enabled: false,
            })
            .with_http_get(move |url| {
                hits2.fetch_add(1, Ordering::SeqCst);
                if url == "https://satis.example.com/packages.json" {
                    return Ok(
                        br#"{"packages":{},"metadata-url":"/p2/%package%.json"}"#.to_vec(),
                    );
                }
                if url == "https://satis.example.com/p2/acme/widget.json" {
                    return Ok(br#"{"packages":{"acme/widget":[{"name":"acme/widget","version":"1.0.0"}]}}"#.to_vec());
                }
                Err(Error::Replay(ReplayError::Miss(url.to_owned())))
            });
        let bytes = loader.get("Acme/Widget").unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("acme/widget"));
        // packages.json + metadata; second get is memory-cached
        let _ = loader.get("acme/widget").unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn composer_repo_miss_falls_through_to_packagist() {
        let loader = P2Loader::new(ReplayMode::Live, None, None)
            .unwrap()
            .with_repository_config(RepositoryConfig {
                composer_urls: vec!["https://private.example.com".into()],
                packagist_enabled: true,
            })
            .with_base_url("https://packagist.test")
            .with_http_get(|url| {
                if url.contains("private.example.com/packages.json") {
                    return Ok(br#"{"metadata-url":"https://private.example.com/p2/%package%.json"}"#.to_vec());
                }
                if url.contains("private.example.com/p2/") {
                    return Err(Error::Replay(ReplayError::Miss("missing".into())));
                }
                if url == "https://packagist.test/p2/symfony/http-foundation.json" {
                    return Ok(br#"{"packages":{"symfony/http-foundation":[]}}"#.to_vec());
                }
                Err(Error::Message(format!("unexpected url {url}")))
            });
        let bytes = loader.get("symfony/http-foundation").unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("symfony/http-foundation"));
    }

    #[test]
    fn packagist_disabled_misses_without_composer_hit() {
        let loader = P2Loader::new(ReplayMode::Live, None, None)
            .unwrap()
            .with_composer_json(&json!({
                "repositories": [ { "packagist.org": false } ]
            }))
            .with_http_get(|_| Err(Error::Message("no http".into())));
        let err = loader.get("acme/alone").unwrap_err();
        assert!(matches!(err, Error::Replay(ReplayError::Miss(_))));
    }

    #[test]
    fn composer_repo_preferred_over_packagist() {
        let loader = P2Loader::new(ReplayMode::Live, None, None)
            .unwrap()
            .with_repository_config(RepositoryConfig {
                composer_urls: vec!["https://first.example.com".into()],
                packagist_enabled: true,
            })
            .with_http_get(|url| {
                if url.ends_with("/packages.json") {
                    return Ok(br#"{"metadata-url":"https://first.example.com/p2/%package%.json"}"#.to_vec());
                }
                if url.contains("first.example.com/p2/acme/priv.json") {
                    return Ok(br#"{"packages":{"acme/priv":[{"version":"1.0.0"}]}}"#.to_vec());
                }
                Err(Error::Message(format!("should not reach packagist: {url}")))
            });
        let bytes = loader.get("acme/priv").unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("acme/priv"));
    }

    #[test]
    #[ignore = "network: live Packagist smoke"]
    fn live_packagist_smoke() {
        let loader = P2Loader::new(ReplayMode::Live, None, None).unwrap();
        let bytes = loader.get("monolog/monolog").expect("fetch monolog");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("monolog/monolog"));
    }
}
