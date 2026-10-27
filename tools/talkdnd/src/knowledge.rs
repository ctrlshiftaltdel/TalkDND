//! Read-only agent knowledge API (`talkdnd knowledge`).
//!
//! Five commands — `search`, `get`, `cite`, `context`, `related` — expose the
//! DM's local knowledge sources through stable ids and a fixed result shape.
//! The module never writes a save, never builds a persistent index, never
//! touches the network and never reads `dm-secrets.md` into an output entry.
//!
//! Sources and authority labels:
//!
//! | authority | source |
//! |---|---|
//! | `ruling` | the save's `Rules/rulings*.md` (or shipped defaults when absent) |
//! | `save` | `entities.yaml`, `relations.yaml`, `memory.yaml`, `reveals.yaml` |
//! | `book` | converted player books under `Rules/<edition>/<slug>/index/` |
//! | `srd` | tracked SRD content packs under `Rules/combat/<edition>/srd/` |
//! | `adventure` | the save's converted adventure index and player-safe handouts |
//! | `original` | `original.yaml` (hidden faces are DM-only until revealed) |
//!
//! The authority labels are source categories, not a global precedence order.
//! The table authority for adjudication is the save's `Rules/rulings*.md`;
//! rulebooks and the SRD supply rules text; the adventure supplies story facts;
//! original content is the DM's creation.

use crate::{
    adventure, charsheet, check, entity, graph, handout, identify, library, memory, original,
    progress, reveal, save, secret, start,
};
use anyhow::{anyhow, bail, Result};
use clap::{Subcommand, ValueEnum};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const SEARCH_DEFAULT_LIMIT: usize = 20;
const SEARCH_HARD_CAP: usize = 500;
const EXCERPT_CAP: usize = 400;
const GET_CAP: usize = 4000;
const CONTEXT_PARTY_CAP: usize = 20;
const CONTEXT_KNOWN_CAP: usize = 20;
const CONTEXT_RELATION_CAP: usize = 20;
const CONTEXT_RULE_CAP: usize = 5;
const RELATED_HARD_CAP: usize = 100;
const RELATED_FRONTIER_CAP: usize = 50;

/// One knowledge result. `search`, `get` and `related` all use this shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub authority: String,
    pub title: String,
    pub citation: String,
    pub excerpt: String,
    /// Is the entry visible to players? Hidden entries are `false`.
    pub known: bool,
    /// `player` or `dm`.
    pub visibility: String,
    /// Why `related` returned this entry. Empty for `search` and `get`.
    pub via: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scope {
    Srd,
    Book,
    Adventure,
    Save,
    Original,
    All,
}

impl Scope {
    fn authority(self) -> Option<&'static str> {
        match self {
            Scope::Srd => Some("srd"),
            Scope::Book => Some("book"),
            Scope::Adventure => Some("adventure"),
            Scope::Save => Some("save"),
            Scope::Original => Some("original"),
            Scope::All => None,
        }
    }
}

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Search all knowledge sources (default: player-known entries only)
    Search {
        /// Words or phrase to find
        query: Vec<String>,
        /// srd | book | adventure | save | original | all
        #[arg(long, value_enum, default_value = "all")]
        scope: Scope,
        /// Maximum results (default 20; 0 means the hard cap of 500)
        #[arg(long, default_value_t = SEARCH_DEFAULT_LIMIT)]
        limit: usize,
        /// Also return DM-only save/adventure/original entries (never dm-secrets)
        #[arg(long)]
        include_hidden: bool,
    },
    /// Return one entry by id, with its citation
    Get {
        id: String,
        /// Allow a DM-only entry (never a dm-secrets entry)
        #[arg(long)]
        include_hidden: bool,
    },
    /// Print only the citation for one player-visible id
    Cite { id: String },
    /// Current scene, party, known facts, active relations and rules pointers
    Context { key: String },
    /// Graph relations and shared references for one id
    Related {
        id: String,
        /// 1 (default) or 2
        #[arg(long, default_value_t = 1)]
        depth: u8,
    },
}

pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Search {
            query,
            scope,
            limit,
            include_hidden,
        } => {
            let q = query.join(" ");
            let entries = search(root, save, &q, scope, limit, include_hidden)?;
            if entries.is_empty() {
                // Do not echo the query: it may be a term the party has not
                // learned yet, and this line can end up in a session log.
                println!(
                    "没有找到知识条目；试试更短的词、--scope all，或先构建书籍索引（talkdnd adventure index build）"
                );
            } else {
                for e in &entries {
                    print_entry(e);
                    println!();
                }
            }
        }
        Cmd::Get { id, include_hidden } => print_entry(&get(root, save, &id, include_hidden)?),
        Cmd::Cite { id } => println!("{}", cite(root, save, &id)?),
        Cmd::Context { key } => {
            for line in context(root, save, &key)? {
                println!("{line}");
            }
        }
        Cmd::Related { id, depth } => {
            let entries = related(root, save, &id, depth)?;
            if entries.is_empty() {
                println!("无相关条目");
            } else {
                for e in &entries {
                    print_entry(e);
                    println!();
                }
            }
        }
    }
    Ok(())
}

fn print_entry(e: &Entry) {
    println!("id: {}", e.id);
    println!("authority: {}", e.authority);
    println!("title: {}", one_line(&e.title));
    println!("citation: {}", e.citation);
    println!("excerpt: {}", one_line(&e.excerpt));
    println!("known: {}", e.known);
    println!("visibility: {}", e.visibility);
    if !e.via.is_empty() {
        println!("via: {}", e.via);
    }
}

// --------------------------------------------------------------- public API

/// Search the selected sources. The query is matched case-insensitively
/// against the id, title and a bounded excerpt. Results are grouped by fixed
/// authority order (ruling, save, book, srd, adventure, original), then by
/// relevance inside the group, then by id.
pub fn search(
    root: &Path,
    save: &Path,
    query: &str,
    scope: Scope,
    limit: usize,
    include_hidden: bool,
) -> Result<Vec<Entry>> {
    if limit > SEARCH_HARD_CAP {
        bail!("--limit 最大 {SEARCH_HARD_CAP}（收到 {limit}）；0 表示使用这个上限");
    }
    let q = adventure::normalize_for_search(query);
    if q.is_empty() {
        bail!("搜索词去掉标点后为空；写一个词或短语");
    }
    let index = Index::build_scoped(root, save, scope)?;
    let mut hits: Vec<(u8, &RawEntry)> = Vec::new();
    for r in &index.entries {
        if r.blocked {
            continue;
        }
        if !include_hidden && r.entry.visibility != "player" {
            continue;
        }
        if let Some(authority) = scope.authority() {
            if r.entry.authority != authority {
                continue;
            }
        }
        if !r.search_text.contains(&q) {
            continue;
        }
        let title = adventure::normalize_for_search(&r.entry.title);
        let score = if title == q {
            0
        } else if title.contains(&q) {
            1
        } else {
            2
        };
        hits.push((score, r));
    }
    // Group by source type first, then relevance inside the group, then id.
    // The group order is a stable presentation order, not a precedence claim.
    hits.sort_by(|a, b| {
        (
            authority_rank(&a.1.entry.authority),
            a.0,
            a.1.entry.id.as_str(),
        )
            .cmp(&(
                authority_rank(&b.1.entry.authority),
                b.0,
                b.1.entry.id.as_str(),
            ))
    });
    let take = if limit == 0 { SEARCH_HARD_CAP } else { limit };
    Ok(hits
        .into_iter()
        .take(take)
        .map(|(_, r)| public_entry(r, false))
        .collect())
}

