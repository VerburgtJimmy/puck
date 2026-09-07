//! Run Composer event scripts from root `composer.json`.
//!
//! M2 scope: enough for a typical Laravel app / `laravel-skeleton` install path.
//!
//! Composer: `Composer\EventDispatcher\EventDispatcher` (2.8.x)
//! - `@php …` → PHP binary + args
//! - `ClassName::method` → PHP callback (we special-case Laravel `ComposerScripts`)
//! - plain shell commands
//! - `@putenv`, named `@script` aliases (basic)
//!
//! When native package discovery already wrote `packages.php`, skip the standard
//! `@php artisan package:discover` handler so installs without `artisan` still succeed.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use indexmap::IndexMap;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("script `{script}` for event `{event}` failed with exit code {code}")]
    ScriptFailed {
        event: String,
        script: String,
        code: i32,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Options for running install-related Composer scripts.
#[derive(Debug, Clone, Default)]
pub struct RunScriptsOptions {
    /// When true, skip `@php artisan package:discover` (optional flags allowed).
    pub skip_package_discover: bool,
    /// Override PHP binary; otherwise `puck_php::find_php()`.
    pub php: Option<PathBuf>,
}

/// Summary of what ran for one or more events.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptReport {
    pub ran: usize,
    pub skipped: usize,
    pub notes: Vec<String>,
}

/// Env mutations from `@putenv` applied to subsequent handler processes.
#[derive(Debug, Default)]
struct ScriptEnv {
    /// `Some(value)` sets; `None` removes from the child environment.
    vars: HashMap<String, Option<String>>,
}

impl ScriptEnv {
    fn putenv(&mut self, spec: &str) {
        if let Some((key, value)) = spec.split_once('=') {
            self.vars.insert(key.to_owned(), Some(value.to_owned()));
        } else {
            self.vars.insert(spec.to_owned(), None);
        }
    }

    fn apply(&self, cmd: &mut Command) {
        for (key, value) in &self.vars {
            match value {
                Some(v) => {
                    cmd.env(key, v);
                }
                None => {
                    cmd.env_remove(key);
                }
            }
        }
    }
}

/// Run the Composer events that fire at the end of a normal `install`.
///
/// Order matches Composer: `post-autoload-dump`, then `post-install-cmd`.
pub fn run_install_scripts(
    project_root: impl AsRef<Path>,
    scripts: &IndexMap<String, Value>,
    options: &RunScriptsOptions,
) -> Result<ScriptReport> {
    let root = project_root.as_ref();
    let mut report = ScriptReport::default();
    let mut env = ScriptEnv::default();
    for event in ["post-autoload-dump", "post-install-cmd"] {
        let partial = run_event_scripts_with_env(root, scripts, event, options, &mut env)?;
        report.ran += partial.ran;
        report.skipped += partial.skipped;
        report.notes.extend(partial.notes);
    }
    Ok(report)
}

/// Run every handler listed under `scripts[event]`.
pub fn run_event_scripts(
    project_root: impl AsRef<Path>,
    scripts: &IndexMap<String, Value>,
    event: &str,
    options: &RunScriptsOptions,
) -> Result<ScriptReport> {
    let mut env = ScriptEnv::default();
    run_event_scripts_with_env(project_root, scripts, event, options, &mut env)
}

fn run_event_scripts_with_env(
    project_root: impl AsRef<Path>,
    scripts: &IndexMap<String, Value>,
    event: &str,
    options: &RunScriptsOptions,
    env: &mut ScriptEnv,
) -> Result<ScriptReport> {
    let root = project_root.as_ref();
    let Some(handlers) = script_handlers(scripts, event) else {
        return Ok(ScriptReport::default());
    };

    let php = options.php.clone().or_else(puck_php::find_php);
    let mut report = ScriptReport::default();
    let mut stack = HashSet::new();
    run_handlers(
        root,
        scripts,
        event,
        &handlers,
        options,
        php.as_deref(),
        env,
        &mut stack,
        &mut report,
    )?;
    Ok(report)
}

