//! Per-save original-content ledger (`Saves/<slot>/original.yaml`, DM only).
//!
//! A save declares how far the DM may leave the published adventure
//! (`deviation` in `save-meta.yaml`, set once at save creation):
//!
//! - scope 0: fill blanks only (names for unnamed NPCs, dialogue wording,
//!   scene description). No original content is registered.
//! - scope 1: original encounters, NPCs, items and locations.
//! - scope 2: the same plus side quests.
//!
//! Every original element must be registered here *before* it appears in
//! play. The per-chapter budget is
//! `original / (original + canon_progress) <= ratio`, checked after adding:
//! `original+1 <= floor(ratio% * (original+1+canon_progress))`. Canon progress
//! counts XP/milestone entries and revealed facts (see [`Context`]).
//!
//! Names, dialogue and descriptions that fill a gap in the book are never
//! original content. Rarity is capped by both the scope and the party level;
//! the effective cap is the lower of the two.

use crate::check;
use crate::migrate::Deviation;
use crate::progress::{self, Progress};
use crate::reveal;
use crate::save;
use anyhow::{anyhow, bail, Context as _, Result};
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "original.yaml";
const LEDGER_VERSION: u32 = 2;
/// Track-less saves still need a chapter label for the ledger.
pub const UNCHAPTERED: &str = "未分章";
/// Local long-term DM plan (never committed).
pub const PLAN_FILE: &str = "dm-plan.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Encounter,
    Npc,
    Item,
    Location,
    Sidequest,
}

