//! Rule generation (`Composer\DependencyResolver\RuleSetGenerator`).

use crate::order::PresentMap;
use crate::platform::is_platform_package;
use crate::pool::Pool;
use crate::request::Request;
use crate::rule::{Rule, RuleReason, RuleType};
use crate::rule_set::RuleSet;
use crate::{Literal, PackageId, Result};
use indexmap::IndexMap;
use std::collections::VecDeque;

/// Builds the SAT rule set for a request against a pool.
pub struct RuleSetGenerator<'a> {
    pool: &'a mut Pool,
    rules: RuleSet,
    /// Insertion-ordered so conflict-rule walks match Composer.
    added_map: PresentMap,
    added_packages_by_names: IndexMap<String, Vec<PackageId>>,
}

impl<'a> RuleSetGenerator<'a> {
    pub fn new(pool: &'a mut Pool) -> Self {
        Self {
            pool,
            rules: RuleSet::new(),
            added_map: PresentMap::new(),
            added_packages_by_names: IndexMap::new(),
        }
    }

    /// `RuleSetGenerator::getRulesFor`.
    pub fn get_rules_for(mut self, request: &Request) -> Result<RuleSet> {
        self.add_rules_for_request(request)?;
        self.add_rules_for_root_aliases()?;
        self.add_conflict_rules()?;
        self.added_map.clear();
        self.added_packages_by_names.clear();
        Ok(std::mem::take(&mut self.rules))
    }

    fn create_require_rule(
        package_id: PackageId,
        providers: &[PackageId],
        reason: RuleReason,
    ) -> Option<Rule> {
        let mut literals: Vec<Literal> = vec![-(package_id as Literal)];
        for &provider in providers {
            if provider == package_id {
                return None;
            }
            literals.push(provider as Literal);
        }
        Some(Rule::generic(literals, reason))
    }

    fn create_install_one_of_rule(packages: &[PackageId], reason: RuleReason) -> Rule {
        let literals: Vec<Literal> = packages.iter().map(|&id| id as Literal).collect();
        Rule::generic(literals, reason)
    }

    fn create_rule_2_literals(
        issuer: PackageId,
        provider: PackageId,
        reason: RuleReason,
    ) -> Option<Rule> {
        if issuer == provider {
            return None;
        }
        Some(Rule::two_literals(
            -(issuer as Literal),
            -(provider as Literal),
            reason,
        ))
    }

    fn create_multi_conflict_rule(packages: &[PackageId], reason: RuleReason) -> Rule {
        let literals: Vec<Literal> = packages.iter().map(|&id| -(id as Literal)).collect();
        if literals.len() == 2 {
            return Rule::two_literals(literals[0], literals[1], reason);
        }
        Rule::multi_conflict(literals, reason)
    }

    fn add_rule(&mut self, rule_type: RuleType, rule: Option<Rule>) {
        if let Some(rule) = rule {
            self.rules.add(rule, rule_type);
        }
    }

    fn add_rules_for_package(&mut self, package_id: PackageId) -> Result<()> {
        let mut work: VecDeque<PackageId> = VecDeque::new();
        work.push_back(package_id);

        while let Some(package_id) = work.pop_front() {
            if self.added_map.contains_key(&package_id) {
                continue;
            }
            self.added_map.insert(package_id, ());

            let alias_of = self.pool.package_by_id(package_id).alias_of;
            if let Some(alias_of_id) = alias_of {
                // Composer: enqueue aliasOf, add alias <-> aliasOf require rules.
                work.push_back(alias_of_id);
                self.add_rule(
                    RuleType::Package,
                    Self::create_require_rule(
                        package_id,
                        &[alias_of_id],
                        RuleReason::PackageAlias,
                    ),
                );
                self.add_rule(
                    RuleType::Package,
                    Self::create_require_rule(
                        alias_of_id,
                        &[package_id],
                        RuleReason::PackageInverseAlias,
                    ),
                );
                if !self.pool.package_by_id(package_id).has_self_version_requires {
                    continue;
                }
            } else {
                // Non-aliases only: SAME_NAME / conflict indexing (Composer skips AliasPackage).
                let names = self.pool.package_by_id(package_id).names(false);
                for name in names {
                    self.added_packages_by_names
                        .entry(name)
                        .or_default()
                        .push(package_id);
                }
            }

            let requires: Vec<(String, puck_version::ConstraintExpr)> = self
                .pool
                .package_by_id(package_id)
                .requires
                .iter()
                .map(|(t, link)| (t.clone(), link.constraint.clone()))
                .collect();

            for (target, constraint) in requires {
                if is_platform_package(&target) {
                    continue;
                }
                let providers = self.pool.what_provides(&target, Some(&constraint))?;
                self.add_rule(
                    RuleType::Package,
                    Self::create_require_rule(
                        package_id,
                        &providers,
                        RuleReason::PackageRequires {
                            target: target.clone(),
                        },
                    ),
                );
                for provider in providers {
                    work.push_back(provider);
                }
            }
        }
        Ok(())
    }

