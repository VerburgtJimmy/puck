//! Packagist / Satis client with optional VCR-style replay.

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod replay;

pub use replay::{ReplayError, ReplayMode, ReplayStore};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Replay(#[from] ReplayError),
}

pub type Result<T> = std::result::Result<T, Error>;