const KINDS_SCOPE1: [Kind; 4] = [Kind::Encounter, Kind::Npc, Kind::Item, Kind::Location];
const KINDS_SCOPE2: [Kind; 5] = [
    Kind::Encounter,
    Kind::Npc,
    Kind::Item,
    Kind::Location,
    Kind::Sidequest,
];
const KINDS_NONE: [Kind; 0] = [];

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Encounter => "encounter",
            Kind::Npc => "npc",
            Kind::Item => "item",
            Kind::Location => "location",
            Kind::Sidequest => "sidequest",
        }
    }

    pub fn zh(self) -> &'static str {
        match self {
            Kind::Encounter => "遭遇",
            Kind::Npc => "NPC",
            Kind::Item => "物品",
            Kind::Location => "地点",
            Kind::Sidequest => "支线",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        let t = s.trim().to_lowercase().replace('_', "-");
        match t.as_str() {
            "encounter" | "遭遇" => Ok(Kind::Encounter),
            "npc" | "角色" => Ok(Kind::Npc),
            "item" | "物品" => Ok(Kind::Item),
            "location" | "地点" => Ok(Kind::Location),
            "sidequest" | "side-quest" | "支线" => Ok(Kind::Sidequest),
            _ => bail!(
                "未知的原创类型 `{s}`；可用：{}",
                Kind::all()
                    .iter()
                    .map(|k| k.id())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    pub fn all() -> [Kind; 5] {
        [
            Kind::Encounter,
            Kind::Npc,
            Kind::Item,
            Kind::Location,
            Kind::Sidequest,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    VeryRare,
    Legendary,
}

impl Rarity {
    pub fn id(self) -> &'static str {
        match self {
            Rarity::Common => "common",
            Rarity::Uncommon => "uncommon",
            Rarity::Rare => "rare",
            Rarity::VeryRare => "very-rare",
            Rarity::Legendary => "legendary",
        }
    }

    pub fn zh(self) -> &'static str {
        match self {
            Rarity::Common => "普通",
            Rarity::Uncommon => "非普通",
            Rarity::Rare => "珍稀",
            Rarity::VeryRare => "极珍稀",
            Rarity::Legendary => "传奇",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        let t = s.trim().to_lowercase().replace('_', "-");
        match t.as_str() {
            "common" | "普通" => Ok(Rarity::Common),
            "uncommon" | "非普通" => Ok(Rarity::Uncommon),
            "rare" | "珍稀" => Ok(Rarity::Rare),
            "very-rare" | "very rare" | "veryrare" | "极珍稀" => Ok(Rarity::VeryRare),
            "legendary" | "传奇" => Ok(Rarity::Legendary),
            _ => bail!("未知稀有度 `{s}`；可用：common, uncommon, rare, very-rare, legendary"),
        }
    }

    fn label(self) -> String {
        format!("{}（{}）", self.zh(), self.id())
    }
}

/// Types allowed at a scope (empty for scope 0 and unknown scopes).
pub fn allowed_kinds(scope: u32) -> &'static [Kind] {
    match scope {
        1 => &KINDS_SCOPE1,
        2 => &KINDS_SCOPE2,
        _ => &KINDS_NONE,
    }
}

/// Highest rarity the scope allows, regardless of party level.
pub fn scope_cap(scope: u32) -> Option<Rarity> {
    match scope {
        1 => Some(Rarity::Uncommon),
        2 => Some(Rarity::Rare),
        _ => None,
    }
}

/// Highest rarity a party of this level may meet: 1-4 uncommon, 5-10 rare,
/// 11-16 very rare, 17+ legendary.
pub fn party_cap(level: i32) -> Rarity {
    match level {
        ..=4 => Rarity::Uncommon,
        5..=10 => Rarity::Rare,
        11..=16 => Rarity::VeryRare,
        _ => Rarity::Legendary,
    }
}

/// The lower of the scope and party-level caps.
pub fn effective_cap(scope: u32, party_level: i32) -> Option<Rarity> {
    scope_cap(scope).map(|s| s.min(party_cap(party_level)))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// `O-001`, `O-002`, ...
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub rarity: String,
    /// Chapter label at the time it was added (`未分章` without a track).
    #[serde(default)]
    pub chapter: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// DM-only hidden aspect introduced by this element (schema 2). It is a
    /// secret term source and must not appear in player-visible text until
    /// `talkdnd reveal add --original O-xxx` records the reveal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<String>,
    /// The idea came from a player; adopted player hooks count as original.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub player_hook: bool,
    #[serde(default)]
    pub logged: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Ledger {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            version: LEDGER_VERSION,
            entries: Vec::new(),
        }
    }
}

pub fn path(save_dir: &Path) -> PathBuf {
    save_dir.join(FILE)
}

pub fn load(save_dir: &Path) -> Result<Ledger> {
    let p = path(save_dir);
    if !p.is_file() {
        return Ok(Ledger::default());
    }
    let ledger: Ledger = serde_norway::from_str(&fs::read_to_string(&p)?)
        .with_context(|| format!("invalid {}", p.display()))?;
    if ledger.version > LEDGER_VERSION {
        bail!(
            "{}: unsupported original ledger version {} (this talkdnd supports {LEDGER_VERSION}); update talkdnd",
            p.display(),
            ledger.version
        );
    }
    Ok(ledger)
}

pub(crate) fn store(save_dir: &Path, ledger: &Ledger) -> Result<()> {
    let mut current = ledger.clone();
    current.version = LEDGER_VERSION;
    let text = format!(
        "# TalkDND original-content ledger (DM only; players must never see this).\n# Register with `talkdnd original add` before introducing the element. `hidden:` is DM-only.\n{}",
        serde_norway::to_string(&current)?
    );
    atomic_write(path(save_dir), text.as_bytes())
}

/// The locked deviation of this save (default scope 0 before migration).
pub fn deviation(save_dir: &Path) -> Result<Deviation> {
    Ok(crate::migrate::load(save_dir)?
        .map(|m| m.deviation)
        .unwrap_or_default())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonUnits {
    pub xp: usize,
    pub reveals: usize,
}

impl CanonUnits {
    pub fn total(self) -> usize {
        self.xp + self.reveals
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub party_level: i32,
    pub chapter: String,
    pub chapter_level: i32,
    /// Canon units since the last major milestone (all of them without one).
    pub chapter_units: CanonUnits,
    pub campaign_units: CanonUnits,
}

/// Current chapter and canon-progress counts.
///
/// Chapter: the last track step at or below the lowest party level; without a
/// track every save is `未分章`. Chapter canon units: every XP/milestone
/// entry since the last milestone entry (inclusive) plus every revealed fact
/// dated at or after it; without a milestone, every campaign unit. Campaign
/// units are all XP/milestone entries plus all revealed facts. See
/// `docs/deviation.md`.
pub fn context(save_dir: &Path) -> Result<Context> {
    let sheets = check::sheets(save_dir);
    let party_level = sheets
        .iter()
        .map(|s| s.block.get("level").and_then(|v| v.as_i64()).unwrap_or(1) as i32)
        .min()
        .unwrap_or(1)
        .max(1);
    let p = Progress::load(save_dir)?;
    let track = progress::track(save_dir, &p);
    let (chapter, chapter_level) = if track.is_empty() {
        (UNCHAPTERED.to_string(), 0)
    } else {
        let i = track
            .iter()
            .rposition(|s| s.level <= party_level)
            .unwrap_or(0);
        (track[i].name.clone(), track[i].level)
    };
    let facts = reveal::load(save_dir)?.facts;
    let campaign_units = CanonUnits {
        xp: p.xp_log.len(),
        reveals: facts.len(),
    };
    let chapter_units = match p.xp_log.iter().rposition(|e| e.level.is_some()) {
        None => campaign_units,
        Some(i) => {
            let since = p.xp_log[i].date.as_str();
            CanonUnits {
                xp: p.xp_log.len() - i,
                reveals: facts.iter().filter(|f| f.date.as_str() >= since).count(),
            }
        }
    };
    Ok(Context {
        party_level,
        chapter,
        chapter_level,
        chapter_units,
        campaign_units,
    })
}

pub fn count_chapter(ledger: &Ledger, chapter: &str) -> usize {
    ledger
        .entries
        .iter()
        .filter(|e| e.chapter == chapter)
        .count()
}

/// Percentage of original content in a chapter/campaign, 0.0 when empty.
pub fn percent(original: usize, canon: usize) -> f64 {
    let total = original + canon;
    if total == 0 {
        0.0
    } else {
        original as f64 * 100.0 / total as f64
    }
}

/// The budget test after adding one more original element:
/// `original+1 <= floor(ratio% * (original+1+canon))`.
pub fn budget_ok(original: usize, canon: usize, ratio: u32) -> bool {
    (original + 1) * 100 <= ratio as usize * (original + 1 + canon)
}

/// Largest number of originals the budget allows at this canon count.
pub fn max_original(canon: usize, ratio: u32) -> usize {
    if ratio >= 100 {
        return usize::MAX;
    }
    ratio as usize * canon / (100 - ratio as usize)
}

pub struct New<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    /// `None` only for non-item types (defaults to common).
    pub rarity: Option<&'a str>,
    pub note: &'a str,
    /// DM-only hidden aspect (secret term source until revealed).
    pub hidden: &'a str,
    pub player_hook: bool,
}

/// Register one original element. Refuses when the scope, rarity cap or
/// per-chapter budget does not allow it.
pub fn add(save_dir: &Path, new: &New<'_>) -> Result<Vec<String>> {
    let dev = deviation(save_dir)?;
    dev.check()?;
    if dev.scope == 0 {
        bail!(
            "本存档是 scope 0（不偏离）：只能补必要空白（给未命名 NPC 起名、对白措辞、场景描述），不能登记原创内容。需要原创时只能在开新档时用 --scope 1 或 --scope 2；开档后不能修改。"
        );
    }
    let kind = Kind::parse(new.kind)?;
    if !allowed_kinds(dev.scope).contains(&kind) {
        bail!(
            "scope {} 不允许原创{}；允许的类型：{}",
            dev.scope,
            kind.zh(),
            allowed_kinds(dev.scope)
                .iter()
                .map(|k| k.zh())
                .collect::<Vec<_>>()
                .join("、")
        );
    }
    let rarity = match (kind, new.rarity) {
        (Kind::Item, None) => bail!("原创物品必须用 --rarity 给出稀有度（{}）", rarity_list()),
        (_, Some(r)) => Rarity::parse(r)?,
        (_, None) => Rarity::Common,
    };
    let ctx = context(save_dir)?;
    if let Some(cap) = effective_cap(dev.scope, ctx.party_level) {
        if rarity > cap {
            bail!(
                "原创{}「{}」的稀有度 {} 超过上限 {}（档位上限 {}；队伍等级 {} 的档位上限 {}；取较低者）。降低稀有度，或等队伍升级后再登记。",
                kind.zh(),
                new.name.trim(),
                rarity.label(),
                cap.label(),
                scope_cap(dev.scope).map(Rarity::zh).unwrap_or("无"),
                ctx.party_level,
                party_cap(ctx.party_level).zh()
            );
        }
    }
    let name = new.name.trim();
    if name.is_empty() {
        bail!("--name 不能为空");
    }
    let ratio = dev.ratio.context("scope 需要 ratio")?;
    let mut ledger = load(save_dir)?;
    let chapter_original = count_chapter(&ledger, &ctx.chapter);
    let canon = ctx.chapter_units.total();
    if !budget_ok(chapter_original, canon, ratio) {
        bail!(
            "本章原创预算已满：已有 {chapter_original} 项原创、{canon} 项正典进度，再加一项会超过 {ratio}%（本章原创 {}%）。回到主线（继续推进正典内容）后再登记。",
            format_args!("{:.1}", percent(chapter_original, canon))
        );
    }
    let next = ledger
        .entries
        .iter()
        .filter_map(|e| {
            e.id.strip_prefix("O-")
                .and_then(|n| n.parse::<usize>().ok())
        })
        .max()
        .unwrap_or(0)
        + 1;
    let entry = Entry {
        id: format!("O-{next:03}"),
        kind: kind.id().to_string(),
        name: name.to_string(),
        rarity: rarity.id().to_string(),
        chapter: ctx.chapter.clone(),
        note: new.note.trim().to_string(),
        hidden: Some(new.hidden.trim().to_string()).filter(|h| !h.is_empty()),
        player_hook: new.player_hook,
        logged: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
    };
    ledger.entries.push(entry.clone());
    store(save_dir, &ledger)?;
    let chapter_original = chapter_original + 1;
    let campaign_original = ledger.entries.len();
    let hook = if entry.player_hook {
        "，玩家钩子"
    } else {
        ""
    };
    let line = format!(
        "原创登记 {}：{}「{}」（{}，{}{hook}）",
        entry.id,
        kind.zh(),
        entry.name,
        rarity.id(),
        ctx.chapter
    );
    if let Some(log) = save::latest_log(save_dir) {
        save::append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(vec![
        line,
        format!(
            "本章原创 {chapter_original} 项，正典 {canon} 项，占比 {:.1}%（上限 {ratio}%）",
            percent(chapter_original, canon)
        ),
        format!(
            "全战役原创 {campaign_original} 项，正典 {} 项，占比 {:.1}%（仅参考）",
            ctx.campaign_units.total(),
            percent(campaign_original, ctx.campaign_units.total())
        ),
        "提醒：登记后才能引入；不要告诉玩家哪些内容是原创。".into(),
    ])
}

fn rarity_list() -> String {
    [
        Rarity::Common,
        Rarity::Uncommon,
        Rarity::Rare,
        Rarity::VeryRare,
        Rarity::Legendary,
    ]
    .iter()
    .map(|r| r.id())
    .collect::<Vec<_>>()
    .join(", ")
}

pub fn list(save_dir: &Path) -> Result<Vec<String>> {
    let ledger = load(save_dir)?;
    if ledger.entries.is_empty() {
        return Ok(vec!["（还没有登记原创内容）".into()]);
    }
    Ok(ledger
        .entries
        .iter()
        .map(|e| {
            let mut s = format!(
                "{}  {}  {}  {}  {}",
                e.id, e.kind, e.name, e.rarity, e.chapter
            );
            if e.player_hook {
                s += "  [玩家钩子]";
            }
            if e.hidden.is_some() {
                s += if revealed(save_dir, &e.id) {
                    "  [隐藏面已揭示]"
                } else {
                    "  [有隐藏面]"
                };
            }
            if !e.note.is_empty() {
                s += &format!("  — {}", e.note);
            }
            s += &format!("  ({})", e.logged);
            s
        })
        .collect())
}

/// Has this original element's hidden aspect been revealed? The reveal ledger
/// records it as a tag (`original:O-001`), written by
/// `talkdnd reveal add --original O-001`.
pub fn revealed(save_dir: &Path, id: &str) -> bool {
    let wanted = format!("original:{}", id.trim().to_ascii_lowercase());
    reveal::load(save_dir)
        .map(|l| {
            l.facts
                .iter()
                .any(|f| f.tags.iter().any(|t| t.eq_ignore_ascii_case(&wanted)))
        })
        .unwrap_or(false)
}

/// Entries the players have met: no hidden aspect, or the hidden aspect has
/// been revealed. Used by `talkdnd known` (never print the ledger itself).
pub fn known_entries(save_dir: &Path) -> Vec<Entry> {
    let Ok(ledger) = load(save_dir) else {
        return Vec::new();
    };
    ledger
        .entries
        .into_iter()
        .filter(|e| e.hidden.is_none() || revealed(save_dir, &e.id))
        .collect()
}

/// Create the local long-term plan file (`Saves/<slot>/dm-plan.md`, DM only,
/// gitignored) from a template when it does not exist yet.
pub fn plan(save_dir: &Path) -> Result<String> {
    let p = save_dir.join(PLAN_FILE);
    if p.is_file() {
        return Ok(format!(
            "DM 长期计划已在 `{}`；只给 DM 看，不要提交",
            p.display()
        ));
    }
    let text = "# DM 长期计划（只给 DM；已 gitignore，不要提交）\n\n\
本文件写本档的长期计划、伏笔和后续安排。`talkdnd secret` 会从本文件收集\n\
`- term: <词>` 行和 `[[<词>]]` 标记，作为防剧透检查的秘密词；\n\
例如：`- term: 馆长的真名` 或 `幕后黑手是 [[红面具]]`。\n\n\
## 计划\n\n\
# - term: <秘密词>（把行首的 # 去掉才生效）\n\n\
## 伏笔与回收\n\n";
    atomic_write(&p, text.as_bytes())?;
    Ok(format!(
        "已创建 `{}`（只给 DM，已 gitignore；用 `- term: X` 或 `[[X]]` 标记秘密词）",
        p.display()
    ))
}

fn log_text(save_dir: &Path) -> String {
    let mut out = String::new();
    let Ok(rd) = fs::read_dir(save_dir.join("Sessions")) else {
        return out;
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "md")
                && !p
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("README.md"))
        })
        .collect();
    files.sort();
    for f in files {
        if let Ok(t) = fs::read_to_string(f) {
            out.push_str(&t.to_lowercase());
            out.push('\n');
        }
    }
    out
}

