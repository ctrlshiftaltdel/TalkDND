//! Read-only identity diagnostic for one save: complete discovery of the
//! character cards, their structured sheets and the campaign party, plus a
//! deterministic list of identity problems.
//!
//! Scope (see `docs/ADVANCEMENT-IDENTITY-CONTRACT.md` §3–§5): this module only
//! *reports*. It never writes, never creates a sidecar, never migrates, never
//! rebinds and never touches schema, party, progress or logs. `Status::Ok`
//! means "consistent inside the checks listed below"; it is **not** an
//! identity proof, authentication, authorization, freshness or tamper
//! evidence, and it says nothing about saves, paths or file systems this
//! module did not read.
//!
//! Discovery covers `Characters/*.md`, `NPCs/*.md`, `sheets/*.yaml` and the
//! `party` list of `combat/campaign.yaml`. It does not reuse
//! [`crate::check::sheets`], which drops unreadable files and unparsable
//! combat blocks: here a read, encoding or parse failure becomes
//! `Status::Incomplete` instead of an empty result.
//!
//! What is checked:
//!
//! * every card has a non-empty `id` and `name` in its `combat yaml` block;
//! * card ids, sheet ids and sheet file names are valid character ids
//!   ([`crate::chargen::valid_id`]: lowercase ASCII letters, digits and
//!   hyphens, starting with a letter or digit, at most 80 bytes);
//! * ids are unique case-insensitively, names are unique case-insensitively,
//!   and no id equals another card's name (across `Characters/` and `NPCs/`);
//! * `sheets/<id>.yaml` exists for a card, its `id` equals the card id, and
//!   its `file` names that card's Markdown and stays inside the save;
//! * every sheet belongs to a card (otherwise it is an orphan);
//! * sheet `version` is supported (1) and the identity fields are present and
//!   well typed; an absent or `null` `controller` is the old format and is
//!   accepted, any other value must be `player` or `dm`;
//! * unknown top-level sheet fields are counted (their names and values are
//!   never echoed);
//! * the party list, as the tools themselves read it, repeats no id and names
//!   no unknown id. `party` must be the manifest's top-level key, written as the
//!   **only** inline list at column 0, and that line must really be a mapping
//!   key: a block list, an indented `party:` line (the tools skip leading
//!   whitespace and would read a nested one), a `party:` line inside a
//!   multi-line quoted scalar, a second top-level `party:` line, or any form
//!   where the tools' line reader disagrees with the YAML is reported, not
//!   guessed.
//!
//! Paths are compared as real [`Path`] components, never as normalised display
//! strings: on Unix a `\` is part of a file name, so `Characters/a\\b.md` is
//! not `Characters/a/b.md`. The report shows a path as its components joined
//! with `/` (no separator is rewritten inside a component); a name that is not
//! valid UTF-8 cannot be shown or compared truthfully and is reported as an
//! `InvalidUtf8` finding for its directory instead of being replaced with a
//! lossy stand-in.
//!
//! A card with no structured sheet is `Status::Unknown`: no token has been
//! enrolled for it yet and its history is unknown. That is a state to report,
//! not an error.
//!
//! Duplicate ids, duplicate names and id/name collisions are computed over the
//! cards (the carriers), across `Characters/` and `NPCs/`. A sheet that
//! disagrees with its own file name or `file` is reported on its own instead.
//! Party entries are compared to card ids exactly; a case-only difference is
//! therefore both an unknown id and a duplicate.
//!
//! **Saved values are not echoed.** An [`Issue::detail`] holds only a fixed
//! category (plus a count, and the affected relative paths for a duplicate
//! group, which the caller needs to act); ids, names, `file` values, unknown
//! keys and versions from the save appear only where they are this module's own
//! vocabulary. [`Character`] and [`Report`] implement `Debug` by hand: ids,
//! names and the paths derived from them print as `<redacted>`, and a `Report`
//! prints its counters only. The raw values are reachable through
//! [`Character::id`] and [`Character::name`], which return untrusted save
//! content and must not be logged or displayed verbatim. [`Character::markdown`]
//! and [`Character::sheet`] return the locators, for callers that need to act.
//! [`Issue`] does show relative paths, because a finding is useless without
//! them: `Issue` output is for the DM's own save, and a save whose *file names*
//! are themselves sensitive must be handled by the caller.
//!
//! Every entry is classified, never silently dropped. The only exemptions are
//! entries with no identity meaning: dotfiles whose name is not a card/sheet
//! candidate (OS metadata such as `.DS_Store`) and `Characters/_party.md`
//! **when it carries no character block** (the companion overview documented
//! in `LetsPlay/docs/layout.md`). A `_`-prefixed `.md`/`.yaml` file is a
//! carrier candidate like any other; any other entry is reported.
//!
//! Not covered (contract §3.1/§6, still `UNKNOWN` there): sidecar tokens,
//! symlink and hard-link targets, case-insensitive file systems, permissions
//! semantics beyond this process's own access, external or older writers,
//! copies/rollbacks, and the loader's remaining requirements (for example
//! `ruleset`). `Characters`, `NPCs`, `sheets`, `combat` and the campaign
//! manifest are only used after `symlink_metadata` confirms the expected type,
//! so a symbolic link, a special file or a dangling link is reported and its
//! target is never read.
//!
//! Those checks are point-in-time observations. A concurrent writer can replace
//! a path between the check and the read; this diagnostic does not claim to
//! close that window and no no-follow guarantee under concurrent replacement is
//! proven here.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_norway::Value;

