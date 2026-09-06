//! Project-level require/resolve helpers (Composer `require` subset).
//!
//! Partial-update approximation for the first slice: keep every locked package
//! **fixed** except packages listed in `unlock`, then solve against a
//! constraint-filtered VCR/p2 pool. Enough for requiring a package whose deps
//! are already satisfied (or platform-only).

use crate::request::UpdateAllowTransitive;
use crate::metadata::{find_p2_version_value, packages_from_lock_json};
use crate::package::Package;
use crate::path_repo::{load_path_packages, PathPackage};
use crate::platform::is_platform_package;
use crate::pool_builder::{ArrayRepository, PoolBuilder};
use crate::request::Request;
use crate::solver::Solver;
use crate::transaction::Operation;
use crate::vcr_pool::{array_repository_from_p2_constraints, P2Getter};
use crate::{Error, Result};
use indexmap::{IndexMap, IndexSet};
use puck_lock::{build_lock_document, LockWriteInput, PLUGIN_API_VERSION};
use puck_version::{parse_constraints, Stability};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

/// Expand a partial-update package list using lock `require` edges.
///
/// Mirrors Composer `-w` / `-W`:
/// - [`UpdateAllowTransitive::OnlyListed`]: return `listed` unchanged
/// - [`UpdateAllowTransitive::ListedWithTransitiveDepsNoRootRequire`]: unlock
///   transitive requires of listed packages, but stop at (and do not unlock)
///   root requirements
/// - [`UpdateAllowTransitive::ListedWithTransitiveDeps`]: unlock the full
///   transitive require closure, including root requirements
pub fn expand_update_unlock(
    lock_bytes: &[u8],
    root_require_names: &[String],
    listed: &[String],
    mode: UpdateAllowTransitive,
) -> Result<Vec<String>> {
    if matches!(mode, UpdateAllowTransitive::OnlyListed) || listed.is_empty() {
        return Ok(listed
            .iter()
            .map(|n| n.to_ascii_lowercase())
            .collect());
    }

    let lock: Value = serde_json::from_slice(lock_bytes)
        .map_err(|e| Error::Message(format!("invalid composer.lock: {e}")))?;

    let mut requires: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for key in ["packages", "packages-dev"] {
        let Some(arr) = lock.get(key).and_then(|v| v.as_array()) else {
            continue;
        };
        for pkg in arr {
            let Some(name) = pkg.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let name = name.to_ascii_lowercase();
            let mut deps = Vec::new();
            if let Some(map) = pkg.get("require").and_then(|v| v.as_object()) {
                for dep in map.keys() {
                    if is_platform_package(dep) {
                        continue;
                    }
                    deps.push(dep.to_ascii_lowercase());
                }
            }
            requires.insert(name, deps);
        }
    }

    let root: IndexSet<String> = root_require_names
        .iter()
        .map(|n| n.to_ascii_lowercase())
        .collect();
    let exclude_root =
        matches!(mode, UpdateAllowTransitive::ListedWithTransitiveDepsNoRootRequire);

    let mut unlock: IndexSet<String> = listed.iter().map(|n| n.to_ascii_lowercase()).collect();
    let mut queue: VecDeque<String> = unlock.iter().cloned().collect();

    while let Some(name) = queue.pop_front() {
        let Some(deps) = requires.get(&name) else {
            continue;
        };
        for dep in deps {
            if !requires.contains_key(dep) {
                continue;
            }
            if exclude_root && root.contains(dep) {
                continue;
            }
            if unlock.insert(dep.clone()) {
                queue.push_back(dep.clone());
            }
        }
    }

    Ok(unlock.into_iter().collect())
}

