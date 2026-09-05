//! Minimal pool builder (`Composer\DependencyResolver\PoolBuilder` subset).
//!
//! Loads locked packages plus packages reachable from root requires (by name),
//! including provide/replace providers. Full stability/alias/partial-update
//! filtering comes later.

use crate::order::PresentMap;
use crate::package::Package;
use crate::pool::Pool;
use crate::request::Request;
use crate::{PackageId, Result};
use indexmap::{IndexMap, IndexSet};
use std::collections::VecDeque;

/// In-memory package repository (`ArrayRepository` subset).
#[derive(Debug, Default, Clone)]
pub struct ArrayRepository {
    packages: Vec<Package>,
}

impl ArrayRepository {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_package(&mut self, package: Package) {
        self.packages.push(package);
    }

    pub fn packages(&self) -> &[Package] {
        &self.packages
    }
}

/// Build a [`Pool`] from repositories + locked packages for a request.
pub struct PoolBuilder;

impl PoolBuilder {
    /// Returns `(pool, present_map)` where `present_map` contains locked/fixed package ids.
    ///
    /// Updates `request` so `lock_package` / `fix_package` ids match the new pool.
    /// Locked and fixed packages are placed first in the pool (stable ids for present map).
    pub fn build(
        repos: &[&ArrayRepository],
        locked: &[Package],
        fixed: &[Package],
        request: &mut Request,
    ) -> Result<(Pool, PresentMap)> {
        let mut repo_packages: Vec<Package> = Vec::new();
        for repo in repos {
            repo_packages.extend(repo.packages.iter().cloned());
        }

        let mut by_name: IndexMap<String, Vec<usize>> = IndexMap::new();
        for (idx, package) in repo_packages.iter().enumerate() {
            for name in package.names(true) {
                by_name.entry(name).or_default().push(idx);
            }
        }

        // IndexSet: first-seen load order, not HashSet shuffle before sort.
        let mut needed_repo: IndexSet<usize> = IndexSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        for name in request.requires.keys() {
            queue.push_back(name.clone());
        }
        for package in locked.iter().chain(fixed.iter()) {
            for name in package.names(true) {
                queue.push_back(name);
            }
            for link in package.requires.values() {
                queue.push_back(link.target.clone());
            }
        }

        while let Some(name) = queue.pop_front() {
            let Some(idxs) = by_name.get(&name).cloned() else {
                continue;
            };
            for idx in idxs {
                if !needed_repo.insert(idx) {
                    continue;
                }
                for link in repo_packages[idx].requires.values() {
                    queue.push_back(link.target.clone());
                }
            }
        }

        for name in request.requires.keys() {
            if let Some(idxs) = by_name.get(name) {
                for &idx in idxs {
                    needed_repo.insert(idx);
                }
            }
        }

        let mut selected: Vec<Package> = locked.iter().chain(fixed.iter()).cloned().collect();

        let mut rest: Vec<Package> = needed_repo
            .into_iter()
            .map(|idx| repo_packages[idx].clone())
            .collect();
        // Explicit sort where Composer sorts pool candidates by name/version.
        rest.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.version.cmp(&b.version))
        });
        selected.extend(rest);

        let pool = Pool::new(selected);

        let mut present = PresentMap::new();
        for (i, _) in locked.iter().enumerate() {
            let id = (i as PackageId) + 1;
            request.lock_package(id);
            present.insert(id, ());
        }
        let fixed_start = locked.len();
        for (i, _) in fixed.iter().enumerate() {
            let id = (fixed_start + i) as PackageId + 1;
            request.fix_package(id);
            present.insert(id, ());
        }

        Ok((pool, present))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Link;
    use puck_version::parse_constraints;

    #[test]
    fn loads_transitive_requires() {
        let mut root = Package::new("root/root", "1.0.0.0", "1.0");
        root.requires.insert(
            "a/a".into(),
            Link::new(
                "root/root",
                "a/a",
                "*",
                parse_constraints("*").unwrap(),
            ),
        );
        let dep = Package::new("a/a", "1.0.0.0", "1.0");
        let unused = Package::new("z/z", "1.0.0.0", "1.0");

        let mut repo = ArrayRepository::new();
        repo.add_package(root);
        repo.add_package(dep);
        repo.add_package(unused);

        let mut request = Request::new();
        request.require_name("root/root", None).unwrap();

        let (pool, present) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();
        assert!(present.is_empty());
        assert!(pool.packages().iter().any(|p| p.name == "root/root"));
        assert!(pool.packages().iter().any(|p| p.name == "a/a"));
        assert!(!pool.packages().iter().any(|p| p.name == "z/z"));
    }

    #[test]
    fn locked_packages_are_present() {
        let locked = Package::new("a/a", "1.0.0.0", "1.0");
        let newer = Package::new("a/a", "1.1.0.0", "1.1");
        let mut repo = ArrayRepository::new();
        repo.add_package(newer);

        let mut request = Request::new();
        request.require_name("a/a", None).unwrap();

        let (pool, present) =
            PoolBuilder::build(&[&repo], &[locked], &[], &mut request).unwrap();
        assert_eq!(present.len(), 1);
        assert!(present.contains_key(&1));
        assert_eq!(pool.package_by_id(1).version, "1.0.0.0");
        assert!(pool.packages().iter().any(|p| p.version == "1.1.0.0"));
    }
}