/// Folders holding character cards.
const CARD_DIRS: [&str; 2] = ["Characters", "NPCs"];
/// Folder holding the structured character data.
const SHEETS_DIR: &str = "sheets";
/// Campaign manifest with the `party` list.
const MANIFEST: &str = "combat/campaign.yaml";
/// The only structured-sheet version this diagnostic can interpret.
const SUPPORTED_SHEET_VERSION: u64 = 1;
/// The documented non-card overview file inside `Characters/`.
const PARTY_OVERVIEW: &str = "_party.md";

/// Fixed `InvalidId` categories (no value from the save is echoed).
const ID_RULE: &str = "lowercase letters, digits and hyphens; at most 80 bytes";
const CARD_ID_INVALID: &str = "a card id is not a valid character id";
const SHEET_ID_INVALID: &str = "the sheet id is not a valid character id";
const SHEET_STEM_INVALID: &str = "the sheet file name is not a valid character id";

/// Top-level keys of `sheets/<id>.yaml`, mirroring [`crate::charsheet::CharFile`].
/// Anything else is counted as an unknown field: an unrecognised key may carry
/// identity facts this version cannot interpret.
const KNOWN_SHEET_FIELDS: [&str; 26] = [
    "version",
    "id",
    "name",
    "file",
    "controller",
    "ruleset",
    "info",
    "fighting_style",
    "feats",
    "languages",
    "tools",
    "features",
    "weapon_prof",
    "masteries",
    "inventory",
    "ac_base",
    "ac_base_armored",
    "ac_base_has_style",
    "inspiration",
    "exhaustion",
    "notes",
    "dm_notes",
    "body_md",
    "tail_md",
    "combat",
    "spells",
];

/// Overall verdict, ordered from least to most severe: a save always reports
/// its worst finding (`Incomplete > Ambiguous > Unknown > Ok`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// No finding in the covered checks.
    #[default]
    Ok,
    /// Discovery succeeded, but some identity fact is not established
    /// (a card without a structured sheet; an unrecognised sheet field).
    Unknown,
    /// Discovery succeeded and the objects disagree with each other.
    Ambiguous,
    /// Discovery itself failed (I/O, encoding, parse, unsupported version), so
    /// nothing about that part of the save can be concluded.
    Incomplete,
}

/// One finding. Variants are declared in report order: `Incomplete` findings
/// first, then `Ambiguous`, then `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueKind {
    // ---- Incomplete: discovery or interpretation failed ----
    /// `Characters/`, `NPCs/` or `sheets/` exists but cannot be listed.
    UnreadableDirectory,
    /// A directory entry cannot be read.
    UnreadableEntry,
    /// A file cannot be read (including a manifest that is not a regular file).
    UnreadableFile,
    /// A file is not valid UTF-8.
    InvalidUtf8,
    /// An entry that is not a character card, a sheet or OS metadata.
    UnexpectedEntry,
    /// A card file has no ```combat yaml``` block.
    MissingCombatBlock,
    /// A card's ```combat yaml``` block cannot be parsed.
    UnparsableCombatBlock,
    /// A card's block has no usable `id` or `name`.
    MarkdownMissingField,
    /// A structured sheet cannot be parsed, or is not a YAML mapping.
    UnparsableSheet,
    /// A structured sheet lacks a required identity field.
    SheetMissingField,
    /// A structured sheet has an identity field with an unusable value.
    SheetInvalidField,
    /// A structured sheet declares a version this diagnostic cannot interpret.
    UnsupportedSheetVersion,
    /// `combat/campaign.yaml` cannot be parsed, or the tools and the YAML
    /// disagree about its `party` list.
    UnparsableCampaignManifest,
    // ---- Ambiguous: the objects disagree ----
    /// A card id, sheet id or sheet file name is not a valid character id.
    InvalidId,
    /// Two cards share an id, ignoring case.
    DuplicateId,
    /// Two cards share a name, ignoring case.
    DuplicateName,
    /// A card id equals another card's name, ignoring case.
    IdNameCollision,
    /// `sheets/<id>.yaml` carries another `id`.
    SheetIdMismatch,
    /// A sheet's `file` names a different card than the one it belongs to.
    SheetFileMismatch,
    /// A sheet's `file` is absolute, empty or leaves the save.
    SheetFileEscape,
    /// A sheet belongs to no card.
    OrphanSheet,
    /// The party names an id no card carries.
    PartyUnknownId,
    /// The party repeats an id, ignoring case.
    PartyDuplicateId,
    // ---- Unknown: not established ----
    /// A card has no `sheets/<id>.yaml` (history not enrolled).
    MarkdownWithoutSheet,
    /// A sheet has top-level fields this version does not know.
    UnknownSheetField,
}

impl IssueKind {
    /// The status this finding forces on the report.
    pub fn severity(self) -> Status {
        use IssueKind::*;
        match self {
            UnreadableDirectory
            | UnreadableEntry
            | UnreadableFile
            | InvalidUtf8
            | UnexpectedEntry
            | MissingCombatBlock
            | UnparsableCombatBlock
            | MarkdownMissingField
            | UnparsableSheet
            | SheetMissingField
            | SheetInvalidField
            | UnsupportedSheetVersion
            | UnparsableCampaignManifest => Status::Incomplete,
            InvalidId | DuplicateId | DuplicateName | IdNameCollision | SheetIdMismatch
            | SheetFileMismatch | SheetFileEscape | OrphanSheet | PartyUnknownId
            | PartyDuplicateId => Status::Ambiguous,
            MarkdownWithoutSheet | UnknownSheetField => Status::Unknown,
        }
    }
}

