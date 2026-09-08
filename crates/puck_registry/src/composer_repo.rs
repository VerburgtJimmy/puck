//! Composer `type: composer` repositories (`packages.json` + Composer 2 `metadata-url`).
//!
//! Subset of `Composer\Repository\ComposerRepository` / `Composer\Config` repository merge:
//! - Parse root `repositories` for `type: composer` URLs and `{"packagist.org": false}`
//! - Fetch `{url}/packages.json`, read `metadata-url`, substitute `%package%`
//! - Skip V1 `provider-includes` / `providers-url` (documented gap)

use crate::{Error, Result};
use serde_json::Value;
use std::collections::HashMap;

const DEFAULT_PACKAGIST_URL: &str = "https://repo.packagist.org";

/// Parsed repository configuration from root `composer.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryConfig {
    /// User-listed `type: composer` base URLs, in Composer precedence order (first wins).
    pub composer_urls: Vec<String>,
    /// Whether the implicit Packagist.org default remains enabled.
    pub packagist_enabled: bool,
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self {
            composer_urls: Vec::new(),
            packagist_enabled: true,
        }
    }
}

/// Parse `repositories` from a root composer.json value.
///
/// Honour:
/// - `{ "type": "composer", "url": "…" }`
/// - `{ "packagist.org": false }` and BC `{ "packagist": false }`
/// - object-keyed `"packagist.org": false` / `"packagist": false`
///
/// Redefining a packagist.org URL as `type: composer` disables the default named repo
/// (Composer `Config` behaviour) and keeps the listed URL in `composer_urls`.
pub fn parse_repositories(root: &Value) -> RepositoryConfig {
    let Some(repos) = root.get("repositories") else {
        return RepositoryConfig::default();
    };

    let mut composer_urls = Vec::new();
    let mut packagist_enabled = true;

    match repos {
        Value::Array(arr) => {
            for entry in arr {
                apply_repo_entry(entry, None, &mut composer_urls, &mut packagist_enabled);
            }
        }
        Value::Object(map) => {
            for (name, entry) in map {
                apply_repo_entry(
                    entry,
                    Some(name.as_str()),
                    &mut composer_urls,
                    &mut packagist_enabled,
                );
            }
        }
        _ => {}
    }

    RepositoryConfig {
        composer_urls,
        packagist_enabled,
    }
}

fn apply_repo_entry(
    entry: &Value,
    keyed_name: Option<&str>,
    composer_urls: &mut Vec<String>,
    packagist_enabled: &mut bool,
) {
    // Object-keyed disable: "repositories": { "packagist.org": false }
    if let Some(name) = keyed_name
        && entry.as_bool() == Some(false)
        && is_packagist_disable_name(name)
    {
        *packagist_enabled = false;
        return;
    }

    let Some(obj) = entry.as_object() else {
        return;
    };

    // Anonymous disable: { "packagist.org": false } or { "packagist": false }
    if obj.len() == 1
        && let Some((key, Value::Bool(false))) = obj.iter().next()
        && is_packagist_disable_name(key)
    {
        *packagist_enabled = false;
        return;
    }

    let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if !typ.eq_ignore_ascii_case("composer") {
        return;
    }
    let Some(url) = obj.get("url").and_then(|v| v.as_str()) else {
        return;
    };
    let url = normalize_repo_base_url(url);
    if is_packagist_org_url(&url) {
        // Composer auto-deactivates the default packagist.org named repo.
        *packagist_enabled = false;
    }
    if !composer_urls.iter().any(|u| u == &url) {
        composer_urls.push(url);
    }
}

fn is_packagist_disable_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "packagist.org" || n == "packagist"
}

/// True for packagist.org / *.packagist.org HTTP(S) URLs (Composer Config regex subset).
pub fn is_packagist_org_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    host == "packagist.org" || host == "repo.packagist.org" || host.ends_with(".packagist.org")
}

/// Strip trailing slash; leave path intact (Satis / Private Packagist org paths).
pub fn normalize_repo_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_owned()
}

/// URL for the repository root index (`packages.json`).
pub fn packages_json_url(repo_base: &str) -> String {
    let base = normalize_repo_base_url(repo_base);
    if base.to_ascii_lowercase().ends_with(".json") {
        base
    } else {
        format!("{base}/packages.json")
    }
}

/// Read Composer 2 `metadata-url` from a packages.json document.
///
/// Returns `None` when absent (V1 provider-includes path - not implemented).
pub fn metadata_url_template(packages_json: &Value) -> Option<String> {
    packages_json
        .get("metadata-url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned())
}

/// Resolve `metadata-url` (absolute or relative) against the composer repo base URL.
pub fn canonicalize_metadata_url(repo_base: &str, metadata_url: &str) -> String {
    let meta = metadata_url.trim();
    if meta.starts_with("http://") || meta.starts_with("https://") {
        return meta.to_owned();
    }
    let base = normalize_repo_base_url(repo_base);
    if meta.starts_with('/') {
        match origin_of(&base) {
            Some(origin) => format!("{origin}{meta}"),
            None => format!("{base}{meta}"),
        }
    } else {
        format!("{base}/{meta}")
    }
}

fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        ("https", r)
    } else {
        let r = url.strip_prefix("http://")?;
        ("http", r)
    };
    let host_port = rest.split('/').next().unwrap_or("");
    if host_port.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host_port}"))
}