/// `talkdnd check`: registered hidden aspects that appear verbatim in session
/// logs without a reveal record. Deliberately only the DM's own registered
/// text — never a scan of the adventure book.
pub fn leak_warnings(save_dir: &Path) -> Vec<String> {
    let Ok(ledger) = load(save_dir) else {
        return Vec::new();
    };
    let logs = log_text(save_dir);
    if logs.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for e in ledger.entries.iter().filter(|e| e.hidden.is_some()) {
        if revealed(save_dir, &e.id) {
            continue;
        }
        let hidden = e.hidden.as_deref().unwrap_or("").trim();
        let mut terms = vec![hidden.to_string()];
        let mut rest = hidden;
        while let Some(start) = rest.find("[[") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("]]") else {
                break;
            };
            terms.push(after[..end].trim().to_string());
            rest = &after[end + 2..];
        }
        let hit = terms
            .iter()
            .filter(|t| t.chars().count() >= 2)
            .find(|t| logs.contains(&t.to_lowercase()));
        if let Some(term) = hit {
            out.push(format!(
                "原创 {}「{}」的隐藏面（出现：{}）出现在会话日志里，但没有揭示记录；若玩家已经知道，运行 `talkdnd reveal add --original {}`（否则把它从玩家可见文本里删掉）",
                e.id, e.name, term, e.id
            ));
        }
    }
    out
}

