//! `Composer\Package\Dumper\ArrayDumper` + `Locker::lockPackages` shaping for p2 rows.
//!
//! Ports composer/composer @ `85ae025`. Input is an expanded Packagist p2 version
//! object (after MetadataMinifier expand). Output is a lock package entry.

use serde_json::{Map, Value};

const PACKAGIST_NOTIFICATION_URL: &str = "https://packagist.org/downloads/";

/// Link types in `BasePackage::$supportedLinkTypes` iteration order.
const LINK_TYPES: &[&str] = &["require", "conflict", "provide", "replace", "require-dev"];

/// Keys copied via first `dumpValues` pass (`ArrayDumper::dump`).
const PACKAGE_KEYS: &[&str] = &[
    "bin",
    "type",
    "extra",
    "autoload",
    "autoload-dev",
    "notification-url",
    "include-path",
    "php-ext",
];

/// Keys copied for `CompletePackageInterface` (`ArrayDumper::dump`).
const COMPLETE_KEYS: &[&str] = &[
    "scripts",
    "license",
    "authors",
    "description",
    "homepage",
    "keywords",
    "repositories",
    "support",
    "funding",
];

/// Dump an expanded p2 version into a lock package object.
///
/// Applies ArrayDumper field selection/order, then Locker::lockPackages
/// (drop `version_normalized` / `installation-source`, move `time` last,
/// ensure Packagist `notification-url`).
pub fn dump_lock_package_from_p2(version: &Value) -> Value {
    let Some(src) = version.as_object() else {
        return version.clone();
    };

    let mut data = Map::new();

    insert_string(&mut data, "name", src.get("name"));
    insert_string(&mut data, "version", src.get("version"));
    // version_normalized is dumped by ArrayDumper then stripped by Locker.

    if let Some(v) = src.get("target-dir") {
        if !is_empty_value(v) {
            data.insert("target-dir".into(), v.clone());
        }
    }

    if let Some(source) = src.get("source").and_then(|v| v.as_object()) {
        let mut out = Map::new();
        // ArrayDumper order: type, url, reference, mirrors
        for key in ["type", "url", "reference", "mirrors"] {
            if let Some(v) = source.get(key) {
                if !is_empty_value(v) || key == "reference" {
                    // reference may be present even when empty string? skip empty
                    if !is_empty_value(v) {
                        out.insert(key.into(), v.clone());
                    }
                }
            }
        }
        if !out.is_empty() {
            data.insert("source".into(), Value::Object(out));
        }
    }
    if let Some(dist) = src.get("dist").and_then(|v| v.as_object()) {
        let mut out = Map::new();
        // ArrayDumper order: type, url, reference, shasum, mirrors
        for key in ["type", "url", "reference", "shasum", "mirrors"] {
            if let Some(v) = dist.get(key) {
                // Composer includes empty shasum string
                if key == "shasum" || !is_empty_value(v) {
                    out.insert(key.into(), v.clone());
                }
            }
        }
        if !out.is_empty() {
            data.insert("dist".into(), Value::Object(out));
        }
    }

    for link_type in LINK_TYPES {
        if let Some(Value::Object(map)) = src.get(*link_type) {
            if map.is_empty() {
                continue;
            }
            let mut sorted = map.clone();
            sort_json_object(&mut sorted);
            data.insert((*link_type).into(), Value::Object(sorted));
        }
    }

    if let Some(Value::Object(suggest)) = src.get("suggest") {
        if !suggest.is_empty() {
            let mut sorted = suggest.clone();
            sort_json_object(&mut sorted);
            data.insert("suggest".into(), Value::Object(sorted));
        }
    }

    // time is set during dump then moved to the end by Locker.
    let time = src.get("time").cloned().filter(|v| !is_empty_value(v));

    if src.get("default-branch") == Some(&Value::Bool(true)) {
        data.insert("default-branch".into(), Value::Bool(true));
    }

    for key in PACKAGE_KEYS {
        if *key == "notification-url" {
            continue; // set below (Packagist ComposerRepository)
        }
        copy_if_present(&mut data, src, key);
    }

    // Packagist sets notification URL when loading from the composer repo.
    data.insert(
        "notification-url".into(),
        Value::String(PACKAGIST_NOTIFICATION_URL.into()),
    );

    if let Some(archive) = src.get("archive") {
        if !is_empty_value(archive) {
            data.insert("archive".into(), archive.clone());
        }
    }

    for key in COMPLETE_KEYS {
        if *key == "keywords" {
            if let Some(Value::Array(keywords)) = src.get("keywords") {
                if !keywords.is_empty() {
                    let mut sorted = keywords.clone();
                    sorted.sort_by(|a, b| match (a.as_str(), b.as_str()) {
                        (Some(a), Some(b)) => a.cmp(b),
                        _ => std::cmp::Ordering::Equal,
                    });
                    data.insert("keywords".into(), Value::Array(sorted));
                }
            }
            continue;
        }
        copy_if_present(&mut data, src, key);
    }

    if let Some(abandoned) = src.get("abandoned") {
        if abandoned.as_bool() == Some(true) || abandoned.as_str().is_some() {
            data.insert("abandoned".into(), abandoned.clone());
        }
    }

    if let Some(time) = time {
        data.insert("time".into(), time);
    }

    Value::Object(data)
}


