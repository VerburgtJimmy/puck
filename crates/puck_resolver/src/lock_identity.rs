//! Lock-identity gates: solve against recorded lock/VCR metadata.
//!
//! M3 exit gate is identical locks vs Composer. This module checks that the
//! solver’s install set for root requires matches `composer.lock` packages
//! (name + pretty version) for lock-dump, VCR pin, and constraint-filtered pools.

use crate::metadata::{
    find_p2_version_value, packages_from_lock_json, packages_from_p2_lock_pins,
};
use crate::package::Package;
use crate::platform::is_platform_package;
use crate::pool_builder::{ArrayRepository, PoolBuilder};
use crate::request::Request;
use crate::solver::Solver;
use crate::transaction::Operation;
use crate::vcr_pool::array_repository_from_p2_constraints;
use puck_lock::{build_lock_document, LockWriteInput, PLUGIN_API_VERSION};
use puck_version::{parse_constraints, Stability};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn skeleton_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-skeleton")
}

fn app_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-app")
}

fn p2_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/registry/packagist/p2")
}

fn load_root_requires(composer_json: &[u8], include_dev: bool) -> Vec<(String, String)> {
    let data: Value = serde_json::from_slice(composer_json).expect("composer.json");
    let mut out = Vec::new();
    for key in if include_dev {
        ["require", "require-dev"].as_slice()
    } else {
        ["require"].as_slice()
    } {
        let Some(map) = data.get(*key).and_then(|v| v.as_object()) else {
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

#[test]
fn laravel_skeleton_no_dev_solve_matches_lock_packages() {
    let dir = skeleton_dir();
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_lock_json(&lock_bytes, false).expect("parse lock packages");
    assert_solve_matches_lock(&dir, locked, false, "lock dump");
}

#[test]
fn laravel_app_no_dev_solve_matches_lock_packages() {
    let dir = app_dir();
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_lock_json(&lock_bytes, false).expect("parse lock packages");
    assert_solve_matches_lock(&dir, locked, false, "lock dump");
}

#[test]
fn laravel_skeleton_with_dev_solve_matches_lock_packages() {
    let dir = skeleton_dir();
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_lock_json(&lock_bytes, true).expect("parse lock packages");
    assert_solve_matches_lock(&dir, locked, true, "lock dump");
}

#[test]
fn laravel_app_with_dev_solve_matches_lock_packages() {
    let dir = app_dir();
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_lock_json(&lock_bytes, true).expect("parse lock packages");
    assert_solve_matches_lock(&dir, locked, true, "lock dump");
}

#[test]
fn laravel_skeleton_no_dev_solve_matches_vcr_p2_pins() {
    let dir = skeleton_dir();
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_p2_lock_pins(&p2_dir(), &lock_bytes, false).expect("vcr p2 pins");
    assert_solve_matches_lock(&dir, locked, false, "vcr p2 pins");
}

#[test]
fn laravel_app_no_dev_solve_matches_vcr_p2_pins() {
    let dir = app_dir();
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_p2_lock_pins(&p2_dir(), &lock_bytes, false).expect("vcr p2 pins");
    assert_solve_matches_lock(&dir, locked, false, "vcr p2 pins");
}

#[test]
fn laravel_skeleton_with_dev_solve_matches_vcr_p2_pins() {
    let dir = skeleton_dir();
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_p2_lock_pins(&p2_dir(), &lock_bytes, true).expect("vcr p2 pins");
    assert_solve_matches_lock(&dir, locked, true, "vcr p2 pins");
}

#[test]
fn laravel_skeleton_no_dev_solve_matches_constraint_filtered_vcr() {
    let dir = skeleton_dir();
    let json_bytes = fs::read(dir.join("composer.json")).expect("composer.json");
    let requires = load_root_requires(&json_bytes, false);
    let repo = array_repository_from_p2_constraints(&p2_dir(), &requires, Stability::Stable)
        .expect("constraint-filtered vcr pool");
    assert_solve_matches_lock(&dir, repo.packages().to_vec(), false, "constraint-filtered vcr");
}

#[test]
fn laravel_app_no_dev_solve_matches_constraint_filtered_vcr() {
    let dir = app_dir();
    let json_bytes = fs::read(dir.join("composer.json")).expect("composer.json");
    let requires = load_root_requires(&json_bytes, false);
    let repo = array_repository_from_p2_constraints(&p2_dir(), &requires, Stability::Stable)
        .expect("constraint-filtered vcr pool");
    assert_solve_matches_lock(&dir, repo.packages().to_vec(), false, "constraint-filtered vcr");
}

#[test]
fn laravel_skeleton_with_dev_solve_matches_constraint_filtered_vcr() {
    let dir = skeleton_dir();
    let json_bytes = fs::read(dir.join("composer.json")).expect("composer.json");
    let requires = load_root_requires(&json_bytes, true);
    let repo = array_repository_from_p2_constraints(&p2_dir(), &requires, Stability::Stable)
        .expect("constraint-filtered vcr pool");
    assert_solve_matches_lock(&dir, repo.packages().to_vec(), true, "constraint-filtered vcr");
}

#[test]
fn laravel_app_with_dev_solve_matches_constraint_filtered_vcr() {
    let dir = app_dir();
    let json_bytes = fs::read(dir.join("composer.json")).expect("composer.json");
    let requires = load_root_requires(&json_bytes, true);
    let repo = array_repository_from_p2_constraints(&p2_dir(), &requires, Stability::Stable)
        .expect("constraint-filtered vcr pool");
    assert_solve_matches_lock(&dir, repo.packages().to_vec(), true, "constraint-filtered vcr");
}

#[test]
fn laravel_skeleton_no_dev_lock_packages_match_install_critical_fields() {
    assert_written_lock_matches_fixture(&skeleton_dir(), false);
}

#[test]
fn laravel_skeleton_with_dev_lock_document_matches_m3_lock_gate() {
    assert_written_lock_matches_fixture(&skeleton_dir(), true);
}

#[test]
fn laravel_app_with_dev_lock_document_matches_m3_lock_gate() {
    assert_written_lock_matches_fixture(&app_dir(), true);
}

fn assert_written_lock_matches_fixture(dir: &PathBuf, include_dev: bool) {
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let json_text = fs::read_to_string(dir.join("composer.json")).expect("composer.json");
    let json_bytes = json_text.as_bytes();

    let prod_requires = load_root_requires(json_bytes, false);
    let prod_repo =
        array_repository_from_p2_constraints(&p2_dir(), &prod_requires, Stability::Stable)
            .expect("prod vcr pool");
    let prod_names = solve_install_names(&prod_repo, &prod_requires);
    let prod_name_set: BTreeSet<String> = prod_names.into_iter().map(|(n, _)| n).collect();

    let all_requires = load_root_requires(json_bytes, include_dev);
    let all_repo = array_repository_from_p2_constraints(&p2_dir(), &all_requires, Stability::Stable)
        .expect("full vcr pool");
    let installed = solve_install_packages(&all_repo, &all_requires);

    let mut packages = Vec::new();
    let mut packages_dev = Vec::new();
    for (name, pretty) in &installed {
        let path = p2_dir().join(format!("{}.json", name.replace('/', "$")));
        let bytes = fs::read(&path).expect("p2");
        let raw = find_p2_version_value(&bytes, pretty)
            .unwrap()
            .unwrap_or_else(|| panic!("missing p2 body for {name} {pretty}"));
        if prod_name_set.contains(name) {
            packages.push(raw);
        } else if include_dev {
            packages_dev.push(raw);
        } else {
            panic!("unexpected non-prod package under --no-dev: {name} {pretty}");
        }
    }

    let root: Value = serde_json::from_str(&json_text).unwrap();
    let written = build_lock_document(
        &json_text,
        LockWriteInput {
            packages,
            packages_dev: if include_dev {
                Some(packages_dev)
            } else {
                Some(Vec::new())
            },
            platform: platform_reqs_from_root(&root, false),
            platform_dev: platform_reqs_from_root(&root, true),
            aliases: Vec::new(),
            minimum_stability: root
                .get("minimum-stability")
                .and_then(|v| v.as_str())
                .unwrap_or("stable")
                .to_string(),
            stability_flags: serde_json::Map::new(),
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
    .expect("build lock");

    let expected: Value = serde_json::from_slice(&lock_bytes).unwrap();
    assert_eq!(written["content-hash"], expected["content-hash"]);
    assert_eq!(written["minimum-stability"], expected["minimum-stability"]);
    assert_eq!(written["prefer-stable"], expected["prefer-stable"]);
    assert_eq!(written["prefer-lowest"], expected["prefer-lowest"]);
    assert_eq!(written["plugin-api-version"], expected["plugin-api-version"]);
    assert_eq!(written["platform"], expected["platform"]);
    assert_eq!(written["platform-dev"], expected["platform-dev"]);
    assert_eq!(written["aliases"], expected["aliases"]);

    assert_critical_package_fields(
        written["packages"].as_array().unwrap(),
        expected["packages"].as_array().unwrap(),
        "packages",
    );
    if include_dev {
        assert_critical_package_fields(
            written["packages-dev"].as_array().unwrap(),
            expected["packages-dev"].as_array().unwrap(),
            "packages-dev",
        );
    }
}

fn platform_reqs_from_root(root: &Value, dev: bool) -> serde_json::Map<String, Value> {
    let key = if dev { "require-dev" } else { "require" };
    let mut out = serde_json::Map::new();
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

fn solve_install_names(
    repo: &ArrayRepository,
    requires: &[(String, String)],
) -> BTreeSet<(String, String)> {
    solve_install_packages(repo, requires).into_iter().collect()
}

fn solve_install_packages(
    repo: &ArrayRepository,
    requires: &[(String, String)],
) -> Vec<(String, String)> {
    let mut request = Request::new();
    for (name, constraint) in requires {
        request
            .require_name(name.clone(), Some(parse_constraints(constraint).unwrap()))
            .unwrap();
    }
    let (mut pool, present) = PoolBuilder::build(&[repo], &[], &[], &mut request).unwrap();
    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .expect("solve");
    let mut out = Vec::new();
    for op in tx.operations() {
        let package_id = match op {
            Operation::Install { package_id } | Operation::Update { to: package_id, .. } => {
                *package_id
            }
            Operation::Remove { .. }
            | Operation::MarkAliasInstalled { .. }
            | Operation::MarkAliasUninstalled { .. } => continue,
        };
        let p = pool.package_by_id(package_id);
        out.push((p.name.clone(), p.pretty_version.clone()));
    }
    out
}

fn assert_critical_package_fields(got: &[Value], want: &[Value], label: &str) {
    assert_eq!(got.len(), want.len(), "{label} length");
    for (g, w) in got.iter().zip(want.iter()) {
        assert_eq!(g.get("name"), w.get("name"), "{label} name");
        assert_eq!(g.get("version"), w.get("version"), "{label} version");
        assert_eq!(
            g.pointer("/dist/reference"),
            w.pointer("/dist/reference"),
            "{label} dist.reference for {:?}",
            w.get("name")
        );
        assert_eq!(
            g.pointer("/source/reference"),
            w.pointer("/source/reference"),
            "{label} source.reference for {:?}",
            w.get("name")
        );
        assert_eq!(g.get("time"), w.get("time"), "{label} time for {:?}", w.get("name"));
    }
}

fn assert_solve_matches_lock(
    dir: &PathBuf,
    pool_packages: Vec<Package>,
    include_dev: bool,
    source: &str,
) {
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let json_bytes = fs::read(dir.join("composer.json")).expect("composer.json");
    let mode = if include_dev { "with-dev" } else { "--no-dev" };

    let expected: BTreeSet<(String, String)> = packages_from_lock_json(&lock_bytes, include_dev)
        .expect("parse lock packages")
        .into_iter()
        .map(|p| (p.name, p.pretty_version))
        .collect();

    let mut repo = ArrayRepository::new();
    for package in pool_packages {
        repo.add_package(package);
    }

    let mut request = Request::new();
    for (name, constraint) in load_root_requires(&json_bytes, include_dev) {
        request
            .require_name(name, Some(parse_constraints(&constraint).unwrap()))
            .unwrap();
    }

    let (mut pool, present) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();
    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap_or_else(|e| panic!("solve {} {mode} ({source}): {e}", dir.display()));

    let mut got: BTreeSet<(String, String)> = BTreeSet::new();
    for op in tx.operations() {
        match op {
            Operation::Install { package_id } | Operation::Update { to: package_id, .. } => {
                let p = pool.package_by_id(*package_id);
                got.insert((p.name.clone(), p.pretty_version.clone()));
            }
            Operation::Remove { .. }
            | Operation::MarkAliasInstalled { .. }
            | Operation::MarkAliasUninstalled { .. } => {}
        }
    }

    assert_eq!(
        got, expected,
        "solver install set must match lock packages ({mode}, {source}) for {}\nonly in lock: {:?}\nonly in solve: {:?}",
        dir.display(),
        expected.difference(&got).collect::<Vec<_>>(),
        got.difference(&expected).collect::<Vec<_>>()
    );
}
