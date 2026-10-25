//! Relations between entities (P04; LetsPlay/docs/campaign-continuity.md §4.4/§5.3).
//!
//! `talkdnd relation ...` records member/ally/enemy/located_at/holds/seeks/
//! knows links between entities in `Saves/<slot>/relations.yaml` (version 1).
//! A relation is in force at session `S` iff `since_session <= S` and
//! (`until_session` is empty or `S < until_session`), compared as file-name
//! strings. Ending a relation writes `until_session` and keeps the row —
//! corrections are `end` plus a note, never deletions. Default outputs are
//! the player view: `dm_only` relations are hidden and `--all` is the DM view.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::entity::{self, Entity, Ledger};

pub const FILE: &str = "relations.yaml";
pub const VERSION: u32 = 1;
pub const TYPES: [&str; 7] = [
    "member",
    "ally",
    "enemy",
    "located_at",
    "holds",
    "seeks",
    "knows",
];

/// One relation row (contract §4.4; field names frozen).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Relation {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub since_session: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub until_session: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default)]
    pub dm_only: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// `relations.yaml`, version 1.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Relations {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub relations: Vec<Relation>,
}

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Add a relation (idempotent: an existing identical relation keeps its id)
    Add {
        /// member | ally | enemy | located_at | holds | seeks | knows
        #[arg(long = "type")]
        kind: String,
        /// Source entity id or name
        #[arg(long)]
        from: String,
        /// Target entity id or name
        #[arg(long)]
        to: String,
        /// Session the relation started in
        #[arg(long)]
        since: Option<String>,
        /// Where this knowledge comes from
        #[arg(long)]
        source: Option<String>,
        /// Note (logged)
        #[arg(long)]
        note: Option<String>,
        /// DM-only relation (never in the player view)
        #[arg(long)]
        dm_only: bool,
    },
    /// List relations (default: active in the current session)
    List {
        /// Only relations of this entity
        #[arg(long)]
        entity: Option<String>,
        /// Only relations active at this session
        #[arg(long)]
        at: Option<String>,
        /// Include ended relations
        #[arg(long)]
        all: bool,
    },
    /// End a relation (writes until_session; the record is kept)
    End {
        /// Relation id
        id: String,
        /// Session the relation ended in
        #[arg(long)]
        at: Option<String>,
        /// Note (logged)
        #[arg(long)]
        note: Option<String>,
    },
    /// Show every field of one relation
    Show { id: String },
    /// Entity summary plus its currently active relations (player-filtered)
    Context {
        /// Entity id or name
        entity: String,
    },
}

