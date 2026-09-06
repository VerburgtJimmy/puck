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

    assert_eq!(tx.operations(), &[Operation::Install { package_id: 1 }]);
}

#[test]
fn solver_remove_if_not_requested() {
    let package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut pool = Pool::new(vec![package_a]);
    let request = Request::new();
    let mut present = crate::PresentMap::new();
    present.insert(1, ());

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();

    assert_eq!(tx.operations(), &[Operation::Remove { package_id: 1 }]);
}

#[test]
fn solver_install_with_deps() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", "<1.1", parse_constraints("<1.1").unwrap()),
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
        Link::new("b/b", "a/a", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    package_b.requires.insert(
        "c/c".into(),
        Link::new("b/b", "c/c", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    package_c.requires.insert(
        "a/a".into(),
        Link::new("c/c", "a/a", ">=1.0", parse_constraints(">=1.0").unwrap()),
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
    assert_eq!(tx.operations(), &[Operation::Update { from: 1, to: 2 }]);
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
    assert_eq!(tx.operations(), &[Operation::Update { from: 1, to: 2 }]);
}

#[test]
fn solver_three_alternative_require_and_conflict() {
    let mut package_a = Package::new("a/a", "2.0.0.0", "2.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", "<1.1", parse_constraints("<1.1").unwrap()),
    );
    package_a.conflicts.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", "<1.0", parse_constraints("<1.0").unwrap()),
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
        Link::new("b/b", "a/a", "*", parse_constraints("*").unwrap()),
    );

    let mut pool = Pool::new(vec![package_a, package_b]);
    let mut request = Request::new();
    request.require_name("b/b", None).unwrap();
    let mut present = crate::PresentMap::new();
    present.insert(1, ());

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
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
        Link::new("a/a", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    let mut package_q = Package::new("q/q", "1.0.0.0", "1.0");
    package_q.replaces.insert(
        "b/b".into(),
        Link::new("q/q", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
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
        Link::new("a/a", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    let mut package_q = Package::new("q/q", "1.0.0.0", "1.0");
    package_q.replaces.insert(
        "b/b".into(),
        Link::new("q/q", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
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
    let (mut pool, present) = PoolBuilder::build(&[&repo], &[locked], &[], &mut request).unwrap();

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
    assert_eq!(tx.operations(), &[Operation::Update { from: 1, to: 2 }]);
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
        Link::new("a/a", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    let package_b1 = Package::new("b/b", "0.9.0.0", "0.9");
    let mut package_b2 = Package::new("b/b", "1.1.0.0", "1.1");
    package_b2.requires.insert(
        "a/a".into(),
        Link::new("b/b", "a/a", ">=1.0", parse_constraints(">=1.0").unwrap()),
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
        Link::new("a/a", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    package_a.requires.insert(
        "c/c".into(),
        Link::new("a/a", "c/c", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");
    let mut package_d = Package::new("d/d", "1.0.0.0", "1.0");
    let mut package_d2 = Package::new("d/d", "1.1.0.0", "1.1");
    for pkg in [&mut package_d, &mut package_d2] {
        pkg.replaces.insert(
            "b/b".into(),
            Link::new("d/d", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
        );
        pkg.replaces.insert(
            "c/c".into(),
            Link::new("d/d", "c/c", ">=1.0", parse_constraints(">=1.0").unwrap()),
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
    assert_eq!(tx.operations(), &[Operation::Install { package_id: 1 }]);
}

/// Composer `ArrayRepository` / createPool never loads a provide-only package
/// for a transitive require; solve then fails (must require the provider).
#[test]
fn solver_install_provider_alone_fails() {
    use crate::pool_builder::{ArrayRepository, PoolBuilder};

    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    let mut package_q = Package::new("q/q", "1.0.0.0", "1.0");
    package_q.provides.insert(
        "b/b".into(),
        Link::new("q/q", "b/b", "=1.0", parse_constraints("=1.0").unwrap()),
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
        Link::new("a/a", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    let mut package_q = Package::new("q/q", "1.0.0.0", "1.0");
    package_q.replaces.insert(
        "b/b".into(),
        Link::new("q/q", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
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
        Link::new("a/a", "b/b", ">= 2.0", parse_constraints(">=2.0").unwrap()),
    );
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");

    let mut pool = Pool::new(vec![package_a, package_b]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let err = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap_err();
    let crate::Error::Unsolvable(problems) = err else {
        panic!("expected Unsolvable, got {err}");
    };
    assert_eq!(problems.problems.len(), 1);
    let msg = problems.pretty_string(&mut pool, &request);
    let expected = "\n  Problem 1\n    - Root composer.json requires a/a * -> satisfiable by a/a[1.0].\n    - a/a 1.0 requires b/b >= 2.0 -> found b/b[1.0] but it does not match the constraint.\n";
    assert_eq!(msg, expected);
}

#[test]
fn solver_conflict_result_fails() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.conflicts.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", ">= 1.0", parse_constraints(">=1.0").unwrap()),
    );
    let package_b = Package::new("b/b", "1.0.0.0", "1.0");

    let mut pool = Pool::new(vec![package_a, package_b]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();
    request.require_name("b/b", None).unwrap();

    let err = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap_err();
    let crate::Error::Unsolvable(problems) = err else {
        panic!("expected Unsolvable, got {err}");
    };
    assert_eq!(problems.problems.len(), 1);
    let msg = problems.pretty_string(&mut pool, &request);
    let expected = "\n  Problem 1\n    - Root composer.json requires a/a * -> satisfiable by a/a[1.0].\n    - Root composer.json requires b/b * -> satisfiable by b/b[1.0].\n    - a/a 1.0 conflicts with b/b 1.0.\n";
    assert_eq!(msg, expected);
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
    assert_eq!(tx.operations(), &[Operation::Update { from: 1, to: 3 }]);
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
    assert_eq!(tx.operations(), &[Operation::Update { from: 2, to: 3 }]);
}

#[test]
fn solver_all_jobs() {
    let package_d = Package::new("d/d", "1.0.0.0", "1.0");
    let old_c = Package::new("c/c", "1.0.0.0", "1.0");
    let mut package_a = Package::new("a/a", "2.0.0.0", "2.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", "<1.1", parse_constraints("<1.1").unwrap()),
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
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
        Link::new("a/a", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
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
        Link::new("c/c", "a/a", "=1.0", parse_constraints("=1.0").unwrap()),
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
        Link::new("d/d", "a/a", "=1.0", parse_constraints("=1.0").unwrap()),
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

    let tx = Solver::new(&mut pool).solve(&request, &present).unwrap();
    let mut names: Vec<_> = tx
        .operations()
        .iter()
        .map(|op| match op {
            Operation::Install { package_id } => pool.package_by_id(*package_id).name.clone(),
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
        Link::new("b/b", "a/a", "<2.0", parse_constraints("<2.0").unwrap()),
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
        Link::new("a/a", "b/b", "=2.0", parse_constraints("=2.0").unwrap()),
    );
    package_b.requires.insert(
        "a/a".into(),
        Link::new("b/b", "a/a", ">=2.0", parse_constraints(">=2.0").unwrap()),
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

/// Same name+version, different requires: require order can change the pick
/// when the "wrong" extension is first in the pool.
#[test]
fn solver_multi_package_name_version_depends_on_require_order() {
    use puck_version::normalize;

    let php74 = Package::new("ourcustom/php", normalize("7.4.23").unwrap(), "7.4.23");
    let php80 = Package::new("ourcustom/php", normalize("8.0.10").unwrap(), "8.0.10");
    let mut ext74 = Package::new("ourcustom/ext-foobar", normalize("1.0").unwrap(), "1.0");
    let mut ext80 = Package::new("ourcustom/ext-foobar", normalize("1.0").unwrap(), "1.0");
    ext74.requires.insert(
        "ourcustom/php".into(),
        Link::new(
            "ourcustom/ext-foobar",
            "ourcustom/php",
            ">=7.4.0,<7.5.0",
            parse_constraints(">=7.4.0,<7.5.0").unwrap(),
        ),
    );
    ext80.requires.insert(
        "ourcustom/php".into(),
        Link::new(
            "ourcustom/ext-foobar",
            "ourcustom/php",
            ">=8.0.0,<8.1.0",
            parse_constraints(">=8.0.0,<8.1.0").unwrap(),
        ),
    );

    // Pool order: php74, php80, ext74, ext80 (ext74 first among same-name).
    let mut pool = Pool::new(vec![php74, php80, ext74, ext80]);

    // PHP before ext -> prefer highest PHP (8.0) then matching ext.
    let mut request = Request::new();
    request.require_name("ourcustom/php", None).unwrap();
    request.require_name("ourcustom/ext-foobar", None).unwrap();
    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    let names_versions: Vec<_> = tx
        .operations()
        .iter()
        .filter_map(|op| match op {
            Operation::Install { package_id } => {
                let p = pool.package_by_id(*package_id);
                Some((p.name.as_str(), p.pretty_version.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        names_versions,
        [("ourcustom/php", "8.0.10"), ("ourcustom/ext-foobar", "1.0")]
    );
    // Confirm the ext is the php80-linked one (id 4).
    assert!(matches!(
        tx.operations(),
        [
            Operation::Install { package_id: 2 },
            Operation::Install { package_id: 4 },
        ]
    ));

    // Ext before PHP -> first same-name ext (ext74) wins, pulling php74.
    let mut request = Request::new();
    request.require_name("ourcustom/ext-foobar", None).unwrap();
    request.require_name("ourcustom/php", None).unwrap();
    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert!(matches!(
        tx.operations(),
        [
            Operation::Install { package_id: 1 },
            Operation::Install { package_id: 3 },
        ]
    ));
}

/// When the higher PHP-matching extension is first in the pool, require order
/// no longer matters.
#[test]
fn solver_multi_package_name_version_independent_when_ordered_descending() {
    use puck_version::normalize;

    let php74 = Package::new("ourcustom/php", normalize("7.4").unwrap(), "7.4");
    let php80 = Package::new("ourcustom/php", normalize("8.0").unwrap(), "8.0");
    let mut ext80 = Package::new("ourcustom/ext-foobar", normalize("1.0").unwrap(), "1.0");
    let mut ext74 = Package::new("ourcustom/ext-foobar", normalize("1.0").unwrap(), "1.0");
    ext80.requires.insert(
        "ourcustom/php".into(),
        Link::new(
            "ourcustom/ext-foobar",
            "ourcustom/php",
            ">=8.0.0,<8.1.0",
            parse_constraints(">=8.0.0,<8.1.0").unwrap(),
        ),
    );
    ext74.requires.insert(
        "ourcustom/php".into(),
        Link::new(
            "ourcustom/ext-foobar",
            "ourcustom/php",
            ">=7.4.0,<7.5.0",
            parse_constraints(">=7.4.0,<7.5.0").unwrap(),
        ),
    );

    // ext80 before ext74 in pool.
    let mut pool = Pool::new(vec![php74, php80, ext80, ext74]);

    for order in [
        ["ourcustom/php", "ourcustom/ext-foobar"],
        ["ourcustom/ext-foobar", "ourcustom/php"],
    ] {
        let mut request = Request::new();
        for name in order {
            request.require_name(name, None).unwrap();
        }
        let tx = Solver::new(&mut pool)
            .solve(&request, &empty_present())
            .unwrap();
        assert!(
            matches!(
                tx.operations(),
                [
                    Operation::Install { package_id: 2 },
                    Operation::Install { package_id: 3 },
                ]
            ),
            "order {order:?} => {:?}",
            tx.operations()
        );
    }
}

/// Composer Issue #265 / SolverTest: with default minimum-stability stable,
/// createPool rejects -dev packages (no stability flags), so the root require
/// cannot be satisfied.
#[test]
fn solver_issue_265_unsatisfiable() {
    use crate::pool_builder::{ArrayRepository, PoolBuilder};
    use puck_version::normalize;

    let a1 = Package::new(
        "a/a",
        normalize("2.0.999999-dev").unwrap(),
        "2.0.999999-dev",
    );
    let a2 = Package::new("a/a", normalize("2.1-dev").unwrap(), "2.1-dev");
    let a3 = Package::new("a/a", normalize("2.2-dev").unwrap(), "2.2-dev");
    let mut b1 = Package::new("b/b", normalize("2.0.10").unwrap(), "2.0.10");
    let mut b2 = Package::new("b/b", normalize("2.0.9").unwrap(), "2.0.9");
    let mut c = Package::new("c/c", normalize("2.0-dev").unwrap(), "2.0-dev");
    let mut d = Package::new("d/d", normalize("2.0.9").unwrap(), "2.0.9");

    c.requires.insert(
        "a/a".into(),
        Link::new("c/c", "a/a", ">=2.0", parse_constraints(">=2.0").unwrap()),
    );
    c.requires.insert(
        "d/d".into(),
        Link::new("c/c", "d/d", ">=2.0", parse_constraints(">=2.0").unwrap()),
    );
    d.requires.insert(
        "a/a".into(),
        Link::new("d/d", "a/a", ">=2.1", parse_constraints(">=2.1").unwrap()),
    );
    d.requires.insert(
        "b/b".into(),
        Link::new(
            "d/d",
            "b/b",
            ">=2.0-dev",
            parse_constraints(">=2.0-dev").unwrap(),
        ),
    );
    b1.requires.insert(
        "a/a".into(),
        Link::new(
            "b/b",
            "a/a",
            "=2.1.0.0-dev",
            parse_constraints("=2.1.0.0-dev").unwrap(),
        ),
    );
    b2.requires.insert(
        "a/a".into(),
        Link::new(
            "b/b",
            "a/a",
            "=2.1.0.0-dev",
            parse_constraints("=2.1.0.0-dev").unwrap(),
        ),
    );
    b2.replaces.insert(
        "d/d".into(),
        Link::new(
            "b/b",
            "d/d",
            "=2.0.9.0",
            parse_constraints("=2.0.9.0").unwrap(),
        ),
    );

    let mut repo = ArrayRepository::new();
    for p in [a1, a2, a3, b1, b2, c, d] {
        repo.add_package(p);
    }

    let mut request = Request::new();
    request
        .require_name("c/c", Some(parse_constraints("=2.0.0.0-dev").unwrap()))
        .unwrap();
    let (mut pool, present) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();

    let _ = Solver::new(&mut pool)
        .solve(&request, &present)
        .unwrap_err();
}

#[test]
fn solver_require_mismatch_exception() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    package_a.requires.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", ">= 1.0", parse_constraints(">=1.0").unwrap()),
    );
    let mut package_b = Package::new("b/b", "1.0.0.0", "1.0");
    package_b.requires.insert(
        "c/c".into(),
        Link::new("b/b", "c/c", ">= 1.0", parse_constraints(">=1.0").unwrap()),
    );
    let package_b2 = Package::new("b/b", "0.9.0.0", "0.9");
    let mut package_c = Package::new("c/c", "1.0.0.0", "1.0");
    package_c.requires.insert(
        "d/d".into(),
        Link::new("c/c", "d/d", ">= 1.0", parse_constraints(">=1.0").unwrap()),
    );
    let mut package_d = Package::new("d/d", "1.0.0.0", "1.0");
    package_d.requires.insert(
        "b/b".into(),
        Link::new("d/d", "b/b", "< 1.0", parse_constraints("<1.0").unwrap()),
    );

    let mut pool = Pool::new(vec![package_a, package_b, package_b2, package_c, package_d]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let err = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap_err();
    let crate::Error::Unsolvable(problems) = err else {
        panic!("expected Unsolvable, got {err}");
    };
    assert_eq!(problems.problems.len(), 1);
    let msg = problems.pretty_string(&mut pool, &request);
    let expected = "\n  Problem 1\n    - Root composer.json requires a/a * -> satisfiable by a/a[1.0].\n    - a/a 1.0 requires b/b >= 1.0 -> satisfiable by b/b[1.0].\n    - b/b 1.0 requires c/c >= 1.0 -> satisfiable by c/c[1.0].\n    - c/c 1.0 requires d/d >= 1.0 -> satisfiable by d/d[1.0].\n    - d/d 1.0 requires b/b < 1.0 -> satisfiable by b/b[0.9].\n    - You can only install one version of a package, so only one of these can be installed: b/b[0.9, 1.0].\n";
    assert_eq!(msg, expected);
}

/// Composer `testLearnLiteralsWithSortedRuleLiterals` - success path exercising
/// replace + version pick under learn/backtrack.
#[test]
fn solver_learn_literals_with_sorted_rule_literals() {
    let twig2 = Package::new("twig/twig", "2.0.0.0", "2.0");
    let twig16 = Package::new("twig/twig", "1.6.0.0", "1.6");
    let twig15 = Package::new("twig/twig", "1.5.0.0", "1.5");
    let mut symfony = Package::new("symfony/symfony", "2.0.0.0", "2.0");
    let mut twig_bridge = Package::new("symfony/twig-bridge", "2.0.0.0", "2.0");
    twig_bridge.requires.insert(
        "twig/twig".into(),
        Link::new(
            "symfony/twig-bridge",
            "twig/twig",
            "<2.0",
            parse_constraints("<2.0").unwrap(),
        ),
    );
    symfony.replaces.insert(
        "symfony/twig-bridge".into(),
        Link::new(
            "symfony/symfony",
            "symfony/twig-bridge",
            "=2.0",
            parse_constraints("=2.0").unwrap(),
        ),
    );

    let mut pool = Pool::new(vec![twig2, twig16, twig15, symfony, twig_bridge]);
    let mut request = Request::new();
    request.require_name("symfony/twig-bridge", None).unwrap();
    request.require_name("twig/twig", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    assert_eq!(
        tx.operations(),
        &[
            Operation::Install { package_id: 2 }, // twig 1.6
            Operation::Install { package_id: 5 }, // twig-bridge
        ]
    );
}

/// Composer `testLearnPositiveLiteral` - complex graph that forces learning a
/// positive assertion from a negative decision. Assert install **set** (order
/// may differ); Composer also asserts an internal learn-path flag we omit.
#[test]
fn solver_learn_positive_literal() {
    let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
    let mut package_b = Package::new("b/b", "1.0.0.0", "1.0");
    let mut package_c1 = Package::new("c/c", "1.0.0.0", "1.0");
    let mut package_c2 = Package::new("c/c", "2.0.0.0", "2.0");
    let mut package_d = Package::new("d/d", "1.0.0.0", "1.0");
    let mut package_e = Package::new("e/e", "1.0.0.0", "1.0");
    let package_f1 = Package::new("f/f", "1.0.0.0", "1.0");
    let package_f2 = Package::new("f/f", "2.0.0.0", "2.0");
    let package_g1 = Package::new("g/g", "1.0.0.0", "1.0");
    let package_g2 = Package::new("g/g", "2.0.0.0", "2.0");
    let package_g3 = Package::new("g/g", "3.0.0.0", "3.0");

    package_a.requires.insert(
        "b/b".into(),
        Link::new("a/a", "b/b", "=1.0", parse_constraints("=1.0").unwrap()),
    );
    package_a.requires.insert(
        "c/c".into(),
        Link::new("a/a", "c/c", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    package_a.requires.insert(
        "d/d".into(),
        Link::new("a/a", "d/d", "=1.0", parse_constraints("=1.0").unwrap()),
    );
    package_b.requires.insert(
        "e/e".into(),
        Link::new("b/b", "e/e", "=1.0", parse_constraints("=1.0").unwrap()),
    );
    package_c1.requires.insert(
        "f/f".into(),
        Link::new("c/c", "f/f", "=1.0", parse_constraints("=1.0").unwrap()),
    );
    package_c2.requires.insert(
        "f/f".into(),
        Link::new("c/c", "f/f", "=1.0", parse_constraints("=1.0").unwrap()),
    );
    package_c2.requires.insert(
        "g/g".into(),
        Link::new("c/c", "g/g", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    package_d.requires.insert(
        "f/f".into(),
        Link::new("d/d", "f/f", ">=1.0", parse_constraints(">=1.0").unwrap()),
    );
    package_e.requires.insert(
        "g/g".into(),
        Link::new("e/e", "g/g", "<=2.0", parse_constraints("<=2.0").unwrap()),
    );

    let mut pool = Pool::new(vec![
        package_a, package_b, package_c1, package_c2, package_d, package_e, package_f1, package_f2,
        package_g1, package_g2, package_g3,
    ]);
    let mut request = Request::new();
    request.require_name("a/a", None).unwrap();

    let tx = Solver::new(&mut pool)
        .solve(&request, &empty_present())
        .unwrap();
    let mut names: Vec<_> = tx
        .operations()
        .iter()
        .filter_map(|op| match op {
            Operation::Install { package_id } => {
                let p = pool.package_by_id(*package_id);
                Some(format!("{}@{}", p.name, p.pretty_version))
            }
            _ => None,
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "a/a@1.0", "b/b@1.0", "c/c@2.0", "d/d@1.0", "e/e@1.0", "f/f@1.0", "g/g@2.0",
        ]
    );
}
