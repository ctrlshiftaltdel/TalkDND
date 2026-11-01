//! Save schema metadata (`Saves/<slot>/save-meta.yaml`) and migrations.
//!
//! The schema number describes the on-disk layout of a save:
//! 1 = Markdown sheets only, 2 = structured `sheets/<id>.yaml`, 3 = versioned
//! YAML files plus `save-meta.yaml`, 4 = the locked adventure-deviation
//! settings (`deviation:`) in `save-meta.yaml`, 5 = per-save `settings:`
//! (dice display, default on), 6 = every party member (player characters and
//! DM-run companions) in `Characters/` with a `controller:` field in
//! `sheets/<id>.yaml`; `NPCs/` keeps only story NPCs, 7 = book paths in save
//! files use the one-folder-per-book library (`Adventures/<slug>/`,
//! `Rules/<edition>/<slug>/`, converted index in `index/`; see
//! [`crate::library`]), 8 = per-save DM style settings (`settings.style`:
//! narration, detail, pace, npc_voice, options; see [`Style`] and
//! `LetsPlay/docs/narration.md`), 9 = per-save automation switches
//! (`settings.automation`: fixed categories with `manual` / `assisted` /
//! `auto` levels, save defaults plus scene and encounter overrides; see
//! [`Automation`] and `tools/talkdnd/docs/AUTOMATION.md`), 10 = structured
//! spell lists (`spells` block on every `sheets/<id>.yaml`, adopted precisely
//! from `chargen.json` or the Markdown `## 法术` section, unmatched names kept
//! in `legacy`; hand-edited (dirty) sheets are never rewritten — see
//! `migrate_spells_10` and docs/SPELLCASTING.md §3.6). `talkdnd start` and
//! `talkdnd migrate` back the whole save up into `Saves/_backups/` before
//! writing, then run the migrations in order. A save from a newer talkdnd is
//! refused: this binary cannot know its format.
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
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "save-meta.yaml";
pub const CURRENT_SCHEMA: u32 = 10;
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
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Read dice values, DCs and modifiers aloud. Off = narrate outcomes only;
    /// the programs still roll and log every die (`rolls.log`).
    #[serde(default = "dice_on")]
    pub dice_display: bool,
    /// How the DM narrates this table (schema 8; see [`Style`]). Older saves
    /// migrate to the defaults with the usual backup; only explicit player
    /// feedback ever changes it, never automatic inference.
    #[serde(default)]
    pub style: Style,
    /// Per-save automation switches (schema 9; see [`Automation`]). Older
    /// saves migrate to the charter defaults with the usual backup; unknown
    /// categories, levels or fields are refused when the save loads.
    #[serde(default)]
    pub automation: Automation,
}

fn dice_on() -> bool {
    true
}

/// The DM's narration style, one fixed value per field
/// (`LetsPlay/docs/campaign-continuity.md` §5.2). Defaults follow
/// `LetsPlay/docs/narration.md`: standard beats, standard detail, standard
/// pace, distinct companion voices and 2-4 closing options.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Style {
    /// Length of a narration beat: short | standard | long.
    #[serde(default = "style_standard")]
    pub narration: String,
    /// How much sensory and scene detail: low | standard | high.
    #[serde(default = "style_standard")]
    pub detail: String,
    /// Scene pacing: slow | standard | fast.
    #[serde(default = "style_standard")]
    pub pace: String,
    /// Companion and NPC voices: distinct | neutral.
    #[serde(default = "style_distinct")]
    pub npc_voice: String,
    /// Suggested options at the end of a scene: 2-4 | none.
    #[serde(default = "style_two_four")]
    pub options: String,
}

fn style_standard() -> String {
    "standard".into()
}

fn style_distinct() -> String {
    "distinct".into()
}

fn style_two_four() -> String {
    "2-4".into()
}

impl Default for Style {
    fn default() -> Self {
        Self {
            narration: style_standard(),
            detail: style_standard(),
            pace: style_standard(),
            npc_voice: style_distinct(),
            options: style_two_four(),
        }
    }
}

impl Style {
    /// One-line summary of the five values, e.g. for migration reports.
    pub fn describe(&self) -> String {
        format!(
            "narration={}、detail={}、pace={}、npc_voice={}、options={}",
            self.narration, self.detail, self.pace, self.npc_voice, self.options
        )
    }
}

// ---------------------------------------------------------------------------
// Automation switches (schema 9)
// ---------------------------------------------------------------------------

/// The three fixed automation levels, weakest first: `manual`, `assisted`,
/// `auto`. `auto` performs the action without asking; `assisted` pauses and
/// asks or warns; `manual` waits for an explicit request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Manual,
    Assisted,
    Auto,
}

pub const LEVELS: [&str; 3] = ["manual", "assisted", "auto"];

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Assisted => "assisted",
            Self::Auto => "auto",
        }
    }

    pub fn zh(self) -> &'static str {
        match self {
            Self::Manual => "手动",
            Self::Assisted => "辅助",
            Self::Auto => "自动",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "manual" => Ok(Self::Manual),
            "assisted" => Ok(Self::Assisted),
            "auto" => Ok(Self::Auto),
            other => bail!("自动化级别只能是 manual、assisted、auto（收到 `{other}`）；存档未改动"),
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Level {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Level::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// One fixed, documented automation category. The default follows the
/// charter's automation principles: no-choice resolution defaults to `auto`,
/// player choices and reactions default to `assisted` and may never be `auto`,
/// and near-limit conditions warn instead of resolving silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutomationCategory {
    pub name: &'static str,
    pub zh: &'static str,
    pub default: Level,
    /// Player-owned decisions: `auto` is refused for this category.
    pub player_owned: bool,
    pub note: &'static str,
}

pub const AUTOMATION_CATEGORIES: [AutomationCategory; 6] = [
    AutomationCategory {
        name: "resolution",
        zh: "结算",
        default: Level::Auto,
        player_owned: false,
        note: "检定、伤害、HP/资源记账等没有玩家选择的结算",
    },
    AutomationCategory {
        name: "combat",
        zh: "战斗流程",
        default: Level::Auto,
        player_owned: false,
        note: "先攻、怪物回合、条件与移动等战斗流程",
    },
    AutomationCategory {
        name: "npc",
        zh: "NPC 对话",
        default: Level::Assisted,
        player_owned: false,
        note: "NPC 的台词、语气与临场反应，DM 演绎后可被玩家打断",
    },
    AutomationCategory {
        name: "player_choice",
        zh: "玩家选择",
        default: Level::Assisted,
        player_owned: true,
        note: "属于玩家角色的决定与选项，只能询问或手动，不能自动代做",
    },
    AutomationCategory {
        name: "reaction",
        zh: "玩家反应",
        default: Level::Assisted,
        player_owned: true,
        note: "护盾术、幸运、激励、借机攻击等玩家反应，只能询问或手动",
    },
    AutomationCategory {
        name: "near_limit",
        zh: "接近上限",
        default: Level::Assisted,
        player_owned: false,
        note: "HP 见底、法术位/资源将尽、力竭或负重接近上限时先警告",
    },
];

pub fn automation_category(name: &str) -> Option<&'static AutomationCategory> {
    AUTOMATION_CATEGORIES.iter().find(|c| c.name == name)
}