/// Substitute `%package%` in a metadata-url template (package name lowercased).
pub fn metadata_url_for_package(template: &str, package: &str) -> String {
    template.replace("%package%", &package.to_ascii_lowercase())
}

/// Build the final per-package metadata URL from packages.json + package name.
pub fn resolve_package_metadata_url(
    repo_base: &str,
    packages_json: &Value,
    package: &str,
) -> Result<Option<String>> {
    let Some(template) = metadata_url_template(packages_json) else {
        return Ok(None);
    };
    if !template.contains("%package%") {
        return Err(Error::Message(format!(
            "composer repo {repo_base}: metadata-url missing %package% placeholder"
        )));
    }
    let absolute = canonicalize_metadata_url(repo_base, &template);
    Ok(Some(metadata_url_for_package(&absolute, package)))
}

/// In-memory cache of packages.json → metadata-url template per repo base URL.
#[derive(Debug, Default)]
pub struct PackagesJsonCache {
    /// repo base → Ok(Some(template)) | Ok(None) = no metadata-url | Err message cached as None try
    templates: HashMap<String, Option<String>>,
}

impl PackagesJsonCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, repo_base: &str) -> Option<&Option<String>> {
        self.templates.get(repo_base)
    }

    pub fn insert(&mut self, repo_base: String, template: Option<String>) {
        self.templates.insert(repo_base, template);
    }
}

/// Default Packagist base used when `packagist_enabled` (matches Composer default).
pub fn default_packagist_url() -> &'static str {
    DEFAULT_PACKAGIST_URL
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_composer_and_disable_packagist() {
        let root = json!({
            "repositories": [
                { "type": "composer", "url": "https://repo.packagist.com/acme/" },
                { "packagist.org": false }
            ]
        });
        let cfg = parse_repositories(&root);
        assert_eq!(
            cfg.composer_urls,
            vec!["https://repo.packagist.com/acme".to_owned()]
        );
        assert!(!cfg.packagist_enabled);
    }

    #[test]
    fn parse_packagist_bc_key() {
        let root = json!({
            "repositories": [
                { "packagist": false }
            ]
        });
        let cfg = parse_repositories(&root);
        assert!(cfg.composer_urls.is_empty());
        assert!(!cfg.packagist_enabled);
    }

    #[test]
    fn parse_object_keyed_disable() {
        let root = json!({
            "repositories": {
                "packagist.org": false,
                "private": { "type": "composer", "url": "https://satis.example.com" }
            }
        });
        let cfg = parse_repositories(&root);
        assert!(!cfg.packagist_enabled);
        assert_eq!(
            cfg.composer_urls,
            vec!["https://satis.example.com".to_owned()]
        );
    }

    #[test]
    fn redefining_packagist_url_disables_default() {
        let root = json!({
            "repositories": [
                { "type": "composer", "url": "https://repo.packagist.org" }
            ]
        });
        let cfg = parse_repositories(&root);
        assert!(!cfg.packagist_enabled);
        assert_eq!(
            cfg.composer_urls,
            vec!["https://repo.packagist.org".to_owned()]
        );
    }

    #[test]
    fn default_when_no_repositories() {
        let cfg = parse_repositories(&json!({}));
        assert!(cfg.packagist_enabled);
        assert!(cfg.composer_urls.is_empty());
    }

    #[test]
    fn packages_json_url_appends() {
        assert_eq!(
            packages_json_url("https://repo.packagist.com/org/"),
            "https://repo.packagist.com/org/packages.json"
        );
        assert_eq!(
            packages_json_url("https://example.com/packages.json"),
            "https://example.com/packages.json"
        );
    }

    #[test]
    fn metadata_url_parse_and_substitute() {
        let doc = json!({
            "packages": {},
            "metadata-url": "/p2/%package%.json"
        });
        assert_eq!(
            metadata_url_template(&doc).as_deref(),
            Some("/p2/%package%.json")
        );
        let url = resolve_package_metadata_url("https://repo.packagist.com/acme", &doc, "Acme/Foo")
            .unwrap()
            .unwrap();
        assert_eq!(url, "https://repo.packagist.com/p2/acme/foo.json");
    }

    #[test]
    fn metadata_url_absolute_template() {
        let doc = json!({
            "metadata-url": "https://repo.packagist.org/p2/%package%.json"
        });
        let url =
            resolve_package_metadata_url("https://repo.packagist.org", &doc, "monolog/monolog")
                .unwrap()
                .unwrap();
        assert_eq!(url, "https://repo.packagist.org/p2/monolog/monolog.json");
    }

    #[test]
    fn relative_path_metadata_url() {
        let abs = canonicalize_metadata_url("https://satis.example.com/foo", "p2/%package%.json");
        assert_eq!(abs, "https://satis.example.com/foo/p2/%package%.json");
    }

    #[test]
    fn missing_metadata_url_returns_none() {
        let doc = json!({ "provider-includes": {} });
        assert!(
            resolve_package_metadata_url("https://example.com", &doc, "a/b")
                .unwrap()
                .is_none()
        );
    }
}
