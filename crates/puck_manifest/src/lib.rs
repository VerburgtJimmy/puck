//! composer.json parsing and Composer-compatible normalisation.
//!
//! Name and type lowercasing follow `Composer\Package\BasePackage` and
//! `ArrayLoader::configureObject` (Composer 2.8.x).

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod manifest;
mod edit;

pub use edit::{
    add_requirement, add_requirement_to_file, remove_requirement, sort_packages_enabled,
    PackageRequirement,
};
pub use manifest::{Autoload, Manifest, PackageLinks};

use std::path::Path;
use std::str::FromStr;

/// Errors from reading or normalising a manifest.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to parse composer.json: {0}")]
    Parse(String),
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("composer.json root must be an object")]
    RootNotObject,
    #[error("composer.json is missing a package name")]
    MissingName,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Manifest {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        text.parse()
    }
}

impl FromStr for Manifest {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
        Self::from_value(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(rel)
    }

    #[test]
    fn parses_laravel_skeleton() {
        let m = Manifest::from_path(fixture("laravel-skeleton/composer.json")).expect("parse");
        assert_eq!(m.name, "laravel/laravel");
        assert_eq!(m.pretty_name, "laravel/laravel");
        assert_eq!(m.package_type, "project");
        assert!(m.require.contains_key("laravel/framework"));
        assert!(m.require_dev.contains_key("phpunit/phpunit"));
        assert!(m.scripts.contains_key("post-autoload-dump"));
    }

    #[test]
    fn lowercases_name_and_defaults_type() {
        let m: Manifest = r#"{ "name": "Acme/Widget", "require": { "php": "^8.2" } }"#
            .parse()
            .expect("parse");
        assert_eq!(m.name, "acme/widget");
        assert_eq!(m.pretty_name, "Acme/Widget");
        assert_eq!(m.package_type, "library");
    }

    #[test]
    fn lowercases_link_targets() {
        let m: Manifest = r#"{
                "name": "acme/app",
                "require": { "Foo/Bar": "^1.0" },
                "require-dev": { "Baz/Qux": "*@dev" }
            }"#
        .parse()
        .expect("parse");
        assert!(m.require.contains_key("foo/bar"));
        assert!(!m.require.contains_key("Foo/Bar"));
        assert!(m.require_dev.contains_key("baz/qux"));
    }
}
