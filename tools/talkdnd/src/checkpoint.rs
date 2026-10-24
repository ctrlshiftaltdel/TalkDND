//! Checkpoints (P01; LetsPlay/docs/campaign-continuity.md §5.1).
//!
//! `talkdnd checkpoint <text>` (text as arguments or on stdin) appends a
//! `## 存档点` section to the newest session log — the original behavior,
//! kept here so that lib.rs only registers and forwards.
//! `checkpoint snapshot|list|show|restore` manage whole-save checkpoint
//! snapshots; TASK-0057 lands the command surface only, so those bail with
//! `not implemented in TASK-0057` until TASK-0059.

use anyhow::{bail, Result};
use clap::Subcommand;
use std::io::Read;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// New checkpoint: snapshot the whole save and write a manifest
    Snapshot {
        /// Why this checkpoint is taken (logged)
        #[arg(long)]
        reason: Option<String>,
        /// Single-line JSON to stdout (the manifest content)
        #[arg(long)]
        json: bool,
    },
    /// List checkpoints: id, created, reason, log anchor, usable
    List {
        /// Single-line JSON to stdout
        #[arg(long)]
        json: bool,
    },
    /// Show one checkpoint: manifest fields, file count, total bytes
    Show {
        /// Checkpoint id
        id: String,
        /// Single-line JSON to stdout
        #[arg(long)]
        json: bool,
    },
    /// Restore a checkpoint (the whole save is backed up first); needs --yes
    Restore {
        /// Checkpoint id
        id: String,
        /// Confirm the restore (required)
        #[arg(long)]
        yes: bool,
        /// Single-line JSON to stdout
        #[arg(long)]
        json: bool,
    },
}

/// `checkpoint snapshot|list|show|restore` (P01, TASK-0059).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = (root, save, cmd);
    bail!("not implemented in TASK-0057")
}

/// `checkpoint <text>` (arguments or stdin): append a `## 存档点` section to
/// the newest session log. Behavior unchanged from before the checkpoint
/// command group split; it lives here so lib.rs stays forwarding-only.
pub fn append(root: &Path, save: &Path, text: Vec<String>) -> Result<()> {
    let mut body = text.join(" ");
    if body.trim().is_empty() {
        std::io::stdin().read_to_string(&mut body)?;
    }
    if body.trim().is_empty() {
        bail!("checkpoint text is empty (pass it as arguments or on stdin)");
    }
    let log = crate::save::log_path(save)?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M");
    crate::save::append(&log, &format!("\n## 存档点（{now}）\n\n{}\n", body.trim()))?;
    println!("checkpoint appended to {}", rel(root, &log));
    Ok(())
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}
