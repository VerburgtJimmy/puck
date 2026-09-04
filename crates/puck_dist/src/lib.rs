//! Dist archive download, checksum verification, and extraction.

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod checksum;
mod extract;
mod fetch;

pub use checksum::{sha1_hex, sha256_hex, verify_shasum};
pub use extract::{ArchiveKind, extract_archive};
pub use fetch::{DownloadedDist, download};

/// Errors from dist fetch / extract.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("download failed for {url}: {message}")]
    Download { url: String, message: String },
    #[error("checksum mismatch for {url}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("unsupported archive type: {0}")]
    UnsupportedArchive(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