/// Return one entry, with a longer excerpt than `search`.
pub fn get(root: &Path, save: &Path, id: &str, include_hidden: bool) -> Result<Entry> {
    let id = id.trim();
    if id.is_empty() {
        bail!("id 不能为空；先用 `talkdnd knowledge search` 找到条目");
    }
    let index = Index::build(root, save)?;
    let r = index
        .by_id(id)
        .ok_or_else(|| anyhow!("未知的知识 id `{id}`；先用 `talkdnd knowledge search` 找到条目"))?;
    if r.blocked {
        bail!(
            "`{id}` 含未揭示的秘密词，不能输出；先用 `talkdnd reveal add` 记录揭示，再检查 `talkdnd secret list`"
        );
    }
    if r.entry.visibility != "player" && !include_hidden {
        bail!("`{id}` 是 DM 内容；加 --include-hidden 查看（dm-secrets 永不输出）");
    }
    Ok(public_entry(r, true))
}

/// Print only the citation for a player-visible id. Never returns a body.
pub fn cite(root: &Path, save: &Path, id: &str) -> Result<String> {
    let id = id.trim();
    if id.is_empty() {
        bail!("id 不能为空；先用 `talkdnd knowledge search` 找到条目");
    }
    let index = Index::build(root, save)?;
    let r = index
        .by_id(id)
        .ok_or_else(|| anyhow!("未知的知识 id `{id}`；先用 `talkdnd knowledge search` 找到条目"))?;
    if r.blocked || r.entry.visibility != "player" {
        bail!("`{id}` 是 DM 内容或含未揭示秘密词；cite 只引用玩家可见条目");
    }
    Ok(r.entry.citation.clone())
}

/// The fixed `context <key>` view: scene → party → known → relations → rules.
/// Missing parts print `无`. Every item names its source.
pub fn context(root: &Path, save: &Path, key: &str) -> Result<Vec<String>> {
    let key = key.trim();
    let index = Index::build(root, save)?;
    let edition = edition(root, save)?;
    let terms = secret_terms(root, save)?;
    if hits_secret(key, &terms) {
        // Do not repeat the key: it may itself be an unrevealed secret term.
        bail!("该 key 含未揭示的秘密词；context 不输出");
    }
    let scene = progress::scene(save);
    let entities = load_entities(root, save)?;
    let focus = match entity::resolve(&entities, key) {
        Ok(e) if e.dm_only => bail!("{} 是 DM-only 实体；不在玩家视图输出", e.id),
        Ok(e) => Some(e),
        Err(_) => None,
    };
    let focus_name = focus
        .map(|e| e.name.clone())
        .unwrap_or_else(|| key.to_string());
    let mut out = Vec::new();

    // 1. Current scene.
    match &scene {
        Some(s) if !hits_secret(s, &terms) => out.push(format!(
            "当前场景：{s}（来源：{}）",
            save_rel(root, save, progress::FILE)
        )),
        Some(_) => out.push("当前场景：无".into()),
        None => out.push("当前场景：无".into()),
    }
    if let Some(e) = focus {
        if !hits_secret(&e.name, &terms) {
            out.push(format!(
                "焦点：{}（{}，来源：{}）",
                e.name,
                e.id,
                save_rel(root, save, entity::FILE)
            ));
        }
    } else if !key.is_empty() && !hits_secret(key, &terms) {
        out.push(format!("焦点：{key}（来源：输入）"));
    }

    // 2. Party.
    let mut party = party_lines(root, save);
    party.retain(|l| !hits_secret(l, &terms));
    if party.is_empty() {
        out.push("队伍：无".into());
    } else {
        out.push("队伍：".into());
        out.extend(party);
    }

    // 3. Known facts, memories, original content and identified names.
    let mut known = known_lines(root, save)?;
    known.retain(|l| !hits_secret(l, &terms));
    if known.is_empty() {
        out.push("已知：无".into());
    } else {
        out.push("已知：".into());
        out.extend(known);
    }

    // 4. Active relations (all for a scene key, the focus entity's for an entity).
    let mut relations = relation_lines(root, save, focus)?;
    relations.retain(|l| !hits_secret(l, &terms));
    if relations.is_empty() {
        out.push("关系：无".into());
    } else {
        out.push("关系：".into());
        out.extend(relations);
    }

    // 5. Rules pointers.
    let mut rules = rule_lines(root, &index, &edition, key, &focus_name);
    rules.retain(|l| !hits_secret(l, &terms));
    if rules.is_empty() {
        out.push("规则：无".into());
    } else {
        out.push("规则：".into());
        out.extend(rules);
    }
    Ok(out)
}

