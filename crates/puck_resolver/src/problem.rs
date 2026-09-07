//! Solver problems (`Composer\DependencyResolver\Problem` subset).

use crate::package::Package;
use crate::pool::Pool;
use crate::request::Request;
use crate::rule::{Rule, RuleReason};
use indexmap::IndexSet;
use puck_version::ConstraintExpr;

/// One unsolvable conflict section.
#[derive(Debug, Default, Clone)]
pub struct Problem {
    rules: Vec<Rule>,
}

impl Problem {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Minimal string without pool (root-require miss only).
    pub fn pretty_string(&self) -> String {
        if self.rules.len() == 1 {
            let rule = &self.rules[0];
            if let RuleReason::RootRequire {
                package_name,
                constraint,
            } = rule.reason()
            {
                return format!(
                    "\n    - {}",
                    root_require_missing(package_name.as_str(), &constraint, &[])
                );
            }
        }
        format!("\n    - {} conflicting rule(s)", self.rules.len())
    }

    /// `Problem::getPrettyString` subset with pool context (no RepositorySet).
    pub fn pretty_string_with_pool(&self, pool: &mut Pool, request: &Request) -> String {
        let mut rules = self.rules.clone();
        if rules.len() == 1 {
            let rule = &rules[0];
            if let RuleReason::RootRequire {
                package_name,
                constraint,
            } = rule.reason()
            {
                let providers = pool
                    .what_provides(&package_name, Some(&constraint))
                    .unwrap_or_default();
                if providers.is_empty() {
                    let named = packages_named(pool, &package_name);
                    return format!(
                        "\n    - {}",
                        root_require_missing(&package_name, &constraint, &named)
                    );
                }
            }
        }

        rules.sort_by(|a, b| {
            rule_priority(b)
                .cmp(&rule_priority(a))
                .then_with(|| sortable_string(pool, a).cmp(&sortable_string(pool, b)))
        });

        let mut seen = IndexSet::new();
        let mut lines = Vec::new();
        for rule in &rules {
            if !seen.insert(rule.object_id()) {
                continue;
            }
            let line = rule_pretty_string(rule, pool, request);
            if !line.is_empty() {
                lines.push(line);
            }
        }
        if lines.is_empty() {
            return format!("\n    - {} conflicting rule(s)", self.rules.len());
        }
        format!("\n    - {}", lines.join("\n    - "))
    }
}

/// `SolverProblemsException::getPrettyString` header wrapping.
pub fn format_problems(problems: &[Problem], pool: &mut Pool, request: &Request) -> String {
    let mut text = String::from("\n");
    for (i, problem) in problems.iter().enumerate() {
        text.push_str(&format!(
            "  Problem {}{}\n",
            i + 1,
            problem.pretty_string_with_pool(pool, request)
        ));
    }
    text
}

fn rule_priority(rule: &Rule) -> i32 {
    match rule.reason() {
        RuleReason::Fixed { .. } | RuleReason::LockedFilterListRemoved { .. } => 3,
        RuleReason::RootRequire { .. } => 2,
        RuleReason::PackageConflict { .. } | RuleReason::PackageRequires { .. } => 1,
        _ => 0,
    }
}

fn sortable_string(pool: &Pool, rule: &Rule) -> String {
    match rule.reason() {
        RuleReason::RootRequire { package_name, .. } => package_name,
        RuleReason::PackageRequires { target, .. } => {
            let src = rule_source_package(pool, rule);
            format!(
                "{}//{target}",
                src.map(|p| p.pretty_string()).unwrap_or_default()
            )
        }
        RuleReason::PackageConflict { source, .. } => source,
        RuleReason::PackageSameName { package_name } => package_name,
        _ => String::new(),
    }
}

fn rule_source_package<'a>(pool: &'a Pool, rule: &Rule) -> Option<&'a Package> {
    let literals = rule.literals();
    let first = *literals.first()?;
    Some(pool.literal_to_package(first))
}

