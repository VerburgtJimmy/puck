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
    /// Remaining package fields (`require`, `type`, `autoload`, …).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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
