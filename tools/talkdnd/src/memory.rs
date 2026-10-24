//! Long-term memory and narration style (P03; LetsPlay/docs/campaign-continuity.md §5.2).
//!
//! `talkdnd memory ...` keeps facts that survive session gaps;
//! `talkdnd settings style ...` holds the table's narration preferences.
//! P03 pairs the two groups (TASK-0061), so both Cmd enums live here.
//! TASK-0057 lands the command surface only: every subcommand bails with
//! `not implemented in TASK-0057` until TASK-0061.

use anyhow::{bail, Result};
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Record a long-term fact (empty text is rejected)
    Add {
        /// The fact to remember
        text: Vec<String>,
        /// player-confirmed (default) | import | manual
        #[arg(long, default_value = "player-confirmed")]
        source: String,
        /// Tag such as npc:<id> (repeatable)
        #[arg(long)]
        tag: Vec<String>,
    },
    /// List memories (default: player-confirmed only)
    List {
        /// Only entries with this tag
        #[arg(long)]
        tag: Option<String>,
        /// Also list import/manual entries (DM view)
        #[arg(long)]
        all: bool,
    },
    /// Remove one memory by id (unknown id exits 1)
    Remove { id: String },
}

/// `talkdnd settings style show|set` (same task as memory: P03, TASK-0061).
#[derive(Subcommand, Clone)]
pub enum StyleCmd {
    /// Show the current narration/detail/pace/npc_voice/options
    Show,
    /// Set one style field; --reason is required and logged
    Set {
        /// narration | detail | pace | npc_voice | options
        field: String,
        value: String,
        /// Why the style changes (required; written to the session log)
        #[arg(long)]
        reason: String,
    },
}

/// `memory add|list|remove` (P03, TASK-0061).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = (root, save, cmd);
    bail!("not implemented in TASK-0057")
}

/// `settings style show|set` (P03, TASK-0061).
pub fn run_style(root: &Path, save: &Path, cmd: StyleCmd) -> Result<()> {
    let _ = (root, save, cmd);
    bail!("not implemented in TASK-0057")
}
