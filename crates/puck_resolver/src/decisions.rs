//! Decisions (`Composer\DependencyResolver\Decisions`).

use crate::order::PresentMap;
use crate::rule::Rule;
use crate::{Error, Literal, PackageId, Pool, Result};
use indexmap::IndexMap;

/// Install / remove decisions with reasons.
#[derive(Debug)]
pub struct Decisions {
    /// Package id → signed decision level (lookup only; queue owns order).
    decision_map: IndexMap<PackageId, i32>,
    decision_queue: Vec<(Literal, Rule)>,
}

impl Default for Decisions {
    fn default() -> Self {
        Self::new()
    }
}

impl Decisions {
    pub fn new() -> Self {
        Self {
            decision_map: IndexMap::new(),
            decision_queue: Vec::new(),
        }
    }

    pub fn decide(&mut self, pool: &Pool, literal: Literal, level: i32, why: Rule) -> Result<()> {
        self.add_decision(pool, literal, level)?;
        self.decision_queue.push((literal, why));
        Ok(())
    }

    pub fn satisfy(&self, literal: Literal) -> bool {
        let package_id = literal.unsigned_abs();
        let Some(&level) = self.decision_map.get(&package_id) else {
            return false;
        };
        (literal > 0 && level > 0) || (literal < 0 && level < 0)
    }

    pub fn conflict(&self, literal: Literal) -> bool {
        let package_id = literal.unsigned_abs();
        let Some(&level) = self.decision_map.get(&package_id) else {
            return false;
        };
        (level > 0 && literal < 0) || (level < 0 && literal > 0)
    }

    pub fn decided(&self, literal_or_package_id: Literal) -> bool {
        self.decision_map
            .get(&literal_or_package_id.unsigned_abs())
            .copied()
            .unwrap_or(0)
            != 0
    }

    pub fn undecided(&self, literal_or_package_id: Literal) -> bool {
        !self.decided(literal_or_package_id)
    }

    pub fn decided_install(&self, literal_or_package_id: Literal) -> bool {
        self.decision_map
            .get(&literal_or_package_id.unsigned_abs())
            .copied()
            .unwrap_or(0)
            > 0
    }

    pub fn decision_level(&self, literal_or_package_id: Literal) -> i32 {
        self.decision_map
            .get(&literal_or_package_id.unsigned_abs())
            .copied()
            .unwrap_or(0)
            .abs()
    }

    pub fn decision_rule(&self, literal_or_package_id: Literal) -> Result<Rule> {
        let package_id = literal_or_package_id.unsigned_abs();
        for (literal, reason) in &self.decision_queue {
            if package_id == literal.unsigned_abs() {
                return Ok(reason.clone());
            }
        }
        Err(Error::SolverBug(format!(
            "Did not find a decision rule using {literal_or_package_id}"
        )))
    }

    pub fn at_offset(&self, queue_offset: usize) -> Option<&(Literal, Rule)> {
        self.decision_queue.get(queue_offset)
    }

    pub fn valid_offset(&self, queue_offset: i32) -> bool {
        queue_offset >= 0 && (queue_offset as usize) < self.decision_queue.len()
    }

    pub fn len(&self) -> usize {
        self.decision_queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.decision_queue.is_empty()
    }

    pub fn last_literal(&self) -> Option<Literal> {
        self.decision_queue.last().map(|(l, _)| *l)
    }

    pub fn last_reason(&self) -> Option<Rule> {
        self.decision_queue.last().map(|(_, r)| r.clone())
    }

    pub fn reset(&mut self) {
        while let Some((literal, _)) = self.decision_queue.pop() {
            self.decision_map.insert(literal.unsigned_abs(), 0);
        }
    }

    /// `Decisions::resetToOffset`.
    pub fn reset_to_offset(&mut self, offset: i32) {
        while (self.decision_queue.len() as i32) > offset + 1 {
            if let Some((literal, _)) = self.decision_queue.pop() {
                self.decision_map.insert(literal.unsigned_abs(), 0);
            }
        }
    }

    pub fn revert_last(&mut self) {
        if let Some((literal, _)) = self.decision_queue.pop() {
            self.decision_map.insert(literal.unsigned_abs(), 0);
        }
    }

    /// Iterate decisions oldest-first (Composer iterates reverse for analyzeUnsolvable).
    pub fn iter_rev(&self) -> impl Iterator<Item = &(Literal, Rule)> {
        self.decision_queue.iter().rev()
    }

    fn add_decision(&mut self, pool: &Pool, literal: Literal, level: i32) -> Result<()> {
        let package_id = literal.unsigned_abs();
        let previous = self.decision_map.get(&package_id).copied().unwrap_or(0);
        if previous != 0 {
            let literal_string = pool.literal_to_pretty_string(literal, &PresentMap::new());
            let package = pool.literal_to_package(literal);
            return Err(Error::SolverBug(format!(
                "Trying to decide {literal_string} on level {level}, even though {} was previously decided as {previous}.",
                package.pretty_string()
            )));
        }
        let signed_level = if literal > 0 { level } else { -level };
        self.decision_map.insert(package_id, signed_level);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;
    use crate::rule::{Rule, RuleReason};
    fn pool_one() -> Pool {
        Pool::new(vec![Package::new("a/a", "1.0.0.0", "1.0")])
    }

    #[test]
    fn decide_install_and_satisfy() {
        let pool = pool_one();
        let mut decisions = Decisions::new();
        let why = Rule::generic(vec![1], RuleReason::Learned { why: 0 });
        decisions.decide(&pool, 1, 1, why).unwrap();
        assert!(decisions.satisfy(1));
        assert!(decisions.conflict(-1));
        assert!(decisions.decided_install(1));
        assert_eq!(decisions.decision_level(1), 1);
    }
}