/// Graph relations plus shared references (entity / scene / original /
/// adventure). `depth` is 1 or 2; results are deduplicated and sorted.
pub fn related(root: &Path, save: &Path, id: &str, depth: u8) -> Result<Vec<Entry>> {
    if !(1..=2).contains(&depth) {
        bail!("--depth 只能是 1 或 2（收到 {depth}）");
    }
    let id = id.trim();
    if id.is_empty() {
        bail!("id 不能为空；先用 `talkdnd knowledge search` 找到条目");
    }
    let index = Index::build(root, save)?;
    let seed = index
        .by_id(id)
        .ok_or_else(|| anyhow!("未知的知识 id `{id}`；先用 `talkdnd knowledge search` 找到条目"))?;
    if seed.blocked || seed.entry.visibility != "player" {
        bail!("`{id}` 是 DM 内容或含未揭示秘密词；related 只展开玩家可见条目");
    }
    let session = entity::current_session(save).unwrap_or_default();
    let entities = load_entities(root, save)?;
    let relations = load_relations(root, save)?;

    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    visited.insert(seed.entry.id.clone());
    let mut frontier: Vec<String> = vec![seed.entry.id.clone()];
    for _ in 0..depth {
        let mut next: BTreeSet<String> = BTreeSet::new();
        for node in &frontier {
            for (neighbor, via) in neighbors(&index, &entities, &relations, &session, node) {
                if neighbor == seed.entry.id || visited.contains(&neighbor) {
                    continue;
                }
                let Some(entry) = index.by_id(&neighbor) else {
                    continue;
                };
                if entry.blocked || entry.entry.visibility != "player" {
                    continue;
                }
                seen.entry(neighbor.clone())
                    .and_modify(|old| {
                        if via < *old {
                            *old = via.clone();
                        }
                    })
                    .or_insert(via);
                next.insert(neighbor);
            }
        }
        visited.extend(next.iter().cloned());
        frontier = next.into_iter().take(RELATED_FRONTIER_CAP).collect();
    }

    let mut out: Vec<Entry> = seen
        .into_iter()
        .filter_map(|(id, via)| {
            index.by_id(&id).map(|r| {
                let mut e = public_entry(r, false);
                e.via = via;
                e
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (authority_rank(&a.authority), a.id.as_str())
            .cmp(&(authority_rank(&b.authority), b.id.as_str()))
    });
    out.truncate(RELATED_HARD_CAP);
    Ok(out)
}

// ------------------------------------------------------------------ index

struct Index {
    entries: Vec<RawEntry>,
    by_id: BTreeMap<String, usize>,
}

impl Index {
    fn build(root: &Path, save: &Path) -> Result<Self> {
        Self::build_scoped(root, save, Scope::All)
    }

    fn build_scoped(root: &Path, save: &Path, scope: Scope) -> Result<Self> {
        let all = scope == Scope::All;
        let mut b = Builder::new(root, save);
        if all || scope == Scope::Save {
            b.add_save()?;
        }
        if all || scope == Scope::Original {
            b.add_original()?;
        }
        if all {
            // There is no `--scope ruling`; rulings are part of the whole view.
            b.add_rulings()?;
        }
        if all || scope == Scope::Srd {
            b.add_srd()?;
        }
        if all || scope == Scope::Book {
            b.add_books()?;
        }
        if all || scope == Scope::Adventure {
            b.add_adventure()?;
        }
        b.apply_secret_filter()?;
        let mut entries: Vec<RawEntry> = b.map.into_values().collect();
        entries.sort_by(|a, b| {
            (authority_rank(&a.entry.authority), a.entry.id.as_str())
                .cmp(&(authority_rank(&b.entry.authority), b.entry.id.as_str()))
        });
        let by_id = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.entry.id.clone(), i))
            .collect();
        Ok(Self { entries, by_id })
    }

    fn by_id(&self, id: &str) -> Option<&RawEntry> {
        self.by_id.get(id).map(|i| &self.entries[*i])
    }
}

struct RawEntry {
    entry: Entry,
    /// Full, bounded body used by `get`.
    body: String,
    /// Normalised title + id + body for search.
    search_text: String,
    /// Lower-case title + id + body for the secret-term filter.
    secret_text: String,
    /// Entries on the same adventure page share this key.
    group: String,
    /// True when an unrevealed secret term occurs in the text. Such an entry
    /// is never returned, not even with `--include-hidden`.
    blocked: bool,
}

struct Builder<'a> {
    root: &'a Path,
    save: &'a Path,
    map: BTreeMap<String, RawEntry>,
}

