//! `puck` - native PHP package manager (Laravel-first).

use clap::{Parser, Subcommand};

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
    /// Install dependencies from composer.lock (M1)
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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Install { .. }
        | Commands::Update { .. }
        | Commands::Require { .. }
        | Commands::Remove { .. }
        | Commands::DumpAutoload { .. }
        | Commands::Store { .. }
        | Commands::Php { .. } => {
            eprintln!("puck: this command is not implemented yet");
            std::process::exit(2);
        }
    }
}
