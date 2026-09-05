//! Packagist Composer 2 (`/p2/{vendor}/{package}.json`) metadata access.

use crate::replay::{ReplayError, ReplayMode, ReplayStore};
use std::path::{Path, PathBuf};

/// File layout used by `fixtures/registry/packagist/p2/`:
/// `vendor$name.json` for package `vendor/name`.
pub fn p2_filename(package: &str) -> String {
    format!("{}.json", package.to_ascii_lowercase().replace('/', "$"))
}

/// Resolve the on-disk path for a package under a registry root that contains `packagist/p2/`.
pub fn p2_path(registry_root: impl AsRef<Path>, package: &str) -> PathBuf {
    registry_root
        .as_ref()
        .join("packagist/p2")
        .join(p2_filename(package))
}

/// Read recorded Packagist v2 metadata bytes for `package` (`vendor/name`).
pub fn load_p2_metadata(
    registry_root: impl AsRef<Path>,
    package: &str,
) -> Result<Vec<u8>, ReplayError> {
    let path = p2_path(registry_root, package);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(bytes),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Err(ReplayError::Miss(package.to_ascii_lowercase()))
        }
        Err(source) => Err(ReplayError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Replay store rooted at `…/packagist/p2` using Packagist URL path keys.
pub fn p2_replay_store(p2_dir: impl Into<PathBuf>, mode: ReplayMode) -> ReplayStore {
    ReplayStore::new(p2_dir, mode)
}

/// Key used when recording via URL path (`p2/vendor/package.json`).
pub fn p2_url_key(package: &str) -> String {
    format!("p2/{}.json", package.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_uses_dollar() {
        assert_eq!(p2_filename("Laravel/Framework"), "laravel$framework.json");
    }

    #[test]
    fn loads_fixture_snapshot() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/registry");
        let bytes = load_p2_metadata(&root, "laravel/framework").expect("meta");
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.contains("laravel/framework"));
    }
}
