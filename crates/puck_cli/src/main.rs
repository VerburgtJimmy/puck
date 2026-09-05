//! `puck` - native PHP package manager (Laravel-first).

use clap::{Parser, Subcommand};
use puck_autoload::{DumpOptions, dump, dump_is_current};
use puck_install::{
    ExecuteTimings, InstallAction, InstallOptions, execute_install, install_binaries, plan_install,
    read_installed, reconcile_vendor_presence,
};
use puck_laravel::{DiscoverStatus, discover};
use puck_lock::LockFile;
use puck_manifest::{PackageRequirement, Manifest, add_requirement, remove_requirement};
use puck_plugins::{PhpstanExtensionInstallStatus, run_phpstan_extension_installer};
use puck_registry::p2_path;
use puck_resolver::resolve_lock_document;
use puck_scripts::{RunScriptsOptions, run_install_scripts};
use puck_store::Store;
use serde_json::Value;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

#[derive(Debug, Parser)]
#[command(
    name = "puck",
    version,
    about = "Native package manager for PHP. Laravel-first. Composer-compatible.",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Install dependencies from composer.lock
    Install {
        /// Do not install packages listed under require-dev
        #[arg(long)]
        no_dev: bool,

        /// Generate optimized autoload files
        #[arg(long, short = 'o')]
        optimize: bool,

        /// Succeed with zero network calls (warm store + lock required)
        #[arg(long)]
        offline: bool,

        /// Skip composer.json scripts (default: run them, matching Composer)
        #[arg(long)]
        no_scripts: bool,

        /// Fail if any plugin lacks a native adapter
        #[arg(long)]
        strict_native: bool,

        /// Project directory (defaults to current directory)
        #[arg(long, value_name = "DIR")]
        working_dir: Option<PathBuf>,
    },
    /// Add a package to composer.json and update the lock (M3)
    Require {
        packages: Vec<String>,
        #[arg(long)]
        dev: bool,
        /// Edit composer.json and lock only; do not install into vendor/
        #[arg(long)]
        no_install: bool,
        /// Packagist p2 metadata root (contains `packagist/p2/`). Defaults to `$PUCK_REGISTRY`.
        #[arg(long, value_name = "DIR")]
        registry: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        working_dir: Option<PathBuf>,
    },
    /// Remove a package from composer.json and update the lock (M3)
    Remove {
        packages: Vec<String>,
        /// Edit composer.json and lock only; do not change vendor/
        #[arg(long)]
        no_install: bool,
        /// Packagist p2 metadata root (contains `packagist/p2/`). Defaults to `$PUCK_REGISTRY`.
        #[arg(long, value_name = "DIR")]
        registry: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        working_dir: Option<PathBuf>,
    },
    /// Rewrite composer.lock from registry metadata without changing versions (M3)
    Lock {
        #[arg(long)]
        no_install: bool,
        #[arg(long, value_name = "DIR")]
        registry: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        working_dir: Option<PathBuf>,
    },
    /// Update packages within constraints (M3)
    Update {
        packages: Vec<String>,
        #[arg(long)]
        no_install: bool,
        #[arg(long, value_name = "DIR")]
        registry: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        working_dir: Option<PathBuf>,
    },
    /// Regenerate autoload files
    #[command(name = "dump-autoload", visible_alias = "dumpautoload")]
    DumpAutoload {
        #[arg(long, short = 'o')]
        optimize: bool,
        #[arg(long, short = 'a')]
        authoritative: bool,
        #[arg(long)]
        no_dev: bool,
        #[arg(long, value_name = "DIR")]
        working_dir: Option<PathBuf>,
    },
    /// Shared store utilities (M2)
    Store {
        #[command(subcommand)]
        command: StoreCommands,
    },
    /// Run the project's PHP binary (M5)
    Php {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum StoreCommands {
    /// Print the global store path
    Path,
    /// Garbage-collect unreferenced store entries
    Gc,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Install {
            no_dev,
            optimize,
            offline,
            no_scripts,
            strict_native: _,
            working_dir,
        } => {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("puck: failed to start async runtime: {err}");
                    return ExitCode::from(1);
                }
            };
            match runtime.block_on(run_install(
                working_dir,
                no_dev,
                offline,
                optimize,
                no_scripts,
            )) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("puck: {err}");
                    ExitCode::from(1)
                }
            }
        }
        Commands::Store {
            command: StoreCommands::Path,
        } => {
            println!("{}", puck_store::default_store_root().display());
            ExitCode::SUCCESS
        }
        Commands::Store {
            command: StoreCommands::Gc,
        } => match run_store_gc() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("puck: {err}");
                ExitCode::from(1)
            }
        },
        Commands::DumpAutoload {
            optimize,
            authoritative,
            no_dev,
            working_dir,
        } => match run_dump_autoload(working_dir, optimize, authoritative, no_dev) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("puck: {err}");
                ExitCode::from(1)
            }
        },
        Commands::Require {
            packages,
            dev,
            no_install,
            registry,
            working_dir,
        } => {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("puck: failed to start async runtime: {err}");
                    return ExitCode::from(1);
                }
            };
            match runtime.block_on(run_require(
                working_dir,
                packages,
                dev,
                no_install,
                registry,
            )) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("puck: {err}");
                    ExitCode::from(1)
                }
            }
        }
        Commands::Remove {
            packages,
            no_install,
            registry,
            working_dir,
        } => {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("puck: failed to start async runtime: {err}");
                    return ExitCode::from(1);
                }
            };
            match runtime.block_on(run_remove(working_dir, packages, no_install, registry)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("puck: {err}");
                    ExitCode::from(1)
                }
            }
        }
        Commands::Lock {
            no_install,
            registry,
            working_dir,
        } => {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("puck: failed to start async runtime: {err}");
                    return ExitCode::from(1);
                }
            };
            match runtime.block_on(run_lock(working_dir, no_install, registry)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("puck: {err}");
                    ExitCode::from(1)
                }
            }
        }
        Commands::Update {
            packages,
            no_install,
            registry,
            working_dir,
        } => {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("puck: failed to start async runtime: {err}");
                    return ExitCode::from(1);
                }
            };
            match runtime.block_on(run_update(working_dir, packages, no_install, registry)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("puck: {err}");
                    ExitCode::from(1)
                }
            }
        }
        Commands::Php { .. } => {
            eprintln!("puck: this command is not implemented yet");
            ExitCode::from(2)
        }
    }
}

