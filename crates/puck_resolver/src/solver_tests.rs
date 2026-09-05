//! Early Composer `SolverTest` ports.

use crate::link::Link;
use crate::package::Package;
use crate::pool::Pool;
use crate::request::Request;
use crate::solver::Solver;
use crate::transaction::Operation;
use puck_version::parse_constraints;

fn empty_present() -> crate::PresentMap {
    crate::PresentMap::new()
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
    let mut present = crate::PresentMap::new();
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
        msg.contains("Root composer.json requires b/b") || msg.contains("could not be found"),
        "unexpected error: {msg}"
    );
}

#[test]
fn solver_fix_locked() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut pool = Pool::new(vec![package_a]);
    let mut request = Request::new();
    request.fix_package(1);
    let mut present = crate::PresentMap::new();
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
    let mut present = crate::PresentMap::new();
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
    let mut present = crate::PresentMap::new();
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
    let mut present = crate::PresentMap::new();
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
    let mut present = crate::PresentMap::new();
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
    let mut present = crate::PresentMap::new();
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

#[test]
fn solver_illuminate_support_via_framework_replace_from_p2() {
    use crate::metadata::packages_from_p2_json;
    use std::fs;
    use std::path::PathBuf;

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/registry/packagist/p2/laravel$framework.json");
    let packages = packages_from_p2_json(&fs::read(path).unwrap()).unwrap();
    // Keep the fixture pin only - full history makes SAME_NAME huge.
    let mut fw = packages
        .into_iter()
        .find(|p| p.pretty_version == "v13.30.1")
        .expect("v13.30.1");
    // Focus on replace semantics; full transitive closure needs the VCR set.
    fw.requires.clear();
    fw.conflicts.clear();

    // ArrayRepository / PoolBuilder load by real name only (Composer). Once
    // framework is in the pool (as Packagist would load it when requiring
    // laravel/framework), replace satisfies illuminate/support.
    let mut pool = Pool::new(vec![fw]);
    let mut request = Request::new();
    request
        .require_name(
            "illuminate/support",
            Some(parse_constraints("^13.30").unwrap()),
        )
        .unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();

    assert_eq!(tx.operations().len(), 1);
    let Operation::Install { package_id } = tx.operations()[0] else {
        panic!("expected install");
    };
    assert_eq!(pool.package_by_id(package_id).name, "laravel/framework");
}

#[test]
fn solver_install_circular_require() {
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
    let package_b1 = Package::new("b/b", "0.9.0.0", "0.9");
    let mut package_b2 = Package::new("b/b", "1.1.0.0", "1.1");
    package_b2.requires.insert(
        "a/a".into(),
        Link::new(
            "b/b",
            "a/a",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );

    let mut pool = Pool::new(vec![package_a, package_b1, package_b2]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    // Circular: Composer installs b then a; we may differ on order. Both must appear.
    let mut ids: Vec<_> = tx
        .operations()
        .iter()
        .map(|op| match op {
            Operation::Install { package_id } => *package_id,
            other => panic!("unexpected op {other:?}"),
        })
        .collect();
    ids.sort();
    assert_eq!(ids, vec![1, 3]);
}

#[test]
fn solver_use_replacer_if_necessary() {
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
    package_a.requires.insert(
        "c/c".into(),
        Link::new(
            "a/a",
            "c/c",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");
    let mut package_d = Package::new("d/d", "1.0.0.0", "1.0");
    let mut package_d2 = Package::new("d/d", "1.1.0.0", "1.1");
    for pkg in [&mut package_d, &mut package_d2] {
        pkg.replaces.insert(
            "b/b".into(),
            Link::new(
                "d/d",
                "b/b",
                ">=1.0",
                parse_constraints(">=1.0").unwrap(),
            ),
        );
        pkg.replaces.insert(
            "c/c".into(),
            Link::new(
                "d/d",
                "c/c",
                ">=1.0",
                parse_constraints(">=1.0").unwrap(),
            ),
        );
    }

    let mut pool = Pool::new(vec![package_a, package_b, package_d, package_d2]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    request.require_name("d/d", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 4 }, // d/d 1.1
            Operation::Install { package_id: 1 }, // a/a
        ]
    );
}

#[test]
fn solver_pick_older_if_newer_conflicts() {
    let mut package_x = Package::new("x/x", "1.0.0.0", "1.0");
    package_x.requires.insert(
        "a/a".into(),
        Link::new(
            "x/x",
            "a/a",
            ">=2.0.0.0",
            parse_constraints(">=2.0.0.0").unwrap(),
        ),
    );
    package_x.requires.insert(
        "b/b".into(),
        Link::new(
            "x/x",
            "b/b",
            ">=2.0.0.0",
            parse_constraints(">=2.0.0.0").unwrap(),
        ),
    );

    let mut package_a = Package::new("a/a", "2.0.0.0", "2.0.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            ">=2.0.0.0",
            parse_constraints(">=2.0.0.0").unwrap(),
        ),
    );
    let mut new_package_a = Package::new("a/a", "2.1.0.0", "2.1.0");
    new_package_a.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            ">=2.2.0.0",
            parse_constraints(">=2.2.0.0").unwrap(),
        ),
    );
    let new_package_b = Package::new("b/b", "2.1.0.0", "2.1.0");
    let mut package_s = Package::new("s/s", "2.0.0.0", "2.0.0");
    package_s.replaces.insert(
        "a/a".into(),
        Link::new(
            "s/s",
            "a/a",
            ">=2.0.0.0",
            parse_constraints(">=2.0.0.0").unwrap(),
        ),
    );
    package_s.replaces.insert(
        "b/b".into(),
        Link::new(
            "s/s",
            "b/b",
            ">=2.0.0.0",
            parse_constraints(">=2.0.0.0").unwrap(),
        ),
    );

    let mut pool = Pool::new(vec![
        package_x,
        package_a,
        new_package_a,
        new_package_b,
        package_s,
    ]);
    let mut request = Request::new();
    request.require_name("x/x", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 4 }, // b/b 2.1
            Operation::Install { package_id: 2 }, // a/a 2.0
            Operation::Install { package_id: 1 }, // x/x
        ]
    );
}

