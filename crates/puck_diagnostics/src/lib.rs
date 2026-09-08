//! puck doctor: preflight blockers, warnings, and info for switching from Composer.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use indexmap::IndexMap;
use puck_lock::{Abandoned, LockFile, PLUGIN_API_VERSION, content_hash};
use puck_manifest::Manifest;
use puck_php::find_php;
use puck_plugins::unsupported_allowed_plugins;
use puck_store::Store;
use puck_version::satisfies;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Composer plugins that lack a native adapter today but are listed as planned in plugins.md.
pub const PLANNED_ADAPTERS: &[&str] = &[
    "dealerdirect/phpcodesniffer-composer-installer",
    "php-http/discovery",
    "composer/installers",
    "cweagans/composer-patches",
];

/// `config` keys puck understands (others present → warning).
pub const KNOWN_CONFIG_KEYS: &[&str] = &[
    "sort-packages",
    "allow-plugins",
    "optimize-autoloader",
    "preferred-install",
    "platform",
    "audit",
    "max-parallel-http",
];

const LARAVEL_COMPOSER_SCRIPTS: &str = "Illuminate\\Foundation\\ComposerScripts";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Blocker,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// Short kind label for columns / JSON (e.g. `unsupported plugin`).
    pub category: String,
    /// Primary identifier (package name, path, key, ...).
    pub subject: String,
    /// Extra detail; empty when unused.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl Finding {
    pub fn new(
        severity: Severity,
        category: impl Into<String>,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            category: category.into(),
            subject: subject.into(),
            detail: detail.into(),
        }
    }

    fn severity_label(&self) -> &'static str {
        match self.severity {
            Severity::Blocker => "blocker",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn blockers(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Blocker)
    }

    pub fn blocker_count(&self) -> usize {
        self.blockers().count()
    }

    pub fn exit_code(&self) -> u8 {
        if self.blocker_count() > 0 { 1 } else { 0 }
    }

    pub fn summary_line(&self) -> String {
        let n = self.blocker_count();
        if n == 0 {
            "ready to switch".to_owned()
        } else if n == 1 {
            "1 blocker".to_owned()
        } else {
            format!("{n} blockers")
        }
    }
}

/// Run doctor against `working_dir` (project root with composer.json / lock).
pub fn diagnose(working_dir: &Path) -> Result<Report> {
    let mut report = Report::default();

    if cfg!(windows) {
        report.findings.push(Finding::new(
            Severity::Blocker,
            "platform",
            "windows",
            "macOS and Linux only",
        ));
    }

    let manifest_path = working_dir.join("composer.json");
    let lock_path = working_dir.join("composer.lock");

    if !manifest_path.is_file() {
        return Err(Error::Message(format!(
            "no composer.json in {}",
            working_dir.display()
        )));
    }
    if !lock_path.is_file() {
        return Err(Error::Message(format!(
            "no composer.lock in {}",
            working_dir.display()
        )));
    }

    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| Error::Message(format!("read {}: {e}", manifest_path.display())))?;
    let manifest_value: Value = serde_json::from_str(&manifest_text)
        .map_err(|e| Error::Message(format!("parse composer.json: {e}")))?;
    let manifest = Manifest::from_value(manifest_value)
        .map_err(|e| Error::Message(format!("parse composer.json: {e}")))?;
    let lock = LockFile::from_path(&lock_path)
        .map_err(|e| Error::Message(format!("parse composer.lock: {e}")))?;

    collect_repository_blockers(&manifest, &mut report);
    collect_plugin_blockers(&manifest, &lock, &mut report);
    collect_lock_identity_blockers(&manifest_text, &lock, &mut report);
    collect_script_warnings(&manifest, &mut report);
    collect_config_warnings(&manifest, &mut report);
    collect_php_warnings(&manifest, &lock, &mut report);
    collect_abandoned_warnings(&lock, &mut report);
    collect_info(&manifest, &mut report);

    Ok(report)
}

fn collect_repository_blockers(manifest: &Manifest, report: &mut Report) {
    for (label, repo_type) in unsupported_repositories(&manifest.repositories) {
        report.findings.push(Finding::new(
            Severity::Blocker,
            "repository",
            label,
            format!("type `{repo_type}` (vcs / artifact / package not supported)"),
        ));
    }
}

