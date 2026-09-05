//! Transaction operations from solver decisions.

use crate::decisions::Decisions;
use crate::order::{PackageIdSet, PresentMap};
use crate::package::Package;
use crate::pool::Pool;
use crate::PackageId;
use indexmap::IndexMap;

/// One install / remove / update / alias step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Install { package_id: PackageId },
    Remove { package_id: PackageId },
    Update { from: PackageId, to: PackageId },
    MarkAliasInstalled { package_id: PackageId },
    MarkAliasUninstalled { package_id: PackageId },
}

/// Result of a successful solve (`LockTransaction` / `Transaction` subset).
#[derive(Debug, Clone)]
pub struct Transaction {
    operations: Vec<Operation>,
}

impl Transaction {
    /// Build operations from present packages vs positive decisions.
    ///
    /// Mirrors Composer `Transaction::calculateOperations` (alias handling,
    /// dependency-before-dependent install order). Removals are prepended;
    /// alias uninstalls are appended.
    pub fn from_decisions(pool: &Pool, present: &PresentMap, decisions: &Decisions) -> Self {
        let mut result: Vec<PackageId> = Vec::new();
        for i in 0..decisions.len() {
            let Some(&(literal, _)) = decisions.at_offset(i) else {
                break;
            };
            if literal > 0 {
                result.push(literal as PackageId);
            }
        }

        let result_set: PackageIdSet = result.iter().copied().collect();

        let mut present_by_name: IndexMap<String, PackageId> = IndexMap::new();
        let mut remove_by_name: IndexMap<String, PackageId> = IndexMap::new();
        let mut present_alias: IndexMap<String, PackageId> = IndexMap::new();
        let mut remove_alias: IndexMap<String, PackageId> = IndexMap::new();
        for &id in present.keys() {
            let package = pool.package_by_id(id);
            if package.is_alias() {
                let key = alias_key(package);
                present_alias.insert(key.clone(), id);
                remove_alias.insert(key, id);
            } else {
                present_by_name.insert(package.name.clone(), id);
                remove_by_name.insert(package.name.clone(), id);
            }
        }

        let mut is_required: PackageIdSet = PackageIdSet::new();
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
        // Composer `strcmp` on package names (byte order), inverted for stack pop.
        roots.sort_by(|a, b| {
            pool.package_by_id(*b)
                .name
                .cmp(&pool.package_by_id(*a).name)
        });

        let mut stack = roots;
        let mut visited: PackageIdSet = PackageIdSet::new();
        let mut processed: PackageIdSet = PackageIdSet::new();
        let mut result_ops = Vec::new();

        while let Some(package_id) = stack.pop() {
            if processed.contains(&package_id) {
                continue;
            }
            if !visited.contains(&package_id) {
                visited.insert(package_id);
                stack.push(package_id);
                let package = pool.package_by_id(package_id);
                if let Some(alias_of) = package.alias_of {
                    if result_set.contains(&alias_of) {
                        stack.push(alias_of);
                    }
                } else {
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
                }
            } else {
                processed.insert(package_id);
                let package = pool.package_by_id(package_id);
                if package.is_alias() {
                    let key = alias_key(package);
                    if present_alias.contains_key(&key) {
                        remove_alias.shift_remove(&key);
                    } else {
                        result_ops.push(Operation::MarkAliasInstalled { package_id });
                    }
                } else if let Some(&from_id) = present_by_name.get(&package.name) {
                    let from = pool.package_by_id(from_id);
                    if from.version != package.version {
                        result_ops.push(Operation::Update {
                            from: from_id,
                            to: package_id,
                        });
                    }
                    remove_by_name.shift_remove(&package.name);
                } else {
                    result_ops.push(Operation::Install { package_id });
                    remove_by_name.shift_remove(&package.name);
                }
            }
        }

        let mut operations = Vec::new();
        let mut removals: Vec<PackageId> = remove_by_name.values().copied().collect();
        // Stable sort by id for deterministic remove order.
        removals.sort();
        for package_id in removals {
            operations.push(Operation::Remove { package_id });
        }
        operations.extend(result_ops);

        let mut alias_uninstalls: Vec<PackageId> = remove_alias.values().copied().collect();
        alias_uninstalls.sort();
        for package_id in alias_uninstalls {
            operations.push(Operation::MarkAliasUninstalled { package_id });
        }

        Self { operations }
    }

    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    pub fn package<'a>(&self, pool: &'a Pool, package_id: PackageId) -> &'a Package {
        pool.package_by_id(package_id)
    }
}

fn alias_key(package: &Package) -> String {
    format!("{}::{}", package.name, package.version)
}
