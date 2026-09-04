//! Constraint parsing (Composer `VersionParser::parseConstraints`).
//!
//! Port of composer/semver 3.4.3 constraint AST construction, including
//! `MultiConstraint::create` contiguous-OR collapsing.

use crate::parser::{
    MODIFIER, STABILITIES, VERSION_REGEX, manipulate_version_string, normalize, parse_stability,
};
use crate::{Error, Result, Stability};
use regex::Regex;
use std::fmt;
use std::sync::LazyLock;

static RE_OR_SPLIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\|\|?\s*").expect("regex"));
static RE_ALIAS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^,\s]+) +as +([^,\s]+)$").expect("regex"));
static RE_STABILITY_FLAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)^([^,\s]*?)@({STABILITIES})$")).expect("regex")
});
static RE_HASH_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(dev-[^,\s@]+?|[^,\s@]+?\.x-dev)#.+$").expect("regex")
});
static RE_MATCH_ALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(v)?[xX*](\.[xX*])*$").expect("regex"));
static RE_TILDE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)^~>?{VERSION_REGEX}$")).expect("regex")
});
static RE_CARET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)^\^{VERSION_REGEX}($)")).expect("regex")
});
static RE_X_RANGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:\.[xX*])+$").expect("regex")
});
static RE_HYPHEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^(?P<from>{VERSION_REGEX}) +- +(?P<to>{VERSION_REGEX})($)"
    ))
    .expect("regex")
});
static RE_BASIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(<>|!=|>=?|<=?|==?)?\s*(.*)$").expect("regex"));
static RE_HAS_MODIFIER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)-{MODIFIER}$")).expect("regex"));
static RE_DEV_RECOVER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9a-zA-Z-./]+$").expect("regex"));

/// Comparison operators after Composer normalisation (`=` -> `==`, `<>` -> `!=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
}

impl Operator {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "=" | "==" => Some(Self::Eq),
            "!=" | "<>" => Some(Self::Neq),
            "<" => Some(Self::Lt),
            "<=" => Some(Self::Lte),
            ">" => Some(Self::Gt),
            ">=" => Some(Self::Gte),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Neq => "!=",
            Self::Lt => "<",
            Self::Lte => "<=",
            Self::Gt => ">",
            Self::Gte => ">=",
        }
    }

    /// `str_replace('=', '', $op)` used by `matchSpecific`.
    fn without_equals(self) -> &'static str {
        match self {
            Self::Eq => "",
            Self::Neq => "!",
            Self::Lt => "<",
            Self::Lte => "<",
            Self::Gt => ">",
            Self::Gte => ">",
        }
    }
}

impl fmt::Display for Operator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parsed constraint expression tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintExpr {
    MatchAll,
    Simple {
        operator: Operator,
        version: String,
    },
    Multi {
        conjunctive: bool,
        constraints: Vec<ConstraintExpr>,
    },
}

impl ConstraintExpr {
    pub fn simple(operator: Operator, version: impl Into<String>) -> Self {
        Self::Simple {
            operator,
            version: version.into(),
        }
    }

    /// Composer `Constraint::__toString` form used by contiguous-OR optimisation.
    fn composer_string(&self) -> Option<String> {
        match self {
            Self::Simple { operator, version } => {
                Some(format!("{} {version}", operator.as_str()))
            }
            _ => None,
        }
    }

    /// `ConstraintInterface::matches` - does `self` match the provider constraint.
    ///
    /// For `Semver::satisfies`, call with provider `== normalized_version`.
    pub fn matches_provider(&self, provider: &ConstraintExpr) -> bool {
        match self {
            Self::MatchAll => true,
            Self::Simple { .. } => match provider {
                Self::Simple { .. } => self.match_specific(provider, false),
                _ => provider.matches_provider(self),
            },
            Self::Multi {
                conjunctive,
                constraints,
            } => {
                if !*conjunctive {
                    return constraints
                        .iter()
                        .any(|c| provider.matches_provider(c));
                }
                if let Self::Multi {
                    conjunctive: false, ..
                } = provider
                {
                    return provider.matches_provider(self);
                }
                constraints
                    .iter()
                    .all(|c| provider.matches_provider(c))
            }
        }
    }

