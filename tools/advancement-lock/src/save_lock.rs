//! Cooperative advancement lock for one save slot — design v1.
//!
//! This module implements §2 of `docs/ADVANCEMENT-LOCK-DESIGN.md`: two fixed
//! files in `Saves/<slot>/`, one explicit initializer, one atomic state file,
//! one writable predicate and bounded lock acquisition. It lives in the shared
//! `advancement-lock` crate so `talkdnd` and `combat-sim` take the same SAVE
//! lock (decision #292, `ADV-P1-1b`). Ordinary writers use
//! [`AdvancementLock::check_ordinary_write`]; a D commit must still use
//! [`AdvancementLock::check_writable`]. This module does not implement the D
//! stage machine, does not change the save schema, and does not auto-init.
//!
//! ## The two fixed files
//!
//! * `.talkdnd.advancement.lock` — created **only** by [`init`] with
//!   `create_new`, locked through the same handle it was created with, given
//!   exactly one immutable header, and then never removed, renamed or
//!   rewritten. Its existence means "the protocol is initialized", never
//!   "the state is clean".
//! * `.talkdnd.advancement.state` — the only authority for whether a normal
//!   write may start. It is never truncated in place: every update writes a
//!   unique same-directory temporary file, `sync_all`s it, re-reads it, links
//!   it into place without overwriting and syncs the parent directory. A
//!   missing, zero-byte or checksum-invalid state is UNKNOWN and fails closed;
//!   it is never treated as a fresh initialization and never rebuilt.
//!
//! ## Save identity comes from the caller's root, not from a string
//!
//! [`init`] and [`open`] take the trusted repository `root` and the actual
//! `save_dir`; the library derives the recorded `save_path` itself (normalized
//! POSIX, relative to `root`) and refuses a save outside the root, a symlinked
//! component, `..`, `.` or an ambiguous textual form. A caller cannot claim a
//! path the save does not have.
//!
//! ## The writable predicate
//!
//! [`AdvancementLock::check_writable`] is the only "may start a normal write"
//! answer, and it re-reads the on-disk state first. It accepts `ready`,
//! `complete` with no pending intent and every publish finished, and `failed`
//! that provably stopped before any target write. `active` and `in_doubt`
//! always refuse. [`AdvancementLock::finish_complete`] refuses unless the
//! caller states that the candidate comparison, the log append and the
//! integrity update all completed and no intent is pending;
//! [`AdvancementLock::finish_failed`] refuses once anything was published or
//! intended. Partial or unclear work is only ever `in_doubt`.
//!
//! Every mutation re-reads the state file and validates it (existence, parse,
//! checksum, schema, `save_id`, derived `save_path`, lock identity) and
//! requires it to equal the revision this holder read. A deleted, zero-byte,
//! corrupt or replaced state fails closed before the temporary file is staged:
//! nothing is renamed, nothing is rebuilt. The read and the later link are two
//! syscalls, so a non-cooperating writer in between is still outside what this
//! protocol can promise.
//!
//! ## Bounded acquisition
//!
//! Acquisition is `try_lock` in a bounded loop ([`AcquireOptions`], five
//! seconds by default). A timeout fails closed: the waiter never deletes the
//! lock, never rewrites the header and never takes over. Only the platform's
//! "already locked" answer counts as contention; any other error fails closed.
//! [`init`] additionally proves that a second handle on the same file cannot
//! take the lock, so a filesystem whose lock API does not enforce exclusion is
//! refused. That self-check proves **this client's** lock API only: it does not
//! identify a network share, FUSE mount or sync folder, and this library has no
//! admission gate that could refuse one.
//!
//! ## Copying and moving a save
//!
//! v1 does **not** support advancement commits for a copied or moved save. The
//! state records the derived `save_path` and the identity of the lock file. A
//! save whose path changed is refused with [`ErrorKind::SavePathMismatch`]; a
//! save copied with the same relative path is normally refused with
//! [`ErrorKind::IdentityMismatch`]. A whole-disk image that preserves both the
//! relative path and the file identity **may not be detectable**, and nothing
//! here claims otherwise: a human must confirm. There is no `adopt`, no
//! rebinding and no automatic repair.
//!
//! ## Not promised
//!
//! Multi-file atomicity, elimination of TOCTOU, cooperation from old binaries,
//! external editors or sync clients, network-filesystem semantics, durable
//! directory entries on platforms without directory `fsync` (Windows), replay
//! detection for a self-consistent older state without an external anchor,
//! `resume`/`abort`, the D stage machine and the recovery work record. A
//! `PASS` in a test only proves the synthetic fixture, not that a real save is
//! safe.
//!
//! ## Platform notes
//!
//! * Unix (Linux, macOS): the lock file identity is `dev` + `ino`, so a
//!   replaced path is detected exactly. Directory `fsync` is attempted and its
//!   error is reported.
//! * Windows: the stable standard library exposes no file index, so the
//!   identity is a best-effort creation time, read from a handle on both sides
//!   of the comparison. Size and write time cannot be part of it: writing the
//!   immutable header changes them. A replacement or copy that also preserves
//!   the creation time may go unnoticed, so a same-relative-path copy is not
//!   claimed to be detectable there.
//! * Windows also has no directory `fsync`. [`init`] still publishes the state
//!   (a hard link cannot be undone), but it returns
//!   [`ErrorKind::StatePublishedNotDurable`] instead of `Ok`: the state may
//!   already be published, its durability is unknown, and it must not be
//!   repaired automatically. [`AdvancementLock::check_writable`] then refuses
//!   every commit on such a platform, so local advancement commits are **not
//!   supported** on Windows in v1. That capability is not a writable proof.
//! * Network shares, FUSE and sync folders are unsupported and unverified.
//!   Nothing in this build detects them.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The only protocol schema this build understands. Anything else fails closed.
pub const SCHEMA: u32 = 1;
/// Default bound for acquiring the lock.
pub const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
/// Shared wording for the copy/move limitation (also used in error messages).
pub const COPY_LIMITATION: &str = "v1 does not support advancement commits for a copied or moved save; \
restore the original path or wait for a later task. A whole-disk image that keeps the same relative \
path and file identity may not be detectable, so a human must confirm.";

const RETRY_INTERVAL: Duration = Duration::from_millis(20);

// ------------------------------------------------------------------ error ----

/// Why an operation was refused. Every variant is a fail-closed outcome; none
/// of them means "probably fine".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// `init` found the lock or the state (even zero-byte) already present.
    AlreadyInitialized,
    /// A normal operation found no lock file. Normal operation never creates it.
    NotInitialized,
    /// Another holder kept the lock for the whole deadline.
    LockTimeout,
    /// The platform or filesystem did not enforce an exclusive lock.
    LockUnsupported,
    /// The path no longer names the file this handle locked.
    PathReplaced,
    /// `save_id` or `lock_identity` does not match the current files.
    IdentityMismatch,
    /// `save_path` differs: a copied or moved save.
    SavePathMismatch,
    /// Missing, zero-byte, unparseable or checksum-invalid protocol data.
    CorruptState,
    /// The state exists but the writable predicate refuses it.
    UnwritableState,
    /// The state was published but its directory entry could not be synced, so
    /// its durability is unknown. Never repair this automatically.
    StatePublishedNotDurable,
    /// The pre-operation snapshot could not be created, synced or verified.
    EvidenceFailed,
    /// The caller asked for something the protocol does not allow.
    InvalidRequest,
    /// Any other I/O or encoding failure.
    Io,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::AlreadyInitialized => "already_initialized",
            ErrorKind::NotInitialized => "not_initialized",
            ErrorKind::LockTimeout => "lock_timeout",
            ErrorKind::LockUnsupported => "lock_unsupported",
            ErrorKind::PathReplaced => "path_replaced",
            ErrorKind::IdentityMismatch => "identity_mismatch",
            ErrorKind::SavePathMismatch => "save_path_mismatch",
            ErrorKind::CorruptState => "corrupt_state",
            ErrorKind::UnwritableState => "unwritable_state",
            ErrorKind::StatePublishedNotDurable => "state_published_not_durable",
            ErrorKind::EvidenceFailed => "evidence_failed",
            ErrorKind::InvalidRequest => "invalid_request",
            ErrorKind::Io => "io",
        }
    }
}

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.detail)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn err(kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error {
        kind,
        detail: detail.into(),
    }
}

fn ctx(kind: ErrorKind, what: impl std::fmt::Display, source: impl std::fmt::Display) -> Error {
    err(kind, format!("{what}: {source}"))
}

// ------------------------------------------------------------------ types ----

/// The five v1 states. The name never replaces the predicate conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Ready,
    Active,
    Complete,
    Failed,
    InDoubt,
}

/// Where an operation stopped. `StoppedBeforeWrite` is what makes `failed`
/// distinguishable from `in_doubt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Init,
    Admit,
    Backup,
    Candidate,
    Publish,
    Logs,
    Done,
    StoppedBeforeWrite,
}

/// The immutable lock header. Written once, by `init`, through the locked
/// handle; never rewritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockHeader {
    pub schema: u32,
    pub save_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Published {
    pub path: String,
    pub before: String,
    pub after: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub path: String,
    pub before: String,
    pub after: String,
}

