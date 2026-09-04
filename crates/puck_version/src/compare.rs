//! PHP `version_compare` for Composer-normalised version strings.
//!
//! Port of `php_version_compare` / `php_canonicalize_version` from
//! php-src `ext/standard/versioning.c` (PHP 8.4), plus Composer branch rules
//! from `Constraint::versionCompare`.

use crate::constraint::Operator;

/// Compare two version strings the way PHP `version_compare($a, $b, $op)` does,
/// with Composer `dev-` branch special-cases from `Constraint::versionCompare`.
pub fn version_compare(a: &str, b: &str, op: Operator) -> bool {
    version_compare_with_branches(a, b, op, false)
}

/// `Constraint::versionCompare($a, $b, $operator, $compareBranches)`.
pub fn version_compare_with_branches(
    a: &str,
    b: &str,
    op: Operator,
    compare_branches: bool,
) -> bool {
    let a_is_branch = a.starts_with("dev-");
    let b_is_branch = b.starts_with("dev-");

    if op == Operator::Neq && (a_is_branch || b_is_branch) {
        return a != b;
    }

    if a_is_branch && b_is_branch {
        return op == Operator::Eq && a == b;
    }

    // When branches are not comparable, dev branches never match anything.
    if !compare_branches && (a_is_branch || b_is_branch) {
        return false;
    }

    apply_operator(php_version_compare(a, b), op)
}

fn apply_operator(cmp: i8, op: Operator) -> bool {
    match op {
        Operator::Eq => cmp == 0,
        Operator::Neq => cmp != 0,
        Operator::Lt => cmp < 0,
        Operator::Lte => cmp <= 0,
        Operator::Gt => cmp > 0,
        Operator::Gte => cmp >= 0,
    }
}

/// Raw PHP `version_compare` ordering: -1, 0, or 1.
pub fn php_version_compare(orig_ver1: &str, orig_ver2: &str) -> i8 {
    if orig_ver1.is_empty() || orig_ver2.is_empty() {
        if orig_ver1.is_empty() && orig_ver2.is_empty() {
            return 0;
        }
        return if orig_ver1.is_empty() { -1 } else { 1 };
    }

    let ver1 = if orig_ver1.starts_with('#') {
        orig_ver1.to_owned()
    } else {
        canonicalize_version(orig_ver1)
    };
    let ver2 = if orig_ver2.starts_with('#') {
        orig_ver2.to_owned()
    } else {
        canonicalize_version(orig_ver2)
    };

    let parts1: Vec<&str> = split_version_parts(&ver1);
    let parts2: Vec<&str> = split_version_parts(&ver2);

    let mut i = 0;
    let mut compare = 0i8;
    while i < parts1.len() && i < parts2.len() {
        let p1 = parts1[i];
        let p2 = parts2[i];
        compare = compare_parts(p1, p2);
        if compare != 0 {
            break;
        }
        i += 1;
    }

    if compare == 0 {
        if i < parts1.len() {
            let rest = parts1[i];
            if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                compare = 1;
            } else {
                compare = php_version_compare(rest, "#N#");
            }
        } else if i < parts2.len() {
            let rest = parts2[i];
            if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                compare = -1;
            } else {
                compare = php_version_compare("#N#", rest);
            }
        }
    }

    compare
}

fn split_version_parts(ver: &str) -> Vec<&str> {
    if ver.is_empty() {
        return Vec::new();
    }
    ver.split('.').filter(|s| !s.is_empty()).collect()
}

fn compare_parts(p1: &str, p2: &str) -> i8 {
    let d1 = p1.chars().next().is_some_and(|c| c.is_ascii_digit());
    let d2 = p2.chars().next().is_some_and(|c| c.is_ascii_digit());
    if d1 && d2 {
        let l1: i64 = p1.parse().unwrap_or(0);
        let l2: i64 = p2.parse().unwrap_or(0);
        normalize_bool(l1 - l2)
    } else if !d1 && !d2 {
        compare_special_version_forms(p1, p2)
    } else if d1 {
        compare_special_version_forms("#N#", p2)
    } else {
        compare_special_version_forms(p1, "#N#")
    }
}

fn normalize_bool(n: i64) -> i8 {
    match n.cmp(&0) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn compare_special_version_forms(form1: &str, form2: &str) -> i8 {
    normalize_bool(i64::from(special_order(form1)) - i64::from(special_order(form2)))
}

/// Prefix match against PHP special forms; unknown => -1.
fn special_order(form: &str) -> i8 {
    const FORMS: &[(&str, i8)] = &[
        ("dev", 0),
        ("alpha", 1),
        ("a", 1),
        ("beta", 2),
        ("b", 2),
        ("RC", 3),
        ("rc", 3),
        ("#", 4),
        ("pl", 5),
        ("p", 5),
    ];
    for &(name, order) in FORMS {
        if form.starts_with(name) {
            return order;
        }
    }
    -1
}

/// `php_canonicalize_version`
fn canonicalize_version(version: &str) -> String {
    let bytes = version.as_bytes();
    if bytes.is_empty() {
        return String::new();
    }

    let mut buf = String::with_capacity(bytes.len() * 2);
    let mut lp = bytes[0] as char;
    buf.push(lp);

    for &b in &bytes[1..] {
        let p = b as char;
        let lq = buf.chars().last().unwrap_or('\0');
        let is_special = matches!(p, '-' | '_' | '+');
        let is_dig = |c: char| c.is_ascii_digit();
        let is_ndig = |c: char| !c.is_ascii_digit() && c != '.';

        if is_special {
            if lq != '.' {
                buf.push('.');
            }
        } else if (is_ndig(lp) && is_dig(p)) || (is_dig(lp) && is_ndig(p)) {
            if lq != '.' {
                buf.push('.');
            }
            buf.push(p);
        } else if !p.is_ascii_alphanumeric() {
            if lq != '.' {
                buf.push('.');
            }
        } else {
            buf.push(p);
        }
        lp = p;
    }

    if buf.ends_with('.') {
        buf.pop();
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_beta() {
        assert_eq!(canonicalize_version("1.0.0.0-beta1"), "1.0.0.0.beta.1");
    }

    #[test]
    fn compare_dev_below_final() {
        assert_eq!(php_version_compare("1.0.0.0-dev", "1.0.0.0"), -1);
        assert_eq!(php_version_compare("1.0.0.0", "1.0.0.0-dev"), 1);
    }

    #[test]
    fn compare_extra_zero() {
        assert_eq!(php_version_compare("1.0.0.0", "1.0.0"), 1);
    }
}