/// Resolve root requires against p2 metadata (+ path repositories) and build a lock document.
///
/// `load_p2` returns `Ok(None)` when metadata is missing (pool skips; lock dump errors).
/// `unlock` names are not fixed from the existing lock (they may change version
/// or be newly installed). All other locked packages are fixed.
///
/// `project_root` resolves path repository `url` values (relative to the project).
pub fn resolve_lock_document(
    composer_json: &str,
    lock_bytes: Option<&[u8]>,
    load_p2: &P2Getter<'_>,
    unlock: &[String],
    include_dev: bool,
    project_root: &Path,
) -> Result<Value> {
    let root: Value = serde_json::from_str(composer_json)
        .map_err(|e| Error::Message(format!("invalid composer.json: {e}")))?;

    let path_packages = load_path_packages(project_root, &root)?;
    let path_by_name: IndexMap<String, PathPackage> = path_packages
        .iter()
        .map(|p| (p.package.name.clone(), p.clone()))
        .collect();

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

    // Path packages default to `dev-main` (VersionGuesser fallback); allow them
    // under stable minimum-stability like Composer root requires of path pkgs.
    let mut stability_flags: IndexMap<String, Stability> = IndexMap::new();
    for name in path_by_name.keys() {
        stability_flags.insert(name.clone(), Stability::Dev);
    }

    let prod_requires = root_requires_from_json(&root, false);
    let all_requires = root_requires_from_json(&root, include_dev);

    let mut prod_repo = array_repository_from_p2_constraints(load_p2, &prod_requires, stability)?;
    merge_path_packages(&mut prod_repo, &path_packages);
    let prod_names = solve_names(
        &prod_repo,
        &prod_requires,
        &fixed_prod,
        stability,
        &stability_flags,
    )?;

    let mut all_repo = array_repository_from_p2_constraints(load_p2, &all_requires, stability)?;
    merge_path_packages(&mut all_repo, &path_packages);
    let installed = solve_packages(
        &all_repo,
        &all_requires,
        &fixed_all,
        stability,
        &stability_flags,
    )?;

    let mut packages = Vec::new();
    let mut packages_dev = Vec::new();
    for (name, pretty) in &installed {
        let raw = if let Some(path_pkg) = path_by_name.get(name) {
            path_pkg.to_lock_value()
        } else {
            let bytes = load_p2(name)
                .map_err(|e| Error::Message(format!("p2 for {name}: {e}")))?
                .ok_or_else(|| Error::Message(format!("missing p2 metadata for {name}")))?;
            find_p2_version_value(&bytes, pretty)?.ok_or_else(|| {
                Error::Message(format!("p2 for {name} has no version {pretty}"))
            })?
        };
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

fn merge_path_packages(repo: &mut ArrayRepository, path_packages: &[PathPackage]) {
    for path_pkg in path_packages {
        repo.add_package(path_pkg.package.clone());
    }
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
    minimum_stability: Stability,
    stability_flags: &IndexMap<String, Stability>,
) -> Result<BTreeSet<String>> {
    Ok(solve_packages(repo, requires, fixed, minimum_stability, stability_flags)?
        .into_iter()
        .map(|(n, _)| n)
        .collect())
}

fn solve_packages(
    repo: &ArrayRepository,
    requires: &[(String, String)],
    fixed: &[Package],
    minimum_stability: Stability,
    stability_flags: &IndexMap<String, Stability>,
) -> Result<Vec<(String, String)>> {
    let mut request = Request::new();
    for (name, constraint) in requires {
        request
            .require_name(name.clone(), Some(parse_constraints(constraint)?))
            .map_err(Error::Message)?;
    }
    let (mut pool, present) = PoolBuilder::build_with_stability(
        &[repo],
        &[],
        fixed,
        &mut request,
        minimum_stability,
        stability_flags,
    )?;
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
            Operation::MarkAliasInstalled { .. } | Operation::MarkAliasUninstalled { .. } => {}
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
    use crate::vcr_pool::p2_dir_getter;
    use std::path::PathBuf;

    fn skeleton() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-skeleton")
    }

    fn p2_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/registry/packagist/p2")
    }

    fn fixture_p2() -> impl Fn(&str) -> std::result::Result<Option<Vec<u8>>, String> {
        p2_dir_getter(p2_dir())
    }



    #[test]
    fn require_webmozart_assert_dev_keeps_prod_lock_pins() {
        let get = fixture_p2();
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
            &get,
            &["webmozart/assert".into()],
            true,
            &dir,
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

    #[test]
    fn remove_laravel_pail_drops_dev_package_keeps_prod() {
        let get = fixture_p2();
        let dir = skeleton();
        let mut root: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("composer.json")).unwrap(),
        )
        .unwrap();
        assert!(root["require-dev"]
            .as_object_mut()
            .unwrap()
            .shift_remove("laravel/pail")
            .is_some());

        let composer = serde_json::to_string_pretty(&root).unwrap();
        let lock_bytes = std::fs::read(dir.join("composer.lock")).unwrap();
        let expected: Value = serde_json::from_slice(&lock_bytes).unwrap();
        let before_dev = expected["packages-dev"].as_array().unwrap().len();

        let doc = resolve_lock_document(
            &composer,
            Some(&lock_bytes),
            &get,
            &["laravel/pail".into()],
            true,
            &dir,
        )
        .expect("resolve");

        let packages = doc["packages"].as_array().unwrap();
        let packages_dev = doc["packages-dev"].as_array().unwrap();
        assert_eq!(
            packages.len(),
            expected["packages"].as_array().unwrap().len()
        );
        assert_eq!(packages_dev.len(), before_dev - 1);
        assert!(!packages_dev.iter().any(|p| p["name"] == "laravel/pail"));
        assert!(packages.iter().any(|p| p["name"] == "laravel/framework"));
    }

    #[test]
    fn expand_with_dependencies_skips_root_requires() {
        let lock = br#"{
            "packages": [
                {
                    "name": "a/a",
                    "version": "1.0.0",
                    "require": { "b/b": "^1", "root/dep": "^1", "php": ">=8" }
                },
                {
                    "name": "b/b",
                    "version": "1.0.0",
                    "require": { "c/c": "^1" }
                },
                { "name": "c/c", "version": "1.0.0" },
                { "name": "root/dep", "version": "1.0.0" }
            ],
            "packages-dev": []
        }"#;
        let roots = vec!["root/dep".into()];
        let listed = vec!["a/a".into()];
        let w = expand_update_unlock(
            lock,
            &roots,
            &listed,
            UpdateAllowTransitive::ListedWithTransitiveDepsNoRootRequire,
        )
        .unwrap();
        let set: BTreeSet<_> = w.into_iter().collect();
        assert!(set.contains("a/a"));
        assert!(set.contains("b/b"));
        assert!(set.contains("c/c"));
        assert!(!set.contains("root/dep"));

        let all = expand_update_unlock(
            lock,
            &roots,
            &listed,
            UpdateAllowTransitive::ListedWithTransitiveDeps,
        )
        .unwrap();
        let set: BTreeSet<_> = all.into_iter().collect();
        assert!(set.contains("root/dep"));
        assert!(set.contains("c/c"));
    }

    #[test]
    fn resolve_path_local_selects_path_dist() {
        let get = fixture_p2();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/path-local");
        let composer = std::fs::read_to_string(root.join("composer.json")).unwrap();
        // Empty unlock + no prior lock: full resolve from path repo only.
        let doc = resolve_lock_document(&composer, None, &get, &[], true, &root)
            .expect("resolve path-local");
        let packages = doc["packages"].as_array().unwrap();
        assert_eq!(packages.len(), 1);
        let pkg = &packages[0];
        assert_eq!(pkg["name"], "acme/hello");
        assert_eq!(pkg["version"], "dev-main");
        assert_eq!(pkg["dist"]["type"], "path");
        assert_eq!(pkg["dist"]["url"], "packages/acme-hello");
        assert_eq!(pkg["transport-options"]["symlink"], true);
        assert!(pkg.get("notification-url").is_none());
    }

    #[test]
    fn resolve_path_local_rewrites_from_existing_lock() {
        let get = fixture_p2();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/path-local");
        let composer = std::fs::read_to_string(root.join("composer.json")).unwrap();
        let lock_bytes = std::fs::read(root.join("composer.lock")).unwrap();
        let doc = resolve_lock_document(
            &composer,
            Some(&lock_bytes),
            &get,
            &["acme/hello".into()],
            true,
            &root,
        )
        .expect("resolve path-local with unlock");
        let pkg = doc["packages"].as_array().unwrap().iter().find(|p| p["name"] == "acme/hello").unwrap();
        assert_eq!(pkg["dist"]["type"], "path");
        assert_eq!(pkg["dist"]["url"], "packages/acme-hello");
    }
}
