//! Read `vendor/composer/installed.json` (Composer 2 format).

use crate::{Error, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Currently installed package as recorded by Composer / puck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub is_dev: bool,
}

/// Snapshot of what is already in `vendor/`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledState {
    pub packages: BTreeMap<String, InstalledPackage>,
}

#[derive(Debug, Deserialize)]
struct InstalledFile {
    #[serde(default)]
    packages: Vec<InstalledEntry>,
    /// Composer 2.0 used a bare array; accept either.
    #[serde(default)]
    #[allow(dead_code)]
    dev: Option<bool>,
    #[serde(rename = "dev-package-names", default)]
    dev_package_names: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct InstalledEntry {
    name: String,
    version: String,
    #[serde(default)]
    dev: Option<bool>,
}

/// Load installed state from `vendor/composer/installed.json`.
///
/// Missing file => empty state (fresh install).
pub fn read_installed(vendor_composer_dir: impl AsRef<Path>) -> Result<InstalledState> {
    let path = vendor_composer_dir.as_ref().join("installed.json");
    if !path.exists() {
        return Ok(InstalledState::default());
    }
    let text = fs::read_to_string(&path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse_installed_json(&text)
}

fn parse_installed_json(text: &str) -> Result<InstalledState> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| Error::InstalledParse(e.to_string()))?;

    // Composer 1: bare array. Composer 2: object with `packages`.
    let (entries, dev_names): (Vec<InstalledEntry>, Vec<String>) = match value {
        Value::Array(_) => {
            let entries: Vec<InstalledEntry> =
                serde_json::from_str(text).map_err(|e| Error::InstalledParse(e.to_string()))?;
            (entries, Vec::new())
        }
        Value::Object(_) => {
            let file: InstalledFile =
                serde_json::from_str(text).map_err(|e| Error::InstalledParse(e.to_string()))?;
            (file.packages, file.dev_package_names)
        }
        _ => {
            return Err(Error::InstalledParse(
                "installed.json root must be an object or array".into(),
            ));
        }
    };

    let mut packages = BTreeMap::new();
    for entry in entries {
        let name = entry.name.to_ascii_lowercase();
        let is_dev = entry.dev.unwrap_or_else(|| {
            dev_names
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&entry.name))
        });
        packages.insert(
            name.clone(),
            InstalledPackage {
                name,
                version: entry.version,
                is_dev,
            },
        );
    }
    Ok(InstalledState { packages })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_composer2_installed() {
        let json = r#"{
            "packages": [
                { "name": "Foo/Bar", "version": "1.0.0.0", "version_normalized": "1.0.0.0" },
                { "name": "acme/dev", "version": "dev-main", "dev": true }
            ],
            "dev": true,
            "dev-package-names": ["acme/dev"]
        }"#;
        let state = parse_installed_json(json).expect("parse");
        assert_eq!(state.packages.len(), 2);
        assert_eq!(state.packages["foo/bar"].version, "1.0.0.0");
        assert!(!state.packages["foo/bar"].is_dev);
        assert!(state.packages["acme/dev"].is_dev);
    }
}
