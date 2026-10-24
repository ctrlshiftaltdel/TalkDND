//! Story entities (P04; LetsPlay/docs/campaign-continuity.md §4.3/§5.3).
//!
//! `talkdnd entity ...` keeps the stable-id ledger of NPCs, locations,
//! factions, quests and items in `Saves/<slot>/entities.yaml` (version 1).
//! Name/alias clashes are refused and never auto-merged; `rename` keeps the
//! old name as an alias and logs the change; `retire` keeps the record.
//! Default outputs are the player view: `dm_only` entities never appear and
//! retired entities stay out of the default list. Candidate suggestions from
//! logs or handouts are never written automatically — `entity add` is the
//! only write path, and re-applying the same suggestion is idempotent.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "entities.yaml";
pub const VERSION: u32 = 1;
pub const TYPES: [&str; 5] = ["npc", "location", "faction", "quest", "item"];

/// One entity row (contract §4.3; field names frozen).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub first_seen: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub dm_only: bool,
}

/// `entities.yaml`, version 1.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Ledger {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entities: Vec<Entity>,
}

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Register a new entity (name/alias conflicts are rejected)
    Add {
        /// npc | location | faction | quest | item
        #[arg(long = "type")]
        kind: String,
        #[arg(long)]
        name: String,
        /// Alternative name (repeatable)
        #[arg(long)]
        alias: Vec<String>,
        /// Where or when it first appeared
        #[arg(long)]
        first_seen: Option<String>,
        /// DM-only entity (never in the player view)
        #[arg(long)]
        dm_only: bool,
    },
    /// List entities (default: active only)
    List {
        /// Only this type: npc | location | faction | quest | item
        #[arg(long = "type")]
        kind: Option<String>,
        /// Include retired entities (marked)
        #[arg(long)]
        all: bool,
    },
    /// Show every field of one entity and its relation count
    Show { id: String },
    /// Rename an entity (the old name becomes an alias)
    Rename {
        /// Entity id or name
        id: String,
        /// The new name
        name: String,
    },
    /// Retire an entity (the record is kept)
    Retire { id: String },
}

/// `entity add|list|show|rename|retire` (P04, TASK-0062).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = root;
    let lines = match cmd {
        Cmd::Add {
            kind,
            name,
            alias,
            first_seen,
            dm_only,
        } => vec![add(
            save,
            &kind,
            &name,
            &alias,
            first_seen.as_deref(),
            dm_only,
        )?],
        Cmd::List { kind, all } => list(save, kind.as_deref(), all)?,
        Cmd::Show { id } => show(save, &id)?,
        Cmd::Rename { id, name } => vec![rename(save, &id, &name)?],
        Cmd::Retire { id } => vec![retire(save, &id)?],
    };
    for l in lines {
        println!("{l}");
    }
    Ok(())
}

// ------------------------------------------------------------------- ledger

fn path(save: &Path) -> PathBuf {
    save.join(FILE)
}

/// Load `entities.yaml`; a missing file is an empty ledger, a newer schema
/// version is refused.
pub fn load(save: &Path) -> Result<Ledger> {
    let p = path(save);
    if !p.is_file() {
        return Ok(Ledger::default());
    }
    let ledger: Ledger = serde_norway::from_str(
        &fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?,
    )
    .with_context(|| format!("invalid {}", p.display()))?;
    if ledger.version > VERSION {
        bail!(
            "{}: unsupported entity schema version {} (this talkdnd supports {VERSION}); update talkdnd",
            p.display(),
            ledger.version
        );
    }
    Ok(ledger)
}

fn store(save: &Path, ledger: &Ledger) -> Result<()> {
    let mut current = ledger.clone();
    current.version = VERSION;
    let text = format!(
        "# TalkDND entity ledger: stable ids for story NPCs, locations, factions, quests, items.
# Write with `talkdnd entity ...`; hand edits are reported by `talkdnd check`.
{}",
        serde_norway::to_string(&current)?
    );
    atomic_write(path(save), text.as_bytes())
}

