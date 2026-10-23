//! Save schema metadata (`Saves/<slot>/save-meta.yaml`) and migrations.
//!
//! The schema number describes the on-disk layout of a save:
//! 1 = Markdown sheets only, 2 = structured `sheets/<id>.yaml`, 3 = versioned
//! YAML files plus `save-meta.yaml`, 4 = the locked adventure-deviation
//! settings (`deviation:`) in `save-meta.yaml`, 5 = per-save `settings:`
//! (dice display, default on), 6 = every party member (player characters and
//! DM-run companions) in `Characters/` with a `controller:` field in
//! `sheets/<id>.yaml`; `NPCs/` keeps only story NPCs. `talkdnd start` and `talkdnd migrate` back the
//! whole save up into `Saves/_backups/` before writing, then run the
//! migrations in order. A save from a newer talkdnd is refused: this binary
//! cannot know its format.
//!
//! Migrations only update the integrity manifest for files that were clean
//! before; a file the DM had already hand-edited stays flagged.

use crate::classdata::Book;
use crate::progress::Progress;
use crate::reveal;
use crate::{charsheet, check, save};
use anyhow::{bail, Context, Result};
use combat_sim::fsutil::atomic_write;
use combat_sim::party::PartyState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "save-meta.yaml";
pub const CURRENT_SCHEMA: u32 = 6;
pub const BACKUP_DIR: &str = "_backups";

/// How far this save may deviate from the published adventure.
///
/// Set once when the save is created and stored in `save-meta.yaml`; there is
/// deliberately no command that changes it later. `scope` is 0 (fill blanks
/// only), 1 (minor encounters/NPCs/items/locations) or 2 (also side quests).
/// `ratio` is the per-chapter original-content ceiling in percent and is
/// `None` for scope 0.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Deviation {
    #[serde(default)]
    pub scope: u32,
    #[serde(default)]
    pub ratio: Option<u32>,
}

impl Deviation {
    pub const MIN_RATIO: u32 = 5;
    pub const MAX_RATIO: u32 = 25;

    /// Validate a player's choice from `talkdnd new`.
    pub fn new(scope: Option<u32>, ratio: Option<u32>) -> Result<Self> {
        let scope = scope.unwrap_or(0);
        if scope > 2 {
            bail!("剧本偏离度 scope 只能是 0、1、2（收到 {scope}）；scope 3 及以上暂未开放");
        }
        let ratio = match (scope, ratio) {
            (0, Some(_)) => bail!("scope 0 表示不偏离剧本，不能设置 --ratio"),
            (0, None) => None,
            (_, None) => bail!(
                "scope {scope} 需要 --ratio（{} 到 {} 的整数百分比）",
                Self::MIN_RATIO,
                Self::MAX_RATIO
            ),
            (_, Some(r)) => {
                if !(Self::MIN_RATIO..=Self::MAX_RATIO).contains(&r) {
                    bail!(
                        "ratio 必须是 {} 到 {} 的整数百分比（收到 {r}）",
                        Self::MIN_RATIO,
                        Self::MAX_RATIO
                    );
                }
                Some(r)
            }
        };
        Ok(Self { scope, ratio })
    }

    /// Validate a deviation read back from disk (also used by `talkdnd
    /// check`, so a hand-edited value is reported instead of silently used).
    pub fn check(&self) -> Result<()> {
        Self::new(Some(self.scope), self.ratio).map(|_| ())
    }

    pub fn scope_name(&self) -> &'static str {
        match self.scope {
            0 => "scope 0（不偏离：只补必要空白）",
            1 => "scope 1（次要原创：遭遇/NPC/物品/地点）",
            _ => "scope 2（完整支线）",
        }
    }

    pub fn describe(&self) -> String {
        match self.ratio {
            Some(r) => format!("{}，比例上限 {r}%", self.scope_name()),
            None => self.scope_name().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub date: String,
    /// Path relative to the play folder, e.g. `Saves/_backups/<name>`.
    #[serde(default)]
    pub backup: String,
}

/// Per-save table settings. They are changeable mid-game (unlike the locked
/// deviation) and every change is written to the session log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    /// Read dice values, DCs and modifiers aloud. Off = narrate outcomes only;
    /// the programs still roll and log every die (`rolls.log`).
    #[serde(default = "dice_on")]
    pub dice_display: bool,
}

