//! `puck` - native PHP package manager (Laravel-first).

use clap::{Parser, Subcommand};
use puck_autoload::{DumpOptions, dump, dump_is_current};
use puck_install::{
    InstallAction, InstallOptions, execute_install, install_binaries, plan_install,
    read_installed, reconcile_vendor_presence,
};
use puck_laravel::{DiscoverStatus, discover};
use puck_lock::LockFile;
use puck_manifest::Manifest;
use puck_scripts::{RunScriptsOptions, run_install_scripts};
use puck_store::Store;
use std::path::PathBuf;
use std::process::ExitCode;

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
    /// Update packages within constraints (M3)
    Update { packages: Vec<String> },
    /// Add a package to composer.json and install (M3)
    Require {
        packages: Vec<String>,
        #[arg(long)]
        dev: bool,
    },
    /// Remove a package (M2)
    Remove { packages: Vec<String> },
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
        Commands::Update { .. }
        | Commands::Require { .. }
        | Commands::Remove { .. }
        | Commands::Php { .. } => {
            eprintln!("puck: this command is not implemented yet");
            ExitCode::from(2)
        }
    }
}

async fn run_install(
    working_dir: Option<PathBuf>,
    no_dev: bool,
    offline: bool,
    optimize: bool,
    no_scripts: bool,
) -> Result<(), String> {
    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = root.join("composer.json");
    let lock_path = root.join("composer.lock");

    if !lock_path.is_file() {
        return Err(format!(
            "no composer.lock in {} (run Composer once to lock, or wait for puck update)",
            root.display()
        ));
    }

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

    eprintln!(
        "puck: plan  install={install}  update={update}  keep={keep}  remove={remove}  (lock {})",
        lock.content_hash
    );
    if optimize {
        eprintln!("puck: optimize-autoloader enabled");
    }

    let packages_changed = install + update + remove > 0;
    if !packages_changed {
        eprintln!("puck: nothing to install");
        install_binaries(&root, &lock, no_dev).map_err(|e| e.to_string())?;
    } else {
        let store = Store::default_global();
        execute_install(&root, &lock, &plan, options, &store, manifest.as_ref())
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
        return Ok(());
    }

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
    eprintln!(
        "puck: dumped autoload{}",
        if optimize { " (-o)" } else { "" }
    );

    let mut packages_written = false;
    match discover(&root).map_err(|e| e.to_string())? {
        DiscoverStatus::Written { package_count, .. } => {
            eprintln!("puck: discovered {package_count} packages");
            packages_written = true;
        }
        DiscoverStatus::Skipped => {}
    }

    if !no_scripts
        && let Some(ref manifest) = manifest
    {
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

        // ComposerScripts::clearCompiled deletes packages.php; rewrite if gone.
        if packages_written {
            let packages_php = root.join("bootstrap/cache/packages.php");
            if !packages_php.is_file() {
                match discover(&root).map_err(|e| e.to_string())? {
                    DiscoverStatus::Written { package_count, .. } => {
                        eprintln!("puck: rediscovered {package_count} packages after scripts");
                    }
                    DiscoverStatus::Skipped => {}
                }
            }
        }
    }

    eprintln!("puck: done");
    Ok(())
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
