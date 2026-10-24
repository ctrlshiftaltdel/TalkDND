//! Relations between entities (P04; LetsPlay/docs/campaign-continuity.md §5.3).
//!
//! `talkdnd relation ...` records member/ally/enemy/located_at/holds/seeks/
//! knows links between entities, scoped to sessions and filtered for the
//! player view. TASK-0057 lands the command surface only, so the stubs bail
//! until TASK-0062.

use anyhow::{bail, Result};
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Add a relation (idempotent: an existing identical relation keeps its id)
    Add {
        /// member | ally | enemy | located_at | holds | seeks | knows
        #[arg(long = "type")]
        kind: String,
        /// Source entity id or name
        #[arg(long)]
        from: String,
        /// Target entity id or name
        #[arg(long)]
        to: String,
        /// Session the relation started in
        #[arg(long)]
        since: Option<String>,
        /// Where this knowledge comes from
        #[arg(long)]
        source: Option<String>,
        /// Note (logged)
        #[arg(long)]
        note: Option<String>,
        /// DM-only relation (never in the player view)
        #[arg(long)]
        dm_only: bool,
    },
    /// List relations (default: active in the current session)
    List {
        /// Only relations of this entity
        #[arg(long)]
        entity: Option<String>,
        /// Only relations active at this session
        #[arg(long)]
        at: Option<String>,
        /// Include ended relations
        #[arg(long)]
        all: bool,
    },
    /// End a relation (writes until_session; the record is kept)
    End {
        /// Relation id
        id: String,
        /// Session the relation ended in
        #[arg(long)]
        at: Option<String>,
        /// Note (logged)
        #[arg(long)]
        note: Option<String>,
    },
    /// Show every field of one relation
    Show { id: String },
    /// Entity summary plus its currently active relations (player-filtered)
    Context {
        /// Entity id or name
        entity: String,
    },
}

/// `relation add|list|end|show|context` (P04, TASK-0062).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = (root, save, cmd);
    bail!("not implemented in TASK-0057")
}
