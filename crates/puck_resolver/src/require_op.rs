//! Project-level require/resolve helpers (Composer `require` subset).
//!
//! Partial-update approximation for the first slice: keep every locked package
//! **fixed** except packages listed in `unlock`, then solve against a
//! constraint-filtered VCR/p2 pool. Enough for requiring a package whose deps
//! are already satisfied (or platform-only).

use crate::metadata::{find_p2_version_value, packages_from_lock_json};
use crate::package::Package;
use crate::platform::is_platform_package;
use crate::pool_builder::{ArrayRepository, PoolBuilder};
use crate::request::Request;
use crate::solver::Solver;
use crate::transaction::Operation;
use crate::vcr_pool::array_repository_from_p2_constraints;
use crate::{Error, Result};
use indexmap::{IndexMap, IndexSet};
use puck_lock::{build_lock_document, LockWriteInput, PLUGIN_API_VERSION};
use puck_version::{parse_constraints, Stability};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

/// Resolve root requires against `p2_dir` and build a lock document.
///
/// `unlock` names are not fixed from the existing lock (they may change version
/// or be newly installed). All other locked packages are fixed.
pub fn resolve_lock_document(
    composer_json: &str,
    lock_bytes: Option<&[u8]>,
    p2_dir: &Path,
    unlock: &[String],
    include_dev: bool,
) -> Result<Value> {
    let root: Value = serde_json::from_str(composer_json)
        .map_err(|e| Error::Message(format!("invalid composer.json: {e}")))?;

    let unlock_set: IndexSet<String> = unlock.iter().map(|n| n.to_ascii_lowercase()).collect();

    let (locked_prod, locked_dev) = match lock_bytes {
        Some(bytes) => {
            let prod = packages_from_lock_json(bytes, false)?;
            let all = packages_from_lock_json(bytes, true)?;
            let prod_names: BTreeSet<String> = prod.iter().map(|p| p.name.clone()).collect();
            let dev: Vec<Package> = all
                .into_iter()
                .filter(|p| !prod_names.contains(&p.name))
                .collect();
            (prod, dev)
        }
        None => (Vec::new(), Vec::new()),
    };

    let fixed_prod: Vec<Package> = locked_prod
        .iter()
        .filter(|p| !unlock_set.contains(&p.name))
        .cloned()
        .collect();
    let fixed_all: Vec<Package> = locked_prod
        .iter()
        .chain(locked_dev.iter())
        .filter(|p| !unlock_set.contains(&p.name))
        .cloned()
        .collect();

    let stability = match root.get("minimum-stability").and_then(|v| v.as_str()) {
        Some("RC") | Some("rc") => Stability::Rc,
        Some("beta") => Stability::Beta,
        Some("alpha") => Stability::Alpha,
        Some("dev") => Stability::Dev,
        _ => Stability::Stable,
    };

    let prod_requires = root_requires_from_json(&root, false);
    let all_requires = root_requires_from_json(&root, include_dev);

    let prod_repo = array_repository_from_p2_constraints(p2_dir, &prod_requires, stability)?;
    let prod_names = solve_names(&prod_repo, &prod_requires, &fixed_prod)?;

    let all_repo = array_repository_from_p2_constraints(p2_dir, &all_requires, stability)?;
    let installed = solve_packages(&all_repo, &all_requires, &fixed_all)?;

    let mut packages = Vec::new();
    let mut packages_dev = Vec::new();
    for (name, pretty) in &installed {
        let path = p2_dir.join(format!("{}.json", name.replace('/', "$")));
        let bytes = std::fs::read(&path).map_err(|e| {
            Error::Message(format!("missing p2 for {name} at {}: {e}", path.display()))
        })?;
        let raw = find_p2_version_value(&bytes, pretty)?.ok_or_else(|| {
            Error::Message(format!("p2 for {name} has no version {pretty}"))
        })?;
        if prod_names.contains(name) {
            packages.push(raw);
        } else {
            packages_dev.push(raw);
        }
    }

    build_lock_document(
        composer_json,
        LockWriteInput {
            packages,
            packages_dev: Some(packages_dev),
            platform: platform_reqs_from_root(&root, false),
            platform_dev: platform_reqs_from_root(&root, true),
            aliases: Vec::new(),
            minimum_stability: root
                .get("minimum-stability")
                .and_then(|v| v.as_str())
                .unwrap_or("stable")
                .to_string(),
            stability_flags: Map::new(),
            prefer_stable: root
                .get("prefer-stable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            prefer_lowest: root
                .get("prefer-lowest")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            plugin_api_version: PLUGIN_API_VERSION.into(),
        },
    )
    .map_err(|e| Error::Message(e.to_string()))
}

fn root_requires_from_json(root: &Value, include_dev: bool) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for key in if include_dev {
        ["require", "require-dev"].as_slice()
    } else {
        ["require"].as_slice()
    } {
        let Some(map) = root.get(*key).and_then(|v| v.as_object()) else {
            continue;
        };
        for (name, c) in map {
            let Some(c) = c.as_str() else {
                continue;
            };
            if is_platform_package(name) {
                continue;
            }
            out.push((name.to_ascii_lowercase(), c.to_string()));
        }
    }
    out
}

