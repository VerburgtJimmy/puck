//! Version value type built on Composer normalisation.

use crate::{Error, Result, normalize, parse_stability};
use std::fmt;

/// Composer stability flags, ordered from most to least stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stability {
    Stable,
    Rc,
    Beta,
    Alpha,
    Dev,
}

impl Stability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Rc => "RC",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
            Self::Dev => "dev",
        }
    }
}

/// A version held in Composer normalised form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Composer-normalised string (e.g. `1.2.3.0`, `1.0.0.0-RC1`, `dev-main`).
    pub normalized: String,
    pub stability: Stability,
    pub original: String,
}

impl Version {
    pub fn parse(input: &str) -> Result<Self> {
        let normalized = normalize(input)?;
        Ok(Self {
            stability: parse_stability(&normalized),
            normalized,
            original: input.to_owned(),
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.normalized)
    }
}

impl TryFrom<&str> for Version {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}
