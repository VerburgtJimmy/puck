//! composer.lock parsing and Composer content-hash.
//!
//! Reads Composer 2 lock files into typed structures and computes
//! `content-hash` matching `Composer\Package\Locker::getContentHash`.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

mod array_dumper;
mod content_hash;
mod lock;
mod write;

pub use array_dumper::{dump_lock_package_from_p2, dump_lock_package_from_path};
pub use content_hash::content_hash;
pub use lock::{Abandoned, Dist, LockFile, LockedPackage, Source, abandoned_warnings};
pub use write::{
    LockWriteInput, PLUGIN_API_VERSION, build_lock_document, format_lock_package,
    sort_lock_packages,
};

/// Errors from lock parsing and content-hash computation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to parse composer.lock: {0}")]
    Parse(String),
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to compute content-hash: {0}")]
    ContentHash(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;
    use std::str::FromStr;

    fn workspace_fixture(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(rel)
    }

    #[test]
    fn parse_minimal_lock() {
        let path = workspace_fixture("minimal/composer.lock");
        let lock = LockFile::from_path(&path).expect("parse minimal lock");
        assert_eq!(lock.content_hash, "5990a31168e72d02970542ea15afa375");
        assert!(lock.packages.is_empty());
        assert!(lock.packages_dev.is_empty());
        assert_eq!(lock.minimum_stability.as_deref(), Some("stable"));
        assert_eq!(lock.prefer_stable, Some(false));
        assert_eq!(lock.prefer_lowest, Some(false));
        assert_eq!(lock.plugin_api_version.as_deref(), Some("2.9.0"));
        assert!(lock.extra.contains_key("_readme"));
    }

    #[test]
    fn round_trip_minimal_lock_fields() {
        let path = workspace_fixture("minimal/composer.lock");
        let original = fs::read_to_string(&path).expect("read");
        let lock = LockFile::from_str(&original).expect("parse");
        let encoded = serde_json::to_value(&lock).expect("to value");
        let again: LockFile = serde_json::from_value(encoded).expect("reparse");
        assert_eq!(again.content_hash, lock.content_hash);
        assert_eq!(again.packages.len(), lock.packages.len());
        assert_eq!(again.packages_dev.len(), lock.packages_dev.len());
        assert_eq!(again.plugin_api_version, lock.plugin_api_version);
        assert_eq!(again.minimum_stability, lock.minimum_stability);
    }

    #[test]
    fn content_hash_matches_minimal_lock() {
        let json_path = workspace_fixture("minimal/composer.json");
        let lock_path = workspace_fixture("minimal/composer.lock");
        let json = fs::read_to_string(&json_path).expect("read composer.json");
        let lock = LockFile::from_path(&lock_path).expect("parse lock");
        let hash = content_hash(&json).expect("content_hash");
        assert_eq!(hash, lock.content_hash);
    }

    #[test]
    fn parse_laravel_skeleton_if_valid() {
        let path = workspace_fixture("laravel-skeleton/composer.lock");
        if !path.is_file() {
            return;
        }
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return,
        };
        let value: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => return,
        };
        let Some(obj) = value.as_object() else {
            return;
        };
        let packages = obj
            .get("packages")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        let packages_dev = obj
            .get("packages-dev")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        if packages + packages_dev == 0 {
            return;
        }

        let lock = LockFile::from_str(&text).expect("parse laravel lock");
        assert!(!lock.content_hash.is_empty());
        assert!(lock.packages.len() + lock.packages_dev.len() > 0);

        let json_path = workspace_fixture("laravel-skeleton/composer.json");
        let json = fs::read_to_string(&json_path).expect("read laravel composer.json");
        let hash = content_hash(&json).expect("content_hash");
        assert_eq!(
            hash, lock.content_hash,
            "content-hash must match Composer lock for laravel-skeleton"
        );
    }
}