/// `relation add|list|end|show|context` (P04, TASK-0062).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = root;
    let lines = match cmd {
        Cmd::Add {
            kind,
            from,
            to,
            since,
            source,
            note,
            dm_only,
        } => vec![add(
            save,
            &kind,
            &from,
            &to,
            since.as_deref(),
            source.as_deref(),
            note.as_deref(),
            dm_only,
        )?],
        Cmd::List { entity, at, all } => list(save, entity.as_deref(), at.as_deref(), all)?,
        Cmd::End { id, at, note } => vec![end(save, &id, at.as_deref(), note.as_deref())?],
        Cmd::Show { id } => show(save, &id)?,
        Cmd::Context { entity } => context(save, &entity)?,
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

/// Load `relations.yaml`; a missing file is an empty ledger, a newer schema
/// version is refused.
pub fn load(save: &Path) -> Result<Relations> {
    let p = path(save);
    if !p.is_file() {
        return Ok(Relations::default());
    }
    let ledger: Relations = serde_norway::from_str(
        &fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?,
    )
    .with_context(|| format!("invalid {}", p.display()))?;
    if ledger.version > VERSION {
        bail!(
            "{}: unsupported relation schema version {} (this talkdnd supports {VERSION}); update talkdnd",
            p.display(),
            ledger.version
        );
    }
    Ok(ledger)
}

fn store(save: &Path, ledger: &Relations) -> Result<()> {
    let mut current = ledger.clone();
    current.version = VERSION;
    let text = format!(
        "# TalkDND relation ledger: time-aware links between entities (write with `talkdnd relation ...`).
# Corrections end a relation with a note; rows are never deleted.
{}",
        serde_norway::to_string(&current)?
    );
    atomic_write(path(save), text.as_bytes())
}

fn next_id(ledger: &Relations) -> String {
    let max = ledger
        .relations
        .iter()
        .filter_map(|r| r.id.strip_prefix("R-").and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("R-{:03}", max + 1)
}

/// Canonical relation id from loose input: `r1`, `R-1`, `r-001` → `R-001`.
fn canonical_id(input: &str) -> Option<String> {
    entity::canonical_prefixed_id('r', input)
}

/// Read-only cross-reference audit for `talkdnd check` (contract §3.4): one
/// line per relation endpoint whose id does not exist in the entity ledger.
/// Relations always store canonical ids, so the comparison is exact; the
/// relation id is included so the DM can find the row. Never writes.
pub fn audit_references(relations: &Relations, ledger: &Ledger) -> Vec<String> {
    let mut out = Vec::new();
    for r in &relations.relations {
        for (role, id) in [("from", &r.from), ("to", &r.to)] {
            if !ledger.entities.iter().any(|e| &e.id == id) {
                out.push(format!(
                    "关系 {}（{}）：{role} 指向不存在的实体 {id}",
                    r.id, r.kind
                ));
            }
        }
    }
    out
}

/// In force at `session` iff `since <= session` and (`until` empty or
/// `session < until`) — file-name lexicographic order (contract §4.4).
pub fn active_at(r: &Relation, session: &str) -> bool {
    let s = entity::session_key(session);
    if entity::session_key(&r.since_session) > s {
        return false;
    }
    r.until_session.is_empty() || s < entity::session_key(&r.until_session)
}

// ----------------------------------------------------------------- commands

/// `relation add`: one relation between two existing entities. Re-adding the
/// same (type, from, to) while it has not ended returns the existing id.
#[allow(clippy::too_many_arguments)]
pub fn add(
    save: &Path,
    kind: &str,
    from: &str,
    to: &str,
    since: Option<&str>,
    source: Option<&str>,
    note: Option<&str>,
    dm_only: bool,
) -> Result<String> {
    let kind = kind.trim().to_lowercase();
    if !TYPES.contains(&kind.as_str()) {
        bail!("--type 必须是 {} 之一", TYPES.join(" / "));
    }
    let entities = entity::load(save)?;
    let (fid, fname) = {
        let e = entity::resolve(&entities, from)?;
        (e.id.clone(), e.name.clone())
    };
    let (tid, tname) = {
        let e = entity::resolve(&entities, to)?;
        (e.id.clone(), e.name.clone())
    };
    let since = match since.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => entity::resolve_session(save, s),
        None => entity::current_session(save)?,
    };
    let ledger = load(save)?;
    if let Some(r) = ledger
        .relations
        .iter()
        .find(|r| r.kind == kind && r.from == fid && r.to == tid && r.until_session.is_empty())
    {
        return Ok(format!(
            "已存在：{}（未结束）；未改动，返回既有 id",
            line(r, &entities)
        ));
    }
    let relation = Relation {
        id: next_id(&ledger),
        kind: kind.clone(),
        from: fid.clone(),
        to: tid.clone(),
        since_session: since.clone(),
        until_session: String::new(),
        source: source.map(str::trim).unwrap_or_default().to_string(),
        dm_only,
        note: note.map(str::trim).unwrap_or_default().to_string(),
    };
    let id = relation.id.clone();
    let line = line(&relation, &entities);
    let mut relations = ledger.relations;
    relations.push(relation);
    store(
        save,
        &Relations {
            version: VERSION,
            relations,
        },
    )?;
    entity::log_line(
        save,
        &format!("关系 {id} 登记：{kind}  {fname}（{fid}）→ {tname}（{tid}），自 {since}"),
    )?;
    Ok(line)
}

/// One-line rendering of a relation; `all` adds the DM-only / ended markers.
fn line(r: &Relation, entities: &Ledger) -> String {
    let mut s = format!(
        "{}  {}  {} → {}  自 {}",
        r.id,
        r.kind,
        entity::display(entities, &r.from),
        entity::display(entities, &r.to),
        r.since_session
    );
    if !r.source.is_empty() {
        s += &format!("  来源 {}", r.source);
    }
    if !r.note.is_empty() {
        s += &format!("  备注 {}", r.note);
    }
    if !r.until_session.is_empty() {
        s += &format!("  [已结束 {}]", r.until_session);
    }
    if r.dm_only {
        s += "  [DM-only]";
    }
    s
}

/// `relation list`: default is the player view — relations in force in the
/// current session (or at `--at`), without dm_only rows. `--all` is the DM
/// view including ended and dm_only relations.
pub fn list(
    save: &Path,
    entity_key: Option<&str>,
    at: Option<&str>,
    all: bool,
) -> Result<Vec<String>> {
    let entities = entity::load(save)?;
    let filter = match entity_key.map(str::trim).filter(|s| !s.is_empty()) {
        Some(k) => {
            let e = entity::resolve(&entities, k)?;
            if e.dm_only {
                bail!("{} 是 DM-only 实体；不在玩家视图输出", e.id);
            }
            Some(e.id.clone())
        }
        None => None,
    };
    let session = match at.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(entity::resolve_session(save, s)),
        None if all => None,
        None => Some(entity::current_session(save)?),
    };
    let ledger = load(save)?;
    let mut out = Vec::new();
    for r in &ledger.relations {
        if let Some(f) = &filter {
            if r.from != *f && r.to != *f {
                continue;
            }
        }
        if let Some(s) = &session {
            if !all && !active_at(r, s) {
                continue;
            }
        }
        if !all && r.dm_only {
            continue;
        }
        out.push(line(r, &entities));
    }
    if out.is_empty() {
        out.push("（没有符合条件的关系）".into());
    }
    Ok(out)
}