/// Extract `(label, type)` for unsupported repository types.
pub fn unsupported_repositories(repositories: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    match repositories {
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                if let Some(t) = repo_type(item)
                    && is_unsupported_repo_type(&t)
                {
                    let label = repo_label(item).unwrap_or_else(|| format!("#{i}"));
                    out.push((label, t));
                }
            }
        }
        Value::Object(map) => {
            for (name, item) in map {
                if let Some(t) = repo_type(item)
                    && is_unsupported_repo_type(&t)
                {
                    out.push((name.clone(), t));
                }
            }
        }
        _ => {}
    }
    out
}

fn repo_type(item: &Value) -> Option<String> {
    item.get("type")
        .and_then(Value::as_str)
        .map(|s| s.to_ascii_lowercase())
}

fn repo_label(item: &Value) -> Option<String> {
    item.get("url")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            item.get("package")
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

fn is_unsupported_repo_type(t: &str) -> bool {
    matches!(t, "vcs" | "artifact" | "package")
}

fn collect_plugin_blockers(manifest: &Manifest, lock: &LockFile, report: &mut Report) {
    let pkgs = lock
        .packages
        .iter()
        .chain(lock.packages_dev.iter())
        .map(|p| (p.name.clone(), p.package_type().to_string()));
    let allows = |name: &str| manifest.allows_plugin(name);
    for name in unsupported_allowed_plugins(pkgs, allows) {
        let planned = PLANNED_ADAPTERS
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&name));
        let note = if planned {
            "adapter planned"
        } else {
            "adapter not planned"
        };
        report.findings.push(Finding::new(
            Severity::Blocker,
            "unsupported plugin",
            name,
            format!("no native adapter ({note})"),
        ));
    }
}

fn collect_lock_identity_blockers(manifest_text: &str, lock: &LockFile, report: &mut Report) {
    match content_hash(manifest_text) {
        Ok(expected) => {
            if expected != lock.content_hash {
                report.findings.push(Finding::new(
                    Severity::Blocker,
                    "content-hash",
                    lock.content_hash.clone(),
                    format!("puck recomputed {expected}"),
                ));
            }
        }
        Err(e) => {
            report.findings.push(Finding::new(
                Severity::Blocker,
                "content-hash",
                "composer.json",
                format!("cannot recompute: {e}"),
            ));
        }
    }

    let lock_api = lock.plugin_api_version.as_deref().unwrap_or("");
    if lock_api != PLUGIN_API_VERSION {
        report.findings.push(Finding::new(
            Severity::Blocker,
            "plugin-api-version",
            lock_api,
            format!("puck expects {PLUGIN_API_VERSION}"),
        ));
    }
}

/// Class::method script handlers puck does not run like Composer (non-Laravel).
pub fn non_laravel_class_method_scripts(scripts: &IndexMap<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for handlers in scripts.values() {
        for handler in flatten_script_handlers(handlers) {
            if let Some(class_method) = class_method_callback(&handler) {
                if class_method.starts_with(LARAVEL_COMPOSER_SCRIPTS) {
                    continue;
                }
                if seen.insert(handler.clone()) {
                    out.push(handler);
                }
            }
        }
    }
    out
}

fn flatten_script_handlers(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn class_method_callback(handler: &str) -> Option<&str> {
    let trimmed = handler.trim();
    if trimmed.starts_with('@') {
        return None;
    }
    // Require a PHP-ish Class::method (backslash or letter before ::).
    let (class, method) = trimmed.split_once("::")?;
    if class.is_empty() || method.is_empty() {
        return None;
    }
    if method.contains(' ') || method.contains('"') || method.contains('\'') {
        return None;
    }
    // Shell-ish `@php artisan` already excluded; skip `@`-aliases and plain commands.
    if !class.contains('\\') && !class.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return None;
    }
    Some(trimmed)
}

fn collect_script_warnings(manifest: &Manifest, report: &mut Report) {
    for handler in non_laravel_class_method_scripts(&manifest.scripts) {
        report.findings.push(Finding::new(
            Severity::Warning,
            "script",
            handler,
            "Class::method callback puck skips or runs differently",
        ));
    }
}

