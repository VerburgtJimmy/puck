//! Port of `Composer\DependencyResolver` (Composer CDCL SAT solver).
//!
//! Decision: ADR 0001 in puck-notes - PubGrub litmus failed on Laravel `replace`.
//! Reference: composer/composer @ `85ae025` (see `fixtures/composer-DependencyResolver/COMPOSER_COMMIT.txt`).

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod decisions;
mod link;
mod metadata;
mod order;
mod package;
mod platform;
mod policy;
mod pool;
mod pool_builder;
mod problem;
mod request;
mod rule;
mod rule_set;
mod rule_set_generator;
mod solver;
mod transaction;
mod watch;

pub use decisions::Decisions;
pub use link::Link;
pub use metadata::{
    find_p2_version, package_from_composer_package, package_from_p2_version, packages_from_lock_json,
    packages_from_p2_json,
};
pub use order::{PackageIdSet, PresentMap};
pub use package::Package;
pub use platform::is_platform_package;
pub use policy::DefaultPolicy;
pub use pool::Pool;
pub use pool_builder::{ArrayRepository, PoolBuilder};
pub use problem::Problem;
pub use request::{Request, UpdateAllowTransitive};
pub use rule::{Rule, RuleReason, RuleType};
pub use rule_set::RuleSet;
pub use rule_set_generator::RuleSetGenerator;
pub use solver::{Solver, SolverProblems};
pub use transaction::{Operation, Transaction};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Version(#[from] puck_version::Error),
    #[error("solver bug: {0}")]
    SolverBug(String),
    #[error("{0}")]
    Unsolvable(SolverProblems),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Package id in a [`Pool`] (1-based, matching Composer).
pub type PackageId = u32;

/// SAT literal: positive = install package id, negative = do not install.
pub type Literal = i32;

#[cfg(test)]
mod litmus;
#[cfg(test)]
mod lock_identity;
#[cfg(test)]
mod solver_tests;