    /// `Constraint::matchSpecific`
    fn match_specific(&self, provider: &ConstraintExpr, compare_branches: bool) -> bool {
        let Self::Simple {
            operator: this_op,
            version: this_ver,
        } = self
        else {
            return false;
        };
        let Self::Simple {
            operator: prov_op,
            version: prov_ver,
        } = provider
        else {
            return false;
        };

        let no_equal_op = this_op.without_equals();
        let provider_no_equal_op = prov_op.without_equals();

        let is_equal_op = *this_op == Operator::Eq;
        let is_non_equal_op = *this_op == Operator::Neq;
        let is_provider_equal_op = *prov_op == Operator::Eq;
        let is_provider_non_equal_op = *prov_op == Operator::Neq;

        if is_non_equal_op || is_provider_non_equal_op {
            if is_non_equal_op
                && !is_provider_non_equal_op
                && !is_provider_equal_op
                && prov_ver.starts_with("dev-")
            {
                return false;
            }
            if is_provider_non_equal_op
                && !is_non_equal_op
                && !is_equal_op
                && this_ver.starts_with("dev-")
            {
                return false;
            }
            if !is_equal_op && !is_provider_equal_op {
                return true;
            }
            return crate::compare::version_compare_with_branches(
                prov_ver,
                this_ver,
                Operator::Neq,
                compare_branches,
            );
        }

        if *this_op != Operator::Eq && no_equal_op == provider_no_equal_op {
            return !(this_ver.starts_with("dev-") || prov_ver.starts_with("dev-"));
        }

        let (version1, version2, operator) = if is_equal_op {
            (this_ver.as_str(), prov_ver.as_str(), *prov_op)
        } else {
            (prov_ver.as_str(), this_ver.as_str(), *this_op)
        };

        if crate::compare::version_compare_with_branches(
            version1,
            version2,
            operator,
            compare_branches,
        ) {
            // Special case: e.g. require >= 1.0 and provide < 1.0
            let exclusive_provider = prov_op.as_str() == provider_no_equal_op;
            let inclusive_this = this_op.as_str() != no_equal_op;
            return !(exclusive_provider
                && inclusive_this
                && crate::compare::php_version_compare(prov_ver, this_ver) == 0);
        }

        false
    }
}

/// `Composer\Semver\Semver::satisfies`
pub fn satisfies(version: &str, constraint: &str) -> Result<bool> {
    let normalized = normalize(version)?;
    let parsed = parse_constraints(constraint)?;
    let provider = ConstraintExpr::simple(Operator::Eq, normalized);
    Ok(parsed.matches_provider(&provider))
}

/// `Composer\Semver\VersionParser::parseConstraints`
pub fn parse_constraints(input: &str) -> Result<ConstraintExpr> {
    let trimmed = input.trim();
    let or_parts: Vec<&str> = RE_OR_SPLIT.split(trimmed).collect();
    let mut or_groups = Vec::with_capacity(or_parts.len());

    for or_part in or_parts {
        let and_parts = split_and_constraints(or_part);
        let mut objects = Vec::new();
        if and_parts.len() > 1 {
            for and_part in and_parts {
                objects.extend(parse_constraint(and_part)?);
            }
        } else {
            objects = parse_constraint(and_parts[0])?;
        }

        let group = if objects.len() == 1 {
            objects.remove(0)
        } else {
            ConstraintExpr::Multi {
                conjunctive: true,
                constraints: objects,
            }
        };
        or_groups.push(group);
    }

    Ok(multi_create(or_groups, false))
}

/// `MultiConstraint::create`
fn multi_create(mut constraints: Vec<ConstraintExpr>, conjunctive: bool) -> ConstraintExpr {
    if constraints.is_empty() {
        return ConstraintExpr::MatchAll;
    }
    if constraints.len() == 1 {
        return constraints.remove(0);
    }

    if let Some((optimized, conj)) = optimize_constraints(&constraints, conjunctive) {
        let mut optimized = optimized;
        if optimized.len() == 1 {
            return optimized.remove(0);
        }
        return ConstraintExpr::Multi {
            conjunctive: conj,
            constraints: optimized,
        };
    }

    ConstraintExpr::Multi {
        conjunctive,
        constraints,
    }
}

/// `MultiConstraint::optimizeConstraints` - collapse contiguous OR ranges.
fn optimize_constraints(
    constraints: &[ConstraintExpr],
    conjunctive: bool,
) -> Option<(Vec<ConstraintExpr>, bool)> {
    if conjunctive {
        return None;
    }

    let mut left = constraints[0].clone();
    let mut merged = Vec::new();
    let mut optimized = false;

    for right in constraints.iter().skip(1) {
        if let Some(collapsed) = try_collapse_contiguous(&left, right) {
            optimized = true;
            left = collapsed;
        } else {
            merged.push(left);
            left = right.clone();
        }
    }

    if optimized {
        merged.push(left);
        Some((merged, false))
    } else {
        None
    }
}