/// One finding. `path` is the affected path relative to the save, with `/`
/// separators — a locator the caller needs to act on, not a field value;
/// `detail` is a fixed category (with a count, or the affected relative paths
/// of a duplicate group) and never echoes a value read from the save.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Issue {
    pub kind: IssueKind,
    pub path: String,
    pub detail: String,
}

/// One character card as discovered, with the structured sheet bound to it.
///
/// The fields are private because they hold untrusted save content: read them
/// through the accessors, and never log a value returned by [`Character::id`]
/// or [`Character::name`]. `Debug` prints those two as `<redacted>`.
#[derive(Clone, PartialEq, Eq)]
pub struct Character {
    id: String,
    name: String,
    /// Real card path, relative to the save; used for every comparison.
    markdown: PathBuf,
    /// Display form of `markdown`.
    markdown_text: String,
    /// Real bound sheet path, when one exists.
    sheet: Option<PathBuf>,
    /// Display form of `sheet`.
    sheet_text: Option<String>,
}

impl Character {
    /// The raw `id` from the card's combat block. Untrusted save content:
    /// never log it or show it verbatim; use `Debug` for a redacted view.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The raw `name` from the card's combat block. Untrusted save content:
    /// never log it or show it verbatim; use `Debug` for a redacted view.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Card path relative to the save, e.g. `Characters/Bruni.md` (a locator).
    pub fn markdown(&self) -> &str {
        &self.markdown_text
    }

    pub(crate) fn markdown_path(&self) -> &Path {
        &self.markdown
    }

    /// Bound structured sheet path (`sheets/<id>.yaml`), when one exists.
    pub fn sheet(&self) -> Option<&str> {
        self.sheet_text.as_deref()
    }
}

impl fmt::Debug for Character {
    /// Fully redacted: ids and names are save content, and the card/sheet paths
    /// are derived from them (`characters/<slug>.md`), so a path can carry the
    /// same value. Locators stay available through the accessors.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Character")
            .field("id", &"<redacted>")
            .field("name", &"<redacted>")
            .field("markdown", &"<redacted>")
            .field("sheet", &self.sheet.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Result of one read-only pass. `issues` is sorted and stable; `characters`
/// is sorted by id and card path.
#[derive(Clone, PartialEq, Eq)]
pub struct Report {
    pub status: Status,
    pub issues: Vec<Issue>,
    pub characters: Vec<Character>,
}

impl fmt::Debug for Report {
    /// Counters only: a report is often logged, and its contents are save data.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Report")
            .field("status", &self.status)
            .field("issues", &self.issues.len())
            .field("characters", &self.characters.len())
            .finish()
    }
}

/// Discover and diagnose `save_dir` without writing anything.
pub fn diagnose(save_dir: &Path) -> Report {
    let mut diag = Diag::default();
    let mut characters = diag.discover_cards(save_dir);
    let sheets = diag.discover_sheets(save_dir);
    diag.bind_sheets(&mut characters, &sheets);
    diag.check_conflicts(&characters);
    diag.check_party(save_dir, &characters);
    characters.sort_by(|a, b| (&a.id, &a.markdown_text).cmp(&(&b.id, &b.markdown_text)));
    diag.issues.sort();
    Report {
        status: diag.status,
        issues: diag.issues,
        characters,
    }
}

#[derive(Default)]
struct Diag {
    status: Status,
    issues: Vec<Issue>,
}

/// One directory entry, already sorted by name by [`Diag::list_dir`].
///
/// `rel` is the real relative path and is what identity is decided on; `text` is
/// its display form for the report. The two are never interchangeable: on Unix a
/// name may contain `\`, so `Characters/a\b.md` is not `Characters/a/b.md`.
struct Entry {
    rel: PathBuf,
    text: String,
    name: String,
    abs: PathBuf,
    file_type: fs::FileType,
}

/// A parsed structured sheet, before it is bound to a card.
struct SheetFile {
    rel: std::path::PathBuf,
    text: String,
    id: String,
    /// The `file` value as written in the YAML; compared as a path, never as a
    /// normalised string.
    file: String,
}

impl Diag {
    fn push(&mut self, kind: IssueKind, path: impl Into<String>, detail: impl Into<String>) {
        let severity = kind.severity();
        if severity > self.status {
            self.status = severity;
        }
        self.issues.push(Issue {
            kind,
            path: path.into(),
            detail: detail.into(),
        });
    }