fn automation_version() -> u32 {
    1
}

/// Per-save automation settings: one save-wide default per category plus
/// optional partial overrides keyed by scene id and encounter id. Effective
/// resolution order is encounter > scene > save default.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Automation {
    /// On-disk schema of this block. Only version 1 is understood.
    #[serde(default = "automation_version")]
    pub version: u32,
    /// Save-wide default level per category. Missing categories fall back to
    /// [`AutomationCategory::default`]; unknown categories and levels fail.
    #[serde(default, deserialize_with = "de_level_map")]
    pub default: BTreeMap<String, Level>,
    /// Scene overrides, keyed by scene id. Each map holds only the categories
    /// it overrides.
    #[serde(default, deserialize_with = "de_override_map")]
    pub scenes: BTreeMap<String, BTreeMap<String, Level>>,
    /// Encounter overrides, keyed by encounter id, with the same shape.
    #[serde(default, deserialize_with = "de_override_map")]
    pub encounters: BTreeMap<String, BTreeMap<String, Level>>,
}

impl Default for Automation {
    fn default() -> Self {
        let mut automation = Self {
            version: 1,
            default: BTreeMap::new(),
            scenes: BTreeMap::new(),
            encounters: BTreeMap::new(),
        };
        automation.fill_defaults();
        automation
    }
}

impl Automation {
    /// Write the charter default for every category that has no explicit
    /// save default yet. Existing values and overrides are preserved.
    pub fn fill_defaults(&mut self) {
        for category in AUTOMATION_CATEGORIES {
            self.default
                .entry(category.name.to_string())
                .or_insert(category.default);
        }
    }

    pub fn default_level(&self, category: &str) -> Level {
        self.default
            .get(category)
            .copied()
            .or_else(|| automation_category(category).map(|c| c.default))
            .unwrap_or(Level::Assisted)
    }

    /// Validate a block read from disk or built by the CLI. Unknown
    /// categories, unknown levels, illegal ids, empty override maps and
    /// `auto` on player-owned categories are refused with an actionable
    /// error; nothing is silently dropped.
    pub fn check(&self) -> Result<()> {
        if self.version != 1 {
            bail!(
                "automation.version 只能是 1（收到 {}）；请更新 talkdnd 后再打开这个存档",
                self.version
            );
        }
        check_levels("存档默认", &self.default, false)?;
        for (id, levels) in &self.scenes {
            check_id("场景", id)?;
            if levels.is_empty() {
                bail!("场景 `{id}` 的自动化覆盖为空；请删除这个覆盖或写入至少一个类别");
            }
            check_levels(&format!("场景 {id}"), levels, true)?;
        }
        for (id, levels) in &self.encounters {
            check_id("遭遇", id)?;
            if levels.is_empty() {
                bail!("遭遇 `{id}` 的自动化覆盖为空；请删除这个覆盖或写入至少一个类别");
            }
            check_levels(&format!("遭遇 {id}"), levels, true)?;
        }
        Ok(())
    }
}

fn check_levels(scope: &str, levels: &BTreeMap<String, Level>, allow_partial: bool) -> Result<()> {
    for (name, level) in levels {
        let Some(category) = automation_category(name) else {
            bail!(
                "自动化类别只能是 {}（收到 `{name}`，位置：{scope}）；存档未改动",
                AUTOMATION_CATEGORIES
                    .iter()
                    .map(|c| c.name)
                    .collect::<Vec<_>>()
                    .join("、")
            );
        };
        if category.player_owned && *level == Level::Auto {
            bail!(
                "{} 不能设为 auto：玩家选择/反应绝不能自动代做（可用 assisted 或 manual）；存档未改动",
                category.name
            );
        }
    }
    if !allow_partial && levels.is_empty() {
        // An explicitly empty save-default map is not a valid document: every
        // category has a charter default, and migration writes them all.
        bail!("automation.default 不能为空；请运行 `talkdnd migrate` 或写入六个类别");
    }
    Ok(())
}

pub(crate) fn check_id(kind: &str, id: &str) -> Result<()> {
    let trimmed = id.trim();
    let valid_len = (1..=80).contains(&trimmed.chars().count());
    let valid_chars = trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid_len || !valid_chars || trimmed == "." || trimmed == ".." {
        bail!("{kind} id 只能是 1-80 个字母、数字、-、_ 或 .（收到 `{id}`）；存档未改动");
    }
    Ok(())
}

fn de_level_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, Level>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct LevelMapVisitor;

    impl<'de> serde::de::Visitor<'de> for LevelMapVisitor {
        type Value = BTreeMap<String, Level>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a mapping of automation category to manual/assisted/auto")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut out = BTreeMap::new();
            while let Some((name, level)) = map.next_entry::<String, Level>()? {
                if out.insert(name.clone(), level).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "自动化类别 `{name}` 重复；存档未改动"
                    )));
                }
            }
            Ok(out)
        }
    }

    deserializer.deserialize_map(LevelMapVisitor)
}

#[derive(Default)]
struct LevelMap(BTreeMap<String, Level>);

impl<'de> Deserialize<'de> for LevelMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        de_level_map(deserializer).map(Self)
    }
}