fn dice_on() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { dice_display: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SaveMeta {
    pub schema: u32,
    #[serde(default)]
    pub tool: String,
    /// Locked when the save is created; see [`Deviation`].
    #[serde(default)]
    pub deviation: Deviation,
    /// Changeable table settings (schema 5+; older saves default to dice on).
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub migrations: Vec<Migration>,
}

pub fn path(save: &Path) -> PathBuf {
    save.join(FILE)
}

/// Load `save-meta.yaml`; a schema from a newer talkdnd is an error.
pub fn load(save: &Path) -> Result<Option<SaveMeta>> {
    let p = path(save);
    if !p.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let meta: SaveMeta =
        serde_norway::from_str(&text).with_context(|| format!("invalid {}", p.display()))?;
    if meta.schema > CURRENT_SCHEMA {
        bail!(
            "{}: 存档 schema {} 比这个 talkdnd 支持的版本（{}）新；请更新 talkdnd 后再打开（update talkdnd）",
            p.display(),
            meta.schema,
            CURRENT_SCHEMA
        );
    }
    Ok(Some(meta))
}

/// Current schema of a save. Missing metadata: 1 when a Markdown sheet still
/// has no structured file, else 2 (the pre-`save-meta.yaml` layout).
pub fn detect(save: &Path) -> Result<u32> {
    if let Some(meta) = load(save)? {
        return Ok(meta.schema.max(1));
    }
    for sheet in check::sheets(save) {
        if !charsheet::path(save, &sheet.id).is_file() {
            return Ok(1);
        }
    }
    Ok(2)
}

/// Write the current schema for a brand-new save.
pub fn init(save: &Path) -> Result<()> {
    init_with_deviation(save, &Deviation::default())
}

/// Write the current schema and the deviation chosen at save creation.
pub fn init_with_deviation(save: &Path, deviation: &Deviation) -> Result<()> {
    if path(save).is_file() {
        return Ok(());
    }
    deviation.check()?;
    let meta = SaveMeta {
        schema: CURRENT_SCHEMA,
        tool: env!("CARGO_PKG_VERSION").to_string(),
        deviation: deviation.clone(),
        settings: Settings::default(),
        migrations: Vec::new(),
    };
    write_meta(save, &meta)
}

pub(crate) fn write_meta(save: &Path, meta: &SaveMeta) -> Result<()> {
    let text = format!(
        "# TalkDND save schema metadata (written by talkdnd; do not edit).\n# Backups from migrations live in Saves/_backups/.\n{}",
        serde_norway::to_string(meta)?
    );
    atomic_write(path(save), text.as_bytes())
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub from: u32,
    pub to: u32,
    pub dry_run: bool,
    /// Backup path relative to the play folder.
    pub backup: Option<String>,
    pub sheets: Vec<String>,
    /// 3→4: the old save had no deviation; it becomes scope 0 (no deviation).
    pub deviation_default: bool,
    /// 4→5: the old save had no settings; dice display becomes on.
    pub settings_default: bool,
    /// 5→6: companion sheets moved from NPCs/ to Characters/ ("old → new").
    pub companions: Vec<String>,
    /// 5→6: sheets/<id>.yaml files that got a `controller:` field ("id=controller").
    pub controllers: Vec<String>,
    /// 5→6: anything the DM should look at (name collisions, ...).
    pub warnings: Vec<String>,
    pub log_text: Option<String>,
}

impl Report {
    pub fn already_current(&self) -> bool {
        self.from == self.to
    }

    pub fn lines(&self) -> Vec<String> {
        if self.already_current() {
            return vec![format!("存档 schema {}：无需迁移", self.to)];
        }
        if self.dry_run {
            let mut v = vec![format!(
                "dry-run：存档 schema {} → {}；会先把整份存档备份到 Saves/{BACKUP_DIR}/，再迁移（未写入任何文件）",
                self.from, self.to
            )];
            if !self.sheets.is_empty() {
                v.push(format!("会迁移角色卡：{}", self.sheets.join(", ")));
            }
            if self.deviation_default {
                v.push("会写入剧本偏离度：scope 0（不偏离）、无比例（旧存档没有设置过）".into());
            }
            if self.settings_default {
                v.push(
                    "会写入桌面设置：骰子显示开（默认；可随时用 `talkdnd settings dice off` 关闭）"
                        .into(),
                );
            }
            if !self.companions.is_empty() {
                v.push(format!(
                    "会把队友从 NPCs/ 移到 Characters/：{}",
                    self.companions.join("，")
                ));
            }
            if !self.controllers.is_empty() {
                v.push(format!(
                    "会写入操控方 controller：{}",
                    self.controllers.join(", ")
                ));
            }
            for w in &self.warnings {
                v.push(format!("注意：{w}"));
            }
            return v;
        }
        let mut v = vec![format!("存档 schema {} → {}：迁移完成", self.from, self.to)];
        if let Some(b) = &self.backup {
            v.push(format!("迁移前整份备份：`{b}`"));
        }
        if !self.sheets.is_empty() {
            v.push(format!(
                "已迁移角色卡 {} 张：{}",
                self.sheets.len(),
                self.sheets.join(", ")
            ));
        }
        if self.deviation_default {
            v.push("剧本偏离度：旧存档未设置，按 scope 0（不偏离）、无比例处理".into());
        }
        if self.settings_default {
            v.push("桌面设置：骰子显示默认开（可用 `talkdnd settings dice off` 关闭）".into());
        }
        if !self.companions.is_empty() {
            v.push(format!(
                "队友已统一到 Characters/：{}",
                self.companions.join("，")
            ));
        }
        if !self.controllers.is_empty() {
            v.push(format!(
                "已写入操控方 controller（player = 玩家角色，dm = DM 操控的同伴）：{}",
                self.controllers.join(", ")
            ));
        }
        for w in &self.warnings {
            v.push(format!("注意：{w}"));
        }
        if let Some(log) = &self.log_text {
            v.push(format!("已写入会话日志：{log}"));
        }
        v
    }
}

/// Migrate `save` to [`CURRENT_SCHEMA`]. `dry_run` reports only and writes
/// nothing. Idempotent: a save already at the current schema is left untouched.
pub fn ensure(save: &Path, root: &Path, dry_run: bool) -> Result<Report> {
    let from = detect(save)?;
    if from >= CURRENT_SCHEMA {
        return Ok(Report {
            from,
            to: from,
            dry_run,
            ..Default::default()
        });
    }
    let mut report = Report {
        from,
        to: CURRENT_SCHEMA,
        dry_run,
        deviation_default: from < 4,
        settings_default: from < 5,
        ..Default::default()
    };
    if dry_run {
        if from < 2 {
            for sheet in check::sheets(save) {
                if !charsheet::path(save, &sheet.id).is_file() {
                    report.sheets.push(sheet.id);
                }
            }
        }
        if from < 6 {
            let plan = plan_companions(save);
            report.companions = plan.moves.iter().map(|m| m.describe()).collect();
            report.warnings = plan.warnings.clone();
        }
        return Ok(report);
    }

    // Snapshot before any write: after the migration only files that were
    // clean before are recorded, so pre-existing hand edits stay flagged.
    let before = combat_sim::integrity::snapshot(save);
    // Files the DM already edited (or added/removed) are not rewritten by the
    // migration: a serde round-trip would drop unknown fields, comments and
    // formatting. They stay on disk exactly as they are and keep loading
    // because version 0 is accepted.
    let dirty: BTreeSet<String> = match combat_sim::integrity::load(save)? {
        Some(manifest) => combat_sim::integrity::changes_against(&manifest, &before)
            .into_iter()
            .map(|change| change.path().to_string())
            .collect(),
        None => BTreeSet::new(),
    };
    let slot = save
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "save".into());
    let backup = backup_save(save, root, &slot, from)?;
    report.backup = Some(backup.clone());

    if from < 2 {
        report.sheets = migrate_sheets(save, root)?;
    }
    add_versions(save, &dirty)?;
    let moved = if from < 6 {
        let r = move_companions(save)?;
        report.companions = r.moves.iter().map(|m| m.describe()).collect();
        report.controllers = r.controllers;
        report.warnings = r.warnings;
        r.moves
    } else {
        Vec::new()
    };

    let mut meta = load(save)?.unwrap_or_default();
    meta.schema = CURRENT_SCHEMA;
    meta.tool = env!("CARGO_PKG_VERSION").to_string();
    if from < 4 {
        // Old saves never had the setting: they are plain adventure runs.
        meta.deviation = Deviation::default();
    }
    let date = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    if from < 2 {
        meta.migrations.push(Migration {
            from: 1,
            to: 2,
            date: date.clone(),
            backup: backup.clone(),
        });
    }
    if from < 3 {
        meta.migrations.push(Migration {
            from: from.max(2),
            to: 3,
            date: date.clone(),
            backup: backup.clone(),
        });
    }
    if from < 4 {
        meta.migrations.push(Migration {
            from: from.max(3),
            to: 4,
            date: date.clone(),
            backup: backup.clone(),
        });
    }
    if from < 5 {
        meta.migrations.push(Migration {
            from: from.max(4),
            to: 5,
            date: date.clone(),
            backup: backup.clone(),
        });
    }
    if from < 6 {
        meta.migrations.push(Migration {
            from: from.max(5),
            to: 6,
            date: date.clone(),
            backup: backup.clone(),
        });
    }
    write_meta(save, &meta)?;

    let deviation_note = if from < 4 {
        "；剧本偏离度按 scope 0（不偏离）迁移"
    } else {
        ""
    };
    let settings_note = if from < 5 {
        "；骰子显示默认开"
    } else {
        ""
    };
    let companion_note = if report.companions.is_empty() {
        String::new()
    } else {
        format!("；队友移到 Characters/：{}", report.companions.join("，"))
    };
    let text = format!(
        "[{}] 存档结构迁移 schema {from}→{CURRENT_SCHEMA}（备份：{backup}）{deviation_note}{settings_note}{companion_note}",
        chrono::Local::now().format("%H:%M")
    );
    if let Some(log) = save::latest_log(save) {
        save::append(&log, &format!("- {text}"))?;
        report.log_text = Some(text);
    }

    combat_sim::integrity::after_write(save, &before)?;
    carry_dirty_flags(save, &moved, &dirty)?;
    Ok(report)
}

