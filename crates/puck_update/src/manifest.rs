//! Release `manifest.json` types.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    pub artifacts: BTreeMap<String, Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|e| Error::Manifest(e.to_string()))
    }

    pub fn artifact_for(&self, target: &str) -> Result<&Artifact> {
        self.artifacts
            .get(target)
            .ok_or_else(|| Error::Manifest(format!("no artifact for target {target}")))
    }
}