async fn run_require(
    working_dir: Option<PathBuf>,
    packages: Vec<String>,
    dev: bool,
    no_install: bool,
    registry: Option<PathBuf>,
) -> Result<(), String> {
    if packages.is_empty() {
        return Err("missing package argument (e.g. vendor/package:^1.0)".into());
    }

    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = root.join("composer.json");
    let lock_path = root.join("composer.lock");
    if !manifest_path.is_file() {
        return Err(format!("no composer.json in {}", root.display()));
    }

    let registry_root = resolve_registry_root(registry)?;
    let p2_dir = registry_root.join("packagist/p2");

    let mut unlock = Vec::new();
    let mut root_json: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    for spec in &packages {
        let req = PackageRequirement::parse(spec).map_err(|e| e.to_string())?;
        // Ensure VCR has metadata before mutating the project.
        let meta_path = p2_path(&registry_root, &req.name);
        if !meta_path.is_file() {
            return Err(format!(
                "no recorded p2 metadata for {} at {} (record with benches/record-packagist-p2.sh or widen the VCR)",
                req.name,
                meta_path.display()
            ));
        }
        add_requirement(&mut root_json, &req, dev).map_err(|e| e.to_string())?;
        unlock.push(req.name.clone());
        eprintln!(
            "puck: require {} {} ({})",
            req.name,
            req.constraint,
            if dev { "require-dev" } else { "require" }
        );
    }

    let composer_text = format_composer_json(&root_json)?;
    std::fs::write(&manifest_path, &composer_text).map_err(|e| e.to_string())?;

    let lock_bytes = if lock_path.is_file() {
        Some(std::fs::read(&lock_path).map_err(|e| e.to_string())?)
    } else {
        None
    };

    let lock_doc = resolve_lock_document(
        &composer_text,
        lock_bytes.as_deref(),
        &p2_dir,
        &unlock,
        true,
    )
    .map_err(|e| e.to_string())?;

    let lock_text = format!("{}\n", serde_json::to_string_pretty(&lock_doc).map_err(|e| e.to_string())?);
    let lock_text = reindent_json_pretty_4(&lock_text);
    std::fs::write(&lock_path, &lock_text).map_err(|e| e.to_string())?;
    eprintln!(
        "puck: wrote composer.lock (content-hash {})",
        lock_doc
            .get("content-hash")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    if no_install {
        eprintln!("puck: --no-install; skip vendor/");
        return Ok(());
    }

    run_install(Some(root), false, false, false, false).await
}

async fn run_remove(
    working_dir: Option<PathBuf>,
    packages: Vec<String>,
    no_install: bool,
    registry: Option<PathBuf>,
) -> Result<(), String> {
    if packages.is_empty() {
        return Err("missing package argument (e.g. vendor/package)".into());
    }

    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = root.join("composer.json");
    let lock_path = root.join("composer.lock");
    if !manifest_path.is_file() {
        return Err(format!("no composer.json in {}", root.display()));
    }
    if !lock_path.is_file() {
        return Err(format!("no composer.lock in {}", root.display()));
    }

    let registry_root = resolve_registry_root(registry)?;
    let p2_dir = registry_root.join("packagist/p2");

    let mut unlock = Vec::new();
    let mut root_json: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    for spec in &packages {
        let name = PackageRequirement::parse(spec)
            .map(|r| r.name)
            .or_else(|_| {
                let name = spec.trim().to_ascii_lowercase();
                if name.contains('/') {
                    Ok(name)
                } else {
                    Err(format!(
                        "invalid package name {spec:?}; expected vendor/package"
                    ))
                }
            })?;
        let found = remove_requirement(&mut root_json, &name).map_err(|e| e.to_string())?;
        if !found {
            return Err(format!(
                "{name} is not required in composer.json require or require-dev"
            ));
        }
        unlock.push(name.clone());
        eprintln!("puck: remove {name}");
    }

    let composer_text = format_composer_json(&root_json)?;
    std::fs::write(&manifest_path, &composer_text).map_err(|e| e.to_string())?;

    let lock_bytes = std::fs::read(&lock_path).map_err(|e| e.to_string())?;
    let lock_doc = resolve_lock_document(
        &composer_text,
        Some(&lock_bytes),
        &p2_dir,
        &unlock,
        true,
    )
    .map_err(|e| e.to_string())?;

    let lock_text =
        format!("{}\n", serde_json::to_string_pretty(&lock_doc).map_err(|e| e.to_string())?);
    let lock_text = reindent_json_pretty_4(&lock_text);
    std::fs::write(&lock_path, &lock_text).map_err(|e| e.to_string())?;
    eprintln!(
        "puck: wrote composer.lock (content-hash {})",
        lock_doc
            .get("content-hash")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    if no_install {
        eprintln!("puck: --no-install; skip vendor/");
        return Ok(());
    }

    run_install(Some(root), false, false, false, false).await
}

async fn run_lock(
    working_dir: Option<PathBuf>,
    no_install: bool,
    registry: Option<PathBuf>,
) -> Result<(), String> {
    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = root.join("composer.json");
    let lock_path = root.join("composer.lock");
    if !manifest_path.is_file() {
        return Err(format!("no composer.json in {}", root.display()));
    }
    if !lock_path.is_file() {
        return Err(format!("no composer.lock in {}", root.display()));
    }

    let registry_root = resolve_registry_root(registry)?;
    let p2_dir = registry_root.join("packagist/p2");
    let composer_text = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
    let lock_bytes = std::fs::read(&lock_path).map_err(|e| e.to_string())?;

    // Empty unlock: keep every locked package fixed; rewrite dump from VCR p2.
    let lock_doc = resolve_lock_document(&composer_text, Some(&lock_bytes), &p2_dir, &[], true)
        .map_err(|e| e.to_string())?;

    write_lock_file(&lock_path, &lock_doc)?;
    eprintln!(
        "puck: wrote composer.lock (content-hash {})",
        lock_doc
            .get("content-hash")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    if no_install {
        eprintln!("puck: --no-install; skip vendor/");
        return Ok(());
    }
    run_install(Some(root), false, false, false, false).await
}

async fn run_update(
    working_dir: Option<PathBuf>,
    packages: Vec<String>,
    no_install: bool,
    registry: Option<PathBuf>,
) -> Result<(), String> {
    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = root.join("composer.json");
    let lock_path = root.join("composer.lock");
    if !manifest_path.is_file() {
        return Err(format!("no composer.json in {}", root.display()));
    }

    let registry_root = resolve_registry_root(registry)?;
    let p2_dir = registry_root.join("packagist/p2");
    let composer_text = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
    let lock_bytes = if lock_path.is_file() {
        Some(std::fs::read(&lock_path).map_err(|e| e.to_string())?)
    } else {
        None
    };

    let unlock: Vec<String> = if packages.is_empty() {
        // Full update: unlock everything by not fixing (pass all names from lock).
        match &lock_bytes {
            Some(bytes) => {
                let data: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
                let mut names = Vec::new();
                for key in ["packages", "packages-dev"] {
                    if let Some(arr) = data.get(key).and_then(|v| v.as_array()) {
                        for pkg in arr {
                            if let Some(name) = pkg.get("name").and_then(|v| v.as_str()) {
                                names.push(name.to_ascii_lowercase());
                            }
                        }
                    }
                }
                names
            }
            None => Vec::new(),
        }
    } else {
        packages
            .iter()
            .map(|s| {
                PackageRequirement::parse(s)
                    .map(|r| r.name)
                    .or_else(|_| {
                        let name = s.trim().to_ascii_lowercase();
                        if name.contains('/') {
                            Ok(name)
                        } else {
                            Err(format!(
                                "invalid package name {s:?}; expected vendor/package"
                            ))
                        }
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    if packages.is_empty() {
        eprintln!("puck: update (all)");
    } else {
        for name in &unlock {
            eprintln!("puck: update {name}");
        }
    }

    let lock_doc = resolve_lock_document(
        &composer_text,
        lock_bytes.as_deref(),
        &p2_dir,
        &unlock,
        true,
    )
    .map_err(|e| e.to_string())?;

    write_lock_file(&lock_path, &lock_doc)?;
    eprintln!(
        "puck: wrote composer.lock (content-hash {})",
        lock_doc
            .get("content-hash")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    if no_install {
        eprintln!("puck: --no-install; skip vendor/");
        return Ok(());
    }
    run_install(Some(root), false, false, false, false).await
}

fn write_lock_file(path: &std::path::Path, lock_doc: &Value) -> Result<(), String> {
    let lock_text =
        format!("{}\n", serde_json::to_string_pretty(lock_doc).map_err(|e| e.to_string())?);
    let lock_text = reindent_json_pretty_4(&lock_text);
    std::fs::write(path, &lock_text).map_err(|e| e.to_string())
}

fn resolve_registry_root(registry: Option<PathBuf>) -> Result<PathBuf, String> {
    let registry_root = registry
        .or_else(|| std::env::var_os("PUCK_REGISTRY").map(PathBuf::from))
        .ok_or_else(|| {
            "need --registry DIR or $PUCK_REGISTRY pointing at a registry root with packagist/p2/"
                .to_string()
        })?;
    let p2_dir = registry_root.join("packagist/p2");
    if !p2_dir.is_dir() {
        return Err(format!(
            "registry p2 dir missing: {} (expected packagist/p2 under registry root)",
            p2_dir.display()
        ));
    }
    Ok(registry_root)
}

fn format_composer_json(root: &Value) -> Result<String, String> {
    let pretty =
        serde_json::to_string_pretty(root).map_err(|e| format!("encode composer.json: {e}"))?;
    Ok(reindent_json_pretty_4(&format!("{pretty}\n")))
}

fn reindent_json_pretty_4(pretty_2: &str) -> String {
    let mut out = String::with_capacity(pretty_2.len());
    for line in pretty_2.lines() {
        let trimmed = line.trim_start();
        let spaces = line.len() - trimmed.len();
        let level = spaces / 2;
        for _ in 0..level {
            out.push_str("    ");
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

async fn run_install(
    working_dir: Option<PathBuf>,
    no_dev: bool,
    offline: bool,
    optimize: bool,
    no_scripts: bool,
) -> Result<(), String> {
    let total_started = Instant::now();
    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = root.join("composer.json");
    let lock_path = root.join("composer.lock");

    if !lock_path.is_file() {
        return Err(format!(
            "no composer.lock in {} (run Composer once to lock, or wait for puck update)",
            root.display()
        ));
    }

    let plan_started = Instant::now();
    let manifest = if manifest_path.is_file() {
        let manifest = Manifest::from_path(&manifest_path).map_err(|e| e.to_string())?;
        eprintln!("puck: project {}", manifest.pretty_name);
        Some(manifest)
    } else {
        None
    };

    let lock = LockFile::from_path(&lock_path).map_err(|e| e.to_string())?;
    let installed = read_installed(root.join("vendor/composer")).map_err(|e| e.to_string())?;
    let options = InstallOptions { no_dev, offline };
    let mut plan = plan_install(&lock, &installed, options).map_err(|e| e.to_string())?;
    reconcile_vendor_presence(&mut plan, &root.join("vendor"));

    // CLI `-o` wins; otherwise honour composer.json `config.optimize-autoloader`.
    let optimize = optimize
        || manifest
            .as_ref()
            .is_some_and(Manifest::optimize_autoloader);

    let mut install = 0usize;
    let mut update = 0usize;
    let mut keep = 0usize;
    let mut remove = 0usize;
    for pkg in &plan.packages {
        match pkg.action {
            InstallAction::Install => install += 1,
            InstallAction::Update => update += 1,
            InstallAction::Keep => keep += 1,
            InstallAction::Remove => remove += 1,
        }
    }
    let plan_ms = plan_started.elapsed().as_millis();

    eprintln!(
        "puck: plan  install={install}  update={update}  keep={keep}  remove={remove}  (lock {})",
        lock.content_hash
    );
    if optimize {
        eprintln!("puck: optimize-autoloader enabled");
    }

    let mut exec_timings = ExecuteTimings::default();
    let packages_changed = install + update + remove > 0;
    if !packages_changed {
        eprintln!("puck: nothing to install");
        let meta_started = Instant::now();
        install_binaries(&root, &lock, no_dev).map_err(|e| e.to_string())?;
        exec_timings.installed_meta_ms = meta_started.elapsed().as_millis();
    } else {
        let store = Store::default_global();
        exec_timings = execute_install(&root, &lock, &plan, options, &store, manifest.as_ref())
            .await
            .map_err(|e| e.to_string())?;
    }

    // Warm keep: skip dump when packages and dump meta (hash + optimize) match.
    let need_dump = packages_changed
        || !dump_is_current(&root, &lock)
        || !dump_meta_current(&root, &lock.content_hash, optimize);
    if !need_dump {
        eprintln!("puck: autoload up to date");
        eprintln!("puck: done");
        print_install_timings(
            plan_ms,
            &exec_timings,
            0,
            0,
            0,
            total_started.elapsed().as_millis(),
        );
        return Ok(());
    }

    let dump_started = Instant::now();
    dump(
        &root,
        &lock,
        manifest.as_ref(),
        DumpOptions {
            optimize,
            authoritative: false,
            no_dev,
        },
    )
    .map_err(|e| e.to_string())?;
    write_dump_meta(&root, &lock.content_hash, optimize).map_err(|e| e.to_string())?;
    let dump_ms = dump_started.elapsed().as_millis();
    eprintln!(
        "puck: dumped autoload{}",
        if optimize { " (-o)" } else { "" }
    );

    let mut packages_written = false;
    let discover_started = Instant::now();
    match discover(&root).map_err(|e| e.to_string())? {
        DiscoverStatus::Written { package_count, .. } => {
            eprintln!("puck: discovered {package_count} packages");
            packages_written = true;
        }
        DiscoverStatus::Skipped => {}
    }
    let mut discover_ms = discover_started.elapsed().as_millis();

    // Tier 1: phpstan/extension-installer (POST_INSTALL_CMD equivalent).
    let allow_phpstan_plugin = manifest
        .as_ref()
        .is_none_or(|m| m.allows_plugin("phpstan/extension-installer"));
    match run_phpstan_extension_installer(&root, allow_phpstan_plugin).map_err(|e| e.to_string())?
    {
        PhpstanExtensionInstallStatus::Written {
            extension_count, ..
        } => {
            eprintln!("puck: phpstan extensions registered ({extension_count})");
        }
        PhpstanExtensionInstallStatus::Skipped => {}
    }

    let mut scripts_ms = 0u128;
    if !no_scripts
        && let Some(ref manifest) = manifest
    {
        let scripts_started = Instant::now();
        let report = run_install_scripts(
            &root,
            &manifest.scripts,
            &RunScriptsOptions {
                skip_package_discover: packages_written,
                php: None,
            },
        )
        .map_err(|e| e.to_string())?;
        for note in &report.notes {
            eprintln!("puck: {note}");
        }
        if report.ran > 0 || report.skipped > 0 {
            eprintln!(
                "puck: scripts  ran={}  skipped={}",
                report.ran, report.skipped
            );
        }
        scripts_ms = scripts_started.elapsed().as_millis();

        // ComposerScripts::clearCompiled deletes packages.php; rewrite if gone.
        if packages_written {
            let packages_php = root.join("bootstrap/cache/packages.php");
            if !packages_php.is_file() {
                let rediscover_started = Instant::now();
                match discover(&root).map_err(|e| e.to_string())? {
                    DiscoverStatus::Written { package_count, .. } => {
                        eprintln!("puck: rediscovered {package_count} packages after scripts");
                    }
                    DiscoverStatus::Skipped => {}
                }
                discover_ms += rediscover_started.elapsed().as_millis();
            }
        }
    }

    eprintln!("puck: done");
    print_install_timings(
        plan_ms,
        &exec_timings,
        dump_ms,
        discover_ms,
        scripts_ms,
        total_started.elapsed().as_millis(),
    );
    Ok(())
}

fn timings_enabled() -> bool {
    std::env::var_os("PUCK_TIMINGS").is_some()
}

fn print_install_timings(
    plan_ms: u128,
    exec: &ExecuteTimings,
    dump_ms: u128,
    discover_ms: u128,
    scripts_ms: u128,
    total_ms: u128,
) {
    if !timings_enabled() {
        return;
    }
    eprintln!("puck: timing  plan_ms={plan_ms}");
    eprintln!("puck: timing  fetch_ms={}", exec.fetch_ms);
    eprintln!("puck: timing  fetch_cache_hit_ms={}", exec.fetch_cache_hit_ms);
    eprintln!("puck: timing  fetch_download_ms={}", exec.fetch_download_ms);
    eprintln!("puck: timing  link_ms={}", exec.link_ms);
    eprintln!("puck: timing  installed_meta_ms={}", exec.installed_meta_ms);
    eprintln!("puck: timing  dump_ms={dump_ms}");
    eprintln!("puck: timing  discover_ms={discover_ms}");
    eprintln!("puck: timing  scripts_ms={scripts_ms}");
    eprintln!("puck: timing  total_ms={total_ms}");
}

fn run_dump_autoload(
    working_dir: Option<PathBuf>,
    optimize: bool,
    authoritative: bool,
    no_dev: bool,
) -> Result<(), String> {
    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let lock_path = root.join("composer.lock");
    if !lock_path.is_file() {
        return Err(format!("no composer.lock in {}", root.display()));
    }
    let lock = LockFile::from_path(&lock_path).map_err(|e| e.to_string())?;
    let manifest = {
        let path = root.join("composer.json");
        if path.is_file() {
            Some(Manifest::from_path(&path).map_err(|e| e.to_string())?)
        } else {
            None
        }
    };
    let optimize = optimize || manifest.as_ref().is_some_and(Manifest::optimize_autoloader);

    dump(
        &root,
        &lock,
        manifest.as_ref(),
        DumpOptions {
            optimize,
            authoritative: authoritative || optimize,
            no_dev,
        },
    )
    .map_err(|e| e.to_string())?;
    write_dump_meta(&root, &lock.content_hash, optimize).map_err(|e| e.to_string())?;
    eprintln!(
        "puck: dumped autoload{}",
        if optimize { " (-o)" } else { "" }
    );
    Ok(())
}

fn dump_meta_path(root: &std::path::Path) -> PathBuf {
    root.join(".puck/autoload-meta")
}

fn dump_meta_current(root: &std::path::Path, content_hash: &str, optimize: bool) -> bool {
    let Ok(text) = std::fs::read_to_string(dump_meta_path(root)) else {
        return false;
    };
    let expected = format!("content-hash={content_hash}\noptimize={}\n", optimize as u8);
    text == expected
}

fn write_dump_meta(
    root: &std::path::Path,
    content_hash: &str,
    optimize: bool,
) -> Result<(), String> {
    let dir = root.join(".puck");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir .puck: {e}"))?;
    let body = format!("content-hash={content_hash}\noptimize={}\n", optimize as u8);
    std::fs::write(dump_meta_path(root), body).map_err(|e| format!("write dump meta: {e}"))
}

fn run_store_gc() -> Result<(), String> {
    let store = Store::default_global();
    let report = puck_store::gc(&store).map_err(|e| e.to_string())?;
    eprintln!(
        "puck: gc  removed_packages={}  removed_index_keys={}  kept={}",
        report.removed_packages, report.removed_index_keys, report.kept_packages
    );
    Ok(())
}
