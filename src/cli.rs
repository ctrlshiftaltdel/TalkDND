//! Command-line interface.
//!
//! Step 1 only defines the global flags. Subcommands and their handlers arrive
//! in later steps.

use std::path::PathBuf;

use clap::Parser;

/// Global options shared by every command.
#[derive(Debug, Parser)]
#[command(
    name = "combat-sim",
    version,
    about = "Headless D&D 5e (2024 rules) combat helper"
)]
pub struct Cli {
    /// Path to the persisted encounter state.
    #[arg(
        long,
        global = true,
        value_name = "FILE",
        default_value = "./combat-state.json"
    )]
    pub state: PathBuf,

    /// Seed for deterministic rolls (tests and replays only).
    #[arg(long, global = true, value_name = "N")]
    pub seed: Option<u64>,

    /// Emit machine-readable JSON where supported.
    #[arg(long, global = true)]
    pub json: bool,
}

/// Parse the command line and dispatch the requested command.
///
/// No commands exist yet, so parsing is the whole job for now.
pub fn run() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    Ok(())
}
