//! Locate a PHP CLI binary for running user Composer scripts.
//!
//! Install/link paths never call PHP; this crate is only for script execution.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Find a usable `php` executable on `PATH` (or `PHP_BINARY` if set).
///
/// Returns `None` when no candidate exists or the candidate does not run.
pub fn find_php() -> Option<PathBuf> {
    if let Ok(explicit) = env::var("PHP_BINARY") {
        let path = PathBuf::from(explicit);
        if php_works(&path) {
            return Some(path);
        }
    }

    which("php").filter(|p| php_works(p))
}

/// Resolve `name` on `PATH` the way a shell `which` would.
pub fn which(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            for ext in ["exe", "bat", "cmd", "com"] {
                let with_ext = dir.join(format!("{name}.{ext}"));
                if is_executable(&with_ext) {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

fn php_works(path: &Path) -> bool {
    Command::new(path)
        .arg("-v")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_php_or_none() {
        // Environment-dependent; just ensure it does not panic.
        let _ = find_php();
    }
}
