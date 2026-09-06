//! Packagist / Satis client with optional VCR-style replay and live p2 / composer-repo fetch.

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod audit;
mod composer_repo;
mod loader;
mod packagist;
mod replay;

pub use audit::{AdvisoryHit, SecurityAdvisory, advisories_for_package, find_advisory_hits};
pub use composer_repo::{
    canonicalize_metadata_url, default_packagist_url, is_packagist_org_url,
    metadata_url_for_package, metadata_url_template, packages_json_url, parse_repositories,
    resolve_package_metadata_url, RepositoryConfig,
};
pub use loader::{load_p2_optional, P2Loader};
pub use packagist::{load_p2_metadata, p2_filename, p2_path, p2_replay_store, p2_url_key};
pub use replay::{ReplayError, ReplayMode, ReplayStore};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Replay(#[from] ReplayError),
}

pub type Result<T> = std::result::Result<T, Error>;
