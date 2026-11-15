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
//! `party` list of `combat/campaign.yaml`. It does not reuse [`crate::check::sheets`],
//! which drops unreadable files and unparsable combat blocks: here a read,
//! encoding or parse failure becomes `Status::Incomplete` instead of an empty
//! result.
//!
//! What is checked:
//!
//! * every card has a non-empty `id` and `name` in its `combat yaml` block;
//! * ids are unique case-insensitively, names are unique case-insensitively,
//!   and no id equals another card's name (across `Characters/` and `NPCs/`);
//! * `sheets/<id>.yaml` exists for a card, its `id` equals the card id, and
//!   its `file` names that card's Markdown and stays inside the save;
//! * every sheet belongs to a card (otherwise it is an orphan);
//! * sheet `version` is supported (1) and the identity fields are present and
//!   well typed; unknown top-level fields are reported by name only;
//! * the party list does not repeat an id and does not name an unknown id.
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
//! Not covered (contract §3.1/§6, still `UNKNOWN` there): sidecar tokens,
//! symlink and hard-link targets, case-insensitive file systems, permissions
//! semantics beyond this process's own access, external or older writers,
//! copies/rollbacks, and the loader's remaining requirements (for example
//! `ruleset`). Directory entries other than regular `.md`/`.yaml` files are
//! reported, never assumed empty. Names starting with `.` (OS metadata) and
//! `_` (`Characters/_party.md` is the companion overview, see
//! `LetsPlay/docs/layout.md`) are not identity carriers, in either folder.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path};

use serde_norway::Value;

/// Folders holding character cards.
const CARD_DIRS: [&str; 2] = ["Characters", "NPCs"];
/// Folder holding the structured character data.
const SHEETS_DIR: &str = "sheets";
/// Campaign manifest with the `party` list.
const MANIFEST: &str = "combat/campaign.yaml";
/// The only structured-sheet version this diagnostic can interpret.
const SUPPORTED_SHEET_VERSION: u64 = 1;

/// Top-level keys of `sheets/<id>.yaml`, mirroring [`crate::charsheet::CharFile`].
/// Anything else is reported as an unknown field: an unrecognised key may carry
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
    /// A file cannot be read at all.
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
    /// `combat/campaign.yaml` cannot be parsed, or `party` is not an id list.
    UnparsableCampaignManifest,
    // ---- Ambiguous: the objects disagree ----
    /// A card id is not a single safe file name (so it cannot name a sheet).
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
/// separators; `detail` names ids and paths only — never sheet text, values or
/// secrets.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Issue {
    pub kind: IssueKind,
    pub path: String,
    pub detail: String,
}

/// One character card as discovered, with the structured sheet bound to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    /// `id` from the card's combat block, verbatim.
    pub id: String,
    /// `name` from the card's combat block, verbatim.
    pub name: String,
    /// Card path relative to the save, e.g. `Characters/Bruni.md`.
    pub markdown: String,
    /// Bound structured sheet (`sheets/<id>.yaml`) when one exists.
    pub sheet: Option<String>,
}

/// Result of one read-only pass. `issues` is sorted and stable; `characters`
/// is sorted by id and card path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub status: Status,
    pub issues: Vec<Issue>,
    pub characters: Vec<Character>,
}

