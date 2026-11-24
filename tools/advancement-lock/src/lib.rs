//! Shared advancement-protocol control files, the cross-CLI transition gate,
//! and the SAVE lock primitive.
//!
//! The advancement protocol (see `docs/ADVANCEMENT-LOCK-DESIGN.md`) owns two
//! fixed control files inside one save slot:
//!
//! * `.talkdnd.advancement.lock` — the stable lock file / initialisation mark;
//! * `.talkdnd.advancement.state` — the only authoritative protocol state.
//!
//! This crate is the single home for those names, for the **transition gate**
//! that `talkdnd` and `combat-sim` share (decision #292, `ADV-P1-1pre`), and
//! for the SAVE lock primitive itself (`ADV-P1-1b`). Neither binary may grow
//! its own copy of the semantics: a save that has either control file has
//! adopted the protocol, and every save-writing entry that has not adopted it
//! must fail closed instead of writing. Adopted writers acquire the SAVE lock
//! from this crate (`open` + `check_ordinary_write`) before writing.
//!
//! The transition gate (`has_protocol`, `check_save`, the writer registry)
//! still only answers presence and classifies entries; it does not acquire the
//! lock. The D commit flow and auto-init are not implemented here. Ordinary
//! writers must not call [`check_writable`], which requires directory sync and
//! would refuse every ordinary write on Windows.
//!
//! Fail-closed rules for the gate:
//!
//! * presence is decided with `symlink_metadata`, so a dangling or otherwise
//!   unreadable control file still counts as present;
//! * any metadata error other than "not found" counts as present;
//! * nothing in the gate creates a control file. A save without one keeps its
//!   existing behaviour.

use std::fmt;
use std::fs;
use std::io::ErrorKind as IoErrorKind;
use std::path::{Path, PathBuf};

/// Fixed lock file name inside one save slot.
pub const LOCK_FILE: &str = ".talkdnd.advancement.lock";
/// Fixed state file name inside one save slot.
pub const STATE_FILE: &str = ".talkdnd.advancement.state";

/// One line every affected command shows in `--help`, and the wording the
/// player-facing documentation repeats.
pub const REFUSAL_HELP: &str =
    "存档启用成长协议（.talkdnd.advancement.lock / .talkdnd.advancement.state）后，尚未接入成长协议的写命令以退出码 1 拒绝执行且不写任何目标文件。已接入的 talkdnd 与 combat-sim 写命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时同样以退出码 1 拒绝且不写任何目标文件（接入期对缺失 state 的保守处理，不自动初始化）。`combat-sim` 的 content import / pack build|release 仅在 `--out` 指向已启用协议的存档内时取 SAVE 锁；`--out` 在存档外时按存档外写入允许；`pack build|release --dry-run` 只读。只读命令与 `guard`/`secret list` 的缓存写入仍可用，不取 SAVE 锁。`start` 的用户数据/迁移写入取 SAVE 锁；其 digest/缓存仍是控制元数据，不单独取 SAVE 锁，但锁不可用时整条命令拒绝，也不写 digest/缓存。没有协议文件的存档保持既有行为。";

/// `--help` wording for a group-A command that only writes (no read-only form).
pub const ADOPTED_WRITE_HELP: &str =
    "存档启用成长协议后，本命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写任何目标文件。没有协议文件的存档保持既有行为，不自动初始化。";

/// `--help` wording for a group-A family that has both write and read-only forms.
pub const ADOPTED_WRITE_MIXED_HELP: &str =
    "存档启用成长协议后，本命令的写操作在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写目标文件；只读用法仍可用。没有协议文件的存档保持既有行为，不自动初始化。";

/// `--help` wording for a content command whose `--out` may land inside a save.
pub const ADOPTED_OUT_HELP: &str =
    "存档启用成长协议后，当 --out 指向已启用协议的存档内时，本命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写任何目标文件。--out 在任何存档之外时按存档外写入允许。没有协议文件的存档保持既有行为，不自动初始化。";

/// `--help` wording for `content pack build|release`, which also have `--dry-run`.
pub const ADOPTED_OUT_MIXED_HELP: &str =
    "存档启用成长协议后，当 --out 指向已启用协议的存档内时，本命令的写操作在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写目标文件；--dry-run 只读仍可用；--out 在任何存档之外时按存档外写入允许。没有协议文件的存档保持既有行为，不自动初始化。";

