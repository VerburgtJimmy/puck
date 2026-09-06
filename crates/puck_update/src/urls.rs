//! Distribution URL constants. Keep in sync with `install.sh` and `docs/distribution.md`.

/// GitHub Releases manifest — source of truth for `puck upgrade` / install.
pub const MANIFEST_URL: &str =
    "https://github.com/VerburgtJimmy/puck/releases/latest/download/manifest.json";

/// GitHub repository (`owner/name`).
pub const REPO: &str = "VerburgtJimmy/puck";

/// Optional site mirror of install.sh (never the upgrade source of truth).
pub const INSTALL_MIRROR: &str = "https://puck.jimmyverburgt.com/install";

/// Optional site mirror of stable channel JSON (never the upgrade source of truth).
pub const STABLE_MIRROR: &str = "https://puck.jimmyverburgt.com/releases/stable.json";

/// Default install root name under `$HOME`.
pub const DEFAULT_INSTALL_ROOT_NAME: &str = ".puck";

/// minisign public key file contents (untrusted comment + key line).
pub const MINISIGN_PUBLIC_KEY: &str = include_str!("../../../dist/minisign/minisign.pub");

/// Release asset / tag manifest URL for a pinned version (`0.1.0` or `v0.1.0`).
pub fn manifest_url_for_version(version: &str) -> String {
    let tag = if version.starts_with('v') {
        version.to_owned()
    } else {
        format!("v{version}")
    };
    format!("https://github.com/{REPO}/releases/download/{tag}/manifest.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minisign_pubkey_is_real() {
        assert!(!MINISIGN_PUBLIC_KEY.contains("REPLACE_ME"));
        assert!(MINISIGN_PUBLIC_KEY.lines().any(|l| l.starts_with("RW")));
    }

    #[test]
    fn pinned_manifest_url() {
        assert_eq!(
            manifest_url_for_version("0.1.0"),
            "https://github.com/VerburgtJimmy/puck/releases/download/v0.1.0/manifest.json"
        );
        assert_eq!(
            manifest_url_for_version("v0.2.0"),
            "https://github.com/VerburgtJimmy/puck/releases/download/v0.2.0/manifest.json"
        );
    }
}