    /// Every entry of `dir`, sorted by name, with `UnreadableEntry` for an
    /// entry that cannot be read. A missing directory is normal; an unreadable
    /// one is not. Nothing is filtered here: classification is the caller's job
    /// so that every entry is accounted for.
    fn list_dir(&mut self, save: &Path, dir: &str) -> Vec<Entry> {
        let abs = save.join(dir);
        // The directory itself is checked without following links: a symlinked,
        // non-directory or dangling directory is reported, and its target is
        // never listed.
        let meta = match fs::symlink_metadata(&abs) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                self.push(
                    IssueKind::UnreadableDirectory,
                    dir,
                    format!("directory metadata cannot be read ({:?})", e.kind()),
                );
                return Vec::new();
            }
        };
        if !meta.file_type().is_dir() {
            self.push(
                IssueKind::UnreadableDirectory,
                dir,
                "not a directory (links are not followed)",
            );
            return Vec::new();
        }
        let entries = match fs::read_dir(&abs) {
            Ok(rd) => rd,
            Err(e) => {
                self.push(
                    IssueKind::UnreadableDirectory,
                    dir,
                    format!("directory cannot be listed ({:?})", e.kind()),
                );
                return Vec::new();
            }
        };
        let mut found: Vec<(OsString, std::path::PathBuf, fs::FileType)> = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    self.push(
                        IssueKind::UnreadableEntry,
                        dir,
                        format!("directory entry cannot be read ({:?})", e.kind()),
                    );
                    continue;
                }
            };
            let Ok(file_type) = entry.file_type() else {
                self.push(IssueKind::UnreadableEntry, dir, "entry type cannot be read");
                continue;
            };
            found.push((entry.file_name(), entry.path(), file_type));
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out = Vec::new();
        for (_, abs, file_type) in found {
            let Some(rel) = rel_to(save, &abs) else {
                self.push(
                    IssueKind::UnreadableEntry,
                    dir,
                    "entry path cannot be related to the save",
                );
                continue;
            };
            // A name that is not valid UTF-8 cannot be shown or compared
            // truthfully, so it is reported instead of being lossily renamed.
            let Some(text) = display_path(&rel) else {
                self.push(
                    IssueKind::InvalidUtf8,
                    dir,
                    "an entry name is not valid UTF-8",
                );
                continue;
            };
            let Some(name) = rel.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
                self.push(
                    IssueKind::InvalidUtf8,
                    dir,
                    "an entry name is not valid UTF-8",
                );
                continue;
            };
            out.push(Entry {
                rel,
                text,
                name,
                abs,
                file_type,
            });
        }
        out
    }

    fn read_text(&mut self, entry: &Entry) -> Option<String> {
        match fs::read(&entry.abs) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Some(text),
                Err(_) => {
                    self.push(
                        IssueKind::InvalidUtf8,
                        &entry.text,
                        "file is not valid UTF-8",
                    );
                    None
                }
            },
            Err(e) => {
                self.push(
                    IssueKind::UnreadableFile,
                    &entry.text,
                    format!("file cannot be read ({:?})", e.kind()),
                );
                None
            }
        }
    }

    fn discover_cards(&mut self, save: &Path) -> Vec<Character> {
        let mut out = Vec::new();
        for dir in CARD_DIRS {
            for entry in self.list_dir(save, dir) {
                if !entry.file_type.is_file() {
                    self.push(
                        IssueKind::UnexpectedEntry,
                        &entry.text,
                        "is not a regular file (links and directories are not followed)",
                    );
                    continue;
                }
                if !has_extension(&entry.name, "md") {
                    if is_os_metadata(&entry.name) {
                        continue;
                    }
                    self.push(
                        IssueKind::UnexpectedEntry,
                        &entry.text,
                        "is not a character card (expected .md)",
                    );
                    continue;
                }
                // `Characters/_party.md` is the companion overview: a known
                // non-card only while it carries no character block.
                let overview = dir == "Characters" && entry.name == PARTY_OVERVIEW;
                let Some(text) = self.read_text(&entry) else {
                    continue;
                };
                let block = match combat_block(&text) {
                    Ok(Some(block)) => block,
                    Ok(None) if overview => continue,
                    Ok(None) => {
                        self.push(
                            IssueKind::MissingCombatBlock,
                            &entry.text,
                            "no ```combat yaml``` block",
                        );
                        continue;
                    }
                    Err(()) => {
                        self.push(
                            IssueKind::UnparsableCombatBlock,
                            &entry.text,
                            "the ```combat yaml``` block cannot be parsed",
                        );
                        continue;
                    }
                };
                let id = text_field(&block, "id");
                let name = text_field(&block, "name");
                if id.is_none() || name.is_none() {
                    if id.is_none() {
                        self.push(
                            IssueKind::MarkdownMissingField,
                            &entry.text,
                            "the combat block has no usable `id`",
                        );
                    }
                    if name.is_none() {
                        self.push(
                            IssueKind::MarkdownMissingField,
                            &entry.text,
                            "the combat block has no usable `name`",
                        );
                    }
                    continue;
                }
                let (markdown, markdown_text) = (entry.rel, entry.text);
                out.push(Character {
                    id: id.unwrap_or_default(),
                    name: name.unwrap_or_default(),
                    markdown,
                    markdown_text,
                    sheet: None,
                    sheet_text: None,
                });
            }
        }
        out
    }

    fn discover_sheets(&mut self, save: &Path) -> Vec<SheetFile> {
        let mut out = Vec::new();
        for entry in self.list_dir(save, SHEETS_DIR) {
            if !entry.file_type.is_file() {
                self.push(
                    IssueKind::UnexpectedEntry,
                    &entry.text,
                    "is not a regular file (links and directories are not followed)",
                );
                continue;
            }
            if !has_extension(&entry.name, "yaml") {
                if is_os_metadata(&entry.name) {
                    continue;
                }
                self.push(
                    IssueKind::UnexpectedEntry,
                    &entry.text,
                    "is not a structured sheet (expected .yaml)",
                );
                continue;
            }
            let rel = entry.text.clone();
            let Some(text) = self.read_text(&entry) else {
                continue;
            };
            let value: Value = match serde_norway::from_str(&text) {
                Ok(v) => v,
                Err(_) => {
                    self.push(IssueKind::UnparsableSheet, &rel, "not parseable YAML");
                    continue;
                }
            };
            let Value::Mapping(map) = value else {
                self.push(IssueKind::UnparsableSheet, &rel, "not a YAML mapping");
                continue;
            };
            let mut fields: BTreeMap<&str, &Value> = BTreeMap::new();
            let mut unknown = 0usize;
            for (key, value) in &map {
                match key.as_str() {
                    Some(key) => {
                        fields.insert(key, value);
                    }
                    None => unknown += 1,
                }
            }
            unknown += fields
                .keys()
                .filter(|k| !KNOWN_SHEET_FIELDS.contains(k))
                .count();
            if unknown > 0 {
                self.push(
                    IssueKind::UnknownSheetField,
                    &rel,
                    format!("{unknown} unrecognised top-level field(s)"),
                );
            }
            let Some(version) = self.sheet_field(&fields, &rel, "version") else {
                continue;
            };
            let Some(version) = version.as_u64() else {
                self.push(
                    IssueKind::SheetInvalidField,
                    &rel,
                    "`version` is not an integer",
                );
                continue;
            };
            if version > SUPPORTED_SHEET_VERSION {
                self.push(
                    IssueKind::UnsupportedSheetVersion,
                    &rel,
                    "unsupported sheet version (this diagnostic supports 1)",
                );
                continue;
            }
            let Some(id) = self
                .sheet_field(&fields, &rel, "id")
                .and_then(|v| required_str(v, &rel, "id", self))
            else {
                continue;
            };
            // `name` is required by the loader; the card's own name is the one
            // this diagnostic compares, so the sheet copy is only validated.
            if self
                .sheet_field(&fields, &rel, "name")
                .and_then(|v| required_str(v, &rel, "name", self))
                .is_none()
            {
                continue;
            }
            let Some(file) = self
                .sheet_field(&fields, &rel, "file")
                .and_then(|v| required_str(v, &rel, "file", self))
            else {
                continue;
            };
            // An absent `controller` (and YAML `null`) is the old format; only
            // a value that is present and unreadable is a conflict.
            if let Some(controller) = fields.get("controller").copied() {
                match controller {
                    Value::Null => {}
                    Value::String(c)
                        if c == crate::charsheet::PLAYER || c == crate::charsheet::DM => {}
                    _ => {
                        self.push(
                            IssueKind::SheetInvalidField,
                            &rel,
                            "`controller` must be `player` or `dm`",
                        );
                        continue;
                    }
                }
            }
            // An unusable id or file name is reported, but the sheet is still
            // classified (bound, mismatched or orphan) instead of dropped.
            if !crate::chargen::valid_id(&id) {
                self.push(
                    IssueKind::InvalidId,
                    &rel,
                    format!("{SHEET_ID_INVALID} ({ID_RULE})"),
                );
            }
            let stem = entry
                .rel
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if !crate::chargen::valid_id(&stem) {
                self.push(
                    IssueKind::InvalidId,
                    &rel,
                    format!("{SHEET_STEM_INVALID} ({ID_RULE})"),
                );
            }
            out.push(SheetFile {
                rel: entry.rel,
                text: entry.text,
                id,
                file,
            });
        }
        out
    }

    /// The field when present; a missing required identity field is reported.
    fn sheet_field<'a>(
        &mut self,
        fields: &BTreeMap<&'a str, &'a Value>,
        rel: &str,
        key: &str,
    ) -> Option<&'a Value> {
        match fields.get(key) {
            Some(value) => Some(*value),
            None => {
                self.push(
                    IssueKind::SheetMissingField,
                    rel,
                    format!("required field `{key}` is missing"),
                );
                None
            }
        }
    }

    fn bind_sheets(&mut self, characters: &mut [Character], sheets: &[SheetFile]) {
        let mut bound: BTreeSet<&Path> = BTreeSet::new();
        for character in characters.iter_mut() {
            if !crate::chargen::valid_id(&character.id) {
                self.push(
                    IssueKind::InvalidId,
                    &character.markdown_text,
                    format!("{CARD_ID_INVALID} ({ID_RULE})"),
                );
                continue;
            }
            let want = Path::new(SHEETS_DIR).join(format!("{}.yaml", character.id));
            let sheet = match sheets.iter().find(|s| s.rel == want) {
                Some(sheet) => sheet,
                None => {
                    self.push(
                        IssueKind::MarkdownWithoutSheet,
                        &character.markdown_text,
                        "no structured sheet for this card (history not enrolled)",
                    );
                    continue;
                }
            };
            bound.insert(sheet.rel.as_path());
            character.sheet = Some(sheet.rel.clone());
            character.sheet_text = Some(sheet.text.clone());
            if sheet.id != character.id {
                self.push(
                    IssueKind::SheetIdMismatch,
                    &sheet.text,
                    "the sheet id does not match the card id",
                );
            }
            if !safe_relative(&sheet.file) {
                self.push(
                    IssueKind::SheetFileEscape,
                    &sheet.text,
                    "the sheet `file` is not a safe path inside the save",
                );
            } else if Path::new(&sheet.file) != character.markdown.as_path() {
                // Compared as real path components: a `\` inside a Unix file
                // name is not a separator, so it can never equal a subdirectory.
                self.push(
                    IssueKind::SheetFileMismatch,
                    &sheet.text,
                    "the sheet `file` does not name this card",
                );
            }
        }
        for sheet in sheets {
            if bound.contains(sheet.rel.as_path()) {
                continue;
            }
            if !safe_relative(&sheet.file) {
                self.push(
                    IssueKind::SheetFileEscape,
                    &sheet.text,
                    "the sheet `file` is not a safe path inside the save",
                );
                continue;
            }
            match characters
                .iter()
                .find(|c| Path::new(&sheet.file) == c.markdown.as_path())
            {
                Some(_) => self.push(
                    IssueKind::SheetFileMismatch,
                    &sheet.text,
                    "the sheet `file` names another card's file",
                ),
                None => self.push(
                    IssueKind::OrphanSheet,
                    &sheet.text,
                    "no card matches this sheet",
                ),
            }
        }
    }

    fn check_conflicts(&mut self, characters: &[Character]) {
        let mut by_id: BTreeMap<String, Vec<&Character>> = BTreeMap::new();
        let mut by_name: BTreeMap<String, Vec<&Character>> = BTreeMap::new();
        for character in characters {
            by_id
                .entry(character.id.to_lowercase())
                .or_default()
                .push(character);
            by_name
                .entry(character.name.to_lowercase())
                .or_default()
                .push(character);
        }
        for group in by_id.values() {
            if group.len() > 1 {
                let mut paths: Vec<&str> = group.iter().map(|c| c.markdown_text.as_str()).collect();
                paths.sort();
                self.push(
                    IssueKind::DuplicateId,
                    paths[0],
                    format!(
                        "id is shared by {} cards: {}",
                        paths.len(),
                        paths.join(", ")
                    ),
                );
            }
        }
        for group in by_name.values() {
            if group.len() > 1 {
                let mut paths: Vec<&str> = group.iter().map(|c| c.markdown_text.as_str()).collect();
                paths.sort();
                self.push(
                    IssueKind::DuplicateName,
                    paths[0],
                    format!(
                        "name is shared by {} cards: {}",
                        paths.len(),
                        paths.join(", ")
                    ),
                );
            }
        }
        for character in characters {
            let id_key = character.id.to_lowercase();
            let Some(group) = by_name.get(&id_key) else {
                continue;
            };
            if group.iter().any(|c| c.markdown != character.markdown) {
                self.push(
                    IssueKind::IdNameCollision,
                    &character.markdown_text,
                    "a card id equals another card's name",
                );
            }
        }
    }

    fn check_party(&mut self, save: &Path, characters: &[Character]) {
        // The manifest's own directory is checked without following links, so a
        // symlinked or non-directory `combat/` is reported and nothing inside it
        // is read.
        match fs::symlink_metadata(save.join("combat")) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => {
                self.push(
                    IssueKind::UnreadableDirectory,
                    "combat",
                    "not a directory (links are not followed)",
                );
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                self.push(
                    IssueKind::UnreadableDirectory,
                    "combat",
                    format!("directory metadata cannot be read ({:?})", e.kind()),
                );
                return;
            }
        }
        let abs = save.join(MANIFEST);
        // Refuse anything that is not a regular file: a directory, a symbolic
        // link or a FIFO must never be opened (a FIFO could block forever).
        let meta = match fs::symlink_metadata(&abs) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                self.push(
                    IssueKind::UnreadableFile,
                    MANIFEST,
                    format!("manifest metadata cannot be read ({:?})", e.kind()),
                );
                return;
            }
        };
        if !meta.file_type().is_file() {
            self.push(
                IssueKind::UnreadableFile,
                MANIFEST,
                "the manifest is not a regular file",
            );
            return;
        }
        let bytes = match fs::read(&abs) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.push(
                    IssueKind::UnreadableFile,
                    MANIFEST,
                    format!("manifest cannot be read ({:?})", e.kind()),
                );
                return;
            }
        };
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                self.push(
                    IssueKind::InvalidUtf8,
                    MANIFEST,
                    "the manifest is not valid UTF-8",
                );
                return;
            }
        };
        let value: Value = match serde_norway::from_str(&text) {
            Ok(value) => value,
            Err(_) => {
                self.push(
                    IssueKind::UnparsableCampaignManifest,
                    MANIFEST,
                    "the manifest is not parseable YAML",
                );
                return;
            }
        };
        let Value::Mapping(map) = value else {
            self.push(
                IssueKind::UnparsableCampaignManifest,
                MANIFEST,
                "the manifest is not a YAML mapping",
            );
            return;
        };
        let mut party = None;
        for (key, value) in &map {
            if key.as_str() == Some("party") {
                party = Some(value);
            }
        }
        let party_key_present = party.is_some();
        let ids: Vec<String> = match party {
            None => Vec::new(),
            Some(Value::Sequence(entries)) => {
                let mut ids = Vec::new();
                for entry in entries {
                    match entry.as_str() {
                        Some(id) => ids.push(id.to_string()),
                        None => {
                            self.push(
                                IssueKind::UnparsableCampaignManifest,
                                MANIFEST,
                                "`party` has a non-string entry",
                            );
                            return;
                        }
                    }
                }
                ids
            }
            Some(_) => {
                self.push(
                    IssueKind::UnparsableCampaignManifest,
                    MANIFEST,
                    "`party` is not a list",
                );
                return;
            }
        };
        // The manifest must read the same way to the tools that consume it.
        // They look for a top-level `party:` line and read an inline list, so a
        // block list or a key their line reader cannot see is reported even when
        // both sides end up empty. `start::party_ids` returns an empty list when
        // the file cannot be read, so read errors are reported here, above.
        // The line the tools read must be the manifest's own top-level `party`
        // entry, written as an inline list — and only once.
        let hits = party_hits(&text);
        if !hits.is_empty() && !party_key_present {
            self.push(
                IssueKind::UnparsableCampaignManifest,
                MANIFEST,
                "the tools read a `party:` line the manifest does not define at top level",
            );
            return;
        }
        match hits.first() {
            None if party_key_present => {
                self.push(
                    IssueKind::UnparsableCampaignManifest,
                    MANIFEST,
                    "the tools do not read a `party:` line from the manifest",
                );
                return;
            }
            None => {}
            Some(first) => {
                // Leading whitespace means the first line the tools would read
                // belongs to another key, not to the top-level `party`.
                if !first.top_level {
                    self.push(
                        IssueKind::UnparsableCampaignManifest,
                        MANIFEST,
                        "the tools' `party:` line is nested, not top-level",
                    );
                    return;
                }
                if hits.iter().filter(|hit| hit.top_level).count() != 1 {
                    self.push(
                        IssueKind::UnparsableCampaignManifest,
                        MANIFEST,
                        "the manifest has more than one top-level `party:` line",
                    );
                    return;
                }
                if !first.inline {
                    self.push(
                        IssueKind::UnparsableCampaignManifest,
                        MANIFEST,
                        "the `party` line is not an inline list the tools read",
                    );
                    return;
                }
                // A column-0 line can still be text inside a multi-line scalar;
                // then it is not a key and must not stand in for one.
                if !prefix_is_complete_document(&text, first.start) {
                    self.push(
                        IssueKind::UnparsableCampaignManifest,
                        MANIFEST,
                        "the `party:` line is inside a multi-line scalar, not a top-level key",
                    );
                    return;
                }
            }
        }
        if crate::start::party_ids(save) != ids {
            self.push(
                IssueKind::UnparsableCampaignManifest,
                MANIFEST,
                "the tools read a different party list from the manifest",
            );
            return;
        }
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        let mut unknown = 0usize;
        for id in &ids {
            *seen.entry(id.to_lowercase()).or_default() += 1;
            if !characters.iter().any(|c| &c.id == id) {
                unknown += 1;
            }
        }
        if unknown > 0 {
            self.push(
                IssueKind::PartyUnknownId,
                MANIFEST,
                format!("party names {unknown} id(s) no card carries"),
            );
        }
        for count in seen.values() {
            if *count > 1 {
                self.push(
                    IssueKind::PartyDuplicateId,
                    MANIFEST,
                    format!("party repeats an id ({count} entries)"),
                );
            }
        }
    }
}

