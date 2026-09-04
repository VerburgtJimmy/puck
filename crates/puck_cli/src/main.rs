//! `puck` - native PHP package manager (Laravel-first).

use clap::{Parser, Subcommand};
use puck_install::{InstallAction, InstallOptions, plan_install, read_installed};
use puck_lock::LockFile;
use puck_manifest::Manifest;
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

        /// Fail if any plugin lacks a native adapter
        #[arg(long)]
        strict_native: bool,

        /// Project directory (defaults to current directory)
        #[arg(long, value_name = "DIR")]
        working_dir: Option<PathBuf>,
    },
    /// Update packages within constraints (M3)
    Update {
        /// Package names to update; empty means all
        packages: Vec<String>,
    },
    /// Add a package to composer.json and install (M3)
    Require {
        packages: Vec<String>,
        #[arg(long)]
        dev: bool,
    },
    /// Remove a package (M2)
    Remove { packages: Vec<String> },
    /// Regenerate autoload files (M2)
    #[command(name = "dump-autoload", visible_alias = "dumpautoload")]
    DumpAutoload {
        #[arg(long, short = 'o')]
        optimize: bool,
        #[arg(long, short = 'a')]
        authoritative: bool,
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
            optimize: _,
            offline: _,
            strict_native: _,
            working_dir,
        } => match run_install(working_dir, no_dev) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("puck: {err}");
                ExitCode::from(1)
            }
        },
        Commands::Update { .. }
        | Commands::Require { .. }
        | Commands::Remove { .. }
        | Commands::DumpAutoload { .. }
        | Commands::Store { .. }
        | Commands::Php { .. } => {
            eprintln!("puck: this command is not implemented yet");
            ExitCode::from(2)
        }
    }
}

fn run_install(working_dir: Option<PathBuf>, no_dev: bool) -> Result<(), String> {
    let root = working_dir.unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = root.join("composer.json");
    let lock_path = root.join("composer.lock");

    if !lock_path.is_file() {
        return Err(format!(
            "no composer.lock in {} (run Composer once to lock, or wait for puck update)",
            root.display()
        ));
    }

    // Validate manifest when present; install still keys off the lock.
    if manifest_path.is_file() {
        let manifest = Manifest::from_path(&manifest_path).map_err(|e| e.to_string())?;
        eprintln!("puck: project {}", manifest.pretty_name);
    }

    let lock = LockFile::from_path(&lock_path).map_err(|e| e.to_string())?;
    let installed = read_installed(root.join("vendor/composer")).map_err(|e| e.to_string())?;
    let plan =
        plan_install(&lock, &installed, InstallOptions { no_dev }).map_err(|e| e.to_string())?;

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

    // Execution (fetch / link / autoload) lands in M1.
    Err("install execution is not implemented yet (planner only). Fetch and link come next.".into())
}
