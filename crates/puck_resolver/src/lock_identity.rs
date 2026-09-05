//! Lock-identity gates: solve against recorded lock/VCR metadata.
//!
//! M3 exit gate is identical locks vs Composer. This module starts with a
//! weaker but necessary check: given only the locked package universe, the
//! solver’s install set for root requires must match `composer.lock`
//! `packages` (name + pretty version).

use crate::metadata::packages_from_lock_json;
use crate::platform::is_platform_package;
use crate::pool_builder::{ArrayRepository, PoolBuilder};
use crate::request::Request;
use crate::solver::Solver;
use crate::transaction::Operation;
use puck_version::parse_constraints;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn skeleton_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-skeleton")
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
    assert_no_dev_solve_matches_lock(&skeleton_dir());
}

#[test]
fn laravel_app_no_dev_solve_matches_lock_packages() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-app");
    assert_no_dev_solve_matches_lock(&dir);
}

fn assert_no_dev_solve_matches_lock(dir: &PathBuf) {
    let lock_bytes = fs::read(dir.join("composer.lock")).expect("lock");
    let json_bytes = fs::read(dir.join("composer.json")).expect("composer.json");

    let locked = packages_from_lock_json(&lock_bytes, false).expect("parse lock packages");
    let expected: BTreeSet<(String, String)> = locked
        .iter()
        .map(|p| (p.name.clone(), p.pretty_version.clone()))
        .collect();

    let mut repo = ArrayRepository::new();
    for package in locked {
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
        .unwrap_or_else(|e| panic!("solve {} --no-dev: {e}", dir.display()));

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
        "solver install set must match lock packages (--no-dev) for {}\nonly in lock: {:?}\nonly in solve: {:?}",
        dir.display(),
        expected.difference(&got).collect::<Vec<_>>(),
        got.difference(&expected).collect::<Vec<_>>()
    );
}