/// The single authoritative state document.
///
/// Every field is required and unknown fields are rejected, so a missing or
/// future field fails closed instead of being silently defaulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateRecord {
    pub schema: u32,
    pub revision: u64,
    pub checksum: String,
    pub save_id: String,
    pub operation_id: String,
    pub state: State,
    pub stage: Stage,
    pub lock_identity: String,
    pub save_path: String,
    pub published: Vec<Published>,
    pub pending: Vec<Pending>,
    pub backup_dir: Option<String>,
    pub candidate_dir: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A lock file's identity as the current platform can express it.
///
/// The encoded form is what the state stores, so a state written on one
/// platform never matches a lock file on another (a cross-platform copy is a
/// copy and must be refused).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockIdentity(String);

impl LockIdentity {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse and validate the encoded form. An unrecognized shape is rejected
    /// instead of being compared loosely.
    pub fn parse(raw: &str) -> Option<LockIdentity> {
        let mut parts = raw.split(':');
        let tag = parts.next()?;
        let expected = match tag {
            "unix" => 3,
            "windows" => 2,
            "generic" => 3,
            _ => return None,
        };
        let mut count = 1;
        for part in parts {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            count += 1;
        }
        if count != expected {
            return None;
        }
        Some(LockIdentity(raw.to_string()))
    }

    fn of_file(file: &File) -> Result<LockIdentity> {
        let meta = file
            .metadata()
            .map_err(|e| ctx(ErrorKind::Io, "stat the advancement lock handle", e))?;
        identity_of(&meta)
    }

    fn of_path(path: &Path) -> Result<LockIdentity> {
        // Derive the identity from a handle so both sides of every comparison
        // use the same platform query. A path-only query can report a different
        // value than the handle on Windows.
        let file = File::open(path).map_err(|e| {
            ctx(
                ErrorKind::PathReplaced,
                format!("open {}", path.display()),
                e,
            )
        })?;
        LockIdentity::of_file(&file)
    }
}

#[cfg(unix)]
fn identity_of(meta: &fs::Metadata) -> Result<LockIdentity> {
    use std::os::unix::fs::MetadataExt;
    Ok(LockIdentity(format!("unix:{}:{}", meta.dev(), meta.ino())))
}

#[cfg(windows)]
fn identity_of(meta: &fs::Metadata) -> Result<LockIdentity> {
    use std::os::windows::fs::MetadataExt;
    // The standard library still has no stable file index on Windows, so this
    // is a best-effort identity: the creation time only. Size and write time
    // change when the immutable header is written, so they cannot identify a
    // file. A replacement or copy that also preserves the creation time may go
    // unnoticed; the module and design documents say so.
    Ok(LockIdentity(format!("windows:{}", meta.creation_time())))
}

#[cfg(not(any(unix, windows)))]
fn identity_of(_meta: &fs::Metadata) -> Result<LockIdentity> {
    Err(err(
        ErrorKind::LockUnsupported,
        "this platform has no lock-file identity this build can verify",
    ))
}

/// Bound and retry interval for taking the lock. Tests inject short values.
#[derive(Debug, Clone, Copy)]
pub struct AcquireOptions {
    pub timeout: Duration,
    pub retry: Duration,
}

impl Default for AcquireOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_ACQUIRE_TIMEOUT,
            retry: RETRY_INTERVAL,
        }
    }
}

/// Synthetic-test injection points for [`init`]. Production callers leave both
/// `None`; the callbacks only run between the identity rechecks that §2.3
/// already requires, so they cannot skip one.
#[derive(Default)]
pub struct InitHooks<'a> {
    pub after_create: Option<&'a dyn Fn(&Path)>,
    pub before_publish: Option<&'a dyn Fn(&Path)>,
}

pub struct InitOptions<'a> {
    pub acquire: AcquireOptions,
    pub hooks: InitHooks<'a>,
}

impl<'a> InitOptions<'a> {
    pub fn new() -> Self {
        Self {
            acquire: AcquireOptions::default(),
            hooks: InitHooks::default(),
        }
    }
}

impl Default for InitOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct OpenOptions {
    pub acquire: AcquireOptions,
}

