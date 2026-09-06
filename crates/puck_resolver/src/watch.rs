//! Watch graph (`Composer\DependencyResolver\RuleWatch*`).

use crate::decisions::Decisions;
use crate::rule::Rule;
use crate::{Literal, Pool, Result};
use indexmap::IndexMap;
use std::collections::VecDeque;

/// `RuleWatchNode`.
#[derive(Debug)]
pub struct RuleWatchNode {
    pub watch1: Literal,
    pub watch2: Literal,
    rule: Rule,
}

impl RuleWatchNode {
    pub fn new(rule: Rule) -> Self {
        let literals = rule.literals();
        let watch1 = literals.first().copied().unwrap_or(0);
        let watch2 = literals.get(1).copied().unwrap_or(0);
        Self {
            watch1,
            watch2,
            rule,
        }
    }

    pub fn rule(&self) -> &Rule {
        &self.rule
    }

    pub fn watch2_on_highest(&mut self, decisions: &Decisions) {
        let literals = self.rule.literals();
        if literals.len() < 3 || self.rule.is_multi_conflict() {
            return;
        }
        let mut watch_level = 0;
        for literal in literals {
            let level = decisions.decision_level(literal);
            if level > watch_level {
                self.watch2 = literal;
                watch_level = level;
            }
        }
    }

    pub fn other_watch(&self, literal: Literal) -> Literal {
        if self.watch1 == literal {
            self.watch2
        } else {
            self.watch1
        }
    }

    pub fn move_watch(&mut self, from: Literal, to: Literal) {
        if self.watch1 == from {
            self.watch1 = to;
        } else {
            self.watch2 = to;
        }
    }
}

/// `RuleWatchGraph`.
#[derive(Debug, Default)]
pub struct RuleWatchGraph {
    /// Node storage; chains hold indices into this vec.
    nodes: Vec<RuleWatchNode>,
    watch_chains: IndexMap<Literal, VecDeque<usize>>,
}

impl RuleWatchGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, node: RuleWatchNode) {
        if node.rule().is_assertion() {
            return;
        }

        let node_id = self.nodes.len();
        let multi = node.rule().is_multi_conflict();
        let watches: Vec<Literal> = if multi {
            node.rule().literals()
        } else {
            vec![node.watch1, node.watch2]
        };
        self.nodes.push(node);

        for literal in watches {
            self.watch_chains
                .entry(literal)
                .or_default()
                .push_front(node_id);
        }
    }

    /// `RuleWatchGraph::propagateLiteral`.
    pub fn propagate_literal(
        &mut self,
        pool: &Pool,
        decided_literal: Literal,
        level: i32,
        decisions: &mut Decisions,
    ) -> Result<Option<Rule>> {
        let literal = -decided_literal;
        if !self.watch_chains.contains_key(&literal) {
            return Ok(None);
        }

        // Index into the chain; mutate carefully like Composer's SplDoublyLinkedList.
        let mut index = 0;
        loop {
            let chain_len = self
                .watch_chains
                .get(&literal)
                .map(|c| c.len())
                .unwrap_or(0);
            if index >= chain_len {
                break;
            }
            let node_id = self.watch_chains[&literal][index];

            if !self.nodes[node_id].rule().is_multi_conflict() {
                let other_watch = self.nodes[node_id].other_watch(literal);
                let rule = self.nodes[node_id].rule().clone();

                if !rule.is_disabled() && !decisions.satisfy(other_watch) {
                    let rule_literals = rule.literals();
                    let alternative = rule_literals.iter().copied().find(|&rule_literal| {
                        rule_literal != literal
                            && rule_literal != other_watch
                            && !decisions.conflict(rule_literal)
                    });

                    if let Some(alternative_literal) = alternative {
                        self.move_watch(literal, alternative_literal, node_id, index);
                        // Composer `continue` after moveWatch leaves iterator on next via seek.
                        continue;
                    }

                    if decisions.conflict(other_watch) {
                        return Ok(Some(rule));
                    }

                    decisions.decide(pool, other_watch, level, rule)?;
                }
            } else {
                let rule = self.nodes[node_id].rule().clone();
                for other_literal in rule.literals() {
                    if literal != other_literal && !decisions.satisfy(other_literal) {
                        if decisions.conflict(other_literal) {
                            return Ok(Some(rule));
                        }
                        decisions.decide(pool, other_literal, level, rule.clone())?;
                    }
                }
            }

            index += 1;
        }

        Ok(None)
    }

    fn move_watch(
        &mut self,
        from_literal: Literal,
        to_literal: Literal,
        node_id: usize,
        from_index: usize,
    ) {
        self.nodes[node_id].move_watch(from_literal, to_literal);
        if let Some(chain) = self.watch_chains.get_mut(&from_literal) {
            chain.remove(from_index);
        }
        self.watch_chains
            .entry(to_literal)
            .or_default()
            .push_front(node_id);
    }
}
