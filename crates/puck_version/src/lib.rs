//! Composer version parsing, normalisation, constraint parsing and matching.
//!
//! Own implementation. Do not use the `semver` crate - Composer's rules
//! (4-part versions, stability suffixes, `dev-` branches, `~`, `^`, `*`, `as`
//! aliases, `@stability` flags) are not SemVer.
//!
//! Port target: https://github.com/composer/semver (pinned fixtures: 3.4.3).

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

mod compare;
mod constraint;
mod parser;
mod version;

pub use compare::{php_version_compare, version_compare, version_compare_with_branches};
pub use constraint::{ConstraintExpr, Operator, parse_constraints, satisfies};
pub use parser::{
    normalize, normalize_branch, normalize_stability, parse_numeric_alias_prefix, parse_stability,
};
pub use version::{Stability, Version};

/// Errors from version / constraint parsing and matching.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid version string \"{0}\"")]
    InvalidVersion(String),
    #[error("invalid version string \"{version}\"{extra}")]
    InvalidVersionExtra { version: String, extra: String },
    #[error("invalid constraint: {0}")]
    InvalidConstraint(String),
    #[error("invalid stability string \"{0}\", expected one of stable, RC, beta, alpha or dev")]
    InvalidStability(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use serde::Deserialize;
    use std::fs;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name)
    }

    #[derive(Debug, Deserialize)]
    struct NormCase {
        input: String,
        normalized: String,
    }

    #[derive(Debug, Deserialize)]
    struct FailCase {
        input: String,
    }

    #[derive(Debug, Deserialize)]
    struct StabilityCase {
        version: String,
        stability: String,
    }

    #[derive(Debug, Deserialize)]
    struct AliasCase {
        input: String,
        prefix: serde_json::Value,
    }

    #[test]
    fn normalize_success_fixtures() {
        let cases: Vec<NormCase> = serde_json::from_str(
            &fs::read_to_string(fixture("normalize_success.json")).expect("read fixture"),
        )
        .expect("parse fixture");
        for case in cases {
            let got = normalize(&case.input).unwrap_or_else(|e| {
                panic!("normalize({:?}) failed: {e}", case.input);
            });
            assert_eq!(got, case.normalized, "normalize({:?})", case.input);
        }
    }

    #[test]
    fn normalize_fail_fixtures() {
        let cases: Vec<FailCase> = serde_json::from_str(
            &fs::read_to_string(fixture("normalize_fail.json")).expect("read fixture"),
        )
        .expect("parse fixture");
        for case in cases {
            assert!(
                normalize(&case.input).is_err(),
                "expected normalize({:?}) to fail",
                case.input
            );
        }
    }

    #[test]
    fn normalize_branch_fixtures() {
        let cases: Vec<NormCase> = serde_json::from_str(
            &fs::read_to_string(fixture("normalize_branch.json")).expect("read fixture"),
        )
        .expect("parse fixture");
        for case in cases {
            assert_eq!(
                normalize_branch(&case.input),
                case.normalized,
                "normalize_branch({:?})",
                case.input
            );
        }
    }

    #[test]
    fn parse_stability_fixtures() {
        let cases: Vec<StabilityCase> = serde_json::from_str(
            &fs::read_to_string(fixture("parse_stability.json")).expect("read fixture"),
        )
        .expect("parse fixture");
        for case in cases {
            assert_eq!(
                parse_stability(&case.version).as_str(),
                case.stability,
                "parse_stability({:?})",
                case.version
            );
        }
    }

    #[test]
    fn numeric_alias_prefix_fixtures() {
        let cases: Vec<AliasCase> = serde_json::from_str(
            &fs::read_to_string(fixture("numeric_alias_prefix.json")).expect("read fixture"),
        )
        .expect("parse fixture");
        for case in cases {
            let got = parse_numeric_alias_prefix(&case.input);
            if case.prefix.as_bool() == Some(false) {
                assert!(got.is_none(), "expected None for {:?}", case.input);
            } else {
                let expected = case.prefix.as_str().expect("string prefix");
                assert_eq!(got.as_deref(), Some(expected), "alias {:?}", case.input);
            }
        }
    }

    #[derive(Debug, Deserialize)]
    struct ConstraintCase {
        input: String,
        tree: serde_json::Value,
    }

    fn constraint_tree_json(expr: &ConstraintExpr) -> serde_json::Value {
        match expr {
            ConstraintExpr::MatchAll => serde_json::json!({ "type": "match_all" }),
            ConstraintExpr::Simple { operator, version } => serde_json::json!({
                "type": "constraint",
                "operator": operator.to_string(),
                "version": version,
            }),
            ConstraintExpr::Multi {
                conjunctive,
                constraints,
            } => serde_json::json!({
                "type": "multi",
                "conjunctive": conjunctive,
                "constraints": constraints.iter().map(constraint_tree_json).collect::<Vec<_>>(),
            }),
        }
    }

    fn assert_constraint_fixtures(name: &str) {
        let cases: Vec<ConstraintCase> =
            serde_json::from_str(&fs::read_to_string(fixture(name)).expect("read fixture"))
                .expect("parse fixture");
        for case in cases {
            let got = parse_constraints(&case.input).unwrap_or_else(|e| {
                panic!("parse_constraints({:?}) failed: {e}", case.input);
            });
            assert_eq!(
                constraint_tree_json(&got),
                case.tree,
                "parse_constraints({:?})",
                case.input
            );
        }
    }

    #[test]
    fn constraints_simple_fixtures() {
        assert_constraint_fixtures("constraints_simple.json");
    }

    #[test]
    fn constraints_wildcard_fixtures() {
        assert_constraint_fixtures("constraints_wildcard.json");
    }

    #[test]
    fn constraints_tilde_fixtures() {
        assert_constraint_fixtures("constraints_tilde.json");
    }

    #[test]
    fn constraints_caret_fixtures() {
        assert_constraint_fixtures("constraints_caret.json");
    }

    #[test]
    fn constraints_hyphen_fixtures() {
        assert_constraint_fixtures("constraints_hyphen.json");
    }

    #[test]
    fn constraints_multi_fixtures() {
        assert_constraint_fixtures("constraints_multi.json");
    }

    #[test]
    fn constraints_fail_fixtures() {
        let cases: Vec<FailCase> = serde_json::from_str(
            &fs::read_to_string(fixture("constraints_fail.json")).expect("read fixture"),
        )
        .expect("parse fixture");
        for case in cases {
            assert!(
                parse_constraints(&case.input).is_err(),
                "expected parse_constraints({:?}) to fail",
                case.input
            );
        }
    }

    #[derive(Debug, Deserialize)]
    struct SatisfiesCase {
        version: String,
        constraint: String,
        satisfies: bool,
    }

    #[test]
    fn satisfies_fixtures() {
        let cases: Vec<SatisfiesCase> = serde_json::from_str(
            &fs::read_to_string(fixture("satisfies.json")).expect("read fixture"),
        )
        .expect("parse fixture");
        for case in cases {
            let got = satisfies(&case.version, &case.constraint).unwrap_or_else(|e| {
                panic!(
                    "satisfies({:?}, {:?}) failed: {e}",
                    case.version, case.constraint
                );
            });
            assert_eq!(
                got, case.satisfies,
                "satisfies({:?}, {:?})",
                case.version, case.constraint
            );
        }
    }
}