#[test]
fn solver_install_one_of_two_alternatives() {
    // Two identical name/version candidates; prefer lower pool id (Composer first).
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let package_b = Package::new("a/a", "1.0.0.0", "1.0");
    let mut pool = Pool::new(vec![package_a, package_b]);
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

/// Composer `ArrayRepository` / createPool never loads a provide-only package
/// for a transitive require; solve then fails (must require the provider).
#[test]
fn solver_install_provider_alone_fails() {
    use crate::pool_builder::{ArrayRepository, PoolBuilder};

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
    package_q.provides.insert(
        "b/b".into(),
        Link::new(
            "q/q",
            "b/b",
            "=1.0",
            parse_constraints("=1.0").unwrap(),
        ),
    );

    let mut repo = ArrayRepository::new();
    repo.add_package(package_a);
    repo.add_package(package_q);

    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    let (mut pool, present) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();

    let _ = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap_err();
}

/// Same as provide-only for replace when the replaced package is absent.
#[test]
fn solver_no_install_replacer_of_missing_package() {
    use crate::pool_builder::{ArrayRepository, PoolBuilder};

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

    let mut repo = ArrayRepository::new();
    repo.add_package(package_a);
    repo.add_package(package_q);

    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    let (mut pool, present) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();

    let _ = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap_err();
}

#[test]
fn solver_unsatisfiable_requires() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            ">=2.0",
            parse_constraints(">=2.0").unwrap(),
        ),
    );
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");

    let mut pool = Pool::new(vec![package_a, package_b]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let err = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("b/b")
            || msg.contains("conflict")
            || msg.contains("Problem")
            || msg.contains("could not"),
        "unexpected error: {msg:?}"
    );
}