// ---------- 5→6: party companions move from NPCs/ to Characters/ ----------

#[derive(Debug, Clone, PartialEq)]
struct Move {
    /// File names inside NPCs/ and Characters/.
    from: String,
    to: String,
    /// sheets/<id>.yaml that points at the file, if any.
    sheet: Option<PathBuf>,
}

impl Move {
    fn describe(&self) -> String {
        format!("NPCs/{} → Characters/{}", self.from, self.to)
    }
}

#[derive(Debug, Default)]
struct CompanionPlan {
    moves: Vec<Move>,
    warnings: Vec<String>,
}

#[derive(Debug, Default)]
struct CompanionResult {
    moves: Vec<Move>,
    controllers: Vec<String>,
    warnings: Vec<String>,
}

/// `file:` value of a sheets/<id>.yaml text (first top-level `file:` line).
fn sheet_file_field(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        l.strip_prefix("file:")
            .map(|v| v.trim().trim_matches(['"', '\'']).replace('\\', "/"))
    })
}

fn has_top_level(text: &str, key: &str) -> bool {
    let prefix = format!("{key}:");
    text.lines().any(|l| l.starts_with(&prefix))
}

/// A free name in Characters/ for `name`: the same name when free, else
/// `<stem>-companion.md`, `<stem>-companion-2.md`, ...
fn free_name(chars: &Path, name: &str, taken: &BTreeSet<String>) -> String {
    let free = |n: &str| !chars.join(n).exists() && !taken.contains(n);
    if free(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    let mut cand = format!("{stem}-companion{ext}");
    let mut n = 2;
    while !free(&cand) {
        cand = format!("{stem}-companion-{n}{ext}");
        n += 1;
    }
    cand
}

/// Which NPCs/ files are party members. A file is a companion when a
/// sheets/<id>.yaml points at it (the structured data exists only for party
/// members), or when its combat block is on the party side / on the party
/// list / in party-state. `_party.md` (the companion overview) moves too.
/// Everything else (story NPCs, notes) stays in NPCs/.
fn plan_companions(save: &Path) -> CompanionPlan {
    let mut plan = CompanionPlan::default();
    let npcs = save.join("NPCs");
    if !npcs.is_dir() {
        return plan;
    }
    let chars = save.join("Characters");
    // NPCs/<name> -> sheet path
    let mut by_sheet: std::collections::BTreeMap<String, PathBuf> = Default::default();
    if let Ok(rd) = fs::read_dir(save.join("sheets")) {
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for f in files {
            if f.extension().is_none_or(|x| x != "yaml") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&f) else {
                continue;
            };
            if let Some(name) = sheet_file_field(&text).and_then(|v| {
                v.strip_prefix("NPCs/")
                    .filter(|n| !n.contains('/'))
                    .map(str::to_string)
            }) {
                by_sheet.insert(name, f);
            }
        }
    }
    let party: BTreeSet<String> = crate::start::party_ids(save).into_iter().collect();
    let live: BTreeSet<String> =
        PartyState::load(&save.join("combat").join(combat_sim::party::FILE_NAME))
            .map(|ps| ps.members.keys().cloned().collect())
            .unwrap_or_default();
    let mut names: Vec<String> = fs::read_dir(&npcs)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let mut taken = BTreeSet::new();
    for name in names {
        let is_overview = name == "_party.md";
        let sheet = by_sheet.get(&name).cloned();
        let companion = is_overview
            || sheet.is_some()
            || (name.ends_with(".md")
                && fs::read_to_string(npcs.join(&name))
                    .ok()
                    .and_then(|t| crate::start::combat_block(&t))
                    .is_some_and(|b| {
                        let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        b.get("side").and_then(|v| v.as_str()) == Some("party")
                            || (!id.is_empty() && (party.contains(id) || live.contains(id)))
                    }));
        if !companion {
            continue;
        }
        let to = free_name(&chars, &name, &taken);
        if to != name {
            plan.warnings.push(format!(
                "Characters/{name} 已存在，NPCs/{name} 改名为 Characters/{to}（两份都保留，请人工核对）"
            ));
        }
        taken.insert(to.clone());
        plan.moves.push(Move {
            from: name,
            to,
            sheet,
        });
    }
    plan
}

