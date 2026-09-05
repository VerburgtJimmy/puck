//! Rules (`Composer\DependencyResolver\Rule`, `GenericRule`, `Rule2Literals`).

use crate::Literal;
use std::hash::{Hash, Hasher};

/// Reason constants (`Rule::RULE_*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleReason {
    RootRequire { package_name: String },
    Fixed { package_id: u32 },
    PackageConflict,
    PackageRequires { target: String },
    PackageSameName { package_name: String },
    Learned { rule_id: i32 },
    PackageAlias,
    PackageInverseAlias,
    LockedFilterListRemoved { package_id: u32 },
}

/// Rule type (`RuleSet::TYPE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RuleType {
    Package = 0,
    Request = 1,
    Learned = 4,
}

/// Enabled SAT clause (Composer `GenericRule` / `Rule2Literals` unified).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    literals: Vec<Literal>,
    reason: RuleReason,
    rule_type: RuleType,
    disabled: bool,
}

impl Rule {
    pub fn generic(mut literals: Vec<Literal>, reason: RuleReason) -> Self {
        literals.sort_unstable();
        Self {
            literals,
            reason,
            rule_type: RuleType::Package,
            disabled: false,
        }
    }

    pub fn two_literals(a: Literal, b: Literal, reason: RuleReason) -> Self {
        let (literal1, literal2) = if a < b { (a, b) } else { (b, a) };
        Self {
            literals: vec![literal1, literal2],
            reason,
            rule_type: RuleType::Package,
            disabled: false,
        }
    }

    pub fn literals(&self) -> &[Literal] {
        &self.literals
    }

    pub fn reason(&self) -> &RuleReason {
        &self.reason
    }

    pub fn rule_type(&self) -> RuleType {
        self.rule_type
    }

    pub fn set_type(&mut self, rule_type: RuleType) {
        self.rule_type = rule_type;
    }

    pub fn is_assertion(&self) -> bool {
        self.literals.len() == 1
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn disable(&mut self) {
        self.disabled = true;
    }

    pub fn enable(&mut self) {
        self.disabled = false;
    }

    /// Hash for duplicate detection (`GenericRule::getHash` / `Rule2Literals::getHash`).
    ///
    /// Composer uses xxh3; we use a stable Fx-style hash of the literal list.
    pub fn hash_key(&self) -> u64 {
        let mut hasher = rustc_hash::FxHasher::default();
        self.literals.hash(&mut hasher);
        hasher.finish()
    }

    /// Ignores disabled bit (`Rule::equals`).
    pub fn equals(&self, other: &Rule) -> bool {
        self.literals == other.literals
    }
}
