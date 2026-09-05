//! Pool package (`Composer\Package\BasePackage` subset needed by the solver).

use crate::link::Link;
use indexmap::IndexMap;

/// A concrete package version in the pool.
#[derive(Debug, Clone)]
pub struct Package {
    /// Assigned by [`crate::Pool`] (1-based). Zero until inserted.
    pub id: u32,
    pub name: String,
    /// Normalized version (`VersionParser::normalize`).
    pub version: String,
    pub pretty_version: String,
    pub requires: IndexMap<String, Link>,
    pub conflicts: IndexMap<String, Link>,
    pub provides: IndexMap<String, Link>,
    pub replaces: IndexMap<String, Link>,
}

impl Package {
    pub fn new(name: impl Into<String>, version: impl Into<String>, pretty_version: impl Into<String>) -> Self {
        let pretty_version = pretty_version.into();
        let version = version.into();
        Self {
            id: 0,
            name: name.into().to_ascii_lowercase(),
            version,
            pretty_version,
            requires: IndexMap::new(),
            conflicts: IndexMap::new(),
            provides: IndexMap::new(),
            replaces: IndexMap::new(),
        }
    }

    /// `BasePackage::getNames($provides = true)`.
    pub fn names(&self, include_provides: bool) -> Vec<String> {
        let mut names = indexmap::IndexSet::new();
        names.insert(self.name.clone());
        if include_provides {
            for link in self.provides.values() {
                names.insert(link.target.clone());
            }
        }
        for link in self.replaces.values() {
            names.insert(link.target.clone());
        }
        names.into_iter().collect()
    }

    pub fn pretty_string(&self) -> String {
        format!("{} {}", self.name, self.pretty_version)
    }
}
