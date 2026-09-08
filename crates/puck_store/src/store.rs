//! Content-addressable store: extract once, link many times.

use crate::link::link_file;
use crate::paths::{default_store_root, package_store_path};
use puck_dist::{ArchiveKind, extract_archive};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Dist(#[from] puck_dist::Error),
    #[error(transparent)]
    Link(#[from] crate::LinkError),
}

/// On-disk content-addressable package store.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn default_global() -> Self {
        Self::new(default_store_root())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn package_dir(&self, sha256: &str) -> PathBuf {
        package_store_path(&self.root, sha256)
    }

    pub fn contains(&self, sha256: &str) -> bool {
        self.package_dir(sha256).join(".puck-ok").is_file()
    }
}

/// Extract archive bytes into the store under `sha256` if not already present.
///
/// Returns the package directory path. Extracted files are made read-only on
/// Unix (`0444`) so accidental edits under `vendor/` (hardlinked) do not silently
/// corrupt the shared store. Package bins are made executable again later by
/// `puck_install::bins` via those same hardlinks.
pub fn put_archive(
    store: &Store,
    sha256: &str,
    bytes: &[u8],
    kind: ArchiveKind,
) -> Result<PathBuf, StoreError> {
    let dest = store.package_dir(sha256);
    if store.contains(sha256) {
        #[cfg(unix)]
        ensure_package_readonly(&dest)?;
        return Ok(dest);
    }

    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|source| StoreError::Io {
            path: dest.display().to_string(),
            source,
        })?;
    }
    fs::create_dir_all(&dest).map_err(|source| StoreError::Io {
        path: dest.display().to_string(),
        source,
    })?;

    extract_archive(bytes, kind, &dest)?;

    let marker = dest.join(".puck-ok");
    fs::write(&marker, b"ok").map_err(|source| StoreError::Io {
        path: marker.display().to_string(),
        source,
    })?;

    #[cfg(unix)]
    ensure_package_readonly(&dest)?;

    Ok(dest)
}

#[cfg(unix)]
fn ensure_package_readonly(root: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    fn walk(path: &Path) -> Result<(), StoreError> {
        let meta = fs::symlink_metadata(path).map_err(|source| StoreError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let ft = meta.file_type();
        if ft.is_symlink() {
            return Ok(());
        }
        if ft.is_dir() {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).map_err(|source| StoreError::Io {
                path: path.display().to_string(),
                source,
            })?;
            for entry in fs::read_dir(path).map_err(|source| StoreError::Io {
                path: path.display().to_string(),
                source,
            })? {
                let entry = entry.map_err(|source| StoreError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
                walk(&entry.path())?;
            }
        } else if ft.is_file() {
            let mut perms = meta.permissions();
            perms.set_mode(0o444);
            fs::set_permissions(path, perms).map_err(|source| StoreError::Io {
                path: path.display().to_string(),
                source,
            })?;
        }
        Ok(())
    }
    walk(root)
}

/// Link every file from `from_dir` into `to_dir` (recursive), preferring hardlink/reflink.
pub fn link_tree(from_dir: &Path, to_dir: &Path) -> Result<usize, StoreError> {
    fs::create_dir_all(to_dir).map_err(|source| StoreError::Io {
        path: to_dir.display().to_string(),
        source,
    })?;
    let mut linked = 0usize;
    link_tree_inner(from_dir, to_dir, &mut linked)?;
    Ok(linked)
}

fn link_tree_inner(from_dir: &Path, to_dir: &Path, linked: &mut usize) -> Result<(), StoreError> {
    for entry in fs::read_dir(from_dir).map_err(|source| StoreError::Io {
        path: from_dir.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| StoreError::Io {
            path: from_dir.display().to_string(),
            source,
        })?;
        let name = entry.file_name();
        if name == ".puck-ok" {
            continue;
        }
        let from = entry.path();
        let to = to_dir.join(&name);
        let ft = entry.file_type().map_err(|source| StoreError::Io {
            path: from.display().to_string(),
            source,
        })?;
        if ft.is_dir() {
            fs::create_dir_all(&to).map_err(|source| StoreError::Io {
                path: to.display().to_string(),
                source,
            })?;
            link_tree_inner(&from, &to, linked)?;
        } else if ft.is_file() {
            link_file(&from, &to)?;
            *linked += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use puck_dist::sha256_hex;
    use std::io::{Cursor, Write};
    use tempfile::tempdir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    #[test]
    fn put_and_reuse_archive() {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            let opts = SimpleFileOptions::default();
            zip.start_file("root/a.txt", opts).expect("start");
            zip.write_all(b"store").expect("write");
            zip.finish().expect("finish");
        }
        let bytes = cursor.into_inner();
        let hash = sha256_hex(&bytes);
        let root = tempdir().expect("temp");
        let store = Store::new(root.path());

        let dir1 = put_archive(&store, &hash, &bytes, ArchiveKind::Zip).expect("put");
        assert!(dir1.join("a.txt").is_file());
        assert!(store.contains(&hash));

        let dir2 = put_archive(&store, &hash, &bytes, ArchiveKind::Zip).expect("reuse");
        assert_eq!(dir1, dir2);
    }
}