impl OpenOptions {
    pub fn new() -> Self {
        Self {
            acquire: AcquireOptions::default(),
        }
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct InitOutcome {
    pub header: LockHeader,
    pub state: StateRecord,
}

/// What the caller must have verified before `complete` may be written.
#[derive(Debug, Clone, Copy, Default)]
pub struct CompletionEvidence {
    pub candidate_matches: bool,
    pub logs_recorded: bool,
    pub integrity_recorded: bool,
}

// -------------------------------------------------------------- utilities ----

pub use crate::{lock_path, state_path, LOCK_FILE, STATE_FILE};

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn counter() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// A random-looking but not secret identifier. It only has to be unique for
/// one save; it is not a capability and it authenticates nothing.
fn new_save_id(save_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"talkdnd-advancement-save-id-v1");
    hasher.update(save_path.as_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(nanos().to_le_bytes());
    hasher.update(counter().to_le_bytes());
    format!("{:x}", hasher.finalize())
}

/// Absolute-path spelling used to find the prefix. Windows spellings are
/// recognized from the encoded bytes so a Unix test host can exercise them:
/// there `Path::is_absolute` is false for `\\?\C:\...` and `Component::Prefix`
/// is never produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AbsoluteSyntax {
    Unix,
    Windows,
}

const fn host_absolute_syntax() -> AbsoluteSyntax {
    if cfg!(windows) {
        AbsoluteSyntax::Windows
    } else {
        AbsoluteSyntax::Unix
    }
}

/// Leading bytes that are the platform prefix plus one physical root separator.
///
/// `rooted_by_separator` is false only for a Windows prefix that is itself the
/// root (`\\server\share`, `\\.\COM1`): nothing after the prefix, and no extra
/// separator byte. An empty tail is then the root, not a trailing separator.
struct AbsoluteHead {
    len: usize,
    rooted_by_separator: bool,
}

fn is_windows_sep(byte: u8) -> bool {
    byte == b'/' || byte == b'\\'
}

fn tail_is_separator(byte: u8, syntax: AbsoluteSyntax) -> bool {
    match syntax {
        AbsoluteSyntax::Unix => byte == b'/',
        // Verbatim tails keep both separators. The previous scan did too;
        // only the prefix is special.
        AbsoluteSyntax::Windows => is_windows_sep(byte),
    }
}

/// Reject a path that is not already in one normalized absolute form, and
/// reject `..` outright instead of resolving it.
///
/// The decision is made on the platform-encoded bytes
/// ([`std::ffi::OsStr::as_encoded_bytes`]), never on a lossy UTF-8 rendering: a
/// name that is not valid UTF-8 must not be able to masquerade as the
/// replacement-character spelling of another name.
///
/// A Windows absolute path starts with a prefix (`C:`, `\\server\share`,
/// `\\?\C:`, `\\?\UNC\server\share`, `\\.\device`) and, when the separator is
/// physically present, one root separator. Those leading bytes are skipped
/// before the empty / `.` / `..` scan. `fs::canonicalize` on Windows returns
/// `\\?\C:\...`; splitting that whole string treats the second backslash of
/// `\\?\` as a repeated separator. [`std::path::Path::components`] is not used
/// for the scan: it drops repeated and trailing separators, which are the
/// ambiguous spellings this check refuses. The skip length is the same number
/// of encoded bytes a `Component::Prefix` plus a physical `Component::RootDir`
/// consume.
fn check_absolute(path: &Path, what: &str) -> Result<()> {
    if !path.is_absolute() {
        return Err(err(
            ErrorKind::InvalidRequest,
            format!("{what} {} must be an absolute path", path.display()),
        ));
    }
    check_absolute_bytes(
        path.as_os_str().as_encoded_bytes(),
        what,
        &path.display(),
        host_absolute_syntax(),
    )
}

fn check_absolute_bytes(
    raw: &[u8],
    what: &str,
    display: &dyn std::fmt::Display,
    syntax: AbsoluteSyntax,
) -> Result<()> {
    let Some(head) = absolute_head(raw, syntax) else {
        return Err(err(
            ErrorKind::InvalidRequest,
            format!("{what} {display} must be an absolute path"),
        ));
    };
    let Some(tail) = raw.get(head.len..) else {
        return Err(err(
            ErrorKind::InvalidRequest,
            format!("{what} {display} must be an absolute path"),
        ));
    };
    if tail.is_empty() {
        if head.rooted_by_separator {
            return Err(repeated_or_trailing(what, display));
        }
        return Ok(());
    }
    for segment in tail.split(|byte| tail_is_separator(*byte, syntax)) {
        if segment.is_empty() {
            return Err(repeated_or_trailing(what, display));
        }
        if segment == b"." || segment == b".." {
            return Err(err(
                ErrorKind::InvalidRequest,
                format!(
                    "{what} {display} contains a relative segment; v1 refuses an ambiguous path instead of resolving it"
                ),
            ));
        }
    }
    Ok(())
}

fn repeated_or_trailing(what: &str, display: &dyn std::fmt::Display) -> Error {
    err(
        ErrorKind::InvalidRequest,
        format!(
            "{what} {display} has a repeated or trailing separator; that is an ambiguous second spelling"
        ),
    )
}

/// `None` when `raw` is not absolute in `syntax`.
fn absolute_head(raw: &[u8], syntax: AbsoluteSyntax) -> Option<AbsoluteHead> {
    match syntax {
        AbsoluteSyntax::Unix => {
            if raw.first() == Some(&b'/') {
                Some(AbsoluteHead {
                    len: 1,
                    rooted_by_separator: true,
                })
            } else {
                None
            }
        }
        AbsoluteSyntax::Windows => windows_absolute_head(raw),
    }
}

struct WindowsPrefix {
    len: usize,
    /// `C:` is not a root by itself. Every other Windows prefix is.
    implicit_root: bool,
}

fn windows_absolute_head(raw: &[u8]) -> Option<AbsoluteHead> {
    let prefix = windows_prefix(raw)?;
    if prefix.len > raw.len() {
        return None;
    }
    let rest = &raw[prefix.len..];
    if rest.first().copied().is_some_and(is_windows_sep) {
        Some(AbsoluteHead {
            len: prefix.len + 1,
            rooted_by_separator: true,
        })
    } else if prefix.implicit_root {
        Some(AbsoluteHead {
            len: prefix.len,
            rooted_by_separator: false,
        })
    } else {
        None
    }
}

/// Prefix length, matching `std::path::Prefix::len` for the forms Rust treats
/// as prefixes. The match normalizes `/` to `\` the way the standard library
/// does, but a verbatim prefix is refused when those first four bytes contain
/// `/` (that spelling is not verbatim).
fn windows_prefix(raw: &[u8]) -> Option<WindowsPrefix> {
    if raw.len() >= 2 && is_windows_sep(raw[0]) && is_windows_sep(raw[1]) {
        return windows_unc_like_prefix(raw);
    }
    if matches!(raw, [drive, b':', ..] if drive.is_ascii_alphabetic()) {
        return Some(WindowsPrefix {
            len: 2,
            implicit_root: false,
        });
    }
    None
}

fn windows_unc_like_prefix(raw: &[u8]) -> Option<WindowsPrefix> {
    // `\\?\` (verbatim) only when the four marker bytes themselves use `\`.
    if raw.len() >= 4 && raw[2] == b'?' && is_windows_sep(raw[3]) && !raw[..4].contains(&b'/') {
        return Some(verbatim_prefix(raw));
    }
    if raw.len() >= 4 && raw[2] == b'.' && is_windows_sep(raw[3]) {
        let (name, _) = split_component(&raw[4..], false);
        return Some(WindowsPrefix {
            len: 4 + name.len(),
            implicit_root: true,
        });
    }
    let (server, after_server) = split_component(&raw[2..], false);
    let (share, _) = split_component(after_server, false);
    if server.is_empty() || share.is_empty() {
        return None;
    }
    Some(WindowsPrefix {
        len: unc_style_len(2, server.len(), share.len()),
        implicit_root: true,
    })
}

fn verbatim_prefix(raw: &[u8]) -> WindowsPrefix {
    let body = &raw[4..];
    if let Some(after_unc) = strip_unc_marker(body) {
        let (server, after_server) = split_component(after_unc, true);
        let (share, _) = split_component(after_server, true);
        return WindowsPrefix {
            len: unc_style_len(8, server.len(), share.len()),
            implicit_root: true,
        };
    }
    if verbatim_disk(body) {
        return WindowsPrefix {
            len: 6,
            implicit_root: true,
        };
    }
    let (name, _) = split_component(body, true);
    WindowsPrefix {
        len: 4 + name.len(),
        implicit_root: true,
    }
}

/// `UNC` plus a separator. `/` counts here because prefix matching normalizes
/// it; later verbatim components still split only on `\`.
fn strip_unc_marker(body: &[u8]) -> Option<&[u8]> {
    match body {
        [b'U', b'N', b'C', sep, rest @ ..] if is_windows_sep(*sep) => Some(rest),
        _ => None,
    }
}

/// `\\?\C:` exactly, or `\\?\C:\...`. A slash after the colon is not a
/// verbatim disk (`C:/...` is a general verbatim prefix instead).
fn verbatim_disk(body: &[u8]) -> bool {
    let third_is_backslash = match body.get(2) {
        Some(&byte) => byte == b'\\',
        None => true,
    };
    third_is_backslash && matches!(body, [drive, b':', ..] if drive.is_ascii_alphabetic())
}

/// Component and the bytes after it, including one separator when one was there.
/// `verbatim` splits only on `\`.
fn split_component(path: &[u8], verbatim: bool) -> (&[u8], &[u8]) {
    let is_sep = |byte: u8| {
        if verbatim {
            byte == b'\\'
        } else {
            is_windows_sep(byte)
        }
    };
    match path.iter().position(|&byte| is_sep(byte)) {
        Some(at) => (&path[..at], &path[at + 1..]),
        None => (path, &[]),
    }
}

/// `base` is `2` for `\\` or `8` for `\\?\UNC\`. The separator between server
/// and share counts only when the share is non-empty, matching `Prefix::len`.
fn unc_style_len(base: usize, server_len: usize, share_len: usize) -> usize {
    if share_len > 0 {
        base + server_len + 1 + share_len
    } else {
        base + server_len
    }
}

/// Derive the recorded save path from the trusted root and the actual save
/// directory. The caller never supplies the string.
///
/// Refuses a relative or non-normalized path, `..`, a save outside the root, a
/// symlinked component below the root, and a non-directory component.
fn derive_save_path(root: &Path, save_dir: &Path) -> Result<String> {
    check_absolute(root, "root")?;
    check_absolute(save_dir, "save_dir")?;
    let relative = save_dir.strip_prefix(root).map_err(|_| {
        err(
            ErrorKind::InvalidRequest,
            format!(
                "save_dir {} is outside root {}; v1 only accepts a save below the repository root",
                save_dir.display(),
                root.display()
            ),
        )
    })?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                // Recording must be lossless: a name that is not valid UTF-8 (or
                // that would need a separator to be written) has no unambiguous
                // POSIX spelling, so it is refused instead of being lossily
                // rewritten into a name that belongs to something else.
                let text = name.to_str().ok_or_else(|| {
                    err(
                        ErrorKind::InvalidRequest,
                        format!(
                            "save_dir {} has a component that is not valid UTF-8; v1 cannot record it losslessly",
                            save_dir.display()
                        ),
                    )
                })?;
                if text.contains('\\') {
                    return Err(err(
                        ErrorKind::InvalidRequest,
                        format!(
                            "save_dir {} has a component containing a backslash; the recorded POSIX path would be ambiguous",
                            save_dir.display()
                        ),
                    ));
                }
                parts.push(text.to_string());
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(err(
                    ErrorKind::InvalidRequest,
                    format!(
                        "save_dir {} is not a plain path below root {}",
                        save_dir.display(),
                        root.display()
                    ),
                ))
            }
        }
    }
    if parts.is_empty() {
        return Err(err(
            ErrorKind::InvalidRequest,
            format!(
                "save_dir {} is the root itself; a save slot must be below it",
                save_dir.display()
            ),
        ));
    }
    let mut current = root.to_path_buf();
    for part in &parts {
        current.push(part);
        let meta = fs::symlink_metadata(&current).map_err(|e| {
            ctx(
                ErrorKind::InvalidRequest,
                format!("inspect {}", current.display()),
                e,
            )
        })?;
        if meta.file_type().is_symlink() {
            return Err(err(
                ErrorKind::InvalidRequest,
                format!(
                    "{} is a symlink; v1 refuses to treat a linked path as a save identity",
                    current.display()
                ),
            ));
        }
        if !meta.is_dir() {
            return Err(err(
                ErrorKind::InvalidRequest,
                format!("{} is not a directory", current.display()),
            ));
        }
    }
    Ok(parts.join("/"))
}

fn validate_operation_id(operation_id: &str) -> Result<()> {
    let bad = operation_id.is_empty()
        || operation_id.len() > 96
        || !operation_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if bad {
        return Err(err(
            ErrorKind::InvalidRequest,
            format!("operation_id {operation_id:?} must be 1-96 ASCII letters, digits, '-' or '_'"),
        ));
    }
    Ok(())
}

fn validate_relative_target(path: &str) -> Result<()> {
    let bad = path.is_empty()
        || path.trim() != path
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if bad {
        return Err(err(
            ErrorKind::InvalidRequest,
            format!("target path {path:?} must be relative to the save slot"),
        ));
    }
    Ok(())
}

// ------------------------------------------------------------- state file ----

/// Serialize `record` with its checksum cleared, then fill the checksum in.
/// Verification runs the same normalization, so formatting alone never makes a
/// state invalid while any content change does.
pub fn reseal(record: &mut StateRecord) -> Result<()> {
    record.checksum.clear();
    let body = serde_json::to_vec_pretty(&*record)
        .map_err(|e| ctx(ErrorKind::Io, "encode the advancement state", e))?;
    record.checksum = digest(&body);
    Ok(())
}

fn verify_checksum(record: &StateRecord) -> Result<()> {
    let stored = record.checksum.clone();
    let mut copy = record.clone();
    reseal(&mut copy)?;
    if copy.checksum != stored {
        return Err(err(
            ErrorKind::CorruptState,
            "the state checksum does not match its body; treating the state as unknown",
        ));
    }
    Ok(())
}

fn parse_record(bytes: &[u8]) -> Result<StateRecord> {
    if bytes.is_empty() {
        return Err(err(
            ErrorKind::CorruptState,
            "the state file is zero bytes; this is unknown, never a first initialization",
        ));
    }
    serde_json::from_slice(bytes)
        .map_err(|e| ctx(ErrorKind::CorruptState, "parse the advancement state", e))
}

