//! Determinism: Composer’s solver is insertion-ordered (PHP arrays).
//!
//! Use [`IndexMap`] / [`IndexSet`] / [`Vec`] for anything whose iteration order
//! can affect which valid solution CDCL picks or how Transaction orders ops.
//! Do not use `HashMap` / `HashSet` / `FxHashMap` in this crate.
//!
//! Sorts that mirror PHP `sort` / `usort` must use stable [`slice::sort`]
//! (PHP 8+ `usort` is stable), not `sort_unstable`.

use crate::PackageId;
use indexmap::{IndexMap, IndexSet};

/// Present / locked package set (`Request::getPresentMap` shape).
pub type PresentMap = IndexMap<PackageId, ()>;

/// Ordered set of package ids (membership + stable iteration).
pub type PackageIdSet = IndexSet<PackageId>;