/// Insert `- **操控**：…` as the first line of the generated part of a
/// rendered sheet, exactly where `charsheet::render` puts it. Sheets that
/// were never rendered (no generated part) get it on their next render.
fn add_controller_line(md: &str, controller: &str) -> Option<String> {
    let anchor = format!("{}\n\n## 角色选项\n\n", charsheet::BEGIN);
    let at = md.find(&anchor)? + anchor.len();
    if md[at..].starts_with("- **操控**：") {
        return None;
    }
    Some(format!(
        "{}{}{}",
        &md[..at],
        charsheet::controller_line(controller),
        &md[at..]
    ))
}

/// Move the companions, point their sheets at the new files and give every
/// structured sheet a `controller:`. Text edits only (no serde round-trip), so
/// comments, unknown fields and hand edits survive. Idempotent.
fn move_companions(save: &Path) -> Result<CompanionResult> {
    let plan = plan_companions(save);
    let mut out = CompanionResult {
        warnings: plan.warnings.clone(),
        ..Default::default()
    };
    let npcs = save.join("NPCs");
    let chars = save.join("Characters");
    if !plan.moves.is_empty() {
        fs::create_dir_all(&chars).with_context(|| format!("create {}", chars.display()))?;
    }
    for m in &plan.moves {
        let src = npcs.join(&m.from);
        let dest = chars.join(&m.to);
        if dest.exists() {
            bail!("{} already exists; not overwriting it", dest.display());
        }
        if m.from == "_party.md" {
            // The overview's links were relative to NPCs/.
            let text =
                fs::read_to_string(&src).with_context(|| format!("read {}", src.display()))?;
            let fixed = text
                .replace("](../Characters/", "](")
                .replace("`../Characters/", "`");
            atomic_write(&dest, fixed.as_bytes())?;
            fs::remove_file(&src).with_context(|| format!("remove {}", src.display()))?;
        } else {
            fs::rename(&src, &dest)
                .with_context(|| format!("move {} to {}", src.display(), dest.display()))?;
        }
        if let Some(sheet) = &m.sheet {
            let text = fs::read_to_string(sheet)?;
            let old = format!("NPCs/{}", m.from);
            let new = format!("Characters/{}", m.to);
            let edited: Vec<String> = text
                .lines()
                .map(|l| {
                    if l.starts_with("file:") && l.contains(&old) {
                        l.replacen(&old, &new, 1)
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            let mut edited = edited.join("\n");
            if text.ends_with('\n') {
                edited.push('\n');
            }
            atomic_write(sheet, edited.as_bytes())?;
        }
        out.moves.push(m.clone());
    }
    let moved_sheets: BTreeSet<PathBuf> =
        plan.moves.iter().filter_map(|m| m.sheet.clone()).collect();

    // controller: for every structured sheet that lacks one.
    let mut sheet_files: Vec<PathBuf> = fs::read_dir(save.join("sheets"))
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    sheet_files.sort();
    for f in sheet_files {
        if f.extension().is_none_or(|x| x != "yaml") {
            continue;
        }
        let text = fs::read_to_string(&f)?;
        let Some(file) = sheet_file_field(&text) else {
            continue;
        };
        let id = f
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let controller = if has_top_level(&text, "controller") {
            text.lines()
                .find_map(|l| l.strip_prefix("controller:"))
                .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
                .unwrap_or_default()
        } else {
            let combat = serde_norway::from_str::<serde_norway::Value>(&text)
                .ok()
                .and_then(|v| v.get("combat").cloned())
                .unwrap_or_default();
            let c = if moved_sheets.contains(&f) {
                charsheet::DM
            } else {
                charsheet::infer_controller(&file, &combat)
            };
            let mut edited = String::new();
            let mut done = false;
            for l in text.split_inclusive('\n') {
                edited.push_str(l);
                if !done && l.starts_with("file:") {
                    if !l.ends_with('\n') {
                        edited.push('\n');
                    }
                    edited.push_str(&format!("controller: {c}\n"));
                    done = true;
                }
            }
            atomic_write(&f, edited.as_bytes())?;
            out.controllers.push(format!("{id}={c}"));
            c.to_string()
        };
        // Rendered sheets show the controller in their generated part.
        let md_path = save.join(&file);
        if md_path.starts_with(&chars) && md_path.is_file() {
            let md = fs::read_to_string(&md_path)?;
            if let Some(new) = add_controller_line(&md, &controller) {
                atomic_write(&md_path, new.as_bytes())?;
            }
        }
    }
    Ok(out)
}

/// A companion sheet the DM had hand-edited before the move stays flagged
/// under its new path: the manifest keeps the old fingerprint.
fn carry_dirty_flags(save: &Path, moved: &[Move], dirty: &BTreeSet<String>) -> Result<()> {
    let Some(mut m) = combat_sim::integrity::load(save)? else {
        return Ok(());
    };
    let mut changed = false;
    for mv in moved {
        let old = format!("NPCs/{}", mv.from);
        if !dirty.contains(&old) || mv.from.starts_with('_') {
            continue;
        }
        let new = format!("Characters/{}", mv.to);
        if let Some(h) = m.files.remove(&old) {
            m.files.insert(new, h);
            changed = true;
        }
    }
    if changed {
        combat_sim::integrity::store(save, &m)?;
    }
    Ok(())
}

/// Recursive copy of the save, skipping machine-only directories.
fn backup_save(save: &Path, root: &Path, slot: &str, schema: u32) -> Result<String> {
    let parent = root.join("Saves").join(BACKUP_DIR);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut name = format!("{slot}-{stamp}-schema{schema}");
    let mut n = 2;
    while parent.join(&name).exists() {
        name = format!("{slot}-{stamp}-schema{schema}-{n}");
        n += 1;
    }
    let dest = parent.join(&name);
    copy_tree(save, &dest)?;
    Ok(format!("Saves/{BACKUP_DIR}/{name}"))
}

fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(name.as_str(), BACKUP_DIR | ".cache" | ".talkdnd") {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

fn migrate_sheets(save: &Path, root: &Path) -> Result<Vec<String>> {
    let book = Book::load(&save::ruleset(save), Some(root))?;
    let mut migrated = Vec::new();
    for sheet in check::sheets(save) {
        if charsheet::path(save, &sheet.id).is_file() {
            continue;
        }
        if !crate::chargen::valid_id(&sheet.id) {
            bail!(
                "invalid character id `{}` in {}; cannot migrate it (use lowercase letters, digits and hyphens)",
                sheet.id,
                sheet.file.display()
            );
        }
        let cf = charsheet::migrate(save, &sheet, &book)?;
        charsheet::store(save, &cf)?;
        migrated.push(sheet.id);
    }
    Ok(migrated)
}

/// 2→3: rewrite the YAML files through their stores so they carry `version: 1`
/// (additive; older files still load). Only files untouched since the integrity
/// baseline are rewritten (the original ledger becomes version 2, still
/// additive). Files already dirty in the integrity manifest are left untouched
/// so hand edits, comments and unknown fields are not lost; they stay flagged
/// and version 0 is still accepted.
fn add_versions(save: &Path, dirty: &BTreeSet<String>) -> Result<()> {
    if Progress::path(save).is_file() && !dirty.contains("progress.yaml") {
        let progress = Progress::load(save)?;
        progress.store(save)?;
    }
    if save.join(reveal::FILE).is_file() && !dirty.contains(reveal::FILE) {
        let ledger = reveal::load(save)?;
        reveal::store(save, &ledger)?;
    }
    let original_rel = crate::original::FILE;
    if save.join(original_rel).is_file() && !dirty.contains(original_rel) {
        let ledger = crate::original::load(save)?;
        crate::original::store(save, &ledger)?;
    }
    let party = save.join("combat").join(combat_sim::party::FILE_NAME);
    let party_rel = format!("combat/{}", combat_sim::party::FILE_NAME);
    if party.is_file() && !dirty.contains(&party_rel) {
        let state = PartyState::load(&party)?;
        state.save(&party)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = "# Hero\n\n- **等级**：战士 2\n\n```combat yaml\nid: hero\nname: Hero\nkind: pc\nside: party\nlevel: 2\nac: 16\nhp: 20\nspeed: {walk: 30}\nprof: 2\nabilities: {str: 16, dex: 10, con: 14, int: 10, wis: 10, cha: 12}\nnotes: \"fighter 2\"\n```\n";

    fn root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("talkdnd-migrate-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Saves")).unwrap();
        dir
    }

    fn save_with_sheet(root: &Path) -> PathBuf {
        let save = root.join("Saves/2030-01-01-01");
        for dir in ["Characters", "NPCs", "Sessions", "combat"] {
            fs::create_dir_all(save.join(dir)).unwrap();
        }
        fs::write(
            save.join("combat/campaign.yaml"),
            "name: X\nruleset: \"2024\"\nparty: []\n",
        )
        .unwrap();
        fs::write(save.join("Characters/hero.md"), SHEET).unwrap();
        fs::write(save.join("Sessions/2030-01-01-session-00.md"), "# log\n").unwrap();
        fs::write(save.join("progress.yaml"), "xp: {hero: 0}\n").unwrap();
        save
    }

    fn meta(save: &Path) -> SaveMeta {
        load(save).unwrap().expect("save-meta.yaml")
    }

    #[test]
    fn migrates_a_v1_save_and_is_idempotent() {
        let root = root("v1");
        let save = save_with_sheet(&root);
        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (1, 6));
        assert!(report.deviation_default);
        assert_eq!(report.sheets, vec!["hero".to_string()]);
        assert!(charsheet::path(&save, "hero").is_file());
        assert_eq!(meta(&save).schema, 6);
        assert_eq!(meta(&save).deviation.scope, 0);
        assert_eq!(meta(&save).deviation.ratio, None);
        assert!(meta(&save)
            .migrations
            .iter()
            .any(|m| m.from == 1 && m.to == 2));
        assert!(meta(&save)
            .migrations
            .iter()
            .any(|m| m.from == 3 && m.to == 4));
        assert!(meta(&save)
            .migrations
            .iter()
            .any(|m| m.from == 4 && m.to == 5));

        let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves").join(BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].join("Characters/hero.md").is_file());
        assert!(backups[0].join("progress.yaml").is_file());
        assert!(!backups[0].join("_backups").exists());

        let log = fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md")).unwrap();
        assert!(log.contains("存档结构迁移 schema 1→6"), "{log}");
        assert!(log.contains("Saves/_backups/"), "{log}");
        assert!(combat_sim::integrity::changes(&save).unwrap().is_empty());

        // A second start does nothing.
        let log_lines = log.lines().count();
        let again = ensure(&save, &root, false).unwrap();
        assert!(again.already_current());
        assert!(again.backup.is_none());
        let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves").join(BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        let log = fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md")).unwrap();
        assert_eq!(log.lines().count(), log_lines);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let root = root("dry");
        let save = save_with_sheet(&root);
        let report = ensure(&save, &root, true).unwrap();
        assert_eq!((report.from, report.to), (1, 6));
        assert!(report.dry_run && report.sheets == vec!["hero".to_string()]);
        assert!(!path(&save).is_file());
        assert!(!charsheet::path(&save, "hero").is_file());
        assert!(!root.join("Saves").join(BACKUP_DIR).exists());
        let log = fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md")).unwrap();
        assert!(!log.contains("迁移"), "{log}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pre_edited_file_stays_flagged_after_migration() {
        let root = root("edited");
        let save = root.join("Saves/2030-02-02-01");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(save.join("Sessions/2030-02-02-session-00.md"), "# log\n").unwrap();
        fs::write(
            save.join("progress.yaml"),
            "xp: {hero: 0}\npurses:\n  hero: {cp: 500}\n",
        )
        .unwrap();
        combat_sim::integrity::ensure(&save).unwrap();
        let edited = fs::read_to_string(save.join("progress.yaml"))
            .unwrap()
            .replace("cp: 500", "cp: 900000");
        fs::write(save.join("progress.yaml"), edited).unwrap();

        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (2, 6));
        assert!(meta(&save).schema == 6);
        let changes = combat_sim::integrity::changes(&save).unwrap();
        assert!(
            changes
                .iter()
                .any(|c| c.path() == "progress.yaml" && c.describe().contains("changed")),
            "{changes:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn migration_skips_dirty_ledgers_and_versions_clean_ones() {
        let root = root("dirty-ledger");

        // Clean ledger: the migration rewrites it through the store.
        let clean = root.join("Saves/2030-03-03-01");
        fs::create_dir_all(clean.join("Sessions")).unwrap();
        fs::write(clean.join("Sessions/log.md"), "# log\n").unwrap();
        fs::write(clean.join("progress.yaml"), "xp: {hero: 0}\n").unwrap();
        combat_sim::integrity::ensure(&clean).unwrap();
        ensure(&clean, &root, false).unwrap();
        let text = fs::read_to_string(clean.join("progress.yaml")).unwrap();
        assert!(text.contains("version: 1"), "{text}");

        // Hand-edited ledger: byte-identical, still flagged, version 0 is fine.
        let dirty = root.join("Saves/2030-03-03-02");
        fs::create_dir_all(dirty.join("Sessions")).unwrap();
        fs::write(dirty.join("Sessions/log.md"), "# log\n").unwrap();
        fs::write(dirty.join("progress.yaml"), "xp: {hero: 0}\n").unwrap();
        combat_sim::integrity::ensure(&dirty).unwrap();
        let edited = "# DM comment: keep this line\nxp: {hero: 0}\nhouse-rule: keep me\n";
        fs::write(dirty.join("progress.yaml"), edited).unwrap();
        ensure(&dirty, &root, false).unwrap();
        assert_eq!(
            fs::read_to_string(dirty.join("progress.yaml")).unwrap(),
            edited
        );
        let changes = combat_sim::integrity::changes(&dirty).unwrap();
        assert!(
            changes
                .iter()
                .any(|c| c.path() == "progress.yaml" && c.describe().contains("changed")),
            "{changes:?}"
        );
        // The hand-edited ledger still parses with the old (0) version.
        crate::progress::Progress::load(&dirty).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn deviation_validation() {
        assert_eq!(Deviation::new(Some(0), None).unwrap().ratio, None);
        assert_eq!(Deviation::new(Some(1), Some(5)).unwrap().ratio, Some(5));
        assert_eq!(Deviation::new(Some(2), Some(25)).unwrap().scope, 2);
        assert_eq!(Deviation::new(None, None).unwrap().scope, 0);

        let e = Deviation::new(Some(3), Some(10)).unwrap_err().to_string();
        assert!(e.contains("3") && e.contains("未开放"), "{e}");
        let e = Deviation::new(Some(0), Some(10)).unwrap_err().to_string();
        assert!(e.contains("scope 0") && e.contains("--ratio"), "{e}");
        for ratio in [4, 26, 0, 100] {
            let e = Deviation::new(Some(1), Some(ratio))
                .unwrap_err()
                .to_string();
            assert!(e.contains("5") && e.contains("25"), "{ratio}: {e}");
        }
        assert!(Deviation::new(Some(1), None)
            .unwrap_err()
            .to_string()
            .contains("--ratio"));
        assert!(Deviation::new(Some(2), Some(15)).unwrap().check().is_ok());
        assert!(Deviation {
            scope: 3,
            ratio: Some(10)
        }
        .check()
        .is_err());
    }

    #[test]
    fn migrates_a_v3_save_to_scope_zero_and_logs_it() {
        let root = root("v3");
        let save = root.join("Saves/2030-04-05-01");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(save.join("Sessions/2030-04-05-session-00.md"), "# log\n").unwrap();
        fs::write(
            save.join("save-meta.yaml"),
            "schema: 3\ntool: 0.6.1\nmigrations: []\n",
        )
        .unwrap();
        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (3, 6));
        assert!(report.deviation_default);
        assert!(report
            .lines()
            .iter()
            .any(|l| l.contains("scope 0") && l.contains("偏离度")));
        let m = meta(&save);
        assert_eq!(m.schema, 6);
        assert_eq!(m.deviation.scope, 0);
        assert_eq!(m.deviation.ratio, None);
        assert!(m.migrations.iter().any(|x| x.from == 3 && x.to == 4));
        assert!(m.migrations.iter().any(|x| x.from == 4 && x.to == 5));
        let log = fs::read_to_string(save.join("Sessions/2030-04-05-session-00.md")).unwrap();
        assert!(log.contains("schema 3→6"), "{log}");
        assert!(log.contains("scope 0"), "{log}");
        let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves").join(BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].join("save-meta.yaml").is_file());
        assert!(combat_sim::integrity::changes(&save).unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn original_ledger_is_bumped_to_version_two_additively() {
        let root = root("origv2");
        let save = root.join("Saves/2030-04-07-01");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(save.join("Sessions/s1.md"), "# log\n").unwrap();
        fs::write(
            save.join("save-meta.yaml"),
            "schema: 4\ntool: 0.7.0\ndeviation: {scope: 0, ratio: null}\nmigrations: []\n",
        )
        .unwrap();
        fs::write(
            save.join("original.yaml"),
            "version: 1\nentries:\n  - {id: O-001, type: npc, name: Guide, rarity: common, chapter: 未分章}\n",
        )
        .unwrap();
        ensure(&save, &root, false).unwrap();
        let text = fs::read_to_string(save.join("original.yaml")).unwrap();
        assert!(text.contains("version: 2"), "{text}");
        let ledger = crate::original::load(&save).unwrap();
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(ledger.entries[0].name, "Guide");
        assert!(ledger.entries[0].hidden.is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn migrates_a_v4_save_and_turns_dice_display_on() {
        let root = root("v4");
        let save = root.join("Saves/2030-04-06-01");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(save.join("Sessions/2030-04-06-session-00.md"), "# log\n").unwrap();
        fs::write(
            save.join("save-meta.yaml"),
            "schema: 4\ntool: 0.7.0\ndeviation: {scope: 2, ratio: 15}\nmigrations: []\n",
        )
        .unwrap();
        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (4, 6));
        assert!(!report.deviation_default);
        assert!(report.settings_default);
        let m = meta(&save);
        assert_eq!(m.schema, 6);
        assert_eq!(m.deviation.scope, 2);
        assert_eq!(m.deviation.ratio, Some(15));
        assert!(m.settings.dice_display);
        assert!(m.migrations.iter().any(|x| x.from == 4 && x.to == 5));
        assert!(m.migrations.iter().any(|x| x.from == 5 && x.to == 6));
        assert_eq!(m.migrations.len(), 2);
        let log = fs::read_to_string(save.join("Sessions/2030-04-06-session-00.md")).unwrap();
        assert!(
            log.contains("schema 4→6") && log.contains("骰子显示默认开"),
            "{log}"
        );
        let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves").join(BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(combat_sim::integrity::changes(&save).unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let root = root("newer");
        let save = save_with_sheet(&root);
        fs::write(path(&save), "schema: 99\ntool: future\n").unwrap();
        let err = ensure(&save, &root, false).unwrap_err().to_string();
        assert!(err.contains("99") && err.contains("talkdnd"), "{err}");
        assert!(!root.join("Saves").join(BACKUP_DIR).exists());
        let _ = fs::remove_dir_all(&root);
    }

    fn v5_save(root: &Path, slot: &str) -> PathBuf {
        let save = root.join("Saves").join(slot);
        for d in ["Characters", "NPCs", "Sessions", "combat", "sheets"] {
            fs::create_dir_all(save.join(d)).unwrap();
        }
        fs::write(save.join("Sessions/2030-05-01-session-00.md"), "# log\n").unwrap();
        fs::write(
            save.join("save-meta.yaml"),
            "schema: 5\ntool: 0.8.0\ndeviation: {scope: 0, ratio: null}\nsettings: {dice_display: true}\nmigrations: []\n",
        )
        .unwrap();
        fs::write(
            save.join("combat/campaign.yaml"),
            "characters: [../Characters, ../NPCs]\nparty: [hero, kel, mo]\n",
        )
        .unwrap();
        fs::write(save.join("Characters/Hero.md"), SHEET).unwrap();
        fs::write(
            save.join("sheets/hero.yaml"),
            "# TalkDND character data\nversion: 1\nid: hero\nname: Hero\nfile: Characters/Hero.md\nruleset: '2024'\ncombat:\n  id: hero\n  kind: pc\n  side: party\n",
        )
        .unwrap();
        let kel_md = format!(
            "# Kel——NPC 队友\n\nvoice notes\n\n{}\n\n## 角色选项\n\n- **背景**：—\n\n{}\n\n```combat yaml\nid: kel\nname: Kel\nkind: pc\nside: party\nhp: 9\nac: 13\n```\n",
            charsheet::BEGIN,
            charsheet::END
        );
        fs::write(save.join("NPCs/Kel-Stone.md"), &kel_md).unwrap();
        fs::write(
            save.join("sheets/kel.yaml"),
            "# TalkDND character data\nversion: 1\nid: kel\nname: Kel\nfile: NPCs/Kel-Stone.md\nruleset: '2024'\ninfo:\n  class: cleric\ncombat:\n  id: kel\n  kind: pc\n  side: party\n",
        )
        .unwrap();
        // A companion without structured data yet (party list + party side).
        fs::write(
            save.join("NPCs/mo.md"),
            "# Mo\n\n```combat yaml\nid: mo\nname: Mo\nkind: npc\nside: party\nhp: 8\nac: 12\n```\n",
        )
        .unwrap();
        // Story NPCs stay.
        fs::write(save.join("NPCs/Barkeep.md"), "# Barkeep\n\nGruff.\n").unwrap();
        fs::write(
            save.join("NPCs/guard.md"),
            "# Guard\n\n```combat yaml\nid: guard\nname: Guard\nkind: npc\nside: enemy\nhp: 11\nac: 16\n```\n",
        )
        .unwrap();
        fs::write(
            save.join("NPCs/_party.md"),
            "# 小队\n\n| Hero | `../Characters/Hero.md` |\n| Kel | `Kel-Stone.md` |\n",
        )
        .unwrap();
        combat_sim::integrity::ensure(&save).unwrap();
        save
    }

    #[test]
    fn v5_companions_move_to_characters_and_story_npcs_stay() {
        let root = root("v5");
        let save = v5_save(&root, "2030-05-01-01");
        let kel_before = fs::read_to_string(save.join("NPCs/Kel-Stone.md")).unwrap();

        let dry = ensure(&save, &root, true).unwrap();
        assert_eq!((dry.from, dry.to), (5, 6));
        assert_eq!(dry.companions.len(), 3, "{:?}", dry.companions);
        assert!(save.join("NPCs/Kel-Stone.md").is_file());
        assert!(!root.join("Saves").join(BACKUP_DIR).exists());

        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (5, 6));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        for f in ["Kel-Stone.md", "mo.md", "_party.md"] {
            assert!(save.join("Characters").join(f).is_file(), "{f}");
            assert!(!save.join("NPCs").join(f).exists(), "{f}");
        }
        assert!(save.join("NPCs/Barkeep.md").is_file());
        assert!(save.join("NPCs/guard.md").is_file());
        // Content kept; only the controller line was added to the generated part.
        let kel = fs::read_to_string(save.join("Characters/Kel-Stone.md")).unwrap();
        assert_eq!(
            kel.replace(&charsheet::controller_line(charsheet::DM), ""),
            kel_before
        );
        assert!(kel.contains("- **操控**：DM 操控的同伴"));
        let sheet = fs::read_to_string(save.join("sheets/kel.yaml")).unwrap();
        assert!(sheet.starts_with("# TalkDND character data\n"), "{sheet}");
        assert!(
            sheet.contains("file: Characters/Kel-Stone.md\ncontroller: dm\n"),
            "{sheet}"
        );
        let hero = fs::read_to_string(save.join("sheets/hero.yaml")).unwrap();
        assert!(
            hero.contains("file: Characters/Hero.md\ncontroller: player\n"),
            "{hero}"
        );
        let overview = fs::read_to_string(save.join("Characters/_party.md")).unwrap();
        assert!(overview.contains("`Hero.md`") && overview.contains("`Kel-Stone.md`"));
        // The structured file still loads and resolves.
        let cf: charsheet::CharFile = serde_norway::from_str(&sheet).unwrap();
        assert_eq!(cf.controller(), charsheet::DM);
        assert_eq!(meta(&save).schema, 6);
        assert!(meta(&save)
            .migrations
            .iter()
            .any(|m| m.from == 5 && m.to == 6));
        assert!(combat_sim::integrity::changes(&save).unwrap().is_empty());
        let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves").join(BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].join("NPCs/Kel-Stone.md").is_file());
        let log = fs::read_to_string(save.join("Sessions/2030-05-01-session-00.md")).unwrap();
        assert!(
            log.contains("schema 5→6") && log.contains("Characters/Kel-Stone.md"),
            "{log}"
        );

        // Idempotent: a second run changes nothing.
        let snap = combat_sim::integrity::snapshot(&save);
        let again = ensure(&save, &root, false).unwrap();
        assert!(again.already_current());
        assert_eq!(snap, combat_sim::integrity::snapshot(&save));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn v5_name_collision_keeps_both_and_hand_edits_stay_flagged() {
        let root = root("v5collide");
        let save = v5_save(&root, "2030-05-02-01");
        // A hand edit to the companion sheet before the migration.
        let p = save.join("NPCs/Kel-Stone.md");
        let edited = fs::read_to_string(&p).unwrap().replace("hp: 9", "hp: 99");
        fs::write(&p, edited).unwrap();
        // Something already sits at the target path.
        fs::write(save.join("Characters/Kel-Stone.md"), "# other Kel\n").unwrap();
        fs::write(save.join("Characters/_party.md"), "# old overview\n").unwrap();

        let report = ensure(&save, &root, false).unwrap();
        assert_eq!(report.warnings.len(), 2, "{:?}", report.warnings);
        assert_eq!(
            fs::read_to_string(save.join("Characters/Kel-Stone.md")).unwrap(),
            "# other Kel\n"
        );
        assert!(
            fs::read_to_string(save.join("Characters/Kel-Stone-companion.md"))
                .unwrap()
                .contains("hp: 99")
        );
        assert!(save.join("Characters/_party-companion.md").is_file());
        assert!(fs::read_to_string(save.join("sheets/kel.yaml"))
            .unwrap()
            .contains("file: Characters/Kel-Stone-companion.md"));
        let changes = combat_sim::integrity::changes(&save).unwrap();
        assert!(
            changes
                .iter()
                .any(|c| c.path() == "Characters/Kel-Stone-companion.md"),
            "{changes:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