fn try_collapse_contiguous(left: &ConstraintExpr, right: &ConstraintExpr) -> Option<ConstraintExpr> {
    let ConstraintExpr::Multi {
        conjunctive: true,
        constraints: left_cs,
    } = left
    else {
        return None;
    };
    let ConstraintExpr::Multi {
        conjunctive: true,
        constraints: right_cs,
    } = right
    else {
        return None;
    };
    if left_cs.len() != 2 || right_cs.len() != 2 {
        return None;
    }

    let left0 = left_cs[0].composer_string()?;
    let left1 = left_cs[1].composer_string()?;
    let right0 = right_cs[0].composer_string()?;
    let right1 = right_cs[1].composer_string()?;

    // ">= X" / "< Y" contiguous with ">= Y" / "< Z"
    if left0.starts_with(">=")
        && left1.starts_with('<')
        && right0.starts_with(">=")
        && right1.starts_with('<')
        && left1.get(2..) == right0.get(3..)
    {
        return Some(ConstraintExpr::Multi {
            conjunctive: true,
            constraints: vec![left_cs[0].clone(), right_cs[1].clone()],
        });
    }
    None
}

/// Split AND constraints. Port of
/// `{(?<!^|as|[=>< ,]) *(?<!-)[, ](?!-) *(?!,|as|$)}` without lookbehind.
fn split_and_constraints(input: &str) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;

    while i < bytes.len() {
        if let Some(match_end) = match_and_separator_at(bytes, i) {
            parts.push(&input[start..i]);
            start = match_end;
            i = match_end;
        } else {
            i += 1;
        }
    }

    parts.push(&input[start..]);
    parts
}

/// Try to match ` *(?<!-)[, ](?!-) *(?!,|as|$)` at `pos`, with the opening
/// lookbehind `(?<!^|as|[=>< ,])` also satisfied.
fn match_and_separator_at(bytes: &[u8], pos: usize) -> Option<usize> {
    // (?<!^|as|[=>< ,])
    if pos == 0 {
        return None;
    }
    let prev = bytes[pos - 1];
    if matches!(prev, b'=' | b'>' | b'<' | b' ' | b',') {
        return None;
    }
    if pos >= 2 && bytes[pos - 2] == b'a' && bytes[pos - 1] == b's' {
        return None;
    }

    // Leading ` *` then `[, ]`, with greedy-backtrack semantics.
    let mut j = pos;
    while j < bytes.len() && bytes[j] == b' ' {
        j += 1;
    }

    // Prefer comma at j; else use the last consumed space as separator.
    let (sep_idx, after_sep) = if j < bytes.len() && bytes[j] == b',' {
        (j, j + 1)
    } else if j > pos {
        (j - 1, j)
    } else {
        return None;
    };

    // (?<!-) immediately before separator
    if sep_idx > 0 && bytes[sep_idx - 1] == b'-' {
        return None;
    }
    // (?!-) immediately after separator
    if after_sep < bytes.len() && bytes[after_sep] == b'-' {
        return None;
    }

    let mut k = after_sep;
    while k < bytes.len() && bytes[k] == b' ' {
        k += 1;
    }

    // (?!,|as|$)
    if k >= bytes.len() {
        return None;
    }
    if bytes[k] == b',' {
        return None;
    }
    if k + 1 < bytes.len() && bytes[k] == b'a' && bytes[k + 1] == b's' {
        return None;
    }

    Some(k)
}

