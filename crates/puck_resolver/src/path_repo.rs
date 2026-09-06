//! Path repository loading (`Composer\Repository\PathRepository` subset).
//!
//! Loads `type: path` entries from root `composer.json` `repositories`, reads each
//! `{url}/composer.json`, and exposes pool packages plus lock dump metadata.
//! Does not expand glob `*` urls (out of scope).

use crate::metadata::package_from_composer_package;
use crate::package::Package;
use crate::{Error, Result};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Transport options for a path repository (`options.symlink` / `options.relative`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathTransportOptions {
    pub symlink: bool,
    pub relative: bool,
}

impl Default for PathTransportOptions {
    fn default() -> Self {
        Self {
            symlink: true,
            relative: true,
        }
    }
}

/// A package loaded from a path repository, ready for the pool and lock dump.
#[derive(Debug, Clone)]
pub struct PathPackage {
    pub package: Package,
    /// Repository `url` as written in root `composer.json` (usually project-relative).
    pub url: String,
    pub options: PathTransportOptions,
    /// Raw package `composer.json` object (with version defaulted when missing).
    pub composer: Value,
}

impl PathPackage {
    /// Build a lock package object (`dist.type=path`, `transport-options`).
    pub fn to_lock_value(&self) -> Value {
        let mut data = Map::new();
        data.insert("name".into(), json!(self.package.name));
        data.insert("version".into(), json!(self.package.pretty_version));

        let mut dist = Map::new();
        dist.insert("type".into(), json!("path"));
        dist.insert("url".into(), json!(self.url));
        // Stable reference without VCS guessing (Composer would use git / content hash).
        dist.insert("reference".into(), json!(path_reference(&self.url)));
        data.insert("dist".into(), Value::Object(dist));

        if let Some(obj) = self.composer.as_object() {
            for key in [
                "type",
                "extra",
                "autoload",
                "autoload-dev",
                "bin",
                "include-path",
                "scripts",
                "license",
                "authors",
                "description",
                "homepage",
                "keywords",
                "support",
                "funding",
            ] {
                if let Some(v) = obj.get(key) {
                    if !is_empty_value(v) {
                        data.insert(key.into(), v.clone());
                    }
                }
            }
            for link_type in ["require", "conflict", "provide", "replace", "require-dev"] {
                if let Some(Value::Object(map)) = obj.get(link_type) {
                    if !map.is_empty() {
                        data.insert(link_type.into(), Value::Object(map.clone()));
                    }
                }
            }
        }

        let mut transport = Map::new();
        transport.insert("symlink".into(), json!(self.options.symlink));
        transport.insert("relative".into(), json!(self.options.relative));
        data.insert("transport-options".into(), Value::Object(transport));

        Value::Object(data)
    }
}

/// Parse root `repositories` and load each `type: path` package under `project_root`.
pub fn load_path_packages(project_root: &Path, root_composer: &Value) -> Result<Vec<PathPackage>> {
    let Some(repos) = root_composer.get("repositories") else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    match repos {
        Value::Array(arr) => {
            for entry in arr {
                if let Some(pkg) = load_one_path_repo(project_root, entry)? {
                    out.push(pkg);
                }
            }
        }
        Value::Object(map) => {
            // Composer also allows object-keyed repositories.
            for (_key, entry) in map {
                if let Some(pkg) = load_one_path_repo(project_root, entry)? {
                    out.push(pkg);
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

/// Package names provided by path repositories (lowercase).
pub fn path_package_names(packages: &[PathPackage]) -> indexmap::IndexSet<String> {
    packages.iter().map(|p| p.package.name.clone()).collect()
}

fn load_one_path_repo(project_root: &Path, entry: &Value) -> Result<Option<PathPackage>> {
    let Some(obj) = entry.as_object() else {
        return Ok(None);
    };
    if obj.get("type").and_then(|v| v.as_str()) != Some("path") {
        return Ok(None);
    }
    let Some(url) = obj.get("url").and_then(|v| v.as_str()) else {
        return Err(Error::Message(
            "path repository missing string url".into(),
        ));
    };
    if url.contains('*') {
        // Glob path urls are out of scope for this slice.
        return Ok(None);
    }

    let options = options_from_entry(obj);
    let package_dir = resolve_path_url(project_root, url);
    let composer_path = package_dir.join("composer.json");
    if !composer_path.is_file() {
        return Err(Error::Message(format!(
            "path repository {}: missing {}",
            url,
            composer_path.display()
        )));
    }
    let bytes = std::fs::read(&composer_path).map_err(|e| {
        Error::Message(format!("read {}: {e}", composer_path.display()))
    })?;
    let mut composer: Value = serde_json::from_slice(&bytes).map_err(|e| {
        Error::Message(format!("invalid {}: {e}", composer_path.display()))
    })?;

    if composer
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty()
    {
        // Composer VersionGuesser fallback when no VCS / version field.
        if let Some(obj) = composer.as_object_mut() {
            obj.insert("version".into(), json!("dev-main"));
        }
    }

    let package = package_from_composer_package(&composer)?.ok_or_else(|| {
        Error::Message(format!(
            "path repository {}: package composer.json has no name/version",
            url
        ))
    })?;

    Ok(Some(PathPackage {
        package,
        url: url.to_string(),
        options,
        composer,
    }))
}

fn options_from_entry(obj: &Map<String, Value>) -> PathTransportOptions {
    let opts = obj.get("options").and_then(|v| v.as_object());
    PathTransportOptions {
        symlink: bool_option(opts, "symlink", true),
        relative: bool_option(opts, "relative", true),
    }
}

fn bool_option(opts: Option<&Map<String, Value>>, key: &str, default: bool) -> bool {
    let Some(opts) = opts else {
        return default;
    };
    match opts.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) if n.as_u64() == Some(0) => false,
        Some(Value::Number(n)) if n.as_u64() == Some(1) => true,
        Some(Value::String(s)) if s == "false" || s == "0" => false,
        Some(Value::String(s)) if s == "true" || s == "1" => true,
        _ => default,
    }
}

fn resolve_path_url(project_root: &Path, url: &str) -> PathBuf {
    let p = Path::new(url);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        project_root.join(p)
    }
}

fn path_reference(url: &str) -> String {
    // Deterministic stand-in (no content hash / git). Prefer stable over empty.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    url.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn path_local_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/path-local")
    }

    #[test]
    fn loads_path_local_fixture_acme_hello() {
        let root = path_local_root();
        let composer: Value = serde_json::from_str(
            &fs::read_to_string(root.join("composer.json")).unwrap(),
        )
        .unwrap();
        let pkgs = load_path_packages(&root, &composer).unwrap();
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].package.name, "acme/hello");
        assert_eq!(pkgs[0].package.pretty_version, "dev-main");
        assert_eq!(pkgs[0].url, "packages/acme-hello");
        assert!(pkgs[0].options.symlink);
        assert!(pkgs[0].options.relative);

        let lock = pkgs[0].to_lock_value();
        assert_eq!(lock["dist"]["type"], "path");
        assert_eq!(lock["dist"]["url"], "packages/acme-hello");
        assert_eq!(lock["transport-options"]["symlink"], true);
        assert_eq!(lock["transport-options"]["relative"], true);
        assert!(lock.get("notification-url").is_none());
    }

    #[test]
    fn loads_from_tempfile_with_options() {
        let dir = std::env::temp_dir().join(format!(
            "puck-path-repo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let pkg_dir = dir.join("local-pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("composer.json"),
            r#"{"name":"tmp/path-pkg","type":"library"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("composer.json"),
            r#"{
                "require": { "tmp/path-pkg": "*" },
                "repositories": [
                    {
                        "type": "path",
                        "url": "local-pkg",
                        "options": { "symlink": false, "relative": false }
                    }
                ]
            }"#,
        )
        .unwrap();

        let composer: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("composer.json")).unwrap()).unwrap();
        let pkgs = load_path_packages(&dir, &composer).unwrap();
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].package.name, "tmp/path-pkg");
        assert_eq!(pkgs[0].package.pretty_version, "dev-main");
        assert!(!pkgs[0].options.symlink);
        assert!(!pkgs[0].options.relative);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_non_path_and_glob_urls() {
        let dir = std::env::temp_dir().join(format!(
            "puck-path-skip-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let root = json!({
            "repositories": [
                { "type": "composer", "url": "https://example.test" },
                { "type": "path", "url": "packages/*" }
            ]
        });
        let pkgs = load_path_packages(&dir, &root).unwrap();
        assert!(pkgs.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