fn de_override_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, BTreeMap<String, Level>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OverrideVisitor;

    impl<'de> serde::de::Visitor<'de> for OverrideVisitor {
        type Value = BTreeMap<String, BTreeMap<String, Level>>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a mapping of scene/encounter id to category levels")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut out = BTreeMap::new();
            while let Some((id, levels)) = map.next_entry::<String, LevelMap>()? {
                if out.insert(id.clone(), levels.0).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "自动化覆盖 id `{id}` 重复；存档未改动"
                    )));
                }
            }
            Ok(out)
        }
    }

    deserializer.deserialize_map(OverrideVisitor)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dice_display: true,
            style: Style::default(),
            automation: Automation::default(),
        }
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
    meta.settings
        .automation
        .check()
        .with_context(|| format!("invalid {}", p.display()))?;
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
    // Fail closed on any programmatic write too, not only on serde load.
    meta.settings.automation.check()?;
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
    /// 7→8: the old save had no style; the narration defaults are written.
    pub style_default: bool,
    /// 8→9: the old save had no automation switches; charter defaults are written.
    pub automation_default: bool,
    /// 5→6: companion sheets moved from NPCs/ to Characters/ ("old → new").
    pub companions: Vec<String>,
    /// 5→6: sheets/<id>.yaml files that got a `controller:` field ("id=controller").
    pub controllers: Vec<String>,
    /// 5→6: anything the DM should look at (name collisions, ...).
    pub warnings: Vec<String>,
    /// 6→7: save files whose book paths were repointed to the
    /// one-folder-per-book library (paths relative to the save).
    pub books: Vec<String>,
    /// 9→10: sheets that got a `spells` block ("id: summary" lines).
    pub spell_sheets: Vec<String>,
    /// 9→10: sheets that already carried a block (defaults filled only).
    pub spell_kept: Vec<String>,
    /// 9→10: dirty sheets left untouched (they stay flagged and keep their
    /// content byte-for-byte; listed so the DM can adopt them by hand).
    pub spell_dirty: Vec<String>,
    /// 9→10: free-text entries that could not be mapped to ids ("id=名字").
    pub spell_legacy: Vec<String>,
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
            if self.style_default {
                v.push(format!(
                    "会写入桌面风格：{}（旧存档没有设置过；`talkdnd settings style show` 查看）",
                    Style::default().describe()
                ));
            }
            if self.automation_default {
                v.push(
                    "会写入自动化设置：按章程默认（结算/战斗流程 auto，NPC/玩家选择/反应/接近上限 assisted；`talkdnd settings automation` 查看）"
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
            if !self.books.is_empty() {
                v.push(format!(
                    "会把书本路径改到一书一文件夹的新位置：{}",
                    self.books.join(", ")
                ));
            }
            if !self.spell_sheets.is_empty() {
                v.push(format!(
                    "会为角色卡添加 spells 法术列表块：{}",
                    self.spell_sheets.join("，")
                ));
            }
            if !self.spell_kept.is_empty() {
                v.push(format!(
                    "已有 spells 块的角色卡只补缺省字段（列表不动）：{}",
                    self.spell_kept.join("，")
                ));
            }
            if !self.spell_dirty.is_empty() {
                v.push(format!(
                    "手改过的角色卡不会被重写（保持原样、保持标记，法术列表之后用命令补）：{}",
                    self.spell_dirty.join("，")
                ));
            }
            if !self.spell_legacy.is_empty() {
                v.push(format!(
                    "无法映射到法术 id 的旧条目会进 legacy：{}",
                    self.spell_legacy.join("，")
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
        if self.style_default {
            v.push(format!(
                "桌面风格：默认已写入（{}；`talkdnd settings style set <字段> <值> --reason <原因>` 修改）",
                Style::default().describe()
            ));
        }
        if self.automation_default {
            v.push(
                "自动化设置：章程默认已写入（结算/战斗流程 auto，NPC/玩家选择/反应/接近上限 assisted；`talkdnd settings automation` 查看与修改）"
                    .into(),
            );
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
        if !self.books.is_empty() {
            v.push(format!(
                "书本路径已改到一书一文件夹的新位置：{}",
                self.books.join(", ")
            ));
        }
        if !self.spell_sheets.is_empty() {
            v.push(format!(
                "已为角色卡添加 spells 法术列表块：{}",
                self.spell_sheets.join("；")
            ));
        }
        if !self.spell_kept.is_empty() {
            v.push(format!(
                "已有 spells 块的角色卡只补了缺省字段（列表未动）：{}",
                self.spell_kept.join("，")
            ));
        }
        if !self.spell_dirty.is_empty() {
            v.push(format!(
                "手改过的角色卡未被重写（保持原样，法术列表之后用命令补）：{}",
                self.spell_dirty.join("，")
            ));
        }
        if !self.spell_legacy.is_empty() {
            v.push(format!(
                "无法映射到法术 id 的旧条目已进 legacy：{}",
                self.spell_legacy.join("；")
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
        style_default: from < 8,
        automation_default: from < 9,
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
        if from < 7 {
            report.books = repoint_names(save, &crate::library::plan_repoint(root, save));
        }
        if from < 10 {
            let no_baseline = combat_sim::integrity::load(save)?.is_none();
            let dirty = integrity_dirty(save)?;
            let mut plan = plan_spells_10(save, root, &dirty)?;
            if no_baseline && !plan.dirty.is_empty() {
                plan.warnings.push(no_baseline_warning());
            }
            report.spell_sheets = plan.sheets.iter().map(|(id, _)| id.clone()).collect();
            report.spell_kept = plan.kept;
            report.spell_dirty = plan.dirty;
            report.spell_legacy = plan
                .legacy
                .iter()
                .map(|(id, name)| format!("{id}={name}"))
                .collect();
            report.warnings.extend(plan.warnings);
        }
        return Ok(report);
    }

    // Snapshot before any write: after the migration only files that were
    // clean before are recorded, so pre-existing hand edits stay flagged.
    let before = combat_sim::integrity::snapshot(save);
    // Files the DM already edited (or added/removed) are not rewritten by the
    // migration: a serde round-trip would drop unknown fields, comments and
    // formatting. They stay on disk exactly as they are and keep loading
    // because version 0 is accepted. Sheets stay untouched too (schema 10:
    // see `migrate_spells_10`) — a serde round-trip would drop their hand
    // edits just the same.
    let manifest = combat_sim::integrity::load(save)?;
    let mut dirty: BTreeSet<String> = match &manifest {
        Some(manifest) => combat_sim::integrity::changes_against(manifest, &before)
            .into_iter()
            .map(|change| change.path().to_string())
            .collect(),
        None => BTreeSet::new(),
    };
    if manifest.is_none() {
        // Fail closed for the sheet rewrite (schema 10): without a
        // baseline every sheet counts as hand-edited and keeps its bytes
        // (a serde round-trip would drop unknown fields and comments).
        dirty.extend(sheet_yaml_paths(save));
    }
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

    if from < 7 {
        let plan = crate::library::plan_repoint(root, save);
        report.books = repoint_names(save, &plan);
        apply_repoint(&plan)?;
    }

    if from < 10 {
        let mut plan = plan_spells_10(save, root, &dirty)?;
        if manifest.is_none() && !plan.dirty.is_empty() {
            plan.warnings.push(no_baseline_warning());
        }
        apply_spells_10(save, &plan)?;
        report.spell_sheets = plan.sheets.iter().map(|(id, _)| id.clone()).collect();
        report.spell_kept = plan.kept;
        report.spell_dirty = plan.dirty;
        report.spell_legacy = plan
            .legacy
            .iter()
            .map(|(id, name)| format!("{id}={name}"))
            .collect();
        report.warnings.extend(plan.warnings);
    }

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
    if from < 7 {
        meta.migrations.push(Migration {
            from: from.max(6),
            to: 7,
            date: date.clone(),
            backup: backup.clone(),
        });
    }
    if from < 8 {
        meta.migrations.push(Migration {
            from: from.max(7),
            to: 8,
            date: date.clone(),
            backup: backup.clone(),
        });
    }
    if from < 9 {
        // Write every charter default explicitly; a partial block that was
        // already present keeps its values and gets the missing categories.
        meta.settings.automation.fill_defaults();
        meta.migrations.push(Migration {
            from: from.max(8),
            to: 9,
            date: date.clone(),
            backup: backup.clone(),
        });
    }
    if from < 10 {
        meta.migrations.push(Migration {
            from: from.max(9),
            to: 10,
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
    let style_note = if from < 8 {
        "；桌面风格默认 standard/distinct/2-4"
    } else {
        ""
    };
    let automation_note = if from < 9 {
        "；自动化设置默认（结算/战斗流程 auto，其余 assisted）"
    } else {
        ""
    };
    let companion_note = if report.companions.is_empty() {
        String::new()
    } else {
        format!("；队友移到 Characters/：{}", report.companions.join("，"))
    };
    let books_note = if report.books.is_empty() {
        String::new()
    } else {
        format!("；书本路径改到一书一文件夹：{}", report.books.join("，"))
    };
    let spells_note = if from < 10 {
        "；角色卡新增 spells 法术列表块"
    } else {
        ""
    };
    let text = format!(
        "[{}] 存档结构迁移 schema {from}→{CURRENT_SCHEMA}（备份：{backup}）{deviation_note}{settings_note}{style_note}{automation_note}{companion_note}{books_note}{spells_note}",
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

// ---------- 6→7: book paths follow the one-folder-per-book library ----------

fn repoint_names(save: &Path, plan: &[(PathBuf, String)]) -> Vec<String> {
    plan.iter()
        .map(|(p, _)| {
            p.strip_prefix(save)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

fn apply_repoint(plan: &[(PathBuf, String)]) -> Result<()> {
    for (p, text) in plan {
        atomic_write(p, text.as_bytes())?;
    }
    Ok(())
}

/// Keep a schema-7 save's book paths in step with later library tidies
/// (`talkdnd start` runs it after [`ensure`]): when a book moved since the
/// save was written, back the save up, rewrite the paths (text only) and
/// refresh the integrity fingerprints of files that were clean before.
/// `dry_run` only reports. Returns digest lines (empty when nothing changed).
pub fn repoint_books(save: &Path, root: &Path, dry_run: bool) -> Result<Vec<String>> {
    if detect(save)? < CURRENT_SCHEMA {
        return Ok(Vec::new());
    }
    let plan = crate::library::plan_repoint(root, save);
    if plan.is_empty() {
        return Ok(Vec::new());
    }
    let names = repoint_names(save, &plan).join("，");
    if dry_run {
        return Ok(vec![format!(
            "会更新书本路径（书已整理到新文件夹）：{names}"
        )]);
    }
    let before = combat_sim::integrity::snapshot(save);
    let slot = save
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "save".into());
    let backup = backup_save(save, root, &slot, CURRENT_SCHEMA)?;
    apply_repoint(&plan)?;
    let text = format!(
        "[{}] 书本路径更新（书库整理后，备份：{backup}）：{names}",
        chrono::Local::now().format("%H:%M")
    );
    if let Some(log) = save::latest_log(save) {
        save::append(&log, &format!("- {text}"))?;
    }
    combat_sim::integrity::after_write(save, &before)?;
    Ok(vec![
        format!("书本路径已更新：{names}"),
        format!("更新前整份备份：`{backup}`"),
    ])
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
    carry_dirty_flags_guarded(save, moved, dirty, None)
}

/// [`carry_dirty_flags`] with a test-only fault hook (production always
/// passes `None`): it runs once the manifest lock is held and just before
/// the manifest is read, so a test can commit a competing writer's update
/// at the point the lock has made impossible to interleave with the read
/// (mirrors `after_write_guarded` in `combat_sim::integrity`).
fn carry_dirty_flags_guarded(
    save: &Path,
    moved: &[Move],
    dirty: &BTreeSet<String>,
    fault: Option<&dyn Fn()>,
) -> Result<()> {
    // The manifest read-modify-write runs under the integrity manifest lock:
    // after_write (above) has released it by the time this runs, so a
    // concurrent writer may have committed in between and this in-lock load
    // must pick its update up instead of overwriting it with a copy read
    // earlier (issue #154). `dirty` is an input — the paths that were
    // already flagged before the migration wrote anything — not a manifest
    // copy.
    let _lock = combat_sim::integrity::lock(save)?;
    if let Some(f) = fault {
        f();
    }
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
    let book = Book::load(&save::edition(save)?, Some(root))?;
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

// ---------- 9→10: structured spell lists ----------

/// Relative `sheets/*.yaml` paths, sorted. Serves as the fail-safe dirty
/// set when a save carries no integrity manifest at all.
fn sheet_yaml_paths(save: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(rd) = fs::read_dir(save.join("sheets")) else {
        return out;
    };
    for p in rd.filter_map(|e| e.ok().map(|e| e.path())) {
        if p.extension().is_some_and(|x| x == "yaml") {
            if let Some(stem) = p.file_stem() {
                out.insert(format!("sheets/{}.yaml", stem.to_string_lossy()));
            }
        }
    }
    out
}

/// Files the integrity manifest flags as changed (or added/removed) against
/// its baseline. Read-only; shared by the dry-run plan and the migration.
/// Fail-safe: with no manifest there is no way to tell hand edits from tool
/// writes, so every sheet counts as dirty and the schema-10 step reports it
/// instead of serde-rewriting (a round-trip would drop unknown fields,
/// comments and formatting).
fn integrity_dirty(save: &Path) -> Result<BTreeSet<String>> {
    let before = combat_sim::integrity::snapshot(save);
    Ok(match combat_sim::integrity::load(save)? {
        Some(manifest) => combat_sim::integrity::changes_against(&manifest, &before)
            .into_iter()
            .map(|change| change.path().to_string())
            .collect(),
        None => sheet_yaml_paths(save),
    })
}

/// Warning for saves without an integrity baseline: the schema-10 spells
/// step skipped every sheet.
fn no_baseline_warning() -> String {
    "没有 integrity 基线：所有 sheets/*.yaml 视为手改，spells 迁移已跳过（不重写）；".to_string()
        + "先运行 `talkdnd check` 建立基线，再重跑迁移"
}

/// The planned schema-10 `spells` blocks for every structured sheet.
#[derive(Default)]
struct SpellsPlan {
    /// (sheet id, CharFile with the block set) for sheets to write.
    sheets: Vec<(String, charsheet::CharFile)>,
    /// Sheets that already carried a block (defaults filled only).
    kept: Vec<String>,
    /// Dirty sheets: never rewritten, reported instead.
    dirty: Vec<String>,
    /// (sheet id, unmapped free-text name) pairs, kept verbatim in `legacy`.
    legacy: Vec<(String, String)>,
    warnings: Vec<String>,
}

/// Plan the 9→10 spells migration (read-only). For every `sheets/*.yaml`:
/// dirty sheets are listed and skipped (a serde round-trip would drop their
/// hand edits); sheets with a block only get missing defaults; the rest get
/// a block adopted precisely (D-9) from `chargen.json` when it belongs to
/// this sheet, else from the Markdown `## 法术` section — exact id or
/// slug-of-name only, everything else lands in `legacy`. Combat blocks and
/// refs are never touched.
fn plan_spells_10(save: &Path, root: &Path, dirty: &BTreeSet<String>) -> Result<SpellsPlan> {
    let mut plan = SpellsPlan::default();
    let creation = chargen_state(save);
    // Sheets per class: the free text in chargen.json may only be adopted
    // when it unambiguously belongs to one sheet.
    let mut class_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut files: Vec<PathBuf> = fs::read_dir(save.join("sheets"))
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.sort();
    for f in &files {
        let class = fs::read_to_string(f)
            .ok()
            .and_then(|t| serde_norway::from_str::<charsheet::CharFile>(&t).ok())
            .and_then(|cf| cf.info.get("class").cloned())
            .map(|c| crate::chargen::slug(&c));
        if let Some(c) = class {
            *class_counts.entry(c).or_default() += 1;
        }
    }
    let mut books: BTreeMap<String, crate::classdata::Book> = BTreeMap::new();
    for f in files {
        if f.extension().is_none_or(|x| x != "yaml") {
            continue;
        }
        let Some(id) = f.file_stem().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        let rel = format!("sheets/{id}.yaml");
        if dirty.contains(&rel) {
            plan.dirty.push(id);
            continue;
        }
        let text = fs::read_to_string(&f)?;
        let mut cf: charsheet::CharFile = match serde_norway::from_str(&text) {
            Ok(cf) => cf,
            Err(e) => {
                plan.warnings
                    .push(format!("{id}: 角色卡无法解析（{e}），已跳过，请人工处理"));
                continue;
            }
        };
        if let Some(existing) = &mut cf.spells {
            existing.fill_defaults();
            plan.kept.push(id.clone());
            plan.sheets.push((id, cf));
            continue;
        }
        let rs = cf.ruleset.clone();
        let class = cf.info.get("class").map(|c| crate::chargen::slug(c));
        let book = match books.entry(rs.clone()) {
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert(crate::classdata::Book::load(&rs, Some(root))?);
                books.get(&rs).expect("just inserted")
            }
            std::collections::btree_map::Entry::Occupied(o) => o.into_mut(),
        };
        let cd = class.as_deref().and_then(|c| book.class(c));
        if cd.is_none() {
            if let Some(cls) = &class {
                plan.warnings.push(format!(
                    "{id}: class `{cls}` 没有职业数据（{rs}），spells 块留空；本地数据缺失时先导入再重跑迁移"
                ));
            }
        }
        let subclass = cd.and_then(|cd| crate::charsheet::subclass_of_cf(&cf, cd));
        let (mode, ability, universe) = match cd {
            Some(cd) => (
                crate::classdata::spell_mode(cd).unwrap_or_default(),
                cd.spell_ability
                    .clone()
                    .or_else(|| {
                        class
                            .as_deref()
                            .and_then(crate::data::class)
                            .and_then(|c| c.spell_ability)
                            .map(str::to_string)
                    })
                    .unwrap_or_default(),
                crate::classdata::spell_universe(book, cd, subclass.as_deref(), cf.level(), None),
            ),
            None => (String::new(), String::new(), BTreeMap::new()),
        };
        // Adopt source: chargen.json when it unambiguously belongs to this
        // sheet, else the Markdown 法术 section. Old spell-slot resources and
        // the Markdown body itself stay untouched.
        let texts = chargen_spells_for(&creation, &class, &class_counts)
            .unwrap_or_else(|| crate::chargen::markdown_spells(&cf.body_md));
        let (mapped, unmapped) = crate::chargen::map_spell_texts(&texts, &universe);
        let mut lists = charsheet::SpellLists {
            version: 1,
            mode: mode.clone(),
            ability,
            ..Default::default()
        };
        for sid in mapped {
            let level = universe.get(&sid).copied().unwrap_or(0);
            if level == 0 {
                lists.cantrips.push(sid);
            } else {
                match mode.as_str() {
                    "known" => lists.known.push(sid),
                    "spellbook" => lists.book.push(sid),
                    _ => lists.prepared.push(sid),
                }
            }
        }
        for name in &unmapped {
            plan.legacy.push((id.clone(), name.clone()));
        }
        lists.legacy = unmapped;
        cf.spells = Some(lists);
        plan.sheets.push((id, cf));
    }
    Ok(plan)
}

/// `chargen.json` if it exists and parses; `None` otherwise (it is an
/// optional adopt source, never a hard requirement).
fn chargen_state(save: &Path) -> Option<crate::chargen::Creation> {
    let p = crate::chargen::state_path(save);
    if !p.is_file() {
        return None;
    }
    serde_json::from_str(&fs::read_to_string(&p).ok()?).ok()
}

/// The free-text spells in `chargen.json`, but only when they unambiguously
/// belong to this sheet: same class, and exactly one sheet of that class in
/// the save (otherwise the Markdown is used instead).
fn chargen_spells_for(
    creation: &Option<crate::chargen::Creation>,
    class: &Option<String>,
    class_counts: &BTreeMap<String, usize>,
) -> Option<Vec<String>> {
    let c = creation.as_ref()?;
    if c.spells.is_empty() {
        return None;
    }
    let cls = class.as_deref()?;
    if c.class.as_deref() != Some(cls) {
        return None;
    }
    if class_counts.get(cls).copied() != Some(1) {
        return None;
    }
    Some(c.spells.clone())
}

/// Write the planned blocks. Per-sheet atomic; a failure leaves the save
/// consistent (schema stays 9 until the meta write, so a re-run converges:
/// already-written sheets take the `kept` path).
fn apply_spells_10(save: &Path, plan: &SpellsPlan) -> Result<()> {
    for (_, cf) in &plan.sheets {
        charsheet::store(save, cf)?;
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
        assert_eq!((report.from, report.to), (1, CURRENT_SCHEMA));
        assert!(report.deviation_default);
        assert_eq!(report.sheets, vec!["hero".to_string()]);
        assert!(charsheet::path(&save, "hero").is_file());
        assert_eq!(meta(&save).schema, CURRENT_SCHEMA);
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
        assert!(
            log.contains(&format!("存档结构迁移 schema 1→{CURRENT_SCHEMA}")),
            "{log}"
        );
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
        assert_eq!((report.from, report.to), (1, CURRENT_SCHEMA));
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
        assert_eq!((report.from, report.to), (2, CURRENT_SCHEMA));
        assert!(meta(&save).schema == CURRENT_SCHEMA);
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
        assert_eq!((report.from, report.to), (3, CURRENT_SCHEMA));
        assert!(report.deviation_default);
        assert!(report
            .lines()
            .iter()
            .any(|l| l.contains("scope 0") && l.contains("偏离度")));
        let m = meta(&save);
        assert_eq!(m.schema, CURRENT_SCHEMA);
        assert_eq!(m.deviation.scope, 0);
        assert_eq!(m.deviation.ratio, None);
        assert!(m.migrations.iter().any(|x| x.from == 3 && x.to == 4));
        assert!(m.migrations.iter().any(|x| x.from == 4 && x.to == 5));
        let log = fs::read_to_string(save.join("Sessions/2030-04-05-session-00.md")).unwrap();
        assert!(log.contains(&format!("schema 3→{CURRENT_SCHEMA}")), "{log}");
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
        assert_eq!((report.from, report.to), (4, CURRENT_SCHEMA));
        assert!(!report.deviation_default);
        assert!(report.settings_default);
        assert!(report.style_default);
        let m = meta(&save);
        assert_eq!(m.schema, CURRENT_SCHEMA);
        assert_eq!(m.deviation.scope, 2);
        assert_eq!(m.deviation.ratio, Some(15));
        assert!(m.settings.dice_display);
        assert_eq!(m.settings.style, Style::default());
        assert!(m.migrations.iter().any(|x| x.from == 4 && x.to == 5));
        assert!(m.migrations.iter().any(|x| x.from == 5 && x.to == 6));
        assert!(m.migrations.iter().any(|x| x.from == 6 && x.to == 7));
        assert!(m.migrations.iter().any(|x| x.from == 7 && x.to == 8));
        assert!(m.migrations.iter().any(|x| x.from == 8 && x.to == 9));
        assert!(m.migrations.iter().any(|x| x.from == 9 && x.to == 10));
        assert_eq!(m.migrations.len(), 6);
        assert_eq!(m.settings.automation, Automation::default());
        let log = fs::read_to_string(save.join("Sessions/2030-04-06-session-00.md")).unwrap();
        assert!(
            log.contains(&format!("schema 4→{CURRENT_SCHEMA}")) && log.contains("骰子显示默认开"),
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
        assert_eq!((dry.from, dry.to), (5, CURRENT_SCHEMA));
        assert_eq!(dry.companions.len(), 3, "{:?}", dry.companions);
        assert!(save.join("NPCs/Kel-Stone.md").is_file());
        assert!(!root.join("Saves").join(BACKUP_DIR).exists());

        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (5, CURRENT_SCHEMA));
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
        // Schema 10 stores the sheet through the canonical writer.
        assert!(sheet.starts_with("# TalkDND character data"), "{sheet}");
        assert!(sheet.contains("spells:"), "{sheet}");
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
        assert_eq!(meta(&save).schema, CURRENT_SCHEMA);
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
            log.contains(&format!("schema 5→{CURRENT_SCHEMA}"))
                && log.contains("Characters/Kel-Stone.md"),
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

    #[test]
    fn v6_book_paths_follow_the_library_and_later_moves() {
        let root = root("v6books");
        let save = root.join("Saves/slot");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(save.join("Sessions/2030-05-01-session-00.md"), "# log\n").unwrap();
        fs::write(
            save.join("save-meta.yaml"),
            "schema: 6\ntool: 0.9.0\ndeviation: {scope: 0, ratio: null}\nsettings: {dice_display: true}\nmigrations: []\n",
        )
        .unwrap();
        fs::write(
            save.join("combat/campaign.yaml"),
            "adventures: [../../../Adventures/keys-from-the-golden-vault/combat]\n",
        )
        .unwrap();
        fs::write(
            save.join("dm-plan.md"),
            "See `Adventures/keys-from-the-golden-vault/.md/Keys from the Golden Vault/index.md`.\n",
        )
        .unwrap();
        let dry = ensure(&save, &root, true).unwrap();
        assert_eq!((dry.from, dry.to), (6, CURRENT_SCHEMA));
        assert_eq!(dry.books, vec!["dm-plan.md".to_string()]);
        let report = ensure(&save, &root, false).unwrap();
        assert_eq!(report.books, vec!["dm-plan.md".to_string()]);
        assert!(report.backup.is_some());
        assert!(report.style_default);
        let plan = fs::read_to_string(save.join("dm-plan.md")).unwrap();
        assert!(
            plan.contains("Adventures/keys-from-the-golden-vault/index/index.md"),
            "{plan}"
        );
        // campaign.yaml already pointed at the right folder: untouched.
        assert!(fs::read_to_string(save.join("combat/campaign.yaml"))
            .unwrap()
            .contains("Adventures/keys-from-the-golden-vault/combat"));
        assert_eq!(meta(&save).schema, CURRENT_SCHEMA);
        assert!(meta(&save)
            .migrations
            .iter()
            .any(|m| m.from == 6 && m.to == 7));
        // Nothing moved since: repoint is a no-op.
        assert!(repoint_books(&save, &root, false).unwrap().is_empty());
        // A later tidy renames the folder: the save follows on the next start.
        fs::create_dir_all(root.join(".talkdnd")).unwrap();
        fs::write(
            root.join(crate::library::MOVES_FILE),
            "moves:\n- from: Adventures/keys-from-the-golden-vault/\n  to: Adventures/golden-vault/\n",
        )
        .unwrap();
        assert_eq!(crate::library::load_moves(&root).len(), 1);
        let lines = repoint_books(&save, &root, true).unwrap();
        assert!(lines[0].contains("combat/campaign.yaml"), "{lines:?}");
        let lines = repoint_books(&save, &root, false).unwrap();
        assert!(lines.iter().any(|l| l.contains("备份")), "{lines:?}");
        assert!(fs::read_to_string(save.join("combat/campaign.yaml"))
            .unwrap()
            .contains("Adventures/golden-vault/combat"));
        assert!(repoint_books(&save, &root, false).unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    fn v7_save(root: &Path, slot: &str) -> PathBuf {
        let save = root.join("Saves").join(slot);
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::write(save.join("Sessions/2030-06-01-session-00.md"), "# log\n").unwrap();
        fs::write(
            save.join("save-meta.yaml"),
            "schema: 7\ntool: 0.11.0\ndeviation: {scope: 0, ratio: null}\nsettings: {dice_display: false}\nmigrations: []\n",
        )
        .unwrap();
        save
    }

    #[test]
    fn migrates_a_v7_save_with_default_style_backup_and_log() {
        let root = root("v7");
        let save = v7_save(&root, "2030-06-01-01");

        let dry = ensure(&save, &root, true).unwrap();
        assert_eq!((dry.from, dry.to), (7, CURRENT_SCHEMA));
        assert!(dry.style_default);
        assert!(!path(&save).is_file() || meta(&save).schema == 7);
        assert!(!root.join("Saves").join(BACKUP_DIR).exists());

        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (7, CURRENT_SCHEMA));
        assert!(report.style_default);
        assert!(report
            .backup
            .as_deref()
            .is_some_and(|b| b.starts_with("Saves/_backups/")));
        let m = meta(&save);
        assert_eq!(m.schema, CURRENT_SCHEMA);
        // The rest of settings is untouched by the schema bump.
        assert!(!m.settings.dice_display);
        // The style fields arrive with the contract defaults.
        assert_eq!(m.settings.style, Style::default());
        // The automation categories arrive with the charter defaults.
        assert_eq!(m.settings.automation, Automation::default());
        assert!(m.migrations.iter().any(|x| x.from == 7 && x.to == 8));
        assert!(m.migrations.iter().any(|x| x.from == 8 && x.to == 9));
        assert!(
            report
                .lines()
                .iter()
                .any(|l| l.contains("narration=standard")),
            "{:?}",
            report.lines()
        );

        // Whole-save backup taken before the write: the copy still says 7,
        // which is the documented restore path.
        let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves").join(BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        let backup_meta = fs::read_to_string(backups[0].join("save-meta.yaml")).unwrap();
        assert!(backup_meta.contains("schema: 7"), "{backup_meta}");

        // The backup path and the style default are written to the log.
        let log = fs::read_to_string(save.join("Sessions/2030-06-01-session-00.md")).unwrap();
        assert!(log.contains(&format!("schema 7→{CURRENT_SCHEMA}")), "{log}");
        assert!(log.contains("Saves/_backups/"), "{log}");
        assert!(log.contains("桌面风格默认"), "{log}");
        assert!(log.contains("自动化设置默认"), "{log}");
        assert!(combat_sim::integrity::changes(&save).unwrap().is_empty());

        // Idempotent: a second run changes nothing.
        let snap = combat_sim::integrity::snapshot(&save);
        let again = ensure(&save, &root, false).unwrap();
        assert!(again.already_current());
        assert_eq!(snap, combat_sim::integrity::snapshot(&save));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_meta_write_leaves_the_old_save_intact() {
        use std::os::unix::fs::PermissionsExt;
        let root = root("failwrite");
        let save = v7_save(&root, "2030-06-02-01");
        // A read-only save folder makes the atomic write of save-meta.yaml
        // fail; nothing else in a 7→9 migration writes inside the save.
        fs::set_permissions(&save, fs::Permissions::from_mode(0o555)).unwrap();
        let err = ensure(&save, &root, false).unwrap_err();
        fs::set_permissions(&save, fs::Permissions::from_mode(0o755)).unwrap();
        let msg = err.to_string();
        assert!(msg.contains("save-meta.yaml"), "{msg}");
        // No half-migrated save-meta.yaml: the old schema still loads.
        assert_eq!(meta(&save).schema, 7);
        assert_eq!(meta(&save).settings.style, Style::default());
        // And the old save is still usable: the retry migrates normally.
        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (7, CURRENT_SCHEMA));
        assert_eq!(meta(&save).schema, CURRENT_SCHEMA);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_v8_to_v9_write_leaves_old_meta_and_the_backup() {
        use std::os::unix::fs::PermissionsExt;
        let root = root("failwrite8");
        let save = root.join("Saves/2030-06-03-01");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::write(save.join("Sessions/s.md"), "# log\n").unwrap();
        fs::write(
            save.join("save-meta.yaml"),
            "schema: 8\ntool: 1.2.0\ndeviation: {scope: 0, ratio: null}\nsettings:\n  dice_display: false\n  style:\n    narration: short\n    detail: standard\n    pace: standard\n    npc_voice: distinct\n    options: 2-4\nmigrations: []\n",
        )
        .unwrap();
        fs::set_permissions(&save, fs::Permissions::from_mode(0o555)).unwrap();
        let err = ensure(&save, &root, false).unwrap_err();
        fs::set_permissions(&save, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(err.to_string().contains("save-meta.yaml"), "{err}");
        // The save-meta write never landed, so the schema-8 file and its
        // dice/style settings still load; the pre-migration backup exists.
        let m = meta(&save);
        assert_eq!(m.schema, 8);
        assert!(!m.settings.dice_display);
        assert_eq!(m.settings.style.narration, "short");
        assert_eq!(m.settings.automation, Automation::default());
        let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves").join(BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        let backup_meta = fs::read_to_string(backups[0].join("save-meta.yaml")).unwrap();
        assert!(backup_meta.contains("schema: 8"), "{backup_meta}");
        // Retry on the repaired save completes the migration.
        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (8, CURRENT_SCHEMA));
        assert_eq!(meta(&save).schema, CURRENT_SCHEMA);
        let _ = fs::remove_dir_all(&root);
    }

    /// Regression test for issue #154: `carry_dirty_flags`' manifest
    /// read-modify-write runs under the integrity manifest lock and reads
    /// the manifest inside it, so a concurrent tool process that commits
    /// between `after_write`'s release and this lock keeps its fingerprint.
    /// Writer B is injected (test-only fault hook) at exactly that point —
    /// after the lock is taken, before the manifest is read. The hook first
    /// proves the lock is held by opening a second handle on
    /// `.talkdnd.integrity.lock` and asserting `try_lock()` returns
    /// `WouldBlock`: without the lock this assertion fails instead of the
    /// whole test silently passing on pre-lock code. B stands in for a real
    /// writer that committed there — a real one reaches the same state
    /// through the lock, but the hook must not re-acquire the lock the test
    /// already holds, so its committed update is written with a raw manifest
    /// read-modify-write. Before the lock existed, carry's store dropped B's
    /// fingerprint (the first assertion below caught `progress.yaml`
    /// regressing to its pre-B value).
    #[test]
    fn carry_dirty_flags_keeps_a_concurrent_writers_fingerprints() {
        let save = std::env::temp_dir().join(format!("talkdnd-carry-race-{}", std::process::id()));
        let _ = fs::remove_dir_all(&save);
        for d in ["Characters", "NPCs"] {
            fs::create_dir_all(save.join(d)).unwrap();
        }
        fs::write(save.join("progress.yaml"), "xp: {hero: 0}\n").unwrap();
        fs::write(save.join("NPCs/Gil.md"), "# Gil\n").unwrap();
        // Baseline manifest: both files at their current fingerprints.
        combat_sim::integrity::ensure(&save).unwrap();
        // The DM hand-edited Gil before the migration: the manifest keeps
        // the stale pre-edit fingerprint and Gil is flagged dirty.
        fs::write(save.join("NPCs/Gil.md"), "# Gil (hand-edited)\n").unwrap();
        let dirty: BTreeSet<String> = ["NPCs/Gil.md".to_string()].into_iter().collect();
        // The migration moves the file; carry_dirty_flags then moves the
        // stale fingerprint to the new path so the flag follows the file.
        fs::rename(save.join("NPCs/Gil.md"), save.join("Characters/Gil.md")).unwrap();
        let moved = vec![Move {
            from: "Gil.md".to_string(),
            to: "Gil.md".to_string(),
            sheet: None,
        }];

        const B_PROGRESS: &str = "xp: {hero: 50}\n";
        let b_save = save.clone();
        let writer_b = move || {
            // The hook runs with the integrity manifest lock held: a second
            // opener must be locked out. This is the assertion that fails on
            // pre-lock code (issue #154).
            let probe = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(b_save.join(combat_sim::integrity::LOCK))
                .unwrap();
            match probe.try_lock() {
                Err(std::fs::TryLockError::WouldBlock) => {}
                other => panic!(
                    "the integrity manifest lock must be held at the hook point; try_lock returned {other:?}"
                ),
            }
            fs::write(b_save.join("progress.yaml"), B_PROGRESS).unwrap();
            let mut m = combat_sim::integrity::load(&b_save).unwrap().unwrap();
            m.files.insert(
                "progress.yaml".to_string(),
                combat_sim::integrity::fingerprint(B_PROGRESS.as_bytes()),
            );
            combat_sim::integrity::store(&b_save, &m).unwrap();
        };
        carry_dirty_flags_guarded(&save, &moved, &dirty, Some(&writer_b)).unwrap();

        // B's committed fingerprint survived the carry's store.
        let m = combat_sim::integrity::load(&save).unwrap().unwrap();
        assert_eq!(
            m.files.get("progress.yaml").map(String::as_str),
            Some(combat_sim::integrity::fingerprint(B_PROGRESS.as_bytes()).as_str()),
            "a concurrent writer's fingerprint was lost to carry_dirty_flags"
        );
        // The dirty flag followed the moved file under its new path, with
        // the stale pre-edit fingerprint (never laundered), and the old
        // entry is gone.
        assert!(!m.files.contains_key("NPCs/Gil.md"));
        assert!(m.files.contains_key("Characters/Gil.md"));
        assert_eq!(
            combat_sim::integrity::changes(&save).unwrap(),
            vec![combat_sim::integrity::Change::Modified(
                "Characters/Gil.md".into()
            )]
        );
        let _ = fs::remove_dir_all(&save);
    }
}
