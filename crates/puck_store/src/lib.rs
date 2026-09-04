//! Content-addressable package store and vendor linking.
//!
//! `link` uses a thin FFI shim for clonefile(2) on macOS (documented unsafe).

#![warn(clippy::unwrap_used)]

mod link;

pub use link::{LinkError, LinkKind, LinkResult, link_file, reflink_file};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Link(#[from] LinkError),
}

pub type Result<T> = std::result::Result<T, Error>;