fn next_id(ledger: &Ledger) -> String {
    let max = ledger
        .entities
        .iter()
        .filter_map(|e| e.id.strip_prefix("E-").and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("E-{:03}", max + 1)
}

// ------------------------------------------------------------- session model

/// The current session: the newest log file name (contract §2.1).
pub fn current_session(save: &Path) -> Result<String> {
    let log = crate::save::log_path(save)?;
    Ok(log
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string())
}

/// Compare key for stored session values: drop any `#anchor`.
pub fn session_key(session: &str) -> String {
    session
        .trim()
        .split('#')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Session log file names in Sessions/, sorted (README.md excluded).
fn log_names(save: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(save.join("Sessions"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".md") && !n.eq_ignore_ascii_case("README.md"))
        .collect();
    v.sort();
    v
}

/// Resolve user input to a canonical session identifier (contract §2.1: the
/// log file name): strip any `#anchor`, then match the rest against the
/// session logs — an exact name wins, otherwise a unique prefix or stem.
/// Unmatched input (a future session without a file yet) compares as
/// `<input>.md`.
pub fn resolve_session(save: &Path, input: &str) -> String {
    let base = input.trim().split('#').next().unwrap_or("").trim();
    if base.is_empty() || base.ends_with(".md") {
        return base.to_string();
    }
    let mut hits: Vec<String> = log_names(save)
        .into_iter()
        .filter(|n| n == base || n.starts_with(base) || n.strip_suffix(".md") == Some(base))
        .collect();
    hits.dedup();
    if hits.len() == 1 {
        hits.remove(0)
    } else {
        format!("{base}.md")
    }
}

/// Append one line to the newest session log (skipped when there is none).
pub(crate) fn log_line(save: &Path, line: &str) -> Result<()> {
    if let Some(log) = crate::save::latest_log(save) {
        crate::save::append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(())
}

// --------------------------------------------------------------- resolution

/// Canonical entity id from loose input: `e1`, `E-1`, `e-001` → `E-001`.
fn canonical_id(input: &str) -> Option<String> {
    canonical_prefixed_id('e', input)
}

/// Shared id normalizer for the `E-`/`R-` families: `r1`, `R-1`, `r-001`
/// → `R-001`. Non-numeric input is not an id.
pub(crate) fn canonical_prefixed_id(prefix: char, input: &str) -> Option<String> {
    let s = input.trim();
    let rest = s
        .strip_prefix(prefix.to_ascii_uppercase())
        .or_else(|| s.strip_prefix(prefix.to_ascii_lowercase()))?;
    let rest = rest.strip_prefix('-').unwrap_or(rest);
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = rest.parse().ok()?;
    Some(format!("{}-{n:03}", prefix.to_ascii_uppercase()))
}

/// Resolve `key` (id, name or alias, case-insensitive) to one entity.
/// Ambiguous keys are refused with the candidates; nothing is guessed.
pub fn resolve<'a>(ledger: &'a Ledger, key: &str) -> Result<&'a Entity> {
    let key = key.trim();
    if key.is_empty() {
        bail!("需要实体 id 或名称");
    }
    if let Some(id) = canonical_id(key) {
        if let Some(e) = ledger.entities.iter().find(|e| e.id == id) {
            return Ok(e);
        }
    }
    let lower = key.to_lowercase();
    let hits: Vec<&Entity> = ledger
        .entities
        .iter()
        .filter(|e| {
            e.name.to_lowercase() == lower || e.aliases.iter().any(|a| a.to_lowercase() == lower)
        })
        .collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => bail!("没有实体 `{key}`；`entity list` 看全部，或用实体 id"),
        many => bail!(
            "`{key}` 有歧义，请用实体 id 指定：{}",
            many.iter()
                .map(|e| format!("{}（{}）", e.id, e.name))
                .collect::<Vec<_>>()
                .join("、")
        ),
    }
}

/// Entities whose name or one of their aliases equals `name`
/// (case-insensitive), as human-readable clash lines. `exclude` skips one id
/// (the entity being renamed).
fn clash_lines(ledger: &Ledger, name: &str, exclude: Option<&str>) -> Vec<String> {
    let lower = name.trim().to_lowercase();
    let mut out = Vec::new();
    for e in &ledger.entities {
        if Some(e.id.as_str()) == exclude {
            continue;
        }
        if e.name.to_lowercase() == lower {
            out.push(format!("{}（{}）的主名", e.id, e.name));
        } else if let Some(a) = e.aliases.iter().find(|a| a.to_lowercase() == lower) {
            out.push(format!("{}（{}）的别名 `{a}`", e.id, e.name));
        }
    }
    out
}

/// `E-001（Mira）`; unknown ids (hand-edited ledgers) render with `（?）`.
pub fn display(ledger: &Ledger, id: &str) -> String {
    match ledger.entities.iter().find(|e| e.id == id) {
        Some(e) => format!("{}（{}）", e.id, e.name),
        None => format!("{id}（?）"),
    }
}

// ----------------------------------------------------------------- commands