/// Path relative to the save with `/` separators.
/// The path relative to the save, as real components. `None` when it cannot be
/// related to the save at all; nothing is guessed from a lossy string.
fn rel_to(save: &Path, path: &Path) -> Option<PathBuf> {
    path.strip_prefix(save).ok().map(Path::to_path_buf)
}

/// The report form of a relative path: its real components joined with `/`.
///
/// This is display only — identity is decided on the [`Path`] itself. A `\`
/// inside a component stays part of that name (on Unix `Characters/a\\b.md` is
/// one file), and a name that is not valid UTF-8 yields `None` instead of a
/// lossy stand-in that could collide with a real path.
fn display_path(rel: &Path) -> Option<String> {
    let mut out = String::new();
    for component in rel.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        let part = part.to_str()?;
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(part);
    }
    (!out.is_empty()).then_some(out)
}

fn has_extension(name: &str, want: &str) -> bool {
    Path::new(name)
        .extension()
        .map(|e| e.eq_ignore_ascii_case(want))
        .unwrap_or(false)
}

/// OS metadata (`.DS_Store`, `.gitkeep`, …) that is not a card or sheet name.
/// Only consulted for entries that are not `.md`/`.yaml` candidates.
fn is_os_metadata(name: &str) -> bool {
    name.starts_with('.')
}

