//! `puck` - native PHP package manager (Laravel-first).

mod dist_urls;
use clap::{Parser, Subcommand};
use puck_autoload::{DumpOptions, dump, dump_is_current};
use puck_install::{
    ExecuteTimings, InstallAction, InstallOptions, execute_install, install_binaries, plan_install,
    read_installed, reconcile_vendor_presence,
};
use puck_laravel::{DiscoverStatus, discover};
use puck_lock::{LockFile, abandoned_warnings};
use puck_manifest::{
    Manifest, PackageRequirement, add_requirement_preserving, remove_requirement_preserving,
    sort_packages_enabled,
};
use puck_plugins::{
    PestPluginDumpStatus, PhpstanExtensionInstallStatus, refuse_message, run_pest_plugin_dump,
    run_phpstan_extension_installer, unsupported_allowed_plugins,
};
use puck_registry::{P2Loader, load_p2_optional};
use puck_resolver::{
    UpdateAllowTransitive, expand_update_unlock, load_path_packages, resolve_lock_document,
};
use puck_scripts::{RunScriptsOptions, run_install_scripts};
use puck_store::Store;
use serde_json::Value;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

#[derive(Debug, Parser)]
#[command(
    name = "puck",
    version = env!("PUCK_FULL_VERSION"),
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
        /// Also unlock dependencies of newly required packages, except root requirements (`-w`)
        #[arg(long, short = 'w', alias = "update-with-dependencies")]
        with_dependencies: bool,
        /// Also unlock dependencies including root requirements (`-W`)
        #[arg(long, short = 'W', alias = "update-with-all-dependencies")]
        with_all_dependencies: bool,
        /// Edit composer.json and lock only; do not install into vendor/
        #[arg(long)]
        no_install: bool,
        /// Optional VCR registry root (`packagist/p2/`). Defaults to `$PUCK_REGISTRY`; omit for live Packagist.
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
        /// Optional VCR registry root (`packagist/p2/`). Defaults to `$PUCK_REGISTRY`; omit for live Packagist.
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
        /// Also update dependencies of listed packages, except root requirements (`-w`)
        #[arg(long, short = 'w')]
        with_dependencies: bool,
        /// Also update dependencies of listed packages, including root requirements (`-W`)
        #[arg(long, short = 'W')]
        with_all_dependencies: bool,
        #[arg(long)]
        no_install: bool,
        #[arg(long, value_name = "DIR")]
        registry: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        working_dir: Option<PathBuf>,
    },
    /// Report security advisories for locked packages (offline p2; early M4)
    Audit {
        /// Do not audit packages listed under packages-dev
        #[arg(long)]
        no_dev: bool,
        /// Optional VCR registry root (`packagist/p2/`). Defaults to `$PUCK_REGISTRY`; omit for live Packagist.
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
    /// Preflight: blockers / warnings before switching from Composer
    Doctor {
        /// Project directory (defaults to current directory)
        #[arg(long, value_name = "DIR")]
        working_dir: Option<PathBuf>,
        /// Machine-readable findings for CI
        #[arg(long)]
        json: bool,
    },
    /// Shared store utilities (M2)
    Store {
        #[command(subcommand)]
        command: StoreCommands,
    },
    /// Upgrade the puck binary from GitHub Releases
    Upgrade {
        /// Install this version instead of latest (`0.1.0` or `v0.1.0`)
        #[arg(long)]
        version: Option<String>,
        /// Restore `~/.puck/bin/puck.previous`
        #[arg(long)]
        rollback: bool,
        /// Canary channel (not available in 0.1)
        #[arg(long)]
        canary: bool,
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

fn run_doctor(working_dir: Option<PathBuf>, json: bool) -> Result<ExitCode, String> {
    let root = working_dir.unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    let report = puck_diagnostics::diagnose(&root).map_err(|e| e.to_string())?;
    if json {
        let body = puck_diagnostics::format_json(&report).map_err(|e| e.to_string())?;
        println!("{body}");
    } else {
        println!("{}", puck_diagnostics::format_text(&report));
    }
    Ok(ExitCode::from(report.exit_code()))
}

fn current_pkg_version() -> &'static str {
    // clap embeds `0.1.0 (<sha> <date>)`; compare / UA use the bare crate version.
    env!("CARGO_PKG_VERSION")
}

fn should_skip_update_notify(command: &Commands) -> bool {
    match command {
        Commands::Doctor { json: true, .. } => true,
        Commands::Upgrade { .. } => true,
        _ => false,
    }
}

fn maybe_notify_after_command(skip: bool) {
    if skip {
        return;
    }
    puck_update::maybe_notify_update(puck_update::NotifyOptions {
        current_version: current_pkg_version().to_owned(),
        manifest_url: None,
        cache_path: None,
        force_tty: None,
        ignore_env_disable: false,
    });
}

fn run_upgrade(version: Option<String>, rollback: bool, canary: bool) -> Result<(), String> {
    if canary {
        return Err(puck_update::Error::CanaryUnavailable.to_string());
    }
    if rollback && version.is_some() {
        return Err("use either --version or --rollback, not both".into());
    }
    if rollback {
        let path = puck_update::rollback_exe(None).map_err(|e| e.to_string())?;
        eprintln!("puck: rolled back to {}", path.display());
        return Ok(());
    }
    let outcome = puck_update::upgrade(puck_update::UpgradeOptions {
        version,
        manifest_url: None,
        current_version: current_pkg_version().to_owned(),
        current_exe: None,
        work_dir: None,
    })
    .map_err(|e| e.to_string())?;
    if let Some(warn) = &outcome.minisign_skipped_warning {
        eprintln!("puck: warning: {warn}");
    }
    eprintln!(
        "puck: upgraded {} -> {} ({}) at {}",
        outcome.from_version,
        outcome.to_version,
        outcome.target,
        outcome.path.display()
    );
    if outcome.minisign_verified {
        eprintln!("puck: minisign signature verified");
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let skip_notify = should_skip_update_notify(&cli.command);
    let code = match cli.command {
        Commands::Install {
            no_dev,
            optimize,
            offline,
            no_scripts,
            strict_native,
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
                strict_native,
            )) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("puck: {err}");
                    ExitCode::from(1)
                }
            }
        }
        Commands::Doctor { working_dir, json } => match run_doctor(working_dir, json) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("puck: {err}");
                ExitCode::from(1)
            }
        },
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
            with_dependencies,
            with_all_dependencies,
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
                with_dependencies,
                with_all_dependencies,
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
            with_dependencies,
            with_all_dependencies,
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
            match runtime.block_on(run_update(
                working_dir,
                packages,
                with_dependencies,
                with_all_dependencies,
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
        Commands::Audit {
            no_dev,
            registry,
            working_dir,
        } => match run_audit(working_dir, no_dev, registry) {
            Ok(0) => ExitCode::SUCCESS,
            Ok(_) => ExitCode::from(1),
            Err(err) => {
                eprintln!("puck: {err}");
                ExitCode::from(1)
            }
        },
        Commands::Upgrade {
            version,
            rollback,
            canary,
        } => match run_upgrade(version, rollback, canary) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("puck: {err}");
                ExitCode::from(1)
            }
        },
        Commands::Php { .. } => {
            eprintln!("puck: this command is not implemented yet");
            ExitCode::from(2)
        }
    };
    maybe_notify_after_command(skip_notify);
    code
}

