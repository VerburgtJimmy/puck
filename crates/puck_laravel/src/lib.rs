//! Native Laravel package discovery (`bootstrap/cache/packages.php`).
//!
//! Mirrors `Illuminate\Foundation\PackageManifest::build` without booting PHP.

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {message}")]
    Parse { path: String, message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Outcome of a discovery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverStatus {
    /// Project does not look like Laravel, or no `installed.json` yet.
    Skipped,
    /// Wrote `bootstrap/cache/packages.php`.
    Written { path: PathBuf, package_count: usize },
}

/// Build and write `bootstrap/cache/packages.php` when the project is Laravel-shaped.
///
/// Writes when `laravel/framework` is among installed packages or `bootstrap/` exists.
/// Creates `bootstrap/cache/` if needed (Composer/Laravel expect the directory).
pub fn discover(project_root: impl AsRef<Path>) -> Result<DiscoverStatus> {
    let root = project_root.as_ref();
    let installed_path = root.join("vendor/composer/installed.json");
    if !installed_path.is_file() {
        return Ok(DiscoverStatus::Skipped);
    }

    let packages = read_installed_packages(&installed_path)?;
    if !should_write_manifest(root, &packages) {
        return Ok(DiscoverStatus::Skipped);
    }

    let ignore = root_dont_discover(root);
    let manifest = build_manifest(&packages, &ignore);
    let path = write_packages_php(root, &manifest)?;
    Ok(DiscoverStatus::Written {
        path,
        package_count: manifest.len(),
    })
}

/// Whether discovery should produce `packages.php`.
pub fn should_write_manifest(project_root: &Path, packages: &[InstalledPackageMeta]) -> bool {
    packages
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case("laravel/framework"))
        || project_root.join("bootstrap").is_dir()
}

/// Package row used for discovery (name + optional `extra.laravel`).
#[derive(Debug, Clone, PartialEq)]
pub struct InstalledPackageMeta {
    pub name: String,
    pub laravel: Map<String, Value>,
}

/// Read installed packages and their `extra.laravel` from `installed.json`.
pub fn read_installed_packages(path: impl AsRef<Path>) -> Result<Vec<InstalledPackageMeta>> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|e| Error::Parse {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;

    let entries = match &value {
        Value::Object(obj) => obj.get("packages").cloned().unwrap_or(Value::Array(vec![])),
        Value::Array(_) => value,
        _ => {
            return Err(Error::Parse {
                path: path.display().to_string(),
                message: "installed.json root must be an object or array".into(),
            });
        }
    };

    let Value::Array(entries) = entries else {
        return Err(Error::Parse {
            path: path.display().to_string(),
            message: "installed.json packages must be an array".into(),
        });
    };

    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let Value::Object(obj) = entry else {
            continue;
        };
        let Some(Value::String(name)) = obj.get("name") else {
            continue;
        };
        let name = format_package_name(name, None);
        let laravel = obj
            .get("extra")
            .and_then(|e| e.get("laravel"))
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        out.push(InstalledPackageMeta { name, laravel });
    }
    Ok(out)
}

