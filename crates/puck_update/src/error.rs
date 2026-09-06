//! Errors for upgrade / notify.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("homebrew-managed binary at {0}; use `brew upgrade puck` instead")]
    HomebrewManaged(PathBuf),
    #[error("unsupported platform for self-upgrade: {0}")]
    UnsupportedTarget(String),
    #[error("manifest: {0}")]
    Manifest(String),
    #[error("download failed for {url}: {message}")]
    Download { url: String, message: String },
    #[error("sha256 mismatch for {name}: expected {expected}, got {actual}")]
    Sha256Mismatch {
        name: String,
        expected: String,
        actual: String,
    },
    #[error("minisign verification failed: {0}")]
    Minisign(String),
    #[error("archive: {0}")]
    Archive(String),
    #[error("no previous binary at {0} (nothing to roll back)")]
    NoPrevious(PathBuf),
    #[error("canary is not available in 0.1")]
    CanaryUnavailable,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, Error>;
