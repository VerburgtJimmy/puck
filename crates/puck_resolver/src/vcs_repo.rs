//! VCS repository loading (`Composer\Repository\VcsRepository` git subset).
//!
//! Loads `type: vcs` entries, mirrors via [`puck_vcs`], and exposes one pool
//! package per tag/branch that has a readable `composer.json`.

use crate::metadata::package_from_composer_package;
use crate::package::Package;
use crate::{Error, Result};
use puck_vcs::{
    GitPackageVersion, default_vcs_cache_root, ensure_git_mirror, list_git_versions,
    read_composer_json_at,
};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// A package version loaded from a git VCS repository.
#[derive(Debug, Clone)]
pub struct VcsPackage {
    pub package: Package,
    /// Repository URL as written in `composer.json`.
    pub url: String,
    /// Resolved commit sha for lock `source.reference`.
    pub reference: String,
    /// Git ref used to read `composer.json` (tag or branch name).
    pub ref_name: String,
    pub canonical: bool,
    /// Raw package `composer.json` object (version forced to pretty).
    pub composer: Value,
}

impl VcsPackage {
    /// Build a lock package object with `source.type=git`.
    pub fn to_lock_value(&self) -> Value {
        let mut data = Map::new();
        data.insert("name".into(), json!(self.package.name));
        data.insert("version".into(), json!(self.package.pretty_version));

        let mut source = Map::new();
        source.insert("type".into(), json!("git"));
        source.insert("url".into(), json!(self.url));
        source.insert("reference".into(), json!(self.reference));
        data.insert("source".into(), Value::Object(source));

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

        Value::Object(data)
    }
}

/// One `type: vcs` repository entry (order + `canonical` preserved).
#[derive(Debug, Clone)]
pub struct VcsRepository {
    pub canonical: bool,
    pub packages: Vec<VcsPackage>,
}

/// Load each `type: vcs` repository in `repositories` order (skips non-vcs).
pub fn load_vcs_repositories(
    project_root: &Path,
    root_composer: &Value,
    cache_root: Option<&Path>,
) -> Result<Vec<VcsRepository>> {
    let Some(repos) = root_composer.get("repositories") else {
        return Ok(Vec::new());
    };

    let cache = cache_root
        .map(Path::to_path_buf)
        .unwrap_or_else(default_vcs_cache_root);

    let mut out = Vec::new();
    match repos {
        Value::Array(arr) => {
            for entry in arr {
                if let Some(repo) = load_one_vcs_repository(project_root, entry, &cache)? {
                    out.push(repo);
                }
            }
        }
        Value::Object(map) => {
            for (_key, entry) in map {
                if let Some(repo) = load_one_vcs_repository(project_root, entry, &cache)? {
                    out.push(repo);
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

/// Package names provided by VCS repositories (lowercase).
pub fn vcs_package_names(packages: &[VcsPackage]) -> indexmap::IndexSet<String> {
    packages.iter().map(|p| p.package.name.clone()).collect()
}

fn load_one_vcs_repository(
    project_root: &Path,
    entry: &Value,
    cache_root: &Path,
) -> Result<Option<VcsRepository>> {
    let Some(obj) = entry.as_object() else {
        return Ok(None);
    };
    if obj.get("type").and_then(|v| v.as_str()) != Some("vcs") {
        return Ok(None);
    }
    let Some(url_raw) = obj.get("url").and_then(|v| v.as_str()) else {
        return Err(Error::Message("vcs repository missing string url".into()));
    };

    let canonical = obj
        .get("canonical")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let url = resolve_vcs_url(project_root, url_raw);

    let mirror = ensure_git_mirror(cache_root, &url).map_err(|e| Error::Message(e.to_string()))?;
    let versions =
        list_git_versions(&mirror).map_err(|e| Error::Message(format!("vcs {url}: {e}")))?;

    let mut packages = Vec::new();
    for ver in versions {
        if let Some(pkg) = load_vcs_package_at(&url, &mirror, &ver, canonical)? {
            packages.push(pkg);
        }
    }

    Ok(Some(VcsRepository {
        canonical,
        packages,
    }))
}

fn load_vcs_package_at(
    url: &str,
    mirror: &Path,
    ver: &GitPackageVersion,
    canonical: bool,
) -> Result<Option<VcsPackage>> {
    let body = match read_composer_json_at(mirror, &ver.ref_name) {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };
    let mut composer: Value = serde_json::from_str(&body).map_err(|e| {
        Error::Message(format!(
            "vcs {url} @{}: invalid composer.json: {e}",
            ver.ref_name
        ))
    })?;
    if let Some(obj) = composer.as_object_mut() {
        obj.insert("version".into(), json!(ver.pretty_version));
    }

    let Some(package) = package_from_composer_package(&composer)? else {
        return Ok(None);
    };

    Ok(Some(VcsPackage {
        package,
        url: url.to_string(),
        reference: ver.reference.clone(),
        ref_name: ver.ref_name.clone(),
        canonical,
        composer,
    }))
}

fn resolve_vcs_url(project_root: &Path, url: &str) -> String {
    let u = url.trim();
    if u.starts_with("git@")
        || u.starts_with("ssh://")
        || u.starts_with("http://")
        || u.starts_with("https://")
        || u.starts_with("file://")
        || u.starts_with("git://")
    {
        return u.to_string();
    }
    let path = PathBuf::from(u);
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    project_root.join(path).to_string_lossy().into_owned()
}

fn is_empty_value(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}
