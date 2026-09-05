//! Lock-identity gates: solve against recorded lock/VCR metadata.
//!
//! M3 exit gate is identical locks vs Composer. This module starts with
//! necessary checks: given a pin universe (from lock dump or VCR p2 pins),
//! the solver’s install set for root requires must match `composer.lock`
//! `packages` (name + pretty version).

use crate::metadata::{packages_from_lock_json, packages_from_p2_lock_pins};
use crate::package::Package;
use crate::platform::is_platform_package;
use crate::pool_builder::{ArrayRepository, PoolBuilder};
use crate::request::Request;
use crate::solver::Solver;
use crate::transaction::Operation;
use crate::vcr_pool::array_repository_from_p2_constraints;
use puck_version::{parse_constraints, Stability};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn skeleton_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-skeleton")
}

fn p2_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/registry/packagist/p2")
}

fn load_root_requires(composer_json: &[u8], dev: bool) -> Vec<(String, String)> {
    let data: Value = serde_json::from_slice(composer_json).expect("composer.json");
    let key = if dev { "require-dev" } else { "require" };
    let Some(map) = data.get(key).and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(name, c)| {
            let c = c.as_str()?;
            if is_platform_package(name) {
                return None;
            }
            Some((name.to_ascii_lowercase(), c.to_string()))
        })
        .collect()
}

#[test]
fn laravel_skeleton_no_dev_solve_matches_lock_packages() {
    let dir = skeleton_dir();
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_lock_json(&lock_bytes, false).expect("parse lock packages");
    assert_no_dev_solve_matches_lock(&dir, locked, "lock dump");
}

#[test]
fn laravel_app_no_dev_solve_matches_lock_packages() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-app");
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_lock_json(&lock_bytes, false).expect("parse lock packages");
    assert_no_dev_solve_matches_lock(&dir, locked, "lock dump");
}

#[test]
fn laravel_skeleton_no_dev_solve_matches_vcr_p2_pins() {
    let dir = skeleton_dir();
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_p2_lock_pins(&p2_dir(), &lock_bytes).expect("vcr p2 pins");
    assert_no_dev_solve_matches_lock(&dir, locked, "vcr p2 pins");
}

#[test]
fn laravel_app_no_dev_solve_matches_vcr_p2_pins() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-app");
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let locked = packages_from_p2_lock_pins(&p2_dir(), &lock_bytes).expect("vcr p2 pins");
    assert_no_dev_solve_matches_lock(&dir, locked, "vcr p2 pins");
}

#[test]
fn laravel_skeleton_no_dev_solve_matches_constraint_filtered_vcr() {
    let dir = skeleton_dir();
    let json_bytes = fs::read(dir.join("composer.json")).expect("composer.json");
    let requires = load_root_requires(&json_bytes, false);
    let repo = array_repository_from_p2_constraints(&p2_dir(), &requires, Stability::Stable)
        .expect("constraint-filtered vcr pool");
    assert_no_dev_solve_matches_lock(&dir, repo.packages().to_vec(), "constraint-filtered vcr");
}

#[test]
fn laravel_app_no_dev_solve_matches_constraint_filtered_vcr() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-app");
    let json_bytes = fs::read(dir.join("composer.json")).expect("composer.json");
    let requires = load_root_requires(&json_bytes, false);
    let repo = array_repository_from_p2_constraints(&p2_dir(), &requires, Stability::Stable)
        .expect("constraint-filtered vcr pool");
    assert_no_dev_solve_matches_lock(&dir, repo.packages().to_vec(), "constraint-filtered vcr");
}

fn assert_no_dev_solve_matches_lock(dir: &PathBuf, pool_packages: Vec<Package>, source: &str) {
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let json_bytes = fs::read(dir.join("composer.json")).expect("composer.json");

    let expected: BTreeSet<(String, String)> = packages_from_lock_json(&lock_bytes, false)
        .expect("parse lock packages")
        .into_iter()
        .map(|p| (p.name, p.pretty_version))
        .collect();

    let mut repo = ArrayRepository::new();
    for package in pool_packages {
        repo.add_package(package);
    }

    let mut request = Request::new();
    for (name, constraint) in load_root_requires(&json_bytes, false) {
        request
            .require_name(name, Some(parse_constraints(&constraint).unwrap()))
            .unwrap();
    }

    let (mut pool, present) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();
    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap_or_else(|e| panic!("solve {} --no-dev ({source}): {e}", dir.display()));

    let mut got: BTreeSet<(String, String)> = BTreeSet::new();
    for op in tx.operations() {
        match op {
            Operation::Install { package_id } | Operation::Update { to: package_id, .. } => {
                let p = pool.package_by_id(*package_id);
                got.insert((p.name.clone(), p.pretty_version.clone()));
            }
            Operation::Remove { .. } => {}
        }
    }

    assert_eq!(
        got, expected,
        "solver install set must match lock packages (--no-dev, {source}) for {}\nonly in lock: {:?}\nonly in solve: {:?}",
        dir.display(),
        expected.difference(&got).collect::<Vec<_>>(),
        got.difference(&expected).collect::<Vec<_>>()
    );
}
