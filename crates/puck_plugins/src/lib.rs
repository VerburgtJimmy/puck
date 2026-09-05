//! Native Composer plugin adapters (Tier 1).
//!
//! Ports plugin side effects that are fully specified by installed package
//! metadata, so puck does not need to host PHP Composer plugins for those.

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

mod native;

pub use native::pest_plugin::{PestPluginDumpStatus, run_pest_plugin_dump};
pub use native::phpstan_extension_installer::{
    PhpstanExtensionInstallStatus, run_phpstan_extension_installer,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {message}")]
    Parse { path: String, message: String },
}

pub type Result<T> = std::result::Result<T, Error>;
