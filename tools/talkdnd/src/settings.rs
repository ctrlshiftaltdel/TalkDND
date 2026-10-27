//! Per-save settings (`save-meta.yaml` -> `settings:`).
//!
//! `dice_display` (default on) controls whether the DM reads dice values,
//! DCs and modifiers aloud. It never changes whether the programs roll: every
//! roll is made by the programs and written to `rolls.log`, and the DM can
//! show it later with `talkdnd log rolls`.
//!
//! `automation` (schema 9) is the DM automation switch board: one save-wide
//! default per fixed category plus optional scene and encounter overrides.
//! Effective order is encounter > scene > save default. Every change is
//! validated, written to `save-meta.yaml` and audited in the current session
//! log without save contents or secrets.

use crate::migrate::{self, automation_category, check_id, Level, Settings, AUTOMATION_CATEGORIES};
use crate::{progress, save, save::append};
use anyhow::{bail, Result};
use clap::Subcommand;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub fn get(save_dir: &Path) -> Settings {
    migrate::load(save_dir)
        .ok()
        .flatten()
        .map(|m| m.settings)
        .unwrap_or_default()
}

pub fn dice_display(save_dir: &Path) -> bool {
    get(save_dir).dice_display
}

/// `talkdnd settings` (show) and after a change.
pub fn lines(save_dir: &Path) -> Result<Vec<String>> {
    let s = migrate::load(save_dir)?
        .map(|m| m.settings)
        .unwrap_or_default();
    let mut out = vec![if s.dice_display {
        "骰子显示：开（每个检定/伤害展示完整算式；见 docs/dice.md）".to_string()
    } else {
        "骰子显示：关（只叙述结果，不报骰值、DC、修正；掷骰仍由程序完成并记入 rolls.log）"
            .to_string()
    }];
    out.push(format!("自动化：{}", automation_digest(save_dir)?));
    Ok(out)
}

/// Change dice display and log it. `--reason` is optional and only recorded.
pub fn set_dice(
    root: &Path,
    save_dir: &Path,
    on: bool,
    reason: Option<&str>,
) -> Result<Vec<String>> {
    // Old saves migrate first so they get a current save-meta.yaml.
    migrate::ensure(save_dir, root, false)?;
    let Some(mut meta) = migrate::load(save_dir)? else {
        bail!("{}: save-meta.yaml is missing", save_dir.display());
    };
    meta.settings.dice_display = on;
    migrate::write_meta(save_dir, &meta)?;
    let why = reason
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(|r| format!("（{r}）"))
        .unwrap_or_default();
    let line = format!("骰子显示：{}{why}", if on { "开" } else { "关" });
    if let Some(log) = save::latest_log(save_dir) {
        append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(vec![line])
}

/// `talkdnd settings dice on|off`; no value shows the current setting.
pub fn parse_dice(value: Option<&str>) -> Result<Option<bool>> {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) if v.eq_ignore_ascii_case("on") || v == "开" => Ok(Some(true)),
        Some(v) if v.eq_ignore_ascii_case("off") || v == "关" => Ok(Some(false)),
        Some(v) => bail!("骰子显示只能是 on 或 off（收到 `{v}`）"),
    }
}

// ---------------------------------------------------------------------------
// Automation switches (schema 9)
// ---------------------------------------------------------------------------

