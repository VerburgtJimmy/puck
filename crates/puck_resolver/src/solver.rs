//! CDCL solver (`Composer\DependencyResolver\Solver`).

use crate::decisions::Decisions;
use crate::policy::DefaultPolicy;
use crate::pool::Pool;
use crate::problem::Problem;
use crate::request::Request;
use crate::rule::{Rule, RuleReason, RuleType};
use crate::rule_set::RuleSet;
use crate::rule_set_generator::RuleSetGenerator;
use crate::transaction::Transaction;
use crate::watch::{RuleWatchGraph, RuleWatchNode};
use crate::{Error, Literal, PackageId, Result};
use crate::order::PresentMap;
use indexmap::IndexMap;

/// Composer CDCL dependency solver.
#[derive(Debug)]
pub struct SolverProblems {
    pub problems: Vec<Problem>,
}

impl std::fmt::Display for SolverProblems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for problem in &self.problems {
            write!(f, "{}", problem.pretty_string())?;
        }
        Ok(())
    }
}

impl std::error::Error for SolverProblems {}

/// Composer CDCL dependency solver.
pub struct Solver<'a> {
    policy: DefaultPolicy,
    pool: &'a mut Pool,
    rules: RuleSet,
    watch_graph: RuleWatchGraph,
    decisions: Decisions,
    fixed_map: PresentMap,
    propagate_index: i32,
    branches: Vec<(Vec<Literal>, i32)>,
    problems: Vec<Problem>,
    learned_pool: Vec<Vec<Rule>>,
    learned_why: IndexMap<usize, usize>,
}

impl<'a> Solver<'a> {
    pub fn new(pool: &'a mut Pool) -> Self {
        Self {
            policy: DefaultPolicy::new(false, false),
            pool,
            rules: RuleSet::new(),
            watch_graph: RuleWatchGraph::new(),
            decisions: Decisions::new(),
            fixed_map: PresentMap::new(),
            propagate_index: 0,
            branches: Vec::new(),
            problems: Vec::new(),
            learned_pool: Vec::new(),
            learned_why: IndexMap::new(),
        }
    }

    pub fn with_policy(mut self, prefer_stable: bool, prefer_lowest: bool) -> Self {
        self.policy = DefaultPolicy::new(prefer_stable, prefer_lowest);
        self
    }

    /// `Solver::solve`.
    pub fn solve(
        mut self,
        request: &Request,
        present: &PresentMap,
    ) -> Result<Transaction> {
        self.setup_fixed_map(request);
        self.rules = RuleSetGenerator::new(self.pool).get_rules_for(request)?;
        self.check_for_root_require_problems(request)?;
        self.decisions = Decisions::new();
        self.watch_graph = RuleWatchGraph::new();

        for rule in self.rules.iter() {
            self.watch_graph.insert(RuleWatchNode::new(rule.clone()));
        }

        self.make_assertion_rule_decisions()?;
        self.run_sat()?;

        if !self.problems.is_empty() {
            return Err(Error::Unsolvable(SolverProblems {
                problems: self.problems.clone(),
            }));
        }

        Ok(Transaction::from_decisions(
            self.pool,
            present,
            &self.decisions,
        ))
    }

    fn setup_fixed_map(&mut self, request: &Request) {
        self.fixed_map = request.fixed_packages.clone();
    }

    fn check_for_root_require_problems(&mut self, request: &Request) -> Result<()> {
        let requires: Vec<(String, puck_version::ConstraintExpr)> = request
            .requires
            .iter()
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect();
        for (package_name, constraint) in requires {
            if self
                .pool
                .what_provides(&package_name, Some(&constraint))?
                .is_empty()
            {
                let mut problem = Problem::new();
                problem.add_rule(Rule::generic(
                    vec![],
                    RuleReason::RootRequire {
                        package_name: package_name.clone(),
                        constraint: constraint.clone(),
                    },
                ));
                self.problems.push(problem);
            }
        }
        Ok(())
    }

