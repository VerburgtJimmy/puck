//! Rule set (`Composer\DependencyResolver\RuleSet`).

use crate::rule::{Rule, RuleType};
use rustc_hash::FxHashMap;
use std::collections::HashMap;

/// Deduplicating store of solver rules by type.
#[derive(Debug, Default)]
pub struct RuleSet {
    /// Lookup by rule id (`RuleSet::$ruleById`).
    pub rule_by_id: Vec<Rule>,
    rules: HashMap<RuleType, Vec<usize>>,
    rules_by_hash: FxHashMap<u64, Vec<usize>>,
}

impl RuleSet {
    pub fn new() -> Self {
        let mut rules = HashMap::new();
        for t in [RuleType::Package, RuleType::Request, RuleType::Learned] {
            rules.insert(t, Vec::new());
        }
        Self {
            rule_by_id: Vec::new(),
            rules,
            rules_by_hash: FxHashMap::default(),
        }
    }

    /// `RuleSet::add` - skips exact duplicates (any type).
    pub fn add(&mut self, mut rule: Rule, rule_type: RuleType) {
        let hash = rule.hash_key();
        if let Some(ids) = self.rules_by_hash.get(&hash) {
            for &id in ids {
                if rule.equals(&self.rule_by_id[id]) {
                    return;
                }
            }
        }

        rule.set_type(rule_type);
        let id = self.rule_by_id.len();
        self.rule_by_id.push(rule);
        self.rules.entry(rule_type).or_default().push(id);
        self.rules_by_hash.entry(hash).or_default().push(id);
    }

    pub fn len(&self) -> usize {
        self.rule_by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rule_by_id.is_empty()
    }

    pub fn rule_by_id(&self, id: usize) -> &Rule {
        &self.rule_by_id[id]
    }

    pub fn rules_of_type(&self, rule_type: RuleType) -> impl Iterator<Item = &Rule> {
        self.rules
            .get(&rule_type)
            .into_iter()
            .flat_map(|ids| ids.iter().map(|&id| &self.rule_by_id[id]))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Rule> {
        self.rule_by_id.iter()
    }

    pub fn count_type(&self, rule_type: RuleType) -> usize {
        self.rules.get(&rule_type).map(|v| v.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::RuleReason;

    #[test]
    fn add_and_count() {
        let mut set = RuleSet::new();
        set.add(
            Rule::generic(vec![1], RuleReason::RootRequire {
                package_name: "a/a".into(),
            }),
            RuleType::Request,
        );
        set.add(
            Rule::generic(vec![2], RuleReason::RootRequire {
                package_name: "b/b".into(),
            }),
            RuleType::Request,
        );
        assert_eq!(set.len(), 2);
        assert_eq!(set.count_type(RuleType::Request), 2);
    }

    #[test]
    fn add_ignores_duplicates() {
        let mut set = RuleSet::new();
        let reason = RuleReason::RootRequire {
            package_name: String::new(),
        };
        set.add(Rule::generic(vec![], reason.clone()), RuleType::Request);
        set.add(Rule::generic(vec![], reason.clone()), RuleType::Request);
        set.add(Rule::generic(vec![], reason), RuleType::Request);
        assert_eq!(set.count_type(RuleType::Request), 1);
        assert_eq!(set.len(), 1);
    }
}
