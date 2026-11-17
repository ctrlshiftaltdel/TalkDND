//! Shared advancement-protocol control files and the cross-CLI transition gate.
//!
//! The advancement protocol (see `docs/ADVANCEMENT-LOCK-DESIGN.md`) owns two
//! fixed control files inside one save slot:
//!
//! * `.talkdnd.advancement.lock` — the stable lock file / initialisation mark;
//! * `.talkdnd.advancement.state` — the only authoritative protocol state.
//!
//! This crate is the single home for those names and for the **transition
//! gate** that `talkdnd` and `combat-sim` share (decision #292, `ADV-P1-1pre`).
//! Neither binary may grow its own copy of the semantics: a save that has
//! either control file has adopted the protocol, and every save-writing entry
//! that has not adopted it must fail closed instead of writing.
//!
//! What this crate does **not** do: it does not create, lock, read or repair
//! the control files, and it does not implement the SAVE lock or the D commit
//! flow. `has_protocol` only answers "is either control file present?"; the
//! rest belongs to later blocks (`ADV-P1-1a` … `ADV-P1-1e`).
//!
//! Fail-closed rules:
//!
//! * presence is decided with `symlink_metadata`, so a dangling or otherwise
//!   unreadable control file still counts as present;
//! * any metadata error other than "not found" counts as present;
//! * nothing here creates a control file. A save without one keeps its
//!   existing behaviour.

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Fixed lock file name inside one save slot.
pub const LOCK_FILE: &str = ".talkdnd.advancement.lock";
/// Fixed state file name inside one save slot.
pub const STATE_FILE: &str = ".talkdnd.advancement.state";

/// One line every affected command shows in `--help`, and the wording the
/// player-facing documentation repeats.
pub const REFUSAL_HELP: &str =
    "存档启用成长协议（.talkdnd.advancement.lock / .talkdnd.advancement.state）后，\
尚未接入成长协议的写命令以退出码 1 拒绝执行且不写任何目标文件；只读命令、`guard`/`secret list` \
的缓存与 `start` 的 digest/缓存仍可用。";

/// The lock file path inside `save_dir`.
pub fn lock_path(save_dir: &Path) -> PathBuf {
    save_dir.join(LOCK_FILE)
}

/// The state file path inside `save_dir`.
pub fn state_path(save_dir: &Path) -> PathBuf {
    save_dir.join(STATE_FILE)
}

fn present(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(err) if err.kind() == ErrorKind::NotFound => false,
        // Unreadable metadata is not proof of absence: fail closed.
        Err(_) => true,
    }
}

/// True when the save has either advancement control file.
///
/// The check never creates, opens for writing, locks or parses anything.
pub fn has_protocol(save_dir: &Path) -> bool {
    present(&lock_path(save_dir)) || present(&state_path(save_dir))
}

/// The control files that are present, in a stable order (for messages).
pub fn protocol_files(save_dir: &Path) -> Vec<&'static str> {
    let mut files = Vec::new();
    if present(&lock_path(save_dir)) {
        files.push(LOCK_FILE);
    }
    if present(&state_path(save_dir)) {
        files.push(STATE_FILE);
    }
    files
}

/// Walk up from `start` (a file, a directory, or a path that does not exist
/// yet) and return the nearest directory holding a control file.
///
/// This is how `combat-sim` finds the save for `--state`/`--out` paths it was
/// given instead of an explicit `--save`.
pub fn enclosing_protocol_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    while let Some(current) = dir {
        if has_protocol(current) {
            return Some(current.to_path_buf());
        }
        dir = current.parent();
    }
    None
}

// ------------------------------------------------------------------- class ----

/// How an entry relates to the advancement protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Reads the save only. Allowed, and the caller must not run its
    /// post-command audit write while the protocol is present.
    ReadOnly,
    /// Writes only control metadata (the secret-term cache, `start`'s
    /// `digest.md`/`.cache`). Allowed: no SAVE lock, never part of the user
    /// data backup.
    ControlMetadata,
    /// Creates a brand-new save slot. It cannot write an existing save, so the
    /// gate does not apply.
    NewSave,
    /// Writes save content through an entry that has not adopted the protocol.
    /// Refused (exit code 1) whenever the save has a control file.
    UnadoptedWrite,
}

impl Class {
    /// True when the entry may write while the protocol is present.
    pub fn allowed_under_protocol(self) -> bool {
        !matches!(self, Class::UnadoptedWrite)
    }
}

// ----------------------------------------------------------------- registry ----

/// Which binary an entry belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cli {
    Talkdnd,
    CombatSim,
}

impl Cli {
    /// The binary name used in messages and documentation.
    pub fn binary(self) -> &'static str {
        match self {
            Cli::Talkdnd => "talkdnd",
            Cli::CombatSim => "combat-sim",
        }
    }
}

