//! Load packages from Packagist Composer 2 (`/p2`) metadata JSON.

use crate::link::Link;
use crate::package::Package;
use crate::Error;
use crate::Result;
use indexmap::IndexMap;
use puck_version::parse_constraints;
use serde_json::Value;

/// Parse all versions of a package from a Packagist p2 response body.
pub fn packages_from_p2_json(bytes: &[u8]) -> Result<Vec<Package>> {
    let data: Value = serde_json::from_slice(bytes)
        .map_err(|e| Error::Message(format!("invalid p2 json: {e}")))?;
    let packages = data
        .get("packages")
        .and_then(|p| p.as_object())
        .ok_or_else(|| Error::Message("p2 json missing packages object".into()))?;

    let mut out = Vec::new();
    for (_name, versions) in packages {
        let Some(versions) = versions.as_array() else {
            continue;
        };
        for version in versions {
            if let Some(package) = package_from_p2_version(version)? {
                out.push(package);
            }
        }
    }
    Ok(out)
}

/// Parse a single p2 version object into a [`Package`].
pub fn package_from_p2_version(version: &Value) -> Result<Option<Package>> {
    let name = match version.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_ascii_lowercase(),
        None => return Ok(None),
    };
    let pretty = version
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let normalized = version
        .get("version_normalized")
        .and_then(|v| v.as_str())
        .unwrap_or(&pretty)
        .to_string();
    if pretty.is_empty() || normalized.is_empty() {
        return Ok(None);
    }

    let mut package = Package::new(name, normalized.clone(), pretty);

    if let Some(map) = version.get("require").and_then(|v| v.as_object()) {
        package.requires = parse_link_map(&package.name, map, &normalized)?;
    }
    if let Some(map) = version.get("conflict").and_then(|v| v.as_object()) {
        package.conflicts = parse_link_map(&package.name, map, &normalized)?;
    }
    if let Some(map) = version.get("provide").and_then(|v| v.as_object()) {
        package.provides = parse_link_map(&package.name, map, &normalized)?;
    }
    if let Some(map) = version.get("replace").and_then(|v| v.as_object()) {
        package.replaces = parse_link_map(&package.name, map, &normalized)?;
    }

    Ok(Some(package))
}

fn parse_link_map(
    source: &str,
    map: &serde_json::Map<String, Value>,
    self_version: &str,
) -> Result<IndexMap<String, Link>> {
    let mut out = IndexMap::new();
    for (target, constraint_v) in map {
        let pretty = constraint_v
            .as_str()
            .ok_or_else(|| Error::Message(format!("link constraint for {target} is not a string")))?;
        // ArrayLoader::createLink: self.version -> package version
        let expanded = if pretty == "self.version" {
            self_version
        } else {
            pretty
        };
        let constraint = parse_constraints(expanded)?;
        let target_l = target.to_ascii_lowercase();
        out.insert(
            target_l.clone(),
            Link::new(source, target_l, expanded, constraint),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn framework_p2() -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/registry/packagist/p2/laravel$framework.json");
        fs::read(path).expect("framework p2")
    }

    #[test]
    fn loads_framework_replace_self_version() {
        let packages = packages_from_p2_json(&framework_p2()).unwrap();
        let fw = packages
            .iter()
            .find(|p| p.pretty_version == "v13.30.1")
            .expect("v13.30.1");
        assert_eq!(fw.replaces.len(), 38);
        let support = fw.replaces.get("illuminate/support").unwrap();
        assert_eq!(support.pretty_constraint, "13.30.1.0");
    }
}
