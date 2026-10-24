//! Structural book index (P07; LetsPlay/docs/campaign-continuity.md §5.5).
//!
//! `talkdnd adventure index ...` builds and searches a title-level index of
//! a converted book (chapters, sections, figures, statblocks) so the DM
//! never reads a whole book to find one section. Output is titles, pages and
//! anchors only — never body text. TASK-0057 lands the command surface only,
//! so the stubs bail until TASK-0060.

use anyhow::{bail, Result};
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Build or refresh the index (unchanged sources are skipped)
    Build {
        /// Book slug or title (default: this save's adventure)
        #[arg(long)]
        book: Option<String>,
    },
    /// Show the index: titles, pages, anchors (never body text)
    Show {
        /// Book slug or title
        book: String,
    },
    /// Search titles/section/figure/statblock labels only
    Search {
        /// Book slug or title
        book: String,
        /// Words or phrase to find
        query: Vec<String>,
    },
}

/// `adventure index build|show|search` (P07, TASK-0060).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = (root, save, cmd);
    bail!("not implemented in TASK-0057")
}
