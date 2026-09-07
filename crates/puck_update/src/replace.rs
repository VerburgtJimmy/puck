//! Atomic replace of the running binary; keep `puck.previous` under `~/.puck/bin`.

use crate::error::{Error, Result};
use crate::urls::DEFAULT_INSTALL_ROOT_NAME;
use std::path::{Path, PathBuf};

pub const PREVIOUS_NAME: &str = "puck.previous";

/// Default `~/.puck` root.
pub fn default_puck_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(DEFAULT_INSTALL_ROOT_NAME)
}

pub fn default_bin_dir() -> PathBuf {
    default_puck_root().join("bin")
}

pub fn default_cache_dir() -> PathBuf {
    default_puck_root().join("cache")
}

/// Whether `exe` lives under `~/.puck/bin` (or a custom root's `bin`).
pub fn is_under_puck_bin(exe: &Path) -> bool {
    let bin = default_bin_dir();
    if let Ok(canon_exe) = exe.canonicalize() {
        if let Ok(canon_bin) = bin.canonicalize() {
            if canon_exe.parent() == Some(canon_bin.as_path()) {
                return true;
            }
        }
    }
    exe.parent() == Some(bin.as_path())
}

/// Atomically replace `current_exe` with bytes from `new_bin`.
/// When under `~/.puck/bin`, moves the old binary to `puck.previous`.
pub fn atomic_replace(current_exe: &Path, new_bin: &Path) -> Result<()> {
    let parent = current_exe.parent().ok_or_else(|| {
        Error::Message(format!(
            "current executable has no parent: {}",
            current_exe.display()
        ))
    })?;
    std::fs::create_dir_all(parent)?;

    let staging = parent.join(format!(
        ".puck.new.{}{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    std::fs::copy(new_bin, &staging)?;
    set_executable(&staging)?;

    let keep_previous = is_under_puck_bin(current_exe)
        || current_exe
            .file_name()
            .is_some_and(|n| n == "puck" || n == "puck.exe");

    if keep_previous && current_exe.exists() {
        let previous = parent.join(PREVIOUS_NAME);
        // Replace any existing previous.
        let _ = std::fs::remove_file(&previous);
        std::fs::rename(current_exe, &previous)?;
    } else if current_exe.exists() {
        // Still free the path via rename to a temp then remove, so the running
        // image keeps its inode.
        let trash = parent.join(format!(".puck.old.{}", std::process::id()));
        let _ = std::fs::remove_file(&trash);
        std::fs::rename(current_exe, &trash)?;
        let _ = std::fs::remove_file(&trash);
    }

    std::fs::rename(&staging, current_exe)?;
    set_executable(current_exe)?;
    Ok(())
}

/// Swap `puck.previous` back onto the current executable path.
pub fn rollback(current_exe: &Path) -> Result<()> {
    let parent = current_exe.parent().ok_or_else(|| {
        Error::Message(format!(
            "current executable has no parent: {}",
            current_exe.display()
        ))
    })?;
    let previous = parent.join(PREVIOUS_NAME);
    if !previous.is_file() {
        return Err(Error::NoPrevious(previous));
    }

    let staging = parent.join(format!(".puck.rollback.{}", std::process::id()));
    let _ = std::fs::remove_file(&staging);
    if current_exe.exists() {
        std::fs::rename(current_exe, &staging)?;
    }
    std::fs::rename(&previous, current_exe)?;
    // Keep the displaced current as the new previous (optional undo of rollback).
    if staging.exists() {
        std::fs::rename(&staging, &previous)?;
    }
    set_executable(current_exe)?;
    Ok(())
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn atomic_replace_keeps_previous() {
        let dir = tempfile::tempdir().unwrap();
        // Pretend this is ~/.puck/bin by using HOME override via path shape:
        // is_under_puck_bin checks default_bin_dir; for unit test we still
        // keep previous when file_name is puck.
        let current = dir.path().join("puck");
        let newer = dir.path().join("newer");
        std::fs::write(&current, b"old").unwrap();
        std::fs::write(&newer, b"new").unwrap();
        atomic_replace(&current, &newer).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), b"new");
        assert_eq!(
            std::fs::read(dir.path().join(PREVIOUS_NAME)).unwrap(),
            b"old"
        );
    }

    #[test]
    fn rollback_swaps() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("puck");
        let previous = dir.path().join(PREVIOUS_NAME);
        std::fs::write(&current, b"v2").unwrap();
        std::fs::write(&previous, b"v1").unwrap();
        rollback(&current).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), b"v1");
        assert_eq!(std::fs::read(&previous).unwrap(), b"v2");
    }

    #[test]
    fn rollback_missing_errors() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("puck");
        std::fs::File::create(&current)
            .unwrap()
            .write_all(b"x")
            .unwrap();
        let err = rollback(&current).unwrap_err();
        assert!(matches!(err, Error::NoPrevious(_)));
    }
}
