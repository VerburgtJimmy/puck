//! Solver request (`Composer\DependencyResolver\Request`).

use crate::PackageId;
use crate::order::PresentMap;
use indexmap::IndexMap;
use puck_version::ConstraintExpr;

/// Partial-update modes (`Request::UPDATE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAllowTransitive {
    OnlyListed = 0,
    ListedWithTransitiveDepsNoRootRequire = 1,
    ListedWithTransitiveDeps = 2,
}

/// Root requires, fixed/locked packages, and partial-update allow list.
///
/// `requires` is an [`IndexMap`] so root-require rule order matches Composer
/// (PHP insertion-ordered arrays).
#[derive(Debug, Default)]
pub struct Request {
    pub requires: IndexMap<String, ConstraintExpr>,
    pub fixed_packages: PresentMap,
    pub locked_packages: PresentMap,
    pub fixed_locked_packages: PresentMap,
    pub update_allow_list: Vec<String>,
    pub update_allow_transitive: Option<UpdateAllowTransitive>,
}

impl Request {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Request::requireName`.
    pub fn require_name(
        &mut self,
        package_name: impl Into<String>,
        constraint: Option<ConstraintExpr>,
    ) -> Result<(), String> {
        let package_name = package_name.into().to_ascii_lowercase();
        let constraint = constraint.unwrap_or(ConstraintExpr::MatchAll);
        if self.requires.contains_key(&package_name) {
            return Err(format!(
                "Overwriting requires seems like a bug ({package_name})"
            ));
        }
        self.requires.insert(package_name, constraint);
        Ok(())
    }

    pub fn fix_package(&mut self, package_id: PackageId) {
        self.fixed_packages.insert(package_id, ());
    }

    pub fn lock_package(&mut self, package_id: PackageId) {
        self.locked_packages.insert(package_id, ());
    }
}
