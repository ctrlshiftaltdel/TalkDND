//! Story entities (P04; LetsPlay/docs/campaign-continuity.md §5.3).
//!
//! `talkdnd entity ...` is the index of NPCs, locations, factions, quests
//! and items. It never replaces canon.md; queries filter dm_only entries and
//! retired entities. TASK-0057 lands the command surface only, so the stubs
//! bail until TASK-0062.

use anyhow::{bail, Result};
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Register a new entity (name/alias conflicts are rejected)
    Add {
        /// npc | location | faction | quest | item
        #[arg(long = "type")]
        kind: String,
        #[arg(long)]
        name: String,
        /// Alternative name (repeatable)
        #[arg(long)]
        alias: Vec<String>,
        /// Where or when it first appeared
        #[arg(long)]
        first_seen: Option<String>,
        /// DM-only entity (never in the player view)
        #[arg(long)]
        dm_only: bool,
    },
    /// List entities (default: active only)
    List {
        /// Only this type: npc | location | faction | quest | item
        #[arg(long = "type")]
        kind: Option<String>,
        /// Include retired entities (marked)
        #[arg(long)]
        all: bool,
    },
    /// Show every field of one entity and its relation count
    Show { id: String },
    /// Rename an entity (the old name becomes an alias)
    Rename {
        /// Entity id or name
        id: String,
        /// The new name
        name: String,
    },
    /// Retire an entity (the record is kept)
    Retire { id: String },
}

/// `entity add|list|show|rename|retire` (P04, TASK-0062).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = (root, save, cmd);
    bail!("not implemented in TASK-0057")
}