/// `relation end`: write `until_session` (default: the current session) and
/// keep the row; a note records the reason (corrections stay traceable).
/// Ending an ended relation is a no-op.
pub fn end(save: &Path, id: &str, at: Option<&str>, note: Option<&str>) -> Result<String> {
    let mut ledger = load(save)?;
    let rid =
        canonical_id(id).ok_or_else(|| anyhow::anyhow!("关系 id `{id}` 不合法（形如 R-001）"))?;
    let pos = ledger
        .relations
        .iter()
        .position(|r| r.id == rid)
        .ok_or_else(|| anyhow::anyhow!("没有关系 `{rid}`；`relation list --all` 看全部"))?;
    if !ledger.relations[pos].until_session.is_empty() {
        return Ok(format!(
            "{} 已在 {} 结束；未改动",
            rid, ledger.relations[pos].until_session
        ));
    }
    let since = entity::session_key(&ledger.relations[pos].since_session);
    let until = match at.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => entity::resolve_session(save, s),
        None => entity::current_session(save)?,
    };
    if !since.is_empty() && until < since {
        bail!(
            "{} 的结束会话 {} 早于生效会话 {}；如需更正请用 --note 说明",
            rid,
            until,
            ledger.relations[pos].since_session
        );
    }
    let note = note.map(str::trim).unwrap_or_default();
    let r = &mut ledger.relations[pos];
    r.until_session = until.clone();
    if !note.is_empty() {
        if r.note.is_empty() {
            r.note = note.to_string();
        } else {
            r.note = format!("{}；{}", r.note, note);
        }
    }
    let line = format!("{}  已结束 {}", rid, until);
    store(save, &ledger)?;
    entity::log_line(
        save,
        &format!(
            "关系 {rid} 结束于 {until}（记录保留）{}",
            if note.is_empty() {
                String::new()
            } else {
                format!("；{note}")
            }
        ),
    )?;
    Ok(line)
}

