//! Improvisation oracle (P08; LetsPlay/docs/campaign-continuity.md §5.6).
//!
//! `talkdnd oracle ...` answers yes/no questions and suggests event focus
//! and meaning phrases from this module's own original lists. Every command
//! is read-only on the save: no dice are faked (they go through combat-sim's
//! engine) and no authoritative state is written. TASK-0057 lands the
//! command surface only, so the stubs bail until TASK-0064.

use anyhow::{bail, Result};
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Yes or no (roll 1d6: 1-3 yes, 4-6 no) through combat-sim's dice engine
    YesNo {
        /// The question (optional)
        question: Vec<String>,
    },
    /// One event focus phrase from the built-in original list
    Event {
        /// Optional focus for the event
        #[arg(long)]
        focus: Option<String>,
    },
    /// One meaning phrase from the built-in original list, with the chaos value
    Meaning {
        /// Chaos level 1-9 (default 5)
        #[arg(long, default_value_t = 5)]
        chaos: u8,
    },
}

/// `oracle yes-no|event|meaning` (P08, TASK-0064).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = (root, save, cmd);
    bail!("not implemented in TASK-0057")
}
