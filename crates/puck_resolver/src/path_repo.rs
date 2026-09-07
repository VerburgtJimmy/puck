//! Path repository loading (`Composer\Repository\PathRepository` subset).
//!
//! Loads `type: path` entries from root `composer.json` `repositories`, reads each
//! `{url}/composer.json`, and exposes pool packages plus lock dump metadata.
//! Relative urls may include `*` globs (e.g. `packages/*`); matches are directories
//! that contain `composer.json`.

use crate::metadata::package_from_composer_package;
use crate::package::Package;
use crate::{Error, Result};
use serde_json::{Map, Value, json};
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
    /// Repository `url` as written for this package (concrete path after glob expand).
    pub url: String,
    pub options: PathTransportOptions,
    /// Composer `canonical` on the path repository entry (default true).
    pub canonical: bool,
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
                if let Some(v) = obj.get(key)
                    && !is_empty_value(v)
                {
                    data.insert(key.into(), v.clone());
                }
            }
            for link_type in ["require", "conflict", "provide", "replace", "require-dev"] {
                if let Some(Value::Object(map)) = obj.get(link_type)
                    && !map.is_empty()
                {
                    data.insert(link_type.into(), Value::Object(map.clone()));
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
                out.extend(load_one_path_repo(project_root, entry)?);
            }
        }
        Value::Object(map) => {
            // Composer also allows object-keyed repositories.
            for (_key, entry) in map {
                out.extend(load_one_path_repo(project_root, entry)?);
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

/// One `type: path` repository entry (order + `canonical` preserved).
#[derive(Debug, Clone)]
pub struct PathRepository {
    pub canonical: bool,
    pub packages: Vec<PathPackage>,
}

/// Load each `type: path` repository in `repositories` order (skips non-path entries).
pub fn load_path_repositories(
    project_root: &Path,
    root_composer: &Value,
) -> Result<Vec<PathRepository>> {
    let Some(repos) = root_composer.get("repositories") else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    match repos {
        Value::Array(arr) => {
            for entry in arr {
                if let Some(repo) = load_one_path_repository(project_root, entry)? {
                    out.push(repo);
                }
            }
        }
        Value::Object(map) => {
            for (_key, entry) in map {
                if let Some(repo) = load_one_path_repository(project_root, entry)? {
                    out.push(repo);
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

fn load_one_path_repo(project_root: &Path, entry: &Value) -> Result<Vec<PathPackage>> {
    Ok(match load_one_path_repository(project_root, entry)? {
        Some(repo) => repo.packages,
        None => Vec::new(),
    })
}

fn load_one_path_repository(project_root: &Path, entry: &Value) -> Result<Option<PathRepository>> {
    let Some(obj) = entry.as_object() else {
        return Ok(None);
    };
    if obj.get("type").and_then(|v| v.as_str()) != Some("path") {
        return Ok(None);
    }
    let Some(url) = obj.get("url").and_then(|v| v.as_str()) else {
        return Err(Error::Message("path repository missing string url".into()));
    };

    let canonical = obj
        .get("canonical")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let options = options_from_entry(obj);
    let targets = expand_path_repo_urls(project_root, url)?;
    if targets.is_empty() {
        if url.contains('*') {
            // Glob with no composer.json matches: empty repository (Composer-like).
            return Ok(Some(PathRepository {
                canonical,
                packages: Vec::new(),
            }));
        }
        return Err(Error::Message(format!(
            "path repository {}: no package directory found",
            url
        )));
    }

    let mut packages = Vec::new();
    for (concrete_url, package_dir) in targets {
        packages.push(load_path_package_at(
            &concrete_url,
            &package_dir,
            options,
            canonical,
        )?);
    }
    Ok(Some(PathRepository {
        canonical,
        packages,
    }))
}

fn load_path_package_at(
    url: &str,
    package_dir: &Path,
    options: PathTransportOptions,
    canonical: bool,
) -> Result<PathPackage> {
    let composer_path = package_dir.join("composer.json");
    if !composer_path.is_file() {
        return Err(Error::Message(format!(
            "path repository {}: missing {}",
            url,
            composer_path.display()
        )));
    }
    let bytes = std::fs::read(&composer_path)
        .map_err(|e| Error::Message(format!("read {}: {e}", composer_path.display())))?;
    let mut composer: Value = serde_json::from_slice(&bytes)
        .map_err(|e| Error::Message(format!("invalid {}: {e}", composer_path.display())))?;

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

    Ok(PathPackage {
        package,
        url: url.to_string(),
        options,
        canonical,
        composer,
    })
}

/// Expand a path-repo `url` into concrete `(lock_url, absolute_dir)` pairs.
///
/// Non-glob urls yield a single candidate. Glob urls (containing `*`) match
/// directories under the pattern that contain `composer.json`.
fn expand_path_repo_urls(project_root: &Path, url: &str) -> Result<Vec<(String, PathBuf)>> {
    if !url.contains('*') {
        let package_dir = resolve_path_url(project_root, url);
        return Ok(vec![(url.to_string(), package_dir)]);
    }

    let pattern = resolve_path_url(project_root, url);
    let matched_dirs = match_glob_directories(&pattern)?;
    let mut out = Vec::new();
    for dir in matched_dirs {
        if !dir.join("composer.json").is_file() {
            continue;
        }
        let concrete = concrete_url_for(project_root, url, &dir);
        out.push((concrete, dir));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn concrete_url_for(project_root: &Path, original_url: &str, dir: &Path) -> String {
    if Path::new(original_url).is_absolute() {
        return dir.to_string_lossy().replace('\\', "/");
    }
    match dir.strip_prefix(project_root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => dir.to_string_lossy().replace('\\', "/"),
    }
}

/// Walk path components of `pattern`, expanding `*` wildcards in components.
fn match_glob_directories(pattern: &Path) -> Result<Vec<PathBuf>> {
    let components: Vec<String> = pattern
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if components.is_empty() {
        return Ok(Vec::new());
    }

    // Absolute patterns keep a root prefix; relative patterns start from "".
    let (mut roots, start_idx) = if pattern.is_absolute() {
        let root = PathBuf::from(&components[0]);
        // On Windows `C:` etc.; on Unix `/` is a component via Prefix/RootDir.
        // `Path::components` yields RootDir as `/` on Unix.
        (vec![root], 1)
    } else {
        (vec![PathBuf::new()], 0)
    };

    for comp in &components[start_idx..] {
        let mut next = Vec::new();
        for root in &roots {
            let base = if root.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                root.clone()
            };
            if comp.contains('*') {
                let read_dir = match std::fs::read_dir(&base) {
                    Ok(rd) => rd,
                    Err(_) => continue,
                };
                for entry in read_dir.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if !wildcard_match(comp, &name_str) {
                        continue;
                    }
                    let path = entry.path();
                    if path.is_dir() {
                        next.push(path);
                    }
                }
            } else {
                let path = base.join(comp);
                if path.is_dir() {
                    next.push(path);
                }
            }
        }
        roots = next;
        if roots.is_empty() {
            break;
        }
    }

    // Normalize "."-relative roots back to absolute-ish paths without "./".
    let cleaned: Vec<PathBuf> = roots
        .into_iter()
        .map(|p| {
            if p.starts_with(".") {
                std::fs::canonicalize(&p).unwrap_or(p)
            } else {
                p
            }
        })
        .collect();
    Ok(cleaned)
}

/// Match `pattern` against `name` where `*` matches any sequence (no `/`).
fn wildcard_match(pattern: &str, name: &str) -> bool {
    fn rec(p: &[u8], n: &[u8]) -> bool {
        match (p.first().copied(), n.first().copied()) {
            (None, None) => true,
            (Some(b'*'), _) => rec(&p[1..], n) || (!n.is_empty() && rec(p, &n[1..])),
            (Some(pc), Some(nc)) if pc == nc => rec(&p[1..], &n[1..]),
            _ => false,
        }
    }
    rec(pattern.as_bytes(), name.as_bytes())
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
        let composer: Value =
            serde_json::from_str(&fs::read_to_string(root.join("composer.json")).unwrap()).unwrap();
        let pkgs = load_path_packages(&root, &composer).unwrap();
        assert!(
            pkgs.iter().any(|p| p.package.name == "acme/hello"),
            "expected acme/hello in {:?}",
            pkgs.iter().map(|p| &p.package.name).collect::<Vec<_>>()
        );
        let hello = pkgs
            .iter()
            .find(|p| p.package.name == "acme/hello")
            .unwrap();
        assert_eq!(hello.package.pretty_version, "dev-main");
        assert_eq!(hello.url, "packages/acme-hello");
        assert!(hello.options.symlink);
        assert!(hello.options.relative);

        let lock = hello.to_lock_value();
        assert_eq!(lock["dist"]["type"], "path");
        assert_eq!(lock["dist"]["url"], "packages/acme-hello");
        assert_eq!(lock["transport-options"]["symlink"], true);
        assert_eq!(lock["transport-options"]["relative"], true);
        assert!(lock.get("notification-url").is_none());
    }

    #[test]
    fn expands_packages_star_glob_in_fixture() {
        let root = path_local_root();
        let composer = json!({
            "repositories": [
                { "type": "path", "url": "packages/*" }
            ]
        });
        let pkgs = load_path_packages(&root, &composer).unwrap();
        let names: Vec<_> = pkgs.iter().map(|p| p.package.name.as_str()).collect();
        assert!(names.contains(&"acme/hello"), "names={names:?}");
        assert!(names.contains(&"acme/world"), "names={names:?}");
        assert_eq!(pkgs.len(), 2);
        for p in &pkgs {
            assert!(p.url.starts_with("packages/"), "url={}", p.url);
            assert!(!p.url.contains('*'));
        }
    }

    #[test]
    fn expands_glob_in_tempfile_and_skips_dirs_without_composer() {
        let dir = std::env::temp_dir().join(format!(
            "puck-path-glob-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let a = dir.join("packages/a");
        let b = dir.join("packages/b");
        let empty = dir.join("packages/empty");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::create_dir_all(&empty).unwrap();
        fs::write(
            a.join("composer.json"),
            r#"{"name":"tmp/a","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(
            b.join("composer.json"),
            r#"{"name":"tmp/b","version":"2.0.0"}"#,
        )
        .unwrap();
        // empty/ has no composer.json
        let root = json!({
            "repositories": [
                { "type": "path", "url": "packages/*" }
            ]
        });
        let pkgs = load_path_packages(&dir, &root).unwrap();
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].url, "packages/a");
        assert_eq!(pkgs[0].package.name, "tmp/a");
        assert_eq!(pkgs[1].url, "packages/b");
        assert_eq!(pkgs[1].package.name, "tmp/b");
        let _ = fs::remove_dir_all(&dir);
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
    fn skips_non_path_and_empty_glob() {
        let dir = std::env::temp_dir().join(format!(
            "puck-path-skip-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join("packages")).unwrap();
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

    #[test]
    fn wildcard_match_basics() {
        assert!(wildcard_match("*", "acme-hello"));
        assert!(wildcard_match("acme-*", "acme-hello"));
        assert!(!wildcard_match("acme-*", "other"));
        assert!(wildcard_match("a*e", "acme"));
    }
}
