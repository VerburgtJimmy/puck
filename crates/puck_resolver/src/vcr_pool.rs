//! Constraint-filtered package loading from Packagist p2 metadata.
//!
//! Subset of Composer `PoolBuilder` name/constraint marking: start from root
//! requires, load matching p2 versions, enqueue their requires, OR-merge
//! constraints when the same name is requested again.

use crate::metadata::{expand_minified_versions, package_from_composer_package};
use crate::package::Package;
use crate::platform::is_platform_package;
use crate::pool_builder::ArrayRepository;
use crate::{Error, Result};
use indexmap::{IndexMap, IndexSet};
use puck_version::{ConstraintExpr, Operator, Stability, parse_constraints, parse_stability};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;

const MINIFIED_MARKER: &str = "composer/2.0";

/// Load p2 JSON for a package name.
///
/// `Ok(None)` means metadata is unavailable (skip; may be replace/provide only).
/// `Err` is a hard failure.
pub type P2Getter<'a> = dyn Fn(&str) -> std::result::Result<Option<Vec<u8>>, String> + 'a;

/// Build an [`ArrayRepository`] from p2 metadata using root requires as the
/// seed, filtering versions by accumulated constraints.
///
/// Names in `skip_names` (claimed by an earlier canonical repository) are never
/// loaded from p2.
pub fn array_repository_from_p2_constraints(
    load_p2: &P2Getter<'_>,
    root_requires: &[(String, String)],
    minimum_stability: Stability,
    skip_names: &IndexSet<String>,
) -> Result<ArrayRepository> {
    let mut to_load: IndexMap<String, ConstraintExpr> = IndexMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for (name, constraint) in root_requires {
        if is_platform_package(name) || skip_names.contains(name) {
            continue;
        }
        let expr = parse_constraints(constraint)?;
        mark_for_loading(&mut to_load, &mut queue, name, expr, skip_names);
    }

    let mut loaded_constraint: IndexMap<String, ConstraintExpr> = IndexMap::new();
    let mut seen: IndexSet<(String, String)> = IndexSet::new();
    let mut repo = ArrayRepository::new();

    while let Some(name) = queue.pop_front() {
        if skip_names.contains(&name) {
            let _ = to_load.swap_remove(&name);
            continue;
        }
        let Some(constraint) = to_load.swap_remove(&name) else {
            continue;
        };
        if let Some(prev) = loaded_constraint.get(&name) {
            if constraint_implies(prev, &constraint) {
                continue;
            }
        }

        let bytes = match load_p2(&name) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                // May be satisfied only via replace/provide of another package.
                loaded_constraint.insert(name, constraint);
                continue;
            }
            Err(e) => {
                return Err(Error::Message(format!("p2 metadata for {name}: {e}")));
            }
        };
        let packages = packages_matching_constraint(&bytes, &name, &constraint, minimum_stability)?;
        for package in packages {
            let key = (package.name.clone(), package.version.clone());
            if !seen.insert(key) {
                continue;
            }
            for link in package.requires.values() {
                if is_platform_package(&link.target) {
                    continue;
                }
                mark_for_loading(
                    &mut to_load,
                    &mut queue,
                    &link.target,
                    link.constraint.clone(),
                    skip_names,
                );
            }
            repo.add_package(package);
        }

        if let Some(prev) = loaded_constraint.swap_remove(&name) {
            loaded_constraint.insert(name, or_constraints(prev, constraint));
        } else {
            loaded_constraint.insert(name, constraint);
        }
    }

    Ok(repo)
}

/// Filesystem getter rooted at a `packagist/p2` directory (tests / VCR).
pub fn p2_dir_getter(
    p2_dir: PathBuf,
) -> impl Fn(&str) -> std::result::Result<Option<Vec<u8>>, String> {
    move |name: &str| {
        let path = p2_dir.join(format!("{}.json", name.replace('/', "$")));
        if !path.is_file() {
            return Ok(None);
        }
        std::fs::read(&path)
            .map(Some)
            .map_err(|e| format!("read {}: {e}", path.display()))
    }
}

fn mark_for_loading(
    to_load: &mut IndexMap<String, ConstraintExpr>,
    queue: &mut VecDeque<String>,
    name: &str,
    constraint: ConstraintExpr,
    skip_names: &IndexSet<String>,
) {
    let name = name.to_ascii_lowercase();
    if skip_names.contains(&name) {
        return;
    }
    if let Some(existing) = to_load.get_mut(&name) {
        *existing = or_constraints(existing.clone(), constraint);
        return;
    }
    to_load.insert(name.clone(), constraint);
    queue.push_back(name);
}

fn or_constraints(a: ConstraintExpr, b: ConstraintExpr) -> ConstraintExpr {
    if a == b {
        return a;
    }
    ConstraintExpr::Multi {
        conjunctive: false,
        constraints: vec![a, b],
    }
}

/// True when every version matching `inner` also matches `outer` is unknown
/// without intervals; we only treat exact equality as "already covered".
fn constraint_implies(loaded: &ConstraintExpr, needed: &ConstraintExpr) -> bool {
    loaded == needed
}

fn version_matches(constraint: &ConstraintExpr, normalized_version: &str) -> bool {
    let provider = ConstraintExpr::simple(Operator::Eq, normalized_version.to_string());
    constraint.matches_provider(&provider)
}

fn packages_matching_constraint(
    bytes: &[u8],
    package_name: &str,
    constraint: &ConstraintExpr,
    minimum_stability: Stability,
) -> Result<Vec<Package>> {
    let data: Value = serde_json::from_slice(bytes)
        .map_err(|e| Error::Message(format!("invalid p2 json: {e}")))?;
    let minified = data.get("minified").and_then(|v| v.as_str()) == Some(MINIFIED_MARKER);
    let packages = data
        .get("packages")
        .and_then(|p| p.as_object())
        .ok_or_else(|| Error::Message("p2 json missing packages object".into()))?;

    let mut out = Vec::new();
    for (_name, versions) in packages {
        let Some(versions) = versions.as_array() else {
            continue;
        };
        let versions = if minified {
            expand_minified_versions(versions)
        } else {
            versions.clone()
        };
        for version in &versions {
            let Some(package) = package_from_composer_package(version)? else {
                continue;
            };
            if package.name != package_name {
                continue;
            }
            if parse_stability(&package.pretty_version) > minimum_stability {
                continue;
            }
            if !version_matches(constraint, &package.version) {
                continue;
            }
            out.push(package);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p2_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/registry/packagist/p2")
    }

    #[test]
    fn loads_framework_versions_for_caret_constraint() {
        let get = p2_dir_getter(p2_dir());
        let repo = array_repository_from_p2_constraints(
            &get,
            &[("laravel/framework".into(), "^13.17".into())],
            Stability::Stable,
            &IndexSet::new(),
        )
        .unwrap();
        let frameworks: Vec<_> = repo
            .packages()
            .iter()
            .filter(|p| p.name == "laravel/framework")
            .collect();
        assert!(
            frameworks.len() >= 10 && frameworks.len() < 100,
            "expected constrained framework set, got {}",
            frameworks.len()
        );
        assert!(frameworks.iter().any(|p| p.pretty_version == "v13.30.1"));
        assert!(frameworks.iter().all(|p| p.version.starts_with("13.")));
    }
}