fn parse_header(bytes: &[u8]) -> Result<LockHeader> {
    if bytes.is_empty() {
        return Err(err(
            ErrorKind::CorruptState,
            "the advancement lock has no header; a human must handle this outside the protocol",
        ));
    }
    serde_json::from_slice(bytes)
        .map_err(|e| ctx(ErrorKind::CorruptState, "parse the lock header", e))
}

fn read_header(file: &File) -> Result<LockHeader> {
    let mut handle = file;
    handle
        .seek(SeekFrom::Start(0))
        .map_err(|e| ctx(ErrorKind::Io, "seek the advancement lock", e))?;
    let mut bytes = Vec::new();
    handle
        .read_to_end(&mut bytes)
        .map_err(|e| ctx(ErrorKind::Io, "read the advancement lock", e))?;
    parse_header(&bytes)
}

fn temp_state_path(save_dir: &Path) -> PathBuf {
    save_dir.join(format!(
        "{STATE_FILE}.tmp-{}-{}-{}",
        std::process::id(),
        nanos(),
        counter()
    ))
}

/// Write and sync a temporary state file, then read it back and verify it.
/// Returns the temporary path; the caller decides whether to publish it.
fn stage_state(save_dir: &Path, record: &mut StateRecord) -> Result<(PathBuf, Vec<u8>)> {
    reseal(record)?;
    let bytes = serde_json::to_vec_pretty(&*record)
        .map_err(|e| ctx(ErrorKind::Io, "encode the advancement state", e))?;
    let tmp = temp_state_path(save_dir);
    let staged = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| ctx(ErrorKind::Io, format!("create {}", tmp.display()), e))?;
        file.write_all(&bytes)
            .map_err(|e| ctx(ErrorKind::Io, format!("write {}", tmp.display()), e))?;
        file.flush()
            .map_err(|e| ctx(ErrorKind::Io, format!("flush {}", tmp.display()), e))?;
        file.sync_all()
            .map_err(|e| ctx(ErrorKind::Io, format!("sync {}", tmp.display()), e))?;
        Ok(())
    })();
    if let Err(e) = staged {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    let readback =
        fs::read(&tmp).map_err(|e| ctx(ErrorKind::Io, format!("read {}", tmp.display()), e))?;
    if readback != bytes {
        let _ = fs::remove_file(&tmp);
        return Err(err(
            ErrorKind::Io,
            "the staged state did not read back byte-identical",
        ));
    }
    Ok((tmp, bytes))
}

/// Publish the staged state for the first time **without overwriting**.
///
/// A hard link fails when the target exists, so a state created by another
/// process between the precheck and this call is never replaced. A filesystem
/// without hard links fails closed. If the directory entry cannot be synced
/// afterwards, the state may already be published: that is reported as
/// [`ErrorKind::StatePublishedNotDurable`] and never repaired automatically.
fn publish_state(save_dir: &Path, tmp: &Path) -> Result<()> {
    let target = state_path(save_dir);
    match fs::hard_link(tmp, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(err(
                ErrorKind::AlreadyInitialized,
                format!(
                    "{} appeared before this init published; refusing to overwrite it",
                    target.display()
                ),
            ))
        }
        Err(e) => {
            return Err(ctx(
                ErrorKind::Io,
                format!(
                    "link {} -> {} (this platform or filesystem cannot publish the state without overwriting it)",
                    tmp.display(),
                    target.display()
                ),
                e,
            ))
        }
    }
    if let Err(e) = fs::remove_file(tmp) {
        // Harmless: a leftover temp file is never read as the state.
        eprintln!(
            "advancement lock: could not remove the staged temp {} ({e}); it is ignored",
            tmp.display()
        );
    }
    match sync_dir(save_dir) {
        Ok(()) => Ok(()),
        Err(e) => Err(err(
            ErrorKind::StatePublishedNotDurable,
            format!(
                "{} may already be published, but its directory entry could not be synced ({}); its durability is unknown, this is not a success, and the protocol must not be repaired automatically",
                target.display(),
                e.detail()
            ),
        )),
    }
}

/// Whether this platform can sync a directory at all. It is a capability fact,
/// not a writable proof: even on a platform that can, the predicate still has
/// to pass.
fn dir_sync_supported() -> bool {
    cfg!(unix)
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<()> {
    let handle =
        File::open(dir).map_err(|e| ctx(ErrorKind::Io, format!("open {}", dir.display()), e))?;
    handle
        .sync_all()
        .map_err(|e| ctx(ErrorKind::Io, format!("sync {}", dir.display()), e))
}

#[cfg(not(unix))]
fn sync_dir(dir: &Path) -> Result<()> {
    // There is no portable directory fsync. Returning success here would claim
    // a durability this platform cannot provide, so it fails closed instead.
    Err(err(
        ErrorKind::EvidenceFailed,
        format!(
            "this platform cannot sync the directory {}; directory durability is unverified, so no commit is allowed",
            dir.display()
        ),
    ))
}

/// The validated levels of `dir` below `root`, without touching the disk.
///
/// Refuses a non-normalized or relative path, `..`, `.`, a directory outside
/// the root, a non-UTF-8 component, and an empty relative path.
fn relative_levels(
    root: &Path,
    dir: &Path,
    root_name: &str,
    dir_name: &str,
) -> Result<Vec<OsString>> {
    // Validate the *whole* paths losslessly, not only the components below the
    // root: a non-UTF-8 root would otherwise hide a non-UTF-8 `dir` behind
    // UTF-8 child names.
    for (path, name) in [(root, root_name), (dir, dir_name)] {
        if path.to_str().is_none() {
            return Err(err(
                ErrorKind::InvalidRequest,
                format!(
                    "{name} {} is not valid UTF-8; v1 cannot handle it losslessly",
                    path.display()
                ),
            ));
        }
    }
    check_absolute(root, root_name)?;
    check_absolute(dir, dir_name)?;
    let relative = dir.strip_prefix(root).map_err(|_| {
        err(
            ErrorKind::InvalidRequest,
            format!(
                "{dir_name} {} is outside {root_name} {}; it must be below it",
                dir.display(),
                root.display()
            ),
        )
    })?;
    let mut levels = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                if name.to_str().is_none() {
                    return Err(err(
                        ErrorKind::InvalidRequest,
                        format!(
                            "{dir_name} {} has a component that is not valid UTF-8; v1 cannot handle it losslessly",
                            dir.display()
                        ),
                    ));
                }
                levels.push(name.to_os_string());
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(err(
                    ErrorKind::InvalidRequest,
                    format!(
                        "{dir_name} {} is not a plain path below {root_name} {}",
                        dir.display(),
                        root.display()
                    ),
                ))
            }
        }
    }
    if levels.is_empty() {
        return Err(err(
            ErrorKind::InvalidRequest,
            format!(
                "{dir_name} {} must be strictly below {root_name} {}",
                dir.display(),
                root.display()
            ),
        ));
    }
    Ok(levels)
}

/// Prepare `dir` below the trusted, already existing `root`, and make every
/// directory entry involved durable.
///
/// Every attempt walks **all** levels from `root` down to `dir`: a missing level
/// is created, and the parent entry of **every** level is synced whether that
/// level was just created or already existed. That is what makes a retry repair
/// an earlier failure instead of skipping the level that failed first. The root
/// itself must exist, be a real directory and be syncable; it is never created.
/// Any failure is [`ErrorKind::EvidenceFailed`] and nothing is written to the
/// state.
fn ensure_evidence_dir(root: &Path, dir: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(root).map_err(|e| {
        ctx(
            ErrorKind::EvidenceFailed,
            format!(
                "the evidence root {} must already exist; it is never created automatically",
                root.display()
            ),
            e,
        )
    })?;
    if meta.file_type().is_symlink() {
        return Err(err(
            ErrorKind::EvidenceFailed,
            format!(
                "the evidence root {} is a symlink; it must be a real directory",
                root.display()
            ),
        ));
    }
    if !meta.is_dir() {
        return Err(err(
            ErrorKind::EvidenceFailed,
            format!("the evidence root {} is not a directory", root.display()),
        ));
    }
    let levels = relative_levels(root, dir, "the evidence root", "the evidence directory")?;
    // The root must be syncable before anything is created below it.
    sync_dir(root).map_err(|e| {
        err(
            ErrorKind::EvidenceFailed,
            format!(
                "the evidence root {} cannot be synced ({}); nothing was created",
                root.display(),
                e.detail()
            ),
        )
    })?;

    let mut current = root.to_path_buf();
    for level in &levels {
        current.push(level);
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(err(
                        ErrorKind::EvidenceFailed,
                        format!(
                            "{} is a symlink; the evidence path must be real directories",
                            current.display()
                        ),
                    ));
                }
                if !meta.is_dir() {
                    return Err(err(
                        ErrorKind::EvidenceFailed,
                        format!("{} is not a directory", current.display()),
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|e| {
                    ctx(
                        ErrorKind::EvidenceFailed,
                        format!("create {}", current.display()),
                        e,
                    )
                })?;
            }
            Err(e) => {
                return Err(ctx(
                    ErrorKind::EvidenceFailed,
                    format!("inspect {}", current.display()),
                    e,
                ))
            }
        }
        // Synced on every attempt, so an entry left behind by an earlier
        // failure is repaired rather than assumed.
        sync_parent_of(&current)?;
    }
    // The directory that will hold the pre-state file itself.
    sync_dir(dir).map_err(|e| {
        err(
            ErrorKind::EvidenceFailed,
            format!(
                "the evidence directory {} could not be synced ({}); stopping without writing active",
                dir.display(),
                e.detail()
            ),
        )
    })?;
    Ok(())
}