/// Discover and diagnose `save_dir` without writing anything.
pub fn diagnose(save_dir: &Path) -> Report {
    let mut diag = Diag::default();
    let mut characters = diag.discover_cards(save_dir);
    let sheets = diag.discover_sheets(save_dir);
    diag.bind_sheets(&mut characters, &sheets);
    diag.check_conflicts(&characters);
    diag.check_party(save_dir, &characters);
    characters.sort_by(|a, b| (&a.id, &a.markdown).cmp(&(&b.id, &b.markdown)));
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

/// A parsed structured sheet, before it is bound to a card.
struct SheetFile {
    rel: String,
    stem: String,
    id: String,
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

    /// Regular files of `dir` (sorted by name), with `UnexpectedEntry` for
    /// anything else. A missing directory is normal; an unreadable one is not.
    /// `.` and `_` prefixed names are OS metadata / non-card files (see module docs).
    fn list_dir(&mut self, save: &Path, dir: &str) -> Vec<(String, std::path::PathBuf)> {
        let abs = save.join(dir);
        let entries = match fs::read_dir(&abs) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
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
                self.push(
                    IssueKind::UnreadableEntry,
                    rel_of(save, &entry.path()),
                    "entry type cannot be read",
                );
                continue;
            };
            found.push((entry.file_name(), entry.path(), file_type));
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        let mut files = Vec::new();
        for (name, path, file_type) in found {
            let name_lossy = name.to_string_lossy();
            if name_lossy.starts_with('.') || name_lossy.starts_with('_') {
                // OS metadata / documented non-card files such as `_party.md`.
                continue;
            }
            let rel = rel_of(save, &path);
            if file_type.is_dir() {
                self.push(
                    IssueKind::UnexpectedEntry,
                    rel,
                    "is a directory; no identity meaning is defined for it",
                );
            } else if !file_type.is_file() {
                self.push(
                    IssueKind::UnexpectedEntry,
                    rel,
                    "is not a regular file (links are not followed)",
                );
            } else {
                files.push((rel, path));
            }
        }
        files
    }

    fn read_text(&mut self, rel: &str, abs: &Path) -> Option<String> {
        match fs::read(abs) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Some(text),
                Err(_) => {
                    self.push(IssueKind::InvalidUtf8, rel, "file is not valid UTF-8");
                    None
                }
            },
            Err(e) => {
                self.push(
                    IssueKind::UnreadableFile,
                    rel,
                    format!("file cannot be read ({:?})", e.kind()),
                );
                None
            }
        }
    }

    fn discover_cards(&mut self, save: &Path) -> Vec<Character> {
        let mut out = Vec::new();
        for dir in CARD_DIRS {
            for (rel, abs) in self.list_dir(save, dir) {
                let is_markdown = Path::new(&rel)
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("md"))
                    .unwrap_or(false);
                if !is_markdown {
                    self.push(
                        IssueKind::UnexpectedEntry,
                        rel,
                        "is not a character card (expected .md)",
                    );
                    continue;
                }
                let Some(text) = self.read_text(&rel, &abs) else {
                    continue;
                };
                let block = match combat_block(&text) {
                    Ok(Some(block)) => block,
                    Ok(None) => {
                        self.push(
                            IssueKind::MissingCombatBlock,
                            rel,
                            "no ```combat yaml``` block",
                        );
                        continue;
                    }
                    Err(()) => {
                        self.push(
                            IssueKind::UnparsableCombatBlock,
                            rel,
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
                            rel.clone(),
                            "the combat block has no usable `id`",
                        );
                    }
                    if name.is_none() {
                        self.push(
                            IssueKind::MarkdownMissingField,
                            rel.clone(),
                            "the combat block has no usable `name`",
                        );
                    }
                    continue;
                }
                out.push(Character {
                    id: id.unwrap_or_default(),
                    name: name.unwrap_or_default(),
                    markdown: rel,
                    sheet: None,
                });
            }
        }
        out
    }

    fn discover_sheets(&mut self, save: &Path) -> Vec<SheetFile> {
        let mut out = Vec::new();
        for (rel, abs) in self.list_dir(save, SHEETS_DIR) {
            let is_yaml = Path::new(&rel)
                .extension()
                .map(|e| e.eq_ignore_ascii_case("yaml"))
                .unwrap_or(false);
            if !is_yaml {
                self.push(
                    IssueKind::UnexpectedEntry,
                    rel,
                    "is not a structured sheet (expected .yaml)",
                );
                continue;
            }
            let Some(text) = self.read_text(&rel, &abs) else {
                continue;
            };
            let value: Value = match serde_norway::from_str(&text) {
                Ok(v) => v,
                Err(_) => {
                    self.push(IssueKind::UnparsableSheet, rel, "not parseable YAML");
                    continue;
                }
            };
            let Value::Mapping(map) = value else {
                self.push(IssueKind::UnparsableSheet, rel, "not a YAML mapping");
                continue;
            };
            let mut fields: BTreeMap<&str, &Value> = BTreeMap::new();
            for (key, value) in &map {
                match key.as_str() {
                    Some(key) => {
                        fields.insert(key, value);
                    }
                    None => self.push(
                        IssueKind::UnknownSheetField,
                        rel.clone(),
                        "unrecognised non-string top-level key",
                    ),
                }
            }
            let unknown: Vec<&str> = fields
                .keys()
                .copied()
                .filter(|k| !KNOWN_SHEET_FIELDS.contains(k))
                .collect();
            if !unknown.is_empty() {
                self.push(
                    IssueKind::UnknownSheetField,
                    rel.clone(),
                    format!("unrecognised field(s): {}", unknown.join(", ")),
                );
            }
            let Some(version) = self.sheet_field(&fields, &rel, "version") else {
                continue;
            };
            let Some(version) = version.as_u64() else {
                self.push(
                    IssueKind::SheetInvalidField,
                    rel.clone(),
                    "`version` is not an integer",
                );
                continue;
            };
            if version > SUPPORTED_SHEET_VERSION {
                self.push(
                    IssueKind::UnsupportedSheetVersion,
                    rel.clone(),
                    format!("version {version} (this diagnostic supports 1)"),
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
            if let Some(controller) = self.sheet_field(&fields, &rel, "controller") {
                match controller.as_str() {
                    Some(crate::charsheet::PLAYER) | Some(crate::charsheet::DM) => {}
                    _ => {
                        self.push(
                            IssueKind::SheetInvalidField,
                            rel.clone(),
                            "`controller` must be `player` or `dm`",
                        );
                        continue;
                    }
                }
            }
            let stem = Path::new(&rel)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(SheetFile {
                rel,
                stem,
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
        let mut bound: BTreeSet<&str> = BTreeSet::new();
        for character in characters.iter_mut() {
            if !safe_name(&character.id) {
                self.push(
                    IssueKind::InvalidId,
                    &character.markdown,
                    format!("id `{}` is not a single safe file name", character.id),
                );
                continue;
            }
            let want = format!("{SHEETS_DIR}/{}.yaml", character.id);
            let sheet = match sheets.iter().find(|s| s.rel == want) {
                Some(sheet) => sheet,
                None => {
                    self.push(
                        IssueKind::MarkdownWithoutSheet,
                        &character.markdown,
                        "no structured sheet for this card (history not enrolled)",
                    );
                    continue;
                }
            };
            bound.insert(sheet.rel.as_str());
            character.sheet = Some(sheet.rel.clone());
            if sheet.id != character.id {
                self.push(
                    IssueKind::SheetIdMismatch,
                    &sheet.rel,
                    format!(
                        "sheet id `{}` but the file is `{}` and the card id is `{}`",
                        sheet.id, sheet.rel, character.id
                    ),
                );
            }
            if !safe_relative(&sheet.file) {
                self.push(
                    IssueKind::SheetFileEscape,
                    &sheet.rel,
                    format!("`file` `{}` is not a safe path inside the save", sheet.file),
                );
            } else if sheet.file != character.markdown {
                self.push(
                    IssueKind::SheetFileMismatch,
                    &sheet.rel,
                    format!(
                        "`file` is `{}` but the card for id `{}` is `{}`",
                        sheet.file, character.id, character.markdown
                    ),
                );
            }
        }
        for sheet in sheets {
            if bound.contains(sheet.rel.as_str()) {
                continue;
            }
            if !safe_relative(&sheet.file) {
                self.push(
                    IssueKind::SheetFileEscape,
                    &sheet.rel,
                    format!("`file` `{}` is not a safe path inside the save", sheet.file),
                );
                continue;
            }
            match characters.iter().find(|c| c.markdown == sheet.file) {
                Some(card) => self.push(
                    IssueKind::SheetFileMismatch,
                    &sheet.rel,
                    format!(
                        "`file` is `{}`, whose id is `{}`, but the sheet file names id `{}`",
                        card.markdown, card.id, sheet.stem
                    ),
                ),
                None => self.push(
                    IssueKind::OrphanSheet,
                    &sheet.rel,
                    format!(
                        "no card carries id `{}` (and `file` `{}` is absent)",
                        sheet.stem, sheet.file
                    ),
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
        for (key, group) in &by_id {
            if group.len() > 1 {
                let mut paths: Vec<&str> = group.iter().map(|c| c.markdown.as_str()).collect();
                paths.sort();
                self.push(
                    IssueKind::DuplicateId,
                    paths[0],
                    format!("id `{key}` is carried by: {}", paths.join(", ")),
                );
            }
        }
        for (key, group) in &by_name {
            if group.len() > 1 {
                let mut paths: Vec<&str> = group.iter().map(|c| c.markdown.as_str()).collect();
                paths.sort();
                self.push(
                    IssueKind::DuplicateName,
                    paths[0],
                    format!("name `{key}` is carried by: {}", paths.join(", ")),
                );
            }
        }
        for character in characters {
            let id_key = character.id.to_lowercase();
            let Some(group) = by_name.get(&id_key) else {
                continue;
            };
            let others: Vec<&str> = group
                .iter()
                .filter(|c| c.markdown != character.markdown)
                .map(|c| c.markdown.as_str())
                .collect();
            if !others.is_empty() {
                self.push(
                    IssueKind::IdNameCollision,
                    &character.markdown,
                    format!(
                        "id `{}` is also the name of: {}",
                        character.id,
                        others.join(", ")
                    ),
                );
            }
        }
    }

    fn check_party(&mut self, save: &Path, characters: &[Character]) {
        let abs = save.join(MANIFEST);
        let bytes = match fs::read(&abs) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
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
                    "manifest is not valid UTF-8",
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
                    "manifest is not parseable YAML",
                );
                return;
            }
        };
        let Some(party) = value.get("party") else {
            return;
        };
        let Some(entries) = party.as_sequence() else {
            self.push(
                IssueKind::UnparsableCampaignManifest,
                MANIFEST,
                "`party` is not a list",
            );
            return;
        };
        let mut ids: Vec<String> = Vec::new();
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
        let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for id in &ids {
            seen.entry(id.to_lowercase()).or_default().push(id.clone());
            if !characters.iter().any(|c| &c.id == id) {
                self.push(
                    IssueKind::PartyUnknownId,
                    MANIFEST,
                    format!("party names unknown id `{id}`"),
                );
            }
        }
        for (key, group) in &seen {
            if group.len() > 1 {
                self.push(
                    IssueKind::PartyDuplicateId,
                    MANIFEST,
                    format!("party repeats id `{key}` ({} entries)", group.len()),
                );
            }
        }
    }
}

/// Path relative to the save with `/` separators.
fn rel_of(save: &Path, path: &Path) -> String {
    path.strip_prefix(save)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
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

/// One path component, no `.`/`..`/root, no surrounding whitespace.
fn safe_name(name: &str) -> bool {
    if name.is_empty() || name.trim() != name {
        return false;
    }
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
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
