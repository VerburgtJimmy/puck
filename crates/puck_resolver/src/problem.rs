//! Solver problems (`Composer\DependencyResolver\Problem` subset).

use crate::rule::Rule;

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
}
