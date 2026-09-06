//! Garbage-collect incomplete and unreferenced store objects.
//!
//! Layout reminder:
//! - `~/.puck/store/<sha256>/` - extracted package (`\.puck-ok` marker)
//! - `~/.puck/store/.index/sha1/<sha1>` / `.index/url/<url-hash>` - maps to sha256
//!
//! GC removes:
//! 1. package dirs without `.puck-ok` (failed extracts)
//! 2. package dirs not referenced by any index key
//! 3. index keys whose sha256 target is missing
//!
//! It does not track which projects still use a package; the index is the
//! reference set. After GC, a later install may re-download if the index
//! entry was pruned with the object.

use crate::store::{Store, StoreError};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Summary of a GC pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub removed_packages: usize,
    pub removed_index_keys: usize,
    pub kept_packages: usize,
}

/// Run garbage collection on `store`.
pub fn gc(store: &Store) -> Result<GcReport, StoreError> {
    let root = store.root();
    if !root.is_dir() {
        return Ok(GcReport::default());
    }

    let mut referenced = HashSet::new();
    let mut stale_index: Vec<PathBuf> = Vec::new();

    collect_index(store, "sha1", &mut referenced, &mut stale_index)?;
    collect_index(store, "url", &mut referenced, &mut stale_index)?;

    let mut report = GcReport::default();

    for path in stale_index {
        fs::remove_file(&path).map_err(|source| StoreError::Io {
            path: path.display().to_string(),
            source,
        })?;
        report.removed_index_keys += 1;
    }

    for entry in fs::read_dir(root).map_err(|source| StoreError::Io {
        path: root.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| StoreError::Io {
            path: root.display().to_string(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if !is_sha256_dir_name(&name) {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let ok = path.join(".puck-ok").is_file();
        let keep = ok && referenced.contains(name.as_ref());
        if keep {
            report.kept_packages += 1;
            continue;
        }

        fs::remove_dir_all(&path).map_err(|source| StoreError::Io {
            path: path.display().to_string(),
            source,
        })?;
        report.removed_packages += 1;

        // Drop index keys that pointed at this object (if any remain).
        // collect_index already removed stale targets; orphans have no keys.
    }

    Ok(report)
}

fn collect_index(
    store: &Store,
    kind: &str,
    referenced: &mut HashSet<String>,
    stale: &mut Vec<PathBuf>,
) -> Result<(), StoreError> {
    let dir = store.root().join(".index").join(kind);
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&dir).map_err(|source| StoreError::Io {
        path: dir.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| StoreError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).map_err(|source| StoreError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let Ok(text) = String::from_utf8(bytes) else {
            stale.push(path);
            continue;
        };
        let sha = text.trim();
        if sha.is_empty() || !store.contains(sha) {
            stale.push(path);
            continue;
        }
        referenced.insert(sha.to_owned());
    }
    Ok(())
}

fn is_sha256_dir_name(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::remember;
    use crate::store::put_archive;
    use puck_dist::{ArchiveKind, sha256_hex};
    use std::io::{Cursor, Write};
    use tempfile::tempdir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn zip_bytes(label: &[u8]) -> (Vec<u8>, String) {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            let opts = SimpleFileOptions::default();
            zip.start_file("root/a.txt", opts).expect("start");
            zip.write_all(label).expect("write");
            zip.finish().expect("finish");
        }
        let bytes = cursor.into_inner();
        let hash = sha256_hex(&bytes);
        (bytes, hash)
    }

    #[test]
    fn gc_removes_orphan_and_incomplete() {
        let root = tempdir().expect("temp");
        let store = Store::new(root.path());
        let (bytes, hash) = zip_bytes(b"keep");
        put_archive(&store, &hash, &bytes, ArchiveKind::Zip).expect("put");
        remember(
            &store,
            &hash,
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            None,
        )
        .expect("remember");

        // Orphan complete package (no index).
        let (orphan_bytes, orphan_hash) = zip_bytes(b"orphan");
        put_archive(&store, &orphan_hash, &orphan_bytes, ArchiveKind::Zip).expect("orphan");

        // Incomplete package dir.
        let incomplete =
            store.package_dir("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        fs::create_dir_all(&incomplete).expect("mkdir");
        fs::write(incomplete.join("x.txt"), b"x").expect("write");

        // Stale index key.
        let stale = store
            .root()
            .join(".index/sha1/cccccccccccccccccccccccccccccccccccccccc");
        fs::create_dir_all(stale.parent().expect("parent")).expect("mkdir");
        fs::write(
            &stale,
            b"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        )
        .expect("stale");

        let report = gc(&store).expect("gc");
        assert_eq!(report.kept_packages, 1);
        assert_eq!(report.removed_packages, 2);
        assert_eq!(report.removed_index_keys, 1);
        assert!(store.contains(&hash));
        assert!(!store.contains(&orphan_hash));
        assert!(!incomplete.exists());
        assert!(!stale.exists());
    }
}