impl<'a> Builder<'a> {
    fn new(root: &'a Path, save: &'a Path) -> Self {
        Self {
            root,
            save,
            map: BTreeMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        id: String,
        authority: &str,
        title: &str,
        citation: String,
        body: String,
        visibility: &str,
        group: String,
    ) {
        if self.map.contains_key(&id) {
            return;
        }
        let title = sanitize_text(self.root, self.save, title);
        let body = clip(&sanitize_text(self.root, self.save, &body), GET_CAP);
        let known = visibility == "player";
        let excerpt = clip(&strip_heading(&body), EXCERPT_CAP);
        let authority_string = authority.to_string();
        let entry = Entry {
            id: id.clone(),
            authority: authority_string,
            title: title.clone(),
            citation,
            excerpt: if excerpt.is_empty() {
                title.clone()
            } else {
                excerpt
            },
            known,
            visibility: visibility.to_string(),
            via: String::new(),
        };
        let search_text = adventure::normalize_for_search(&format!("{} {} {}", title, id, body));
        let secret_text = format!("{} {} {}", title, id, body).to_lowercase();
        self.map.insert(
            id,
            RawEntry {
                entry,
                body,
                search_text,
                secret_text,
                group,
                blocked: false,
            },
        );
    }

    fn add_save(&mut self) -> Result<()> {
        let root = self.root;
        let save = self.save;
        let entities = load_entities(root, save)?;
        for e in &entities.entities {
            let visibility = if e.dm_only { "dm" } else { "player" };
            let aliases = if e.aliases.is_empty() {
                "（无）".to_string()
            } else {
                e.aliases.join("、")
            };
            let body = format!(
                "{}（{}） 类型：{}；别名：{}；首见：{}；状态：{}",
                e.name,
                e.id,
                e.kind,
                aliases,
                if e.first_seen.is_empty() {
                    "（未记录）"
                } else {
                    &e.first_seen
                },
                if e.status.is_empty() {
                    "active"
                } else {
                    &e.status
                }
            );
            self.add(
                format!("save:entity:{}", e.id),
                "save",
                &e.name,
                format!("{} · {}", save_rel(root, save, entity::FILE), e.id),
                body,
                visibility,
                String::new(),
            );
        }

        let memories = load_memory(root, save)?;
        for m in &memories.entries {
            let visibility = if m.source == "player-confirmed" {
                "player"
            } else {
                "dm"
            };
            let tags = if m.tags.is_empty() {
                String::new()
            } else {
                format!("；标签：{}", m.tags.join("、"))
            };
            let body = format!(
                "{}（{}，{}，{}）{}",
                m.text, m.id, m.source, m.added_at, tags
            );
            self.add(
                format!("save:memory:{}", m.id),
                "save",
                &clip(&m.text, 80),
                format!("{} · {}", save_rel(root, save, memory::FILE), m.id),
                body,
                visibility,
                String::new(),
            );
        }

        let reveals = load_reveals(root, save)?;
        for f in &reveals.facts {
            let tags = if f.tags.is_empty() {
                String::new()
            } else {
                format!("；标签：{}", f.tags.join("、"))
            };
            let body = format!(
                "#{} {}（来源：{}，{}）{}",
                f.n,
                f.fact,
                if f.source.is_empty() {
                    "未记录"
                } else {
                    &f.source
                },
                f.date,
                tags
            );
            self.add(
                format!("save:reveal:{}", f.n),
                "save",
                &clip(&f.fact, 80),
                format!("{} · #{}", save_rel(root, save, reveal::FILE), f.n),
                body,
                "player",
                String::new(),
            );
        }

        let relations = load_relations(root, save)?;
        for r in &relations.relations {
            let visibility = if r.dm_only { "dm" } else { "player" };
            let from = entity::display(&entities, &r.from);
            let to = entity::display(&entities, &r.to);
            let mut body = format!("{} {} → {}；自 {}", r.kind, from, to, r.since_session);
            if !r.until_session.is_empty() {
                body += &format!("；至 {}", r.until_session);
            }
            if !r.source.is_empty() {
                body += &format!("；来源 {}", r.source);
            }
            if !r.note.is_empty() {
                body += &format!("；备注 {}", r.note);
            }
            self.add(
                format!("save:relation:{}", r.id),
                "save",
                &format!("{}：{} → {}", r.kind, from, to),
                format!("{} · {}", save_rel(root, save, graph::FILE), r.id),
                body,
                visibility,
                String::new(),
            );
        }
        Ok(())
    }

    fn add_original(&mut self) -> Result<()> {
        let root = self.root;
        let save = self.save;
        let ledger = load_original(root, save)?;
        let reveals = load_reveals(root, save)?;
        let revealed: BTreeSet<String> = reveals
            .facts
            .iter()
            .flat_map(|f| f.tags.iter())
            .filter_map(|t| t.strip_prefix("original:").map(|s| s.to_ascii_lowercase()))
            .collect();
        for e in &ledger.entries {
            let hidden = e.hidden.as_deref().map(str::trim).filter(|h| !h.is_empty());
            let hidden_revealed = revealed.contains(&e.id.to_ascii_lowercase());
            let visibility = if hidden.is_none() || hidden_revealed {
                "player"
            } else {
                "dm"
            };
            let mut body = format!("{}（{}，{}）", e.name, e.id, e.kind);
            if !e.rarity.is_empty() {
                body += &format!("；稀有度 {}", e.rarity);
            }
            if !e.chapter.is_empty() {
                body += &format!("；章节 {}", e.chapter);
            }
            if !e.note.is_empty() {
                body += &format!("；备注 {}", e.note);
            }
            if visibility == "dm" {
                if let Some(h) = hidden {
                    body += &format!("；隐藏面：{h}");
                }
            } else if hidden.is_some() {
                body += "；隐藏面已揭示（见 reveals.yaml）";
            }
            self.add(
                format!("original:{}", e.id),
                "original",
                &e.name,
                format!("{} · {}", save_rel(root, save, original::FILE), e.id),
                body,
                visibility,
                String::new(),
            );
        }
        Ok(())
    }

    fn add_rulings(&mut self) -> Result<()> {
        let root = self.root;
        let save = self.save;
        let edition = edition(root, save)?;
        for (label, local, default) in [
            (
                "common",
                root.join("Rules/rulings.md"),
                root.join("Rules/defaults/rulings.md"),
            ),
            (
                edition.as_str(),
                root.join(format!("Rules/rulings-{edition}.md")),
                root.join(format!("Rules/defaults/rulings-{edition}.md")),
            ),
        ] {
            let path = if local.is_file() {
                local
            } else if default.is_file() {
                default
            } else {
                continue;
            };
            let text = fs::read_to_string(&path)
                .map_err(|e| anyhow!("无法读取 {}：{}", rel(root, &path), e))?;
            for section in adventure::sections(&text) {
                if section.level < 3 {
                    continue;
                }
                let Some(anchor) = section
                    .heading
                    .split_whitespace()
                    .next()
                    .filter(|a| is_ruling_anchor(a))
                else {
                    continue;
                };
                let body = strip_heading(section.text(&text));
                self.add(
                    format!("ruling:{label}:{anchor}"),
                    "ruling",
                    &section.heading,
                    format!("{} · {}", rel(root, &path), anchor),
                    body,
                    "player",
                    String::new(),
                );
            }
        }
        Ok(())
    }

    fn add_srd(&mut self) -> Result<()> {
        let root = self.root;
        let save = self.save;
        let edition = edition(root, save)?;
        let dir = root.join("Rules/combat").join(&edition).join("srd");
        if !dir.is_dir() {
            return Ok(());
        }
        let mut packs: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|e| anyhow!("无法读取 {}：{e}", rel(root, &dir)))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .is_some_and(|n| n.to_string_lossy().ends_with(".pack.json"))
            })
            .collect();
        packs.sort();
        for path in packs {
            let text = fs::read_to_string(&path)
                .map_err(|e| anyhow!("无法读取 {}：{e}", rel(root, &path)))?;
            let value: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| anyhow!("{} 损坏：{e}", rel(root, &path)))?;
            self.add_srd_pack(&edition, &value);
        }
        Ok(())
    }

    fn add_srd_pack(&mut self, edition: &str, value: &serde_json::Value) {
        let label = srd_label(edition);
        let mut added = false;
        if let Some(content) = value.get("content").and_then(|c| c.as_object()) {
            for (key, kind) in SRD_KINDS {
                let Some(items) = content.get(key).and_then(|v| v.as_array()) else {
                    continue;
                };
                for item in items {
                    if self.add_srd_item(label, kind, item) {
                        added = true;
                    }
                }
            }
        }
        if added {
            return;
        }
        // A pack with only an index (no content section) still contributes
        // title-level entries.
        if let Some(entries) = value
            .get("index")
            .and_then(|i| i.get("entries"))
            .and_then(|e| e.as_array())
        {
            for item in entries {
                let kind = item.get("kind").and_then(|k| k.as_str()).unwrap_or("entry");
                self.add_srd_item(label, kind, item);
            }
        }
    }

    fn add_srd_item(&mut self, label: &str, kind: &str, item: &serde_json::Value) -> bool {
        let Some(id) = item
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            return false;
        };
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(id);
        let section = item.get("section").and_then(|v| v.as_str()).unwrap_or(kind);
        let page = item.get("page").and_then(|v| v.as_u64()).unwrap_or(0);
        let aliases = item
            .get("aliases")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            })
            .unwrap_or_default();
        let body = item
            .get("text")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("description").and_then(|v| v.as_str()))
            .or_else(|| item.get("notes").and_then(|v| v.as_str()))
            .unwrap_or("");
        let mut excerpt = body.to_string();
        if !aliases.is_empty() {
            excerpt += &format!("\n别名：{aliases}");
        }
        if excerpt.trim().is_empty() {
            excerpt = format!("{name}（{kind}，{section}）");
        }
        let citation = if page > 0 {
            format!("{label} · {section} p.{page} · {id}")
        } else {
            format!("{label} · {section} · {id}")
        };
        self.add(
            format!("srd:{kind}:{id}"),
            "srd",
            name,
            citation,
            excerpt,
            "player",
            String::new(),
        );
        true
    }

    fn add_books(&mut self) -> Result<()> {
        let root = self.root;
        let save = self.save;
        let edition = edition(root, save)?;
        let category = root.join("Rules").join(&edition);
        for (slug, dir) in library::category_indexes(&category) {
            let Some(idx) = read_structured(root, &dir)? else {
                continue;
            };
            let title = book_title(&idx, &dir, &slug);
            let pages = read_pages(&dir);
            let body_source = pages.as_ref().map(|(_, text, _)| text.as_str());
            let sections = pages.as_ref().map(|(_, _, sections)| sections.as_slice());
            let mut used = BTreeSet::new();
            for (kind, entries) in structured_groups(&idx) {
                for (n, item) in entries.iter().enumerate() {
                    if item.title.trim().is_empty() {
                        continue;
                    }
                    let base = if !item.anchor.is_empty() {
                        item.anchor.clone()
                    } else if item.page > 0 {
                        format!("p{}", item.page)
                    } else {
                        format!("{kind}-{n}")
                    };
                    let anchor = unique_anchor(&base, &mut used);
                    let body = body_for(
                        body_source,
                        sections,
                        &item.title,
                        &format!("{kind}：{}", item.title),
                    );
                    let citation = if item.page > 0 {
                        format!("{} p.{}", title, item.page)
                    } else {
                        format!("{} · {}", title, kind)
                    };
                    self.add(
                        format!("book:{edition}:{slug}:{anchor}"),
                        "book",
                        &item.title,
                        citation,
                        body,
                        "player",
                        String::new(),
                    );
                }
            }
        }
        Ok(())
    }

    fn add_adventure(&mut self) -> Result<()> {
        let root = self.root;
        let save = self.save;
        let Some(slug) = start::adventure_slug(save) else {
            return Ok(());
        };
        let dirs = library::adventure_indexes(root, &slug);
        let multi = dirs.len() > 1;
        for dir in dirs {
            let Some(idx) = read_structured(root, &dir)? else {
                continue;
            };
            let title = book_title(&idx, &dir, &slug);
            let pages = read_pages(&dir);
            let body_source = pages.as_ref().map(|(_, text, _)| text.as_str());
            let sections = pages.as_ref().map(|(_, _, sections)| sections.as_slice());
            let part = if multi {
                format!("{}-", library::slug(&library::index_stem(&dir)))
            } else {
                String::new()
            };
            let mut used = BTreeSet::new();
            for (kind, entries) in structured_groups(&idx) {
                for (n, item) in entries.iter().enumerate() {
                    if item.title.trim().is_empty() {
                        continue;
                    }
                    let base = if !item.anchor.is_empty() {
                        item.anchor.clone()
                    } else if item.page > 0 {
                        format!("p{}", item.page)
                    } else {
                        format!("{kind}-{n}")
                    };
                    let anchor = format!("{part}{}", unique_anchor(&base, &mut used));
                    let section = sections.and_then(|s| find_section(s, &item.title));
                    let hidden = if let Some(s) = section {
                        adventure::dm_only(s) || adventure::ahead_of_progress(save, s).is_some()
                    } else {
                        // A structured index without the matching body still
                        // respects DM-only / chapter markers in the title.
                        let pseudo = adventure::Section {
                            level: 0,
                            heading: item.title.clone(),
                            start: 0,
                            end: 0,
                            start_byte: 0,
                            end_byte: 0,
                            path: vec![item.title.clone()],
                        };
                        adventure::dm_only(&pseudo)
                            || adventure::ahead_of_progress(save, &pseudo).is_some()
                    };
                    let visibility = if hidden { "dm" } else { "player" };
                    let body = body_for(
                        body_source,
                        sections,
                        &item.title,
                        &format!("{kind}：{}", item.title),
                    );
                    let citation = section
                        .and_then(|s| {
                            pages.as_ref().map(|(path, _, _)| {
                                format!(
                                    "{} · {}（{}#p{}）",
                                    title,
                                    s.path.join(" / "),
                                    rel(root, path),
                                    item.page
                                )
                            })
                        })
                        .unwrap_or_else(|| format!("{} · {}（#{}）", title, item.title, anchor));
                    self.add(
                        format!("adventure:{slug}:{anchor}"),
                        "adventure",
                        &item.title,
                        citation,
                        body,
                        visibility,
                        format!("{slug}|{}|{}", library::index_stem(&dir), item.page),
                    );
                }
            }

            let registry = handout::load(&dir).map_err(|e| {
                anyhow!(
                    "无法读取 {}：{}",
                    rel(root, &dir.join(handout::REGISTRY)),
                    sanitize(root, save, &e.to_string())
                )
            })?;
            for h in &registry.entries {
                let visibility = if h.player() { "player" } else { "dm" };
                let mut title = h.caption.clone();
                if title.is_empty() {
                    title = if h.area.is_empty() {
                        format!("p.{} {}", h.page, h.kind)
                    } else {
                        format!("{}（p.{}）", h.area, h.page)
                    };
                }
                let body = format!(
                    "资料图 {}：{}；类型 {}；页码 {}；区域 {}",
                    h.id, title, h.kind, h.page, h.area
                );
                let anchor = format!("{part}handout-{}", h.id);
                self.add(
                    format!("adventure:{slug}:{anchor}"),
                    "adventure",
                    &title,
                    format!(
                        "{} · 资料图 {}（{}，p.{}）",
                        title,
                        h.id,
                        rel(root, &dir),
                        h.page
                    ),
                    body,
                    visibility,
                    format!("{slug}|{}|{}", library::index_stem(&dir), h.page),
                );
            }
        }
        Ok(())
    }

    fn apply_secret_filter(&mut self) -> Result<()> {
        let terms = secret_terms(self.root, self.save)?;
        if terms.is_empty() {
            return Ok(());
        }
        for r in self.map.values_mut() {
            if terms.iter().any(|t| contains_term(&r.secret_text, t)) {
                r.blocked = true;
                r.entry.visibility = "dm".into();
                r.entry.known = false;
            }
        }
        Ok(())
    }
}