async fn run_require(
    working_dir: Option<PathBuf>,
    packages: Vec<String>,
    dev: bool,
    with_dependencies: bool,
    with_all_dependencies: bool,
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

    let loader = p2_loader_for(registry, &root)?;
    let load_p2 = p2_getter(&loader);

    let mut unlock = Vec::new();
    let mut composer_text = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;

    let root_for_path: Value = serde_json::from_str(&composer_text).map_err(|e| e.to_string())?;
    let path_provided: std::collections::BTreeSet<String> =
        load_path_packages(&root, &root_for_path)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| p.package.name)
            .collect();

    for spec in &packages {
        let req = PackageRequirement::parse(spec).map_err(|e| e.to_string())?;
        // Ensure metadata is loadable before mutating the project (path repos exempt).
        if !path_provided.contains(&req.name) {
            match load_p2_optional(&loader, &req.name) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(format!(
                        "no p2 metadata for {} (live Packagist miss or VCR gap; set --registry / $PUCK_REGISTRY or check the package name)",
                        req.name
                    ));
                }
                Err(e) => {
                    return Err(format!("p2 metadata for {}: {e}", req.name));
                }
            }
        }
        let root_json: Value = serde_json::from_str(&composer_text).map_err(|e| e.to_string())?;
        let sort = sort_packages_enabled(&root_json);
        composer_text =
            add_requirement_preserving(&composer_text, &req.name, &req.constraint, dev, sort)
                .map_err(|e| e.to_string())?;
        unlock.push(req.name.clone());
        eprintln!(
            "puck: require {} {} ({})",
            req.name,
            req.constraint,
            if dev { "require-dev" } else { "require" }
        );
    }

    std::fs::write(&manifest_path, &composer_text).map_err(|e| e.to_string())?;
    let root_json: Value = serde_json::from_str(&composer_text).map_err(|e| e.to_string())?;

    let lock_bytes = if lock_path.is_file() {
        Some(std::fs::read(&lock_path).map_err(|e| e.to_string())?)
    } else {
        None
    };

    let mode = if with_all_dependencies {
        UpdateAllowTransitive::ListedWithTransitiveDeps
    } else if with_dependencies {
        UpdateAllowTransitive::ListedWithTransitiveDepsNoRootRequire
    } else {
        UpdateAllowTransitive::OnlyListed
    };

    if !matches!(mode, UpdateAllowTransitive::OnlyListed) {
        let Some(bytes) = lock_bytes.as_deref() else {
            return Err("--with-dependencies requires an existing composer.lock".into());
        };
        let mut root_names = Vec::new();
        for key in ["require", "require-dev"] {
            if let Some(map) = root_json.get(key).and_then(|v| v.as_object()) {
                for name in map.keys() {
                    if name.contains('/') {
                        root_names.push(name.to_ascii_lowercase());
                    }
                }
            }
        }
        unlock =
            expand_update_unlock(bytes, &root_names, &unlock, mode).map_err(|e| e.to_string())?;
        eprintln!(
            "puck: unlock {} package{} ({})",
            unlock.len(),
            if unlock.len() == 1 { "" } else { "s" },
            match mode {
                UpdateAllowTransitive::ListedWithTransitiveDepsNoRootRequire => "-w",
                UpdateAllowTransitive::ListedWithTransitiveDeps => "-W",
                UpdateAllowTransitive::OnlyListed => unreachable!(),
            }
        );
    }

    let lock_doc = resolve_lock_document(
        &composer_text,
        lock_bytes.as_deref(),
        &load_p2,
        &unlock,
        true,
        &root,
    )
    .map_err(|e| e.to_string())?;

    let lock_text = format!(
        "{}\n",
        serde_json::to_string_pretty(&lock_doc).map_err(|e| e.to_string())?
    );
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

    run_install(Some(root), false, false, false, false, false).await
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

    let loader = p2_loader_for(registry, &root)?;
    let load_p2 = p2_getter(&loader);

    let mut unlock = Vec::new();
    let mut composer_text = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;

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
        let (next, found) =
            remove_requirement_preserving(&composer_text, &name).map_err(|e| e.to_string())?;
        if !found {
            return Err(format!(
                "{name} is not required in composer.json require or require-dev"
            ));
        }
        composer_text = next;
        unlock.push(name.clone());
        eprintln!("puck: remove {name}");
    }

    std::fs::write(&manifest_path, &composer_text).map_err(|e| e.to_string())?;

    let lock_bytes = std::fs::read(&lock_path).map_err(|e| e.to_string())?;
    let lock_doc = resolve_lock_document(
        &composer_text,
        Some(&lock_bytes),
        &load_p2,
        &unlock,
        true,
        &root,
    )
    .map_err(|e| e.to_string())?;

    let lock_text = format!(
        "{}\n",
        serde_json::to_string_pretty(&lock_doc).map_err(|e| e.to_string())?
    );
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

    run_install(Some(root), false, false, false, false, false).await
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

    let loader = p2_loader_for(registry, &root)?;
    let load_p2 = p2_getter(&loader);
    let composer_text = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
    let lock_bytes = std::fs::read(&lock_path).map_err(|e| e.to_string())?;

    // Empty unlock: keep every locked package fixed; rewrite dump from p2 / path repos.
    let lock_doc = resolve_lock_document(
        &composer_text,
        Some(&lock_bytes),
        &load_p2,
        &[],
        true,
        &root,
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
    run_install(Some(root), false, false, false, false, false).await
}

