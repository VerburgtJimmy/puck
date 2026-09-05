//! Native port of `pestphp/pest-plugin` `DumpCommand` / `Manager::registerPlugins`.
//!
//! Upstream: pestphp/pest-plugin @ v4.0.0.
//! Writes `vendor/pest-plugins.json` (JSON_PRETTY_PRINT class list).

use crate::{Error, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const CACHE_FILE: &str = "pest-plugins.json";
const PLUGIN_VENDOR_DIR: &str = "vendor/pestphp/pest-plugin";


/// Outcome of the native pest plugin dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PestPluginDumpStatus {
    /// Plugin package not installed or not allowed.
    Skipped,
    /// Wrote `vendor/pest-plugins.json`.
    Written { path: PathBuf, plugin_count: usize },
}

/// Dump Pest plugin class names when `pestphp/pest-plugin` is installed.
///
/// Mirrors DumpCommand: walk installed packages (installed.json order), then
/// the root package, merging `extra.pest.plugins`.
pub fn run_pest_plugin_dump(
    project_root: impl AsRef<Path>,
    allow_plugin: bool,
) -> Result<PestPluginDumpStatus> {
    let root = project_root.as_ref();
    if !allow_plugin {
        return Ok(PestPluginDumpStatus::Skipped);
    }

    let plugin_dir = root.join(PLUGIN_VENDOR_DIR);
    if !plugin_dir.is_dir() {
        return Ok(PestPluginDumpStatus::Skipped);
    }

    let installed_path = root.join("vendor/composer/installed.json");
    if !installed_path.is_file() {
        return Ok(PestPluginDumpStatus::Skipped);
    }

    let mut plugins = collect_plugins_from_installed(&installed_path)?;
    plugins.extend(collect_plugins_from_root(root)?);

    let path = root.join("vendor").join(CACHE_FILE);
    let contents = php_json_pretty_print(&plugins);
    fs::write(&path, contents).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;

    Ok(PestPluginDumpStatus::Written {
        path,
        plugin_count: plugins.len(),
    })
}

fn collect_plugins_from_installed(path: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|e| Error::Parse {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let entries = match &value {
        Value::Object(obj) => obj
            .get("packages")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
        Value::Array(_) => value,
        _ => {
            return Err(Error::Parse {
                path: path.display().to_string(),
                message: "installed.json root must be an object or array".into(),
            });
        }
    };
    let Value::Array(entries) = entries else {
        return Err(Error::Parse {
            path: path.display().to_string(),
            message: "installed.json packages must be an array".into(),
        });
    };

    let mut plugins = Vec::new();
    for entry in entries {
        plugins.extend(plugins_from_extra(entry.get("extra")));
    }
    Ok(plugins)
}

fn collect_plugins_from_root(root: &Path) -> Result<Vec<String>> {
    let path = root.join("composer.json");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|e| Error::Parse {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    Ok(plugins_from_extra(value.get("extra")))
}

fn plugins_from_extra(extra: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(arr)) = extra
        .and_then(|e| e.get("pest"))
        .and_then(|p| p.get("plugins"))
    else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect()
}

/// Match PHP `json_encode($plugins, JSON_PRETTY_PRINT)` (4-space indent, no trailing newline).
fn php_json_pretty_print(plugins: &[String]) -> String {
    if plugins.is_empty() {
        return "[]".into();
    }
    let mut out = String::from("[\n");
    for (i, name) in plugins.iter().enumerate() {
        out.push_str("    ");
        out.push_str(&serde_json::to_string(name).unwrap_or_else(|_| format!("\"{name}\"")));
        if i + 1 != plugins.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dumps_merged_plugins_in_installed_then_root_order() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(PLUGIN_VENDOR_DIR)).unwrap();
        fs::create_dir_all(root.join("vendor/composer")).unwrap();
        let installed = json!({
            "packages": [
                {
                    "name": "pestphp/pest",
                    "extra": {
                        "pest": {
                            "plugins": [
                                "Pest\\Mutate\\Plugins\\Mutate",
                                "Pest\\Plugins\\Bail"
                            ]
                        }
                    }
                },
                {
                    "name": "pestphp/pest-plugin-arch",
                    "extra": { "pest": { "plugins": ["Pest\\Arch\\Plugin"] } }
                },
                { "name": "pestphp/pest-plugin", "type": "composer-plugin" }
            ]
        });
        fs::write(
            root.join("vendor/composer/installed.json"),
            serde_json::to_string(&installed).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("composer.json"),
            r#"{"name":"app/app","extra":{"pest":{"plugins":["App\\PestPlugin"]}},"config":{"allow-plugins":{"pestphp/pest-plugin":true}}}"#,
        )
        .unwrap();

        let status = run_pest_plugin_dump(root, true).unwrap();
        match status {
            PestPluginDumpStatus::Written { plugin_count, .. } => assert_eq!(plugin_count, 4),
            other => panic!("expected Written, got {other:?}"),
        }
        let text = fs::read_to_string(root.join("vendor").join(CACHE_FILE)).unwrap();
        assert_eq!(
            text,
            "[\n    \"Pest\\\\Mutate\\\\Plugins\\\\Mutate\",\n    \"Pest\\\\Plugins\\\\Bail\",\n    \"Pest\\\\Arch\\\\Plugin\",\n    \"App\\\\PestPlugin\"\n]"
        );
    }

    #[test]
    fn skips_when_plugin_missing_or_denied() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            run_pest_plugin_dump(dir.path(), true).unwrap(),
            PestPluginDumpStatus::Skipped
        );
        fs::create_dir_all(dir.path().join(PLUGIN_VENDOR_DIR)).unwrap();
        assert_eq!(
            run_pest_plugin_dump(dir.path(), false).unwrap(),
            PestPluginDumpStatus::Skipped
        );
    }
}