/// Config keys present under `config` that puck does not honour.
pub fn unknown_config_keys(config: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut out: Vec<String> = config
        .keys()
        .filter(|k| {
            !KNOWN_CONFIG_KEYS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(k))
        })
        .cloned()
        .collect();
    out.sort();
    out
}

fn collect_config_warnings(manifest: &Manifest, report: &mut Report) {
    for key in unknown_config_keys(&manifest.config) {
        report.findings.push(Finding::new(
            Severity::Warning,
            "config",
            key,
            "ignored by puck",
        ));
    }
}

fn collect_php_warnings(manifest: &Manifest, lock: &LockFile, report: &mut Report) {
    let Some(php_path) = find_php() else {
        report.findings.push(Finding::new(
            Severity::Warning,
            "php",
            "PATH",
            "no PHP found (scripts and some checks need php)",
        ));
        return;
    };
    let Some(version) = php_version_string(&php_path) else {
        report.findings.push(Finding::new(
            Severity::Warning,
            "php",
            php_path.display().to_string(),
            "could not read version",
        ));
        return;
    };

    if let Some(platform_php) = manifest
        .config
        .get("platform")
        .and_then(|p| p.get("php"))
        .and_then(Value::as_str)
    {
        // config.platform.php is a pinned version Composer pretends to run;
        // PATH PHP should still be able to satisfy `==` that version loosely via the
        // project's real constraint - warn when PATH PHP fails `== platform` when
        // used as a constraint string, else when it fails lock platform.
        let constraint = if platform_php.contains('^')
            || platform_php.contains('~')
            || platform_php.contains('*')
            || platform_php.contains('|')
            || platform_php.contains('>')
            || platform_php.contains('<')
            || platform_php.contains(' ')
        {
            platform_php.to_owned()
        } else {
            format!("=={platform_php}")
        };
        match satisfies(&version, &constraint) {
            Ok(false) => {
                report.findings.push(Finding::new(
                    Severity::Warning,
                    "platform php",
                    platform_php.to_owned(),
                    format!("host {version} does not satisfy config.platform.php"),
                ));
            }
            Err(_) => {}
            Ok(true) => {}
        }
    }

    if let Some(Value::String(constraint)) = lock.platform.get("php") {
        match satisfies(&version, constraint) {
            Ok(false) => {
                report.findings.push(Finding::new(
                    Severity::Warning,
                    "platform php",
                    constraint.clone(),
                    format!("host {version} does not satisfy lock platform.php"),
                ));
            }
            Err(_) => {}
            Ok(true) => {}
        }
    }
}

fn php_version_string(php: &Path) -> Option<String> {
    let output = Command::new(php)
        .arg("-r")
        .arg("echo PHP_VERSION;")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if v.is_empty() { None } else { Some(v) }
}

fn collect_abandoned_warnings(lock: &LockFile, report: &mut Report) {
    for pkg in lock.packages.iter().chain(lock.packages_dev.iter()) {
        let Some(abandoned) = pkg.abandoned() else {
            continue;
        };
        let detail = match &abandoned {
            Abandoned::NoReplacement => "no replacement suggested".to_owned(),
            Abandoned::Replacement(r) => format!("use {r} instead"),
        };
        report.findings.push(Finding::new(
            Severity::Warning,
            "abandoned package",
            pkg.name.clone(),
            detail,
        ));
    }
}

fn collect_info(manifest: &Manifest, report: &mut Report) {
    match find_php() {
        Some(path) => {
            let why = if std::env::var_os("PHP_BINARY").is_some() {
                "PHP_BINARY"
            } else {
                "PATH"
            };
            let detail = match php_version_string(&path) {
                Some(v) => format!("{v} (from {why})"),
                None => format!("from {why}"),
            };
            report.findings.push(Finding::new(
                Severity::Info,
                "php",
                path.display().to_string(),
                detail,
            ));
        }
        None => {
            report.findings.push(Finding::new(
                Severity::Info,
                "php",
                "(none)",
                "no PHP on PATH / PHP_BINARY",
            ));
        }
    }

    let optimize = manifest.optimize_autoloader();
    report.findings.push(Finding::new(
        Severity::Info,
        "optimize-autoloader",
        if optimize { "true" } else { "false" },
        "",
    ));

    let store = Store::default_global();
    let root = store.root();
    let exists = root.exists();
    let warm = exists && store_index_nonempty(root);
    let warm_label = if !exists {
        "missing"
    } else if warm {
        "warm (index non-empty)"
    } else {
        "present (index empty or missing)"
    };
    report.findings.push(Finding::new(
        Severity::Info,
        "store",
        root.display().to_string(),
        warm_label,
    ));
}