/// The lock file path inside `save_dir`.
pub fn lock_path(save_dir: &Path) -> PathBuf {
    save_dir.join(LOCK_FILE)
}

/// The state file path inside `save_dir`.
pub fn state_path(save_dir: &Path) -> PathBuf {
    save_dir.join(STATE_FILE)
}

/// Marker file at the top of a play folder (shipped in the release).
pub const ROOT_MARKER: &str = ".talkdnd-root";

/// A play folder: has the marker, or the classic `AGENTS.md` + `Saves/` pair.
///
/// Both CLIs use this so `combat-sim` can call [`open`] without depending on
/// `talkdnd`.
pub fn is_play_root(path: &Path) -> bool {
    path.join(ROOT_MARKER).is_file()
        || (path.join("AGENTS.md").is_file() && path.join("Saves").is_dir())
}

/// The play folder that contains `save_dir`, if one can be found by walking
/// ancestors. The returned path is the ancestor as it appears in `save_dir`
/// (absolute, not canonicalized), so [`open`] can derive the recorded
/// `save_path` without introducing `.` / `..`.
pub fn play_root_for_save(save_dir: &Path) -> Option<PathBuf> {
    let absolute = if save_dir.is_absolute() {
        save_dir.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(save_dir)
    };
    for dir in absolute.ancestors() {
        if is_play_root(dir) {
            return Some(dir.to_path_buf());
        }
    }
    None
}

/// Make `path` absolute by prefixing the current directory. Does not resolve
/// `.` / `..` or follow symlinks: the SAVE lock refuses those spellings.
pub fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn present(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(err) if err.kind() == IoErrorKind::NotFound => false,
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

/// A write target whose absolute location could not be established reliably.
///
/// The gate fails closed on these instead of guessing: a target it cannot
/// resolve could belong to a save that has adopted the protocol.
#[derive(Debug, Clone)]
pub struct Unresolved {
    /// The target as the caller passed it.
    pub target: PathBuf,
    /// Why it could not be resolved.
    pub reason: String,
}

impl fmt::Display for Unresolved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "写目标 {} 无法可靠解析为绝对路径（{}）：已拒绝执行，未写入任何目标文件。\
请改用绝对路径，并确认路径上的目录存在且可访问。",
            self.target.display(),
            self.reason
        )
    }
}

impl std::error::Error for Unresolved {}

/// A gate outcome: either a protocol refusal or an unresolvable target.
///
/// Both mean "do not write"; both reach the CLI as exit code 1.
#[derive(Debug, Clone)]
pub enum GateError {
    /// A save that has adopted the protocol owns the target.
    Refused(Refused),
    /// The target's absolute location could not be established.
    Unresolved(Unresolved),
}

