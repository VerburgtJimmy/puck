//! Transaction operations from solver decisions.

use crate::decisions::Decisions;
use crate::package::Package;
use crate::pool::Pool;
use crate::PackageId;
use rustc_hash::FxHashMap;
use std::collections::HashSet;

/// One install / remove / update step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Install { package_id: PackageId },
    Remove { package_id: PackageId },
}

/// Result of a successful solve (`LockTransaction` / `Transaction` subset).
#[derive(Debug, Clone)]
pub struct Transaction {
    operations: Vec<Operation>,
}

impl Transaction {
    /// Build operations from present packages vs positive decisions.
    ///
    /// Install order follows Composer `Transaction::calculateOperations` topo walk
    /// (dependencies before dependents). Removals are prepended.
    pub fn from_decisions(
        pool: &Pool,
        present: &FxHashMap<PackageId, ()>,
        decisions: &Decisions,
    ) -> Self {
        let mut result: Vec<PackageId> = Vec::new();
        for i in 0..decisions.len() {
            let Some(&(literal, _)) = decisions.at_offset(i) else {
                break;
            };
            if literal > 0 {
                result.push(literal as PackageId);
            }
        }

        let result_set: HashSet<PackageId> = result.iter().copied().collect();
        let mut present_by_name: FxHashMap<String, PackageId> = FxHashMap::default();
        let mut remove_by_name: FxHashMap<String, PackageId> = FxHashMap::default();
        for &id in present.keys() {
            let name = pool.package_by_id(id).name.clone();
            present_by_name.insert(name.clone(), id);
            remove_by_name.insert(name, id);
        }

        // Root packages: in result but not required by another result package.
        let mut is_required: HashSet<PackageId> = HashSet::new();
        for &id in &result {
            let package = pool.package_by_id(id);
            for link in package.requires.values() {
                for &other in &result {
                    if pool
                        .match_package(other, &link.target, Some(&link.constraint))
                        .unwrap_or(false)
                    {
                        is_required.insert(other);
                    }
                }
            }
        }
        let mut roots: Vec<PackageId> = result
            .iter()
            .copied()
            .filter(|id| !is_required.contains(id))
            .collect();
        if roots.is_empty() {
            roots = result.clone();
        }
        // Composer picks lowest name on cycles; sort roots by name desc for stack pop order.
        roots.sort_by(|a, b| {
            pool.package_by_id(*b)
                .name
                .cmp(&pool.package_by_id(*a).name)
        });

        let mut stack = roots;
        let mut visited: HashSet<PackageId> = HashSet::new();
        let mut processed: HashSet<PackageId> = HashSet::new();
        let mut install_ops = Vec::new();

        while let Some(package_id) = stack.pop() {
            if processed.contains(&package_id) {
                continue;
            }
            if !visited.contains(&package_id) {
                visited.insert(package_id);
                stack.push(package_id);
                let package = pool.package_by_id(package_id);
                for link in package.requires.values() {
                    for &other in &result {
                        if result_set.contains(&other)
                            && pool
                                .match_package(other, &link.target, Some(&link.constraint))
                                .unwrap_or(false)
                        {
                            stack.push(other);
                        }
                    }
                }
            } else {
                processed.insert(package_id);
                let package = pool.package_by_id(package_id);
                if present_by_name.contains_key(&package.name) {
                    // keep / update - for now treat same name as keep (no update yet)
                    remove_by_name.remove(&package.name);
                } else {
                    install_ops.push(Operation::Install { package_id });
                    remove_by_name.remove(&package.name);
                }
            }
        }

        let mut operations = Vec::new();
        let mut removals: Vec<PackageId> = remove_by_name.values().copied().collect();
        removals.sort_unstable();
        for package_id in removals {
            operations.push(Operation::Remove { package_id });
        }
        operations.extend(install_ops);

        Self { operations }
    }

    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    pub fn package<'a>(&self, pool: &'a Pool, package_id: PackageId) -> &'a Package {
        pool.package_by_id(package_id)
    }
}