/// `entity add`: register one entity. Re-adding the same primary name and
/// type returns the existing id (idempotent candidate application); any other
/// name/alias clash is refused with the conflicting entities.
pub fn add(
    save: &Path,
    kind: &str,
    name: &str,
    aliases: &[String],
    first_seen: Option<&str>,
    dm_only: bool,
) -> Result<String> {
    let kind = kind.trim().to_lowercase();
    if !TYPES.contains(&kind.as_str()) {
        bail!("--type 必须是 {} 之一", TYPES.join(" / "));
    }
    let name = name.trim();
    if name.is_empty() {
        bail!("--name 不能为空");
    }
    let trimmed: Vec<String> = aliases.iter().map(|a| a.trim().to_string()).collect();
    let mut aliases: Vec<String> = Vec::new();
    for a in trimmed {
        if a.is_empty() {
            bail!("--alias 不能为空");
        }
        if a.to_lowercase() == name.to_lowercase() {
            bail!("别名 `{a}` 与主名相同");
        }
        if !aliases.iter().any(|x| x.to_lowercase() == a.to_lowercase()) {
            aliases.push(a);
        }
    }
    let ledger = load(save)?;
    if let Some(e) = ledger
        .entities
        .iter()
        .find(|e| e.kind == kind && e.name.to_lowercase() == name.to_lowercase())
    {
        return Ok(format!(
            "已存在：{}（{} {}）；未改动，返回既有 id",
            e.id, e.kind, e.name
        ));
    }
    let mut clashes = clash_lines(&ledger, name, None);
    for a in &aliases {
        clashes.extend(clash_lines(&ledger, a, None));
    }
    if !clashes.is_empty() {
        bail!(
            "名称/别名冲突，未登记（不自动合并；请改名，或直接使用既有实体）：
- {}",
            clashes.join(
                "
- "
            )
        );
    }
    let first_seen = match first_seen.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => current_session(save)?,
    };
    let entity = Entity {
        id: next_id(&ledger),
        kind: kind.clone(),
        name: name.to_string(),
        aliases,
        first_seen: first_seen.clone(),
        status: "active".into(),
        dm_only,
    };
    let line = entity_line(&entity, false);
    let id = entity.id.clone();
    let mut entities = ledger.entities;
    entities.push(entity);
    store(
        save,
        &Ledger {
            version: VERSION,
            entities,
        },
    )?;
    log_line(
        save,
        &format!("实体 {id} 登记：{kind}「{name}」（首见 {first_seen}）"),
    )?;
    Ok(line)
}

fn entity_line(e: &Entity, all: bool) -> String {
    let mut line = format!("{}  {}  {}", e.id, e.kind, e.name);
    if !e.aliases.is_empty() {
        line += &format!("（别名：{}）", e.aliases.join("、"));
    }
    if all {
        if e.status == "retired" {
            line += "  [retired]";
        }
        if e.dm_only {
            line += "  [DM-only]";
        }
    }
    line
}

/// `entity list`: default is the player view (active, non-dm_only);
/// `--all` is the DM view with status and dm_only markers.
pub fn list(save: &Path, kind: Option<&str>, all: bool) -> Result<Vec<String>> {
    let ledger = load(save)?;
    let kind = kind.map(|k| k.trim().to_lowercase());
    if let Some(k) = &kind {
        if !TYPES.contains(&k.as_str()) {
            bail!("--type 必须是 {} 之一", TYPES.join(" / "));
        }
    }
    let mut out = Vec::new();
    for e in &ledger.entities {
        if let Some(k) = &kind {
            if e.kind != *k {
                continue;
            }
        }
        if !all && (e.dm_only || e.status == "retired") {
            continue;
        }
        out.push(entity_line(e, all));
    }
    if out.is_empty() {
        out.push("（没有符合条件的实体；`entity add --type npc --name …` 登记）".into());
    }
    Ok(out)
}

/// `entity show`: every field of one entity plus its active relation count.
/// dm_only entities are refused so their details cannot reach the player view.
pub fn show(save: &Path, key: &str) -> Result<Vec<String>> {
    let ledger = load(save)?;
    let e = resolve(&ledger, key)?;
    if e.dm_only {
        bail!(
            "{} 是 DM-only 实体；不在玩家视图输出（`entity list --all` 可见）",
            e.id
        );
    }
    let session = current_session(save)?;
    let relations = crate::graph::load(save)?;
    let active = relations
        .relations
        .iter()
        .filter(|r| !r.dm_only && (r.from == e.id || r.to == e.id))
        .filter(|r| crate::graph::active_at(r, &session))
        .count();
    let mut out = vec![entity_line(e, false)];
    out.push(format!("- 首见：{}", e.first_seen));
    out.push(format!(
        "- 状态：{}",
        if e.status == "retired" {
            "retired"
        } else {
            "active"
        }
    ));
    out.push(format!("- 生效关系：{active} 条"));
    Ok(out)
}