/// `talkdnd settings automation show|set|reset`.
#[derive(Subcommand, Clone)]
pub enum AutomationCmd {
    /// Show save defaults and overrides, or the effective table for a context
    Show {
        /// Scene id for the effective table
        #[arg(long)]
        scene: Option<String>,
        /// Encounter id for the effective table
        #[arg(long)]
        encounter: Option<String>,
    },
    /// Set one category at the save/scene/encounter scope
    Set {
        /// resolution | combat | npc | player_choice | reaction | near_limit
        category: String,
        /// manual | assisted | auto
        level: String,
        /// Override this scene id (from the save or its adventure combat data)
        #[arg(long)]
        scene: Option<String>,
        /// Override this encounter id (from the save or its adventure combat data)
        #[arg(long)]
        encounter: Option<String>,
        /// Why the setting changes (required; written to the session log)
        #[arg(long)]
        reason: String,
    },
    /// Reset one category (or all) at the save/scene/encounter scope
    Reset {
        /// category name, or `all`
        category: String,
        /// Reset the scene override for this id
        #[arg(long)]
        scene: Option<String>,
        /// Reset the encounter override for this id
        #[arg(long)]
        encounter: Option<String>,
        /// Why the setting changes (required; written to the session log)
        #[arg(long)]
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    SaveDefault,
    Scene(String),
    Encounter(String),
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Self::SaveDefault => "存档默认".to_string(),
            Self::Scene(id) => format!("场景 {id}"),
            Self::Encounter(id) => format!("遭遇 {id}"),
        }
    }

    fn short(&self) -> String {
        match self {
            Self::SaveDefault => String::new(),
            Self::Scene(id) => format!("（场景 {id}）"),
            Self::Encounter(id) => format!("（遭遇 {id}）"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub name: &'static str,
    pub zh: &'static str,
    pub level: Level,
    pub source: Source,
}

/// Effective levels for one context. `encounter_scene` is the scene the
/// encounter links to (if its content declares one); an explicit scene wins
/// only when the encounter has no link.
pub fn resolve(
    settings: &Settings,
    scene: Option<&str>,
    encounter: Option<&str>,
    encounter_scene: Option<&str>,
) -> Vec<Resolved> {
    let linked_scene = encounter_scene.or(scene);
    AUTOMATION_CATEGORIES
        .iter()
        .map(|category| {
            if let Some(id) = encounter {
                if let Some(level) = settings
                    .automation
                    .encounters
                    .get(id)
                    .and_then(|levels| levels.get(category.name))
                {
                    return Resolved {
                        name: category.name,
                        zh: category.zh,
                        level: *level,
                        source: Source::Encounter(id.to_string()),
                    };
                }
            }
            if let Some(id) = linked_scene {
                if let Some(level) = settings
                    .automation
                    .scenes
                    .get(id)
                    .and_then(|levels| levels.get(category.name))
                {
                    return Resolved {
                        name: category.name,
                        zh: category.zh,
                        level: *level,
                        source: Source::Scene(id.to_string()),
                    };
                }
            }
            Resolved {
                name: category.name,
                zh: category.zh,
                level: settings.automation.default_level(category.name),
                source: Source::SaveDefault,
            }
        })
        .collect()
}

/// One deterministic line for the `talkdnd start` digest and bare `settings`:
/// effective values at the current scene, with override counts.
pub fn automation_digest(save_dir: &Path) -> Result<String> {
    let meta = migrate::load(save_dir)?.unwrap_or_default();
    let known = known_ids(save_dir);
    let scene = progress::scene(save_dir).map(|label| known.scene_for_label(&label));
    let levels = resolve(&meta.settings, scene.as_deref(), None, None);
    let values = levels
        .iter()
        .map(|r| format!("{} {}{}", r.zh, r.level, r.source.short()))
        .collect::<Vec<_>>()
        .join("、");
    Ok(format!(
        "{}；覆盖：场景 {}、遭遇 {}（`talkdnd settings automation` 查看来源）",
        values,
        meta.settings.automation.scenes.len(),
        meta.settings.automation.encounters.len()
    ))
}

/// `settings automation show` lines. Without a context it lists the save
/// defaults plus every override; with `--scene`/`--encounter` it prints one
/// effective value per category and names its source.
pub fn automation_lines(
    save_dir: &Path,
    scene: Option<&str>,
    encounter: Option<&str>,
) -> Result<Vec<String>> {
    let Some(meta) = migrate::load(save_dir)? else {
        bail!("{}: save-meta.yaml is missing", save_dir.display());
    };
    let automation = &meta.settings.automation;
    let mut out =
        vec!["自动化开关（settings.automation v1；解析顺序：遭遇 > 场景 > 存档默认）".to_string()];
    if scene.is_some() || encounter.is_some() {
        let known = known_ids(save_dir);
        let scene = scene.map(str::trim).filter(|s| !s.is_empty());
        let encounter = encounter.map(str::trim).filter(|s| !s.is_empty());
        if let Some(id) = scene {
            check_id("场景", id)?;
            if !known.scenes.contains(id) {
                bail!("{}", unknown_scene(id, &known));
            }
        }
        if let Some(id) = encounter {
            check_id("遭遇", id)?;
            if !known.encounters.contains_key(id) {
                bail!("{}", unknown_encounter(id, &known));
            }
        }
        let linked_scene = encounter
            .and_then(|id| known.encounters.get(id).and_then(|s| s.as_deref()))
            .or(scene);
        for resolved in resolve(&meta.settings, scene, encounter, linked_scene) {
            out.push(format!(
                "- {}（{}）：{} — 来源：{}",
                resolved.name,
                resolved.zh,
                resolved.level,
                resolved.source.describe()
            ));
        }
        return Ok(out);
    }

    for category in AUTOMATION_CATEGORIES {
        out.push(format!(
            "- {}（{}）：{} — 来源：存档默认（{}）",
            category.name,
            category.zh,
            automation.default_level(category.name),
            category.note
        ));
    }
    out.push(if automation.scenes.is_empty() {
        "场景覆盖：无".to_string()
    } else {
        let mut lines = vec!["场景覆盖：".to_string()];
        for (id, levels) in &automation.scenes {
            lines.push(format!("- {}：{}", id, format_levels(levels)));
        }
        lines.join("\n")
    });
    out.push(if automation.encounters.is_empty() {
        "遭遇覆盖：无".to_string()
    } else {
        let mut lines = vec!["遭遇覆盖：".to_string()];
        for (id, levels) in &automation.encounters {
            lines.push(format!("- {}：{}", id, format_levels(levels)));
        }
        lines.join("\n")
    });
    Ok(out)
}

fn format_levels(levels: &BTreeMap<String, Level>) -> String {
    levels
        .iter()
        .map(|(name, level)| format!("{name}={level}"))
        .collect::<Vec<_>>()
        .join("、")
}

#[derive(Debug)]
enum Scope {
    Save,
    Scene(String),
    Encounter(String),
}

impl Scope {
    fn describe(&self) -> String {
        match self {
            Self::Save => "存档默认".to_string(),
            Self::Scene(id) => format!("场景 {id}"),
            Self::Encounter(id) => format!("遭遇 {id}"),
        }
    }
}

fn scope(scene: Option<&str>, encounter: Option<&str>) -> Result<Scope> {
    let scene = match scene {
        Some(value) if value.trim().is_empty() => bail!("--scene 不能为空"),
        Some(value) => Some(value.trim()),
        None => None,
    };
    let encounter = match encounter {
        Some(value) if value.trim().is_empty() => bail!("--encounter 不能为空"),
        Some(value) => Some(value.trim()),
        None => None,
    };
    match (scene, encounter) {
        (Some(_), Some(_)) => {
            bail!("--scene 与 --encounter 不能同时使用；一次只覆盖一个层级")
        }
        (Some(id), None) => {
            check_id("场景", id)?;
            Ok(Scope::Scene(id.to_string()))
        }
        (None, Some(id)) => {
            check_id("遭遇", id)?;
            Ok(Scope::Encounter(id.to_string()))
        }
        (None, None) => Ok(Scope::Save),
    }
}

fn audit_reason(reason: &str) -> Result<&str> {
    let reason = reason.trim();
    if reason.is_empty() {
        bail!("--reason 不能为空：写一句玩家为什么要求改（会记进会话日志）");
    }
    if reason.chars().count() > 200 {
        bail!("--reason 太长（最多 200 个字符）；请写一句简短原因");
    }
    if reason.chars().any(char::is_control) {
        bail!("--reason 不能包含控制字符");
    }
    Ok(reason)
}

/// Validate that a scene/encounter id exists in this save's known ids. The
/// known set comes from the current scene label, live `combat/*.state.json`
/// and the adventure `combat/` data named by the save's campaign manifest.
fn require_known(save_dir: &Path, scope: &Scope) -> Result<()> {
    let known = known_ids(save_dir);
    match scope {
        Scope::Save => Ok(()),
        Scope::Scene(id) if known.scenes.contains(id) => Ok(()),
        Scope::Scene(id) => bail!("{}", unknown_scene(id, &known)),
        Scope::Encounter(id) if known.encounters.contains_key(id) => Ok(()),
        Scope::Encounter(id) => bail!("{}", unknown_encounter(id, &known)),
    }
}

fn unknown_scene(id: &str, known: &KnownIds) -> String {
    let list = if known.scenes.is_empty() {
        "（没有已知场景；先运行 `talkdnd scene` 或让冒险内容加载场景）".to_string()
    } else {
        known.scenes.iter().cloned().collect::<Vec<_>>().join("、")
    };
    format!("未知场景 id `{id}`；已知场景：{list}；存档未改动")
}

fn unknown_encounter(id: &str, known: &KnownIds) -> String {
    let list = if known.encounters.is_empty() {
        "（没有已知遭遇；先加载冒险 combat/encounters.yaml）".to_string()
    } else {
        known
            .encounters
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join("、")
    };
    format!("未知遭遇 id `{id}`；已知遭遇：{list}；存档未改动")
}

pub fn set_automation(
    root: &Path,
    save_dir: &Path,
    category: &str,
    level: &str,
    scene: Option<&str>,
    encounter: Option<&str>,
    reason: &str,
) -> Result<Vec<String>> {
    let category = category.trim();
    let Some(category_def) = automation_category(category) else {
        bail!(
            "自动化类别只能是 {}（收到 `{category}`）；存档未改动",
            AUTOMATION_CATEGORIES
                .iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
                .join("、")
        );
    };
    let level = Level::parse(level)?;
    if category_def.player_owned && level == Level::Auto {
        bail!(
            "{} 不能设为 auto：玩家选择/反应绝不能自动代做（可用 assisted 或 manual）；存档未改动",
            category_def.name
        );
    }
    let reason = audit_reason(reason)?;
    let scope = scope(scene, encounter)?;
    require_known(save_dir, &scope)?;

    // Old saves migrate first: schema 9 writes every charter default.
    migrate::ensure(save_dir, root, false)?;
    let Some(mut meta) = migrate::load(save_dir)? else {
        bail!("{}: save-meta.yaml is missing", save_dir.display());
    };
    meta.settings.automation.fill_defaults();
    let old = match &scope {
        Scope::Save => Some(meta.settings.automation.default_level(category)),
        Scope::Scene(id) => meta
            .settings
            .automation
            .scenes
            .get(id)
            .and_then(|levels| levels.get(category))
            .copied(),
        Scope::Encounter(id) => meta
            .settings
            .automation
            .encounters
            .get(id)
            .and_then(|levels| levels.get(category))
            .copied(),
    };
    if old == Some(level) {
        return Ok(vec![format!(
            "自动化 {category}（{}）已是 {level}；未改动、未记日志",
            scope.describe()
        )]);
    }
    let old_text = old.map_or_else(|| "未覆盖".to_string(), |l| l.to_string());
    match &scope {
        Scope::Save => {
            meta.settings
                .automation
                .default
                .insert(category.to_string(), level);
        }
        Scope::Scene(id) => {
            meta.settings
                .automation
                .scenes
                .entry(id.clone())
                .or_default()
                .insert(category.to_string(), level);
        }
        Scope::Encounter(id) => {
            meta.settings
                .automation
                .encounters
                .entry(id.clone())
                .or_default()
                .insert(category.to_string(), level);
        }
    }
    meta.settings.automation.check()?;
    migrate::write_meta(save_dir, &meta)?;
    let line = format!(
        "自动化 {category}（{}）：{old_text} → {level}（{reason}）",
        scope.describe()
    );
    log_line(save_dir, &line)?;
    Ok(vec![line])
}

pub fn reset_automation(
    root: &Path,
    save_dir: &Path,
    category: &str,
    scene: Option<&str>,
    encounter: Option<&str>,
    reason: &str,
) -> Result<Vec<String>> {
    let category = category.trim();
    let reset_all = category == "all";
    if !reset_all && automation_category(category).is_none() {
        bail!(
            "自动化类别只能是 {} 或 all（收到 `{category}`）；存档未改动",
            AUTOMATION_CATEGORIES
                .iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
                .join("、")
        );
    }
    let reason = audit_reason(reason)?;
    let scope = scope(scene, encounter)?;
    require_known(save_dir, &scope)?;

    migrate::ensure(save_dir, root, false)?;
    let Some(mut meta) = migrate::load(save_dir)? else {
        bail!("{}: save-meta.yaml is missing", save_dir.display());
    };
    meta.settings.automation.fill_defaults();

    let names: Vec<&str> = if reset_all {
        AUTOMATION_CATEGORIES.iter().map(|c| c.name).collect()
    } else {
        vec![category]
    };
    let mut removed = Vec::new();
    let mut old_values = Vec::new();
    match &scope {
        Scope::Save => {
            for name in names {
                let old = meta.settings.automation.default_level(name);
                let target = automation_category(name).map(|c| c.default).unwrap();
                if old != target {
                    old_values.push(format!("{name}={old}"));
                    meta.settings
                        .automation
                        .default
                        .insert(name.to_string(), target);
                    removed.push(name.to_string());
                }
            }
        }
        Scope::Scene(id) => {
            let mut empty = false;
            if let Some(levels) = meta.settings.automation.scenes.get_mut(id) {
                for name in names {
                    if let Some(old) = levels.remove(name) {
                        old_values.push(format!("{name}={old}"));
                        removed.push(name.to_string());
                    }
                }
                empty = levels.is_empty();
            }
            if empty {
                meta.settings.automation.scenes.remove(id);
            }
        }
        Scope::Encounter(id) => {
            let mut empty = false;
            if let Some(levels) = meta.settings.automation.encounters.get_mut(id) {
                for name in names {
                    if let Some(old) = levels.remove(name) {
                        old_values.push(format!("{name}={old}"));
                        removed.push(name.to_string());
                    }
                }
                empty = levels.is_empty();
            }
            if empty {
                meta.settings.automation.encounters.remove(id);
            }
        }
    }

    if removed.is_empty() {
        return Ok(vec![if matches!(scope, Scope::Save) {
            format!(
                "自动化（{}）已经是章程默认；未改动、未记日志",
                scope.describe()
            )
        } else {
            format!(
                "自动化（{}）没有可重置的覆盖；未改动、未记日志",
                scope.describe()
            )
        }]);
    }
    meta.settings.automation.check()?;
    migrate::write_meta(save_dir, &meta)?;
    let line = if reset_all {
        format!(
            "自动化重置（{}）：{} → {}（{reason}）",
            scope.describe(),
            old_values.join("、"),
            if matches!(scope, Scope::Save) {
                "章程默认"
            } else {
                "未覆盖"
            }
        )
    } else {
        format!(
            "自动化 {category}（{}）：{} → {}（{reason}）",
            scope.describe(),
            old_values
                .first()
                .map(|v| v.split_once('=').map(|(_, v)| v).unwrap_or(v))
                .unwrap_or("未覆盖"),
            if matches!(scope, Scope::Save) {
                "章程默认"
            } else {
                "未覆盖"
            }
        )
    };
    log_line(save_dir, &line)?;
    Ok(vec![line])
}

fn log_line(save_dir: &Path, line: &str) -> Result<()> {
    if let Some(log) = save::latest_log(save_dir) {
        append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(())
}

pub fn run_automation(root: &Path, save_dir: &Path, cmd: AutomationCmd) -> Result<()> {
    match cmd {
        AutomationCmd::Show { scene, encounter } => {
            for line in automation_lines(save_dir, scene.as_deref(), encounter.as_deref())? {
                println!("{line}");
            }
        }
        AutomationCmd::Set {
            category,
            level,
            scene,
            encounter,
            reason,
        } => {
            for line in set_automation(
                root,
                save_dir,
                &category,
                &level,
                scene.as_deref(),
                encounter.as_deref(),
                &reason,
            )? {
                println!("{line}");
            }
        }
        AutomationCmd::Reset {
            category,
            scene,
            encounter,
            reason,
        } => {
            for line in reset_automation(
                root,
                save_dir,
                &category,
                scene.as_deref(),
                encounter.as_deref(),
                &reason,
            )? {
                println!("{line}");
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Known scene/encounter ids
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct KnownIds {
    scenes: BTreeSet<String>,
    /// Scene display name -> id, for resolving the free-text `progress.scene`
    /// label to a content id.
    scene_names: BTreeMap<String, String>,
    /// Encounter id -> linked scene id, when the content declares one.
    encounters: BTreeMap<String, Option<String>>,
}

impl KnownIds {
    fn scene_for_label(&self, label: &str) -> String {
        if let Some(id) = self.scene_names.get(label) {
            return id.clone();
        }
        if self.scenes.contains(label) {
            return label.to_string();
        }
        label.to_string()
    }
}

fn known_ids(save_dir: &Path) -> KnownIds {
    let mut known = KnownIds::default();
    if let Some(scene) = progress::scene(save_dir) {
        // The free-text scene label doubles as an id only when it is a valid
        // one; otherwise it can still be resolved through a content name.
        if check_id("场景", &scene).is_ok() {
            known.scenes.insert(scene);
        }
    }
    // Live encounter state files carry the scene id and encounter name.
    if let Ok(rd) = fs::read_dir(save_dir.join("combat")) {
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for file in files {
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if let Some(id) = value
                .get("scene")
                .and_then(|s| s.get("id"))
                .and_then(|v| v.as_str())
            {
                known.scenes.insert(id.to_string());
            }
            if let Some(name) = value.get("encounter_name").and_then(|v| v.as_str()) {
                let scene = value
                    .get("scene")
                    .and_then(|s| s.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                known.encounters.insert(name.to_string(), scene);
            }
        }
    }
    // Adventure combat data named by the save's campaign manifest.
    for dir in adventure_dirs(save_dir) {
        scan_dir(&dir, &mut known);
    }
    known
}

fn adventure_dirs(save_dir: &Path) -> Vec<PathBuf> {
    let combat = save_dir.join("combat");
    let Ok(text) = fs::read_to_string(combat.join("campaign.yaml")) else {
        return Vec::new();
    };
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("adventures:") else {
            continue;
        };
        let rest = rest
            .split('#')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(['[', ']']);
        return rest
            .split(',')
            .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
            .filter(|s| !s.is_empty())
            .map(|s| {
                let p = PathBuf::from(&s);
                if p.is_absolute() {
                    p
                } else {
                    combat.join(p)
                }
            })
            .collect();
    }
    Vec::new()
}

fn scan_dir(dir: &Path, known: &mut KnownIds) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            scan_dir(&path, known);
        } else if path
            .extension()
            .is_some_and(|x| matches!(x.to_str(), Some("yaml" | "yml" | "json")))
        {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = serde_norway::from_str::<serde_norway::Value>(&text) else {
                continue;
            };
            scan_value(&value, known);
        }
    }
}

fn scan_value(value: &serde_norway::Value, known: &mut KnownIds) {
    let Some(map) = value.as_mapping() else {
        return;
    };
    if let Some(scenes) = map
        .get(serde_norway::Value::String("scenes".into()))
        .and_then(|v| v.as_sequence())
    {
        for scene in scenes {
            let Some(map) = scene.as_mapping() else {
                continue;
            };
            let Some(id) = map
                .get(serde_norway::Value::String("id".into()))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            known.scenes.insert(id.to_string());
            if let Some(name) = map
                .get(serde_norway::Value::String("name".into()))
                .and_then(|v| v.as_str())
            {
                known.scene_names.insert(name.to_string(), id.to_string());
            }
        }
    }
    if let Some(encounters) = map
        .get(serde_norway::Value::String("encounters".into()))
        .and_then(|v| v.as_sequence())
    {
        for encounter in encounters {
            let Some(map) = encounter.as_mapping() else {
                continue;
            };
            let Some(id) = map
                .get(serde_norway::Value::String("id".into()))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            let scene = map
                .get(serde_norway::Value::String("scene".into()))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            known.encounters.insert(id.to_string(), scene);
        }
    }
    // Single-item files are accepted too (the loader reads them as one item).
    let Some(id) = map
        .get(serde_norway::Value::String("id".into()))
        .and_then(|v| v.as_str())
    else {
        return;
    };
    if map.contains_key(serde_norway::Value::String("members".into())) {
        let scene = map
            .get(serde_norway::Value::String("scene".into()))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        known.encounters.insert(id.to_string(), scene);
    } else if map.contains_key(serde_norway::Value::String("zones".into()))
        || map.contains_key(serde_norway::Value::String("mode".into()))
        || map.contains_key(serde_norway::Value::String("light".into()))
    {
        known.scenes.insert(id.to_string());
        if let Some(name) = map
            .get(serde_norway::Value::String("name".into()))
            .and_then(|v| v.as_str())
        {
            known.scene_names.insert(name.to_string(), id.to_string());
        }
    }
}

/// Used by tests: the effective map with a synthetic context.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::{Automation, CURRENT_SCHEMA};

    fn root(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("talkdnd-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("Saves/2030-01-01-01/Sessions")).unwrap();
        std::fs::write(
            p.join("Saves/2030-01-01-01/Sessions/2030-01-01-session-00.md"),
            "# log\n",
        )
        .unwrap();
        crate::migrate::init(&p.join("Saves/2030-01-01-01")).unwrap();
        p
    }

    #[test]
    fn default_is_on_and_changes_are_logged() {
        let r = root("toggle");
        let save = r.join("Saves/2030-01-01-01");
        assert!(dice_display(&save));
        assert_eq!(
            migrate::load(&save).unwrap().unwrap().schema,
            CURRENT_SCHEMA
        );
        assert!(lines(&save).unwrap()[0].contains("开"));
        let out = set_dice(&r, &save, false, Some("table vote")).unwrap();
        assert!(out[0].contains("关") && out[0].contains("table vote"));
        assert!(!dice_display(&save));
        let log = std::fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md")).unwrap();
        assert!(log.contains("骰子显示：关"), "{log}");
        set_dice(&r, &save, true, None).unwrap();
        assert!(dice_display(&save));
        assert!(parse_dice(Some("on")).unwrap() == Some(true));
        assert!(parse_dice(Some("off")).unwrap() == Some(false));
        assert!(parse_dice(None).unwrap().is_none());
        assert!(parse_dice(Some("maybe")).is_err());
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn precedence_and_player_owned_levels() {
        let mut automation = Automation::default();
        automation
            .scenes
            .insert("s1".into(), BTreeMap::from([("npc".into(), Level::Manual)]));
        automation
            .encounters
            .insert("e1".into(), BTreeMap::from([("npc".into(), Level::Auto)]));
        let settings = Settings {
            automation,
            ..Default::default()
        };
        let resolved = resolve(&settings, Some("s1"), Some("e1"), Some("s1"));
        let npc = resolved.iter().find(|r| r.name == "npc").unwrap();
        assert_eq!(npc.level, Level::Auto);
        assert_eq!(npc.source, Source::Encounter("e1".into()));
        let combat = resolved.iter().find(|r| r.name == "combat").unwrap();
        assert_eq!(combat.source, Source::SaveDefault);

        let mut bad = Automation::default();
        bad.default.insert("player_choice".into(), Level::Auto);
        assert!(bad.check().is_err());
        let mut bad = Automation::default();
        bad.default.insert("bogus".into(), Level::Auto);
        assert!(bad.check().is_err());
        let mut bad = Automation::default();
        bad.scenes.insert(
            "bad id".into(),
            BTreeMap::from([("npc".into(), Level::Auto)]),
        );
        assert!(bad.check().is_err());
    }
}