    fn make_assertion_rule_decisions(&mut self) -> Result<()> {
        let decision_start = self.decisions.len() as i32 - 1;
        let mut rule_index: i32 = 0;
        while (rule_index as usize) < self.rules.len() {
            let rule = self.rules.rule_by_id(rule_index as usize).clone();

            if !rule.is_assertion() || rule.is_disabled() {
                rule_index += 1;
                continue;
            }

            let literals = rule.literals();
            let literal = literals[0];

            if !self.decisions.decided(literal) {
                self.decisions.decide(self.pool, literal, 1, rule)?;
                rule_index += 1;
                continue;
            }

            if self.decisions.satisfy(literal) {
                rule_index += 1;
                continue;
            }

            if rule.rule_type() == RuleType::Learned {
                rule.disable();
                rule_index += 1;
                continue;
            }

            let conflict = self.decisions.decision_rule(literal)?;

            if conflict.rule_type() == RuleType::Package {
                let mut problem = Problem::new();
                problem.add_rule(rule.clone());
                problem.add_rule(conflict);
                rule.disable();
                self.problems.push(problem);
                rule_index += 1;
                continue;
            }

            let mut problem = Problem::new();
            problem.add_rule(rule.clone());
            problem.add_rule(conflict);

            for assert_rule in self.rules.rules_of_type(RuleType::Request) {
                if assert_rule.is_disabled() || !assert_rule.is_assertion() {
                    continue;
                }
                let assert_literals = assert_rule.literals();
                let assert_literal = assert_literals[0];
                if literal.unsigned_abs() != assert_literal.unsigned_abs() {
                    continue;
                }
                problem.add_rule(assert_rule.clone());
                assert_rule.disable();
            }
            self.problems.push(problem);

            self.decisions.reset_to_offset(decision_start);
            rule_index = -1;
            rule_index += 1;
        }
        Ok(())
    }

    fn propagate(&mut self, level: i32) -> Result<Option<Rule>> {
        while self.decisions.valid_offset(self.propagate_index) {
            let (literal, _) = self
                .decisions
                .at_offset(self.propagate_index as usize)
                .expect("valid offset")
                .clone();

            let conflict =
                self.watch_graph
                    .propagate_literal(self.pool, literal, level, &mut self.decisions)?;

            self.propagate_index += 1;

            if conflict.is_some() {
                return Ok(conflict);
            }
        }
        Ok(None)
    }

    fn revert(&mut self, level: i32) {
        while !self.decisions.is_empty() {
            let Some(literal) = self.decisions.last_literal() else {
                break;
            };
            if self.decisions.undecided(literal) {
                break;
            }
            let decision_level = self.decisions.decision_level(literal);
            if decision_level <= level {
                break;
            }
            self.decisions.revert_last();
            self.propagate_index = self.decisions.len() as i32;
        }

        while let Some((_, branch_level)) = self.branches.last() {
            if *branch_level >= level {
                self.branches.pop();
            } else {
                break;
            }
        }
    }

    fn set_propagate_learn(&mut self, mut level: i32, literal: Literal, rule: Rule) -> Result<i32> {
        level += 1;
        self.decisions.decide(self.pool, literal, level, rule)?;

        loop {
            let Some(conflict) = self.propagate(level)? else {
                break;
            };

            if level == 1 {
                self.analyze_unsolvable(&conflict);
                return Ok(0);
            }

            let (learn_literal, new_level, new_rule, why) = self.analyze(level, &conflict)?;

            if new_level <= 0 || new_level >= level {
                return Err(Error::SolverBug(format!(
                    "Trying to revert to invalid level {new_level} from level {level}."
                )));
            }

            level = new_level;
            self.revert(level);
            self.rules.add(new_rule.clone(), RuleType::Learned);
            self.learned_why.insert(new_rule.object_id(), why);

            let mut rule_node = RuleWatchNode::new(new_rule.clone());
            rule_node.watch2_on_highest(&self.decisions);
            self.watch_graph.insert(rule_node);

            self.decisions
                .decide(self.pool, learn_literal, level, new_rule)?;
        }

        Ok(level)
    }

    fn select_and_install(
        &mut self,
        level: i32,
        decision_queue: Vec<Literal>,
        rule: &Rule,
    ) -> Result<i32> {
        let mut literals = self.policy.select_preferred_packages(
            self.pool,
            decision_queue,
            rule.required_package().as_deref(),
        );
        let selected = literals.remove(0);
        if !literals.is_empty() {
            self.branches.push((literals, level));
        }
        self.set_propagate_learn(level, selected, rule.clone())
    }

