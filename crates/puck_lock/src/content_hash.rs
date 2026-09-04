//! Composer `content-hash` (Locker::getContentHash).
//!
//! Parity target: Composer 2.8.8 `Composer\Package\Locker::getContentHash`.
//!
//! Algorithm:
//! 1. Parse composer.json (`JsonFile::parseJson` = `json_decode($json, true)`).
//! 2. Keep relevant top-level keys; also `config.platform` when present.
//! 3. `ksort` the relevant map (alphabetical top-level keys only).
//! 4. `md5(JsonFile::encode($relevant, 0))` where encode flag `0` means PHP
//!    `json_encode` defaults: solidus escaped, unicode escaped, compact.

use crate::{Error, Result};
use md5::{Digest, Md5};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Keys included in Composer's content-hash (order before ksort).
const RELEVANT_KEYS: &[&str] = &[
    "name",
    "version",
    "require",
    "require-dev",
    "conflict",
    "replace",
    "provide",
    "minimum-stability",
    "prefer-stable",
    "repositories",
    "extra",
];

/// MD5 content-hash for `composer.json` contents, matching Composer 2.8.8.
pub fn content_hash(composer_json_contents: &str) -> Result<String> {
    let content: Value = serde_json::from_str(composer_json_contents)
        .map_err(|e| Error::ContentHash(format!("invalid composer.json: {e}")))?;
    let obj = content
        .as_object()
        .ok_or_else(|| Error::ContentHash("composer.json root must be an object".into()))?;

    // PHP json_decode(..., true) turns every JSON object into an assoc array.
    // Empty objects therefore re-encode as `[]`, not `{}`.
    let mut relevant: BTreeMap<&str, Value> = BTreeMap::new();
    for key in RELEVANT_KEYS {
        if let Some(value) = obj.get(*key) {
            relevant.insert(*key, php_assoc_normalize(value.clone()));
        }
    }

    // isset($content['config']['platform']) - false when missing or null.
    if let Some(config) = obj.get("config").and_then(Value::as_object) {
        if let Some(platform) = config.get("platform") {
            if !platform.is_null() {
                let mut config_obj = Map::new();
                config_obj.insert(
                    "platform".to_owned(),
                    php_assoc_normalize(platform.clone()),
                );
                relevant.insert("config", Value::Object(config_obj));
            }
        }
    }

    let encoded = encode_php_json_flag0(&relevant)?;
    let digest = Md5::digest(encoded.as_bytes());
    Ok(format!("{digest:x}"))
}

/// Match PHP `json_decode($json, true)` shape for re-encoding: empty objects
/// become empty arrays; nested objects keep key order from the parse.
fn php_assoc_normalize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                Value::Array(Vec::new())
            } else {
                Value::Object(
                    map.into_iter()
                        .map(|(k, v)| (k, php_assoc_normalize(v)))
                        .collect(),
                )
            }
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(php_assoc_normalize).collect())
        }
        other => other,
    }
}

/// PHP `json_encode($data, 0)`: compact, escape `/`, escape non-ASCII.
fn encode_php_json_flag0(value: &BTreeMap<&str, Value>) -> Result<String> {
    let encoded = serde_json::to_string(value)
        .map_err(|e| Error::ContentHash(format!("json encode failed: {e}")))?;
    // serde_json does not escape solidus; PHP with flags=0 does.
    Ok(encoded.replace('/', "\\/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_require_object_becomes_array_and_slashes_escaped() {
        let json = r#"{
            "name": "puck/minimal-fixture",
            "require": {}
        }"#;
        let hash = content_hash(json).expect("hash");
        assert_eq!(hash, "5990a31168e72d02970542ea15afa375");
    }

    #[test]
    fn includes_config_platform_only() {
        let json = r#"{
            "name": "acme/app",
            "require": {"php": "^8.2"},
            "config": {
                "sort-packages": true,
                "platform": {"php": "8.2.0"}
            }
        }"#;
        let hash = content_hash(json).expect("hash");
        assert_eq!(hash, "7614f36dbb15d06132a2e3615e017ed7");
    }
}
