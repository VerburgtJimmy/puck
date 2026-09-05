//! Port of `Composer\DependencyResolver` (Composer CDCL SAT solver).
//!
//! Decision: ADR 0001 in puck-notes - PubGrub litmus failed on Laravel `replace`.
//! Reference: composer/composer @ `85ae025` (see `fixtures/composer-DependencyResolver/COMPOSER_COMMIT.txt`).

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod decisions;
mod link;
mod package;
mod pool;
mod request;
mod rule;
mod rule_set;
mod rule_set_generator;

pub use decisions::Decisions;
pub use link::Link;
pub use package::Package;
pub use pool::Pool;
pub use request::{Request, UpdateAllowTransitive};
pub use rule::{Rule, RuleReason, RuleType};
pub use rule_set::RuleSet;
pub use rule_set_generator::RuleSetGenerator;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Version(#[from] puck_version::Error),
    #[error("solver bug: {0}")]
    SolverBug(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Package id in a [`Pool`] (1-based, matching Composer).
pub type PackageId = u32;

/// SAT literal: positive = install package id, negative = do not install.
pub type Literal = i32;

#[cfg(test)]
mod litmus;