/// DM-facing status: scope, caps, chapter budget and campaign total. Used by
/// `talkdnd original status` and the `talkdnd start` digest.
pub fn status_lines(save_dir: &Path) -> Result<Vec<String>> {
    let dev = deviation(save_dir)?;
    dev.check()?;
    let ctx = context(save_dir)?;
    let ledger = load(save_dir)?;
    let chapter_original = count_chapter(&ledger, &ctx.chapter);
    let campaign_original = ledger.entries.len();
    let canon = ctx.chapter_units.total();
    let mut out = vec![format!("剧本偏离度：{}", dev.describe())];
    out.push(format!(
        "当前章节：{}（队伍等级 {}）",
        ctx.chapter, ctx.party_level
    ));
    out.push(format!(
        "本章正典进度：{canon} 项（经验/里程碑 {} + 已揭示 {}）",
        ctx.chapter_units.xp, ctx.chapter_units.reveals
    ));
    match dev.scope {
        0 => {
            out.push("允许原创类型：无（scope 0 只补必要空白）".into());
            out.push("稀有度上限：不适用".into());
            out.push(format!(
                "原创登记：{campaign_original} 项（scope 0 不允许登记；补空白不算原创）"
            ));
        }
        scope => {
            out.push(format!(
                "允许原创类型：{}",
                allowed_kinds(scope)
                    .iter()
                    .map(|k| k.zh())
                    .collect::<Vec<_>>()
                    .join("、")
            ));
            let cap = effective_cap(scope, ctx.party_level);
            out.push(format!(
                "稀有度上限：{}（档位上限 {}；队伍等级 {} 上限 {}；取较低者）",
                cap.map(Rarity::label).unwrap_or_else(|| "无".into()),
                scope_cap(scope)
                    .map(Rarity::label)
                    .unwrap_or_else(|| "无".into()),
                ctx.party_level,
                party_cap(ctx.party_level).label()
            ));
            let ratio = dev.ratio.context("scope 需要 ratio")?;
            let pct = percent(chapter_original, canon);
            let over = chapter_original * 100 > ratio as usize * (chapter_original + canon);
            out.push(format!(
                "本章原创：{chapter_original} 项；正典 {canon} 项；占比 {pct:.1}%（上限 {ratio}%）{}",
                if over {
                    " — 已超限：回到主线"
                } else {
                    ""
                }
            ));
            out.push(format!(
                "全战役原创：{campaign_original} 项；正典 {} 项；占比 {:.1}%（仅参考）",
                ctx.campaign_units.total(),
                percent(campaign_original, ctx.campaign_units.total())
            ));
            let room = max_original(canon, ratio).saturating_sub(chapter_original);
            if room == 0 {
                out.push("本章还可登记：0 项 — 回到主线推进正典内容后再登记".into());
            } else {
                out.push(format!("本章还可登记：{room} 项"));
            }
        }
    }
    out.push(
        "关键锚点（主反派、核心秘密、结局方向）任何档位都不能改；登记前不要把原创当成书里的内容。"
            .into(),
    );
    Ok(out)
}

