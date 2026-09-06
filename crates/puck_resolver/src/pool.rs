//! Package pool (`Composer\DependencyResolver\Pool`).

use crate::order::PresentMap;
use crate::package::Package;
use crate::{Literal, PackageId, Result};
use indexmap::IndexMap;
use puck_version::{ConstraintExpr, Operator};

/// All candidate packages for one solve.
///
/// `package_by_name` is insertion-ordered like Composer’s PHP arrays: packages
/// are appended in pool construction order under each provided/replaced name.
#[derive(Debug, Default)]
pub struct Pool {
    packages: Vec<Package>,
    package_by_name: IndexMap<String, Vec<PackageId>>,
    provider_cache: IndexMap<(String, String), Vec<PackageId>>,
}

impl Pool {
    pub fn new(mut packages: Vec<Package>) -> Self {
        let mut pool = Self::default();
        let mut id: PackageId = 1;
        for mut package in packages.drain(..) {
            package.id = id;
            for name in package.names(true) {
                pool.package_by_name.entry(name).or_default().push(id);
            }
            pool.packages.push(package);
            id += 1;
        }
        pool
    }

    pub fn len(&self) -> usize {
        self.packages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    pub fn packages(&self) -> &[Package] {
        &self.packages
    }

    /// `Pool::packageById` - panics if id is out of range (Composer does the same).
    pub fn package_by_id(&self, id: PackageId) -> &Package {
        &self.packages[(id as usize) - 1]
    }

    pub fn literal_to_package(&self, literal: Literal) -> &Package {
        self.package_by_id(literal.unsigned_abs())
    }

    pub fn literal_to_pretty_string(&self, literal: Literal, installed: &PresentMap) -> String {
        let package = self.literal_to_package(literal);
        let prefix = if installed.contains_key(&package.id) {
            if literal > 0 { "keep" } else { "remove" }
        } else if literal > 0 {
            "install"
        } else {
            "don't install"
        };
        format!("{prefix} {}", package.pretty_string())
    }

    /// `Pool::whatProvides`.
    pub fn what_provides(
        &mut self,
        name: &str,
        constraint: Option<&ConstraintExpr>,
    ) -> Result<Vec<PackageId>> {
        let key = (
            name.to_ascii_lowercase(),
            constraint.map(|c| format!("{c:?}")).unwrap_or_default(),
        );
        if let Some(cached) = self.provider_cache.get(&key) {
            return Ok(cached.clone());
        }
        let matches = self.compute_what_provides(&key.0, constraint)?;
        self.provider_cache.insert(key, matches.clone());
        Ok(matches)
    }

    fn compute_what_provides(
        &self,
        name: &str,
        constraint: Option<&ConstraintExpr>,
    ) -> Result<Vec<PackageId>> {
        let Some(candidates) = self.package_by_name.get(name) else {
            return Ok(Vec::new());
        };
        let mut matches = Vec::new();
        for &id in candidates {
            if self.match_package(id, name, constraint)? {
                matches.push(id);
            }
        }
        Ok(matches)
    }

    /// `Pool::match` - own name, provide, or replace.
    pub fn match_package(
        &self,
        package_id: PackageId,
        name: &str,
        constraint: Option<&ConstraintExpr>,
    ) -> Result<bool> {
        let candidate = self.package_by_id(package_id);
        let name = name.to_ascii_lowercase();

        if candidate.name == name {
            return Ok(match constraint {
                None => true,
                Some(c) => {
                    let provider = ConstraintExpr::simple(Operator::Eq, candidate.version.clone());
                    c.matches_provider(&provider)
                }
            });
        }

        if let Some(link) = candidate.provides.get(&name) {
            if constraint_matches(constraint, &link.constraint) {
                return Ok(true);
            }
        }
        if let Some(link) = candidate.replaces.get(&name) {
            if constraint_matches(constraint, &link.constraint) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Composer: `$constraint->matches($link->getConstraint())`.
fn constraint_matches(require: Option<&ConstraintExpr>, provided: &ConstraintExpr) -> bool {
    match require {
        None => true,
        Some(req) => req.matches_provider(provided),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Link;
    use puck_version::parse_constraints;

    #[test]
    fn indexes_replace_names() {
        let mut fw = Package::new("laravel/framework", "13.30.1.0", "v13.30.1");
        fw.replaces.insert(
            "illuminate/support".into(),
            Link::new(
                "laravel/framework",
                "illuminate/support",
                "13.30.1.0",
                parse_constraints("13.30.1.0").unwrap(),
            ),
        );
        let pool = Pool::new(vec![fw]);
        assert_eq!(
            pool.package_by_name.get("illuminate/support").unwrap(),
            &vec![1]
        );
    }
}