/// Sync the directory that contains `level`, so `level`'s entry is durable.
fn sync_parent_of(level: &Path) -> Result<()> {
    let Some(parent) = level.parent() else {
        return Ok(());
    };
    sync_dir(parent).map_err(|e| {
        err(
            ErrorKind::EvidenceFailed,
            format!(
                "the directory entry of {} could not be synced ({}); stopping without writing active",
                level.display(),
                e.detail()
            ),
        )
    })
}

// ---------------------------------------------------------------- acquire ----

fn acquire(file: &File, opts: &AcquireOptions) -> Result<()> {
    let deadline = Instant::now().checked_add(opts.timeout);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => {
                let expired = deadline.is_none_or(|deadline| Instant::now() >= deadline);
                if expired {
                    return Err(err(
                        ErrorKind::LockTimeout,
                        format!(
                            "another holder kept the advancement lock for {} ms; this operation does not delete, rewrite or take over the lock",
                            opts.timeout.as_millis()
                        ),
                    ));
                }
                let remaining = deadline
                    .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                    .unwrap_or(Duration::ZERO);
                let pause = if opts.retry.is_zero() {
                    Duration::from_millis(1)
                } else {
                    opts.retry
                };
                std::thread::sleep(pause.min(remaining));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(ctx(
                    ErrorKind::LockUnsupported,
                    "take the exclusive advancement lock",
                    e,
                ))
            }
        }
    }
}

fn recheck_path(path: &Path, expected: &LockIdentity) -> Result<()> {
    let current = LockIdentity::of_path(path)?;
    if &current != expected {
        return Err(err(
            ErrorKind::PathReplaced,
            format!(
                "{} no longer names the locked file (expected {}, found {}); aborting before any write",
                path.display(),
                expected.as_str(),
                current.as_str()
            ),
        ));
    }
    Ok(())
}

fn self_check_second_handle(path: &Path, expected: &LockIdentity) -> Result<()> {
    let second = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| ctx(ErrorKind::Io, format!("reopen {}", path.display()), e))?;
    let second_identity = LockIdentity::of_file(&second)?;
    if &second_identity != expected {
        return Err(err(
            ErrorKind::PathReplaced,
            "a second handle opened the path but not the locked file",
        ));
    }
    match second.try_lock() {
        Err(std::fs::TryLockError::WouldBlock) => Ok(()),
        Ok(()) => {
            let _ = second.unlock();
            Err(err(
                ErrorKind::LockUnsupported,
                "this platform or filesystem let a second handle take an exclusive lock on the same file; the advancement lock cannot be trusted here",
            ))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(ctx(
            ErrorKind::LockUnsupported,
            "self-check a second exclusive lock",
            e,
        )),
    }
}

// ------------------------------------------------------------------- init ----

/// Explicit initialization: the only way the two fixed files are ever created.
///
/// Refuses when either file exists (including zero-byte). Creates the lock with
/// `create_new`, locks that same handle, proves a second handle cannot lock it,
/// checks the path still names the locked file, writes the one immutable
/// header, stages and verifies `state=ready`, rechecks the path and only then
/// publishes the state with a hard link that cannot overwrite an existing one.
pub fn init(root: &Path, save_dir: &Path, opts: &InitOptions<'_>) -> Result<InitOutcome> {
    let save_path = derive_save_path(root, save_dir)?;
    let lock_file = lock_path(save_dir);
    let state_file = state_path(save_dir);

    for path in [&lock_file, &state_file] {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(err(
                    ErrorKind::AlreadyInitialized,
                    format!(
                    "{} already exists; init never takes over, rewrites or removes protocol files",
                    path.display()
                ),
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ctx(ErrorKind::Io, format!("inspect {}", path.display()), e)),
        }
    }

    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&lock_file)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => err(
                ErrorKind::AlreadyInitialized,
                format!(
                    "{} was created by another initializer; init never takes over",
                    lock_file.display()
                ),
            ),
            _ => ctx(ErrorKind::Io, format!("create {}", lock_file.display()), e),
        })?;
    acquire(&file, &opts.acquire)?;

    if let Some(hook) = opts.hooks.after_create {
        hook(&lock_file);
    }

    let identity = LockIdentity::of_file(&file)?;
    recheck_path(&lock_file, &identity)?;
    self_check_second_handle(&lock_file, &identity)?;
    recheck_path(&lock_file, &identity)?;

    let created_at = now();
    let header = LockHeader {
        schema: SCHEMA,
        save_id: new_save_id(&save_path),
        created_at: created_at.clone(),
    };
    let header_bytes = serde_json::to_vec_pretty(&header)
        .map_err(|e| ctx(ErrorKind::Io, "encode the lock header", e))?;
    // The one and only write to the lock file, through the locked handle.
    {
        let mut handle = &file;
        handle
            .write_all(&header_bytes)
            .map_err(|e| ctx(ErrorKind::Io, format!("write {}", lock_file.display()), e))?;
        handle
            .flush()
            .map_err(|e| ctx(ErrorKind::Io, format!("flush {}", lock_file.display()), e))?;
    }
    file.sync_all()
        .map_err(|e| ctx(ErrorKind::Io, format!("sync {}", lock_file.display()), e))?;

    let mut record = StateRecord {
        schema: SCHEMA,
        revision: 1,
        checksum: String::new(),
        save_id: header.save_id.clone(),
        operation_id: String::new(),
        state: State::Ready,
        stage: Stage::Init,
        lock_identity: identity.as_str().to_string(),
        save_path: save_path.clone(),
        published: Vec::new(),
        pending: Vec::new(),
        backup_dir: None,
        candidate_dir: None,
        last_error: None,
        created_at: created_at.clone(),
        updated_at: created_at,
    };
    let (tmp, _) = stage_state(save_dir, &mut record)?;

    if let Some(hook) = opts.hooks.before_publish {
        hook(&lock_file);
    }

    if let Err(e) = recheck_path(&lock_file, &identity) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = publish_state(save_dir, &tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    Ok(InitOutcome {
        header,
        state: record,
    })
}

// ------------------------------------------------------------------- open ----

/// Take the lock for a normal operation and validate the protocol.
///
/// Opens the lock without `create`, takes the OS lock on that same handle,
/// rechecks the path identity, opens the state without `create`, validates
/// schema, checksum, `save_id`, the derived `save_path`, lock identity and the
/// `ready` invariants, and walks the whole directory. Any failure is a
/// fail-closed error; nothing is repaired.
pub fn open(root: &Path, save_dir: &Path, opts: &OpenOptions) -> Result<AdvancementLock> {
    let save_path = derive_save_path(root, save_dir)?;
    let lock_file = lock_path(save_dir);
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_file)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => err(
                ErrorKind::NotInitialized,
                format!(
                    "{} does not exist; run init explicitly. Normal operation never creates protocol files",
                    lock_file.display()
                ),
            ),
            _ => ctx(ErrorKind::Io, format!("open {}", lock_file.display()), e),
        })?;
    acquire(&file, &opts.acquire)?;

    let identity = LockIdentity::of_file(&file)?;
    recheck_path(&lock_file, &identity)?;
    let header = read_header(&file)?;
    if header.schema != SCHEMA {
        return Err(err(
            ErrorKind::CorruptState,
            format!(
                "the lock header schema is {} but this build understands {SCHEMA}",
                header.schema
            ),
        ));
    }

    let state_file = state_path(save_dir);
    let bytes = fs::read(&state_file).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => err(
            ErrorKind::CorruptState,
            format!(
                "{} is missing while the lock exists; this is unknown, never a first initialization",
                state_file.display()
            ),
        ),
        _ => ctx(ErrorKind::Io, format!("read {}", state_file.display()), e),
    })?;
    let record = parse_record(&bytes)?;
    verify_record(&record, &header, &identity, &save_path)?;
    enumerate(save_dir)?;

    Ok(AdvancementLock {
        save_dir: save_dir.to_path_buf(),
        save_path,
        file,
        identity,
        header,
        state: record,
        poisoned: false,
    })
}

/// Walk every entry of `dir` and propagate an error from any of them. Reading
/// only the iterator would not prove the directory can be listed.
fn enumerate(dir: &Path) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|e| {
        ctx(
            ErrorKind::CorruptState,
            format!("enumerate {}", dir.display()),
            e,
        )
    })?;
    for entry in entries {
        entry.map_err(|e| {
            ctx(
                ErrorKind::CorruptState,
                format!("read a directory entry of {}", dir.display()),
                e,
            )
        })?;
    }
    Ok(())
}

