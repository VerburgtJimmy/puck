//! Packagist / Satis client with optional VCR-style replay.

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod audit;
mod packagist;
mod replay;

pub use audit::{AdvisoryHit, SecurityAdvisory, advisories_for_package, find_advisory_hits};
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
