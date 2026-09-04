//! Store path helpers.

use std::path::PathBuf;

/// Default global store root: `~/.puck/store`.
pub fn default_store_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".puck").join("store")
}

/// Path for a content-addressed package: `<root>/<sha256>/`.
pub fn package_store_path(root: impl Into<PathBuf>, sha256: &str) -> PathBuf {
    root.into().join(sha256)
}
