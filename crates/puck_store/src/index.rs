//! Secondary lookup index: Composer dist identity -> content sha256.
//!
//! The store is content-addressed by sha256 of the archive bytes. Composer locks
//! carry a sha1 `shasum` (and always a URL). This index lets warm / offline
//! installs resolve a lock entry to an existing store object without downloading.

use crate::store::{Store, StoreError};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

/// Remember that `sha256` was fetched for the given Composer `shasum` (sha1) and/or URL.
pub fn remember(
    store: &Store,
    sha256: &str,
    shasum: Option<&str>,
    url: Option<&str>,
) -> Result<(), StoreError> {
    if let Some(s) = shasum.filter(|s| !s.is_empty()) {
        write_key(store, "sha1", s, sha256)?;
    }
    if let Some(u) = url.filter(|u| !u.is_empty()) {
        write_key(store, "url", &url_key(u), sha256)?;
    }
    Ok(())
}

/// Resolve a store sha256 from Composer dist identity, if the object is still present.
pub fn lookup(
    store: &Store,
    shasum: Option<&str>,
    url: Option<&str>,
) -> Result<Option<String>, StoreError> {
    if let Some(s) = shasum.filter(|s| !s.is_empty())
        && let Some(sha) = read_key(store, "sha1", s)?
        && store.contains(&sha)
    {
        return Ok(Some(sha));
    }
    if let Some(u) = url.filter(|u| !u.is_empty())
        && let Some(sha) = read_key(store, "url", &url_key(u))?
        && store.contains(&sha)
    {
        return Ok(Some(sha));
    }
    Ok(None)
}

fn index_dir(store: &Store, kind: &str) -> PathBuf {
    store.root().join(".index").join(kind)
}

fn key_path(store: &Store, kind: &str, key: &str) -> PathBuf {
    // Keep keys filesystem-safe (sha1/url-hash are already hex).
    index_dir(store, kind).join(key)
}

fn write_key(store: &Store, kind: &str, key: &str, sha256: &str) -> Result<(), StoreError> {
    let dir = index_dir(store, kind);
    fs::create_dir_all(&dir).map_err(|source| StoreError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    let path = key_path(store, kind, key);
    fs::write(&path, sha256.as_bytes()).map_err(|source| StoreError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn read_key(store: &Store, kind: &str, key: &str) -> Result<Option<String>, StoreError> {
    let path = key_path(store, kind, key);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|source| StoreError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let text = String::from_utf8(bytes).map_err(|e| StoreError::Io {
        path: path.display().to_string(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
    })?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

fn url_key(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push(format_hex_nibble(b >> 4));
        out.push(format_hex_nibble(b & 0xf));
    }
    out
}

fn format_hex_nibble(n: u8) -> char {
    char::from(b"0123456789abcdef"[n as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::put_archive;
    use puck_dist::{ArchiveKind, sha256_hex};
    use std::io::{Cursor, Write};
    use tempfile::tempdir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn sample_zip() -> (Vec<u8>, String) {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            let opts = SimpleFileOptions::default();
            zip.start_file("root/a.txt", opts).expect("start");
            zip.write_all(b"index").expect("write");
            zip.finish().expect("finish");
        }
        let bytes = cursor.into_inner();
        let hash = sha256_hex(&bytes);
        (bytes, hash)
    }

    #[test]
    fn remember_and_lookup_by_shasum_and_url() {
        let root = tempdir().expect("temp");
        let store = Store::new(root.path());
        let (bytes, hash) = sample_zip();
        put_archive(&store, &hash, &bytes, ArchiveKind::Zip).expect("put");

        remember(
            &store,
            &hash,
            Some("deadbeefcafebabe000000000000000000000000"),
            Some("https://example.test/pkg.zip"),
        )
        .expect("remember");

        let by_sha = lookup(
            &store,
            Some("deadbeefcafebabe000000000000000000000000"),
            None,
        )
        .expect("lookup")
        .expect("hit");
        assert_eq!(by_sha, hash);

        let by_url = lookup(&store, None, Some("https://example.test/pkg.zip"))
            .expect("lookup")
            .expect("hit");
        assert_eq!(by_url, hash);

        assert!(
            lookup(
                &store,
                Some("ffffffffffffffffffffffffffffffffffffffff"),
                None
            )
            .expect("miss")
            .is_none()
        );
    }
}
