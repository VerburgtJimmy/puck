//! PubGrub litmus: `laravel/framework` replace `illuminate/support` on recorded metadata.
//!
//! Passes when Composer's pool model answers `whatProvides` correctly.
//! Documents why PubGrub is not adopted (see puck-notes ADR 0001).

use crate::link::Link;
use crate::package::Package;
use crate::pool::Pool;
use puck_version::parse_constraints;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn registry_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/registry/packagist/p2")
}

fn load_framework_v13_30_1() -> Package {
    let path = registry_root().join("laravel$framework.json");
    let raw = fs::read_to_string(&path).expect("read laravel/framework p2");
    let data: Value = serde_json::from_str(&raw).expect("parse p2");
    let versions = data["packages"]["laravel/framework"]
        .as_array()
        .expect("versions array");
    let v = versions
        .iter()
        .find(|v| v["version"].as_str() == Some("v13.30.1"))
        .expect("v13.30.1 in snapshot");

    let name = v["name"].as_str().unwrap().to_string();
    let version = v["version_normalized"].as_str().unwrap().to_string();
    let pretty = v["version"].as_str().unwrap().to_string();
    let mut package = Package::new(name, version.clone(), pretty);

    let replaces = v["replace"].as_object().expect("replace map");
    assert!(
        replaces.len() >= 38,
        "expected >=38 replaces, got {}",
        replaces.len()
    );

    for (target, constraint_v) in replaces {
        let pretty_c = constraint_v.as_str().expect("replace constraint string");
        // ArrayLoader::createLink: self.version -> package version
        let expanded = if pretty_c == "self.version" {
            version.as_str()
        } else {
            pretty_c
        };
        let constraint = parse_constraints(expanded).expect("parse replace constraint");
        let link = Link::new(
            package.name.clone(),
            target.clone(),
            expanded.to_string(),
            constraint,
        );
        package.replaces.insert(target.to_ascii_lowercase(), link);
    }

    package
}

#[test]
fn framework_replace_count_and_self_version() {
    let fw = load_framework_v13_30_1();
    assert_eq!(fw.replaces.len(), 38);
    let support = fw.replaces.get("illuminate/support").expect("support");
    assert_eq!(support.pretty_constraint, "13.30.1.0");
}

#[test]
fn pool_what_provides_illuminate_support_via_framework_replace() {
    let fw = load_framework_v13_30_1();
    let mut pool = Pool::new(vec![fw]);
    let constraint = parse_constraints("^13.0").unwrap();
    let providers = pool
        .what_provides("illuminate/support", Some(&constraint))
        .unwrap();
    assert_eq!(providers, vec![1]);
    assert_eq!(pool.package_by_id(1).name, "laravel/framework");
}

#[test]
fn naive_name_only_model_misses_replace() {
    // Illustrates the PubGrub-shaped mistake: looking up only packages whose
    // primary name is illuminate/support finds nothing when only framework is loaded.
    let fw = load_framework_v13_30_1();
    let by_primary: Vec<_> = std::iter::once(&fw)
        .filter(|p| p.name == "illuminate/support")
        .collect();
    assert!(
        by_primary.is_empty(),
        "primary-name lookup must miss framework replace"
    );
    assert!(
        fw.names(true).iter().any(|n| n == "illuminate/support"),
        "Composer getNames includes replace targets"
    );
}
