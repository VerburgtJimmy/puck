//! Pool package (`Composer\Package\BasePackage` / `AliasPackage` subset).

use crate::link::Link;
use crate::PackageId;
use indexmap::IndexMap;
use puck_version::{parse_constraints, Operator};

/// A concrete package version in the pool (or an [`AliasPackage`](Self::alias)).
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
    /// `AliasPackage::getAliasOf` pool id (set before [`crate::Pool::new`]).
    pub alias_of: Option<PackageId>,
    /// `AliasPackage::isRootPackageAlias`.
    pub root_package_alias: bool,
    /// `AliasPackage::hasSelfVersionRequires`.
    pub has_self_version_requires: bool,
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
            alias_of: None,
            root_package_alias: false,
            has_self_version_requires: false,
        }
    }

    pub fn is_alias(&self) -> bool {
        self.alias_of.is_some()
    }

    /// `AliasPackage` constructor subset. `alias_of_id` is the 1-based pool id the
    /// aliased package will receive when both are passed to [`crate::Pool::new`]
    /// in order (aliased package first).
    pub fn alias(
        alias_of: &Package,
        alias_of_id: PackageId,
        version: impl Into<String>,
        pretty_version: impl Into<String>,
    ) -> Self {
        let version = version.into();
        let pretty_version = pretty_version.into();
        let mut package = Self::new(alias_of.name.clone(), version.clone(), pretty_version.clone());
        package.alias_of = Some(alias_of_id);
        let (requires, has_self) = rewrite_self_version_links(&alias_of.requires, &version, &pretty_version);
        package.requires = requires;
        package.has_self_version_requires = has_self;
        package.conflicts = rewrite_self_version_links(&alias_of.conflicts, &version, &pretty_version).0;
        package.provides = rewrite_self_version_links(&alias_of.provides, &version, &pretty_version).0;
        package.replaces = rewrite_self_version_links(&alias_of.replaces, &version, &pretty_version).0;
        package
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

/// Composer `AliasPackage::replaceSelfVersionDependencies` (require path).
fn rewrite_self_version_links(
    links: &IndexMap<String, Link>,
    version: &str,
    pretty_version: &str,
) -> (IndexMap<String, Link>, bool) {
    let mut out = IndexMap::new();
    let mut has_self = false;
    for (key, link) in links {
        if link.pretty_constraint == "self.version" {
            has_self = true;
            let constraint = parse_constraints(&format!("={version}"))
                .unwrap_or_else(|_| {
                    puck_version::ConstraintExpr::simple(Operator::Eq, version.to_string())
                });
            out.insert(
                key.clone(),
                Link::new(
                    link.source.clone(),
                    link.target.clone(),
                    pretty_version,
                    constraint,
                ),
            );
        } else {
            out.insert(key.clone(), link.clone());
        }
    }
    (out, has_self)
}
