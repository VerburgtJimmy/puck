//! Early Composer `SolverTest` ports.

use crate::link::Link;
use crate::package::Package;
use crate::pool::Pool;
use crate::request::Request;
use crate::solver::Solver;
use crate::transaction::Operation;
use puck_version::parse_constraints;
use rustc_hash::FxHashMap;

fn empty_present() -> FxHashMap<u32, ()> {
    FxHashMap::default()
}

#[test]
fn solver_install_single() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut pool = Pool::new(vec![package_a]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();

    assert_eq!(
        tx.operations(),
        &[Operation::Install { package_id: 1 }]
    );
}

#[test]
fn solver_remove_if_not_requested() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut pool = Pool::new(vec![package_a]);
    let request = Request::new();
    let mut present = FxHashMap::default();
    present.insert(1, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();

    assert_eq!(tx.operations(), &[Operation::Remove { package_id: 1 }]);
}

#[test]
fn solver_install_with_deps() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            "<1.1",
            parse_constraints("<1.1").unwrap(),
        ),
    );
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");
    let package_b11 = Package::new("b/b", "1.1.0.0", "1.1");

    let mut pool = Pool::new(vec![package_a, package_b, package_b11]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();

    // b/b 1.0 before a/a (topo); b/b 1.1 excluded by <1.1
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 2 }, // b/b 1.0
            Operation::Install { package_id: 1 }, // a/a
        ]
    );
}

#[test]
fn solver_install_prefers_highest() {
    let a1 = Package::new("a/a", "1.0.0.0", "1.0");
    let a2 = Package::new("a/a", "2.0.0.0", "2.0");
    let mut pool = Pool::new(vec![a1, a2]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();

    assert_eq!(
        tx.operations(),
        &[Operation::Install { package_id: 2 }] // 2.0
    );
}

#[test]
fn solver_install_deps_in_order() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut package_b = Package::new("b/b", "1.0.0.0", "1.0");
    let mut package_c = Package::new("c/c", "1.0.0.0", "1.0");
    package_b.requires.insert(
        "a/a".into(),
        Link::new(
            "b/b",
            "a/a",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );
    package_b.requires.insert(
        "c/c".into(),
        Link::new(
            "b/b",
            "c/c",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );
    package_c.requires.insert(
        "a/a".into(),
        Link::new(
            "c/c",
            "a/a",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );

    let mut pool = Pool::new(vec![package_a, package_b, package_c]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    request.require_name("b/b", None).unwrap();
    request.require_name("c/c", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();

    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 1 }, // a/a
            Operation::Install { package_id: 3 }, // c/c
            Operation::Install { package_id: 2 }, // b/b
        ]
    );
}

#[test]
fn solver_install_non_existing_fails() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut pool = Pool::new(vec![package_a]);
    let mut request = Request::new();
    request
        .require_name("b/b", Some(parse_constraints("==1").unwrap()))
        .unwrap();

    let err = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unsolvable") || msg.contains("problem"),
        "unexpected error: {msg}"
    );
}
