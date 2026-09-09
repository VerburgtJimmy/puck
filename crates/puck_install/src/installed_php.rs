//! Generate Composer-shaped `vendor/composer/installed.php`.

use crate::Result;
use crate::plan::{InstallAction, PlannedPackage};
use indexmap::IndexMap;
use puck_lock::{LockFile, LockedPackage};
use puck_manifest::Manifest;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

const DEFAULT_PRETTY_VERSION: &str = "1.0.0+no-version-set";
const DEFAULT_NORMALIZED_VERSION: &str = "1.0.0.0";

static PLATFORM_PACKAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:php(?:-64bit|-ipv6|-zts|-debug)?|hhvm|(?:ext|lib)-[a-z0-9](?:[_.-]?[a-z0-9]+)*|composer(?:-(?:plugin|runtime)-api)?)$",
    )
    .expect("platform package regex")
});

/// Root package fields for `installed.php`.
#[derive(Debug, Clone)]
pub struct RootPackageMeta {
    pub name: String,
    pub package_type: String,
    pub pretty_version: String,
    pub version: String,
    pub reference: Option<String>,
    pub replace: IndexMap<String, String>,
    pub provide: IndexMap<String, String>,
}

impl RootPackageMeta {
    /// Build root metadata from an optional manifest.
    ///
    /// Without a `version` field (and without VersionGuesser), matches Composer’s
    /// auto-versioned root: pretty `1.0.0+no-version-set`, normalized `1.0.0.0`.
    pub fn from_manifest(manifest: Option<&Manifest>) -> Self {
        let Some(manifest) = manifest else {
            return Self {
                name: "__root__".into(),
                package_type: "library".into(),
                pretty_version: DEFAULT_PRETTY_VERSION.into(),
                version: DEFAULT_NORMALIZED_VERSION.into(),
                reference: None,
                replace: IndexMap::new(),
                provide: IndexMap::new(),
            };
        };

        let (pretty_version, version) = match manifest.rest.get("version").and_then(|v| v.as_str())
        {
            Some(raw) if !raw.is_empty() => {
                let normalized = puck_version::normalize(raw).unwrap_or_else(|_| raw.to_owned());
                (raw.to_owned(), normalized)
            }
            _ => (
                DEFAULT_PRETTY_VERSION.to_owned(),
                DEFAULT_NORMALIZED_VERSION.to_owned(),
            ),
        };

        Self {
            name: manifest.name.clone(),
            package_type: manifest.package_type.clone(),
            pretty_version,
            version,
            reference: None,
            replace: manifest.replace.clone(),
            provide: manifest.provide.clone(),
        }
    }
}

