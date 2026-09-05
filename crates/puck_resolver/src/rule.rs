//! Rules (`Composer\DependencyResolver\Rule`, `GenericRule`, `Rule2Literals`).

use crate::Literal;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Reason constants (`Rule::RULE_*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleReason {
    RootRequire {
        package_name: String,
        constraint: puck_version::ConstraintExpr,
    },
    Fixed { package_id: u32 },
    PackageConflict {
        /// Conflicting package name (link source).
        source: String,
        target: String,
        pretty_constraint: String,
    },
    PackageRequires {
        target: String,
        pretty_constraint: String,
    },
    PackageSameName { package_name: String },
    Learned { why: i32 },
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

/// Shared rule handle (Composer rules are objects by identity).
#[derive(Debug, Clone)]
pub struct Rule(Rc<RefCell<RuleData>>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleData {
    literals: Vec<Literal>,
    reason: RuleReason,
    rule_type: RuleType,
    disabled: bool,
    /// `MultiConflictRule` - watch every literal.
    multi_conflict: bool,
}

impl Rule {
    pub fn generic(mut literals: Vec<Literal>, reason: RuleReason) -> Self {
        // PHP `sort($literals)` - stable since PHP 8.
        literals.sort();
        Self(Rc::new(RefCell::new(RuleData {
            literals,
            reason,
            rule_type: RuleType::Package,
            disabled: false,
            multi_conflict: false,
        })))
    }

    pub fn multi_conflict(mut literals: Vec<Literal>, reason: RuleReason) -> Self {
        // PHP `sort($literals)` - stable since PHP 8.
        literals.sort();
        Self(Rc::new(RefCell::new(RuleData {
            literals,
            reason,
            rule_type: RuleType::Package,
            disabled: false,
            multi_conflict: true,
        })))
    }

    pub fn two_literals(a: Literal, b: Literal, reason: RuleReason) -> Self {
        let (literal1, literal2) = if a < b { (a, b) } else { (b, a) };
        Self(Rc::new(RefCell::new(RuleData {
            literals: vec![literal1, literal2],
            reason,
            rule_type: RuleType::Package,
            disabled: false,
            multi_conflict: false,
        })))
    }

    pub fn ptr_eq(&self, other: &Rule) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub fn object_id(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }

    pub fn literals(&self) -> Vec<Literal> {
        self.0.borrow().literals.clone()
    }

    pub fn reason(&self) -> RuleReason {
        self.0.borrow().reason.clone()
    }

    pub fn rule_type(&self) -> RuleType {
        self.0.borrow().rule_type
    }

    pub fn set_type(&self, rule_type: RuleType) {
        self.0.borrow_mut().rule_type = rule_type;
    }

    pub fn is_assertion(&self) -> bool {
        self.0.borrow().literals.len() == 1
    }

    pub fn is_disabled(&self) -> bool {
        self.0.borrow().disabled
    }

    pub fn is_enabled(&self) -> bool {
        !self.is_disabled()
    }

    pub fn is_multi_conflict(&self) -> bool {
        self.0.borrow().multi_conflict
    }

    pub fn disable(&self) {
        self.0.borrow_mut().disabled = true;
    }

    pub fn enable(&self) {
        self.0.borrow_mut().disabled = false;
    }

    pub fn hash_key(&self) -> u64 {
        let data = self.0.borrow();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data.literals.hash(&mut hasher);
        hasher.finish()
    }

    pub fn equals(&self, other: &Rule) -> bool {
        self.0.borrow().literals == other.0.borrow().literals
    }

    /// `Rule::getRequiredPackage`.
    pub fn required_package(&self) -> Option<String> {
        match &self.0.borrow().reason {
            RuleReason::RootRequire { package_name, .. } => Some(package_name.clone()),
            RuleReason::Fixed { .. } | RuleReason::LockedFilterListRemoved { .. } => None,
            RuleReason::PackageRequires { target, .. } => Some(target.clone()),
            _ => None,
        }
    }
}

impl PartialEq for Rule {
    fn eq(&self, other: &Self) -> bool {
        self.equals(other)
    }
}

impl Eq for Rule {}