    fn analyze(&mut self, level: i32, start_rule: &Rule) -> Result<(Literal, i32, Rule, usize)> {
        let analyzed_rule = start_rule.clone();
        let mut rule = start_rule.clone();
        let mut rule_level = 1i32;
        let mut num = 0i32;
        let mut l1num = 0i32;
        let mut seen: IndexMap<PackageId, ()> = IndexMap::new();
        let mut learned_literal: Option<Literal> = None;
        let mut other_learned_literals: Vec<Literal> = Vec::new();
        let mut decision_id = self.decisions.len() as i32;

        self.learned_pool.push(Vec::new());

        'outer: loop {
            let pool_idx = self.learned_pool.len() - 1;
            self.learned_pool[pool_idx].push(rule.clone());

            for literal in rule.literals() {
                if rule.is_multi_conflict() && !self.decisions.decided(literal) {
                    continue;
                }
                if self.decisions.satisfy(literal) {
                    continue;
                }
                let package_id = literal.unsigned_abs();
                if seen.contains_key(&package_id) {
                    continue;
                }
                seen.insert(package_id, ());

                let l = self.decisions.decision_level(literal);
                if l == 1 {
                    l1num += 1;
                } else if level == l {
                    num += 1;
                } else {
                    other_learned_literals.push(literal);
                    if l > rule_level {
                        rule_level = l;
                    }
                }
            }

            let mut l1retry = true;
            while l1retry {
                l1retry = false;

                if num == 0 {
                    l1num -= 1;
                    if l1num == 0 {
                        break 'outer;
                    }
                }

                let literal = loop {
                    if decision_id <= 0 {
                        return Err(Error::SolverBug(format!(
                            "Reached invalid decision id {decision_id} while analyzing rule."
                        )));
                    }
                    decision_id -= 1;
                    let (literal, _) = self
                        .decisions
                        .at_offset(decision_id as usize)
                        .expect("decision")
                        .clone();
                    if seen.contains_key(&literal.unsigned_abs()) {
                        break literal;
                    }
                };

                seen.shift_remove(&literal.unsigned_abs());

                if num != 0 {
                    num -= 1;
                    if num == 0 {
                        learned_literal = Some(-literal);
                        if l1num == 0 {
                            break 'outer;
                        }
                        for other in &other_learned_literals {
                            seen.shift_remove(&other.unsigned_abs());
                        }
                        l1num += 1;
                        l1retry = true;
                        continue;
                    }
                }

                let (_, reason) = self
                    .decisions
                    .at_offset(decision_id as usize)
                    .expect("decision")
                    .clone();
                rule = reason;
                if rule.is_multi_conflict() {
                    for rule_literal in rule.literals() {
                        if !seen.contains_key(&rule_literal.unsigned_abs())
                            && self.decisions.satisfy(-rule_literal)
                        {
                            let pool_idx = self.learned_pool.len() - 1;
                            self.learned_pool[pool_idx].push(rule.clone());
                            let l = self.decisions.decision_level(rule_literal);
                            if l == 1 {
                                l1num += 1;
                            } else if level == l {
                                num += 1;
                            } else {
                                other_learned_literals.push(rule_literal);
                                if l > rule_level {
                                    rule_level = l;
                                }
                            }
                            seen.insert(rule_literal.unsigned_abs(), ());
                            break;
                        }
                    }
                    l1retry = true;
                }
            }

            let (_, reason) = self
                .decisions
                .at_offset(decision_id as usize)
                .expect("decision")
                .clone();
            rule = reason;
        }