/// Render `installed.php` for the active install set.
pub fn render_installed_php(
    lock: &LockFile,
    planned: &[PlannedPackage],
    root: &RootPackageMeta,
    dev_mode: bool,
) -> String {
    let active: HashMap<&str, &PlannedPackage> = planned
        .iter()
        .filter(|p| p.action != InstallAction::Remove)
        .map(|p| (p.name.as_str(), p))
        .collect();

    let mut versions: BTreeMap<String, IndexMap<String, PhpValue>> = BTreeMap::new();

    versions.insert(
        root.name.clone(),
        installed_package_fields(
            &root.pretty_version,
            &root.version,
            root.reference.as_deref(),
            &root.package_type,
            "../../",
            false,
        ),
    );

    for pkg in lock.packages.iter().chain(lock.packages_dev.iter()) {
        let key = pkg.name.to_ascii_lowercase();
        let Some(planned) = active.get(key.as_str()) else {
            continue;
        };
        let pretty = pkg.version.as_str();
        let normalized = package_normalized_version(pkg);
        let reference = package_reference(pkg);
        let pkg_type = package_type(pkg);
        let install_path = format!("../{}", pkg.name.replace('\\', "/"));
        versions.insert(
            key,
            installed_package_fields(
                pretty,
                &normalized,
                reference.as_deref(),
                &pkg_type,
                &install_path,
                planned.is_dev,
            ),
        );
    }

    add_provides_replaces(
        &mut versions,
        &root.replace,
        &root.provide,
        &root.pretty_version,
        false,
    );
    for pkg in lock.packages.iter().chain(lock.packages_dev.iter()) {
        let key = pkg.name.to_ascii_lowercase();
        let Some(planned) = active.get(key.as_str()) else {
            continue;
        };
        let replace = link_map(pkg.extra.get("replace"));
        let provide = link_map(pkg.extra.get("provide"));
        add_provides_replaces(
            &mut versions,
            &replace,
            &provide,
            &pkg.version,
            planned.is_dev,
        );
    }

    for entry in versions.values_mut() {
        for key in ["aliases", "replaced", "provided"] {
            if let Some(PhpValue::List(items)) = entry.get_mut(key) {
                items.sort_by(|a, b| match (a, b) {
                    (PhpValue::String(x), PhpValue::String(y)) => nat_cmp(x, y),
                    _ => std::cmp::Ordering::Equal,
                });
            }
        }
    }

    let mut root_obj = IndexMap::new();
    root_obj.insert("name".into(), PhpValue::String(root.name.clone()));
    root_obj.insert(
        "pretty_version".into(),
        PhpValue::String(root.pretty_version.clone()),
    );
    root_obj.insert("version".into(), PhpValue::String(root.version.clone()));
    root_obj.insert(
        "reference".into(),
        match &root.reference {
            Some(r) => PhpValue::String(r.clone()),
            None => PhpValue::Null,
        },
    );
    root_obj.insert("type".into(), PhpValue::String(root.package_type.clone()));
    root_obj.insert(
        "install_path".into(),
        PhpValue::InstallPath("../../".into()),
    );
    root_obj.insert("aliases".into(), PhpValue::List(Vec::new()));
    root_obj.insert("dev".into(), PhpValue::Bool(dev_mode));

    let mut versions_obj = IndexMap::new();
    for (name, entry) in versions {
        versions_obj.insert(name, PhpValue::Object(entry));
    }

    let mut top = IndexMap::new();
    top.insert("root".into(), PhpValue::Object(root_obj));
    top.insert("versions".into(), PhpValue::Object(versions_obj));

    format!("<?php return {};\n", dump_php(&PhpValue::Object(top), 0))
}

/// Write helper used by execute.
pub fn build_installed_php(
    lock: &LockFile,
    planned: &[PlannedPackage],
    root: &RootPackageMeta,
    dev_mode: bool,
) -> Result<String> {
    Ok(render_installed_php(lock, planned, root, dev_mode))
}

fn installed_package_fields(
    pretty: &str,
    normalized: &str,
    reference: Option<&str>,
    pkg_type: &str,
    install_path: &str,
    dev_requirement: bool,
) -> IndexMap<String, PhpValue> {
    let mut obj = IndexMap::new();
    obj.insert("pretty_version".into(), PhpValue::String(pretty.into()));
    obj.insert("version".into(), PhpValue::String(normalized.into()));
    obj.insert(
        "reference".into(),
        match reference {
            Some(r) => PhpValue::String(r.into()),
            None => PhpValue::Null,
        },
    );
    obj.insert("type".into(), PhpValue::String(pkg_type.into()));
    obj.insert(
        "install_path".into(),
        PhpValue::InstallPath(install_path.into()),
    );
    obj.insert("aliases".into(), PhpValue::List(Vec::new()));
    obj.insert("dev_requirement".into(), PhpValue::Bool(dev_requirement));
    obj
}

fn add_provides_replaces(
    versions: &mut BTreeMap<String, IndexMap<String, PhpValue>>,
    replace: &IndexMap<String, String>,
    provide: &IndexMap<String, String>,
    pretty_version: &str,
    is_dev: bool,
) {
    for (target, constraint) in replace {
        if is_platform_package(target) {
            continue;
        }
        let replaced = if constraint == "self.version" {
            pretty_version.to_owned()
        } else {
            constraint.clone()
        };
        push_link(versions, target, is_dev, "replaced", replaced);
    }
    for (target, constraint) in provide {
        if is_platform_package(target) {
            continue;
        }
        let provided = if constraint == "self.version" {
            pretty_version.to_owned()
        } else {
            constraint.clone()
        };
        push_link(versions, target, is_dev, "provided", provided);
    }
}

