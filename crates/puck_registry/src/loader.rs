//! Packagist Composer 2 (`/p2/…`) metadata loader with VCR-first modes.
//!
//! - **Replay**: filesystem only; miss is an error (CI / `--registry`).
//! - **Live**: filesystem first when a VCR root is set, else HTTP; miss → HTTP.
//! - **Record**: like Live, but successful HTTP responses are written under the VCR root.

use crate::packagist::{load_p2_metadata, p2_path};
use crate::replay::{ReplayError, ReplayMode};
use crate::{Error, Result};
use puck_dist::{AuthHeader, AuthStore};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://repo.packagist.org";

/// Loads Packagist p2 metadata bytes for `vendor/package` names.
pub struct P2Loader {
    mode: ReplayMode,
    /// Registry root containing `packagist/p2/` (optional in pure Live).
    vcr_root: Option<PathBuf>,
    base_url: String,
    auth: AuthStore,
    cache: RefCell<HashMap<String, Vec<u8>>>,
    /// Optional override for HTTP (unit tests); receives full URL.
    http_get: Option<Box<dyn Fn(&str) -> Result<Vec<u8>>>>,
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

    pub fn mode(&self) -> ReplayMode {
        self.mode
    }

    pub fn vcr_root(&self) -> Option<&Path> {
        self.vcr_root.as_deref()
    }

    /// Fetch p2 metadata bytes for `vendor/package` (lowercased cache key).
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

        // Live or Record: HTTP
        let bytes = self.fetch_http(package)?;
        if self.mode == ReplayMode::Record {
            if let Some(root) = &self.vcr_root {
                self.write_recording(root, package, &bytes)?;
            }
        }
        Ok(bytes)
    }

    fn fetch_http(&self, package: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/p2/{}.json",
            self.base_url.trim_end_matches('/'),
            package
        );
        if let Some(f) = &self.http_get {
            return f(&url);
        }
        http_get_p2(&url, &self.auth)
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

fn http_get_p2(url: &str, auth: &AuthStore) -> Result<Vec<u8>> {
    // `reqwest::blocking` must not run on a Tokio worker (CLI uses `block_on`).
    let url = url.to_owned();
    let auth = auth.clone();
    std::thread::spawn(move || http_get_p2_sync(&url, &auth))
        .join()
        .unwrap_or_else(|_| Err(Error::Message("p2 HTTP worker panicked".into())))
}

fn http_get_p2_sync(url: &str, auth: &AuthStore) -> Result<Vec<u8>> {
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
        let pkg = url
            .rsplit_once("/p2/")
            .map(|(_, rest)| rest.trim_end_matches(".json").to_owned())
            .unwrap_or_else(|| url.to_owned());
        return Err(Error::Replay(ReplayError::Miss(pkg)));
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
    #[ignore = "network: live Packagist smoke"]
    fn live_packagist_smoke() {
        let loader = P2Loader::new(ReplayMode::Live, None, None).unwrap();
        let bytes = loader.get("monolog/monolog").expect("fetch monolog");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("monolog/monolog"));
    }
}
