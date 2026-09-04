//! Port of `Composer\Semver\VersionParser` (composer/semver 3.4.3).

use crate::{Error, Result, Stability};
use regex::Regex;
use std::sync::LazyLock;

/// Modifier / pre-release suffix used by Composer.
///
/// PHP source uses possessive quantifiers (`++`, `*+`); the Rust `regex` crate
/// does not support them. Greedy equivalents match Composer on the fixture set.
pub(crate) const MODIFIER: &str =
    r"[._-]?(?:(stable|beta|b|RC|alpha|a|patch|pl|p)((?:[.-]?\d+)*)?)?([.-]?dev)?";
pub(crate) const STABILITIES: &str = r"stable|RC|beta|alpha|dev";

/// Version number + optional stability / `.x-dev` used by constraint parsing.
/// Capture groups: 1 major, 2 minor, 3 patch, 4 revision, 5 stab, 6 stab-num,
/// 7 `-dev`, 8 `.x-dev`.
pub(crate) const VERSION_REGEX: &str = concat!(
    r"v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:\.(\d+))?(?:",
    r"[._-]?(?:(stable|beta|b|RC|alpha|a|patch|pl|p)((?:[.-]?\d+)*)?)?([.-]?dev)?",
    r"|\.([xX*][.-]?dev))(?:\+[^\s]+)?"
);

static RE_ALIAS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^,\s]+) +as +([^,\s]+)$").expect("regex"));
static RE_STABILITY_FLAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)@(?:{STABILITIES})$")).expect("regex"));
static RE_BUILD_META: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^,\s+]+)\+[^\s]+$").expect("regex"));
static RE_CLASSICAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^v?(\d{{1,5}})(\.\d+)?(\.\d+)?(\.\d+)?{MODIFIER}$"
    ))
    .expect("regex")
});
static RE_DATETIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^v?(\d{{4}}(?:[.:-]?\d{{2}}){{1,6}}(?:[.:-]?\d{{1,3}}){{0,2}}){MODIFIER}$"
    ))
    .expect("regex")
});
static RE_DEV_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(.*?)[.-]?dev$").expect("regex"));
static RE_HASH_STRIP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#.+$").expect("regex"));
static RE_MODIFIER_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i){MODIFIER}(?:\+.*)?$")).expect("regex"));
static RE_BRANCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^v?(\d+)(\.(?:\d+|[xX*]))?(\.(?:\d+|[xX*]))?(\.(?:\d+|[xX*]))?$")
        .expect("regex")
});
static RE_NUMERIC_ALIAS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^((?:\d+\.)*\d+)(?:\.x)?-dev$").expect("regex"));
static RE_NON_DIGIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\D").expect("regex"));

/// Normalize a version string the way Composer does.
///
/// See `Composer\Semver\VersionParser::normalize`.
pub fn normalize(version: &str) -> Result<String> {
    normalize_with_full(version, None)
}