fn script_handlers(scripts: &IndexMap<String, Value>, event: &str) -> Option<Vec<String>> {
    let value = scripts.get(event)?;
    match value {
        Value::String(s) => Some(vec![s.clone()]),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                if let Value::String(s) = item {
                    out.push(s.clone());
                }
            }
            Some(out)
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_handlers(
    root: &Path,
    scripts: &IndexMap<String, Value>,
    event: &str,
    handlers: &[String],
    options: &RunScriptsOptions,
    php: Option<&Path>,
    env: &mut ScriptEnv,
    stack: &mut HashSet<String>,
    report: &mut ScriptReport,
) -> Result<()> {
    for raw in handlers {
        let handler = strip_no_additional_args(raw.trim());
        if handler.is_empty() {
            continue;
        }

        if options.skip_package_discover && is_package_discover_script(&handler) {
            report.skipped += 1;
            report.notes.push(format!(
                "skipped `{handler}` (native package discovery already wrote packages.php)"
            ));
            continue;
        }

        if is_php_callback(&handler) {
            run_php_callback(root, event, &handler, php, env, report)?;
            continue;
        }

        if let Some(spec) = handler.strip_prefix("@putenv ") {
            env.putenv(spec);
            report.ran += 1;
            continue;
        }

        // Named script alias: `@foo` or `@foo args` (Composer `isComposerScript`).
        if handler.starts_with('@')
            && !handler.starts_with("@php ")
            && !handler.starts_with("@putenv ")
        {
            run_script_alias(
                root, scripts, event, &handler, options, php, env, stack, report,
            )?;
            continue;
        }

        run_shell_or_php(root, event, &handler, php, env, report)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_script_alias(
    root: &Path,
    scripts: &IndexMap<String, Value>,
    event: &str,
    handler: &str,
    options: &RunScriptsOptions,
    php: Option<&Path>,
    env: &mut ScriptEnv,
    stack: &mut HashSet<String>,
    report: &mut ScriptReport,
) -> Result<()> {
    let rest = handler.trim_start_matches('@');
    let name = rest.split_once(' ').map(|(n, _)| n).unwrap_or(rest);
    if !stack.insert(name.to_owned()) {
        return Err(Error::Message(format!(
            "circular script reference involving `@{name}`"
        )));
    }
    let Some(nested) = script_handlers(scripts, name) else {
        stack.remove(name);
        report.skipped += 1;
        report
            .notes
            .push(format!("skipped `{handler}` (unknown script alias)"));
        return Ok(());
    };
    run_handlers(
        root, scripts, name, &nested, options, php, env, stack, report,
    )?;
    stack.remove(name);
    let _ = event;
    Ok(())
}

fn run_php_callback(
    root: &Path,
    event: &str,
    handler: &str,
    php: Option<&Path>,
    env: &ScriptEnv,
    report: &mut ScriptReport,
) -> Result<()> {
    let Some((class, method)) = handler.split_once("::") else {
        report.skipped += 1;
        report.notes.push(format!(
            "skipped `{handler}` (not a Class::method callback)"
        ));
        return Ok(());
    };

    if is_laravel_clear_compiled(class, method) {
        return run_laravel_clear_compiled(root, handler, php, env, report);
    }

    // Generic callbacks need a Composer Event object; M2 only covers Laravel clear.
    report.skipped += 1;
    report.notes.push(format!(
        "skipped `{handler}` (Class::method callbacks need Composer Event; not implemented for M2)"
    ));
    let _ = event;
    Ok(())
}

fn is_laravel_clear_compiled(class: &str, method: &str) -> bool {
    class == "Illuminate\\Foundation\\ComposerScripts"
        && matches!(method, "postAutoloadDump" | "postInstall" | "postUpdate")
}

fn run_laravel_clear_compiled(
    root: &Path,
    handler: &str,
    php: Option<&Path>,
    env: &ScriptEnv,
    report: &mut ScriptReport,
) -> Result<()> {
    let Some(php) = php else {
        report.skipped += 1;
        report.notes.push(format!(
            "skipped `{handler}` (php not on PATH; clears compiled caches only)"
        ));
        return Ok(());
    };

    let autoload = root.join("vendor/autoload.php");
    if !autoload.is_file() {
        report.skipped += 1;
        report.notes.push(format!(
            "skipped `{handler}` (vendor/autoload.php missing; clears compiled caches only)"
        ));
        return Ok(());
    }

    // Mirror ComposerScripts::clearCompiled via reflection so we do not need a
    // Composer\Script\Event. CLI restores packages.php afterward when needed.
    let code = r#"
        require 'vendor/autoload.php';
        $m = new ReflectionMethod('Illuminate\\Foundation\\ComposerScripts', 'clearCompiled');
        $m->setAccessible(true);
        $m->invoke(null);
    "#;

    let mut cmd = Command::new(php);
    cmd.arg("-r")
        .arg(code)
        .current_dir(root)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    env.apply(&mut cmd);

    let status = cmd.status()?;

    if !status.success() {
        return Err(Error::ScriptFailed {
            event: "post-autoload-dump".into(),
            script: handler.into(),
            code: status.code().unwrap_or(1),
        });
    }

    report.ran += 1;
    report.notes.push(format!(
        "ran `{handler}` (clearCompiled; packages.php may be removed until rediscover)"
    ));
    Ok(())
}

fn run_shell_or_php(
    root: &Path,
    event: &str,
    handler: &str,
    php: Option<&Path>,
    env: &ScriptEnv,
    report: &mut ScriptReport,
) -> Result<()> {
    let exec = if let Some(rest) = handler.strip_prefix("@php ") {
        let Some(php) = php else {
            return Err(Error::Message(format!(
                "cannot run `{handler}`: php not found on PATH"
            )));
        };
        format!("{} {}", shell_quote(&php.to_string_lossy()), rest)
    } else {
        handler.to_owned()
    };

    eprintln!("> {exec}");

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&exec)
        .current_dir(root)
        .env("COMPOSER_DEV_MODE", "1")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    env.apply(&mut cmd);

    let status = cmd.status()?;

    if !status.success() {
        return Err(Error::ScriptFailed {
            event: event.into(),
            script: handler.into(),
            code: status.code().unwrap_or(1),
        });
    }
    report.ran += 1;
    Ok(())
}

fn strip_no_additional_args(s: &str) -> String {
    // Composer strips ` @no_additional_args` / `@no_additional_args`.
    let mut out = s.to_owned();
    for token in [" @no_additional_args", "@no_additional_args"] {
        if let Some(idx) = out.find(token) {
            out.replace_range(idx..idx + token.len(), "");
        }
    }
    out.trim().to_owned()
}

/// True for the standard Laravel discover script Composer ships in skeletons.
///
/// Matches `@php artisan package:discover` with optional flags (`--ansi`, etc.).
pub fn is_package_discover_script(handler: &str) -> bool {
    let s = strip_no_additional_args(handler.trim());
    let Some(rest) = s.strip_prefix("@php ") else {
        return false;
    };
    let rest = rest.trim_start();
    let mut parts = rest.split_whitespace();
    matches!(
        (parts.next(), parts.next()),
        (Some("artisan"), Some("package:discover"))
    )
}

fn is_php_callback(handler: &str) -> bool {
    !handler.contains(' ') && handler.contains("::")
}

fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
    {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn detects_package_discover_variants() {
        assert!(is_package_discover_script(
            "@php artisan package:discover --ansi"
        ));
        assert!(is_package_discover_script("@php artisan package:discover"));
        assert!(is_package_discover_script(
            "  @php artisan package:discover --ansi @no_additional_args"
        ));
        assert!(!is_package_discover_script(
            "@php artisan package:discoverfoo"
        ));
        assert!(!is_package_discover_script(
            "@php artisan config:clear --ansi"
        ));
        assert!(!is_package_discover_script(
            "Illuminate\\Foundation\\ComposerScripts::postAutoloadDump"
        ));
    }

    #[test]
    fn skips_discover_when_flag_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let scripts = IndexMap::from([(
            "post-autoload-dump".into(),
            json!([
                "Illuminate\\Foundation\\ComposerScripts::postAutoloadDump",
                "@php artisan package:discover --ansi"
            ]),
        )]);
        let opts = RunScriptsOptions {
            skip_package_discover: true,
            php: None,
        };
        let report =
            run_event_scripts(dir.path(), &scripts, "post-autoload-dump", &opts).expect("run");
        assert!(report.skipped >= 1);
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("package:discover") && n.contains("skipped"))
        );
    }

    #[test]
    fn runs_plain_shell_script() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("ran.txt");
        let scripts = IndexMap::from([(
            "post-install-cmd".into(),
            json!(format!(
                "touch {}",
                marker.file_name().unwrap().to_string_lossy()
            )),
        )]);
        let report = run_event_scripts(
            dir.path(),
            &scripts,
            "post-install-cmd",
            &RunScriptsOptions::default(),
        )
        .expect("run");
        assert_eq!(report.ran, 1);
        assert!(marker.is_file());
    }

    #[test]
    fn run_install_scripts_order_covers_both_events() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        let scripts = IndexMap::from([
            (
                "post-autoload-dump".into(),
                json!(format!(
                    "touch {}",
                    a.file_name().unwrap().to_string_lossy()
                )),
            ),
            (
                "post-install-cmd".into(),
                json!(format!(
                    "touch {}",
                    b.file_name().unwrap().to_string_lossy()
                )),
            ),
        ]);
        let report =
            run_install_scripts(dir.path(), &scripts, &RunScriptsOptions::default()).expect("run");
        assert_eq!(report.ran, 2);
        assert!(a.is_file());
        assert!(b.is_file());
    }

    #[test]
    fn laravel_composer_scripts_missing_autoload_skips() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("vendor")).expect("vendor");
        let fake_php = dir.path().join("fake-php");
        fs::write(&fake_php, "#!/bin/sh\nexit 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&fake_php).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&fake_php, perms).unwrap();
        }
        let scripts = IndexMap::from([(
            "post-autoload-dump".into(),
            json!(["Illuminate\\Foundation\\ComposerScripts::postAutoloadDump"]),
        )]);
        let opts = RunScriptsOptions {
            php: Some(fake_php),
            skip_package_discover: false,
        };
        let report =
            run_event_scripts(dir.path(), &scripts, "post-autoload-dump", &opts).expect("run");
        assert_eq!(report.skipped, 1);
        assert!(report.notes.iter().any(|n| n.contains("autoload.php")));
    }

    #[test]
    fn putenv_applies_to_later_shell() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("env.txt");
        let scripts = IndexMap::from([(
            "post-install-cmd".into(),
            json!([
                "@putenv PUCK_SCRIPTS_TEST=hello",
                format!(
                    "printf '%s' \"$PUCK_SCRIPTS_TEST\" > {}",
                    marker.file_name().unwrap().to_string_lossy()
                )
            ]),
        )]);
        run_event_scripts(
            dir.path(),
            &scripts,
            "post-install-cmd",
            &RunScriptsOptions::default(),
        )
        .expect("run");
        let body = fs::read_to_string(&marker).expect("read");
        assert_eq!(body, "hello");
    }
}
