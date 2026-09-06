//! Project-level require/resolve helpers (Composer `require` subset).
//!
//! Partial-update approximation for the first slice: keep every locked package
//! **fixed** except packages listed in `unlock`, then solve against a
//! constraint-filtered VCR/p2 pool. Enough for requiring a package whose deps
//! are already satisfied (or platform-only).

use crate::metadata::{find_p2_version_value, packages_from_lock_json};
use crate::package::Package;
use crate::path_repo::{PathPackage, PathRepository};
use crate::platform::is_platform_package;
use crate::pool_builder::{ArrayRepository, PoolBuilder};
use crate::request::Request;
use crate::request::UpdateAllowTransitive;
use crate::solver::Solver;
use crate::transaction::Operation;
use crate::vcr_pool::{P2Getter, array_repository_from_p2_constraints};
use crate::{Error, Result};
use indexmap::{IndexMap, IndexSet};
use puck_lock::{LockWriteInput, PLUGIN_API_VERSION, build_lock_document};
use puck_version::{Stability, parse_constraints};
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
        return Ok(listed.iter().map(|n| n.to_ascii_lowercase()).collect());
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
    let exclude_root = matches!(
        mode,
        UpdateAllowTransitive::ListedWithTransitiveDepsNoRootRequire
    );

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

    let (prod_repos, _) =
        build_ordered_repositories(project_root, &root, load_p2, &prod_requires, stability)?;
    let (all_repos, path_by_name) =
        build_ordered_repositories(project_root, &root, load_p2, &all_requires, stability)?;

    // Path packages default to `dev-main` (VersionGuesser fallback); allow them
    // under stable minimum-stability like Composer root requires of path pkgs.
    let mut stability_flags: IndexMap<String, Stability> = IndexMap::new();
    for name in path_by_name.keys() {
        stability_flags.insert(name.clone(), Stability::Dev);
    }

    let prod_refs: Vec<&ArrayRepository> = prod_repos.iter().collect();
    let prod_names = solve_names(
        &prod_refs,
        &prod_requires,
        &fixed_prod,
        stability,
        &stability_flags,
    )?;

    let all_refs: Vec<&ArrayRepository> = all_repos.iter().collect();
    let installed = solve_packages(
        &all_refs,
        &all_requires,
        &fixed_all,
        stability,
        &stability_flags,
    )?;

    let mut packages = Vec::new();
    let mut packages_dev = Vec::new();
    for (name, pretty) in &installed {
        let raw = if let Some(path_pkg) = path_by_name.get(name) {
            // Path lock only when the selected version is the path package's version.
            // Otherwise Packagist (or another remote) won the pool.
            if path_pkg.package.pretty_version == *pretty {
                path_pkg.to_lock_value()
            } else {
                let bytes = load_p2(name)
                    .map_err(|e| Error::Message(format!("p2 for {name}: {e}")))?
                    .ok_or_else(|| Error::Message(format!("missing p2 metadata for {name}")))?;
                find_p2_version_value(&bytes, pretty)?.ok_or_else(|| {
                    Error::Message(format!("p2 for {name} has no version {pretty}"))
                })?
            }
        } else {
            let bytes = load_p2(name)
                .map_err(|e| Error::Message(format!("p2 for {name}: {e}")))?
                .ok_or_else(|| Error::Message(format!("missing p2 metadata for {name}")))?;
            find_p2_version_value(&bytes, pretty)?
                .ok_or_else(|| Error::Message(format!("p2 for {name} has no version {pretty}")))?
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

/// Build ArrayRepositories in Composer `repositories` order, honouring `canonical`.
///
/// Names claimed by an earlier canonical repository are omitted from later ones.
/// Packagist / `type: composer` loads via `load_p2` with `skip_names = claimed`.
/// Returns path packages that actually entered a path repo (for lock dump).
fn build_ordered_repositories(
    project_root: &Path,
    root: &Value,
    load_p2: &P2Getter<'_>,
    requires: &[(String, String)],
    stability: Stability,
) -> Result<(Vec<ArrayRepository>, IndexMap<String, PathPackage>)> {
    let plan = repository_pool_plan(project_root, root)?;
    let mut claimed: IndexSet<String> = IndexSet::new();
    let mut repos: Vec<ArrayRepository> = Vec::new();
    let mut path_by_name: IndexMap<String, PathPackage> = IndexMap::new();

    for entry in plan {
        match entry {
            PoolPlanEntry::Path(PathRepository {
                canonical,
                packages,
            }) => {
                let mut repo = ArrayRepository::new();
                let mut added = Vec::new();
                for path_pkg in packages {
                    if claimed.contains(&path_pkg.package.name) {
                        continue;
                    }
                    added.push(path_pkg.package.name.clone());
                    path_by_name.insert(path_pkg.package.name.clone(), path_pkg.clone());
                    repo.add_package(path_pkg.package.clone());
                }
                if !repo.packages().is_empty() {
                    repos.push(repo);
                }
                if canonical {
                    for name in added {
                        claimed.insert(name);
                    }
                }
            }
            PoolPlanEntry::Remote { canonical } => {
                let repo =
                    array_repository_from_p2_constraints(load_p2, requires, stability, &claimed)?;
                if canonical {
                    for package in repo.packages() {
                        claimed.insert(package.name.clone());
                    }
                }
                if !repo.packages().is_empty() {
                    repos.push(repo);
                }
            }
        }
    }

    Ok((repos, path_by_name))
}

#[derive(Debug)]
enum PoolPlanEntry {
    Path(PathRepository),
    Remote { canonical: bool },
}

/// Walk root `repositories` in order, then implicit Packagist last when enabled.
fn repository_pool_plan(project_root: &Path, root: &Value) -> Result<Vec<PoolPlanEntry>> {
    let mut plan = Vec::new();
    let mut packagist_enabled = true;
    let mut remote_placed = false;

    let Some(repos) = root.get("repositories") else {
        plan.push(PoolPlanEntry::Remote { canonical: true });
        return Ok(plan);
    };

    match repos {
        Value::Array(arr) => {
            for entry in arr {
                apply_pool_plan_entry(
                    project_root,
                    entry,
                    None,
                    &mut plan,
                    &mut packagist_enabled,
                    &mut remote_placed,
                )?;
            }
        }
        Value::Object(map) => {
            for (key, entry) in map {
                apply_pool_plan_entry(
                    project_root,
                    entry,
                    Some(key.as_str()),
                    &mut plan,
                    &mut packagist_enabled,
                    &mut remote_placed,
                )?;
            }
        }
        _ => {}
    }

    if packagist_enabled && !remote_placed {
        plan.push(PoolPlanEntry::Remote { canonical: true });
    }

    Ok(plan)
}

fn apply_pool_plan_entry(
    project_root: &Path,
    entry: &Value,
    object_key: Option<&str>,
    plan: &mut Vec<PoolPlanEntry>,
    packagist_enabled: &mut bool,
    remote_placed: &mut bool,
) -> Result<()> {
    if let Some(key) = object_key {
        if is_packagist_repo_name(key) && entry.as_bool() == Some(false) {
            *packagist_enabled = false;
            return Ok(());
        }
    }
    if is_packagist_disable_entry(entry) {
        *packagist_enabled = false;
        return Ok(());
    }
    let Some(obj) = entry.as_object() else {
        return Ok(());
    };
    let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let canonical = obj
        .get("canonical")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    match typ {
        "path" => {
            if let Some(repo) = load_one_path_repository_for_plan(project_root, entry)? {
                plan.push(PoolPlanEntry::Path(repo));
            }
        }
        "composer" => {
            let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if is_packagist_org_url(url) {
                // Redefining packagist.org disables the implicit default.
                *packagist_enabled = false;
            }
            plan.push(PoolPlanEntry::Remote { canonical });
            *remote_placed = true;
        }
        _ => {
            // VCS / package / artifact / etc. — not modeled in the pool yet.
        }
    }
    Ok(())
}

fn is_packagist_repo_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "packagist.org" || n == "packagist"
}

fn load_one_path_repository_for_plan(
    project_root: &Path,
    entry: &Value,
) -> Result<Option<PathRepository>> {
    // Re-load via public path loader API surface (single entry wrapped).
    let fake = serde_json::json!({ "repositories": [entry] });
    let mut repos = crate::path_repo::load_path_repositories(project_root, &fake)?;
    Ok(repos.pop())
}

fn is_packagist_disable_entry(entry: &Value) -> bool {
    let Some(obj) = entry.as_object() else {
        return false;
    };
    // Anonymous: { "packagist.org": false } or { "packagist": false }
    for key in ["packagist.org", "packagist"] {
        if let Some(v) = obj.get(key) {
            if obj.len() == 1 && v.as_bool() == Some(false) {
                return true;
            }
        }
    }
    false
}

fn is_packagist_org_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let trimmed = lower
        .trim_end_matches('/')
        .strip_prefix("https://")
        .or_else(|| lower.trim_end_matches('/').strip_prefix("http://"))
        .unwrap_or(lower.trim_end_matches('/'));
    let host = trimmed.split('/').next().unwrap_or("");
    host == "packagist.org" || host == "repo.packagist.org" || host.ends_with(".packagist.org")
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
    repos: &[&ArrayRepository],
    requires: &[(String, String)],
    fixed: &[Package],
    minimum_stability: Stability,
    stability_flags: &IndexMap<String, Stability>,
) -> Result<BTreeSet<String>> {
    Ok(
        solve_packages(repos, requires, fixed, minimum_stability, stability_flags)?
            .into_iter()
            .map(|(n, _)| n)
            .collect(),
    )
}

