//! puck_install - install.

#![deny(unsafe_code)]
#![warn(clippy::unwrap_used)]

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, Error>;