async fn run_update(
    working_dir: Option<PathBuf>,
    packages: Vec<String>,
    with_dependencies: bool,
    with_all_dependencies: bool,
    no_install: bool,
    registry: Option<PathBuf>,
) -> Result<(), String> {
    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = root.join("composer.json");
    let lock_path = root.join("composer.lock");
    if !manifest_path.is_file() {
        return Err(format!("no composer.json in {}", root.display()));
    }

    let loader = p2_loader_for(registry, &root)?;
    let load_p2 = p2_getter(&loader);
    let composer_text = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
    let lock_bytes = if lock_path.is_file() {
        Some(std::fs::read(&lock_path).map_err(|e| e.to_string())?)
    } else {
        None
    };

    let listed: Vec<String> = if packages.is_empty() {
        Vec::new()
    } else {
        packages
            .iter()
            .map(|s| {
                PackageRequirement::parse(s).map(|r| r.name).or_else(|_| {
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

    let mode = if with_all_dependencies {
        UpdateAllowTransitive::ListedWithTransitiveDeps
    } else if with_dependencies {
        UpdateAllowTransitive::ListedWithTransitiveDepsNoRootRequire
    } else {
        UpdateAllowTransitive::OnlyListed
    };

    let unlock: Vec<String> = if listed.is_empty() {
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
    } else if matches!(mode, UpdateAllowTransitive::OnlyListed) {
        listed.clone()
    } else {
        let Some(bytes) = lock_bytes.as_deref() else {
            return Err("--with-dependencies requires an existing composer.lock".into());
        };
        let root_json: Value = serde_json::from_str(&composer_text).map_err(|e| e.to_string())?;
        let mut root_names = Vec::new();
        for key in ["require", "require-dev"] {
            if let Some(map) = root_json.get(key).and_then(|v| v.as_object()) {
                for name in map.keys() {
                    if !name.contains('/') {
                        continue; // skip platform
                    }
                    root_names.push(name.to_ascii_lowercase());
                }
            }
        }
        expand_update_unlock(bytes, &root_names, &listed, mode).map_err(|e| e.to_string())?
    };

    if listed.is_empty() {
        eprintln!("puck: update (all)");
    } else {
        let mode_label = match mode {
            UpdateAllowTransitive::OnlyListed => "",
            UpdateAllowTransitive::ListedWithTransitiveDepsNoRootRequire => " -w",
            UpdateAllowTransitive::ListedWithTransitiveDeps => " -W",
        };
        eprintln!(
            "puck: update{} ({} package{})",
            mode_label,
            unlock.len(),
            if unlock.len() == 1 { "" } else { "s" }
        );
        for name in &unlock {
            eprintln!("puck:   {name}");
        }
    }

    let lock_doc = resolve_lock_document(
        &composer_text,
        lock_bytes.as_deref(),
        &load_p2,
        &unlock,
        true,
        &root,
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
    run_install(Some(root), false, false, false, false, false).await
}

fn write_lock_file(path: &std::path::Path, lock_doc: &Value) -> Result<(), String> {
    let lock_text = format!(
        "{}\n",
        serde_json::to_string_pretty(lock_doc).map_err(|e| e.to_string())?
    );
    let lock_text = reindent_json_pretty_4(&lock_text);
    std::fs::write(path, &lock_text).map_err(|e| e.to_string())
}

fn run_audit(
    working_dir: Option<PathBuf>,
    no_dev: bool,
    registry: Option<PathBuf>,
) -> Result<u32, String> {
    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let lock_path = root.join("composer.lock");
    if !lock_path.is_file() {
        return Err(format!("no composer.lock in {}", root.display()));
    }
    let lock = LockFile::from_path(&lock_path).map_err(|e| e.to_string())?;
    let registry_root = resolve_registry_root(registry)?.ok_or_else(|| {
        "audit requires --registry DIR or $PUCK_REGISTRY (VCR-only; live advisory fetch not implemented)"
            .to_string()
    })?;

    let mut packages: Vec<(&str, &str)> = lock
        .packages
        .iter()
        .map(|p| (p.name.as_str(), p.version.as_str()))
        .collect();
    if !no_dev {
        packages.extend(
            lock.packages_dev
                .iter()
                .map(|p| (p.name.as_str(), p.version.as_str())),
        );
    }

    let mut hits: u32 = 0;
    let mut missing_p2: u32 = 0;
    for (name, version) in packages {
        // Platform requirements are not Packagist packages.
        if !name.contains('/') {
            continue;
        }
        let p2 = puck_registry::p2_path(&registry_root, name);
        if !p2.is_file() {
            missing_p2 += 1;
            continue;
        }
        let found = puck_registry::find_advisory_hits(&registry_root, name, version)
            .map_err(|e| e.to_string())?;
        for hit in found {
            hits += 1;
            println!(
                "puck: {} {} has advisory {} ({})",
                hit.package, hit.version, hit.advisory_id, hit.affected_versions
            );
        }
    }

    if missing_p2 > 0 {
        eprintln!(
            "puck: skipped {missing_p2} package{} with no recorded p2 metadata",
            if missing_p2 == 1 { "" } else { "s" }
        );
    }

    if hits == 0 {
        eprintln!("puck: no security vulnerability advisories found");
    } else {
        eprintln!("puck: found {hits} security advisories");
    }

    Ok(hits)
}

fn resolve_registry_root(registry: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    let Some(registry_root) =
        registry.or_else(|| std::env::var_os("PUCK_REGISTRY").map(PathBuf::from))
    else {
        return Ok(None);
    };
    let p2_dir = registry_root.join("packagist/p2");
    if !p2_dir.is_dir() {
        return Err(format!(
            "registry p2 dir missing: {} (expected packagist/p2 under registry root)",
            p2_dir.display()
        ));
    }
    Ok(Some(registry_root))
}

fn p2_loader_for(
    registry: Option<PathBuf>,
    project_dir: &std::path::Path,
) -> Result<P2Loader, String> {
    let registry_root = resolve_registry_root(registry)?;
    let mut loader = P2Loader::from_env_and_registry(registry_root, Some(project_dir))
        .map_err(|e| e.to_string())?;
    let manifest_path = project_dir.join("composer.json");
    if manifest_path.is_file() {
        let text = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
        let root: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        loader = loader.with_composer_json(&root);
    }
    Ok(loader)
}

fn p2_getter<'a>(
    loader: &'a P2Loader,
) -> impl Fn(&str) -> std::result::Result<Option<Vec<u8>>, String> + 'a {
    move |name: &str| load_p2_optional(loader, name).map_err(|e| e.to_string())
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
    strict_native: bool,
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

    // Tier 3 (M3 gate 4b): never silently skip an allowed lock plugin without a
    // native adapter. Refusal is unconditional; --strict-native is reserved for
    // a future Tier 2 PHP plugin host.
    let _ = strict_native;
    let lock_plugin_pkgs = lock
        .packages
        .iter()
        .chain(lock.packages_dev.iter())
        .map(|p| (p.name.clone(), p.package_type().to_string()));
    let allows = |name: &str| manifest.as_ref().is_some_and(|m| m.allows_plugin(name));
    let unsupported = unsupported_allowed_plugins(lock_plugin_pkgs, allows);
    if !unsupported.is_empty() {
        return Err(refuse_message(&unsupported));
    }

    let installed = read_installed(root.join("vendor/composer")).map_err(|e| e.to_string())?;
    let options = InstallOptions { no_dev, offline };
    let mut plan = plan_install(&lock, &installed, options).map_err(|e| e.to_string())?;

    // CLI `-o` wins; otherwise honour composer.json `config.optimize-autoloader`.
    let optimize = optimize || manifest.as_ref().is_some_and(Manifest::optimize_autoloader);

    // Warm-keep O(1): lock vs installed.json only. Skip reconcile_vendor_presence
    // (is_dir per Keep package) and install_binaries. Trust installed.json; missing
    // package dirs without an installed.json update are an edge case.
    // With PUCK_TIMINGS=1, plan_ms/total_ms should stay flat vs package count.
    if plan.is_noop()
        && dump_meta_current(&root, &lock.content_hash, optimize)
        && dump_is_current(&root, &lock)
        && root.join("vendor").is_dir()
    {
        let keep = plan.packages.len();
        let plan_ms = plan_started.elapsed().as_millis();
        eprintln!(
            "puck: plan  install=0  update=0  keep={keep}  remove=0  (lock {})",
            lock.content_hash
        );
        if optimize {
            eprintln!("puck: optimize-autoloader enabled");
        }
        eprintln!("puck: nothing to install");
        for line in abandoned_warnings(&lock.packages, &lock.packages_dev, !no_dev) {
            eprintln!("puck: {line}");
        }
        eprintln!("puck: autoload up to date");
        eprintln!("puck: done");
        print_install_timings(
            plan_ms,
            &ExecuteTimings::default(),
            0,
            0,
            0,
            total_started.elapsed().as_millis(),
        );
        return Ok(());
    }

    reconcile_vendor_presence(&mut plan, &root.join("vendor"));

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

    // Composer Installer::run: warn on abandoned locked packages (stderr).
    for line in abandoned_warnings(&lock.packages, &lock.packages_dev, !no_dev) {
        eprintln!("puck: {line}");
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

    // Tier 1: pestphp/pest-plugin (post-autoload-dump equivalent).
    let allow_pest_plugin = manifest
        .as_ref()
        .is_none_or(|m| m.allows_plugin("pestphp/pest-plugin"));
    match run_pest_plugin_dump(&root, allow_pest_plugin).map_err(|e| e.to_string())? {
        PestPluginDumpStatus::Written { plugin_count, .. } => {
            eprintln!("puck: pest plugins dumped ({plugin_count})");
        }
        PestPluginDumpStatus::Skipped => {}
    }

    // Tier 1: phpstan/extension-installer (POST_INSTALL_CMD equivalent).
    let allow_phpstan_plugin = manifest
        .as_ref()
        .is_none_or(|m| m.allows_plugin("phpstan/extension-installer"));
    match run_phpstan_extension_installer(&root, allow_phpstan_plugin).map_err(|e| e.to_string())? {
        PhpstanExtensionInstallStatus::Written {
            extension_count, ..
        } => {
            eprintln!("puck: phpstan extensions registered ({extension_count})");
        }
        PhpstanExtensionInstallStatus::Skipped => {}
    }

    let mut scripts_ms = 0u128;
    if !no_scripts && let Some(ref manifest) = manifest {
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
    eprintln!(
        "puck: timing  fetch_cache_hit_ms={}",
        exec.fetch_cache_hit_ms
    );
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

    let allow_pest_plugin = manifest
        .as_ref()
        .is_none_or(|m| m.allows_plugin("pestphp/pest-plugin"));
    match run_pest_plugin_dump(&root, allow_pest_plugin).map_err(|e| e.to_string())? {
        PestPluginDumpStatus::Written { plugin_count, .. } => {
            eprintln!("puck: pest plugins dumped ({plugin_count})");
        }
        PestPluginDumpStatus::Skipped => {}
    }

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
