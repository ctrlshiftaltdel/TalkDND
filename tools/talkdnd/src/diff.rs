//! Checkpoint snapshots (P01; LetsPlay/docs/campaign-continuity.md §5.1).
//!
//! `talkdnd snapshot diff` is the structured difference between two
//! checkpoints (or a checkpoint and `current`): HP, temporary HP, level,
//! resources, conditions, coins, items, XP and party members. Missing or
//! renamed fields are reported as 未知, never guessed; the same input always
//! produces the same output. TASK-0057 lands the command surface only, so
//! the stub bails until TASK-0059.

use anyhow::{bail, Result};
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Structured diff of two checkpoints (or `current`); same input, same output
    Diff {
        /// First checkpoint id
        #[arg(value_name = "ID_A")]
        a: String,
        /// Second checkpoint id, or `current` for the live save
        #[arg(value_name = "ID_B|CURRENT")]
        b: String,
    },
}

/// `snapshot diff <idA> <idB|current>` (P01, TASK-0059).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = (root, save, cmd);
    bail!("not implemented in TASK-0057")
}