#[test]
fn solver_conflict_result_fails() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.conflicts.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");

    let mut pool = Pool::new(vec![package_a, package_b]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    request.require_name("b/b", None).unwrap();

    let err = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[test]
fn solver_fix_locked_with_alternative() {
    // Locked a/a 1.0 must stay even when 1.1 exists.
    let locked = Package::new("a/a", "1.0.0.0", "1.0");
    let newer = Package::new("a/a", "1.1.0.0", "1.1");
    let mut pool = Pool::new(vec![locked, newer]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    request.fix_package(1);
    let mut present = crate::PresentMap::new();
    present.insert(1, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert!(tx.operations().is_empty());
}

#[test]
fn solver_update_only_updates_selected_package() {
    let old_a = Package::new("a/a", "1.0.0.0", "1.0");
    let old_b = Package::new("b/b", "1.0.0.0", "1.0");
    let new_a = Package::new("a/a", "1.1.0.0", "1.1");
    let new_b = Package::new("b/b", "1.1.0.0", "1.1");
    let mut pool = Pool::new(vec![old_a, old_b, new_a, new_b]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    request.fix_package(2); // keep b/b 1.0
    let mut present = crate::PresentMap::new();
    present.insert(1, ());
    present.insert(2, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[Operation::Update { from: 1, to: 3 }]
    );
}

#[test]
fn solver_update_does_only_update() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            ">=1.0.0.0",
            parse_constraints(">=1.0.0.0").unwrap(),
        ),
    );
    let old_b = Package::new("b/b", "1.0.0.0", "1.0");
    let new_b = Package::new("b/b", "1.1.0.0", "1.1");
    let mut pool = Pool::new(vec![package_a, old_b, new_b]);
    let mut request = Request::new();
    request.fix_package(1);
    request
        .require_name("b/b", Some(parse_constraints("=1.1.0.0").unwrap()))
        .unwrap();
    let mut present = crate::PresentMap::new();
    present.insert(1, ());
    present.insert(2, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[Operation::Update { from: 2, to: 3 }]
    );
}

#[test]
fn solver_all_jobs() {
    let package_d = Package::new("d/d", "1.0.0.0", "1.0");
    let old_c = Package::new("c/c", "1.0.0.0", "1.0");
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
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");
    let new_b = Package::new("b/b", "1.1.0.0", "1.1");
    let package_c = Package::new("c/c", "1.1.0.0", "1.1");
    let package_d_same = Package::new("d/d", "1.0.0.0", "1.0");

    let mut pool = Pool::new(vec![
        package_d,
        old_c,
        package_a,
        package_b,
        new_b,
        package_c,
        package_d_same,
    ]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    request.require_name("c/c", None).unwrap();
    let mut present = crate::PresentMap::new();
    present.insert(1, ()); // d/d
    present.insert(2, ()); // c/c 1.0

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Remove { package_id: 1 },
            Operation::Install { package_id: 4 }, // b/b 1.0
            Operation::Install { package_id: 3 }, // a/a
            Operation::Update { from: 2, to: 6 }, // c/c
        ]
    );
}

/// Composer `testSolverUpdateFullyConstrained` is identical to constrained update
/// in the fixture; covered by `solver_update_constrained`. This ports the prune
/// sibling: unlocked installed packages not required are removed.
#[test]
fn solver_update_fully_constrained_prunes_installed() {
    let old_a = Package::new("a/a", "1.0.0.0", "1.0");
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");
    let mid_a = Package::new("a/a", "1.2.0.0", "1.2");
    let high_a = Package::new("a/a", "2.0.0.0", "2.0");
    let mut pool = Pool::new(vec![old_a, package_b, mid_a, high_a]);
    let mut request = Request::new();
    request
        .require_name("a/a", Some(parse_constraints("<2.0.0.0").unwrap()))
        .unwrap();
    let mut present = crate::PresentMap::new();
    present.insert(1, ());
    present.insert(2, ());

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Remove { package_id: 2 },
            Operation::Update { from: 1, to: 3 },
        ]
    );
}

#[test]
fn solver_install_same_package_from_different_repositories() {
    use crate::pool_builder::{ArrayRepository, PoolBuilder};

    let mut repo1 = ArrayRepository::new();
    repo1.add_package(Package::new("foo/foo", "1.0.0.0", "1"));
    let mut repo2 = ArrayRepository::new();
    repo2.add_package(Package::new("foo/foo", "1.0.0.0", "1"));

    let mut request = Request::new();
    request.require_name("foo/foo", None).unwrap();
    let (mut pool, present) =
        PoolBuilder::build(&[&repo1, &repo2], &[], &[], &mut request).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    // Prefer first repository (lower pool id).
    assert_eq!(tx.operations(), &[Operation::Install { package_id: 1 }]);
    assert_eq!(pool.len(), 2);
}