fn verify_record(
    record: &StateRecord,
    header: &LockHeader,
    identity: &LockIdentity,
    save_path: &str,
) -> Result<()> {
    if record.schema != SCHEMA {
        return Err(err(
            ErrorKind::CorruptState,
            format!(
                "the state schema is {} but this build understands {SCHEMA}",
                record.schema
            ),
        ));
    }
    if record.revision == 0 {
        return Err(err(ErrorKind::CorruptState, "the state revision is 0"));
    }
    if record.save_id.is_empty()
        || record.lock_identity.is_empty()
        || record.save_path.is_empty()
        || record.created_at.is_empty()
        || record.updated_at.is_empty()
    {
        return Err(err(
            ErrorKind::CorruptState,
            "the state is missing a required identity field",
        ));
    }
    verify_checksum(record)?;
    if record.state == State::Ready
        && (record.stage != Stage::Init
            || !record.pending.is_empty()
            || !record.published.is_empty())
    {
        return Err(err(
            ErrorKind::CorruptState,
            "a ready state must have stage init and no pending or published entries; a self-consistent checksum does not make a contradiction valid",
        ));
    }
    if record.save_id != header.save_id {
        return Err(err(
            ErrorKind::IdentityMismatch,
            "the state save_id does not match the lock header",
        ));
    }
    if record.save_path != save_path {
        return Err(err(
            ErrorKind::SavePathMismatch,
            format!(
                "the state records save_path {:?} but this save is {:?}. {}",
                record.save_path, save_path, COPY_LIMITATION
            ),
        ));
    }
    let stored = LockIdentity::parse(&record.lock_identity).ok_or_else(|| {
        err(
            ErrorKind::CorruptState,
            "the recorded lock_identity has an unknown shape",
        )
    })?;
    if &stored != identity {
        return Err(err(
            ErrorKind::IdentityMismatch,
            "the current lock file is not the lock that wrote this state; this only proves it is a different file, not which copy is newer",
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------- lock ----

/// A held advancement lock for one save slot.
///
/// The OS lock is released when this value is dropped or the process exits. The
/// lock file is never removed, so a crash leaves no stale lock to steal — and
/// no record of an unfinished operation either: `active` is what says that.
#[derive(Debug)]
pub struct AdvancementLock {
    save_dir: PathBuf,
    save_path: String,
    file: File,
    identity: LockIdentity,
    header: LockHeader,
    state: StateRecord,
    /// Set when a state link succeeded but its directory sync failed: the new
    /// content is visible while its durability is unknown, so this holder must
    /// not continue.
    poisoned: bool,
}

impl AdvancementLock {
    pub fn header(&self) -> &LockHeader {
        &self.header
    }

    pub fn state(&self) -> &StateRecord {
        &self.state
    }

    pub fn save_dir(&self) -> &Path {
        &self.save_dir
    }

    /// The path derived from the caller's root, never a caller-supplied string.
    pub fn save_path(&self) -> &str {
        &self.save_path
    }

    /// The only "may start a normal write" answer. Re-reads the on-disk state
    /// first, so a state that changed after this holder opened it is refused,
    /// and refuses outright on a platform that cannot sync a directory.
    pub fn check_writable(&self) -> Result<()> {
        if self.poisoned {
            return Err(err(
                ErrorKind::UnwritableState,
                "a previous state publish was linked but may not be durable; reopen the lock instead of continuing",
            ));
        }
        if !dir_sync_supported() {
            return Err(err(
                ErrorKind::UnwritableState,
                "this platform cannot sync a directory, so no advancement commit is allowed here; that capability is not a writable proof",
            ));
        }
        self.recheck_lock()?;
        self.verify_disk_state()?;
        match self.state.state {
            State::Ready => Ok(()),
            State::Complete
                if self.state.pending.is_empty()
                    && self.state.published.iter().all(|entry| entry.done) =>
            {
                Ok(())
            }
            State::Complete => Err(err(
                ErrorKind::UnwritableState,
                "complete with a pending intent or an unfinished publish is a contradiction; treat it as in_doubt",
            )),
            State::Failed
                if self.state.published.is_empty()
                    && self.state.pending.is_empty()
                    && self.state.stage == Stage::StoppedBeforeWrite =>
            {
                Ok(())
            }
            State::Failed => Err(err(
                ErrorKind::UnwritableState,
                "failed records published or intended work; treat it as in_doubt",
            )),
            State::Active => Err(err(
                ErrorKind::UnwritableState,
                "a previous operation never finished (active); a human must recover from the D backup",
            )),
            State::InDoubt => Err(err(
                ErrorKind::UnwritableState,
                "the previous operation published partially or is unknown (in_doubt); a human must recover from the D backup",
            )),
        }
    }

    /// May an adopted ordinary writer (not a D commit) start writing targets?
    ///
    /// This is the ADV-P1-1a predicate. It re-reads the on-disk state and
    /// refuses a missing, unreadable, contradictory, `active` or `in_doubt`
    /// state, same as [`Self::check_writable`], but it does **not** require
    /// directory sync: these writers are the existing non-commit commands,
    /// and `check_writable` would refuse every write on a platform that
    /// cannot fsync a directory (Windows). A D commit still has to use
    /// [`Self::check_writable`]. Opening and checking must not write.
    pub fn check_ordinary_write(&self) -> Result<()> {
        if self.poisoned {
            return Err(err(
                ErrorKind::UnwritableState,
                "a previous state publish was linked but may not be durable; reopen the lock instead of continuing",
            ));
        }
        self.recheck_lock()?;
        self.verify_disk_state()?;
        match self.state.state {
            State::Ready => Ok(()),
            State::Complete
                if self.state.pending.is_empty()
                    && self.state.published.iter().all(|entry| entry.done) =>
            {
                Ok(())
            }
            State::Complete => Err(err(
                ErrorKind::UnwritableState,
                "complete with a pending intent or an unfinished publish is a contradiction; treat it as in_doubt",
            )),
            State::Failed
                if self.state.published.is_empty()
                    && self.state.pending.is_empty()
                    && self.state.stage == Stage::StoppedBeforeWrite =>
            {
                Ok(())
            }
            State::Failed => Err(err(
                ErrorKind::UnwritableState,
                "failed records published or intended work; treat it as in_doubt",
            )),
            State::Active => Err(err(
                ErrorKind::UnwritableState,
                "a previous operation never finished (active); a human must recover from the D backup",
            )),
            State::InDoubt => Err(err(
                ErrorKind::UnwritableState,
                "the previous operation published partially or is unknown (in_doubt); a human must recover from the D backup",
            )),
        }
    }

    /// Snapshot the previous state, then move to `active`/`admit`.
    ///
    /// `evidence_root` is the caller's trusted, already existing and synced
    /// directory; it is never created. `evidence_dir` must be strictly below it
    /// (normalized, no symlink, no `..`, no non-UTF-8 component, and both full
    /// paths must be valid UTF-8). Every attempt walks all levels from the root
    /// down and re-syncs every parent entry, including levels that already
    /// exist, so a retry repairs an earlier failure instead of skipping it.
    /// The snapshot is written, synced, read back and its directory synced
    /// before the state becomes `active`. A failure while **preparing that
    /// evidence** leaves the previous state untouched and never writes
    /// `active`; if the state rename itself succeeds and only the following
    /// directory sync fails, `active` may already be visible while its
    /// durability is unknown, which is reported as an error and must not be
    /// repaired automatically. The snapshot is control metadata, not a
    /// replacement for the D backup.
    /// The writable predicate runs first, so on a platform without directory
    /// sync the answer is `unwritable_state` before any evidence path is
    /// inspected.
    pub fn begin_operation(
        &mut self,
        operation_id: &str,
        evidence_root: &Path,
        evidence_dir: &Path,
    ) -> Result<()> {
        self.check_writable()?;
        validate_operation_id(operation_id)?;
        self.recheck_lock()?;
        self.write_prestate(operation_id, evidence_root, evidence_dir)?;

        let mut next = self.state.clone();
        next.operation_id = operation_id.to_string();
        next.state = State::Active;
        next.stage = Stage::Admit;
        next.published.clear();
        next.pending.clear();
        next.backup_dir = None;
        next.candidate_dir = None;
        next.last_error = None;
        self.store(next)
    }

    /// Record a publish intent (`pending`) before touching a target file.
    pub fn record_intent(&mut self, path: &str, before: &str, after: &str) -> Result<()> {
        self.require_active("record a publish intent")?;
        validate_relative_target(path)?;
        if self.state.pending.iter().any(|p| p.path == path)
            || self.state.published.iter().any(|p| p.path == path)
        {
            return Err(err(
                ErrorKind::InvalidRequest,
                format!("{path:?} already has an intent or a published record in this operation"),
            ));
        }
        let mut next = self.state.clone();
        next.pending.push(Pending {
            path: path.to_string(),
            before: before.to_string(),
            after: after.to_string(),
        });
        next.stage = Stage::Publish;
        self.store(next)
    }

    /// Move a pending intent to `published`/`done` after the file was replaced
    /// and synced.
    pub fn record_published(&mut self, path: &str) -> Result<()> {
        self.require_active("record a published file")?;
        let mut next = self.state.clone();
        let index = next
            .pending
            .iter()
            .position(|p| p.path == path)
            .ok_or_else(|| {
                err(
                    ErrorKind::InvalidRequest,
                    format!("{path:?} has no pending intent in this operation"),
                )
            })?;
        let intent = next.pending.remove(index);
        next.published.push(Published {
            path: intent.path,
            before: intent.before,
            after: intent.after,
            done: true,
        });
        self.store(next)
    }

    /// Write the only successful terminal state. Refuses unless the caller
    /// reports the candidate comparison, the log append and the integrity
    /// update as done and no intent is pending; anything else is `in_doubt`.
    pub fn finish_complete(&mut self, evidence: &CompletionEvidence) -> Result<()> {
        self.require_active("write the complete state")?;
        if !evidence.candidate_matches || !evidence.logs_recorded || !evidence.integrity_recorded {
            return Err(err(
                ErrorKind::InvalidRequest,
                "complete needs the candidate comparison, the log append and the integrity update verified; use in_doubt otherwise",
            ));
        }
        if !self.state.pending.is_empty() {
            return Err(err(
                ErrorKind::InvalidRequest,
                "complete needs an empty pending list; use in_doubt otherwise",
            ));
        }
        let mut next = self.state.clone();
        next.state = State::Complete;
        next.stage = Stage::Done;
        next.last_error = None;
        self.store(next)
    }

    /// Record a stop that provably happened before any target write, which
    /// leaves the save reopenable with a new operation.
    pub fn finish_failed(&mut self, reason: &str) -> Result<()> {
        self.require_active("write the failed state")?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(err(
                ErrorKind::InvalidRequest,
                "failed needs a non-blank reason",
            ));
        }
        if !self.state.published.is_empty() || !self.state.pending.is_empty() {
            return Err(err(
                ErrorKind::InvalidRequest,
                "failed is only for a stop before any target write; use in_doubt",
            ));
        }
        let mut next = self.state.clone();
        next.state = State::Failed;
        next.stage = Stage::StoppedBeforeWrite;
        next.last_error = Some(reason.to_string());
        self.store(next)
    }

    /// Record partial publication or an unknown outcome. Only a human may
    /// recover from here; the tool offers no resume or abort.
    pub fn finish_in_doubt(&mut self, reason: &str) -> Result<()> {
        self.require_active("write the in_doubt state")?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(err(
                ErrorKind::InvalidRequest,
                "in_doubt needs a non-blank reason",
            ));
        }
        let mut next = self.state.clone();
        next.state = State::InDoubt;
        next.last_error = Some(reason.to_string());
        self.store(next)
    }

    /// Recheck "path → handle → identity" and that the immutable header is
    /// unchanged. Every mutation runs this first.
    fn recheck_lock(&self) -> Result<()> {
        recheck_path(&lock_path(&self.save_dir), &self.identity)?;
        let header = read_header(&self.file)?;
        if header != self.header {
            return Err(err(
                ErrorKind::PathReplaced,
                "the lock header changed while it was held; it is immutable and a change aborts the operation",
            ));
        }
        Ok(())
    }

    /// Re-read the state file once and require it to be the exact revision this
    /// holder read. Returns the validated raw bytes so the caller can build the
    /// pre-state snapshot from the same read instead of reading again.
    /// Missing, zero-byte, corrupt or replaced data fails closed; nothing is
    /// rebuilt or overwritten.
    fn verify_disk_state(&self) -> Result<Vec<u8>> {
        let path = state_path(&self.save_dir);
        let bytes = fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => err(
                ErrorKind::CorruptState,
                format!(
                    "{} disappeared while the lock was held; failing closed instead of rebuilding it",
                    path.display()
                ),
            ),
            _ => ctx(ErrorKind::Io, format!("read {}", path.display()), e),
        })?;
        let record = parse_record(&bytes)?;
        verify_record(&record, &self.header, &self.identity, &self.save_path)?;
        if record != self.state {
            return Err(err(
                ErrorKind::CorruptState,
                format!(
                    "the state on disk is revision {} but this holder read revision {}; failing closed instead of overwriting it",
                    record.revision, self.state.revision
                ),
            ));
        }
        Ok(bytes)
    }

    fn require_active(&self, what: &str) -> Result<()> {
        self.recheck_lock()?;
        if self.state.state != State::Active {
            return Err(err(
                ErrorKind::InvalidRequest,
                format!(
                    "cannot {what} from state {:?}; an operation must be active",
                    self.state.state
                ),
            ));
        }
        Ok(())
    }

    /// Stage, verify and publish a new state revision. Never truncates in place
    /// and never rebuilds a state it could not re-read first.
    fn store(&mut self, mut next: StateRecord) -> Result<()> {
        self.recheck_lock()?;
        if self.poisoned {
            return Err(err(
                ErrorKind::UnwritableState,
                "a previous state publish was linked but may not be durable; reopen the lock instead of continuing",
            ));
        }
        self.verify_disk_state()?;
        next.revision = self
            .state
            .revision
            .checked_add(1)
            .ok_or_else(|| err(ErrorKind::InvalidRequest, "the state revision overflowed"))?;
        next.updated_at = now();
        let (tmp, _) = stage_state(&self.save_dir, &mut next)?;
        let target = state_path(&self.save_dir);
        if let Err(e) = fs::rename(&tmp, &target) {
            let _ = fs::remove_file(&tmp);
            return Err(ctx(
                ErrorKind::Io,
                format!("rename {} -> {}", tmp.display(), target.display()),
                e,
            ));
        }
        // Readers already see the new revision, so adopt it here too.
        self.state = next;
        if let Err(e) = sync_dir(&self.save_dir) {
            self.poisoned = true;
            let kind = e.kind();
            return Err(err(
                kind,
                format!(
                    "the new state was renamed but the directory sync failed ({}); its durability is unknown and this holder must not continue",
                    e.detail()
                ),
            ));
        }
        Ok(())
    }

    /// `admit` pre-state evidence: the raw previous state plus the values a
    /// human needs after a failure. Written with `create_new`, synced, read
    /// back and its directory synced before `active` is written.
    ///
    /// The read-back and the later link are separate syscalls, so a
    /// non-cooperating writer between them is outside what this can promise.
    fn write_prestate(
        &self,
        operation_id: &str,
        evidence_root: &Path,
        evidence_dir: &Path,
    ) -> Result<()> {
        // One read: the bytes that were just validated are the bytes recorded,
        // so there is no second-read window between validation and the snapshot.
        let raw = self.verify_disk_state()?;
        let state_bytes = String::from_utf8(raw)
            .map_err(|e| ctx(ErrorKind::CorruptState, "the state is not valid UTF-8", e))?;
        let document = PrestateDocument {
            schema: SCHEMA,
            operation_id,
            captured_at: now(),
            state_revision: self.state.revision,
            state_checksum: self.state.checksum.clone(),
            state_bytes,
            lock_identity: self.identity.as_str().to_string(),
            save_path: self.state.save_path.clone(),
            prior_state: self.state.state,
            prior_stage: self.state.stage,
            prior_operation_id: self.state.operation_id.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&document)
            .map_err(|e| ctx(ErrorKind::Io, "encode the pre-state snapshot", e))?;

        ensure_evidence_dir(evidence_root, evidence_dir)?;
        let path = evidence_dir.join(format!("{operation_id}-prestate.json"));
        let written = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| {
                    ctx(
                        ErrorKind::EvidenceFailed,
                        format!("create {}", path.display()),
                        e,
                    )
                })?;
            file.write_all(&bytes).map_err(|e| {
                ctx(
                    ErrorKind::EvidenceFailed,
                    format!("write {}", path.display()),
                    e,
                )
            })?;
            file.flush().map_err(|e| {
                ctx(
                    ErrorKind::EvidenceFailed,
                    format!("flush {}", path.display()),
                    e,
                )
            })?;
            file.sync_all().map_err(|e| {
                ctx(
                    ErrorKind::EvidenceFailed,
                    format!("sync {}", path.display()),
                    e,
                )
            })?;
            Ok(())
        })();
        written?;

        let readback = fs::read(&path).map_err(|e| {
            ctx(
                ErrorKind::EvidenceFailed,
                format!("read {}", path.display()),
                e,
            )
        })?;
        if readback != bytes {
            return Err(err(
                ErrorKind::EvidenceFailed,
                "the pre-state snapshot did not read back byte-identical",
            ));
        }
        // The file's directory entry must be durable before `active` is written.
        sync_dir(evidence_dir).map_err(|e| {
            err(
                ErrorKind::EvidenceFailed,
                format!(
                    "the pre-state directory {} could not be synced ({}); keeping the previous state and not writing active",
                    evidence_dir.display(),
                    e.detail()
                ),
            )
        })?;
        Ok(())
    }
}