impl From<Unresolved> for GateError {
    fn from(value: Unresolved) -> Self {
        GateError::Unresolved(value)
    }
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateError::Refused(err) => err.fmt(f),
            GateError::Unresolved(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for GateError {}

/// Absolute, symlink-resolved form of a write target's **directory entry**.
///
/// Relative targets are anchored at the process working directory, which is
/// exactly how both CLIs open them. The parent is canonicalized (so symlinks
/// and `..` are resolved the way the operating system will resolve them); the
/// final component is appended literally, because an atomic rename replaces
/// that directory entry and does not follow a final symlink. Anything that
/// cannot be resolved — an unreadable ancestor, a symlink loop, no final
/// component, no working directory — is an [`Unresolved`] error, and the gate
/// refuses the write.
pub fn resolve_target(target: &Path) -> Result<PathBuf, Unresolved> {
    let anchored = anchor(target)?;
    let Some(name) = anchored.file_name() else {
        return Err(Unresolved {
            target: target.to_path_buf(),
            reason: "the path has no final component".to_string(),
        });
    };
    let parent = anchored.parent().unwrap_or(anchored.as_path());
    Ok(canonical_existing(parent, target)?.join(name))
}

/// The two ways a write can land, for one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteTargets {
    /// The directory entry that an atomic rename replaces (the final component
    /// is not followed).
    pub entry: PathBuf,
    /// The path an append or copy follows when the final component already
    /// exists as a symlink. Equal to `entry` when it does not.
    pub followed: PathBuf,
}

/// Resolve both the directory-entry path and the followed path of a target.
///
/// An atomic rename replaces the entry, while an append or copy follows a
/// final symlink; a save that adopted the protocol is reached through either
/// one, so both must be checked.
pub fn resolve_write_targets(target: &Path) -> Result<WriteTargets, Unresolved> {
    let entry = resolve_target(target)?;
    let followed = match fs::symlink_metadata(&entry) {
        Ok(meta) if meta.file_type().is_symlink() => follow_symlink_chain(&entry, target)?,
        Ok(_) => entry.clone(),
        Err(err) if err.kind() == IoErrorKind::NotFound => entry.clone(),
        Err(err) => {
            return Err(Unresolved {
                target: target.to_path_buf(),
                reason: format!("cannot inspect {}: {err}", entry.display()),
            })
        }
    };
    Ok(WriteTargets { entry, followed })
}

/// Follow a final symlink — and any further symlink hops — to the path an
/// append or copy would land on.
///
/// `start` is the directory-entry path of a symlink. Each dangling hop is
/// resolved the same way [`resolve_target`] would resolve it, so a missing
/// landing file still names a path; a hop that is itself a symlink is
/// followed. Loops and unreadable hops fail closed.
fn follow_symlink_chain(start: &Path, original: &Path) -> Result<PathBuf, Unresolved> {
    let mut current = start.to_path_buf();
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(current.clone()) {
            return Err(Unresolved {
                target: original.to_path_buf(),
                reason: format!("symlink loop at {}", current.display()),
            });
        }
        if seen.len() > 256 {
            return Err(Unresolved {
                target: original.to_path_buf(),
                reason: format!("too many symlink hops at {}", current.display()),
            });
        }
        let link = fs::read_link(&current).map_err(|err| Unresolved {
            target: original.to_path_buf(),
            reason: format!("cannot read link {}: {err}", current.display()),
        })?;
        let destination = if link.is_absolute() {
            link
        } else {
            current.parent().unwrap_or_else(|| Path::new("")).join(link)
        };
        match fs::canonicalize(&destination) {
            Ok(path) => return Ok(path),
            Err(err) if err.kind() == IoErrorKind::NotFound => {
                let next = resolve_target(&destination)?;
                match fs::symlink_metadata(&next) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        current = next;
                    }
                    Ok(_) => {
                        return fs::canonicalize(&next).map_err(|err| Unresolved {
                            target: original.to_path_buf(),
                            reason: format!("cannot resolve {}: {err}", next.display()),
                        });
                    }
                    Err(err) if err.kind() == IoErrorKind::NotFound => return Ok(next),
                    Err(err) => {
                        return Err(Unresolved {
                            target: original.to_path_buf(),
                            reason: format!("cannot inspect {}: {err}", next.display()),
                        });
                    }
                }
            }
            Err(err) => {
                return Err(Unresolved {
                    target: original.to_path_buf(),
                    reason: format!("cannot resolve {}: {err}", destination.display()),
                });
            }
        }
    }
}

