//! Link a store object into `vendor/` (hardlink, reflink, or copy).

use std::fs;
use std::io;
use std::path::Path;

/// How a file was placed into the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Hardlink,
    Reflink,
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkResult {
    pub kind: LinkKind,
}

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("link failed from {from} to {to}: {source}")]
    Io {
        from: String,
        to: String,
        #[source]
        source: io::Error,
    },
}

/// Place `from` at `to`, preferring hardlink, then reflink, then copy.
///
/// Destination parent directories must already exist. Existing `to` is replaced.
pub fn link_file(from: &Path, to: &Path) -> Result<LinkResult, LinkError> {
    let map_err = |source: io::Error| LinkError::Io {
        from: from.display().to_string(),
        to: to.display().to_string(),
        source,
    };

    if to.exists() {
        fs::remove_file(to).map_err(map_err)?;
    }

    match fs::hard_link(from, to) {
        Ok(()) => {
            return Ok(LinkResult {
                kind: LinkKind::Hardlink,
            });
        }
        Err(err) if is_cross_device_or_unsupported(&err) => {}
        Err(err) => return Err(map_err(err)),
    }

    match try_reflink(from, to) {
        Ok(()) => {
            return Ok(LinkResult {
                kind: LinkKind::Reflink,
            });
        }
        Err(err) if is_cross_device_or_unsupported(&err) => {}
        Err(err) => return Err(map_err(err)),
    }

    fs::copy(from, to).map_err(map_err)?;
    Ok(LinkResult {
        kind: LinkKind::Copy,
    })
}

/// Attempt a COW/reflink copy without trying hardlink first.
///
/// Used by tests and by directory clone paths later.
pub fn reflink_file(from: &Path, to: &Path) -> Result<LinkResult, LinkError> {
    let map_err = |source: io::Error| LinkError::Io {
        from: from.display().to_string(),
        to: to.display().to_string(),
        source,
    };
    if to.exists() {
        fs::remove_file(to).map_err(map_err)?;
    }
    try_reflink(from, to).map_err(map_err)?;
    Ok(LinkResult {
        kind: LinkKind::Reflink,
    })
}

fn is_cross_device_or_unsupported(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::Unsupported
            | io::ErrorKind::CrossesDevices
            | io::ErrorKind::InvalidInput
            | io::ErrorKind::PermissionDenied
    ) || err.raw_os_error() == Some(18) // EXDEV
        || err.raw_os_error() == Some(45) // ENOTSUP (macOS)
        || err.raw_os_error() == Some(95) // EOPNOTSUPP/ENOTSUP (Linux)
        || err.raw_os_error() == Some(102) // EOPNOTSUPP (macOS)
}

fn try_reflink(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        clonefile_macos(from, to)
    }
    #[cfg(target_os = "linux")]
    {
        ficlone_linux(from, to)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (from, to);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "reflink not available on this platform",
        ))
    }
}

/// clonefile(2) on APFS. Unsafe: FFI to libc only; paths are validated CStrings.
#[cfg(target_os = "macos")]
fn clonefile_macos(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let from_c = CString::new(from.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let to_c = CString::new(to.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    // SAFETY: both pointers are valid NUL-terminated paths from CString.
    // clonefile flags=0 copies metadata and file contents via COW on APFS.
    let rc = unsafe { libc::clonefile(from_c.as_ptr(), to_c.as_ptr(), 0) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `ioctl(FICLONE)` COW clone on btrfs/xfs (and other supporting filesystems).
///
/// Creates the destination file, then clones extents from `from`. Falls through
/// to the caller on EXDEV / EOPNOTSUPP so `link_file` can copy.
#[cfg(target_os = "linux")]
fn ficlone_linux(from: &Path, to: &Path) -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    let src = fs::File::open(from)?;
    let dst = OpenOptions::new().write(true).create_new(true).open(to)?;

    // SAFETY: both fds are open files we own; FICLONE is a well-defined ioctl.
    let rc = unsafe { libc::ioctl(dst.as_raw_fd(), libc::FICLONE, src.as_raw_fd()) };
    if rc == 0 {
        Ok(())
    } else {
        let err = io::Error::last_os_error();
        let _ = fs::remove_file(to);
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn hardlinks_on_same_volume() {
        let dir = tempdir().expect("tempdir");
        let from = dir.path().join("src.txt");
        let to = dir.path().join("dst.txt");
        {
            let mut f = fs::File::create(&from).expect("create");
            f.write_all(b"puck").expect("write");
        }
        let result = link_file(&from, &to).expect("link");
        assert_eq!(result.kind, LinkKind::Hardlink);
        assert_eq!(fs::read_to_string(&to).expect("read"), "puck");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clonefile_on_apfs() {
        let dir = tempdir().expect("tempdir");
        let from = dir.path().join("src.txt");
        let to = dir.path().join("dst.txt");
        {
            let mut f = fs::File::create(&from).expect("create");
            f.write_all(b"puck-clone").expect("write");
        }
        match reflink_file(&from, &to) {
            Ok(result) => {
                assert_eq!(result.kind, LinkKind::Reflink);
                assert_eq!(fs::read_to_string(&to).expect("read"), "puck-clone");
            }
            Err(err) => {
                // Non-APFS volumes (or sandboxed FS) may reject clonefile.
                let msg = err.to_string();
                eprintln!("clonefile unavailable in this environment: {msg}");
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ficlone_when_supported() {
        let dir = tempdir().expect("tempdir");
        let from = dir.path().join("src.txt");
        let to = dir.path().join("dst.txt");
        {
            let mut f = fs::File::create(&from).expect("create");
            f.write_all(b"puck-ficlone").expect("write");
        }
        match reflink_file(&from, &to) {
            Ok(result) => {
                assert_eq!(result.kind, LinkKind::Reflink);
                assert_eq!(fs::read_to_string(&to).expect("read"), "puck-ficlone");
            }
            Err(err) => {
                // ext4 without reflink, overlayfs, etc.
                eprintln!("FICLONE unavailable in this environment: {err}");
            }
        }
    }
}