fn push_link(
    versions: &mut BTreeMap<String, IndexMap<String, PhpValue>>,
    target: &str,
    is_dev: bool,
    key: &str,
    value: String,
) {
    let name = target.to_ascii_lowercase();
    let entry = versions.entry(name).or_default();
    match entry.get("dev_requirement") {
        None => {
            entry.insert("dev_requirement".into(), PhpValue::Bool(is_dev));
        }
        Some(PhpValue::Bool(true)) if !is_dev => {
            entry.insert("dev_requirement".into(), PhpValue::Bool(false));
        }
        _ => {}
    }
    let list = match entry.entry(key.to_owned()) {
        indexmap::map::Entry::Occupied(o) => match o.into_mut() {
            PhpValue::List(items) => items,
            other => {
                *other = PhpValue::List(Vec::new());
                match entry.get_mut(key) {
                    Some(PhpValue::List(items)) => items,
                    _ => return,
                }
            }
        },
        indexmap::map::Entry::Vacant(v) => {
            v.insert(PhpValue::List(Vec::new()));
            match entry.get_mut(key) {
                Some(PhpValue::List(items)) => items,
                _ => return,
            }
        }
    };
    let already = list
        .iter()
        .any(|v| matches!(v, PhpValue::String(s) if s == &value));
    if !already {
        list.push(PhpValue::String(value));
    }
}

fn link_map(value: Option<&Value>) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    let Some(Value::Object(map)) = value else {
        return out;
    };
    for (k, v) in map {
        if let Some(s) = v.as_str() {
            out.insert(k.to_ascii_lowercase(), s.to_owned());
        }
    }
    out
}

fn package_normalized_version(pkg: &LockedPackage) -> String {
    if let Some(Value::String(n)) = pkg.extra.get("version_normalized") {
        return n.clone();
    }
    puck_version::normalize(&pkg.version).unwrap_or_else(|_| pkg.version.clone())
}

fn package_reference(pkg: &LockedPackage) -> Option<String> {
    if let Some(dist) = &pkg.dist
        && let Some(r) = &dist.reference
        && !r.is_empty()
    {
        return Some(r.clone());
    }
    if let Some(source) = &pkg.source
        && let Some(r) = &source.reference
        && !r.is_empty()
    {
        return Some(r.clone());
    }
    None
}

fn package_type(pkg: &LockedPackage) -> String {
    match pkg.extra.get("type").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t.to_ascii_lowercase(),
        _ => "library".into(),
    }
}

fn is_platform_package(name: &str) -> bool {
    PLATFORM_PACKAGE.is_match(name)
}

#[derive(Debug, Clone)]
enum PhpValue {
    String(String),
    Null,
    Bool(bool),
    List(Vec<PhpValue>),
    Object(IndexMap<String, PhpValue>),
    /// Relative path from `vendor/composer`; emitted as `__DIR__ . '/…'`.
    InstallPath(String),
}

fn dump_php(value: &PhpValue, level: usize) -> String {
    match value {
        PhpValue::Object(map) => dump_object(map, level),
        PhpValue::List(items) => {
            if items.is_empty() {
                return "array()".into();
            }
            let mut out = String::from("array(\n");
            let inner = level + 1;
            for (i, item) in items.iter().enumerate() {
                out.push_str(&"    ".repeat(inner));
                out.push_str(&format!("{i} => {},\n", dump_php(item, inner)));
            }
            out.push_str(&"    ".repeat(level));
            out.push(')');
            out
        }
        PhpValue::String(s) => php_export_string(s),
        PhpValue::Null => "null".into(),
        PhpValue::Bool(b) => if *b { "true" } else { "false" }.into(),
        PhpValue::InstallPath(rel) => {
            let path = if rel.starts_with('/') {
                rel.clone()
            } else {
                format!("/{rel}")
            };
            format!("__DIR__ . {}", php_export_string(&path))
        }
    }
}

fn dump_object(map: &IndexMap<String, PhpValue>, level: usize) -> String {
    let mut out = String::from("array(\n");
    let inner = level + 1;
    for (key, value) in map {
        out.push_str(&"    ".repeat(inner));
        out.push_str(&php_export_string(key));
        out.push_str(" => ");
        match value {
            PhpValue::Object(nested) if nested.is_empty() => out.push_str("array(),\n"),
            PhpValue::List(items) if items.is_empty() => out.push_str("array(),\n"),
            _ => {
                out.push_str(&dump_php(value, inner));
                out.push_str(",\n");
            }
        }
    }
    out.push_str(&"    ".repeat(level));
    out.push(')');
    out
}

