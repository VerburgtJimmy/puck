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

#[test]
fn solver_fix_locked() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut pool = Pool::new(vec![package_a]);
    let mut request = Request::new();
    request.fix_package(1);
    let mut present = FxHashMap::default();
    present.insert(1, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert!(tx.operations().is_empty());
}

#[test]
fn solver_update_single() {
    let old_a = Package::new("a/a", "1.0.0.0", "1.0");
    let new_a = Package::new("a/a", "1.1.0.0", "1.1");
    let mut pool = Pool::new(vec![old_a, new_a]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    let mut present = FxHashMap::default();
    present.insert(1, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[Operation::Update { from: 1, to: 2 }]
    );
}

#[test]
fn solver_update_current_noop() {
    let locked = Package::new("a/a", "1.0.0.0", "1.0");
    let same = Package::new("a/a", "1.0.0.0", "1.0");
    let mut pool = Pool::new(vec![locked, same]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    let mut present = FxHashMap::default();
    present.insert(1, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    // Prefer lower id among equal versions -> keep locked id 1
    assert!(tx.operations().is_empty());
}

#[test]
fn solver_update_constrained() {
    let old_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mid_a = Package::new("a/a", "1.2.0.0", "1.2");
    let high_a = Package::new("a/a", "2.0.0.0", "2.0");
    let mut pool = Pool::new(vec![old_a, mid_a, high_a]);
    let mut request = Request::new();
    request
        .require_name("a/a", Some(parse_constraints("<2.0.0.0").unwrap()))
        .unwrap();
    let mut present = FxHashMap::default();
    present.insert(1, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[Operation::Update { from: 1, to: 2 }]
    );
}

#[test]
fn solver_three_alternative_require_and_conflict() {
    let mut package_a = Package::new("a/a", "2.0.0.0", "2.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            "<1.1",
            parse_constraints("<1.1").unwrap(),
        ),
    );
    package_a.conflicts.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            "<1.0",
            parse_constraints("<1.0").unwrap(),
        ),
    );
    let middle_b = Package::new("b/b", "1.0.0.0", "1.0");
    let new_b = Package::new("b/b", "1.1.0.0", "1.1");
    let old_b = Package::new("b/b", "0.9.0.0", "0.9");

    let mut pool = Pool::new(vec![package_a, middle_b, new_b, old_b]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 2 }, // b/b 1.0
            Operation::Install { package_id: 1 }, // a/a
        ]
    );
}

#[test]
fn solver_obsolete_replaced_package() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut package_b = Package::new("b/b", "1.0.0.0", "1.0");
    package_b.replaces.insert(
        "a/a".into(),
        Link::new(
            "b/b",
            "a/a",
            "*",
            parse_constraints("*").unwrap(),
        ),
    );

    let mut pool = Pool::new(vec![package_a, package_b]);
    let mut request = Request::new();
    request.require_name("b/b", None).unwrap();
    let mut present = FxHashMap::default();
    present.insert(1, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Remove { package_id: 1 },
            Operation::Install { package_id: 2 },
        ]
    );
}

#[test]
fn solver_skip_replacer_of_existing_package() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );
    let mut package_q = Package::new("q/q", "1.0.0.0", "1.0");
    package_q.replaces.insert(
        "b/b".into(),
        Link::new(
            "q/q",
            "b/b",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");

    let mut pool = Pool::new(vec![package_a, package_q, package_b]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 3 }, // b/b
            Operation::Install { package_id: 1 }, // a/a
        ]
    );
}

#[test]
fn solver_skip_replaced_package_if_replacer_selected() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );
    let mut package_q = Package::new("q/q", "1.0.0.0", "1.0");
    package_q.replaces.insert(
        "b/b".into(),
        Link::new(
            "q/q",
            "b/b",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");

    let mut pool = Pool::new(vec![package_a, package_q, package_b]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    request.require_name("q/q", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 2 }, // q/q
            Operation::Install { package_id: 1 }, // a/a
        ]
    );
}

#[test]
fn solver_honours_not_equal_operator() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            "<=1.3, !=1.3, !=1.2",
            parse_constraints("<=1.3, !=1.3, !=1.2").unwrap(),
        ),
    );
    let b10 = Package::new("b/b", "1.0.0.0", "1.0");
    let b11 = Package::new("b/b", "1.1.0.0", "1.1");
    let b12 = Package::new("b/b", "1.2.0.0", "1.2");
    let b13 = Package::new("b/b", "1.3.0.0", "1.3");

    let mut pool = Pool::new(vec![package_a, b10, b11, b12, b13]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 3 }, // b/b 1.1
            Operation::Install { package_id: 1 }, // a/a
        ]
    );
}

#[test]
fn solver_update_all_with_deps() {
    let mut old_a = Package::new("a/a", "1.0.0.0", "1.0");
    old_a.requires.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", "*", parse_constraints("*").unwrap()),
    );
    let old_b = Package::new("b/b", "1.0.0.0", "1.0");
    let mut new_a = Package::new("a/a", "1.1.0.0", "1.1");
    new_a.requires.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", "*", parse_constraints("*").unwrap()),
    );
    let new_b = Package::new("b/b", "1.1.0.0", "1.1");

    let mut pool = Pool::new(vec![old_a, old_b, new_a, new_b]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    let mut present = FxHashMap::default();
    present.insert(1, ());
    present.insert(2, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Update { from: 2, to: 4 }, // b/b
            Operation::Update { from: 1, to: 3 }, // a/a
        ]
    );
}

#[test]
fn solver_update_via_pool_builder() {
    use crate::pool_builder::{ArrayRepository, PoolBuilder};

    let locked = Package::new("a/a", "1.0.0.0", "1.0");
    let newer = Package::new("a/a", "1.1.0.0", "1.1");
    let mut repo = ArrayRepository::new();
    repo.add_package(newer);

    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    let (mut pool, present) =
        PoolBuilder::build(&[&repo], &[locked], &[], &mut request).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[Operation::Update { from: 1, to: 2 }]
    );
}