fn platform_reqs_from_root(root: &Value, dev: bool) -> Map<String, Value> {
    let key = if dev { "require-dev" } else { "require" };
    let mut out = Map::new();
    let Some(map) = root.get(key).and_then(|v| v.as_object()) else {
        return out;
    };
    for (name, constraint) in map {
        if is_platform_package(name) {
            out.insert(name.clone(), constraint.clone());
        }
    }
    out
}

fn solve_names(
    repo: &ArrayRepository,
    requires: &[(String, String)],
    fixed: &[Package],
) -> Result<BTreeSet<String>> {
    Ok(solve_packages(repo, requires, fixed)?
        .into_iter()
        .map(|(n, _)| n)
        .collect())
}

fn solve_packages(
    repo: &ArrayRepository,
    requires: &[(String, String)],
    fixed: &[Package],
) -> Result<Vec<(String, String)>> {
    let mut request = Request::new();
    for (name, constraint) in requires {
        request
            .require_name(name.clone(), Some(parse_constraints(constraint)?))
            .map_err(Error::Message)?;
    }
    let (mut pool, present) = PoolBuilder::build(&[repo], &[], fixed, &mut request)?;
    let tx = Solver::new(&mut pool).solve(&request, &present)?;

    let mut removed = BTreeSet::new();
    let mut changed: IndexMap<String, String> = IndexMap::new();
    for op in tx.operations() {
        match op {
            Operation::Install { package_id } | Operation::Update { to: package_id, .. } => {
                let p = pool.package_by_id(*package_id);
                changed.insert(p.name.clone(), p.pretty_version.clone());
            }
            Operation::Remove { package_id } => {
                removed.insert(pool.package_by_id(*package_id).name.clone());
            }
        }
    }

    let mut full = Vec::new();
    for package in fixed {
        if removed.contains(&package.name) {
            continue;
        }
        if let Some((_, pretty)) = changed.shift_remove_entry(&package.name) {
            full.push((package.name.clone(), pretty));
        } else {
            full.push((package.name.clone(), package.pretty_version.clone()));
        }
    }
    for (name, pretty) in changed {
        full.push((name, pretty));
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn skeleton() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-skeleton")
    }

    fn p2_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/registry/packagist/p2")
    }

    #[test]
    fn require_webmozart_assert_dev_keeps_prod_lock_pins() {
        let dir = skeleton();
        let mut root: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("composer.json")).unwrap(),
        )
        .unwrap();
        root["require-dev"].as_object_mut().unwrap().insert(
            "webmozart/assert".into(),
            Value::String("^2.0".into()),
        );
        let map = root["require-dev"].as_object_mut().unwrap();
        let old = std::mem::take(map);
        let mut entries: Vec<_> = old.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (k, v) in entries {
            map.insert(k, v);
        }

        let composer = serde_json::to_string_pretty(&root).unwrap();
        let lock_bytes = std::fs::read(dir.join("composer.lock")).unwrap();
        let doc = resolve_lock_document(
            &composer,
            Some(&lock_bytes),
            &p2_dir(),
            &["webmozart/assert".into()],
            true,
        )
        .expect("resolve");

        let packages = doc["packages"].as_array().unwrap();
        let packages_dev = doc["packages-dev"].as_array().unwrap();
        assert!(packages.iter().any(|p| p["name"] == "laravel/framework"));
        assert!(packages_dev
            .iter()
            .any(|p| p["name"] == "webmozart/assert"));
        assert!(!packages.iter().any(|p| p["name"] == "webmozart/assert"));

        let expected: Value = serde_json::from_slice(&lock_bytes).unwrap();
        assert_eq!(
            packages.len(),
            expected["packages"].as_array().unwrap().len()
        );
        let fw = packages
            .iter()
            .find(|p| p["name"] == "laravel/framework")
            .unwrap();
        let want = expected["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "laravel/framework")
            .unwrap();
        assert_eq!(fw["version"], want["version"]);
        assert_eq!(
            fw.pointer("/dist/reference"),
            want.pointer("/dist/reference")
        );
    }
}