/// Provider selected by explicit require; circular require through virtual.
#[test]
fn solver_install_alternative_with_circular_require() {
    use crate::pool_builder::{ArrayRepository, PoolBuilder};

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
    let mut package_b = Package::new("b/b", "1.0.0.0", "1.0");
    package_b.requires.insert(
        "virtual/virtual".into(),
        Link::new(
            "b/b",
            "virtual/virtual",
            ">=1.0",
            parse_constraints(">=1.0").unwrap(),
        ),
    );
    let mut package_c = Package::new("c/c", "1.0.0.0", "1.0");
    package_c.provides.insert(
        "virtual/virtual".into(),
        Link::new(
            "c/c",
            "virtual/virtual",
            "=1.0",
            parse_constraints("=1.0").unwrap(),
        ),
    );
    package_c.requires.insert(
        "a/a".into(),
        Link::new(
            "c/c",
            "a/a",
            "=1.0",
            parse_constraints("=1.0").unwrap(),
        ),
    );
    let mut package_d = Package::new("d/d", "1.0.0.0", "1.0");
    package_d.provides.insert(
        "virtual/virtual".into(),
        Link::new(
            "d/d",
            "virtual/virtual",
            "=1.0",
            parse_constraints("=1.0").unwrap(),
        ),
    );
    package_d.requires.insert(
        "a/a".into(),
        Link::new(
            "d/d",
            "a/a",
            "=1.0",
            parse_constraints("=1.0").unwrap(),
        ),
    );

    let mut repo = ArrayRepository::new();
    repo.add_package(package_a);
    repo.add_package(package_b);
    repo.add_package(package_c);
    repo.add_package(package_d);

    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    request.require_name("c/c", None).unwrap();
    let (mut pool, present) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap();
    let mut names: Vec<_> = tx
        .operations()
        .iter()
        .map(|op| match op {
            Operation::Install { package_id } => {
                pool.package_by_id(*package_id).name.clone()
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    names.sort();
    assert_eq!(names, ["a/a", "b/b", "c/c"]);
    assert!(!pool.packages().iter().any(|p| p.name == "d/d"));
}

#[test]
fn solver_install_dev_alias() {
    let package_a = Package::new("a/a", "2.0.0.0", "2.0");
    let mut package_b = Package::new("b/b", "1.0.0.0", "1.0");
    package_b.requires.insert(
        "a/a".into(),
        Link::new(
            "b/b",
            "a/a",
            "<2.0",
            parse_constraints("<2.0").unwrap(),
        ),
    );
    // Pool ids: a=1, b=2, alias=3
    let package_a_alias = Package::alias(&package_a, 1, "1.1.0.0", "1.1");

    let mut pool = Pool::new(vec![package_a, package_b, package_a_alias]);
    let mut request = Request::new();
    request
        .require_name("a/a", Some(parse_constraints("=2.0").unwrap()))
        .unwrap();
    request.require_name("b/b", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 1 },
            Operation::MarkAliasInstalled { package_id: 3 },
            Operation::Install { package_id: 2 },
        ]
    );
}

#[test]
fn solver_install_recursive_alias_dependencies() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut package_b = Package::new("b/b", "2.0.0.0", "2.0");
    let mut package_a2 = Package::new("a/a", "2.0.0.0", "2.0");
    package_a2.requires.insert(
        "b/b".into(),
        Link::new(
            "a/a",
            "b/b",
            "=2.0",
            parse_constraints("=2.0").unwrap(),
        ),
    );
    package_b.requires.insert(
        "a/a".into(),
        Link::new(
            "b/b",
            "a/a",
            ">=2.0",
            parse_constraints(">=2.0").unwrap(),
        ),
    );
    // Pool ids: a=1, b=2, a2=3, alias=4
    let package_a2_alias = Package::alias(&package_a2, 3, "1.1.0.0", "1.1");

    let mut pool = Pool::new(vec![package_a, package_b, package_a2, package_a2_alias]);
    let mut request = Request::new();
    request
        .require_name("a/a", Some(parse_constraints("=1.1.0.0").unwrap()))
        .unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    // Install set matches Composer; topo order of a2 vs b may differ.
    assert_eq!(
        tx.operations()
            .iter()
            .filter(|op| matches!(op, Operation::MarkAliasInstalled { package_id: 4 }))
            .count(),
        1
    );
    let mut installed: Vec<_> = tx
        .operations()
        .iter()
        .filter_map(|op| match op {
            Operation::Install { package_id } => Some(*package_id),
            _ => None,
        })
        .collect();
    installed.sort();
    assert_eq!(installed, [2, 3]);
}

#[test]
fn solver_install_root_aliases_if_alias_of_is_installed() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut package_a_alias = Package::alias(&package_a, 1, "1.1.0.0", "1.1");
    package_a_alias.root_package_alias = true;

    let package_b = Package::new("b/b", "1.0.0.0", "1.0");
    let mut package_b_alias = Package::alias(&package_b, 3, "1.1.0.0", "1.1");
    package_b_alias.root_package_alias = true;

    let package_c = Package::new("c/c", "1.0.0.0", "1.0");
    let package_c_alias = Package::alias(&package_c, 5, "1.1.0.0", "1.1");

    let mut pool = Pool::new(vec![
        package_a,
        package_a_alias,
        package_b,
        package_b_alias,
        package_c,
        package_c_alias,
    ]);
    let mut request = Request::new();
    request
        .require_name("a/a", Some(parse_constraints("=1.1").unwrap()))
        .unwrap();
    request
        .require_name("b/b", Some(parse_constraints("=1.0").unwrap()))
        .unwrap();
    request
        .require_name("c/c", Some(parse_constraints("=1.0").unwrap()))
        .unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 1 },
            Operation::MarkAliasInstalled { package_id: 2 },
            Operation::Install { package_id: 3 },
            Operation::MarkAliasInstalled { package_id: 4 },
            Operation::Install { package_id: 5 },
            Operation::MarkAliasInstalled { package_id: 6 },
        ]
    );
}