/// One line [`crate::start::party_ids`] would match.
#[derive(Debug)]
struct PartyHit {
    /// Byte offset of the start of the line.
    start: usize,
    /// The value is an inline `[...]` list once the comment is stripped.
    inline: bool,
    /// The line starts at column 0 (no leading whitespace).
    top_level: bool,
}

/// Every line the tools' reader would match, in document order.
///
/// `start::party_ids` returns only the ids, so the syntactic form — which
/// decides whether the tools can read the list at all — is mirrored here; the
/// ids themselves are always taken from the real reader. The tools skip leading
/// whitespace, so a `party:` line nested under another key, or one inside a
/// multi-line scalar, would be read as the party unless it is rejected.
fn party_hits(text: &str) -> Vec<PartyHit> {
    let mut hits = Vec::new();
    let mut start = 0usize;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let body = body.strip_suffix('\r').unwrap_or(body);
        if let Some(rest) = body.trim_start().strip_prefix("party:") {
            let value = rest.split('#').next().unwrap_or("").trim();
            hits.push(PartyHit {
                start,
                inline: value.starts_with('[') && value.ends_with(']'),
                top_level: body.len() == body.trim_start().len(),
            });
        }
        start += line.len();
    }
    hits
}

/// Whether `text[..end]` already parses as a YAML document. If it does not, the
/// line at `end` continues a multi-line scalar or flow collection, so it is not
/// a mapping key of its own and cannot define the manifest's top-level `party`.
fn prefix_is_complete_document(text: &str, end: usize) -> bool {
    serde_norway::from_str::<Value>(&text[..end]).is_ok()
}

