//! Mutate `composer.json` require maps (Composer `require` command shape).

use crate::{Error, Result};
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

/// Parsed `vendor/name` or `vendor/name:constraint` CLI argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRequirement {
    pub name: String,
    pub constraint: String,
}

impl PackageRequirement {
    /// Parse `vendor/package` or `vendor/package:constraint` (Composer CLI form).
    pub fn parse(spec: &str) -> Result<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err(Error::Parse("empty package requirement".into()));
        }
        // Constraint may contain `:`; split on first `:` after the name slash.
        let (name, constraint) = match spec.find(':') {
            Some(idx) if spec[..idx].contains('/') => {
                (&spec[..idx], spec[idx + 1..].trim())
            }
            _ => (spec, "*"),
        };
        if !name.contains('/') {
            return Err(Error::Parse(format!(
                "invalid package name {name:?}; expected vendor/package"
            )));
        }
        let constraint = if constraint.is_empty() {
            "*".to_string()
        } else {
            constraint.to_string()
        };
        Ok(Self {
            name: name.to_ascii_lowercase(),
            constraint,
        })
    }
}

/// Whether `config.sort-packages` is enabled (Laravel default true).
pub fn sort_packages_enabled(root: &Value) -> bool {
    root.get("config")
        .and_then(|c| c.get("sort-packages"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Insert or replace a requirement in `require` or `require-dev`.
///
/// When `sort-packages` is true, sorts that object's keys alphabetically
/// (Composer `JsonManipulator` behaviour for Laravel projects).
pub fn add_requirement(root: &mut Value, req: &PackageRequirement, dev: bool) -> Result<()> {
    let sort = sort_packages_enabled(root);
    let obj = root
        .as_object_mut()
        .ok_or_else(|| Error::Parse("composer.json root must be an object".into()))?;
    let key = if dev { "require-dev" } else { "require" };
    let map = obj
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let req_map = map
        .as_object_mut()
        .ok_or_else(|| Error::Parse(format!("{key} must be an object")))?;
    req_map.insert(req.name.clone(), Value::String(req.constraint.clone()));

    if sort {
        sort_string_object_keys(req_map);
    }
    Ok(())
}

fn sort_string_object_keys(map: &mut Map<String, Value>) {
    let old = std::mem::take(map);
    let mut entries: Vec<(String, Value)> = old.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in entries {
        map.insert(k, v);
    }
}

/// Read, mutate require, write `composer.json` with 4-space indent (Composer default).
pub fn add_requirement_to_file(
    path: impl AsRef<Path>,
    req: &PackageRequirement,
    dev: bool,
) -> Result<String> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|e| Error::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let mut root: Value = serde_json::from_str(&text).map_err(|e| Error::Parse(e.to_string()))?;
    add_requirement(&mut root, req, dev)?;
    let out = format!("{}\n", serde_json::to_string_pretty(&root).map_err(|e| Error::Parse(e.to_string()))?);
    // Composer uses 4-space indent; serde_json pretty uses 2. Expand.
    let out = reindent_json_pretty_4(&out);
    fs::write(path, &out).map_err(|e| Error::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    Ok(out)
}

/// Convert serde 2-space pretty JSON to 4-space indent.
fn reindent_json_pretty_4(pretty_2: &str) -> String {
    let mut out = String::with_capacity(pretty_2.len());
    for line in pretty_2.lines() {
        let trimmed = line.trim_start();
        let spaces = line.len() - trimmed.len();
        // serde uses 2 spaces per level
        let level = spaces / 2;
        for _ in 0..level {
            out.push_str("    ");
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_name_and_constraint() {
        let r = PackageRequirement::parse("Foo/Bar:^1.2").unwrap();
        assert_eq!(r.name, "foo/bar");
        assert_eq!(r.constraint, "^1.2");
        let r = PackageRequirement::parse("foo/bar").unwrap();
        assert_eq!(r.constraint, "*");
    }

    #[test]
    fn adds_and_sorts_require() {
        let mut root = json!({
            "name": "app/app",
            "config": { "sort-packages": true },
            "require": {
                "php": "^8.3",
                "laravel/framework": "^13.0"
            }
        });
        add_requirement(
            &mut root,
            &PackageRequirement {
                name: "webmozart/assert".into(),
                constraint: "^1.11".into(),
            },
            false,
        )
        .unwrap();
        let keys: Vec<_> = root["require"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            keys,
            vec!["laravel/framework", "php", "webmozart/assert"]
        );
    }
}