fn parse_constraint(constraint: &str) -> Result<Vec<ConstraintExpr>> {
    let mut constraint = constraint.to_owned();
    let mut stability_modifier: Option<String> = None;

    if let Some(caps) = RE_ALIAS.captures(&constraint) {
        constraint = caps[1].to_owned();
    }

    if let Some(caps) = RE_STABILITY_FLAG.captures(&constraint) {
        let body = caps[1].to_owned();
        let flag = caps[2].to_owned();
        constraint = if body.is_empty() {
            "*".to_owned()
        } else {
            body
        };
        if !flag.eq_ignore_ascii_case("stable") {
            stability_modifier = Some(flag);
        }
    }

    if let Some(caps) = RE_HASH_REF.captures(&constraint) {
        constraint = caps[1].to_owned();
    }

    if let Some(caps) = RE_MATCH_ALL.captures(&constraint) {
        let has_v = caps.get(1).is_some_and(|m| !m.as_str().is_empty());
        let has_rest = caps.get(2).is_some_and(|m| !m.as_str().is_empty());
        if has_v || has_rest {
            return Ok(vec![ConstraintExpr::simple(
                Operator::Gte,
                "0.0.0.0-dev",
            )]);
        }
        return Ok(vec![ConstraintExpr::MatchAll]);
    }

    if let Some(caps) = RE_TILDE.captures(&constraint) {
        if constraint.starts_with("~>") {
            return Err(Error::InvalidConstraint(format!(
                "Could not parse version constraint {constraint}: Invalid operator \"~>\", you probably meant to use the \"~\" operator"
            )));
        }

        let mut position = if capt_nonempty(&caps, 4) {
            4
        } else if capt_nonempty(&caps, 3) {
            3
        } else if capt_nonempty(&caps, 2) {
            2
        } else {
            1
        };
        if capt_nonempty(&caps, 8) {
            position += 1;
        }

        let mut stability_suffix = String::new();
        if !capt_nonempty(&caps, 5) && !capt_nonempty(&caps, 7) && !capt_nonempty(&caps, 8) {
            stability_suffix.push_str("-dev");
        }

        let low_version = normalize(&format!("{}{stability_suffix}", &constraint[1..]))
            .map_err(|e| wrap_constraint_err(&constraint, e))?;
        let segments = version_segments_from_caps(&caps);
        let high_position = position.max(1) - 1;
        let high_position = high_position.max(1);
        let high_version = manipulate_version_string(&segments, high_position, 1, "0")
            .ok_or_else(|| {
                Error::InvalidConstraint(format!(
                    "Could not parse version constraint {constraint}"
                ))
            })?;

        return Ok(vec![
            ConstraintExpr::simple(Operator::Gte, low_version),
            ConstraintExpr::simple(Operator::Lt, format!("{high_version}-dev")),
        ]);
    }

    if let Some(caps) = RE_CARET.captures(&constraint) {
        let position = if capt_str(&caps, 1) != Some("0") || !capt_nonempty(&caps, 2) {
            1
        } else if capt_str(&caps, 2) != Some("0") || !capt_nonempty(&caps, 3) {
            2
        } else {
            3
        };

        let mut stability_suffix = String::new();
        if !capt_nonempty(&caps, 5) && !capt_nonempty(&caps, 7) && !capt_nonempty(&caps, 8) {
            stability_suffix.push_str("-dev");
        }

        let low_version = normalize(&format!("{}{stability_suffix}", &constraint[1..]))
            .map_err(|e| wrap_constraint_err(&constraint, e))?;
        let segments = version_segments_from_caps(&caps);
        let high_version = manipulate_version_string(&segments, position, 1, "0").ok_or_else(|| {
            Error::InvalidConstraint(format!(
                "Could not parse version constraint {constraint}"
            ))
        })?;

        return Ok(vec![
            ConstraintExpr::simple(Operator::Gte, low_version),
            ConstraintExpr::simple(Operator::Lt, format!("{high_version}-dev")),
        ]);
    }

    if let Some(caps) = RE_X_RANGE.captures(&constraint) {
        let position = if capt_nonempty(&caps, 3) {
            3
        } else if capt_nonempty(&caps, 2) {
            2
        } else {
            1
        };

        let segments = version_segments_from_caps(&caps);
        let low_version = manipulate_version_string(&segments, position, 0, "0")
            .ok_or_else(|| {
                Error::InvalidConstraint(format!(
                    "Could not parse version constraint {constraint}"
                ))
            })?;
        let high_version = manipulate_version_string(&segments, position, 1, "0").ok_or_else(|| {
            Error::InvalidConstraint(format!(
                "Could not parse version constraint {constraint}"
            ))
        })?;

        let low = format!("{low_version}-dev");
        let high = format!("{high_version}-dev");
        if low == "0.0.0.0-dev" {
            return Ok(vec![ConstraintExpr::simple(Operator::Lt, high)]);
        }
        return Ok(vec![
            ConstraintExpr::simple(Operator::Gte, low),
            ConstraintExpr::simple(Operator::Lt, high),
        ]);
    }

    if let Some(caps) = RE_HYPHEN.captures(&constraint) {
        let from = caps.name("from").map(|m| m.as_str()).unwrap_or("");
        let to = caps.name("to").map(|m| m.as_str()).unwrap_or("");

        // Named `from` is group 1; version body groups are shifted by +1 vs VERSION_REGEX.
        let mut low_stability_suffix = String::new();
        if !capt_nonempty(&caps, 6) && !capt_nonempty(&caps, 8) && !capt_nonempty(&caps, 9) {
            low_stability_suffix.push_str("-dev");
        }

        let low_version = normalize(from).map_err(|e| wrap_constraint_err(&constraint, e))?;
        let lower = ConstraintExpr::simple(
            Operator::Gte,
            format!("{low_version}{low_stability_suffix}"),
        );

        let upper = if (!php_custom_empty(capt_str(&caps, 12))
            && !php_custom_empty(capt_str(&caps, 13)))
            || capt_nonempty(&caps, 15)
            || capt_nonempty(&caps, 17)
            || capt_nonempty(&caps, 18)
        {
            let high_version = normalize(to).map_err(|e| wrap_constraint_err(&constraint, e))?;
            ConstraintExpr::simple(Operator::Lte, high_version)
        } else {
            // validate to version
            normalize(to).map_err(|e| wrap_constraint_err(&constraint, e))?;
            let high_match = [
                None,
                capt_str(&caps, 11),
                capt_str(&caps, 12),
                capt_str(&caps, 13),
                capt_str(&caps, 14),
            ];
            let pos = if php_custom_empty(capt_str(&caps, 12)) {
                1
            } else {
                2
            };
            let high_version = manipulate_version_string(&high_match, pos, 1, "0").ok_or_else(|| {
                Error::InvalidConstraint(format!(
                    "Could not parse version constraint {constraint}"
                ))
            })?;
            ConstraintExpr::simple(Operator::Lt, format!("{high_version}-dev"))
        };

        return Ok(vec![lower, upper]);
    }

    if let Some(caps) = RE_BASIC.captures(&constraint) {
        let version_raw = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        let op_raw = caps.get(1).map(|m| m.as_str()).filter(|s| !s.is_empty());

        let mut version = match normalize(version_raw) {
            Ok(v) => v,
            Err(e) => {
                if version_raw.ends_with("-dev") && RE_DEV_RECOVER.is_match(version_raw) {
                    let branch = &version_raw[..version_raw.len() - 4];
                    normalize(&format!("dev-{branch}"))
                        .map_err(|e2| wrap_constraint_err(&constraint, e2))?
                } else {
                    return Err(wrap_constraint_err(&constraint, e));
                }
            }
        };

        let op = Operator::parse(op_raw.unwrap_or("=")).ok_or_else(|| {
            Error::InvalidConstraint(format!(
                "Could not parse version constraint {constraint}"
            ))
        })?;

        if !matches!(op, Operator::Eq)
            && stability_modifier.is_some()
            && parse_stability(&version) == Stability::Stable
        {
            if let Some(stab) = stability_modifier {
                version.push('-');
                version.push_str(&stab);
            }
        } else if matches!(op, Operator::Lt | Operator::Gte) {
            let lower_raw = version_raw.to_ascii_lowercase();
            if !RE_HAS_MODIFIER.is_match(&lower_raw) && !version_raw.starts_with("dev-") {
                version.push_str("-dev");
            }
        }

        return Ok(vec![ConstraintExpr::simple(op, version)]);
    }

    Err(Error::InvalidConstraint(format!(
        "Could not parse version constraint {constraint}"
    )))
}

fn capt_str<'a>(caps: &'a regex::Captures<'a>, i: usize) -> Option<&'a str> {
    caps.get(i).map(|m| m.as_str())
}

fn capt_nonempty(caps: &regex::Captures<'_>, i: usize) -> bool {
    caps.get(i).is_some_and(|m| !m.as_str().is_empty())
}

fn version_segments_from_caps<'a>(caps: &'a regex::Captures<'a>) -> [Option<&'a str>; 5] {
    [
        None,
        capt_str(caps, 1),
        capt_str(caps, 2),
        capt_str(caps, 3),
        capt_str(caps, 4),
    ]
}

/// Composer hyphen helper: `0` / `"0"` are treated as present.
fn php_custom_empty(s: Option<&str>) -> bool {
    match s {
        None => true,
        Some("0") => false,
        Some(x) => x.is_empty(),
    }
}

fn wrap_constraint_err(constraint: &str, err: Error) -> Error {
    Error::InvalidConstraint(format!(
        "Could not parse version constraint {constraint}: {err}"
    ))
}
