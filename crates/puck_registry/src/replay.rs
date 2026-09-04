//! Offline registry response replay for deterministic tests and CI.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    /// Pass through to the network (not used in CI).
    Live,
    /// Record responses under the fixture directory.
    Record,
    /// Serve only from recordings; error on miss.
    Replay,
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("no recorded response for {0}")]
    Miss(String),
    #[error("io error for {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// File-backed store keyed by a stable request id (e.g. URL path + query).
#[derive(Debug, Clone)]
pub struct ReplayStore {
    root: PathBuf,
    mode: ReplayMode,
}

impl ReplayStore {
    pub fn new(root: impl Into<PathBuf>, mode: ReplayMode) -> Self {
        Self {
            root: root.into(),
            mode,
        }
    }

    pub fn mode(&self) -> ReplayMode {
        self.mode
    }

    fn path_for(&self, key: &str) -> PathBuf {
        // Flatten keys into safe relative paths.
        let safe: String = key
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        self.root.join(format!("{safe}.json"))
    }

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, ReplayError> {
        let path = self.path_for(key);
        match self.mode {
            ReplayMode::Live => Ok(None),
            ReplayMode::Record => read_optional(&path),
            ReplayMode::Replay => match read_optional(&path)? {
                Some(body) => Ok(Some(body)),
                None => Err(ReplayError::Miss(key.to_owned())),
            },
        }
    }

    pub fn put(&self, key: &str, body: &[u8]) -> Result<(), ReplayError> {
        if self.mode != ReplayMode::Record {
            return Ok(());
        }
        let path = self.path_for(key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ReplayError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        fs::write(&path, body).map_err(|source| ReplayError::Io {
            path: path.display().to_string(),
            source,
        })
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, ReplayError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ReplayError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}
