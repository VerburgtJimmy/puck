//! Offline security advisory matching against Packagist p2 metadata.

use crate::packagist::load_p2_metadata;
use crate::replay::ReplayError;
use crate::{Error, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;

/// One Packagist / FriendsofPHP advisory entry (`security-advisories`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SecurityAdvisory {
    #[serde(rename = "advisoryId")]
    pub advisory_id: String,
    #[serde(rename = "affectedVersions")]
    pub affected_versions: String,
}

/// A locked package version that matches an advisory's affected range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryHit {
    pub package: String,
    pub version: String,
    pub advisory_id: String,
    pub affected_versions: String,
}

#[derive(Debug, Deserialize)]
struct P2Document {
    #[serde(default, rename = "security-advisories")]
    security_advisories: Vec<SecurityAdvisory>,
    #[serde(default)]
    packages: serde_json::Map<String, serde_json::Value>,
}

/// Advisories listed for `package` in the VCR / offline registry (package-level
/// and any non-empty per-version `security-advisories` arrays), deduped by id.
pub fn advisories_for_package(
    registry_root: impl AsRef<Path>,
    package: &str,
) -> Result<Vec<SecurityAdvisory>> {
    let bytes = match load_p2_metadata(registry_root, package) {
        Ok(bytes) => bytes,
        Err(ReplayError::Miss(_)) => return Ok(Vec::new()),
        Err(err) => return Err(Error::Replay(err)),
    };
    parse_advisories_from_p2(&bytes)
}

/// Advisories that apply to `locked_version` for `package` (deduped by id).
pub fn find_advisory_hits(
    registry_root: impl AsRef<Path>,
    package: &str,
    locked_version: &str,
) -> Result<Vec<AdvisoryHit>> {
    let advisories = advisories_for_package(registry_root, package)?;
    let mut hits = Vec::new();
    let mut seen = BTreeSet::new();
    for adv in advisories {
        if !seen.insert(adv.advisory_id.clone()) {
            continue;
        }
        match puck_version::satisfies(locked_version, &adv.affected_versions) {
            Ok(true) => hits.push(AdvisoryHit {
                package: package.to_ascii_lowercase(),
                version: locked_version.to_owned(),
                advisory_id: adv.advisory_id,
                affected_versions: adv.affected_versions,
            }),
            Ok(false) => {}
            // Unparseable ranges / versions: skip rather than fail the whole audit.
            Err(_) => {}
        }
    }
    Ok(hits)
}

fn parse_advisories_from_p2(bytes: &[u8]) -> Result<Vec<SecurityAdvisory>> {
    let doc: P2Document = serde_json::from_slice(bytes)
        .map_err(|e| Error::Message(format!("invalid p2 json: {e}")))?;
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    for adv in doc.security_advisories {
        if seen.insert(adv.advisory_id.clone()) {
            out.push(adv);
        }
    }

    // Some metadata attaches advisories on version objects; collect those too.
    for (_name, versions) in doc.packages {
        let Some(arr) = versions.as_array() else {
            continue;
        };
        for version in arr {
            let Some(list) = version
                .get("security-advisories")
                .and_then(|v| v.as_array())
            else {
                continue;
            };
            for item in list {
                let Ok(adv) = serde_json::from_value::<SecurityAdvisory>(item.clone()) else {
                    continue;
                };
                if seen.insert(adv.advisory_id.clone()) {
                    out.push(adv);
                }
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_mini_p2(dir: &Path, package: &str, body: &str) {
        let p2 = dir.join("packagist/p2");
        fs::create_dir_all(&p2).expect("mkdir");
        let path = p2.join(crate::p2_filename(package));
        fs::write(path, body).expect("write p2");
    }

    #[test]
    fn finds_hit_from_package_level_advisories() {
        let dir = tempdir().expect("temp");
        write_mini_p2(
            dir.path(),
            "acme/vulnerable",
            r#"{
              "packages": {
                "acme/vulnerable": [
                  {"name":"acme/vulnerable","version":"1.0.0","version_normalized":"1.0.0.0"},
                  {"name":"acme/vulnerable","version":"1.2.3","version_normalized":"1.2.3.0"}
                ]
              },
              "security-advisories": [
                {"advisoryId":"PKSA-test-1","affectedVersions":">=1.0.0,<1.2.3"}
              ]
            }"#,
        );

        let hits = find_advisory_hits(dir.path(), "acme/vulnerable", "1.0.0").expect("hits");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].advisory_id, "PKSA-test-1");
        assert_eq!(hits[0].affected_versions, ">=1.0.0,<1.2.3");

        let miss = find_advisory_hits(dir.path(), "acme/vulnerable", "1.2.3").expect("miss");
        assert!(miss.is_empty());
    }

    #[test]
    fn finds_hit_from_version_object_advisories() {
        let dir = tempdir().expect("temp");
        write_mini_p2(
            dir.path(),
            "acme/ver",
            r#"{
              "packages": {
                "acme/ver": [
                  {
                    "name":"acme/ver",
                    "version":"2.0.0",
                    "security-advisories": [
                      {"advisoryId":"PKSA-ver-1","affectedVersions":"<2.1.0"}
                    ]
                  }
                ]
              },
              "security-advisories": []
            }"#,
        );

        let hits = find_advisory_hits(dir.path(), "acme/ver", "2.0.0").expect("hits");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].advisory_id, "PKSA-ver-1");
    }

    #[test]
    fn missing_p2_is_empty_not_error() {
        let dir = tempdir().expect("temp");
        fs::create_dir_all(dir.path().join("packagist/p2")).expect("mkdir");
        let hits = find_advisory_hits(dir.path(), "missing/pkg", "1.0.0").expect("ok");
        assert!(hits.is_empty());
    }

    #[test]
    fn fixture_registry_laravel_loads_advisories() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/registry");
        let advisories = advisories_for_package(&root, "laravel/framework").expect("adv");
        assert!(!advisories.is_empty());
        // Locked skeleton is typically outside current advisory ranges; just ensure matching runs.
        let _ = find_advisory_hits(&root, "laravel/framework", "v13.30.1").expect("match");
    }
}