        let why = self.learned_pool.len() - 1;
        let Some(learned_literal) = learned_literal else {
            return Err(Error::SolverBug(format!(
                "Did not find a learnable literal in analyzed rule {analyzed_rule:?}."
            )));
        };
        other_learned_literals.insert(0, learned_literal);
        let new_rule = Rule::generic(
            other_learned_literals,
            RuleReason::Learned { why: why as i32 },
        );
        Ok((learned_literal, rule_level, new_rule, why))
    }

    fn analyze_unsolvable_rule(
        &self,
        problem: &mut Problem,
        conflict_rule: &Rule,
        rule_seen: &mut IndexMap<usize, ()>,
    ) {
        let why = conflict_rule.object_id();
        rule_seen.insert(why, ());

        if conflict_rule.rule_type() == RuleType::Learned {
            if let Some(&learned_why) = self.learned_why.get(&why) {
                for problem_rule in &self.learned_pool[learned_why] {
                    if !rule_seen.contains_key(&problem_rule.object_id()) {
                        self.analyze_unsolvable_rule(problem, problem_rule, rule_seen);
                    }
                }
            }
            return;
        }

        if conflict_rule.rule_type() == RuleType::Package {
            return;
        }

        problem.add_rule(conflict_rule.clone());
    }

    fn analyze_unsolvable(&mut self, conflict_rule: &Rule) {
        let mut problem = Problem::new();
        problem.add_rule(conflict_rule.clone());
        let mut rule_seen = IndexMap::new();
        self.analyze_unsolvable_rule(&mut problem, conflict_rule, &mut rule_seen);

        let mut seen: IndexMap<PackageId, ()> = IndexMap::new();
        for literal in conflict_rule.literals() {
            if self.decisions.satisfy(literal) {
                continue;
            }
            seen.insert(literal.unsigned_abs(), ());
        }

        // Collect reasons first to avoid borrow issues
        let mut reasons = Vec::new();
        for (decision_literal, why) in self.decisions.iter_rev() {
            if !seen.contains_key(&decision_literal.unsigned_abs()) {
                continue;
            }
            reasons.push(why.clone());
            for literal in why.literals() {
                if self.decisions.satisfy(literal) {
                    continue;
                }
                seen.insert(literal.unsigned_abs(), ());
            }
        }

        for why in reasons {
            problem.add_rule(why.clone());
            self.analyze_unsolvable_rule(&mut problem, &why, &mut rule_seen);
        }

        self.problems.push(problem);
    }

    fn run_sat(&mut self) -> Result<()> {
        self.propagate_index = 0;
        let mut level = 1;
        let mut system_level = level + 1;

        loop {
            if level == 1 {
                if let Some(conflict) = self.propagate(level)? {
                    self.analyze_unsolvable(&conflict);
                    return Ok(());
                }
            }

            if level < system_level {
                let request_ids: Vec<usize> = self
                    .rules
                    .rule_ids_of_type(RuleType::Request)
                    .to_vec();
                let mut restart_request_pass = false;

                for &rule_id in &request_ids {
                    let rule = self.rules.rule_by_id(rule_id).clone();
                    if !rule.is_enabled() {
                        continue;
                    }

                    let mut decision_queue = Vec::new();
                    let mut none_satisfied = true;
                    for literal in rule.literals() {
                        if self.decisions.satisfy(literal) {
                            none_satisfied = false;
                            break;
                        }
                        if literal > 0 && self.decisions.undecided(literal) {
                            decision_queue.push(literal);
                        }
                    }

                    if none_satisfied && !decision_queue.is_empty() {
                        let pruned: Vec<Literal> = decision_queue
                            .iter()
                            .copied()
                            .filter(|l| self.fixed_map.contains_key(&l.unsigned_abs()))
                            .collect();
                        if !pruned.is_empty() {
                            decision_queue = pruned;
                        }
                    }

                    if none_satisfied && !decision_queue.is_empty() {
                        let o_level = level;
                        level = self.select_and_install(level, decision_queue, &rule)?;
                        if level == 0 {
                            return Ok(());
                        }
                        if level <= o_level {
                            restart_request_pass = true;
                            break;
                        }
                    }
                }

                system_level = level + 1;
                if restart_request_pass {
                    continue;
                }
            }

            if level < system_level {
                system_level = level;
            }

            let mut rules_count = self.rules.len();
            let mut i = 0usize;
            let mut n = 0usize;

            while n < rules_count {
                if i == rules_count {
                    i = 0;
                }

                let rule = self.rules.rule_by_id(i).clone();
                i += 1;
                n += 1;

                if rule.is_disabled() {
                    continue;
                }

                let literals = rule.literals();
                let mut decision_queue = Vec::new();
                let mut skip = false;

                for literal in literals {
                    if literal <= 0 {
                        if !self.decisions.decided_install(literal) {
                            skip = true;
                            break;
                        }
                    } else if self.decisions.decided_install(literal) {
                        skip = true;
                        break;
                    } else if self.decisions.undecided(literal) {
                        decision_queue.push(literal);
                    }
                }

                if skip || decision_queue.len() < 2 {
                    continue;
                }

                level = self.select_and_install(level, decision_queue, &rule)?;
                if level == 0 {
                    return Ok(());
                }

                rules_count = self.rules.len();
                n = 0;
                // i keeps going; Composer sets n = -1 then n++ in for makes n=0
            }

            if level < system_level {
                continue;
            }

            // minimization
            if !self.branches.is_empty() {
                let mut last_literal = None;
                let mut last_level = None;
                let mut last_branch_index = 0;
                let mut last_branch_offset = 0;

                for (bi, (literals, l)) in self.branches.iter().enumerate().rev() {
                    for (offset, &literal) in literals.iter().enumerate() {
                        if literal > 0 && self.decisions.decision_level(literal) > l + 1 {
                            last_literal = Some(literal);
                            last_branch_index = bi;
                            last_branch_offset = offset;
                            last_level = Some(*l);
                        }
                    }
                }

                if let (Some(last_literal), Some(last_level)) = (last_literal, last_level) {
                    self.branches[last_branch_index].0.remove(last_branch_offset);
                    level = last_level;
                    self.revert(level);
                    let why = self
                        .decisions
                        .last_reason()
                        .ok_or_else(|| Error::SolverBug("missing last reason".into()))?;
                    level = self.set_propagate_learn(level, last_literal, why)?;
                    if level == 0 {
                        return Ok(());
                    }
                    continue;
                }
            }

            break;
        }

        Ok(())
    }
}