/// Validate `--original <id>` on `char … item add`: the id must exist in this
/// save's ledger, be an item entry, and the save must allow original content.
pub fn validate_item_reference(save_dir: &Path, id: &str) -> Result<Entry> {
    let dev = deviation(save_dir)?;
    dev.check()?;
    if dev.scope == 0 {
        bail!("本存档是 scope 0，不能把物品标为原创；先 `talkdnd original add --type item` 也不行");
    }
    let ledger = load(save_dir)?;
    let wanted = id.trim();
    let entry = ledger
        .entries
        .iter()
        .find(|e| e.id.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| {
            anyhow!(
                "原创台账里没有 `{wanted}`；先 `talkdnd original add --type item --name \"…\" --rarity <稀有度>`，再用 `--original {wanted}` 加入物品栏"
            )
        })?;
    if entry.kind != Kind::Item.id() {
        bail!(
            "`{wanted}` 的类型是 {}，不是物品（item）；物品只能用物品类型的原创条目",
            entry.kind
        );
    }
    Ok(entry.clone())
}

/// Does an item name exist in canon data? Weapons (SRD + local), armor,
/// shields and a short common-gear list. Magic items and book-specific gear
/// are not in the data files: pass `--canon` (the DM checked the book) or
/// register an original item and pass `--original`.
pub fn is_canon_item(name: &str, book: &crate::classdata::Book) -> bool {
    let names = [name.to_string()];
    if !crate::classdata::weapons_in(&names, book).is_empty() {
        return true;
    }
    if crate::charsheet::armor_of(name).is_some() || crate::charsheet::is_shield(name) {
        return true;
    }
    const GEAR: &[&str] = &[
        "arrow",
        "bolt",
        "bullet",
        "sling bullet",
        "dart",
        "potion",
        "healing potion",
        "antitoxin",
        "rope",
        "torch",
        "lantern",
        "candle",
        "oil",
        "tinderbox",
        "rations",
        "waterskin",
        "backpack",
        "bedroll",
        "blanket",
        "sack",
        "pouch",
        "quiver",
        "case",
        "map",
        "scroll",
        "spellbook",
        "book",
        "component pouch",
        "focus",
        "holy symbol",
        "tools",
        "kit",
        "crowbar",
        "hammer",
        "pitons",
        "grappling hook",
        "manacles",
        "chain",
        "caltrops",
        "ball bearings",
        "chalk",
        "flask",
        "vial",
        "mirror",
        "fishing tackle",
        "lamp",
        "clothes",
        "costume",
        "disguise",
        "signet ring",
        "magnifying glass",
        "telescope",
        "spyglass",
        "hourglass",
        "ink",
        "paper",
        "parchment",
        "sealing wax",
        "soap",
        "perfume",
        "scale",
        "weight",
        "bell",
        "whistle",
        "horn",
        "绳",
        "火把",
        "灯",
        "蜡烛",
        "油",
        "口粮",
        "水袋",
        "背包",
        "袋子",
        "箭",
        "弩矢",
        "弹丸",
        "药水",
        "治疗",
        "工具",
        "材料包",
        "圣徽",
        "书",
        "卷轴",
        "钩爪",
        "铁钉",
        "粉笔",
        "镜",
        "纸",
        "墨水",
    ];
    let l = name.to_lowercase();
    GEAR.iter().any(|g| l.contains(g))
}