/// One save-writing (or read-only) entry, as recorded in
/// `docs/ADVANCEMENT-WRITER-INVENTORY.md` §2.
///
/// The registry is the single source of truth both CLIs are tested against:
/// every row names the CLI form, the code location, the write targets, the
/// gate class and the reason for that class. `argv` is a representative
/// invocation used by the synthetic tests; the `{save}` token is replaced with
/// the absolute save directory.
#[derive(Debug, Clone, Copy)]
pub struct Writer {
    /// Owning binary.
    pub cli: Cli,
    /// Stable registry id, also used in the refusal message.
    pub entry: &'static str,
    /// CLI form(s) covered by this row.
    pub command: &'static str,
    /// Code location (file and function) at this head.
    pub location: &'static str,
    /// Save-relative write targets.
    pub targets: &'static str,
    /// Gate class.
    pub class: Class,
    /// Why the row has that class (read-only basis, write reason).
    pub basis: &'static str,
    /// Representative invocation; `{save}` is replaced by the save directory.
    pub argv: &'static [&'static str],
}

macro_rules! writer {
    ($cli:ident, $entry:literal, $command:literal, $location:literal, $targets:literal, $class:ident, $basis:literal, $argv:expr) => {
        Writer {
            cli: Cli::$cli,
            entry: $entry,
            command: $command,
            location: $location,
            targets: $targets,
            class: Class::$class,
            basis: $basis,
            argv: $argv,
        }
    };
}