    /// `RuleSetGenerator::addRulesForRootAliases`.
    fn add_rules_for_root_aliases(&mut self) -> Result<()> {
        let package_ids: Vec<PackageId> = self
            .pool
            .packages()
            .iter()
            .map(|p| p.id)
            .collect();
        for package_id in package_ids {
            if self.added_map.contains_key(&package_id) {
                continue;
            }
            let package = self.pool.package_by_id(package_id);
            let Some(alias_of) = package.alias_of else {
                continue;
            };
            if package.root_package_alias || self.added_map.contains_key(&alias_of) {
                self.add_rules_for_package(package_id)?;
            }
        }
        Ok(())
    }

    fn add_conflict_rules(&mut self) -> Result<()> {
        let added: Vec<PackageId> = self.added_map.keys().copied().collect();
        for package_id in added {
            let conflicts: Vec<(String, puck_version::ConstraintExpr)> = self
                .pool
                .package_by_id(package_id)
                .conflicts
                .iter()
                .map(|(t, link)| (t.clone(), link.constraint.clone()))
                .collect();

            for (target, constraint) in conflicts {
                if is_platform_package(&target) {
                    continue;
                }
                if !self.added_packages_by_names.contains_key(&target) {
                    continue;
                }
                let conflict_ids = self.pool.what_provides(&target, Some(&constraint))?;
                for conflict_id in conflict_ids {
                    // Composer: skip AliasPackage conflicts unless name == link target.
                    let conflict = self.pool.package_by_id(conflict_id);
                    if conflict.is_alias() && conflict.name != target {
                        continue;
                    }
                    self.add_rule(
                        RuleType::Package,
                        Self::create_rule_2_literals(
                            package_id,
                            conflict_id,
                            RuleReason::PackageConflict,
                        ),
                    );
                }
            }
        }

        let by_names: Vec<(String, Vec<PackageId>)> = self
            .added_packages_by_names
            .iter()
            .map(|(n, ids)| (n.clone(), ids.clone()))
            .collect();
        for (name, packages) in by_names {
            if packages.len() > 1 {
                self.add_rule(
                    RuleType::Package,
                    Some(Self::create_multi_conflict_rule(
                        &packages,
                        RuleReason::PackageSameName {
                            package_name: name,
                        },
                    )),
                );
            }
        }
        Ok(())
    }

    fn add_rules_for_request(&mut self, request: &Request) -> Result<()> {
        for &package_id in request.fixed_packages.keys() {
            self.add_rules_for_package(package_id)?;
            let rule = Self::create_install_one_of_rule(
                &[package_id],
                RuleReason::Fixed { package_id },
            );
            self.add_rule(RuleType::Request, Some(rule));
        }

        let requires: Vec<(String, puck_version::ConstraintExpr)> = request
            .requires
            .iter()
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect();

        for (package_name, constraint) in requires {
            if is_platform_package(&package_name) {
                continue;
            }
            let packages = self.pool.what_provides(&package_name, Some(&constraint))?;
            if packages.is_empty() {
                continue;
            }
            for &package_id in &packages {
                self.add_rules_for_package(package_id)?;
            }
            let rule = Self::create_install_one_of_rule(
                &packages,
                RuleReason::RootRequire {
                    package_name: package_name.clone(),
                    constraint: constraint.clone(),
                },
            );
            self.add_rule(RuleType::Request, Some(rule));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Link;
    use crate::package::Package;
    use puck_version::parse_constraints;

    #[test]
    fn root_require_install_one_of_and_same_name_conflict() {
        let a1 = Package::new("a/a", "1.0.0.0", "1.0");
        let a2 = Package::new("a/a", "2.0.0.0", "2.0");
        let mut pool = Pool::new(vec![a1, a2]);
        let mut request = Request::new();
        request
            .require_name("a/a", Some(parse_constraints("*").unwrap()))
            .unwrap();

        let rules = RuleSetGenerator::new(&mut pool)
            .get_rules_for(&request)
            .unwrap();

        assert!(rules.count_type(RuleType::Request) >= 1);
        // two versions of a/a -> SAME_NAME multi-conflict
        assert!(
            rules
                .rules_of_type(RuleType::Package)
                .any(|r| matches!(r.reason(), RuleReason::PackageSameName { .. })),
            "expected SAME_NAME conflict between a/a versions"
        );
    }

    #[test]
    fn require_rule_for_dependency() {
        let mut root = Package::new("root/root", "1.0.0.0", "1.0");
        root.requires.insert(
            "a/a".into(),
            Link::new(
                "root/root",
                "a/a",
                "^1.0",
                parse_constraints("^1.0").unwrap(),
            ),
        );
        let dep = Package::new("a/a", "1.0.0.0", "1.0");
        let mut pool = Pool::new(vec![root, dep]);
        let mut request = Request::new();
        request
            .require_name("root/root", Some(parse_constraints("*").unwrap()))
            .unwrap();

        let rules = RuleSetGenerator::new(&mut pool)
            .get_rules_for(&request)
            .unwrap();

        assert!(
            rules
                .rules_of_type(RuleType::Package)
                .any(|r| matches!(
                    r.reason(),
                    RuleReason::PackageRequires { target } if target == "a/a"
                )),
            "expected PACKAGE_REQUIRES for a/a"
        );
    }
}