fn anchor(target: &Path) -> Result<PathBuf, Unresolved> {
    if target.is_absolute() {
        return Ok(target.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|err| Unresolved {
        target: target.to_path_buf(),
        reason: format!("cannot read the current directory: {err}"),
    })?;
    Ok(cwd.join(target))
}

/// Canonicalize the deepest existing ancestor of `path` and append the rest.
fn canonical_existing(path: &Path, original: &Path) -> Result<PathBuf, Unresolved> {
    let mut probe = path;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match fs::symlink_metadata(probe) {
            Ok(_) => break,
            Err(err) if err.kind() == IoErrorKind::NotFound => {
                let Some(name) = probe.file_name() else {
                    return Err(Unresolved {
                        target: original.to_path_buf(),
                        reason: format!("no existing ancestor for {}", probe.display()),
                    });
                };
                tail.push(name.to_os_string());
                let Some(parent) = probe.parent() else {
                    return Err(Unresolved {
                        target: original.to_path_buf(),
                        reason: format!("no existing ancestor for {}", probe.display()),
                    });
                };
                probe = parent;
            }
            Err(err) => {
                return Err(Unresolved {
                    target: original.to_path_buf(),
                    reason: format!("cannot inspect {}: {err}", probe.display()),
                })
            }
        }
    }
    let mut resolved = fs::canonicalize(probe).map_err(|err| Unresolved {
        target: original.to_path_buf(),
        reason: format!("cannot resolve {}: {err}", probe.display()),
    })?;
    for name in tail.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

/// Walk up from a resolved path and return the nearest directory holding a
/// control file.
fn nearest_protocol(path: &Path) -> Option<PathBuf> {
    let mut dir = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(current) = dir {
        if has_protocol(current) {
            return Some(current.to_path_buf());
        }
        dir = current.parent();
    }
    None
}

/// Walk up from a write target and return the nearest directory holding a
/// control file.
///
/// Both the directory entry and the followed path are checked: a final symlink
/// inside a save is reached by an atomic rename even when it points outside,
/// and a symlink outside pointing into a save is reached by an append or copy.
/// This is how `combat-sim` finds the save for `--state`, `--out`, the
/// campaign's `state_dir`, a loaded state's `party_state` and `rolls.log`, and
/// how `talkdnd` checks a `--book` folder that may live in another save.
pub fn enclosing_protocol_dir(target: &Path) -> Result<Option<PathBuf>, Unresolved> {
    let targets = resolve_write_targets(target)?;
    for candidate in [&targets.entry, &targets.followed] {
        if let Some(dir) = nearest_protocol(candidate) {
            return Ok(Some(dir));
        }
    }
    Ok(None)
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
    /// Writes save content through an entry that has adopted the SAVE lock.
    /// The transition gate allows it; the owning CLI must acquire the SAVE
    /// lock ([`open`] + [`AdvancementLock::check_ordinary_write`]) before
    /// writing and fail closed if the lock is unavailable.
    AdoptedWrite,
    /// Writes save content through an entry that has not adopted the protocol.
    /// Refused (exit code 1) whenever the save has a control file.
    UnadoptedWrite,
}

impl Class {
    /// True when the transition gate itself will not refuse the entry.
    ///
    /// [`Class::AdoptedWrite`] is allowed here; the owning CLI still has to
    /// acquire the SAVE lock before writing.
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
        AdoptedWrite,
        "adopted (SAVE lock): user-data and migration writes take the SAVE lock before any target is written; digest.md/.cache are control metadata and do not take SAVE themselves, but are not written when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["level-up", "aria"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:xp-add",
        "talkdnd xp add",
        "lib.rs::dispatch Cmd::Xp; progress.rs::xp_add",
        "progress.yaml, Sessions/*.md",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["scene", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:char",
        "talkdnd char <who> <op> (every op except the plain spells view)",
        "lib.rs::dispatch Cmd::Char; charsheet.rs::run; charsheet.rs::commit",
        "sheets/<id>.yaml, combat/party-state.yaml, Characters/*.md, Sessions/*.md",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["chargen", "array"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:chargen-sheet",
        "talkdnd chargen sheet",
        "lib.rs::chargen_cmd; charsheet.rs::from_chargen; lib.rs::add_to_party",
        "Characters/<id>.md, combat/campaign.yaml, sheets/<id>.yaml",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["chargen", "sheet"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:chargen-confirm",
        "talkdnd chargen confirm",
        "lib.rs::chargen_cmd; progress.rs::loot; chargen.rs::store",
        "progress.yaml, Sessions/*.md, chargen.json",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["companion", "random", "--count", "1"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:companion-sheet",
        "talkdnd companion sheet / talkdnd npc sheet",
        "lib.rs::npc_cmd NpcCmd::Sheet; lib.rs::add_to_party",
        "Characters/<id>.md, combat/campaign.yaml",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["settings", "dice", "on"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:settings-style-write",
        "talkdnd settings style set",
        "lib.rs::dispatch Cmd::Settings; memory.rs::run_style; migrate.rs::write_meta",
        "save-meta.yaml, Sessions/*.md",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["secret", "add", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:secret-import-apply",
        "talkdnd secret import --apply",
        "lib.rs::dispatch Cmd::Secret; secret.rs::import_lines",
        "secret-terms.yaml, secret-terms.cache.json",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["identify", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:reveal-add",
        "talkdnd reveal add",
        "lib.rs::dispatch Cmd::Reveal; reveal.rs::add; reveal.rs::add_original",
        "reveals.yaml, Sessions/*.md",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["original", "add", "--type", "npc", "--name", "Probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:original-plan",
        "talkdnd original plan",
        "lib.rs::dispatch Cmd::Original; original.rs::plan",
        "dm-plan.md",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        "handouts/<id>.<ext>, handouts.yaml, reveals.yaml, Sessions/*.md, book-dir handouts.yaml (built when missing)",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        "talkdnd handout list (player view)",
        "lib.rs::dispatch Cmd::Handout; handout.rs::list (!dm)",
        "none",
        ReadOnly,
        "the player view reads the save's shown list and never touches a book registry",
        &["handout", "list"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:handout-list-dm",
        "talkdnd handout list --dm",
        "lib.rs::dispatch Cmd::Handout; handout.rs::list; handout.rs::registries; handout.rs::build; handout.rs::store",
        "book-dir handouts.yaml",
        UnadoptedWrite,
        "--dm lists the whole registry and builds/writes handouts.yaml when it is missing",
        &["handout", "list", "--dm"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:adventure-section-write",
        "talkdnd adventure section ... --reason",
        "lib.rs::dispatch Cmd::Adventure; adventure.rs::section",
        "Sessions/*.md",
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
        &["adventure", "section", "gate-probe", "--reason", "gate-probe"]
    ),
    writer!(
        Talkdnd,
        "talkdnd:adventure-section-read",
        "talkdnd adventure section (no --reason) / --list",
        "lib.rs::dispatch Cmd::Adventure; adventure.rs::section",
        "none",
        ReadOnly,
        "a normal read writes no log; --list prints headings and returns before the log write, even with --reason",
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
        AdoptedWrite,
        "adopted (SAVE lock): takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable",
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
        AdoptedWrite,
        "adopted (SAVE lock): state.save, party.save and log append run while the SAVE lock is held; a failed step reports which stage was written",
        &["init"]
    ),
    writer!(
        CombatSim,
        "combat-sim:new",
        "combat-sim new",
        "cli.rs::run_cli Command::New; cli.rs::new_state",
        "combat/*.state.json, combat/party-state.yaml",
        AdoptedWrite,
        "adopted (SAVE lock): creates or overwrites a state file (and associates party when the path is in the manifest state_dir) after taking the SAVE lock",
        &["new", "--encounter", "gate-probe"]
    ),
    writer!(
        CombatSim,
        "combat-sim:quick",
        "combat-sim quick",
        "cli.rs::run_cli Command::Quick; cli.rs::quick_state",
        "combat/*.state.json, combat/party-state.yaml",
        AdoptedWrite,
        "adopted (SAVE lock): creates the ad-hoc encounter state after taking the SAVE lock",
        &["quick", "--foe", "gate-probe"]
    ),
    writer!(
        CombatSim,
        "combat-sim:roll",
        "combat-sim roll",
        "cli.rs::run_cli Command::Roll; player.rs::append_roll",
        "combat/*.state.json, combat/party-state.yaml, rolls.log, Sessions/*.md",
        AdoptedWrite,
        "adopted (SAVE lock) by actual write targets: without --state only appends rolls.log; with --state pointing at an existing file also saves state/party and the session log",
        &["roll", "1d20"]
    ),
    writer!(
        CombatSim,
        "combat-sim:party-write",
        "combat-sim party rest|set-hp|sync",
        "cli.rs::run_party",
        "combat/party-state.yaml",
        AdoptedWrite,
        "adopted (SAVE lock): rewrites party-state.yaml after taking the SAVE lock",
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
        AdoptedWrite,
        "adopted (SAVE lock) when --out is inside a protocol-enabled save; out-of-save --out is allowed; no dry-run exists",
        &["content", "import-srd", "--out", "{save}/content"]
    ),
    writer!(
        CombatSim,
        "combat-sim:content-import-srd-prose",
        "combat-sim content import-srd-prose",
        "cli.rs::run_content ContentCommand::ImportSrdProse; srd_prose::import",
        "--out content files (inside the save when --out points there)",
        AdoptedWrite,
        "adopted (SAVE lock) when --out is inside a protocol-enabled save; out-of-save --out is allowed; no dry-run exists",
        &["content", "import-srd-prose", "--edition", "2024", "--index", "gate-probe", "--out", "{save}/content"]
    ),
    writer!(
        CombatSim,
        "combat-sim:content-import-srd-pack",
        "combat-sim content import-srd-pack",
        "cli.rs::run_content ContentCommand::ImportSrdPack; srd_pack::import",
        "--out content files (inside the save when --out points there)",
        AdoptedWrite,
        "adopted (SAVE lock) when --out is inside a protocol-enabled save; out-of-save --out is allowed; no dry-run exists",
        &["content", "import-srd-pack", "--edition", "2024", "--src", "gate-probe", "--out", "{save}/content"]
    ),
    writer!(
        CombatSim,
        "combat-sim:pack-build",
        "combat-sim content pack build (without --dry-run)",
        "cli.rs::run_content PackCommand::Build; content::full_pack::build",
        "--out pack file (inside the save when --out points there)",
        AdoptedWrite,
        "adopted (SAVE lock) when --out is inside a protocol-enabled save; out-of-save --out is allowed; only --dry-run is read-only",
        &["content", "pack", "build", "--edition", "2024", "--book", "phb", "--index", "gate-probe", "--out", "{save}/content"]
    ),
    writer!(
        CombatSim,
        "combat-sim:pack-release",
        "combat-sim content pack release (without --dry-run)",
        "cli.rs::run_content PackCommand::Release; content::full_pack::release",
        "--out release bundle (inside the save when --out points there)",
        AdoptedWrite,
        "adopted (SAVE lock) when --out is inside a protocol-enabled save; out-of-save --out is allowed; only --dry-run is read-only",
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

/// Consult the gate for one resolved-or-resolvable write target.
///
/// Returns `Ok(())` when the entry is allowed under the protocol (read-only,
/// control metadata, new save, or adopted SAVE-lock writers) or when no
/// enclosing directory has a control file. Returns [`GateError::Refused`] when a save that adopted the protocol
/// owns the target, and [`GateError::Unresolved`] when the target's absolute
/// location cannot be established. Both reach the CLI as exit code 1 and write
/// nothing.
pub fn check_target(target: &Path, entry: &str) -> Result<(), GateError> {
    let writer =
        writer(entry).unwrap_or_else(|| panic!("transition gate: unknown writer entry `{entry}`"));
    if writer.class.allowed_under_protocol() {
        return Ok(());
    }
    match enclosing_protocol_dir(target)? {
        Some(save_dir) => Err(GateError::Refused(Refused {
            entry: writer.entry,
            command: writer.command,
            save_dir: save_dir.clone(),
            files: protocol_files(&save_dir),
        })),
        None => Ok(()),
    }
}

/// Consult the gate when the save directory may be unknown.
pub fn check_save(save_dir: Option<&Path>, entry: &str) -> Result<(), GateError> {
    match save_dir {
        Some(dir) => check_target(dir, entry),
        None => Ok(()),
    }
}

/// Consult the gate for several write targets; the first refusal or
/// unresolvable target stops the command.
pub fn check_targets<'a, I>(targets: I, entry: &str) -> Result<(), GateError>
where
    I: IntoIterator<Item = &'a Path>,
{
    for target in targets {
        check_target(target, entry)?;
    }
    Ok(())
}

// ----------------------------------------------------------------- SAVE lock ----

/// The v1 SAVE lock primitive: [`init`], [`open`],
/// [`AdvancementLock::check_ordinary_write`] and
/// [`AdvancementLock::check_writable`].
pub mod save_lock;
pub use save_lock::{
    init, open, reseal, AcquireOptions, AdvancementLock, CompletionEvidence, Error, ErrorKind,
    InitHooks, InitOptions, InitOutcome, LockHeader, LockIdentity, OpenOptions, Pending, Published,
    Stage, State, StateRecord, COPY_LIMITATION, DEFAULT_ACQUIRE_TIMEOUT, SCHEMA,
};

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
        assert!(check_save(Some(&dir), "talkdnd:xp-add").is_ok());
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
        let err = check_save(Some(&dir), "talkdnd:log").expect_err("must refuse");
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
            assert!(
                check_save(Some(&dir), entry).is_ok(),
                "{entry} must be allowed"
            );
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
            enclosing_protocol_dir(&save.join("combat/e.state.json")).unwrap(),
            Some(fs::canonicalize(&save).unwrap())
        );
        assert_eq!(enclosing_protocol_dir(&root).unwrap(), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_target_anchors_relative_paths_at_the_working_directory() {
        // `resolve_target` returns the canonical form; compare with the
        // canonical working directory so the test is platform-neutral.
        let cwd = fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        let resolved = resolve_target(Path::new("relative/child.state.json")).unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(resolved, cwd.join("relative").join("child.state.json"));
    }

    #[test]
    fn resolve_target_resolves_dot_dot_and_symlinks() {
        let root = temp("resolve");
        let real = root.join("real");
        fs::create_dir_all(real.join("combat")).unwrap();
        let nested = real.join("combat/../combat/e.state.json");
        assert_eq!(
            resolve_target(&nested).unwrap(),
            fs::canonicalize(real.join("combat"))
                .unwrap()
                .join("e.state.json")
        );
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert_eq!(
                resolve_target(&link.join("combat/e.state.json")).unwrap(),
                fs::canonicalize(real.join("combat"))
                    .unwrap()
                    .join("e.state.json")
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_target_appends_a_missing_tail_under_the_canonical_ancestor() {
        let root = temp("missing-tail");
        let resolved = resolve_target(&root.join("new/content")).unwrap();
        assert_eq!(
            resolved,
            fs::canonicalize(&root).unwrap().join("new/content")
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_final_symlink_inside_a_protected_save_is_refused() {
        // The symlink's directory entry lives in the save, so an atomic rename
        // replaces it even though it points outside.
        let root = temp("final-symlink");
        let save = root.join("Saves/p");
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(lock_path(&save), b"").unwrap();
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let external = outside.join("e.state.json");
        fs::write(&external, b"{}").unwrap();
        let link = save.join("combat/e.state.json");
        std::os::unix::fs::symlink(&external, &link).unwrap();

        let targets = resolve_write_targets(&link).unwrap();
        assert_eq!(
            targets.entry,
            fs::canonicalize(save.join("combat"))
                .unwrap()
                .join("e.state.json")
        );
        assert_eq!(targets.followed, fs::canonicalize(&external).unwrap());
        assert!(check_target(&link, "combat-sim:state-write").is_ok());
        assert!(matches!(
            check_target(&link, "talkdnd:log"),
            Err(GateError::Refused(_))
        ));

        // The reverse: a symlink outside the save that points into it is
        // followed by an append or copy.
        let alias = outside.join("alias-party.yaml");
        std::os::unix::fs::symlink(save.join("combat/party-state.yaml"), &alias).unwrap();
        assert!(check_target(&alias, "combat-sim:party-write").is_ok());
        assert!(matches!(
            check_target(&alias, "talkdnd:log"),
            Err(GateError::Refused(_))
        ));
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_multi_hop_dangling_symlink_into_a_protected_save_is_refused() {
        // plain/rolls.log -> outside/link -> protected/new-file (dangling).
        // canonicalize(outside/link) fails with NotFound, so the gate must
        // keep following the chain instead of stopping at the intermediate
        // entry, which lives outside the protected save.
        let root = temp("multi-hop-dangling");
        let save = root.join("Saves/p");
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(lock_path(&save), b"").unwrap();
        let landing = save.join("new-file");
        assert!(!landing.exists());
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let mid = outside.join("link");
        std::os::unix::fs::symlink(&landing, &mid).unwrap();
        let plain = root.join("plain");
        fs::create_dir_all(&plain).unwrap();
        let start = plain.join("rolls.log");
        std::os::unix::fs::symlink(&mid, &start).unwrap();

        let targets = resolve_write_targets(&start).unwrap();
        assert_eq!(
            targets.followed,
            fs::canonicalize(&save).unwrap().join("new-file")
        );
        assert!(matches!(
            check_target(&start, "talkdnd:log"),
            Err(GateError::Refused(_))
        ));
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_fails_closed() {
        let root = temp("symlink-loop");
        let a = root.join("a.link");
        let b = root.join("b.link");
        std::os::unix::fs::symlink(&b, &a).unwrap();
        std::os::unix::fs::symlink(&a, &b).unwrap();
        let err = check_target(&a, "talkdnd:log").expect_err("must fail closed");
        assert!(matches!(err, GateError::Unresolved(_)), "{err}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_regular_file_outside_the_saves_is_allowed() {
        let root = temp("regular-file");
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let file = outside.join("party-state.yaml");
        fs::write(&file, b"party: []\n").unwrap();
        let targets = resolve_write_targets(&file).unwrap();
        assert_eq!(targets.entry, targets.followed);
        assert!(check_target(&file, "combat-sim:party-write").is_ok());
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_unresolvable_target_fails_closed() {
        // On Unix a `..` after a missing component has no reliable absolute
        // meaning (the kernel cannot resolve it); Windows normalizes it
        // lexically, so the case only exists here.
        let root = temp("unresolvable");
        let target = root.join("missing/../child");
        let err = check_target(&target, "talkdnd:log").expect_err("must fail closed");
        assert!(matches!(err, GateError::Unresolved(_)), "{err}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_relative_target_inside_a_protected_save_is_refused() {
        // The save is reached through a relative path; the lexical walk alone
        // would only inspect the working directory.
        let root = temp("relative-save");
        let save = root.join("Saves/x");
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(state_path(&save), b"").unwrap();
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();
        let relative = Path::new("Saves/x/combat/e.state.json");
        assert!(check_target(relative, "combat-sim:state-write").is_ok());
        assert!(matches!(
            check_target(relative, "talkdnd:log"),
            Err(GateError::Refused(_))
        ));
        std::env::set_current_dir(previous).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_external_state_file_whose_party_state_points_into_a_save_is_refused() {
        // The gate is asked about the party-state target read from a state file
        // that lives outside the save.
        let root = temp("party-state");
        let save = root.join("Saves/x");
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(lock_path(&save), b"").unwrap();
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        assert!(check_target(&outside.join("party-state.yaml"), "combat-sim:party-write").is_ok());
        assert!(check_target(
            &save.join("combat/party-state.yaml"),
            "combat-sim:party-write"
        )
        .is_ok());
        assert!(matches!(
            check_target(&save.join("combat/party-state.yaml"), "talkdnd:log"),
            Err(GateError::Refused(_))
        ));
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
            if matches!(w.class, Class::UnadoptedWrite | Class::AdoptedWrite) {
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
            .any(|w| w.cli == Cli::Talkdnd && w.class == Class::AdoptedWrite));
        assert!(!WRITERS
            .iter()
            .any(|w| w.cli == Cli::CombatSim && w.class == Class::UnadoptedWrite));
        assert!(WRITERS
            .iter()
            .any(|w| w.cli == Cli::CombatSim && w.class == Class::AdoptedWrite));
    }

    #[test]
    fn adopted_writers_pass_the_transition_gate() {
        let dir = temp("adopted-ok");
        fs::write(lock_path(&dir), b"").unwrap();
        assert!(check_save(Some(&dir), "talkdnd:xp-add").is_ok());
        assert!(check_save(Some(&dir), "talkdnd:start").is_ok());
        assert!(check_save(Some(&dir), "talkdnd:char").is_ok());
        assert!(matches!(
            check_save(Some(&dir), "talkdnd:log"),
            Err(GateError::Refused(_))
        ));
        assert!(check_save(Some(&dir), "combat-sim:state-write").is_ok());
        assert!(check_save(Some(&dir), "combat-sim:roll").is_ok());
        assert!(check_save(Some(&dir), "combat-sim:content-import-srd").is_ok());
        assert!(matches!(
            check_save(Some(&dir), "talkdnd:handout-build"),
            Err(GateError::Refused(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }
}
