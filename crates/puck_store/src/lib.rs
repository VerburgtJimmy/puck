//! Content-addressable package store and vendor linking.
//!
//! `link` uses a thin FFI shim for clonefile(2) on macOS (documented unsafe).

#![warn(clippy::unwrap_used)]

mod index;
mod link;
mod paths;
mod store;

pub use index::{lookup, remember};
pub use link::{LinkError, LinkKind, LinkResult, link_file, reflink_file};
pub use paths::{default_store_root, package_store_path};
pub use store::{Store, StoreError, link_tree, put_archive};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Link(#[from] LinkError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub type Result<T> = std::result::Result<T, Error>;