fn rule_pretty_string(rule: &Rule, pool: &mut Pool, request: &Request) -> String {
    let literals = rule.literals();
    match rule.reason() {
        RuleReason::RootRequire {
            package_name,
            constraint,
        } => {
            let providers = pool
                .what_provides(&package_name, Some(&constraint))
                .unwrap_or_default();
            if providers.is_empty() {
                let named = packages_named(pool, &package_name);
                return root_require_missing(&package_name, &constraint, &named);
            }
            let pkgs: Vec<&Package> = providers
                .iter()
                .map(|&id| pool.package_by_id(id))
                .filter(|p| !p.is_alias())
                .collect();
            format!(
                "Root composer.json requires {package_name}{} -> satisfiable by {}.",
                constraint_pretty_suffix(&constraint),
                format_package_list(&pkgs)
            )
        }
        RuleReason::PackageConflict {
            source,
            target,
            pretty_constraint,
        } => {
            if literals.len() < 2 {
                return String::new();
            }
            let p0 = pool.literal_to_package(literals[0]);
            let p1 = pool.literal_to_package(literals[1]);
            let (conflicter, conflict_target) = if p0.name == source {
                (p0, p1)
            } else {
                (p1, p0)
            };
            let target_text = if conflict_target.name == target {
                conflict_target.pretty_string()
            } else {
                format!(
                    "{target} {} ({})",
                    space_constraint(&pretty_constraint),
                    conflict_target.pretty_string()
                )
            };
            format!(
                "{} conflicts with {target_text}.",
                conflicter.pretty_string()
            )
        }
        RuleReason::PackageRequires {
            target,
            pretty_constraint,
        } => {
            if literals.is_empty() {
                return String::new();
            }
            let source = pool.literal_to_package(literals[0]);
            let providers: Vec<&Package> = literals[1..]
                .iter()
                .map(|&lit| pool.literal_to_package(lit))
                .collect();
            let head = format!(
                "{} requires {target} {}",
                source.pretty_string(),
                space_constraint(&pretty_constraint)
            );
            if providers.is_empty() {
                let named = packages_named(pool, &target);
                if named.is_empty() {
                    format!(
                        "{head} -> could not be found in any version, there may be a typo in the package name."
                    )
                } else {
                    format!(
                        "{head} -> found {} but it does not match the constraint.",
                        format_package_list(&named)
                    )
                }
            } else {
                format!(
                    "{head} -> satisfiable by {}.",
                    format_package_list(&providers)
                )
            }
        }
        RuleReason::PackageSameName { .. } => {
            let pkgs: Vec<&Package> = literals
                .iter()
                .map(|&lit| pool.literal_to_package(lit))
                .collect();
            format!(
                "You can only install one version of a package, so only one of these can be installed: {}.",
                format_package_list(&pkgs)
            )
        }
        RuleReason::Fixed { package_id } => {
            let package = pool.package_by_id(package_id);
            if request.locked_packages.contains_key(&package_id) {
                format!(
                    "{} is locked to version {} and an update of this package was not requested.",
                    package.name, package.pretty_version
                )
            } else {
                format!(
                    "{} is present at version {} and cannot be modified by Composer",
                    package.name, package.pretty_version
                )
            }
        }
        RuleReason::PackageAlias | RuleReason::PackageInverseAlias => String::new(),
        RuleReason::Learned { .. } => {
            if literals.len() == 1 {
                format!(
                    "Conclusion: {} (conflict analysis result)",
                    pool.literal_to_pretty_string(literals[0], &crate::PresentMap::new())
                )
            } else {
                String::new()
            }
        }
        RuleReason::LockedFilterListRemoved { package_id } => {
            let package = pool.package_by_id(package_id);
            format!(
                "{} {} was removed by a dependency policy (e.g. malware) and cannot be installed.",
                package.name, package.pretty_version
            )
        }
    }
}

fn root_require_missing(
    package_name: &str,
    constraint: &ConstraintExpr,
    named: &[&Package],
) -> String {
    if named.is_empty() {
        format!(
            "Root composer.json requires {package_name}{}, it could not be found in any version, there may be a typo in the package name.",
            if matches!(constraint, ConstraintExpr::MatchAll) {
                String::new()
            } else {
                constraint_pretty_suffix(constraint)
            }
        )
    } else {
        format!(
            "Root composer.json requires {package_name}{}, found {} but it does not match the constraint.",
            constraint_pretty_suffix(constraint),
            format_package_list(named)
        )
    }
}

fn packages_named<'a>(pool: &'a Pool, name: &str) -> Vec<&'a Package> {
    pool.packages()
        .iter()
        .filter(|p| p.name == name && !p.is_alias())
        .collect()
}

fn format_package_list(packages: &[&Package]) -> String {
    let mut by_name: indexmap::IndexMap<&str, Vec<&str>> = indexmap::IndexMap::new();
    for package in packages {
        by_name
            .entry(package.name.as_str())
            .or_default()
            .push(package.pretty_version.as_str());
    }
    let mut parts = Vec::new();
    for (name, mut versions) in by_name {
        versions.sort();
        versions.dedup();
        parts.push(format!("{name}[{}]", versions.join(", ")));
    }
    parts.join(", ")
}

fn constraint_pretty_suffix(constraint: &ConstraintExpr) -> String {
    match constraint {
        ConstraintExpr::MatchAll => " *".into(),
        other => {
            let text = constraint_to_compact(other);
            if text.is_empty() {
                String::new()
            } else {
                format!(" {text}")
            }
        }
    }
}

fn constraint_to_compact(constraint: &ConstraintExpr) -> String {
    match constraint {
        ConstraintExpr::MatchAll => "*".into(),
        ConstraintExpr::Simple { operator, version } => {
            format!("{}{}", operator.as_str(), version)
        }
        ConstraintExpr::Multi {
            conjunctive,
            constraints,
        } => {
            let sep = if *conjunctive { ", " } else { " || " };
            constraints
                .iter()
                .map(constraint_to_compact)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(sep)
        }
    }
}

fn space_constraint(pretty: &str) -> String {
    let pretty = pretty.trim();
    if pretty == "*" {
        return "*".into();
    }
    // ">=2.0" / ">= 2.0" / "<1.0" -> ensure space after operator chars
    let op_end = pretty
        .chars()
        .take_while(|c| matches!(c, '>' | '<' | '=' | '!' | '~' | '^'))
        .count();
    if op_end == 0 || op_end >= pretty.len() {
        return pretty.to_string();
    }
    let (op, rest) = pretty.split_at(op_end);
    let rest = rest.trim_start();
    format!("{op} {rest}")
}
