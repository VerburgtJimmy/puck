//! Distribution URL constants. Keep in sync with `install.sh` and `docs/distribution.md`.

#![allow(dead_code)] // consumed by upgrade/notifications in a later change

/// GitHub Releases manifest — source of truth for `puck upgrade` / install.
pub const MANIFEST_URL: &str =
    "https://github.com/VerburgtJimmy/puck/releases/latest/download/manifest.json";

/// GitHub repository (`owner/name`).
pub const REPO: &str = "VerburgtJimmy/puck";

/// Optional site mirror of install.sh (never the upgrade source of truth).
pub const INSTALL_MIRROR: &str = "https://puck.jimmyverburgt.com/install";

/// Optional site mirror of stable channel JSON (never the upgrade source of truth).
pub const STABLE_MIRROR: &str = "https://puck.jimmyverburgt.com/releases/stable.json";

/// Default install root (`~/.puck` when expanded by the install script).
pub const DEFAULT_INSTALL_ROOT_NAME: &str = ".puck";

/// minisign public key file contents (untrusted comment + key line).
pub const MINISIGN_PUBLIC_KEY: &str = include_str!("../../../dist/minisign/minisign.pub");

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