/// The ```combat yaml``` block: `Ok(None)` when there is no fence, `Err(())`
/// when a fence exists but is unterminated or not parseable. Both are distinct
/// on purpose — [`crate::start::combat_block`] collapses them to `None`, which
/// would silently hide a broken card.
fn combat_block(text: &str) -> Result<Option<Value>, ()> {
    let Some(start) = text.find("```combat yaml") else {
        return Ok(None);
    };
    let rest = &text[start..];
    let Some(newline) = rest.find('\n') else {
        return Err(());
    };
    let body_start = newline + 1;
    let Some(end) = rest[body_start..].find("```") else {
        return Err(());
    };
    serde_norway::from_str(&rest[body_start..body_start + end])
        .map(Some)
        .map_err(|_| ())
}

/// A non-blank string field, verbatim (no trimming: surrounding whitespace is a
/// value difference the caller can see).
fn text_field(value: &Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// A required string field of a sheet, reporting a missing or unusable value.
fn required_str(value: &Value, rel: &str, key: &str, diag: &mut Diag) -> Option<String> {
    match value.as_str() {
        Some(text) if !text.trim().is_empty() => Some(text.to_string()),
        _ => {
            diag.push(
                IssueKind::SheetInvalidField,
                rel,
                format!("`{key}` is not a non-blank string"),
            );
            None
        }
    }
}

/// A relative path that stays inside the save (`markdown_path` in charsheet.rs
/// applies the same rule before writing).
fn safe_relative(file: &str) -> bool {
    if file.is_empty() || file.trim() != file {
        return false;
    }
    let path = Path::new(file);
    !path.is_absolute() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::display_path;
    use std::path::Path;

    /// The display form must name exactly the real components: a backslash is
    /// never a separator, and a name that is not valid UTF-8 yields `None`
    /// instead of a lossy stand-in. Runs on every Unix, including the platforms
    /// whose file system refuses to store such a name.
    #[cfg(unix)]
    #[test]
    fn display_path_never_fabricates_a_name() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let backslash = Path::new(r"Characters/a\b.md");
        let nested = Path::new("Characters/a/b.md");
        assert_eq!(
            display_path(backslash).as_deref(),
            Some(r"Characters/a\b.md")
        );
        assert_eq!(display_path(nested).as_deref(), Some("Characters/a/b.md"));
        assert_ne!(
            display_path(backslash),
            display_path(nested),
            "one Unix file name can never display as two components"
        );

        let unnamed = Path::new(OsStr::from_bytes(b"Characters/x\xff.md"));
        assert_eq!(display_path(unnamed), None, "no lossy stand-in");
    }

    #[test]
    fn party_hits_mark_top_level_and_inline() {
        let text =
            "name: x\nparty: []\nother:\n  party: [a, b]  # note\nnote: \"q\nparty: []\nz\"\n";
        let hits = super::party_hits(text);
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert!(hits[0].top_level && hits[0].inline);
        assert!(!hits[1].top_level && hits[1].inline);
        assert!(hits[2].top_level && hits[2].inline);
        assert_eq!(&text[hits[0].start..hits[0].start + 6], "party:");
        assert_eq!(&text[hits[1].start..hits[1].start + 8], "  party:");
    }

    /// A `party:` line inside a multi-line scalar has no complete document
    /// before it, so it cannot be the manifest's top-level key.
    #[test]
    fn a_party_line_inside_a_multiline_scalar_has_no_document_prefix() {
        let inside = "note: \"hello\nparty: []\nworld\"\nparty : []\n";
        let hits = super::party_hits(inside);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(!super::prefix_is_complete_document(inside, hits[0].start));

        let normal = "name: x\nparty: []\n";
        let hits = super::party_hits(normal);
        assert_eq!(hits.len(), 1);
        assert!(super::prefix_is_complete_document(normal, hits[0].start));

        let first_line = "party: []\n";
        let hits = super::party_hits(first_line);
        assert!(super::prefix_is_complete_document(
            first_line,
            hits[0].start
        ));
    }

    /// Only plain relative components can be displayed: an absolute path, a
    /// parent step or an empty path has no display form.
    #[test]
    fn display_path_requires_normal_components_only() {
        assert_eq!(
            display_path(Path::new("Characters/Bruni.md")).as_deref(),
            Some("Characters/Bruni.md")
        );
        assert_eq!(display_path(Path::new("/etc/passwd")), None);
        assert_eq!(display_path(Path::new("a/../b")), None);
        assert_eq!(display_path(Path::new("")), None);
    }
}