fn php_export_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            _ => out.push(ch),
        }
    }
    out.push('\'');
    out
}

/// PHP `SORT_NATURAL`-ish compare for provide/replace lists.
fn nat_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(ac), Some(bc)) if ac.is_ascii_digit() && bc.is_ascii_digit() => {
                let mut an = 0u64;
                while let Some(c) = ai.peek().copied() {
                    if let Some(d) = c.to_digit(10) {
                        an = an.saturating_mul(10).saturating_add(u64::from(d));
                        ai.next();
                    } else {
                        break;
                    }
                }
                let mut bn = 0u64;
                while let Some(c) = bi.peek().copied() {
                    if let Some(d) = c.to_digit(10) {
                        bn = bn.saturating_mul(10).saturating_add(u64::from(d));
                        bi.next();
                    } else {
                        break;
                    }
                }
                match an.cmp(&bn) {
                    std::cmp::Ordering::Equal => {}
                    other => return other,
                }
            }
            (Some(ac), Some(bc)) => {
                ai.next();
                bi.next();
                match ac.cmp(&bc) {
                    std::cmp::Ordering::Equal => {}
                    other => return other,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::InstallAction;
    use std::str::FromStr;

    #[test]
    fn renders_root_and_package_with_dev_requirement() {
        let lock = LockFile::from_str(
            r#"{
                "content-hash": "x",
                "packages": [{
                    "name": "brick/math",
                    "version": "0.18.0",
                    "type": "library",
                    "dist": { "type": "zip", "url": "https://example.test", "reference": "abc123" },
                    "replace": { "mtdowling/cron-expression": "^1.0" }
                }],
                "packages-dev": []
            }"#,
        )
        .expect("lock");
        let planned = vec![PlannedPackage {
            name: "brick/math".into(),
            version: "0.18.0".into(),
            is_dev: false,
            action: InstallAction::Install,
            dist_url: None,
            dist_shasum: None,
            dist_type: None,
            source_type: None,
            source_url: None,
            source_reference: None,
        }];
        let root = RootPackageMeta {
            name: "laravel/laravel".into(),
            package_type: "project".into(),
            pretty_version: DEFAULT_PRETTY_VERSION.into(),
            version: DEFAULT_NORMALIZED_VERSION.into(),
            reference: None,
            replace: IndexMap::new(),
            provide: IndexMap::new(),
        };
        let php = render_installed_php(&lock, &planned, &root, false);
        assert!(php.starts_with("<?php return array("));
        assert!(php.contains("'name' => 'laravel/laravel'"));
        assert!(php.contains("'pretty_version' => '1.0.0+no-version-set'"));
        assert!(php.contains("'version' => '1.0.0.0'"));
        assert!(php.contains("'dev' => false"));
        assert!(php.contains("'brick/math' => array("));
        assert!(php.contains("'version' => '0.18.0.0'"));
        assert!(php.contains("'reference' => 'abc123'"));
        assert!(php.contains("'dev_requirement' => false"));
        assert!(php.contains("'mtdowling/cron-expression'"));
        assert!(php.contains("'replaced' =>"));
        assert!(php.contains("__DIR__ . '/../brick/math'"));
        assert!(php.contains("__DIR__ . '/../../'"));
    }

    #[test]
    fn skips_platform_provides() {
        let lock = LockFile::from_str(
            r#"{
                "content-hash": "x",
                "packages": [{
                    "name": "symfony/polyfill-mbstring",
                    "version": "v1.0.0",
                    "provide": { "ext-mbstring": "*", "psr/foo-implementation": "1.0" }
                }],
                "packages-dev": []
            }"#,
        )
        .expect("lock");
        let planned = vec![PlannedPackage {
            name: "symfony/polyfill-mbstring".into(),
            version: "v1.0.0".into(),
            is_dev: false,
            action: InstallAction::Install,
            dist_url: None,
            dist_shasum: None,
            dist_type: None,
            source_type: None,
            source_url: None,
            source_reference: None,
        }];
        let root = RootPackageMeta::from_manifest(None);
        let php = render_installed_php(&lock, &planned, &root, true);
        assert!(!php.contains("ext-mbstring"));
        assert!(php.contains("psr/foo-implementation"));
    }
}
