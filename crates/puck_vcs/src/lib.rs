//! puck_vcs - git source fetch for `type: vcs` repositories and lock `source` installs.
//!
//! Shells out to `git` (Composer does the same; no libgit2). Subset of
//! `Composer\Repository\Vcs\GitDriver`: mirror clone/fetch, list tags/branches,
//! read `composer.json` at a ref, checkout a commit into a work tree.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

mod git;

pub use git::{
    GitPackageVersion, checkout_reference, default_vcs_cache_root, ensure_git_mirror,
    list_git_versions, read_composer_json_at,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("git failed: {0}")]
    Git(String),
}

pub type Result<T> = std::result::Result<T, Error>;
