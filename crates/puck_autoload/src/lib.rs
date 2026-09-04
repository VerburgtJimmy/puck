//! Composer-compatible autoload dump (`vendor/autoload.php` + `vendor/composer/*`).
//!
//! M1 generates PSR-4 / PSR-0 / files maps without classmap scanning (`optimize`
//! is accepted but ignored until a later milestone). Files autoload identifiers
//! use `md5("{package}:{path}")`, matching Composer’s `getFileIdentifier`.

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod collect;
mod generate;
mod php;
mod platform_check;

use collect::CollectedAutoloads;
use puck_lock::LockFile;
use puck_manifest::Manifest;
use std::path::Path;

/// Options for [`dump`].
#[derive(Debug, Clone, Copy, Default)]
pub struct DumpOptions {
    /// When true, Composer would scan PSR dirs into the classmap. Ignored for now.
    pub optimize: bool,
    /// Class map is authoritative (no PSR fallback). Applied even without optimize.
    pub authoritative: bool,
    /// Exclude `packages-dev` and root `autoload-dev`.
    pub no_dev: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Write Composer-shaped autoload files under `{project_root}/vendor/`.
pub fn dump(
    project_root: &Path,
    lock: &LockFile,
    manifest: Option<&Manifest>,
    options: DumpOptions,
) -> Result<()> {
    let _ = options.optimize; // classmap scan lands later
    let collected = CollectedAutoloads::from_lock_and_manifest(lock, manifest, options.no_dev);
    let suffix = autoload_suffix(lock);
    generate::write_autoload_files(
        project_root,
        &collected,
        &suffix,
        options.authoritative,
        lock,
        manifest,
        options.no_dev,
    )
}

/// Stable suffix: `md5(content-hash)` hex, or md5 of package names if hash empty.
fn autoload_suffix(lock: &LockFile) -> String {
    if !lock.content_hash.is_empty() {
        return md5_hex(lock.content_hash.as_bytes());
    }
    let mut names: Vec<&str> = lock
        .packages
        .iter()
        .chain(lock.packages_dev.iter())
        .map(|p| p.name.as_str())
        .collect();
    names.sort_unstable();
    md5_hex(names.join("\n").as_bytes())
}

fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let digest = Md5::digest(bytes);
    let mut out = String::with_capacity(32);
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
    use puck_lock::LockFile;
    use puck_manifest::Manifest;
    use std::fs;
    use std::str::FromStr;

    #[test]
    fn suffix_from_content_hash() {
        let lock = LockFile::from_str(
            r#"{
                "content-hash": "cda2add6397422fb1a2eb5b9207f1575",
                "packages": [],
                "packages-dev": []
            }"#,
        )
        .expect("lock");
        assert_eq!(
            autoload_suffix(&lock),
            "72565bddcbd249d9a96a8d790293c736"
        );
    }

    #[test]
    fn dump_writes_expected_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock = LockFile::from_str(
            r#"{
                "content-hash": "abc",
                "packages": [{
                    "name": "acme/lib",
                    "version": "1.0.0",
                    "autoload": {
                        "psr-4": { "Acme\\Lib\\": "src/" },
                        "files": ["src/helpers.php"]
                    }
                }],
                "packages-dev": []
            }"#,
        )
        .expect("lock");
        let manifest = Manifest::from_value(serde_json::json!({
            "name": "acme/app",
            "require": { "php": "^8.3" },
            "autoload": { "psr-4": { "App\\": "app/" } },
            "autoload-dev": { "psr-4": { "Tests\\": "tests/" } }
        }))
        .expect("manifest");

        dump(
            dir.path(),
            &lock,
            Some(&manifest),
            DumpOptions {
                optimize: false,
                authoritative: false,
                no_dev: true,
            },
        )
        .expect("dump");

        let vendor = dir.path().join("vendor");
        assert!(vendor.join("autoload.php").is_file());
        assert!(vendor.join("composer/ClassLoader.php").is_file());
        assert!(vendor.join("composer/InstalledVersions.php").is_file());
        assert!(vendor.join("composer/LICENSE").is_file());
        assert!(vendor.join("composer/autoload_real.php").is_file());
        assert!(vendor.join("composer/autoload_static.php").is_file());
        assert!(vendor.join("composer/autoload_psr4.php").is_file());
        assert!(vendor.join("composer/autoload_namespaces.php").is_file());
        assert!(vendor.join("composer/autoload_classmap.php").is_file());
        assert!(vendor.join("composer/autoload_files.php").is_file());
        assert!(vendor.join("composer/platform_check.php").is_file());

        let classmap =
            fs::read_to_string(vendor.join("composer/autoload_classmap.php")).expect("classmap");
        assert!(classmap.contains("Composer\\\\InstalledVersions"));
        assert!(classmap.contains("InstalledVersions.php"));

        let platform =
            fs::read_to_string(vendor.join("composer/platform_check.php")).expect("platform");
        assert!(platform.contains("PHP_VERSION_ID >= 80300"));

        let real =
            fs::read_to_string(vendor.join("composer/autoload_real.php")).expect("real");
        assert!(real.contains("require __DIR__ . '/platform_check.php';"));

        let psr4 = fs::read_to_string(vendor.join("composer/autoload_psr4.php")).expect("psr4");
        assert!(psr4.contains("Acme\\\\Lib\\\\"));
        assert!(psr4.contains("$vendorDir . '/acme/lib/src'"));
        assert!(psr4.contains("App\\\\"));
        assert!(psr4.contains("$baseDir . '/app'"));
        assert!(!psr4.contains("Tests\\\\"));

        let files = fs::read_to_string(vendor.join("composer/autoload_files.php")).expect("files");
        let id = md5_hex(b"acme/lib:src/helpers.php");
        assert!(files.contains(&id));
        assert!(files.contains("$vendorDir . '/acme/lib/src/helpers.php'"));

        let autoload = fs::read_to_string(vendor.join("autoload.php")).expect("autoload");
        let suffix = autoload_suffix(&lock);
        assert!(autoload.contains(&format!("ComposerAutoloaderInit{suffix}")));
    }

    #[test]
    fn file_identifier_matches_composer() {
        assert_eq!(
            md5_hex(b"symfony/deprecation-contracts:function.php"),
            "6e3fae29631ef280660b3cdad06f25a8"
        );
    }
}
