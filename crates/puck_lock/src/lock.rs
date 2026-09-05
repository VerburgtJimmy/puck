//! composer.lock types and parsing.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;
use std::str::FromStr;

/// Parsed Composer 2 `composer.lock`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockFile {
    #[serde(rename = "content-hash", default)]
    pub content_hash: String,
    #[serde(default)]
    pub packages: Vec<LockedPackage>,
    #[serde(rename = "packages-dev", default)]
    pub packages_dev: Vec<LockedPackage>,
    #[serde(rename = "plugin-api-version", default)]
    pub plugin_api_version: Option<String>,
    #[serde(rename = "minimum-stability", default)]
    pub minimum_stability: Option<String>,
    #[serde(rename = "prefer-stable", default)]
    pub prefer_stable: Option<bool>,
    #[serde(rename = "prefer-lowest", default)]
    pub prefer_lowest: Option<bool>,
    #[serde(default)]
    pub aliases: Value,
    #[serde(rename = "stability-flags", default)]
    pub stability_flags: Value,
    #[serde(default)]
    pub platform: Map<String, Value>,
    #[serde(rename = "platform-dev", default)]
    pub platform_dev: Map<String, Value>,
    /// Remaining lock fields (`_readme`, `hash`, …).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A locked package entry in `packages` / `packages-dev`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub dist: Option<Dist>,
    #[serde(default)]
    pub source: Option<Source>,
    /// Executable paths relative to the package root (`composer.json` `bin`).
    #[serde(default)]
    pub bin: Vec<String>,
    /// Remaining package fields (`require`, `type`, `autoload`, …).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl LockedPackage {
    /// Bin entries from the lock, including a fallback if `bin` landed in `extra`.
    pub fn bins(&self) -> Vec<String> {
        if !self.bin.is_empty() {
            return self.bin.clone();
        }
        match self.extra.get("bin") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        }
    }

    /// Package `type` from the lock (`extra["type"]`), defaulting to `library`.
    pub fn package_type(&self) -> &str {
        self.extra
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("library")
    }

    /// Abandoned status from the lock (`extra["abandoned"]`).
    ///
    /// Composer stores `true` or a replacement package name string
    /// (`CompletePackage::setAbandoned` / ArrayDumper).
    pub fn abandoned(&self) -> Option<Abandoned> {
        match self.extra.get("abandoned") {
            Some(Value::Bool(true)) => Some(Abandoned::NoReplacement),
            Some(Value::String(name)) if !name.is_empty() => {
                Some(Abandoned::Replacement(name.clone()))
            }
            _ => None,
        }
    }
}

/// Lock / p2 `abandoned` field (Composer CompletePackage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Abandoned {
    /// `abandoned: true` - no suggested replacement.
    NoReplacement,
    /// `abandoned: "vendor/package"` - suggested replacement.
    Replacement(String),
}

impl Abandoned {
    /// Composer Installer stderr line (without IO colour tags).
    pub fn warning_line(&self, package_name: &str) -> String {
        match self {
            Self::NoReplacement => format!(
                "Package {package_name} is abandoned, you should avoid using it. No replacement was suggested."
            ),
            Self::Replacement(replacement) => format!(
                "Package {package_name} is abandoned, you should avoid using it. Use {replacement} instead."
            ),
        }
    }
}

/// Emit Composer-shaped abandoned warnings for locked packages.
///
/// Mirrors `Composer\Installer::run` after suggestions: locked repo with
/// dev packages when `include_dev` is true.
pub fn abandoned_warnings<'a>(
    packages: impl IntoIterator<Item = &'a LockedPackage>,
    packages_dev: impl IntoIterator<Item = &'a LockedPackage>,
    include_dev: bool,
) -> Vec<String> {
    let mut out = Vec::new();
    for pkg in packages {
        if let Some(abandoned) = pkg.abandoned() {
            out.push(abandoned.warning_line(&pkg.name));
        }
    }
    if include_dev {
        for pkg in packages_dev {
            if let Some(abandoned) = pkg.abandoned() {
                out.push(abandoned.warning_line(&pkg.name));
            }
        }
    }
    out
}

/// Dist reference on a locked package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dist {
    #[serde(rename = "type", default)]
    pub dist_type: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub shasum: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// VCS / source reference on a locked package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    #[serde(rename = "type", default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl LockFile {
    /// Parse a lock file from a filesystem path.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(|e| Error::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        Self::from_str(&contents)
    }
}

impl FromStr for LockFile {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| Error::Parse(e.to_string()))
    }
}

#[cfg(test)]
mod abandoned_tests {
    use super::*;
    use serde_json::json;

    fn pkg(name: &str, abandoned: Option<Value>) -> LockedPackage {
        let mut extra = Map::new();
        if let Some(v) = abandoned {
            extra.insert("abandoned".into(), v);
        }
        LockedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            dist: None,
            source: None,
            bin: Vec::new(),
            extra,
        }
    }

    #[test]
    fn warns_true_and_replacement() {
        let pkgs = vec![
            pkg("old/lib", Some(Value::Bool(true))),
            pkg("old/foo", Some(json!("new/foo"))),
            pkg("ok/lib", None),
        ];
        let lines = abandoned_warnings(&pkgs, &[], true);
        assert_eq!(
            lines,
            vec![
                "Package old/lib is abandoned, you should avoid using it. No replacement was suggested."
                    .to_string(),
                "Package old/foo is abandoned, you should avoid using it. Use new/foo instead."
                    .to_string(),
            ]
        );
    }

    #[test]
    fn skips_dev_when_no_dev() {
        let prod = vec![pkg("a/a", Some(Value::Bool(true)))];
        let dev = vec![pkg("b/b", Some(Value::Bool(true)))];
        let lines = abandoned_warnings(&prod, &dev, false);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("a/a"));
    }
}
