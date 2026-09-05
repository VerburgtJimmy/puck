//! Write helpers for `composer.lock` (`Composer\Package\Locker::setLockData` / `lockPackages`).

use crate::array_dumper::dump_lock_package_from_p2;
use crate::content_hash::content_hash;
use crate::Result;
use serde_json::{Map, Value, json};

/// Matches Composer `PluginInterface::PLUGIN_API_VERSION` used when fixtures were locked.
pub const PLUGIN_API_VERSION: &str = "2.9.0";

/// Inputs for building a Composer 2 lock document.
#[derive(Debug, Clone)]
pub struct LockWriteInput {
    pub packages: Vec<Value>,
    pub packages_dev: Option<Vec<Value>>,
    pub platform: Map<String, Value>,
    pub platform_dev: Map<String, Value>,
    pub aliases: Vec<Value>,
    pub minimum_stability: String,
    pub stability_flags: Map<String, Value>,
    pub prefer_stable: bool,
    pub prefer_lowest: bool,
    pub plugin_api_version: String,
}

impl Default for LockWriteInput {
    fn default() -> Self {
        Self {
            packages: Vec::new(),
            packages_dev: Some(Vec::new()),
            platform: Map::new(),
            platform_dev: Map::new(),
            aliases: Vec::new(),
            minimum_stability: "stable".into(),
            stability_flags: Map::new(),
            prefer_stable: false,
            prefer_lowest: false,
            plugin_api_version: PLUGIN_API_VERSION.into(),
        }
    }
}

/// Build a `composer.lock` JSON object (`Locker::setLockData` shape).
pub fn build_lock_document(composer_json: &str, mut input: LockWriteInput) -> Result<Value> {
    let hash = content_hash(composer_json)?;

    for package in &mut input.packages {
        *package = format_lock_package(package.clone());
    }
    sort_lock_packages(&mut input.packages);

    let packages_dev = if let Some(mut dev) = input.packages_dev.take() {
        for package in &mut dev {
            *package = format_lock_package(package.clone());
        }
        sort_lock_packages(&mut dev);
        Value::Array(dev)
    } else {
        Value::Null
    };

    Ok(json!({
        "_readme": [
            "This file locks the dependencies of your project to a known state",
            "Read more about it at https://getcomposer.org/doc/01-basic-usage.md#installing-dependencies",
            "This file is @generated automatically",
        ],
        "content-hash": hash,
        "packages": input.packages,
        "packages-dev": packages_dev,
        "aliases": input.aliases,
        "minimum-stability": input.minimum_stability,
        "stability-flags": empty_object_or_map(input.stability_flags),
        "prefer-stable": input.prefer_stable,
        "prefer-lowest": input.prefer_lowest,
        "platform": empty_object_or_map(input.platform),
        "platform-dev": empty_object_or_map(input.platform_dev),
        "plugin-api-version": input.plugin_api_version,
    }))
}

fn empty_object_or_map(map: Map<String, Value>) -> Value {
    Value::Object(map)
}

/// Transform an expanded Packagist p2 version object into a lock package entry.
///
/// Delegates to [`dump_lock_package_from_p2`] (ArrayDumper + Locker::lockPackages).
pub fn format_lock_package(version: Value) -> Value {
    dump_lock_package_from_p2(&version)
}

/// Sort lock package entries by name then version (`Locker::lockPackages` usort).
pub fn sort_lock_packages(packages: &mut [Value]) {
    packages.sort_by(|a, b| {
        let an = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let bn = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
        an.cmp(bn).then_with(|| {
            let av = a.get("version").and_then(|v| v.as_str()).unwrap_or("");
            let bv = b.get("version").and_then(|v| v.as_str()).unwrap_or("");
            av.cmp(bv)
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_lock_package_uses_array_dumper() {
        let formatted = format_lock_package(json!({
            "name": "a/a",
            "version": "1.0.0",
            "version_normalized": "1.0.0.0",
            "time": "2020-01-01T00:00:00+00:00",
            "require": { "z/z": "*", "a/a": "*" },
            "keywords": ["b", "a"],
        }));
        let obj = formatted.as_object().unwrap();
        assert!(!obj.contains_key("version_normalized"));
        assert_eq!(
            obj.get("notification-url").and_then(|v| v.as_str()),
            Some("https://packagist.org/downloads/")
        );
        let keys: Vec<_> = obj.keys().cloned().collect();
        assert_eq!(keys.last().map(String::as_str), Some("time"));
        assert_eq!(obj.get("keywords").unwrap(), &json!(["a", "b"]));
    }

    #[test]
    fn sorts_packages_by_name_then_version() {
        let mut packages = vec![
            json!({"name": "b/b", "version": "2.0.0"}),
            json!({"name": "a/a", "version": "2.0.0"}),
            json!({"name": "a/a", "version": "1.0.0"}),
        ];
        sort_lock_packages(&mut packages);
        assert_eq!(packages[0]["name"], "a/a");
        assert_eq!(packages[0]["version"], "1.0.0");
        assert_eq!(packages[1]["version"], "2.0.0");
        assert_eq!(packages[2]["name"], "b/b");
    }

    #[test]
    fn build_lock_document_sets_content_hash_and_meta() {
        let composer = r#"{
            "name": "puck/minimal-fixture",
            "require": {}
        }"#;
        let doc = build_lock_document(
            composer,
            LockWriteInput {
                packages: vec![json!({
                    "name": "a/a",
                    "version": "1.0.0",
                    "version_normalized": "1.0.0.0"
                })],
                packages_dev: Some(vec![]),
                prefer_stable: true,
                ..LockWriteInput::default()
            },
        )
        .unwrap();
        assert_eq!(doc["content-hash"], "5990a31168e72d02970542ea15afa375");
        assert_eq!(doc["prefer-stable"], true);
        assert_eq!(doc["plugin-api-version"], PLUGIN_API_VERSION);
        assert!(doc["packages"].as_array().unwrap()[0]
            .get("version_normalized")
            .is_none());
        assert!(doc["platform"].as_object().unwrap().is_empty());
        assert!(doc["packages-dev"].as_array().unwrap().is_empty());
    }
}