fn store_index_nonempty(root: &Path) -> bool {
    for kind in ["sha1", "url"] {
        let dir = root.join(".index").join(kind);
        if let Ok(rd) = fs::read_dir(&dir) {
            for entry in rd.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    return true;
                }
            }
        }
    }
    false
}

/// Plain-text report: aligned columns (`severity category subject [detail]`), then summary.
pub fn format_text(report: &Report) -> String {
    let sev_w = report
        .findings
        .iter()
        .map(|f| f.severity_label().len())
        .max()
        .unwrap_or(0)
        .max("blocker".len());
    let cat_w = report
        .findings
        .iter()
        .map(|f| f.category.len())
        .max()
        .unwrap_or(0);

    let mut lines: Vec<String> = report
        .findings
        .iter()
        .map(|f| {
            let mut line = format!(
                "{sev:<sev_w$}  {cat:<cat_w$}  {subj}",
                sev = f.severity_label(),
                cat = f.category,
                subj = f.subject,
            );
            if !f.detail.is_empty() {
                line.push_str("  ");
                line.push_str(&f.detail);
            }
            line
        })
        .collect();
    lines.push(report.summary_line());
    lines.join("\n")
}

/// JSON report for CI.
pub fn format_json(report: &Report) -> Result<String> {
    #[derive(Serialize)]
    struct JsonOut<'a> {
        findings: &'a [Finding],
        blockers: usize,
        summary: String,
        ready: bool,
    }
    let out = JsonOut {
        findings: &report.findings,
        blockers: report.blocker_count(),
        summary: report.summary_line(),
        ready: report.blocker_count() == 0,
    };
    serde_json::to_string_pretty(&out).map_err(|e| Error::Message(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn classifies_unsupported_repo_types() {
        let repos = json!([
            {"type": "composer", "url": "https://repo.packagist.org"},
            {"type": "vcs", "url": "https://github.com/acme/foo"},
            {"type": "path", "url": "packages/*"},
            {"type": "artifact", "url": "artifacts/"},
            {"type": "package", "package": {"name": "acme/pkg", "version": "1.0.0"}}
        ]);
        let got = unsupported_repositories(&repos);
        assert_eq!(got.len(), 3);
        assert!(got.iter().any(|(_, t)| t == "vcs"));
        assert!(got.iter().any(|(_, t)| t == "artifact"));
        assert!(got.iter().any(|(_, t)| t == "package"));
    }

    #[test]
    fn planned_adapter_note() {
        assert!(PLANNED_ADAPTERS.contains(&"php-http/discovery"));
        assert!(PLANNED_ADAPTERS.contains(&"dealerdirect/phpcodesniffer-composer-installer"));
    }

    #[test]
    fn unknown_config_keys_filters_known() {
        let mut map = serde_json::Map::new();
        map.insert("sort-packages".into(), json!(true));
        map.insert("bin-dir".into(), json!("bin"));
        map.insert("use-github-api".into(), json!(false));
        let keys = unknown_config_keys(&map);
        assert_eq!(
            keys,
            vec!["bin-dir".to_string(), "use-github-api".to_string()]
        );
    }

    #[test]
    fn script_warnings_skip_laravel_keep_others() {
        let scripts = IndexMap::from([
            (
                "post-autoload-dump".into(),
                json!([
                    "Illuminate\\Foundation\\ComposerScripts::postAutoloadDump",
                    "Composer\\Config::disableProcessTimeout"
                ]),
            ),
            (
                "pre-package-uninstall".into(),
                json!("Illuminate\\Foundation\\ComposerScripts::prePackageUninstall"),
            ),
        ]);
        let got = non_laravel_class_method_scripts(&scripts);
        assert_eq!(
            got,
            vec!["Composer\\Config::disableProcessTimeout".to_string()]
        );
    }

    #[test]
    fn report_summary_ready_or_blockers() {
        let mut r = Report::default();
        assert_eq!(r.summary_line(), "ready to switch");
        assert_eq!(r.exit_code(), 0);
        r.findings
            .push(Finding::new(Severity::Blocker, "test", "x", ""));
        assert_eq!(r.summary_line(), "1 blocker");
        assert_eq!(r.exit_code(), 1);
        r.findings
            .push(Finding::new(Severity::Blocker, "test", "y", ""));
        assert_eq!(r.summary_line(), "2 blockers");
    }

    #[test]
    fn format_text_columns() {
        let mut r = Report::default();
        r.findings.push(Finding::new(
            Severity::Warning,
            "script",
            "Composer\\Config::disableProcessTimeout",
            "Class::method callback puck skips or runs differently",
        ));
        r.findings.push(Finding::new(
            Severity::Info,
            "store",
            "~/.puck/store",
            "warm (index non-empty)",
        ));
        let text = format_text(&r);
        assert!(text.contains("warning  script  Composer\\Config::disableProcessTimeout"));
        assert!(text.contains("info     store   ~/.puck/store  warm (index non-empty)"));
        assert!(text.ends_with("ready to switch"));
    }

    #[test]
    fn diagnose_fixture_laravel_skeleton_ready() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-skeleton");
        let report = diagnose(&root).expect("diagnose");
        assert_eq!(report.blocker_count(), 0, "{:?}", report.findings);
        assert_eq!(report.summary_line(), "ready to switch");
    }

    #[test]
    fn diagnose_fixture_laravel_app_ready() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel-app");
        let report = diagnose(&root).expect("diagnose");
        assert_eq!(report.blocker_count(), 0, "{:?}", report.findings);
        assert_eq!(report.summary_line(), "ready to switch");
    }

    #[test]
    fn unsupported_plugin_is_blocker() {
        let dir = tempdir().unwrap();
        let json = r#"{
          "name": "acme/tmp",
          "require": {},
          "config": { "allow-plugins": { "php-http/discovery": true } }
        }"#;
        let lock = r#"{
          "content-hash": "00000000000000000000000000000000",
          "packages": [{
            "name": "php-http/discovery",
            "version": "1.0.0",
            "type": "composer-plugin"
          }],
          "packages-dev": [],
          "aliases": [],
          "minimum-stability": "stable",
          "stability-flags": {},
          "prefer-stable": true,
          "prefer-lowest": false,
          "platform": {},
          "platform-dev": {},
          "plugin-api-version": "2.9.0"
        }"#;
        // Fix content-hash to match
        let hash = puck_lock::content_hash(json).unwrap();
        let lock = lock.replace("00000000000000000000000000000000", &hash);
        fs::write(dir.path().join("composer.json"), json).unwrap();
        fs::write(dir.path().join("composer.lock"), lock).unwrap();
        let report = diagnose(dir.path()).unwrap();
        assert!(report.blocker_count() >= 1);
        assert!(report.findings.iter().any(|f| {
            f.severity == Severity::Blocker
                && f.subject.contains("php-http/discovery")
                && f.detail.contains("planned")
        }));
    }

    #[test]
    fn vcs_repo_is_blocker() {
        let dir = tempdir().unwrap();
        let json = r#"{
          "name": "acme/tmp",
          "require": {},
          "repositories": [ { "type": "vcs", "url": "https://github.com/acme/foo" } ]
        }"#;
        let hash = puck_lock::content_hash(json).unwrap();
        let lock = format!(
            r#"{{
          "content-hash": "{hash}",
          "packages": [],
          "packages-dev": [],
          "aliases": [],
          "minimum-stability": "stable",
          "stability-flags": {{}},
          "prefer-stable": true,
          "prefer-lowest": false,
          "platform": {{}},
          "platform-dev": {{}},
          "plugin-api-version": "2.9.0"
        }}"#
        );
        fs::write(dir.path().join("composer.json"), json).unwrap();
        fs::write(dir.path().join("composer.lock"), lock).unwrap();
        let report = diagnose(dir.path()).unwrap();
        assert!(report.findings.iter().any(|f| {
            f.severity == Severity::Blocker
                && f.category == "repository"
                && f.detail.contains("vcs")
        }));
        assert_eq!(report.exit_code(), 1);
    }
}