// --------------------------------------------------------------- source IO

fn load_entities(root: &Path, save: &Path) -> Result<entity::Ledger> {
    entity::load(save).map_err(|e| {
        anyhow!(
            "无法读取 {}：{}",
            save_rel(root, save, entity::FILE),
            sanitize(root, save, &e.to_string())
        )
    })
}

fn load_relations(root: &Path, save: &Path) -> Result<graph::Relations> {
    graph::load(save).map_err(|e| {
        anyhow!(
            "无法读取 {}：{}",
            save_rel(root, save, graph::FILE),
            sanitize(root, save, &e.to_string())
        )
    })
}

fn load_memory(root: &Path, save: &Path) -> Result<memory::Memory> {
    memory::load(save).map_err(|e| {
        anyhow!(
            "无法读取 {}：{}",
            save_rel(root, save, memory::FILE),
            sanitize(root, save, &e.to_string())
        )
    })
}

fn load_reveals(root: &Path, save: &Path) -> Result<reveal::Ledger> {
    reveal::load(save).map_err(|e| {
        anyhow!(
            "无法读取 {}：{}",
            save_rel(root, save, reveal::FILE),
            sanitize(root, save, &e.to_string())
        )
    })
}

fn load_original(root: &Path, save: &Path) -> Result<original::Ledger> {
    original::load(save).map_err(|e| {
        anyhow!(
            "无法读取 {}：{}",
            save_rel(root, save, original::FILE),
            sanitize(root, save, &e.to_string())
        )
    })
}

