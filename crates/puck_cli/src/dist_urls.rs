//! Distribution URL constants. Keep in sync with `install.sh` and `docs/distribution.md`.
//!
//! Canonical definitions live in `puck_update::urls`; this module re-exports them
//! for the CLI crate and unit tests.

#[allow(unused_imports)] // re-exported for docs / discoverability; upgrade uses puck_update directly
pub use puck_update::{
    DEFAULT_INSTALL_ROOT_NAME, INSTALL_MIRROR, MANIFEST_URL, MINISIGN_PUBLIC_KEY, REPO,
    STABLE_MIRROR,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minisign_pubkey_is_real() {
        assert!(
            !MINISIGN_PUBLIC_KEY.contains("REPLACE_ME"),
            "dist/minisign/minisign.pub must be a real public key"
        );
        assert!(
            MINISIGN_PUBLIC_KEY.lines().any(|l| l.starts_with("RW")),
            "expected minisign public key line starting with RW"
        );
    }

    #[test]
    fn manifest_url_points_at_github_releases() {
        assert!(MANIFEST_URL.contains("VerburgtJimmy/puck"));
        assert!(MANIFEST_URL.ends_with("/manifest.json"));
    }
}
