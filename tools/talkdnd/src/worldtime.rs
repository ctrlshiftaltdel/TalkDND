//! World time (P06; LetsPlay/docs/campaign-continuity.md §5.4).
//!
//! `talkdnd time ...` keeps an optional in-world calendar with due-dated
//! events. It is off by default, never advances on its own, and refuses
//! every write while disabled. TASK-0057 lands the command surface only, so
//! the stubs bail until TASK-0063.

use anyhow::{bail, Result};
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Show enabled/calendar/date/phase/elapsed_days and the event count
    Show,
    /// Enable world time
    Enable,
    /// Disable world time
    Disable,
    /// Set the date label (and optional phase); refused while disabled
    Set {
        /// New date label
        #[arg(long)]
        date: String,
        /// New phase label
        #[arg(long)]
        phase: Option<String>,
    },
    /// Advance the clock (pure addition; at least one of --days/--hours)
    Advance {
        /// Days to add
        #[arg(long)]
        days: Option<u32>,
        /// Hours to add
        #[arg(long)]
        hours: Option<u32>,
    },
    /// Due-dated events
    Event {
        #[command(subcommand)]
        cmd: EventCmd,
    },
}

#[derive(Subcommand, Clone)]
pub enum EventCmd {
    /// Add a due-dated event
    Add {
        /// Due label (the date label it comes due on)
        #[arg(long)]
        due: String,
        #[arg(long)]
        title: String,
        /// DM-only event (never in the player view)
        #[arg(long)]
        dm_only: bool,
        /// Note (may reference entity ids)
        #[arg(long)]
        note: Option<String>,
    },
    /// List all events (pending ones due today are marked)
    List,
    /// Mark an event done
    Done { id: String },
    /// Skip an event
    Skip { id: String },
}

/// `time show|enable|disable|set|advance|event ...` (P06, TASK-0063).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = (root, save, cmd);
    bail!("not implemented in TASK-0057")
}