/// Every entry the transition gate knows about, for both CLIs.
///
/// Rows mirror `docs/ADVANCEMENT-WRITER-INVENTORY.md` §2 one-for-one; the
/// inventory table is the prose copy of this list.
pub static WRITERS: &[Writer] = &[
    // ---------------------------------------------------------- talkdnd ----
    writer!(
        Talkdnd,
        "talkdnd:start",
        "talkdnd start",
        "lib.rs::dispatch Cmd::Start; start.rs::digest",
        "save-meta.yaml, sheets/*.yaml, combat/campaign.yaml, digest.md, .cache/, Sessions/*.md",
        UnadoptedWrite,
        "resume digest: migrates old schemas, rewrites campaign content sources, writes digest and cache",
        &["start", "--no-pdf2md"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:new",
        "talkdnd new",
        "lib.rs::new_save",
        "new Saves/<slot>/ tree",
        NewSave,
        "creates a new slot; an existing save with the protocol is never written",
        &["new", "--date", "2031-01-01"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:migrate",
        "talkdnd migrate (without --dry-run)",
        "lib.rs::dispatch Cmd::Migrate; save.rs::migrate_ruleset; migrate.rs::ensure",
        "combat/campaign.yaml, save-meta.yaml, sheets/*.yaml, Characters/, Saves/_backups/, Sessions/*.md, integrity.yaml",
        UnadoptedWrite,
        "whole-tree migration plus backup",
        &["migrate"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:migrate-dry-run",
        "talkdnd migrate --dry-run",
        "lib.rs::dispatch Cmd::Migrate; migrate.rs::ensure(dry_run)",
        "none",
        ReadOnly,
        "dry-run branch only reports; writes no backup and migrates nothing",
        &["migrate", "--dry-run"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:level-up",
        "talkdnd level-up <who> (structured and Markdown paths)",
        "lib.rs::dispatch Cmd::LevelUp; progress.rs::level_up",
        "sheets/<id>.yaml, combat/party-state.yaml, Characters/*.md, progress.yaml, Sessions/*.md, rolls.log",
        UnadoptedWrite,
        "old level-up still writes the sheet, party, Markdown, progress and logs",
        &["level-up", "aria"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:xp-add",
        "talkdnd xp add",
        "lib.rs::dispatch Cmd::Xp; progress.rs::xp_add",
        "progress.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes progress then appends the log",
        &["xp", "add", "10", "--reason", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:xp-show",
        "talkdnd xp show",
        "lib.rs::dispatch Cmd::Xp; progress.rs::xp_show",
        "none",
        ReadOnly,
        "reads sheets and progress.yaml only",
        &["xp", "show"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:loot",
        "talkdnd loot add|spend|give|split",
        "lib.rs::dispatch Cmd::Loot; progress.rs::loot",
        "progress.yaml, Sessions/*.md",
        UnadoptedWrite,
        "single-file progress write plus log append",
        &["loot", "add", "--gp", "1", "probe", "--reason", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:loot-show",
        "talkdnd loot show",
        "lib.rs::dispatch Cmd::Loot; progress.rs::loot_show",
        "none",
        ReadOnly,
        "reads progress.yaml only",
        &["loot", "show"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:scene",
        "talkdnd scene <name>",
        "lib.rs::dispatch Cmd::Scene; known.rs::scene_change; progress.rs::set_scene",
        "progress.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes the scene to progress.yaml and appends the log",
        &["scene", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:char",
        "talkdnd char <who> <op> (every op except the plain spells view)",
        "lib.rs::dispatch Cmd::Char; charsheet.rs::run; charsheet.rs::commit",
        "sheets/<id>.yaml, combat/party-state.yaml, Characters/*.md, Sessions/*.md",
        UnadoptedWrite,
        "commit() writes YAML, party state and Markdown; `char show` takes the same path",
        &["char", "aria", "damage", "1"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:char-spells-read",
        "talkdnd char <who> spells (plain view)",
        "lib.rs::dispatch Cmd::Char; charsheet.rs::run (early spells_view return)",
        "none",
        ReadOnly,
        "returns before commit(); adopt/sync/terrain are not part of this view",
        &["char", "aria", "spells"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:chargen-state",
        "talkdnd chargen roll|array|pointbuy|assign|asi|set",
        "lib.rs::chargen_cmd; chargen.rs::store",
        "chargen.json, Sessions/*.md, rolls.log",
        UnadoptedWrite,
        "every step rewrites chargen.json; roll also appends the log and rolls.log",
        &["chargen", "array"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:chargen-sheet",
        "talkdnd chargen sheet",
        "lib.rs::chargen_cmd; charsheet.rs::from_chargen; lib.rs::add_to_party",
        "Characters/<id>.md, combat/campaign.yaml, sheets/<id>.yaml",
        UnadoptedWrite,
        "writes the Markdown sheet, the campaign party line and the structured sheet",
        &["chargen", "sheet"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:chargen-confirm",
        "talkdnd chargen confirm",
        "lib.rs::chargen_cmd; progress.rs::loot; chargen.rs::store",
        "progress.yaml, Sessions/*.md, chargen.json",
        UnadoptedWrite,
        "first confirmation may grant starting gold (progress plus log), then rewrites chargen.json",
        &["chargen", "confirm"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:chargen-status",
        "talkdnd chargen status",
        "lib.rs::chargen_cmd ChargenCmd::Status",
        "none",
        ReadOnly,
        "prints the creation state and returns before chargen::store",
        &["chargen", "status"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:chargen-validate",
        "talkdnd chargen validate",
        "lib.rs::chargen_cmd ChargenCmd::Validate",
        "none",
        ReadOnly,
        "validates in memory and returns before chargen::store",
        &["chargen", "validate"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:companion-random",
        "talkdnd companion random / talkdnd npc random",
        "lib.rs::npc_cmd NpcCmd::Random",
        "npc-drafts/<n>.json, rolls.log, Sessions/*.md",
        UnadoptedWrite,
        "writes draft JSON and appends roll logs when a save exists",
        &["companion", "random", "--count", "1"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:companion-sheet",
        "talkdnd companion sheet / talkdnd npc sheet",
        "lib.rs::npc_cmd NpcCmd::Sheet; lib.rs::add_to_party",
        "Characters/<id>.md, combat/campaign.yaml",
        UnadoptedWrite,
        "writes the companion Markdown sheet and the campaign party line",
        &["companion", "sheet", "1", "--name", "Probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:check",
        "talkdnd check [<who> <what>]",
        "lib.rs::dispatch Cmd::Check; audit.rs::audit; check.rs::run",
        "integrity.yaml, rolls.log, Sessions/*.md",
        UnadoptedWrite,
        "audit may create the manifest for an old save; a real check also logs",
        &["check"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:integrity-status",
        "talkdnd integrity status",
        "lib.rs::dispatch Cmd::Integrity; lib.rs::print_audit; audit.rs::audit",
        "integrity.yaml",
        UnadoptedWrite,
        "first run on an old save creates the manifest and baseline",
        &["integrity", "status"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:integrity-accept",
        "talkdnd integrity accept --reason",
        "lib.rs::dispatch Cmd::Integrity; audit.rs::accept",
        "integrity.yaml, Sessions/*.md",
        UnadoptedWrite,
        "rewrites the manifest baseline and logs the reason",
        &["integrity", "accept", "--reason", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:checkpoint-text",
        "talkdnd checkpoint <text>",
        "lib.rs::dispatch Cmd::Checkpoint; checkpoint.rs::append",
        "Sessions/*.md, integrity.yaml",
        UnadoptedWrite,
        "appends a checkpoint section to the newest session log",
        &["checkpoint", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:checkpoint-snapshot",
        "talkdnd checkpoint snapshot",
        "lib.rs::dispatch Cmd::Checkpoint; checkpoint.rs::snapshot",
        "checkpoints/<id>/, integrity.yaml",
        UnadoptedWrite,
        "copies the whole save and writes the manifest",
        &["checkpoint", "snapshot"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:checkpoint-restore",
        "talkdnd checkpoint restore",
        "lib.rs::dispatch Cmd::Checkpoint; checkpoint.rs::restore_inner",
        "whole save, Saves/_backups/, integrity.yaml",
        UnadoptedWrite,
        "replaces the save tree from a checkpoint after backing it up",
        &["checkpoint", "restore", "probe", "--yes"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:checkpoint-read",
        "talkdnd checkpoint list|show",
        "lib.rs::dispatch Cmd::Checkpoint; checkpoint.rs::run List/Show",
        "none",
        ReadOnly,
        "reads the checkpoint directory and manifests only",
        &["checkpoint", "list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:snapshot-diff",
        "talkdnd snapshot diff",
        "lib.rs::dispatch Cmd::Snapshot; diff.rs::run",
        "none",
        ReadOnly,
        "compares checkpoints and the current tree without writing",
        &["snapshot", "diff", "probe", "current"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:log",
        "talkdnd log <text>",
        "lib.rs::dispatch Cmd::Log; save.rs::append",
        "Sessions/*.md",
        UnadoptedWrite,
        "appends one line to the newest session log",
        &["log", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:log-rolls",
        "talkdnd log rolls",
        "lib.rs::dispatch Cmd::Log; rolls.rs::show",
        "none",
        ReadOnly,
        "prints rolls.log without appending",
        &["log", "rolls"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:memory-write",
        "talkdnd memory add|remove",
        "lib.rs::dispatch Cmd::Memory; memory.rs::run; memory.rs::store",
        "memory.yaml, save-meta.yaml",
        UnadoptedWrite,
        "writes memory.yaml and may migrate an old save first",
        &["memory", "add", "gate-probe", "fact"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:memory-list",
        "talkdnd memory list",
        "lib.rs::dispatch Cmd::Memory; memory.rs::run List",
        "none",
        ReadOnly,
        "loads memory.yaml only",
        &["memory", "list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:entity-write",
        "talkdnd entity add|rename|retire",
        "lib.rs::dispatch Cmd::Entity; entity.rs::run; entity.rs::store; entity.rs::log_line",
        "entities.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes the entity ledger and appends the log",
        &["entity", "add", "--type", "npc", "--name", "Probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:entity-read",
        "talkdnd entity list|show",
        "lib.rs::dispatch Cmd::Entity; entity.rs::run List/Show",
        "none",
        ReadOnly,
        "loads entities.yaml only",
        &["entity", "list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:relation-write",
        "talkdnd relation add|end",
        "lib.rs::dispatch Cmd::Relation; graph.rs::run; graph.rs::store",
        "relations.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes the relation ledger and appends the log",
        &["relation", "add", "--type", "knows", "--from", "a", "--to", "b"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:relation-read",
        "talkdnd relation list|show|context",
        "lib.rs::dispatch Cmd::Relation; graph.rs::run List/Show/Context",
        "none",
        ReadOnly,
        "loads relations.yaml only",
        &["relation", "list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:time-write",
        "talkdnd time enable|disable|set|advance|event add|done|skip",
        "lib.rs::dispatch Cmd::Time; worldtime.rs::run; worldtime.rs::store; worldtime.rs::log_line",
        "world-time.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes world-time.yaml and appends the log",
        &["time", "enable"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:time-read",
        "talkdnd time show / talkdnd time event list",
        "lib.rs::dispatch Cmd::Time; worldtime.rs::run Show/EventList",
        "none",
        ReadOnly,
        "loads world-time.yaml only",
        &["time", "show"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:settings-dice",
        "talkdnd settings dice on|off",
        "lib.rs::dispatch Cmd::Settings; settings.rs::set_dice; migrate.rs::write_meta",
        "save-meta.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes the dice setting to save-meta.yaml and appends the log",
        &["settings", "dice", "on"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:settings-style-write",
        "talkdnd settings style set",
        "lib.rs::dispatch Cmd::Settings; memory.rs::run_style; migrate.rs::write_meta",
        "save-meta.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes the narration style to save-meta.yaml and appends the log",
        &["settings", "style", "set", "pace", "fast", "--reason", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:settings-style-read",
        "talkdnd settings style show",
        "lib.rs::dispatch Cmd::Settings; memory.rs::run_style Show",
        "none",
        ReadOnly,
        "reads memory and settings only",
        &["settings", "style", "show"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:settings-automation-write",
        "talkdnd settings automation set|reset",
        "lib.rs::dispatch Cmd::Settings; settings.rs::run_automation; migrate.rs::write_meta",
        "save-meta.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes automation overrides to save-meta.yaml and appends the log",
        &["settings", "automation", "set", "resolution", "auto", "--reason", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:settings-read",
        "talkdnd settings / talkdnd settings dice / talkdnd settings automation",
        "lib.rs::dispatch Cmd::Settings; settings.rs::lines; settings.rs::automation_lines",
        "none",
        ReadOnly,
        "prints settings; no subcommand and no value are read-only",
        &["settings"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:secret-write",
        "talkdnd secret add|remove",
        "lib.rs::dispatch Cmd::Secret; secret.rs::add/remove; secret.rs::store_manual; secret.rs::invalidate",
        "secret-terms.yaml, secret-terms.cache.json",
        UnadoptedWrite,
        "writes the manual term list and deletes the cache",
        &["secret", "add", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:secret-import-apply",
        "talkdnd secret import --apply",
        "lib.rs::dispatch Cmd::Secret; secret.rs::import_lines",
        "secret-terms.yaml, secret-terms.cache.json",
        UnadoptedWrite,
        "imports the reviewed terms and invalidates the cache",
        &["secret", "import", "--from", "{save}/gate-probe-terms.txt", "--apply"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:secret-import-dry-run",
        "talkdnd secret import (default dry-run)",
        "lib.rs::dispatch Cmd::Secret; secret.rs::import_lines (!apply)",
        "none",
        ReadOnly,
        "previews the import without writing the term list or the cache",
        &["secret", "import", "--from", "{save}/gate-probe-terms.txt"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:secret-scan",
        "talkdnd secret scan",
        "lib.rs::dispatch Cmd::Secret; secret.rs::scan_lines",
        "none",
        ReadOnly,
        "reads one file and reports counts and line numbers",
        &["secret", "scan"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:secret-list",
        "talkdnd secret list",
        "lib.rs::dispatch Cmd::Secret; secret.rs::list_lines; secret.rs::collect; secret.rs::store_cache",
        "secret-terms.cache.json",
        ControlMetadata,
        "control metadata: the cache is written atomically, stays out of the user backup and takes no SAVE lock",
        &["secret", "list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:guard",
        "talkdnd guard <text>",
        "lib.rs::dispatch Cmd::Guard; secret.rs::guard_terms; secret.rs::store_cache",
        "secret-terms.cache.json",
        ControlMetadata,
        "control metadata: the cache is written atomically, stays out of the user backup and takes no SAVE lock",
        &["guard", "gate-probe", "text"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:identify",
        "talkdnd identify / talkdnd identify --appearance",
        "lib.rs::dispatch Cmd::Identify; identify.rs::identify; identify.rs::set_appearance",
        "identified.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes identified.yaml and appends the log",
        &["identify", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:reveal-add",
        "talkdnd reveal add",
        "lib.rs::dispatch Cmd::Reveal; reveal.rs::add; reveal.rs::add_original",
        "reveals.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes the reveal ledger and appends the log",
        &["reveal", "add", "gate-probe", "fact"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:reveal-read",
        "talkdnd reveal list|check",
        "lib.rs::dispatch Cmd::Reveal; reveal.rs::list/check",
        "none",
        ReadOnly,
        "loads reveals.yaml only",
        &["reveal", "list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:original-add",
        "talkdnd original add",
        "lib.rs::dispatch Cmd::Original; original.rs::add; original.rs::store",
        "original.yaml, Sessions/*.md",
        UnadoptedWrite,
        "writes the original-content ledger and appends the log",
        &["original", "add", "--type", "npc", "--name", "Probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:original-plan",
        "talkdnd original plan",
        "lib.rs::dispatch Cmd::Original; original.rs::plan",
        "dm-plan.md",
        UnadoptedWrite,
        "creates dm-plan.md when it does not exist yet",
        &["original", "plan"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:original-read",
        "talkdnd original list|status",
        "lib.rs::dispatch Cmd::Original; original.rs::list/status_lines",
        "none",
        ReadOnly,
        "loads original.yaml only",
        &["original", "list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:handout-show",
        "talkdnd handout show",
        "lib.rs::dispatch Cmd::Handout; handout.rs::show",
        "handouts/<id>.<ext>, handouts.yaml, reveals.yaml, Sessions/*.md",
        UnadoptedWrite,
        "exports the image, records it shown and writes the reveal or log",
        &["handout", "show", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:handout-build",
        "talkdnd handout build",
        "lib.rs::dispatch Cmd::Handout; handout.rs::rebuild",
        "book-dir handouts.yaml",
        UnadoptedWrite,
        "rewrites the handout registry; a book folder may live inside the save, so it is treated as a writer",
        &["handout", "build"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:handout-mark",
        "talkdnd handout mark",
        "lib.rs::dispatch Cmd::Handout; handout.rs::mark",
        "book-dir handouts.yaml",
        UnadoptedWrite,
        "rewrites the handout registry; a book folder may live inside the save, so it is treated as a writer",
        &["handout", "mark", "gate-probe", "--player"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:handout-list",
        "talkdnd handout list",
        "lib.rs::dispatch Cmd::Handout; handout.rs::list",
        "none",
        ReadOnly,
        "reads the registry only",
        &["handout", "list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:adventure-section-write",
        "talkdnd adventure section ... --reason",
        "lib.rs::dispatch Cmd::Adventure; adventure.rs::section",
        "Sessions/*.md",
        UnadoptedWrite,
        "reading ahead of progress or a DM-only section with --reason appends the log",
        &["adventure", "section", "gate-probe", "--reason", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:adventure-section-read",
        "talkdnd adventure section (no --reason) / --list",
        "lib.rs::dispatch Cmd::Adventure; adventure.rs::section",
        "none",
        ReadOnly,
        "a normal read and --list write no log",
        &["adventure", "section", "--list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:adventure-search",
        "talkdnd adventure search",
        "lib.rs::dispatch Cmd::Adventure; adventure.rs::search",
        "none",
        ReadOnly,
        "searches the page edition without writing",
        &["adventure", "search", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:adventure-index-build",
        "talkdnd adventure index build",
        "lib.rs::dispatch Cmd::Adventure; bookindex.rs::run Build; bookindex.rs::build_one",
        "book-dir structured.json (may be inside the save)",
        UnadoptedWrite,
        "writes structured.json; --book may name a folder inside the save",
        &["adventure", "index", "build"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:adventure-index-read",
        "talkdnd adventure index show|search",
        "lib.rs::dispatch Cmd::Adventure; bookindex.rs::run Show/Search",
        "none",
        ReadOnly,
        "reads structured.json only",
        &["adventure", "index", "show", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:known",
        "talkdnd known",
        "lib.rs::dispatch Cmd::Known; known.rs::lines",
        "none",
        ReadOnly,
        "player view assembled from reads only",
        &["known"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:knowledge",
        "talkdnd knowledge search|get|cite|context|related",
        "lib.rs::dispatch Cmd::Knowledge; knowledge.rs::run",
        "none",
        ReadOnly,
        "read-only knowledge API",
        &["knowledge", "search", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:oracle",
        "talkdnd oracle",
        "lib.rs::dispatch Cmd::Oracle; oracle.rs::run",
        "none",
        ReadOnly,
        "reads and prints; never touches the save",
        &["oracle", "yes-no"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:options",
        "talkdnd options",
        "lib.rs::dispatch Cmd::Options; lib.rs::print_options",
        "none",
        ReadOnly,
        "prints ruleset options",
        &["options"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:data-status",
        "talkdnd data status",
        "lib.rs::data_cmd DataCmd::Status",
        "none",
        ReadOnly,
        "reads local data files only",
        &["data", "status"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:data-import",
        "talkdnd data import",
        "lib.rs::data_cmd DataCmd::Import; classdata.rs::run_import",
        "Rules/chargen/local/imported-<ruleset>.yaml (outside the save)",
        ReadOnly,
        "writes a rules data file outside the save, never save content",
        &["data", "import"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:library",
        "talkdnd library tidy|merge|scan|packs|choose",
        "lib.rs::dispatch Cmd::Library; library.rs",
        "book library outside the save (.talkdnd/books.yaml, Adventures/, Rules/)",
        ReadOnly,
        "writes the book library outside the save, never save content",
        &["library", "packs"]
    ),
    // ------------------------------------------------------ combat-sim ----
    writer!(
        CombatSim,
        "combat-sim:state-write",
        "combat-sim init|attack|grapple|shove|damage|heal|save|check|cast|use|condition add|remove|move|ready|trigger|ready-clear|fly|fall|interact|deathsave|next|rest|activate|add",
        "cli.rs::run_cli; cli.rs::run_state_command",
        "combat/*.state.json, combat/party-state.yaml, rolls.log, Sessions/*.md",
        UnadoptedWrite,
        "load, run, state.save, write_party_state and log append with no save-level lock",
        &["init"]
    ),
    writer!(
        CombatSim,
        "combat-sim:new",
        "combat-sim new",
        "cli.rs::run_cli Command::New; cli.rs::new_state",
        "combat/*.state.json, combat/party-state.yaml",
        UnadoptedWrite,
        "creates or overwrites a state file and writes party state",
        &["new", "--encounter", "gate-probe"]
    ),
    writer!(
        CombatSim,
        "combat-sim:quick",
        "combat-sim quick",
        "cli.rs::run_cli Command::Quick; cli.rs::quick_state",
        "combat/*.state.json, combat/party-state.yaml",
        UnadoptedWrite,
        "creates the ad-hoc encounter state and party state",
        &["quick", "--foe", "gate-probe"]
    ),
    writer!(
        CombatSim,
        "combat-sim:roll",
        "combat-sim roll",
        "cli.rs::run_cli Command::Roll; player.rs::append_roll",
        "combat/*.state.json, combat/party-state.yaml, rolls.log, Sessions/*.md",
        UnadoptedWrite,
        "appends rolls.log; with --state pointing at an existing file it also saves state and party",
        &["roll", "1d20"]
    ),
    writer!(
        CombatSim,
        "combat-sim:party-write",
        "combat-sim party rest|set-hp|sync",
        "cli.rs::run_party",
        "combat/party-state.yaml",
        UnadoptedWrite,
        "rewrites party-state.yaml",
        &["party", "set-hp", "gate-probe", "1"]
    ),
    writer!(
        CombatSim,
        "combat-sim:party-show",
        "combat-sim party show",
        "cli.rs::run_party PartyCommand::Show",
        "none",
        ReadOnly,
        "prints party state and returns before party.save",
        &["party", "show"]
    ),
    writer!(
        CombatSim,
        "combat-sim:content-import-srd",
        "combat-sim content import-srd",
        "cli.rs::run_content ContentCommand::ImportSrd",
        "--out content files (inside the save when --out points there)",
        UnadoptedWrite,
        "writes content files under --out; no dry-run exists",
        &["content", "import-srd", "--out", "{save}/content"]
    ),
    writer!(
        CombatSim,
        "combat-sim:content-import-srd-prose",
        "combat-sim content import-srd-prose",
        "cli.rs::run_content ContentCommand::ImportSrdProse; srd_prose::import",
        "--out content files (inside the save when --out points there)",
        UnadoptedWrite,
        "writes content files under --out; no dry-run exists",
        &["content", "import-srd-prose", "--edition", "2024", "--index", "gate-probe", "--out", "{save}/content"]
    ),
    writer!(
        CombatSim,
        "combat-sim:content-import-srd-pack",
        "combat-sim content import-srd-pack",
        "cli.rs::run_content ContentCommand::ImportSrdPack; srd_pack::import",
        "--out content files (inside the save when --out points there)",
        UnadoptedWrite,
        "writes content files under --out; no dry-run exists",
        &["content", "import-srd-pack", "--edition", "2024", "--src", "gate-probe", "--out", "{save}/content"]
    ),
    writer!(
        CombatSim,
        "combat-sim:pack-build",
        "combat-sim content pack build (without --dry-run)",
        "cli.rs::run_content PackCommand::Build; content::full_pack::build",
        "--out pack file (inside the save when --out points there)",
        UnadoptedWrite,
        "writes the full pack under --out; only --dry-run is read-only",
        &["content", "pack", "build", "--edition", "2024", "--book", "phb", "--index", "gate-probe", "--out", "{save}/content"]
    ),
    writer!(
        CombatSim,
        "combat-sim:pack-release",
        "combat-sim content pack release (without --dry-run)",
        "cli.rs::run_content PackCommand::Release; content::full_pack::release",
        "--out release bundle (inside the save when --out points there)",
        UnadoptedWrite,
        "writes the release bundle under --out; only --dry-run is read-only",
        &["content", "pack", "release", "--packs", "gate-probe", "--out", "{save}/content"]
    ),
    writer!(
        CombatSim,
        "combat-sim:pack-build-dry-run",
        "combat-sim content pack build --dry-run",
        "cli.rs::run_content PackCommand::Build (dry_run)",
        "none",
        ReadOnly,
        "--dry-run validates and reports without writing any pack file",
        &["content", "pack", "build", "--edition", "2024", "--book", "phb", "--index", "gate-probe", "--out", "{save}/content", "--dry-run"]
    ),
    writer!(
        CombatSim,
        "combat-sim:pack-release-dry-run",
        "combat-sim content pack release --dry-run",
        "cli.rs::run_content PackCommand::Release (dry_run)",
        "none",
        ReadOnly,
        "--dry-run prints the manifest and writes nothing",
        &["content", "pack", "release", "--packs", "gate-probe", "--out", "{save}/content", "--dry-run"]
    ),
    writer!(
        CombatSim,
        "combat-sim:content-read",
        "combat-sim content list|show|check",
        "cli.rs::run_content List/Show/Check",
        "none",
        ReadOnly,
        "reads content sources only",
        &["content", "list"]
    ),
    writer!(
        CombatSim,
        "combat-sim:pack-read",
        "combat-sim content pack list|show|verify",
        "cli.rs::run_content PackCommand::List/Show/Verify",
        "none",
        ReadOnly,
        "reads loaded packs only",
        &["content", "pack", "list"]
    ),
    writer!(
        CombatSim,
        "combat-sim:status",
        "combat-sim status",
        "cli.rs::run_cli Command::Status; cli.rs::print_status",
        "none",
        ReadOnly,
        "prints the state file; --player only reads identified.yaml",
        &["status"]
    ),
    writer!(
        CombatSim,
        "combat-sim:scene",
        "combat-sim scene",
        "cli.rs::run_cli Command::Scene; cli.rs::print_scene",
        "none",
        ReadOnly,
        "prints the scene from the state file",
        &["scene"]
    ),
    writer!(
        CombatSim,
        "combat-sim:log",
        "combat-sim log",
        "cli.rs::run_cli Command::Log; cli.rs::print_log",
        "none",
        ReadOnly,
        "prints the state log",
        &["log"]
    ),
    writer!(
        CombatSim,
        "combat-sim:simulate",
        "combat-sim simulate",
        "cli.rs::run_cli Command::Simulate; cli.rs::run_simulate",
        "none",
        ReadOnly,
        "runs encounters in memory and writes no state",
        &["simulate", "--encounter", "gate-probe", "--runs", "1"]
    ),
];

/// Look one registry entry up by id.
pub fn writer(entry: &str) -> Option<&'static Writer> {
    WRITERS.iter().find(|w| w.entry == entry)
}

/// The gate class of one registry entry.
///
/// Panics on an unknown id: ids come from the exhaustive `match` in each CLI,
/// so an unknown id is a programming error, not a runtime condition.
pub fn class_of(entry: &str) -> Class {
    writer(entry)
        .unwrap_or_else(|| panic!("transition gate: unknown writer entry `{entry}`"))
        .class
}

/// Why a save-writing entry was refused.
#[derive(Debug, Clone)]
pub struct Refused {
    /// Registry id of the refused entry.
    pub entry: &'static str,
    /// CLI form of the refused entry.
    pub command: &'static str,
    /// Save directory whose control files triggered the refusal.
    pub save_dir: PathBuf,
    /// Control files that are present.
    pub files: Vec<&'static str>,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let files = if self.files.is_empty() {
            format!("{LOCK_FILE} / {STATE_FILE}")
        } else {
            self.files.join(" / ")
        };
        write!(
            f,
            "存档已启用成长协议（{files}）：{} 尚未接入成长协议，已拒绝执行，未写入任何目标文件。\
请升级到已接入该入口的版本，或等待该入口接入后再执行（ADV-P1-1a–1e）；\
不要把协议文件当成普通文件删改。",
            self.command
        )
    }
}

impl std::error::Error for Refused {}

/// Consult the gate for one entry in a known save directory.
///
/// Returns `Ok(())` when the save has no control file, or when the entry is
/// allowed under the protocol (read-only, control metadata, new save). Returns
/// a [`Refused`] error — exit code 1 at both CLIs — for an unadopted writer.
pub fn check(save_dir: &Path, entry: &str) -> Result<(), Refused> {
    let writer =
        writer(entry).unwrap_or_else(|| panic!("transition gate: unknown writer entry `{entry}`"));
    if writer.class.allowed_under_protocol() || !has_protocol(save_dir) {
        return Ok(());
    }
    Err(Refused {
        entry: writer.entry,
        command: writer.command,
        save_dir: save_dir.to_path_buf(),
        files: protocol_files(save_dir),
    })
}

/// Consult the gate when the save directory may be unknown.
pub fn check_save(save_dir: Option<&Path>, entry: &str) -> Result<(), Refused> {
    match save_dir {
        Some(dir) => check(dir, entry),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "advancement-lock-gate-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_control_file_means_no_protocol() {
        let dir = temp("none");
        assert!(!has_protocol(&dir));
        assert!(protocol_files(&dir).is_empty());
        assert!(check(&dir, "talkdnd:xp-add").is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn either_control_file_marks_the_protocol() {
        let dir = temp("either");
        fs::write(lock_path(&dir), b"").unwrap();
        assert!(has_protocol(&dir));
        assert_eq!(protocol_files(&dir), vec![LOCK_FILE]);
        fs::write(state_path(&dir), b"").unwrap();
        assert_eq!(protocol_files(&dir), vec![LOCK_FILE, STATE_FILE]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn zero_byte_and_corrupt_control_files_still_count() {
        let dir = temp("corrupt");
        fs::write(state_path(&dir), b"not a state record").unwrap();
        assert!(has_protocol(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_control_symlink_still_counts() {
        let dir = temp("dangling");
        std::os::unix::fs::symlink(dir.join("missing"), lock_path(&dir)).unwrap();
        assert!(has_protocol(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unadopted_writer_is_refused_with_the_shared_message() {
        let dir = temp("refuse");
        fs::write(lock_path(&dir), b"").unwrap();
        let err = check(&dir, "talkdnd:xp-add").expect_err("must refuse");
        let text = err.to_string();
        assert!(text.contains("已启用成长协议"), "{text}");
        assert!(text.contains("尚未接入成长协议"), "{text}");
        assert!(text.contains("未写入任何目标文件"), "{text}");
        assert!(text.contains(LOCK_FILE), "{text}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_only_control_metadata_and_new_save_are_allowed() {
        let dir = temp("allowed");
        fs::write(state_path(&dir), b"").unwrap();
        for entry in [
            "talkdnd:xp-show",
            "talkdnd:guard",
            "talkdnd:secret-list",
            "talkdnd:new",
            "combat-sim:status",
            "combat-sim:pack-build-dry-run",
        ] {
            assert!(check(&dir, entry).is_ok(), "{entry} must be allowed");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn enclosing_protocol_dir_walks_up_from_a_state_file() {
        let root = temp("enclosing");
        let save = root.join("Saves/x");
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(lock_path(&save), b"").unwrap();
        assert_eq!(
            enclosing_protocol_dir(&save.join("combat/e.state.json")),
            Some(save.clone())
        );
        assert_eq!(enclosing_protocol_dir(&root), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn every_registry_entry_has_a_unique_id_and_known_class() {
        let mut seen = std::collections::BTreeSet::new();
        for w in WRITERS {
            assert!(seen.insert(w.entry), "duplicate entry id {}", w.entry);
            assert!(!w.command.is_empty(), "{} has no command", w.entry);
            assert!(!w.location.is_empty(), "{} has no location", w.entry);
            assert!(!w.targets.is_empty(), "{} has no targets", w.entry);
            assert!(!w.basis.is_empty(), "{} has no basis", w.entry);
            if w.class == Class::UnadoptedWrite {
                assert!(!w.argv.is_empty(), "{} is a writer with no probe", w.entry);
            }
            if !w.argv.is_empty() {
                assert!(
                    !w.argv.iter().any(|a| a.is_empty()),
                    "{} has an empty argument",
                    w.entry
                );
            }
        }
    }

    #[test]
    fn both_clis_are_covered() {
        assert!(WRITERS.iter().any(|w| w.cli == Cli::Talkdnd));
        assert!(WRITERS.iter().any(|w| w.cli == Cli::CombatSim));
        assert!(WRITERS
            .iter()
            .any(|w| w.cli == Cli::Talkdnd && w.class == Class::UnadoptedWrite));
        assert!(WRITERS
            .iter()
            .any(|w| w.cli == Cli::CombatSim && w.class == Class::UnadoptedWrite));
    }
}