fn normalize_with_full(version: &str, full_version: Option<&str>) -> Result<String> {
    let mut version = version.trim().to_owned();
    let orig_version = version.clone();
    let full_version = full_version.unwrap_or(version.as_str()).to_owned();

    if let Some(caps) = RE_ALIAS.captures(&version) {
        version = caps
            .get(1)
            .map(|m| m.as_str().to_owned())
            .unwrap_or(version);
    }

    if let Some(m) = RE_STABILITY_FLAG.find(&version) {
        version = version[..m.start()].to_owned();
    }

    if matches!(version.as_str(), "master" | "trunk" | "default") {
        version = format!("dev-{version}");
    }

    if version.len() >= 4 && version[..4].eq_ignore_ascii_case("dev-") {
        return Ok(format!("dev-{}", &version[4..]));
    }

    if let Some(caps) = RE_BUILD_META.captures(&version) {
        version = caps[1].to_owned();
    }

    let mut matched: Option<(String, usize, regex::Captures<'_>)> = None;

    if let Some(caps) = RE_CLASSICAL.captures(&version) {
        let mut rebuilt = caps[1].to_owned();
        rebuilt.push_str(caps.get(2).map(|m| m.as_str()).unwrap_or(".0"));
        rebuilt.push_str(caps.get(3).map(|m| m.as_str()).unwrap_or(".0"));
        rebuilt.push_str(caps.get(4).map(|m| m.as_str()).unwrap_or(".0"));
        matched = Some((rebuilt, 5, caps));
    } else if let Some(caps) = RE_DATETIME.captures(&version) {
        let rebuilt = RE_NON_DIGIT.replace_all(&caps[1], ".").into_owned();
        matched = Some((rebuilt, 2, caps));
    }

    if let Some((mut version, index, caps)) = matched {
        if let Some(stab) = caps
            .get(index)
            .map(|m| m.as_str())
            .filter(|s| !s.is_empty())
        {
            if stab.eq_ignore_ascii_case("stable") {
                return Ok(version);
            }
            let expanded = expand_stability(stab);
            let num = caps
                .get(index + 1)
                .map(|m| m.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.trim_start_matches(['.', '-']))
                .unwrap_or("");
            version.push('-');
            version.push_str(&expanded);
            version.push_str(num);
        }

        if caps
            .get(index + 2)
            .map(|m| m.as_str())
            .is_some_and(|s| !s.is_empty())
        {
            version.push_str("-dev");
        }

        return Ok(version);
    }

    if let Some(caps) = RE_DEV_SUFFIX.captures(&version) {
        let branch = &caps[1];
        let normalized = normalize_branch(branch);
        if !normalized.starts_with("dev-") {
            return Ok(normalized);
        }
    }

    let mut extra = String::new();
    let quoted = regex::escape(&version);
    let as_alias = Regex::new(&format!(r" +as +{quoted}(?:@(?:{STABILITIES}))?$")).expect("regex");
    let as_source = Regex::new(&format!(r"^{quoted}(?:@(?:{STABILITIES}))? +as +")).expect("regex");
    if as_alias.is_match(&full_version) {
        extra = format!(" in \"{full_version}\", the alias must be an exact version");
    } else if as_source.is_match(&full_version) {
        extra = format!(
            " in \"{full_version}\", the alias source must be an exact version, if it is a branch name you should prefix it with dev-"
        );
    }

    if extra.is_empty() {
        Err(Error::InvalidVersion(orig_version))
    } else {
        Err(Error::InvalidVersionExtra {
            version: orig_version,
            extra,
        })
    }
}

/// `Composer\Semver\VersionParser::parseStability`
pub fn parse_stability(version: &str) -> Stability {
    let version = RE_HASH_STRIP.replace(version, "").into_owned();

    if version.starts_with("dev-") || version.ends_with("-dev") {
        return Stability::Dev;
    }

    let lower = version.to_ascii_lowercase();
    let Some(caps) = RE_MODIFIER_END.captures(&lower) else {
        return Stability::Stable;
    };

    if caps.get(3).is_some_and(|m| !m.as_str().is_empty()) {
        return Stability::Dev;
    }

    if let Some(s) = caps.get(1).map(|m| m.as_str()).filter(|s| !s.is_empty()) {
        return match s {
            "beta" | "b" => Stability::Beta,
            "alpha" | "a" => Stability::Alpha,
            "rc" => Stability::Rc,
            _ => Stability::Stable,
        };
    }

    Stability::Stable
}

/// `Composer\Semver\VersionParser::normalizeStability`
pub fn normalize_stability(stability: &str) -> Result<Stability> {
    match stability.to_ascii_lowercase().as_str() {
        "stable" => Ok(Stability::Stable),
        "rc" => Ok(Stability::Rc),
        "beta" => Ok(Stability::Beta),
        "alpha" => Ok(Stability::Alpha),
        "dev" => Ok(Stability::Dev),
        _ => Err(Error::InvalidStability(stability.to_owned())),
    }
}

/// `Composer\Semver\VersionParser::normalizeBranch`
pub fn normalize_branch(name: &str) -> String {
    let name = name.trim();
    if let Some(caps) = RE_BRANCH.captures(name) {
        let mut version = String::new();
        for i in 1..5 {
            if let Some(m) = caps.get(i) {
                version.push_str(&m.as_str().replace(['*', 'X'], "x"));
            } else {
                version.push_str(".x");
            }
        }
        return format!("{}-dev", version.replace('x', "9999999"));
    }
    format!("dev-{name}")
}

/// `Composer\Semver\VersionParser::parseNumericAliasPrefix`
pub fn parse_numeric_alias_prefix(branch: &str) -> Option<String> {
    RE_NUMERIC_ALIAS
        .captures(branch)
        .map(|caps| format!("{}.", &caps[1]))
}

pub(crate) fn expand_stability(stability: &str) -> String {
    match stability.to_ascii_lowercase().as_str() {
        "a" => "alpha".to_owned(),
        "b" => "beta".to_owned(),
        "p" | "pl" => "patch".to_owned(),
        "rc" => "RC".to_owned(),
        other => other.to_owned(),
    }
}

/// `Composer\Semver\VersionParser::manipulateVersionString`
///
/// `matches` holds version segments at indexes 1..=4 (index 0 unused).
/// Returns `None` on decrement overflow past the major component.
pub(crate) fn manipulate_version_string(
    matches: &[Option<&str>; 5],
    mut position: usize,
    increment: i64,
    pad: &str,
) -> Option<String> {
    let mut parts: [String; 5] = Default::default();
    for i in 1..=4 {
        parts[i] = matches[i].unwrap_or("").to_owned();
    }

    for i in (1..=4).rev() {
        if i > position {
            parts[i] = pad.to_owned();
        } else if i == position && increment != 0 {
            let current = parts[i].parse::<i64>().unwrap_or(0);
            let next = current + increment;
            if next < 0 {
                parts[i] = pad.to_owned();
                if i == 1 {
                    return None;
                }
                position -= 1;
            } else {
                parts[i] = next.to_string();
            }
        }
    }

    Some(format!(
        "{}.{}.{}.{}",
        parts[1], parts[2], parts[3], parts[4]
    ))
}