/// `entity rename`: the old name becomes an alias and the change is logged;
/// repeating the same rename is a no-op. Clashes with other entities are
/// refused; renaming to one of its own aliases promotes that alias.
pub fn rename(save: &Path, key: &str, new_name: &str) -> Result<String> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        bail!("新名称不能为空");
    }
    let mut ledger = load(save)?;
    let e = resolve(&ledger, key)?;
    let (id, old_name) = (e.id.clone(), e.name.clone());
    if old_name.to_lowercase() == new_name.to_lowercase() {
        return Ok(format!("{id} 已名「{new_name}」；未改动"));
    }
    let clashes = clash_lines(&ledger, new_name, Some(&id));
    if !clashes.is_empty() {
        bail!(
            "名称/别名冲突，未改名（不自动合并）：
- {}",
            clashes.join(
                "
- "
            )
        );
    }
    let pos = ledger
        .entities
        .iter()
        .position(|x| x.id == id)
        .context("entity vanished mid-rename")?;
    let e = &mut ledger.entities[pos];
    e.aliases
        .retain(|a| a.to_lowercase() != new_name.to_lowercase());
    if !e
        .aliases
        .iter()
        .any(|a| a.to_lowercase() == old_name.to_lowercase())
    {
        e.aliases.push(old_name.clone());
    }
    e.name = new_name.to_string();
    store(save, &ledger)?;
    log_line(
        save,
        &format!("实体 {id} 改名：{old_name} → {new_name}（旧名保留为别名）"),
    )?;
    Ok(format!(
        "{id} 改名：{old_name} → {new_name}（旧名保留为别名）"
    ))
}

/// `entity retire`: only the status changes; the record is kept. Retiring a
/// retired entity is a no-op.
pub fn retire(save: &Path, key: &str) -> Result<String> {
    let mut ledger = load(save)?;
    let e = resolve(&ledger, key)?;
    let (id, name) = (e.id.clone(), e.name.clone());
    if e.status == "retired" {
        return Ok(format!("{id}（{name}）已退役；未改动"));
    }
    let pos = ledger
        .entities
        .iter()
        .position(|x| x.id == id)
        .context("entity vanished mid-retire")?;
    ledger.entities[pos].status = "retired".into();
    store(save, &ledger)?;
    log_line(save, &format!("实体 {id}（{name}）退役；记录保留"))?;
    Ok(format!("{id}（{name}）已退役；记录保留"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_keys_drop_anchors_only() {
        assert_eq!(
            session_key("2026-10-24-session-01.md"),
            "2026-10-24-session-01.md"
        );
        assert_eq!(
            session_key("2026-10-24-session-01.md#宴会厅"),
            "2026-10-24-session-01.md"
        );
        assert_eq!(session_key("  x  "), "x");
    }

    #[test]
    fn loose_session_input_resolves_to_log_names() {
        let d = std::env::temp_dir().join(format!("talkdnd-session-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Sessions")).unwrap();
        for name in [
            "2030-01-02-session-00-setup.md",
            "2030-01-02-session-01.md",
            "README.md",
        ] {
            fs::write(d.join("Sessions").join(name), "# s\n").unwrap();
        }
        assert_eq!(
            resolve_session(&d, "2030-01-02-session-00"),
            "2030-01-02-session-00-setup.md"
        );
        assert_eq!(
            resolve_session(&d, "2030-01-02-session-01#酒馆"),
            "2030-01-02-session-01.md"
        );
        assert_eq!(
            resolve_session(&d, "2030-01-02-session-01.md"),
            "2030-01-02-session-01.md"
        );
        // A future session without a file falls back to <input>.md.
        assert_eq!(
            resolve_session(&d, "2030-01-02-session-02"),
            "2030-01-02-session-02.md"
        );
        // Ambiguous prefixes stay as given (plus .md).
        assert_eq!(resolve_session(&d, "2030-01-02"), "2030-01-02.md");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn canonical_ids_accept_loose_input() {
        assert_eq!(canonical_id("e1"), Some("E-001".into()));
        assert_eq!(canonical_id("E-12"), Some("E-012".into()));
        assert_eq!(canonical_id("e-003"), Some("E-003".into()));
        assert_eq!(canonical_id("Mira"), None);
        assert_eq!(canonical_id("e"), None);
        assert_eq!(canonical_id("e-x"), None);
    }
}