fn load_identify(root: &Path, save: &Path) -> Result<identify::Ledger> {
    identify::load(save).map_err(|e| {
        anyhow!(
            "无法读取 {}：{}",
            save_rel(root, save, identify::FILE),
            sanitize(root, save, &e.to_string())
        )
    })
}

fn edition(root: &Path, save: &Path) -> Result<String> {
    save::edition(save).map_err(|e| {
        anyhow!(
            "无法确定规则版本（{}）：{}",
            save_rel(root, save, "combat/campaign.yaml"),
            sanitize(root, save, &e.to_string())
        )
    })
}

#[derive(Debug, Default, Deserialize)]
struct StructuredIndex {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    book: String,
    #[serde(default)]
    source: StructuredSource,
    #[serde(default)]
    headings: Vec<StructuredEntry>,
    #[serde(default)]
    chapters: Vec<StructuredEntry>,
    #[serde(default)]
    maps: Vec<StructuredEntry>,
    #[serde(default)]
    figures: Vec<StructuredEntry>,
    #[serde(default)]
    statblocks: Vec<StructuredEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct StructuredSource {
    #[serde(default)]
    path: String,
}

#[derive(Debug, Default, Deserialize)]
struct StructuredEntry {
    #[serde(default)]
    title: String,
    #[serde(default)]
    page: u32,
    #[serde(default)]
    anchor: String,
}

fn read_structured(root: &Path, dir: &Path) -> Result<Option<StructuredIndex>> {
    let path = dir.join("structured.json");
    if !path.is_file() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).map_err(|e| anyhow!("无法读取 {}：{e}", rel(root, &path)))?;
    let idx: StructuredIndex =
        serde_json::from_str(&text).map_err(|e| anyhow!("{} 损坏：{e}", rel(root, &path)))?;
    if idx.version > 1 {
        bail!(
            "{} 的版本是 {}，本程序只支持 1；请升级 talkdnd",
            rel(root, &path),
            idx.version
        );
    }
    Ok(Some(idx))
}

fn structured_groups(idx: &StructuredIndex) -> [(&'static str, &Vec<StructuredEntry>); 5] {
    [
        ("heading", &idx.headings),
        ("chapter", &idx.chapters),
        ("map", &idx.maps),
        ("figure", &idx.figures),
        ("statblock", &idx.statblocks),
    ]
}

fn read_pages(dir: &Path) -> Option<(PathBuf, String, Vec<adventure::Section>)> {
    let path = library::paged_markdown(dir)?;
    let raw = fs::read_to_string(&path).ok()?;
    let text = library::strip_page_markers(&raw);
    let sections = adventure::sections(&text);
    Some((path, text, sections))
}

fn book_title(idx: &StructuredIndex, dir: &Path, slug: &str) -> String {
    let from_source = Path::new(&idx.source.path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if !from_source.is_empty() {
        return from_source;
    }
    let stem = library::index_stem(dir);
    if stem.is_empty() || stem == library::INDEX_DIR || stem == library::LEGACY_INDEX_DIR {
        if idx.book.is_empty() {
            slug.to_string()
        } else {
            idx.book.clone()
        }
    } else {
        stem
    }
}

fn find_section<'a>(
    sections: &'a [adventure::Section],
    title: &str,
) -> Option<&'a adventure::Section> {
    let q = adventure::normalize_for_search(title);
    sections
        .iter()
        .find(|s| adventure::normalize_for_search(&s.heading) == q)
}

fn body_for(
    text: Option<&str>,
    sections: Option<&[adventure::Section]>,
    title: &str,
    fallback: &str,
) -> String {
    match (text, sections) {
        (Some(text), Some(sections)) => find_section(sections, title)
            .map(|s| strip_heading(s.text(text)))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| fallback.to_string()),
        _ => fallback.to_string(),
    }
}

fn unique_anchor(base: &str, used: &mut BTreeSet<String>) -> String {
    let base = {
        let t = base.trim();
        if t.is_empty() {
            "entry"
        } else {
            t
        }
    };
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    for n in 2usize.. {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("unbounded suffix loop")
}

fn is_ruling_anchor(token: &str) -> bool {
    let Some((prefix, digits)) = token.split_once('-') else {
        return false;
    };
    !prefix.is_empty()
        && prefix.len() <= 3
        && prefix.chars().all(|c| c.is_ascii_uppercase())
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
}

fn srd_label(edition: &str) -> &'static str {
    match edition {
        "2014" => "SRD 5.1",
        "2024" => "SRD 5.2.1",
        _ => "SRD",
    }
}

const SRD_KINDS: [(&str, &str); 12] = [
    ("rule_sections", "rule-section"),
    ("abilities", "ability"),
    ("creatures", "creature"),
    ("items", "item"),
    ("classes", "class"),
    ("subclasses", "subclass"),
    ("species", "species"),
    ("backgrounds", "background"),
    ("feats", "feat"),
    ("tables", "table"),
    ("masteries", "mastery"),
    ("conditions", "condition"),
];

// ---------------------------------------------------------------- context

fn party_lines(root: &Path, save: &Path) -> Vec<String> {
    let sheets = check::sheets(save);
    let party_ids = start::party_ids(save);
    let chars_dir = save.join("Characters");
    let mut members: Vec<&check::Sheet> = sheets
        .iter()
        .filter(|s| s.file.starts_with(&chars_dir) || party_ids.contains(&s.id))
        .collect();
    members.sort_by_key(|s| {
        (
            charsheet::controller_for(save, s) == charsheet::DM,
            s.id.clone(),
        )
    });
    let mut out = Vec::new();
    for s in members.into_iter().take(CONTEXT_PARTY_CAP) {
        let level = s.block.get("level").and_then(|v| v.as_i64()).unwrap_or(1);
        let hp = s
            .block
            .get("hp")
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into());
        let ac = s
            .block
            .get("ac")
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into());
        let class = class_of(save, s);
        let role = if charsheet::controller_for(save, s) == charsheet::DM {
            "同伴"
        } else {
            "玩家角色"
        };
        out.push(format!(
            "- {}（{}，{role}）：{class} {level} 级，HP {hp}，AC {ac}（来源：{}）",
            s.name,
            s.id,
            rel(root, &s.file)
        ));
    }
    out
}

fn class_of(save: &Path, s: &check::Sheet) -> String {
    std::fs::read_to_string(save.join("sheets").join(format!("{}.yaml", s.id)))
        .ok()
        .and_then(|t| serde_norway::from_str::<serde_norway::Value>(&t).ok())
        .and_then(|v| {
            v.get("info")
                .and_then(|i| i.get("class"))
                .and_then(|c| c.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            s.block
                .get("class")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            s.block
                .get("notes")
                .and_then(|v| v.as_str())
                .map(|n| n.split_whitespace().next().unwrap_or("").to_string())
        })
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| "?".into())
}