/// Root `extra.laravel.dont-discover` from `composer.json`.
pub fn root_dont_discover(project_root: &Path) -> Vec<String> {
    let path = project_root.join("composer.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    value
        .get("extra")
        .and_then(|e| e.get("laravel"))
        .and_then(|l| l.get("dont-discover"))
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Build the discovery map: package name -> laravel config (sorted by name).
pub fn build_manifest(
    packages: &[InstalledPackageMeta],
    root_ignore: &[String],
) -> BTreeMap<String, Map<String, Value>> {
    let mut ignore: Vec<String> = root_ignore.to_vec();
    let ignore_all = ignore.iter().any(|n| n == "*");

    // First pass: collect package-level dont-discover (Laravel merges these before reject).
    for pkg in packages {
        if let Some(Value::Array(list)) = pkg.laravel.get("dont-discover") {
            for item in list {
                if let Some(name) = item.as_str() {
                    ignore.push(name.to_owned());
                }
            }
        }
    }

    let mut manifest = BTreeMap::new();
    for pkg in packages {
        if ignore_all || ignore.iter().any(|n| n == &pkg.name) {
            continue;
        }
        if pkg.laravel.is_empty() {
            continue;
        }
        manifest.insert(pkg.name.clone(), pkg.laravel.clone());
    }
    manifest
}

/// Write `bootstrap/cache/packages.php` (creates directories as needed).
pub fn write_packages_php(
    project_root: &Path,
    manifest: &BTreeMap<String, Map<String, Value>>,
) -> Result<PathBuf> {
    let cache_dir = project_root.join("bootstrap/cache");
    fs::create_dir_all(&cache_dir).map_err(|source| Error::Io {
        path: cache_dir.display().to_string(),
        source,
    })?;

    let path = cache_dir.join("packages.php");
    let body = render_packages_php(manifest);
    fs::write(&path, body).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}

/// Render PHP matching `<?php return `.var_export($manifest, true).`;`.
pub fn render_packages_php(manifest: &BTreeMap<String, Map<String, Value>>) -> String {
    let mut map = Map::new();
    for (name, config) in manifest {
        map.insert(name.clone(), Value::Object(config.clone()));
    }
    format!("<?php return {};", var_export(&Value::Object(map), 0))
}

/// Strip a vendor-dir prefix the way Laravel's `PackageManifest::format` does.
fn format_package_name(name: &str, vendor_path: Option<&str>) -> String {
    let name = name.replace('\\', "/");
    if let Some(vendor) = vendor_path {
        let prefix = format!("{}/", vendor.trim_end_matches('/'));
        if let Some(rest) = name.strip_prefix(&prefix) {
            return rest.to_owned();
        }
    }
    name
}

/// PHP `var_export` for JSON values (2-space indent, `array (` form).
fn var_export(value: &Value, indent: usize) -> String {
    match value {
        Value::Null => "NULL".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => php_export_string(s),
        Value::Array(items) => {
            let entries: Vec<(String, &Value)> = items
                .iter()
                .enumerate()
                .map(|(i, v)| (i.to_string(), v))
                .collect();
            export_array(&entries, indent, false)
        }
        Value::Object(map) => {
            let entries: Vec<(String, &Value)> = map.iter().map(|(k, v)| (k.clone(), v)).collect();
            export_array(&entries, indent, true)
        }
    }
}

fn export_array(entries: &[(String, &Value)], indent: usize, string_keys: bool) -> String {
    let pad = "  ".repeat(indent);
    let inner = "  ".repeat(indent + 1);
    if entries.is_empty() {
        return format!("array (\n{pad})");
    }

    let mut out = String::from("array (\n");
    for (key, value) in entries {
        out.push_str(&inner);
        if string_keys {
            out.push_str(&php_export_string(key));
        } else {
            out.push_str(key);
        }
        out.push_str(" => ");
        match value {
            Value::Array(_) | Value::Object(_) => {
                out.push('\n');
                out.push_str(&inner);
                out.push_str(&var_export(value, indent + 1));
            }
            _ => out.push_str(&var_export(value, indent + 1)),
        }
        out.push_str(",\n");
    }
    out.push_str(&pad);
    out.push(')');
    out
}

fn php_export_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            _ => out.push(ch),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    fn meta(name: &str, laravel: Value) -> InstalledPackageMeta {
        InstalledPackageMeta {
            name: name.into(),
            laravel: laravel.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn builds_sorted_manifest_skipping_empty() {
        let packages = vec![
            meta("zebra/pkg", json!({"providers": ["Z\\Prov"]})),
            meta(
                "nesbot/carbon",
                json!({"providers": ["Carbon\\Laravel\\ServiceProvider"]}),
            ),
            meta("plain/lib", json!({})),
        ];
        let manifest = build_manifest(&packages, &[]);
        let keys: Vec<_> = manifest.keys().cloned().collect();
        assert_eq!(keys, vec!["nesbot/carbon", "zebra/pkg"]);
    }

    #[test]
    fn honors_star_dont_discover() {
        let packages = vec![meta(
            "nesbot/carbon",
            json!({"providers": ["Carbon\\Laravel\\ServiceProvider"]}),
        )];
        let manifest = build_manifest(&packages, &["*".into()]);
        assert!(manifest.is_empty());
    }

    #[test]
    fn honors_root_and_package_dont_discover() {
        let packages = vec![
            meta(
                "nesbot/carbon",
                json!({"providers": ["Carbon\\Laravel\\ServiceProvider"]}),
            ),
            meta(
                "laravel/tinker",
                json!({"providers": ["Laravel\\Tinker\\TinkerServiceProvider"]}),
            ),
            meta("acme/blocker", json!({"dont-discover": ["laravel/tinker"]})),
        ];
        let manifest = build_manifest(&packages, &["nesbot/carbon".into()]);
        assert!(!manifest.contains_key("nesbot/carbon"));
        assert!(!manifest.contains_key("laravel/tinker"));
        assert!(manifest.contains_key("acme/blocker"));
    }

    #[test]
    fn render_matches_var_export_shape() {
        let mut manifest = BTreeMap::new();
        manifest.insert(
            "nesbot/carbon".into(),
            json!({"providers": ["Carbon\\Laravel\\ServiceProvider"]})
                .as_object()
                .cloned()
                .expect("object"),
        );
        let php = render_packages_php(&manifest);
        assert!(php.starts_with("<?php return array ("));
        assert!(php.contains("'nesbot/carbon'"));
        assert!(php.contains("'providers'"));
        assert!(php.contains("0 => 'Carbon\\\\Laravel\\\\ServiceProvider'"));
        assert!(php.ends_with(");"));
    }

    #[test]
    fn discover_writes_for_framework_without_bootstrap() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("vendor/composer")).expect("mkdir");
        let installed = json!({
            "packages": [
                {
                    "name": "laravel/framework",
                    "version": "v13.0.0",
                    "extra": {}
                },
                {
                    "name": "nesbot/carbon",
                    "version": "3.0.0",
                    "extra": {
                        "laravel": {
                            "providers": ["Carbon\\Laravel\\ServiceProvider"]
                        }
                    }
                }
            ],
            "dev": false,
            "dev-package-names": []
        });
        fs::write(
            root.join("vendor/composer/installed.json"),
            serde_json::to_string_pretty(&installed).expect("json"),
        )
        .expect("write installed");
        fs::write(
            root.join("composer.json"),
            r#"{"name":"app/app","extra":{"laravel":{"dont-discover":[]}}}"#,
        )
        .expect("write composer.json");

        let status = discover(root).expect("discover");
        match status {
            DiscoverStatus::Written {
                path,
                package_count,
            } => {
                assert_eq!(package_count, 1);
                assert!(path.ends_with("bootstrap/cache/packages.php"));
                let body = fs::read_to_string(path).expect("read");
                assert!(body.contains("'nesbot/carbon'"));
            }
            DiscoverStatus::Skipped => panic!("expected write"),
        }
    }

    #[test]
    fn skips_non_laravel_project() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("vendor/composer")).expect("mkdir");
        let installed = json!({
            "packages": [
                { "name": "brick/math", "version": "0.18.0", "extra": {} }
            ],
            "dev": true,
            "dev-package-names": []
        });
        fs::write(
            root.join("vendor/composer/installed.json"),
            serde_json::to_string_pretty(&installed).expect("json"),
        )
        .expect("write");
        assert_eq!(discover(root).expect("ok"), DiscoverStatus::Skipped);
    }
}
