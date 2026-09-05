//! Package link (`Composer\Package\Link`).

use puck_version::ConstraintExpr;

/// A require / conflict / provide / replace edge.
#[derive(Debug, Clone)]
pub struct Link {
    pub source: String,
    pub target: String,
    pub pretty_constraint: String,
    pub constraint: ConstraintExpr,
}

impl Link {
    pub fn new(
        source: impl Into<String>,
        target: impl Into<String>,
        pretty_constraint: impl Into<String>,
        constraint: ConstraintExpr,
    ) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            pretty_constraint: pretty_constraint.into(),
            constraint,
        }
    }
}