/// The pre-operation snapshot document. Not a permanent history and not a
/// replacement for the D backup: it exists so a failure can be judged.
#[derive(Serialize)]
struct PrestateDocument<'a> {
    schema: u32,
    operation_id: &'a str,
    captured_at: String,
    state_revision: u64,
    state_checksum: String,
    state_bytes: String,
    lock_identity: String,
    save_path: String,
    prior_state: State,
    prior_stage: Stage,
    prior_operation_id: String,
}

#[cfg(test)]
mod absolute_spelling_tests {
    use super::{absolute_head, check_absolute, check_absolute_bytes, AbsoluteSyntax, ErrorKind};
    #[cfg(windows)]
    use std::path::Component;
    use std::path::{Path, PathBuf};

    fn judge(raw: &str, syntax: AbsoluteSyntax) -> super::Result<()> {
        check_absolute_bytes(raw.as_bytes(), "path", &raw, syntax)
    }

    fn assert_accept(raw: &str, syntax: AbsoluteSyntax) {
        let result = judge(raw, syntax);
        assert!(
            result.is_ok(),
            "{raw:?} under {syntax:?} should be accepted: {result:?}"
        );
    }

    fn assert_reject(raw: &str, syntax: AbsoluteSyntax, detail_needle: &str) {
        let err = judge(raw, syntax).expect_err(&format!("{raw:?} should be rejected"));
        assert_eq!(err.kind(), ErrorKind::InvalidRequest, "{raw:?}");
        assert!(
            err.detail().contains(detail_needle),
            "{raw:?}: detail {:?} does not contain {detail_needle:?}",
            err.detail()
        );
    }

