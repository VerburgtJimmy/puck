//! Policy (`Composer\DependencyResolver\DefaultPolicy`).

use crate::Literal;
use crate::pool::Pool;
use indexmap::IndexMap;
use puck_version::{Operator, version_compare};

/// Selects preferred install candidates from a decision queue.
#[derive(Debug, Default)]
pub struct DefaultPolicy {
    #[allow(dead_code)]
    prefer_stable: bool,
    prefer_lowest: bool,
}

impl DefaultPolicy {
    pub fn new(prefer_stable: bool, prefer_lowest: bool) -> Self {
        Self {
            prefer_stable,
            prefer_lowest,
        }
    }

    /// `DefaultPolicy::selectPreferredPackages`.
    pub fn select_preferred_packages(
        &self,
        pool: &Pool,
        mut literals: Vec<Literal>,
        required_package: Option<&str>,
    ) -> Vec<Literal> {
        // PHP `sort($literals)` - stable since PHP 8.
        literals.sort();
        // Group by name in first-seen order after the sort (Composer PHP arrays),
        // not alphabetical `BTreeMap` order.
        let mut packages: IndexMap<String, Vec<Literal>> = IndexMap::new();
        for literal in literals {
            let name = pool.literal_to_package(literal).name.clone();
            packages.entry(name).or_default().push(literal);
        }

        for name_literals in packages.values_mut() {
            // PHP `usort` - stable since PHP 8.
            name_literals.sort_by(|&a, &b| {
                self.compare_by_priority(
                    pool,
                    pool.literal_to_package(a),
                    pool.literal_to_package(b),
                    required_package,
                    true,
                )
            });
            *name_literals = self.prune_to_best_version(pool, name_literals);
        }

        let mut selected: Vec<Literal> = packages.into_values().flatten().collect();
        // PHP `usort` across packages (replace-aware).
        selected.sort_by(|&a, &b| {
            self.compare_by_priority(
                pool,
                pool.literal_to_package(a),
                pool.literal_to_package(b),
                required_package,
                false,
            )
        });
        selected
    }

    fn compare_by_priority(
        &self,
        _pool: &Pool,
        a: &crate::package::Package,
        b: &crate::package::Package,
        required_package: Option<&str>,
        ignore_replace: bool,
    ) -> std::cmp::Ordering {
        if !ignore_replace {
            if self.replaces(a, b) {
                return std::cmp::Ordering::Greater; // use b
            }
            if self.replaces(b, a) {
                return std::cmp::Ordering::Less; // use a
            }
            if let Some(required) = required_package
                && let Some(pos) = required.find('/')
            {
                let required_vendor = &required[..pos];
                let a_same = a.name.starts_with(required_vendor);
                let b_same = b.name.starts_with(required_vendor);
                if a_same != b_same {
                    return if a_same {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    };
                }
            }
        }

        // Composer: tie-break by package id (`$a->id < $b->id`).
        a.id.cmp(&b.id)
    }

    fn replaces(&self, source: &crate::package::Package, target: &crate::package::Package) -> bool {
        source.replaces.contains_key(&target.name)
    }

    fn prune_to_best_version(&self, pool: &Pool, literals: &[Literal]) -> Vec<Literal> {
        if literals.is_empty() {
            return Vec::new();
        }
        let operator = if self.prefer_lowest {
            Operator::Lt
        } else {
            Operator::Gt
        };
        // `version_compare` via puck_version (Composer/PHP semantics, not Rust Ord).
        let mut best_literals = vec![literals[0]];
        let mut best_package = pool.literal_to_package(literals[0]);
        for &literal in &literals[1..] {
            let package = pool.literal_to_package(literal);
            if version_compare(&package.version, &best_package.version, operator) {
                best_package = package;
                best_literals = vec![literal];
            } else if version_compare(&package.version, &best_package.version, Operator::Eq) {
                best_literals.push(literal);
            }
        }
        best_literals
    }
}