/// Dump a path-repository package into a lock package object.
///
/// Unlike [`dump_lock_package_from_p2`], keeps `transport-options` and does **not**
/// inject Packagist `notification-url`.
pub fn dump_lock_package_from_path(version: &Value) -> Value {
    let Some(src) = version.as_object() else {
        return version.clone();
    };

    let mut data = Map::new();

    insert_string(&mut data, "name", src.get("name"));
    insert_string(&mut data, "version", src.get("version"));

    if let Some(v) = src.get("target-dir") {
        if !is_empty_value(v) {
            data.insert("target-dir".into(), v.clone());
        }
    }

    if let Some(dist) = src.get("dist").and_then(|v| v.as_object()) {
        let mut out = Map::new();
        for key in ["type", "url", "reference", "shasum", "mirrors"] {
            if let Some(v) = dist.get(key) {
                if key == "shasum" || !is_empty_value(v) {
                    out.insert(key.into(), v.clone());
                }
            }
        }
        if !out.is_empty() {
            data.insert("dist".into(), Value::Object(out));
        }
    }

    for link_type in LINK_TYPES {
        if let Some(Value::Object(map)) = src.get(*link_type) {
            if map.is_empty() {
                continue;
            }
            let mut sorted = map.clone();
            sort_json_object(&mut sorted);
            data.insert((*link_type).into(), Value::Object(sorted));
        }
    }

    if let Some(Value::Object(suggest)) = src.get("suggest") {
        if !suggest.is_empty() {
            let mut sorted = suggest.clone();
            sort_json_object(&mut sorted);
            data.insert("suggest".into(), Value::Object(sorted));
        }
    }

    for key in PACKAGE_KEYS {
        if *key == "notification-url" {
            continue;
        }
        copy_if_present(&mut data, src, key);
    }

    for key in COMPLETE_KEYS {
        if *key == "keywords" {
            if let Some(Value::Array(keywords)) = src.get("keywords") {
                if !keywords.is_empty() {
                    let mut sorted = keywords.clone();
                    sorted.sort_by(|a, b| match (a.as_str(), b.as_str()) {
                        (Some(a), Some(b)) => a.cmp(b),
                        _ => std::cmp::Ordering::Equal,
                    });
                    data.insert("keywords".into(), Value::Array(sorted));
                }
            }
            continue;
        }
        copy_if_present(&mut data, src, key);
    }

    if let Some(to) = src.get("transport-options") {
        if !is_empty_value(to) {
            data.insert("transport-options".into(), to.clone());
        }
    }

    Value::Object(data)
}

fn insert_string(data: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(Value::String(s)) = value {
        if !s.is_empty() {
            data.insert(key.into(), Value::String(s.clone()));
        }
    }
}

fn copy_if_present(data: &mut Map<String, Value>, src: &Map<String, Value>, key: &str) {
    if let Some(value) = src.get(key) {
        if !is_empty_value(value) {
            data.insert(key.into(), value.clone());
        }
    }
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

fn sort_json_object(map: &mut Map<String, Value>) {
    let old = std::mem::take(map);
    let mut entries: Vec<(String, Value)> = old.into_iter().collect();
    // PHP `ksort` - string key order.
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in entries {
        map.insert(k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dumps_in_array_dumper_order_with_sorted_keywords() {
        let dumped = dump_lock_package_from_p2(&json!({
            "name": "a/a",
            "version": "1.0.0",
            "version_normalized": "1.0.0.0",
            "published-time": "ignore-me",
            "time": "2020-01-01T00:00:00+00:00",
            "type": "library",
            "require": { "z/z": "*", "a/a": "*" },
            "keywords": ["b", "a"],
            "authors": [{"name": "Ada"}],
            "homepage": "https://example.test",
            "description": "desc",
        }));
        let obj = dumped.as_object().unwrap();
        let keys: Vec<_> = obj.keys().cloned().collect();
        assert!(!keys.iter().any(|k| k == "version_normalized"));
        assert!(!keys.iter().any(|k| k == "published-time"));
        assert_eq!(keys.last().map(String::as_str), Some("time"));
        assert_eq!(obj["notification-url"], PACKAGIST_NOTIFICATION_URL);
        assert_eq!(obj["keywords"], json!(["a", "b"]));
        assert!(obj.contains_key("authors"));
        assert!(obj.contains_key("homepage"));
        let req = obj["require"].as_object().unwrap();
        assert_eq!(
            req.keys().cloned().collect::<Vec<_>>(),
            vec!["a/a", "z/z"]
        );
        // name/version before type; notification-url after type/extra/autoload cluster
        let name_i = keys.iter().position(|k| k == "name").unwrap();
        let type_i = keys.iter().position(|k| k == "type").unwrap();
        let notif_i = keys.iter().position(|k| k == "notification-url").unwrap();
        assert!(name_i < type_i && type_i < notif_i);
    }

    #[test]
    fn dumps_path_package_keeps_transport_options() {
        let dumped = dump_lock_package_from_path(&json!({
            "name": "acme/hello",
            "version": "dev-main",
            "version_normalized": "dev-main",
            "dist": { "type": "path", "url": "packages/acme-hello", "reference": "abc" },
            "type": "library",
            "autoload": { "psr-4": { "Acme\\Hello\\": "src/" } },
            "transport-options": { "symlink": true, "relative": true },
        }));
        let obj = dumped.as_object().unwrap();
        assert_eq!(obj["dist"]["type"], "path");
        assert_eq!(obj["dist"]["url"], "packages/acme-hello");
        assert_eq!(obj["transport-options"]["symlink"], true);
        assert!(!obj.contains_key("notification-url"));
        assert!(!obj.contains_key("version_normalized"));
    }
}