#[derive(Debug, Default, Deserialize)]
struct SheetInventory {
    #[serde(default)]
    inventory: Vec<SheetItem>,
}

#[derive(Debug, Default, Deserialize)]
struct SheetItem {
    #[serde(default)]
    name: String,
    #[serde(default)]
    original: Option<String>,
}

fn sheet_original_items(save_dir: &Path) -> Vec<(String, String)> {
    let dir = save_dir.join("sheets");
    let Ok(rd) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        if f.extension().is_none_or(|x| x != "yaml") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        let Ok(sheet) = serde_norway::from_str::<SheetInventory>(&text) else {
            continue;
        };
        for item in sheet.inventory {
            if let Some(id) = item.original {
                out.push((item.name, id));
            }
        }
    }
    out
}

/// Warnings for `talkdnd check`: invalid deviation, ledger entries that the
/// scope/rarity cap does not allow, and inventory items marked original with
/// no matching ledger entry.
pub fn audit(save_dir: &Path, dev: &Deviation) -> Vec<String> {
    let mut w = Vec::new();
    if let Err(e) = dev.check() {
        w.push(format!("剧本偏离度设置无效：{e}"));
    }
    let ledger = match load(save_dir) {
        Ok(l) => l,
        Err(e) => {
            w.push(format!("原创台账读不了：{e}"));
            return w;
        }
    };
    let ctx = context(save_dir).ok();
    let party_level = ctx.as_ref().map(|c| c.party_level).unwrap_or(1);
    let cap = effective_cap(dev.scope, party_level);
    for e in &ledger.entries {
        match Kind::parse(&e.kind) {
            Ok(kind) => {
                if !allowed_kinds(dev.scope).contains(&kind) {
                    w.push(format!(
                        "原创台账 {}（{}）的类型 {} 不属于 scope {} 允许范围",
                        e.id, e.name, e.kind, dev.scope
                    ));
                }
            }
            Err(_) => w.push(format!(
                "原创台账 {}（{}）的类型 `{}` 无效",
                e.id, e.name, e.kind
            )),
        }
        match Rarity::parse(&e.rarity) {
            Ok(rarity) => {
                if let Some(cap) = cap {
                    if rarity > cap {
                        w.push(format!(
                            "原创台账 {}（{}）的稀有度 {} 超过上限 {}（scope {}，队伍等级 {}）",
                            e.id,
                            e.name,
                            e.rarity,
                            cap.id(),
                            dev.scope,
                            party_level
                        ));
                    }
                }
            }
            Err(_) => w.push(format!(
                "原创台账 {}（{}）的稀有度 `{}` 无效",
                e.id, e.name, e.rarity
            )),
        }
    }
    for (name, id) in sheet_original_items(save_dir) {
        match ledger
            .entries
            .iter()
            .find(|e| e.id.eq_ignore_ascii_case(&id))
        {
            None => w.push(format!(
                "角色卡物品「{name}」标为原创 {id}，但原创台账里没有这条登记"
            )),
            Some(e) if e.kind != Kind::Item.id() => w.push(format!(
                "角色卡物品「{name}」标为原创 {id}，但该条目的类型是 {}，不是物品",
                e.kind
            )),
            Some(_) => {}
        }
    }
    w.extend(leak_warnings(save_dir));
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::{self, Deviation};

    fn save_dir(tag: &str, scope: u32, ratio: Option<u32>) -> PathBuf {
        let d = std::env::temp_dir().join(format!("talkdnd-original-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for sub in ["Characters", "Sessions", "combat", "sheets"] {
            fs::create_dir_all(d.join(sub)).unwrap();
        }
        fs::write(
            d.join("combat/campaign.yaml"),
            "party: [hero]\nruleset: \"2014\"\n",
        )
        .unwrap();
        fs::write(
            d.join("Characters/Hero.md"),
            "# Hero\n\n```combat yaml\nid: hero\nname: Hero\nlevel: 5\nhp: 30\n```\n",
        )
        .unwrap();
        fs::write(d.join("Sessions/s1.md"), "# log\n").unwrap();
        fs::write(d.join("progress.yaml"), "version: 1\nxp: {hero: 0}\n").unwrap();
        migrate::init_with_deviation(
            &d,
            &Deviation::new(Some(scope), ratio).expect("valid deviation"),
        )
        .unwrap();
        d
    }

    fn canon(d: &Path, xp: usize, facts: usize) {
        let mut p = Progress::load(d).unwrap();
        for i in 0..xp {
            p.xp_log.push(progress::XpEntry {
                date: format!("2026-10-22 10:{i:02}"),
                category: "quest".into(),
                reason: format!("unit {i}"),
                each: 10,
                to: vec!["hero".into()],
                difficulty: None,
                level: None,
                capped: Default::default(),
            });
        }
        p.store(d).unwrap();
        let mut ledger = reveal::Ledger::default();
        for i in 0..facts {
            ledger.facts.push(reveal::Fact {
                n: i + 1,
                date: format!("2026-10-22 11:{i:02}"),
                fact: format!("fact {i}"),
                source: String::new(),
                tags: Vec::new(),
            });
        }
        reveal::store(d, &ledger).unwrap();
    }

    fn add(d: &Path, kind: &str, name: &str, rarity: Option<&str>) -> Result<Vec<String>> {
        super::add(
            d,
            &New {
                kind,
                name,
                rarity,
                note: "",
                hidden: "",
                player_hook: false,
            },
        )
    }

    #[test]
    fn party_and_scope_caps_take_the_lower_value() {
        assert_eq!(party_cap(1), Rarity::Uncommon);
        assert_eq!(party_cap(4), Rarity::Uncommon);
        assert_eq!(party_cap(5), Rarity::Rare);
        assert_eq!(party_cap(10), Rarity::Rare);
        assert_eq!(party_cap(11), Rarity::VeryRare);
        assert_eq!(party_cap(16), Rarity::VeryRare);
        assert_eq!(party_cap(17), Rarity::Legendary);
        assert_eq!(effective_cap(1, 17), Some(Rarity::Uncommon));
        assert_eq!(effective_cap(2, 1), Some(Rarity::Uncommon));
        assert_eq!(effective_cap(2, 5), Some(Rarity::Rare));
        assert_eq!(effective_cap(2, 17), Some(Rarity::Rare));
        assert_eq!(effective_cap(0, 17), None);
    }

    #[test]
    fn scope_type_and_rarity_rules() {
        let d = save_dir("rules", 1, Some(15));
        canon(&d, 20, 0);
        let e = add(&d, "sidequest", "Heist", None).unwrap_err().to_string();
        assert!(e.contains("scope 1") && e.contains("支线"), "{e}");
        let e = add(&d, "encounter", "Bridge", Some("rare"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("上限") && e.contains("rare"), "{e}");
        let e = add(&d, "item", "Moonblade", None).unwrap_err().to_string();
        assert!(e.contains("--rarity"), "{e}");
        add(&d, "encounter", "Bridge", Some("uncommon")).unwrap();
        add(&d, "npc", "Miller", None).unwrap();
        let entries = list(&d).unwrap().join("\n");
        assert!(
            entries.contains("O-001") && entries.contains("O-002"),
            "{entries}"
        );

        let dev = deviation(&d).unwrap();
        assert_eq!(dev.scope, 1);
        assert_eq!(dev.ratio, Some(15));
        let e = add(&d, "location", "Mill", Some("legendary"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("传奇") || e.contains("legendary"), "{e}");

        let zero = save_dir("zero", 0, None);
        let e = add(&zero, "encounter", "X", None).unwrap_err().to_string();
        assert!(e.contains("scope 0") && e.contains("不偏离"), "{e}");

        // Scope 2 allows side quests; party level 5 caps rarity at rare.
        let two = save_dir("scope2", 2, Some(25));
        canon(&two, 3, 1);
        add(&two, "sidequest", "Missing Heir", Some("rare")).unwrap();
        let e = add(&two, "item", "Crown", Some("very-rare"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("上限") && e.contains("very-rare"), "{e}");
    }

    #[test]
    fn budget_refuses_until_canon_progress_catches_up() {
        let d = save_dir("budget", 2, Some(5));
        let e = add(&d, "encounter", "Ambush", None)
            .unwrap_err()
            .to_string();
        assert!(e.contains("预算") && e.contains("回到主线"), "{e}");
        canon(&d, 20, 0);
        assert!(budget_ok(0, 20, 5));
        add(&d, "encounter", "Ambush", None).unwrap();
        let log = fs::read_to_string(d.join("Sessions/s1.md")).unwrap();
        assert!(
            log.contains("原创登记 O-001") && log.contains("Ambush"),
            "{log}"
        );
        // original 1, canon 20, ratio 5: a second would be 2/22 = 9.1%.
        let e = add(&d, "npc", "Guide", None).unwrap_err().to_string();
        assert!(e.contains("预算") && e.contains("回到主线"), "{e}");
        assert_eq!(max_original(20, 5), 1);
        assert_eq!(percent(1, 20), 100.0 / 21.0);

        let status = status_lines(&d).unwrap().join("\n");
        assert!(status.contains("scope 2"), "{status}");
        assert!(status.contains("比例上限 5%"), "{status}");
        assert!(
            status.contains("允许原创类型：遭遇、NPC、物品、地点、支线"),
            "{status}"
        );
        assert!(status.contains("本章原创：1 项"), "{status}");
        assert!(status.contains("全战役原创：1 项"), "{status}");
        assert!(status.contains("回到主线"), "{status}");
    }

    #[test]
    fn chapters_and_campaign_totals() {
        let d = save_dir("chapters", 2, Some(25));
        fs::write(
            d.join("Characters/Hero.md"),
            "# Hero\n\n```combat yaml\nid: hero\nname: Hero\nlevel: 2\nhp: 20\n```\n",
        )
        .unwrap();
        // A Golden Vault track: chapters by expected party level.
        fs::write(
            d.join("combat/campaign.yaml"),
            "party: [hero]\nruleset: \"2014\"\nadventures: [../../../Adventures/keys-from-the-golden-vault/combat]\n",
        )
        .unwrap();
        let mut p = Progress::load(&d).unwrap();
        p.xp_log.push(progress::XpEntry {
            date: "2026-10-22 10:00".into(),
            category: "milestone-major".into(),
            reason: "chapter one done".into(),
            each: 0,
            to: vec!["hero".into()],
            difficulty: None,
            level: Some(2),
            capped: Default::default(),
        });
        p.store(&d).unwrap();
        let c = context(&d).unwrap();
        assert_eq!(c.chapter, "The Stygian Gambit");
        assert_eq!(c.chapter_level, 2);
        assert_eq!(c.chapter_units.xp, 1);
        assert_eq!(c.campaign_units.xp, 1);
        let status = status_lines(&d).unwrap().join("\n");
        assert!(status.contains("The Stygian Gambit"), "{status}");
        assert!(status.contains("本章正典进度：1 项"), "{status}");
    }

    #[test]
    fn hidden_aspects_are_secret_and_leaks_are_flagged() {
        let d = save_dir("hidden", 2, Some(25));
        canon(&d, 20, 0);
        super::add(
            &d,
            &New {
                kind: "npc",
                name: "Guide",
                rarity: None,
                note: "",
                hidden: "Guide is [[Red Mask]]",
                player_hook: false,
            },
        )
        .unwrap();
        let text = fs::read_to_string(d.join(FILE)).unwrap();
        assert!(
            text.contains("version: 2") && text.contains("hidden:"),
            "{text}"
        );
        // A hidden entry is not "introduced" until its hidden aspect is revealed.
        assert!(!known_entries(&d).iter().any(|e| e.id == "O-001"));
        assert!(!revealed(&d, "O-001"));
        // The DM's own plan template and terms are parsed; a leak is flagged.
        fs::write(
            d.join("Sessions/s1.md"),
            "# log\n- [20:00] 玩家听见有人叫出 Red Mask\n",
        )
        .unwrap();
        let leaks = leak_warnings(&d);
        assert!(
            leaks
                .iter()
                .any(|w| w.contains("O-001") && w.contains("Red Mask")),
            "{leaks:?}"
        );
        let dev = deviation(&d).unwrap();
        assert!(audit(&d, &dev).join("\n").contains("O-001"));
        // Revealing the hidden aspect clears the leak and lists it in `known`.
        crate::reveal::add_original(&d, "O-001", None, "test reveal").unwrap();
        assert!(revealed(&d, "O-001"));
        assert!(known_entries(&d).iter().any(|e| e.id == "O-001"));
        assert!(leak_warnings(&d).is_empty());
        assert!(list(&d).unwrap().join("\n").contains("隐藏面已揭示"));
        // The plan file is created once, with only commented term examples.
        let msg = plan(&d).unwrap();
        assert!(
            msg.contains("dm-plan.md") && d.join(PLAN_FILE).is_file(),
            "{msg}"
        );
        assert!(plan(&d).unwrap().contains("长期计划"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn audit_finds_tampering_and_unknown_original_items() {
        let d = save_dir("audit", 1, Some(15));
        let mut ledger = Ledger::default();
        ledger.entries.push(Entry {
            id: "O-001".into(),
            kind: "sidequest".into(),
            name: "Hand edited".into(),
            rarity: "legendary".into(),
            chapter: UNCHAPTERED.into(),
            note: String::new(),
            hidden: None,
            player_hook: false,
            logged: String::new(),
        });
        store(&d, &ledger).unwrap();
        fs::write(
            d.join("sheets/hero.yaml"),
            "version: 1\nid: hero\ninventory:\n  - {name: Moonblade, original: O-999}\n",
        )
        .unwrap();
        let mut dev = deviation(&d).unwrap();
        let w = audit(&d, &dev);
        let text = w.join("\n");
        assert!(text.contains("不属于 scope 1"), "{text}");
        assert!(text.contains("超过上限"), "{text}");
        assert!(text.contains("O-999"), "{text}");
        dev.scope = 3;
        assert!(audit(&d, &dev).join("\n").contains("设置无效"));
    }

    #[test]
    fn item_reference_must_be_an_item_entry() {
        let d = save_dir("ref", 1, Some(25));
        canon(&d, 5, 0);
        add(&d, "npc", "Guide", None).unwrap();
        let e = validate_item_reference(&d, "O-001")
            .unwrap_err()
            .to_string();
        assert!(e.contains("npc") && e.contains("不是物品"), "{e}");
        let e = validate_item_reference(&d, "O-999")
            .unwrap_err()
            .to_string();
        assert!(e.contains("O-999") && e.contains("original add"), "{e}");
    }
}
