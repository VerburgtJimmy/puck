//! Install planning: diff a lock file against the current vendor state.

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod installed;
mod plan;

pub use installed::{InstalledPackage, InstalledState, read_installed};
pub use plan::{InstallAction, InstallOptions, InstallPlan, PlannedPackage, plan_install};

/// Errors from install planning.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Lock(#[from] puck_lock::Error),
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse installed.json: {0}")]
    InstalledParse(String),
}

pub type Result<T> = std::result::Result<T, Error>;