fn known_lines(root: &Path, save: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let reveals = load_reveals(root, save)?;
    for f in reveals.facts.iter().rev().take(CONTEXT_KNOWN_CAP) {
        out.push(format!(
            "- [reveal #{}] {}（来源：{}）",
            f.n,
            f.fact,
            save_rel(root, save, reveal::FILE)
        ));
    }
    let memories = load_memory(root, save)?;
    for m in memories
        .entries
        .iter()
        .filter(|m| m.source == "player-confirmed")
        .take(CONTEXT_KNOWN_CAP.saturating_sub(out.len()))
    {
        out.push(format!(
            "- [memory {}] {}（来源：{}）",
            m.id,
            m.text,
            save_rel(root, save, memory::FILE)
        ));
    }
    let originals = load_original(root, save)?;
    for e in originals
        .entries
        .iter()
        .take(CONTEXT_KNOWN_CAP.saturating_sub(out.len()))
    {
        let hidden = e.hidden.as_deref().map(str::trim).filter(|h| !h.is_empty());
        let revealed = hidden.is_none() || original_revealed(&reveals, &e.id);
        if !revealed {
            continue;
        }
        out.push(format!(
            "- [original {}] {}（来源：{}）",
            e.id,
            e.name,
            save_rel(root, save, original::FILE)
        ));
    }
    let identified = load_identify(root, save)?;
    for e in identified
        .entries
        .iter()
        .filter(|e| e.identified)
        .take(CONTEXT_KNOWN_CAP.saturating_sub(out.len()))
    {
        out.push(format!(
            "- [identified] {}（来源：{}）",
            e.name,
            save_rel(root, save, identify::FILE)
        ));
    }
    for (id, caption) in shown_handouts(root, save)?
        .into_iter()
        .take(CONTEXT_KNOWN_CAP.saturating_sub(out.len()))
    {
        out.push(format!(
            "- [handout {}] {}（来源：{}）",
            id,
            caption,
            save_rel(root, save, "handouts.yaml")
        ));
    }
    out.truncate(CONTEXT_KNOWN_CAP);
    Ok(out)
}

fn shown_handouts(root: &Path, save: &Path) -> Result<Vec<(String, String)>> {
    let path = save.join("handouts.yaml");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| anyhow!("无法读取 {}：{e}", save_rel(root, save, "handouts.yaml")))?;
    let value: serde_norway::Value = serde_norway::from_str(&text)
        .map_err(|e| anyhow!("{} 损坏：{e}", save_rel(root, save, "handouts.yaml")))?;
    let mut out = Vec::new();
    for item in value
        .get("shown")
        .and_then(|s| s.as_sequence())
        .into_iter()
        .flatten()
    {
        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let caption = item.get("caption").and_then(|v| v.as_str()).unwrap_or("");
        if !id.is_empty() && !caption.is_empty() {
            out.push((id.to_string(), caption.to_string()));
        }
    }
    Ok(out)
}

fn original_revealed(reveals: &reveal::Ledger, id: &str) -> bool {
    let wanted = format!("original:{}", id.to_ascii_lowercase());
    reveals
        .facts
        .iter()
        .any(|f| f.tags.iter().any(|t| t.eq_ignore_ascii_case(&wanted)))
}

fn relation_lines(root: &Path, save: &Path, focus: Option<&entity::Entity>) -> Result<Vec<String>> {
    let entities = load_entities(root, save)?;
    let ledger = load_relations(root, save)?;
    let session = entity::current_session(save).unwrap_or_default();
    let mut out = Vec::new();
    for r in &ledger.relations {
        if r.dm_only || !graph::active_at(r, &session) {
            continue;
        }
        if let Some(f) = focus {
            if r.from != f.id && r.to != f.id {
                continue;
            }
        }
        out.push(format!(
            "- {} {} {} → {}（来源：{}）",
            r.id,
            r.kind,
            entity::display(&entities, &r.from),
            entity::display(&entities, &r.to),
            save_rel(root, save, graph::FILE)
        ));
        if out.len() >= CONTEXT_RELATION_CAP {
            break;
        }
    }
    Ok(out)
}

fn rule_lines(
    root: &Path,
    index: &Index,
    edition: &str,
    key: &str,
    focus_name: &str,
) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let hay = adventure::normalize_for_search(&format!("{key} {focus_name}"));
    if hay.chars().count() >= 3 {
        tokens.push(hay);
    }
    for word in format!("{key} {focus_name}").split(|c: char| !c.is_alphanumeric()) {
        let w = word.to_lowercase();
        if w.chars().count() >= 3 && !tokens.contains(&w) {
            tokens.push(w);
        }
    }
    let mut out = Vec::new();
    for r in index
        .entries
        .iter()
        .filter(|r| r.entry.authority == "ruling" && !r.blocked && r.entry.visibility == "player")
    {
        let title = adventure::normalize_for_search(&r.entry.title);
        if tokens.iter().any(|t| title.contains(t)) {
            out.push(format!(
                "- {} {}（来源：{}）",
                r.entry.id, r.entry.title, r.entry.citation
            ));
            if out.len() >= CONTEXT_RULE_CAP {
                break;
            }
        }
    }
    if out.is_empty() {
        let paths = [
            root.join("Rules/rulings.md"),
            root.join(format!("Rules/rulings-{edition}.md")),
        ];
        for p in paths {
            if p.is_file() {
                out.push(format!("- {}（来源：规则文件）", rel(root, &p)));
            }
        }
    }
    out
}

// ----------------------------------------------------------------- related

