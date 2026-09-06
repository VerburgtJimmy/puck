//! Minimal pool builder (`Composer\DependencyResolver\PoolBuilder` subset).
//!
//! Loads locked packages plus packages reachable from root requires by **real
//! package name** only - matching Composer `ArrayRepository::loadPackages`,
//! which keys on `Package::getName()` and does **not** pull in provide/replace
//! providers. Those enter the pool only when their own name is required (or via
//! a ComposerRepository provider map, not modeled here).
//!
//! Applies [`is_package_acceptable`](crate::stability::is_package_acceptable)
//! like Composer (default minimum: stable). Locked and fixed packages are always kept.

use crate::order::PresentMap;
use crate::package::Package;
use crate::pool::Pool;
use crate::request::Request;
use crate::stability::is_package_acceptable;
use crate::{PackageId, Result};
use indexmap::{IndexMap, IndexSet};
use puck_version::Stability;
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
    /// Build with Composer default `minimum-stability: stable` and no flags.
    pub fn build(
        repos: &[&ArrayRepository],
        locked: &[Package],
        fixed: &[Package],
        request: &mut Request,
    ) -> Result<(Pool, PresentMap)> {
        Self::build_with_stability(
            repos,
            locked,
            fixed,
            request,
            Stability::Stable,
            &IndexMap::new(),
        )
    }

    /// Returns `(pool, present_map)` where `present_map` contains locked/fixed package ids.
    ///
    /// Updates `request` so `lock_package` / `fix_package` ids match the new pool.
    /// Locked and fixed packages are placed first in the pool (stable ids for present map).
    pub fn build_with_stability(
        repos: &[&ArrayRepository],
        locked: &[Package],
        fixed: &[Package],
        request: &mut Request,
        minimum_stability: Stability,
        stability_flags: &IndexMap<String, Stability>,
    ) -> Result<(Pool, PresentMap)> {
        let mut repo_packages: Vec<Package> = Vec::new();
        for repo in repos {
            repo_packages.extend(repo.packages.iter().cloned());
        }

        // Composer ArrayRepository: match `getName()` only, not provide/replace.
        let mut by_name: IndexMap<String, Vec<usize>> = IndexMap::new();
        for (idx, package) in repo_packages.iter().enumerate() {
            by_name.entry(package.name.clone()).or_default().push(idx);
        }

        // IndexSet: first-seen load order, not HashSet shuffle before sort.
        let mut needed_repo: IndexSet<usize> = IndexSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        for name in request.requires.keys() {
            queue.push_back(name.clone());
        }
        for package in locked.iter().chain(fixed.iter()) {
            queue.push_back(package.name.clone());
            for link in package.requires.values() {
                queue.push_back(link.target.clone());
            }
        }

        while let Some(name) = queue.pop_front() {
            let Some(idxs) = by_name.get(&name).cloned() else {
                continue;
            };
            for idx in idxs {
                if !is_package_acceptable(&repo_packages[idx], minimum_stability, stability_flags) {
                    continue;
                }
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
                    if is_package_acceptable(
                        &repo_packages[idx],
                        minimum_stability,
                        stability_flags,
                    ) {
                        needed_repo.insert(idx);
                    }
                }
            }
        }

        let mut selected: Vec<Package> = locked.iter().chain(fixed.iter()).cloned().collect();

        let mut rest: Vec<Package> = needed_repo
            .into_iter()
            .map(|idx| repo_packages[idx].clone())
            .collect();
        // Explicit sort where Composer sorts pool candidates by name/version.
        rest.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
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
            Link::new("root/root", "a/a", "*", parse_constraints("*").unwrap()),
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

        let (pool, present) = PoolBuilder::build(&[&repo], &[locked], &[], &mut request).unwrap();
        assert_eq!(present.len(), 1);
        assert!(present.contains_key(&1));
        assert_eq!(pool.package_by_id(1).version, "1.0.0.0");
        assert!(pool.packages().iter().any(|p| p.version == "1.1.0.0"));
    }

    #[test]
    fn does_not_load_provide_only_packages() {
        let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
        package_a.requires.insert(
            "b/b".into(),
            Link::new("a/a", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
        );
        let mut package_q = Package::new("q/q", "1.0.0.0", "1.0");
        package_q.provides.insert(
            "b/b".into(),
            Link::new("q/q", "b/b", "=1.0", parse_constraints("=1.0").unwrap()),
        );

        let mut repo = ArrayRepository::new();
        repo.add_package(package_a);
        repo.add_package(package_q);

        let mut request = Request::new();
        request.require_name("a/a", None).unwrap();

        let (pool, _) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();
        assert!(pool.packages().iter().any(|p| p.name == "a/a"));
        assert!(!pool.packages().iter().any(|p| p.name == "q/q"));
    }

    #[test]
    fn does_not_load_replace_only_packages() {
        let mut package_a = Package::new("a/a", "1.0.0.0", "1.0");
        package_a.requires.insert(
            "b/b".into(),
            Link::new("a/a", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
        );
        let mut package_q = Package::new("q/q", "1.0.0.0", "1.0");
        package_q.replaces.insert(
            "b/b".into(),
            Link::new("q/q", "b/b", ">=1.0", parse_constraints(">=1.0").unwrap()),
        );

        let mut repo = ArrayRepository::new();
        repo.add_package(package_a);
        repo.add_package(package_q);

        let mut request = Request::new();
        request.require_name("a/a", None).unwrap();

        let (pool, _) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();
        assert!(pool.packages().iter().any(|p| p.name == "a/a"));
        assert!(!pool.packages().iter().any(|p| p.name == "q/q"));
    }

    #[test]
    fn stable_minimum_skips_dev_packages() {
        let stable = Package::new("a/a", "1.0.0.0", "1.0");
        let dev = Package::new("a/a", "1.1.0.0-dev", "1.1-dev");
        let mut repo = ArrayRepository::new();
        repo.add_package(stable);
        repo.add_package(dev);

        let mut request = Request::new();
        request.require_name("a/a", None).unwrap();

        let (pool, _) = PoolBuilder::build(&[&repo], &[], &[], &mut request).unwrap();
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.package_by_id(1).version, "1.0.0.0");
    }

    #[test]
    fn stability_flag_allows_dev_package() {
        let dev = Package::new("c/c", "2.0.0.0-dev", "2.0-dev");
        let mut repo = ArrayRepository::new();
        repo.add_package(dev);

        let mut flags = IndexMap::new();
        flags.insert("c/c".into(), Stability::Dev);
        let mut request = Request::new();
        request.require_name("c/c", None).unwrap();

        let (pool, _) = PoolBuilder::build_with_stability(
            &[&repo],
            &[],
            &[],
            &mut request,
            Stability::Stable,
            &flags,
        )
        .unwrap();
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.package_by_id(1).version, "2.0.0.0-dev");
    }
}