    fn assert_head(raw: &str, syntax: AbsoluteSyntax, skipped: &str) {
        let head = absolute_head(raw.as_bytes(), syntax)
            .unwrap_or_else(|| panic!("{raw:?} should have a measurable prefix"));
        assert_eq!(&raw.as_bytes()[..head.len], skipped.as_bytes(), "{raw:?}");
    }

    #[test]
    fn verbatim_disk_prefix_is_not_a_repeated_separator() {
        // The Windows CI failure: canonicalize yields `\\?\C:\...`, and the
        // second backslash of `\\?\` must not be an empty segment.
        let root = r"\\?\C:\Users\runneradmin\AppData\Local\Temp\combat-sim-x";
        let save = r"\\?\C:\Users\runneradmin\AppData\Local\Temp\combat-sim-x\Saves\slot";
        assert_head(root, AbsoluteSyntax::Windows, r"\\?\C:\");
        assert_accept(root, AbsoluteSyntax::Windows);
        assert_accept(save, AbsoluteSyntax::Windows);

        let root_bytes = root.as_bytes();
        let save_bytes = save.as_bytes();
        assert!(save_bytes.starts_with(root_bytes));
        assert_eq!(save_bytes[root_bytes.len()], b'\\');
        assert_eq!(&save_bytes[root_bytes.len() + 1..], br"Saves\slot");
    }

    #[test]
    fn verbatim_disk_still_rejects_a_trailing_or_repeated_separator() {
        let syntax = AbsoluteSyntax::Windows;
        for raw in [
            r"\\?\C:\Users\runneradmin\foo\",
            r"\\?\C:\Users\runneradmin\AppData\Local\Temp\combat-sim-x\Saves\slot\",
            r"\\?\C:\Users\foo\\bar",
            r"\\?\C:\Users\runneradmin\AppData\Local\Temp\combat-sim-x\Saves\\slot",
            r"\\?\C:\\Users",
            r"\\?\C:\",
        ] {
            assert_reject(raw, syntax, "repeated or trailing separator");
        }
        assert_reject(r"\\?\C:\Users\.\foo", syntax, "relative segment");
        assert_reject(r"\\?\C:\Users\..\foo", syntax, "relative segment");
        assert_reject(
            r"\\?\C:\Users\runneradmin\AppData\Local\Temp\combat-sim-x\..\slot",
            syntax,
            "relative segment",
        );
    }

    #[test]
    fn windows_prefixes_skip_the_marker_then_keep_the_tail_strict() {
        let syntax = AbsoluteSyntax::Windows;
        for raw in [
            r"C:\Users\runneradmin\AppData\Local\Temp\combat-sim-x",
            r"c:\Users\runneradmin\foo",
            r"C:/Users/runneradmin/foo",
            r"\\?\c:\Users\runneradmin\foo",
            r"\\server\share\Users\runneradmin",
            r"//server/share/Users/runneradmin",
            r"\\server\share",
            r"\\?\UNC\server\share\Users\runneradmin",
            r"\\?\UNC\server\share",
            r"\\.\device\Users\runneradmin",
            r"\\.\COM1",
            r"\\?\Volume{12345678-1234-1234-1234-123456789abc}\Users\foo",
            r"\\?\foo\bar",
        ] {
            assert_accept(raw, syntax);
        }

        assert_head(r"C:\Users\runneradmin", syntax, r"C:\");
        assert_head(r"C:/Users/runneradmin", syntax, "C:/");
        assert_head(r"\\server\share\foo", syntax, r"\\server\share\");
        assert_head(r"\\server\share", syntax, r"\\server\share");
        assert_head(
            r"\\?\UNC\server\share\foo",
            syntax,
            r"\\?\UNC\server\share\",
        );
        assert_head(r"\\.\device\foo", syntax, r"\\.\device\");
        assert_head(r"\\.\COM1", syntax, r"\\.\COM1");
        assert_head(
            r"\\?\Volume{12345678-1234-1234-1234-123456789abc}\Users\foo",
            syntax,
            r"\\?\Volume{12345678-1234-1234-1234-123456789abc}\",
        );

        for raw in [
            r"C:\foo\",
            r"C:\foo\\bar",
            r"C:/foo/",
            r"C:/foo//bar",
            r"\\server\share\",
            r"\\server\share\foo\",
            r"\\server\share\foo\\bar",
            r"\\?\UNC\server\share\",
            r"\\?\UNC\server\share\foo\",
            r"\\?\UNC\server\share\foo\\bar",
            r"\\.\device\foo\",
            r"\\.\device\foo\\bar",
        ] {
            assert_reject(raw, syntax, "repeated or trailing separator");
        }
        for raw in [
            r"C:\foo\.\bar",
            r"C:\foo\..\bar",
            r"\\server\share\..\foo",
            r"\\?\UNC\server\share\.\foo",
            r"\\.\device\..\foo",
        ] {
            assert_reject(raw, syntax, "relative segment");
        }
        for raw in [
            "",
            r"Users\foo",
            r"C:Users",
            r"\Users\foo",
            r"..\foo",
            ".",
            r"\\server",
            r"\\server\",
        ] {
            assert_reject(raw, syntax, "must be an absolute path");
        }
    }

    #[test]
    fn unix_absolute_bytes_keep_the_existing_rules() {
        let syntax = AbsoluteSyntax::Unix;
        assert_head("/tmp/foo", syntax, "/");
        assert_accept("/tmp/combat-sim-x", syntax);
        assert_accept("/tmp/foo/bar", syntax);
        // A backslash is a name character on Unix, not a second separator.
        assert_accept(r"/tmp/foo\bar", syntax);
        for raw in ["/", "/tmp/foo/", "/tmp//foo", "//tmp"] {
            assert_reject(raw, syntax, "repeated or trailing separator");
        }
        for raw in ["/tmp/./foo", "/tmp/../foo", "/."] {
            assert_reject(raw, syntax, "relative segment");
        }
        for raw in ["", "tmp/foo", r"\\?\C:\Users\foo", r"C:\Users\foo"] {
            assert_reject(raw, syntax, "must be an absolute path");
        }
    }

    #[test]
    fn host_check_absolute_accepts_the_cwd_and_rejects_an_ambiguous_spelling() {
        let cwd = std::env::current_dir().expect("current directory");
        assert!(
            check_absolute(&cwd, "root").is_ok(),
            "cwd {} should be a normalized absolute path",
            cwd.display()
        );

        let trailing = PathBuf::from(format!("{}/", cwd.display()));
        let trailing_err = check_absolute(&trailing, "root").expect_err("trailing separator");
        assert_eq!(trailing_err.kind(), ErrorKind::InvalidRequest);
        assert!(
            trailing_err
                .detail()
                .contains("repeated or trailing separator"),
            "{}",
            trailing_err.detail()
        );

        let parent = cwd.join("..").join("not-a-save");
        let parent_err = check_absolute(&parent, "save_dir").expect_err("parent segment");
        assert_eq!(parent_err.kind(), ErrorKind::InvalidRequest);
        assert!(
            parent_err.detail().contains("relative segment"),
            "{}",
            parent_err.detail()
        );

        let relative = check_absolute(Path::new("relative"), "root").expect_err("relative");
        assert_eq!(relative.kind(), ErrorKind::InvalidRequest);
        assert!(relative.detail().contains("must be an absolute path"));
    }

    /// On Windows the byte parser must agree with `Component::Prefix` plus a
    /// physical `Component::RootDir`. Linux cannot produce those components
    /// for a `\\?\` path, so this oracle runs only there.
    #[cfg(windows)]
    #[test]
    fn windows_head_len_matches_std_components() {
        let samples = [
            r"C:\Users\runneradmin\AppData\Local\Temp\combat-sim-x",
            r"C:\",
            r"\\?\C:\Users\runneradmin\AppData\Local\Temp\combat-sim-x",
            r"\\?\C:\Users\runneradmin\foo\",
            r"\\?\C:\",
            r"\\?\UNC\server\share\Saves\slot",
            r"\\?\UNC\server\share",
            r"\\server\share\Saves\slot",
            r"\\server\share",
            r"\\server\share\",
            r"\\.\device\Saves\slot",
            r"\\.\COM1",
            r"C:relative",
            r"relative\path",
        ];
        for sample in samples {
            let path = Path::new(sample);
            let from_bytes =
                absolute_head(sample.as_bytes(), AbsoluteSyntax::Windows).map(|h| h.len);
            assert_eq!(
                from_bytes,
                component_head_len(path),
                "{sample} bytes {from_bytes:?} vs components"
            );
        }
    }

    #[cfg(windows)]
    fn component_head_len(path: &Path) -> Option<usize> {
        if !path.is_absolute() {
            return None;
        }
        let raw = path.as_os_str().as_encoded_bytes();
        let mut comps = path.components();
        let mut len = 0;
        if let Some(Component::Prefix(prefix)) = comps.next() {
            len += prefix.as_os_str().as_encoded_bytes().len();
        }
        if matches!(comps.next(), Some(Component::RootDir))
            && raw.get(len).copied().is_some_and(super::is_windows_sep)
        {
            len += 1;
        }
        Some(len)
    }
}