fn neighbors(
    index: &Index,
    entities: &entity::Ledger,
    relations: &graph::Relations,
    session: &str,
    node_id: &str,
) -> Vec<(String, String)> {
    let Some(node) = index.by_id(node_id) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut group = String::new();

    if let Some(id) = node_id.strip_prefix("save:entity:") {
        if let Some(e) = entities.entities.iter().find(|e| e.id == id) {
            names.push(e.name.clone());
            names.extend(e.aliases.iter().cloned());
        }
        for r in &relations.relations {
            if r.dm_only || !graph::active_at(r, session) {
                continue;
            }
            if r.from != id && r.to != id {
                continue;
            }
            let via = format!("relation {} ({})", r.id, r.kind);
            out.push((format!("save:relation:{}", r.id), via.clone()));
            let peer = if r.from == id { &r.to } else { &r.from };
            out.push((format!("save:entity:{peer}"), via));
        }
        let tag = format!("npc:{}", id.to_ascii_lowercase());
        let entity_tag = format!("entity:{}", id.to_ascii_lowercase());
        for r in index.entries.iter().filter(|r| {
            r.entry.id.starts_with("save:reveal:") || r.entry.id.starts_with("save:memory:")
        }) {
            let body = r.body.to_lowercase();
            if body.contains(&tag) || body.contains(&entity_tag) {
                out.push((r.entry.id.clone(), "shared reference".to_string()));
            }
        }
    } else if let Some(id) = node_id.strip_prefix("original:") {
        names.push(node.entry.title.clone());
        let tag = format!("original:{}", id.to_ascii_lowercase());
        for r in index
            .entries
            .iter()
            .filter(|r| r.entry.id.starts_with("save:reveal:"))
        {
            if r.body.to_lowercase().contains(&tag) {
                out.push((r.entry.id.clone(), "shared reference".to_string()));
            }
        }
    } else if node.entry.authority == "adventure" {
        group = node.group.clone();
        names.push(node.entry.title.clone());
    } else if let Some(id) = node_id.strip_prefix("save:relation:") {
        if let Some(r) = relations.relations.iter().find(|r| r.id == id) {
            out.push((format!("save:entity:{}", r.from), format!("relation {id}")));
            out.push((format!("save:entity:{}", r.to), format!("relation {id}")));
        }
    } else if node_id.starts_with("save:memory:") || node_id.starts_with("save:reveal:") {
        for (kind, id) in reference_tags(&node.body) {
            let target = match kind {
                "npc" | "entity" => format!("save:entity:{id}"),
                "original" => format!("original:{id}"),
                _ => continue,
            };
            out.push((target, "shared reference".to_string()));
        }
    }

    if names.is_empty() {
        return out;
    }
    let mentioned = |hay: &str| names.iter().any(|n| contains_name(hay, n));
    for r in &index.entries {
        if r.blocked || r.entry.visibility != "player" || r.entry.id == node_id {
            continue;
        }
        if !group.is_empty() && !r.group.is_empty() && r.group == group {
            out.push((
                r.entry.id.clone(),
                format!("same page {}", node.entry.title),
            ));
            continue;
        }
        let authority = r.entry.authority.as_str();
        if !matches!(authority, "save" | "original" | "adventure") {
            continue;
        }
        let hay = format!("{} {}", r.entry.title, r.body);
        if mentioned(&hay) {
            out.push((r.entry.id.clone(), "shared reference".to_string()));
        }
    }
    out
}

// ------------------------------------------------------------------ secret

fn secret_terms(root: &Path, save: &Path) -> Result<Vec<String>> {
    let mut terms: BTreeSet<String> = BTreeSet::new();
    for file in ["dm-secrets.md", original::PLAN_FILE] {
        let path = save.join(file);
        match fs::read_to_string(&path) {
            Ok(text) => {
                for term in secret::parse_marked_terms(&text) {
                    terms.insert(term.trim().to_lowercase());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(anyhow!("无法读取 {}：{e}", save_rel(root, save, file))),
        }
    }
    let path = save.join(secret::FILE);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let manual: secret::Manual = serde_norway::from_str(&text)
                .map_err(|e| anyhow!("{} 损坏：{e}", save_rel(root, save, secret::FILE)))?;
            if manual.version > 1 {
                bail!(
                    "{} 的版本是 {}，本程序只支持 1；请升级 talkdnd",
                    save_rel(root, save, secret::FILE),
                    manual.version
                );
            }
            for t in manual.terms {
                terms.insert(t.term.trim().to_lowercase());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(anyhow!(
                "无法读取 {}：{e}",
                save_rel(root, save, secret::FILE)
            ))
        }
    }
    Ok(terms
        .into_iter()
        .filter(|t| t.chars().count() >= 2 && !reveal::is_revealed(save, t))
        .collect())
}

fn hits_secret(text: &str, terms: &[String]) -> bool {
    let hay = text.to_lowercase();
    terms.iter().any(|t| contains_term(&hay, t))
}

/// The same matching rule as `talkdnd guard`: English terms need word
/// boundaries, Chinese terms match as substrings.
fn contains_term(hay: &str, term: &str) -> bool {
    if term.is_empty() {
        return false;
    }
    let word = |c: char| c.is_ascii_alphanumeric();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(term) {
        let start = from + pos;
        let end = start + term.len();
        let left_ok =
            !term.starts_with(word) || !hay[..start].chars().next_back().is_some_and(word);
        let right_ok = !term.ends_with(word) || !hay[end..].chars().next().is_some_and(word);
        if left_ok && right_ok {
            return true;
        }
        from = start + term.chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// `npc:E-001` / `entity:E-001` / `original:O-001` tags, as written by
/// `reveal add --tag` and `reveal add --original`.
fn reference_tags(text: &str) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for token in text.split(|c: char| {
        c.is_whitespace() || matches!(c, '、' | ',' | '，' | ';' | '；' | '(' | ')' | '（' | '）')
    }) {
        for (prefix, kind) in [
            ("npc:", "npc"),
            ("entity:", "entity"),
            ("original:", "original"),
        ] {
            if let Some(pos) = token.to_lowercase().find(prefix) {
                let rest = &token[pos + prefix.len()..];
                let id: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                if !id.is_empty() {
                    out.push((kind, id));
                }
            }
        }
    }
    out
}

fn contains_name(hay: &str, name: &str) -> bool {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return false;
    }
    if name.is_ascii() {
        // Keep spaces so English names still sit on word boundaries.
        contains_term(&hay.to_lowercase(), &name)
    } else {
        adventure::normalize_for_search(hay).contains(&adventure::normalize_for_search(&name))
    }
}

// ---------------------------------------------------------------- helpers

fn public_entry(r: &RawEntry, full: bool) -> Entry {
    let mut e = r.entry.clone();
    let source = if full { &r.body } else { &r.entry.excerpt };
    e.excerpt = clip(
        &strip_heading(source),
        if full { GET_CAP } else { EXCERPT_CAP },
    );
    if e.excerpt.is_empty() {
        e.excerpt = e.title.clone();
    }
    e
}

fn authority_rank(authority: &str) -> u8 {
    match authority {
        "ruling" => 0,
        "save" => 1,
        "book" => 2,
        "srd" => 3,
        "adventure" => 4,
        "original" => 5,
        _ => 9,
    }
}

fn strip_heading(text: &str) -> String {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    if first.trim_start().starts_with('#') {
        lines.collect::<Vec<_>>().join("\n")
    } else {
        text.to_string()
    }
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn rel(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rest) => rest.to_string_lossy().replace('\\', "/"),
        Err(_) => path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string()),
    }
}

fn save_rel(root: &Path, save: &Path, file: &str) -> String {
    format!("{}/{}", rel(root, save), file)
}

fn sanitize(root: &Path, save: &Path, text: &str) -> String {
    let save_abs = save.display().to_string();
    let root_abs = root.display().to_string();
    let mut out = text.replace(&save_abs, &rel(root, save));
    if !root_abs.is_empty() {
        out = out.replace(&root_abs, ".");
    }
    out
}

fn sanitize_text(root: &Path, save: &Path, text: &str) -> String {
    sanitize(root, save, text)
}
