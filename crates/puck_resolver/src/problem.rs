//! Solver problems (`Composer\DependencyResolver\Problem` subset).

use crate::rule::{Rule, RuleReason};
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

    /// Minimal `Problem::getPrettyString` for single root-require misses.
    ///
    /// Full Composer formatting (RepositorySet, providers list) comes later.
    pub fn pretty_string(&self) -> String {
        if self.rules.len() == 1 {
            let rule = &self.rules[0];
            if let RuleReason::RootRequire {
                package_name,
                constraint,
            } = rule.reason()
            {
                let constraint_text = constraint_to_text(&constraint);
                return format!(
                    "\n    - Root composer.json requires {package_name}{constraint_text}, it could not be found in any version, there may be a typo in the package name."
                );
            }
        }
        format!("\n    - {} conflicting rule(s)", self.rules.len())
    }
}

fn constraint_to_text(constraint: &ConstraintExpr) -> String {
    match constraint {
        ConstraintExpr::MatchAll => String::new(),
        ConstraintExpr::Simple { operator, version } => {
            format!(" {}{}", operator.as_str(), version)
        }
        ConstraintExpr::Multi {
            conjunctive,
            constraints,
        } => {
            let sep = if *conjunctive { ", " } else { " || " };
            let inner: Vec<String> = constraints
                .iter()
                .map(|c| constraint_to_text(c).trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if inner.is_empty() {
                String::new()
            } else {
                format!(" {}", inner.join(sep))
            }
        }
    }
}