fn solve_packages(
    repos: &[&ArrayRepository],
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
        repos,
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
        let mut root: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("composer.json")).unwrap())
                .unwrap();
        root["require-dev"]
            .as_object_mut()
            .unwrap()
            .insert("webmozart/assert".into(), Value::String("^2.0".into()));
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
        assert!(packages_dev.iter().any(|p| p["name"] == "webmozart/assert"));
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
        let mut root: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("composer.json")).unwrap())
                .unwrap();
        assert!(
            root["require-dev"]
                .as_object_mut()
                .unwrap()
                .shift_remove("laravel/pail")
                .is_some()
        );

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
        let pkg = doc["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "acme/hello")
            .unwrap();
        assert_eq!(pkg["dist"]["type"], "path");
        assert_eq!(pkg["dist"]["url"], "packages/acme-hello");
    }

    fn temp_dual_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "puck-repo-order-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("local-pkg")).unwrap();
        dir
    }

    fn dual_p2_getter(
        version: &str,
        reference: &str,
    ) -> impl Fn(&str) -> std::result::Result<Option<Vec<u8>>, String> {
        let p2 = serde_json::json!({
            "packages": {
                "acme/dual": [{
                    "name": "acme/dual",
                    "version": version,
                    "version_normalized": format!("{}.0", version),
                    "dist": {
                        "type": "zip",
                        "url": format!("https://example.test/acme-dual-{version}.zip"),
                        "reference": reference
                    }
                }]
            }
        });
        move |name: &str| -> std::result::Result<Option<Vec<u8>>, String> {
            if name == "acme/dual" {
                Ok(Some(serde_json::to_vec(&p2).unwrap()))
            } else {
                Ok(None)
            }
        }
    }

    /// Path listed first (canonical default): same version → path lock (earlier repo / lower pool id).
    #[test]
    fn repo_order_canonical_path_first_same_version() {
        let dir = temp_dual_dir("same");
        std::fs::write(
            dir.join("local-pkg/composer.json"),
            r#"{"name":"acme/dual","version":"1.0.0","type":"library"}"#,
        )
        .unwrap();
        let composer = r#"{
            "require": { "acme/dual": "*" },
            "repositories": [{ "type": "path", "url": "local-pkg" }]
        }"#;
        std::fs::write(dir.join("composer.json"), composer).unwrap();
        let get = dual_p2_getter("1.0.0", "deadbeef");

        let doc = resolve_lock_document(composer, None, &get, &[], true, &dir)
            .expect("resolve dual-source same version");
        let pkg = doc["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "acme/dual")
            .unwrap();
        assert_eq!(pkg["version"], "1.0.0");
        assert_eq!(pkg["dist"]["type"], "path");
        assert_eq!(pkg["dist"]["url"], "local-pkg");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Path first, canonical (default): Packagist has higher semver but path still wins.
    #[test]
    fn repo_order_canonical_path_first_keeps_path_despite_higher_packagist() {
        let dir = temp_dual_dir("higher");
        // No version → VersionGuesser fallback `dev-main`.
        std::fs::write(
            dir.join("local-pkg/composer.json"),
            r#"{"name":"acme/dual","type":"library"}"#,
        )
        .unwrap();
        let composer = r#"{
            "minimum-stability": "dev",
            "require": { "acme/dual": "*" },
            "repositories": [{ "type": "path", "url": "local-pkg" }]
        }"#;
        std::fs::write(dir.join("composer.json"), composer).unwrap();
        let get = dual_p2_getter("2.0.0", "cafebabe");

        let doc = resolve_lock_document(composer, None, &get, &[], true, &dir)
            .expect("resolve path wins over higher Packagist");
        let pkg = doc["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "acme/dual")
            .unwrap();
        assert_eq!(pkg["version"], "dev-main");
        assert_eq!(pkg["dist"]["type"], "path");
        assert_eq!(pkg["dist"]["url"], "local-pkg");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Packagist redefined before path: name claimed by Packagist → path ignored; lock is zip.
    #[test]
    fn repo_order_packagist_before_path_ignores_path() {
        let dir = temp_dual_dir("packagist-first");
        std::fs::write(
            dir.join("local-pkg/composer.json"),
            r#"{"name":"acme/dual","version":"1.0.0","type":"library"}"#,
        )
        .unwrap();
        let composer = r#"{
            "require": { "acme/dual": "*" },
            "repositories": [
                { "type": "composer", "url": "https://repo.packagist.org" },
                { "type": "path", "url": "local-pkg" }
            ]
        }"#;
        std::fs::write(dir.join("composer.json"), composer).unwrap();
        let get = dual_p2_getter("1.0.0", "deadbeef");

        let doc = resolve_lock_document(composer, None, &get, &[], true, &dir)
            .expect("resolve packagist before path");
        let pkg = doc["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "acme/dual")
            .unwrap();
        assert_eq!(pkg["version"], "1.0.0");
        assert_eq!(pkg["dist"]["type"], "zip");
        assert_eq!(
            pkg["dist"]["url"],
            "https://example.test/acme-dual-1.0.0.zip"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Path with `"canonical": false`: both sources pool; highest version wins (Packagist 2.0.0).
    #[test]
    fn repo_order_path_non_canonical_highest_version_wins() {
        let dir = temp_dual_dir("non-canonical");
        std::fs::write(
            dir.join("local-pkg/composer.json"),
            r#"{"name":"acme/dual","version":"1.0.0","type":"library"}"#,
        )
        .unwrap();
        let composer = r#"{
            "require": { "acme/dual": "*" },
            "repositories": [
                { "type": "path", "url": "local-pkg", "canonical": false }
            ]
        }"#;
        std::fs::write(dir.join("composer.json"), composer).unwrap();
        let get = dual_p2_getter("2.0.0", "cafebabe");

        let doc = resolve_lock_document(composer, None, &get, &[], true, &dir)
            .expect("resolve non-canonical path");
        let pkg = doc["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "acme/dual")
            .unwrap();
        assert_eq!(pkg["version"], "2.0.0");
        assert_eq!(pkg["dist"]["type"], "zip");
        assert_eq!(
            pkg["dist"]["url"],
            "https://example.test/acme-dual-2.0.0.zip"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