/// `relation show`: every field of one relation; dm_only rows are refused so
/// they cannot reach the player view.
pub fn show(save: &Path, id: &str) -> Result<Vec<String>> {
    let ledger = load(save)?;
    let entities = entity::load(save)?;
    let rid =
        canonical_id(id).ok_or_else(|| anyhow::anyhow!("关系 id `{id}` 不合法（形如 R-001）"))?;
    let r = ledger
        .relations
        .iter()
        .find(|r| r.id == rid)
        .ok_or_else(|| anyhow::anyhow!("没有关系 `{rid}`；`relation list --all` 看全部"))?;
    if r.dm_only {
        bail!(
            "{} 是 DM-only 关系；不在玩家视图输出（`relation list --all` 可见）",
            r.id
        );
    }
    Ok(vec![
        line(r, &entities),
        format!("- 自：{}", r.since_session),
        format!(
            "- 至：{}",
            if r.until_session.is_empty() {
                "（仍生效）".to_string()
            } else {
                r.until_session.clone()
            }
        ),
        format!(
            "- 来源：{}",
            if r.source.is_empty() {
                "（无）"
            } else {
                r.source.as_str()
            }
        ),
        format!(
            "- 备注：{}",
            if r.note.is_empty() {
                "（无）"
            } else {
                r.note.as_str()
            }
        ),
    ])
}

/// `relation context`: entity summary plus the relations in force in the
/// current session (player-filtered). Retired and dm_only entities are
/// refused: they never appear in a context view.
pub fn context(save: &Path, key: &str) -> Result<Vec<String>> {
    let entities = entity::load(save)?;
    let e: &Entity = entity::resolve(&entities, key)?;
    if e.dm_only {
        bail!("{} 是 DM-only 实体；不在玩家视图输出", e.id);
    }
    if e.status == "retired" {
        bail!(
            "{}（{}）已退役；不在 relation context（`entity show` 与 `relation list --entity {} --all` 可查）",
            e.id,
            e.name,
            e.id
        );
    }
    let ledger = load(save)?;
    let session = entity::current_session(save)?;
    let active: Vec<&Relation> = ledger
        .relations
        .iter()
        .filter(|r| !r.dm_only && (r.from == e.id || r.to == e.id))
        .filter(|r| active_at(r, &session))
        .collect();
    let mut out = vec![
        entity::display(&entities, &e.id),
        format!("- 类型：{}；首见：{}；状态：active", e.kind, e.first_seen),
        if !e.aliases.is_empty() {
            format!("- 别名：{}", e.aliases.join("、"))
        } else {
            "- 别名：（无）".to_string()
        },
        format!("当前生效关系（{} 条）：", active.len()),
    ];
    if active.is_empty() {
        out.push("- （无）".into());
    } else {
        for r in active {
            out.push(format!("- {}", line(r, &entities)));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relation(kind: &str, since: &str, until: &str) -> Relation {
        Relation {
            id: "R-001".into(),
            kind: kind.into(),
            from: "E-001".into(),
            to: "E-002".into(),
            since_session: since.into(),
            until_session: until.into(),
            ..Default::default()
        }
    }

    #[test]
    fn active_at_follows_the_contract_window() {
        let open = relation("ally", "2030-01-02-session-01.md", "");
        let ended = relation(
            "ally",
            "2030-01-02-session-01.md",
            "2030-01-02-session-03.md",
        );
        assert!(active_at(&open, "2030-01-02-session-01.md"));
        assert!(active_at(&open, "2030-01-02-session-09.md"));
        assert!(active_at(&ended, "2030-01-02-session-01.md"));
        assert!(active_at(&ended, "2030-01-02-session-02.md"));
        // The ending session itself counts as after the relation.
        assert!(!active_at(&ended, "2030-01-02-session-03.md"));
        assert!(!active_at(&ended, "2030-01-02-session-04.md"));
        // Before the start.
        assert!(!active_at(&open, "2030-01-02-session-00-setup.md"));
        // Anchors do not participate in the comparison.
        assert!(active_at(&ended, "2030-01-02-session-01.md#酒馆"));
        assert!(!active_at(&ended, "2030-01-02-session-03.md"));
    }
}
