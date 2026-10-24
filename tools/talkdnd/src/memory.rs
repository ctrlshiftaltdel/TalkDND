//! Long-term memory and narration style (P03; LetsPlay/docs/campaign-continuity.md §4.2, §5.2).
//!
//! `talkdnd memory ...` keeps long-lived facts and preferences that survive
//! session gaps in `Saves/<slot>/memory.yaml` (version 1). It is an index of
//! fixed facts, not a second ledger: event history stays in the session logs,
//! player-known revelations in `reveals.yaml`, improvised world facts in
//! `canon.md` (contract §4.2, boundary rule).
//!
//! Visibility (contract §6.1): the player view — `memory list` without
//! `--all` and the `start` digest's 关键记忆 section — shows
//! `player-confirmed` entries only; `import` and `manual` entries are
//! DM-only. `talkdnd settings style ...` holds the table's narration
//! preferences in `save-meta.yaml` (`settings.style`, schema 8). Style
//! changes only on explicit player feedback, always with a `--reason` that
//! is written to the session log; nothing infers it automatically.

use crate::migrate;
use crate::save::{self, append};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Record a long-term fact (empty text is rejected)
    Add {
        /// The fact to remember
        text: Vec<String>,
        /// player-confirmed (default) | import | manual
        #[arg(long, default_value = "player-confirmed")]
        source: String,
        /// Tag such as npc:<id> (repeatable)
        #[arg(long)]
        tag: Vec<String>,
    },
    /// List memories (default: player-confirmed only)
    List {
        /// Only entries with this tag
        #[arg(long)]
        tag: Option<String>,
        /// Also list import/manual entries (DM view)
        #[arg(long)]
        all: bool,
    },
    /// Remove one memory by id (unknown id exits 1)
    Remove { id: String },
}

/// `talkdnd settings style show|set` (same task as memory: P03, TASK-0061).
#[derive(Subcommand, Clone)]
pub enum StyleCmd {
    /// Show the current narration/detail/pace/npc_voice/options
    Show,
    /// Set one style field; --reason is required and logged
    Set {
        /// narration | detail | pace | npc_voice | options
        field: String,
        value: String,
        /// Why the style changes (required; written to the session log)
        #[arg(long)]
        reason: String,
    },
}

pub const FILE: &str = "memory.yaml";

/// The sources a memory entry can carry. Only `player-confirmed` entries are
/// player-visible; `import` (brought over from an older campaign) and
/// `manual` (DM notes) stay in the DM view.
pub const SOURCES: &[&str] = &["player-confirmed", "import", "manual"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Sequential, never reused: M-001, M-002, ...
    pub id: String,
    /// The fixed fact or preference; never empty.
    pub text: String,
    /// player-confirmed | import | manual (see [`SOURCES`]).
    pub source: String,
    /// YYYY-MM-DD HH:MM (local time).
    pub added_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    /// On-disk schema; 0 and 1 are compatible, newer versions are refused.
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

fn path(save: &Path) -> PathBuf {
    save.join(FILE)
}

/// Load the memory ledger. A missing file is an empty ledger (feature off);
/// a newer version is refused like every other sidecar.
pub fn load(save: &Path) -> Result<Memory> {
    let p = path(save);
    if !p.is_file() {
        return Ok(Memory::default());
    }
    let t = fs::read_to_string(&p)?;
    let m: Memory =
        serde_norway::from_str(&t).with_context(|| format!("invalid {}", p.display()))?;
    if m.version > 1 {
        bail!(
            "{}: unsupported memory schema version {} (this talkdnd supports 1); update talkdnd",
            p.display(),
            m.version
        );
    }
    Ok(m)
}

fn store(save: &Path, m: &Memory) -> Result<()> {
    let mut current = m.clone();
    current.version = 1;
    let text = format!(
        "# TalkDND long-term memory: fixed facts and preferences that survive session gaps.\n# Edit with `talkdnd memory add|remove`; the player view shows player-confirmed entries only.\n{}",
        serde_norway::to_string(&current)?
    );
    atomic_write(path(save), text.as_bytes())
}

/// The next sequential id. Ids are never reused: removing an entry does not
/// lower the counter.
fn next_id(entries: &[Entry]) -> String {
    let max = entries
        .iter()
        .filter_map(|e| e.id.strip_prefix("M-"))
        .filter_map(|n| n.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("M-{:03}", max + 1)
}

/// Record one memory. `source` must be one of [`SOURCES`]; empty text is
/// rejected. Returns the stored entry.
pub fn add(save: &Path, text: &str, source: &str, tags: &[String]) -> Result<Entry> {
    let text = text.trim();
    if text.is_empty() {
        bail!("记忆文本为空；在 `memory add` 后写要长期记住的事实");
    }
    if !SOURCES.contains(&source) {
        bail!("source 只能是 player-confirmed、import、manual（收到 `{source}`）；存档未改动");
    }
    let mut m = load(save)?;
    let entry = Entry {
        id: next_id(&m.entries),
        text: text.to_string(),
        source: source.to_string(),
        added_at: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        tags: tags
            .iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
    };
    m.entries.push(entry.clone());
    store(save, &m)?;
    Ok(entry)
}

/// Player-visible entries only (contract §6.1): `source == player-confirmed`.
/// Used by the `start` digest's 关键记忆 section; import/manual entries and
/// anything from `dm-secrets.md` never reach the player view through here.
pub fn player_entries(save: &Path) -> Vec<Entry> {
    load(save)
        .map(|m| {
            m.entries
                .into_iter()
                .filter(|e| e.source == "player-confirmed")
                .collect()
        })
        .unwrap_or_default()
}

/// One memory line for lists and the digest: `M-001：text（标签：…）`.
pub fn entry_line(e: &Entry) -> String {
    let mut line = format!("{}：{}", e.id, e.text);
    if !e.tags.is_empty() {
        line += &format!("（标签：{}）", e.tags.join("、"));
    }
    line
}

/// `memory add|list|remove` (P03, TASK-0061).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Add { text, source, tag } => {
            // First use on an old save migrates it first (whole-save backup).
            migrate::ensure(save, root, false)?;
            let body = text.join(" ");
            let e = add(save, &body, &source, &tag)?;
            println!("已记录 {}（{}）", entry_line(&e), e.source);
        }
        Cmd::List { tag, all } => {
            let m = load(save)?;
            let shown: Vec<&Entry> = m
                .entries
                .iter()
                .filter(|e| all || e.source == "player-confirmed")
                .filter(|e| tag.as_deref().is_none_or(|t| e.tags.iter().any(|x| x == t)))
                .collect();
            if all {
                println!("固定记忆（{} 条；DM 视图，含全部来源）：", shown.len());
            } else {
                println!(
                    "固定记忆（{} 条；玩家可见的 player-confirmed 条目，`--all` 看 DM 视图）：",
                    shown.len()
                );
            }
            if shown.is_empty() {
                println!("- （还没有；`talkdnd memory add \"<事实>\"` 记录一条）");
            }
            for e in shown {
                let mut line = format!("- {}", entry_line(e));
                if all {
                    line += &format!("［{}，{} 录入］", e.source, e.added_at);
                }
                println!("{line}");
            }
        }
        Cmd::Remove { id } => {
            let id = id.trim();
            let mut m = load(save)?;
            let before = m.entries.len();
            m.entries.retain(|e| e.id != id);
            if m.entries.len() == before {
                bail!("记忆 {id} 不存在（`talkdnd memory list --all` 看全部 id）；存档未改动");
            }
            store(save, &m)?;
            println!("已删除记忆 {id}");
        }
    }
    Ok(())
}

/// One style field: the YAML key, a Chinese label and its legal values.
pub struct StyleField {
    pub name: &'static str,
    pub zh: &'static str,
    pub values: &'static [&'static str],
}

/// The five style fields and their legal values (contract §5.2). Defaults
/// live in [`migrate::Style`].
pub const STYLE_FIELDS: &[StyleField] = &[
    StyleField {
        name: "narration",
        zh: "叙述长度",
        values: &["short", "standard", "long"],
    },
    StyleField {
        name: "detail",
        zh: "细节密度",
        values: &["low", "standard", "high"],
    },
    StyleField {
        name: "pace",
        zh: "节奏",
        values: &["slow", "standard", "fast"],
    },
    StyleField {
        name: "npc_voice",
        zh: "NPC 声音",
        values: &["distinct", "neutral"],
    },
    StyleField {
        name: "options",
        zh: "结尾选项",
        values: &["2-4", "none"],
    },
];

fn style_value(style: &migrate::Style, name: &str) -> String {
    match name {
        "narration" => style.narration.clone(),
        "detail" => style.detail.clone(),
        "pace" => style.pace.clone(),
        "npc_voice" => style.npc_voice.clone(),
        "options" => style.options.clone(),
        _ => String::new(),
    }
}

fn set_style_value(style: &mut migrate::Style, name: &str, value: &str) {
    match name {
        "narration" => style.narration = value.to_string(),
        "detail" => style.detail = value.to_string(),
        "pace" => style.pace = value.to_string(),
        "npc_voice" => style.npc_voice = value.to_string(),
        "options" => style.options = value.to_string(),
        _ => {}
    }
}

/// `settings style show` lines (also the shape of the per-field output).
pub fn style_lines(save_dir: &Path) -> Vec<String> {
    let s = migrate::load(save_dir)
        .ok()
        .flatten()
        .map(|m| m.settings)
        .unwrap_or_default();
    let mut v = vec![
        "叙事风格（settings.style；只有玩家明确反馈才改，改动需 `--reason` 并记日志）:".to_string(),
    ];
    for f in STYLE_FIELDS {
        v.push(format!(
            "- {} {}：{}（可选：{}）",
            f.zh,
            f.name,
            style_value(&s.style, f.name),
            f.values.join("/")
        ));
    }
    v
}

/// `settings style set <字段> <值> --reason R`: validate first (an illegal
/// field/value never touches the save), then write, then log with the reason.
pub fn set_style(
    root: &Path,
    save_dir: &Path,
    field: &str,
    value: &str,
    reason: &str,
) -> Result<Vec<String>> {
    let Some(f) = STYLE_FIELDS.iter().find(|f| f.name == field.trim()) else {
        bail!(
            "风格字段只能是 {}（收到 `{field}`）；存档未改动",
            STYLE_FIELDS
                .iter()
                .map(|f| f.name)
                .collect::<Vec<_>>()
                .join("、")
        );
    };
    let value = value.trim();
    if !f.values.contains(&value) {
        bail!(
            "{} 的值只能是 {}（收到 `{value}`）；存档未改动",
            f.name,
            f.values.join("/")
        );
    }
    let reason = reason.trim();
    if reason.is_empty() {
        bail!("--reason 不能为空：写一句玩家为什么要求改（会记进会话日志）");
    }
    // Old saves migrate first so the style fields exist (whole-save backup).
    migrate::ensure(save_dir, root, false)?;
    let Some(mut meta) = migrate::load(save_dir)? else {
        bail!("{}: save-meta.yaml is missing", save_dir.display());
    };
    let old = style_value(&meta.settings.style, f.name);
    if old == value {
        return Ok(vec![format!(
            "叙事风格 {} 已是 {value}；未改动、未记日志",
            f.name
        )]);
    }
    set_style_value(&mut meta.settings.style, f.name, value);
    migrate::write_meta(save_dir, &meta)?;
    let line = format!(
        "叙事风格 {}（{}）：{} → {}（{}）",
        f.name, f.zh, old, value, reason
    );
    if let Some(log) = save::latest_log(save_dir) {
        append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(vec![line])
}

/// `settings style show|set` (P03, TASK-0061).
pub fn run_style(root: &Path, save_dir: &Path, cmd: StyleCmd) -> Result<()> {
    match cmd {
        StyleCmd::Show => {
            for l in style_lines(save_dir) {
                println!("{l}");
            }
        }
        StyleCmd::Set {
            field,
            value,
            reason,
        } => {
            for l in set_style(root, save_dir, &field, &value, &reason)? {
                println!("{l}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_save(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("talkdnd-memory-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("Sessions")).unwrap();
        fs::write(p.join("Sessions/s.md"), "# log\n").unwrap();
        p
    }

    #[test]
    fn add_list_remove_reject_empty_and_unknown_ids() {
        let save = temp_save("crud");
        let first = add(
            &save,
            "  the bard owes the guard a favor  ",
            "player-confirmed",
            &["npc:guard".into()],
        )
        .unwrap();
        assert_eq!(first.id, "M-001");
        assert_eq!(first.text, "the bard owes the guard a favor");
        let second = add(&save, "table prefers fast pace", "manual", &[]).unwrap();
        assert_eq!(second.id, "M-002", "ids stay sequential, no duplicates");

        assert!(add(&save, "   ", "player-confirmed", &[]).is_err());
        assert!(add(&save, "x", "diary", &[]).is_err());

        // Player view: only player-confirmed; DM view: everything.
        let player = player_entries(&save);
        assert_eq!(player.len(), 1);
        assert_eq!(player[0].id, "M-001");
        let all = load(&save).unwrap();
        assert_eq!(all.entries.len(), 2);

        // Remove works, and the id is not reused afterwards.
        let mut m = load(&save).unwrap();
        m.entries.retain(|e| e.id != "M-001");
        store(&save, &m).unwrap();
        assert_eq!(next_id(&load(&save).unwrap().entries), "M-003");

        // A newer sidecar version is refused.
        fs::write(path(&save), "version: 2\nentries: []\n").unwrap();
        let err = load(&save).unwrap_err().to_string();
        assert!(err.contains("memory.yaml") && err.contains("2"), "{err}");

        // Missing file = empty ledger.
        let empty = temp_save("empty");
        assert!(load(&empty).unwrap().entries.is_empty());
        assert!(player_entries(&empty).is_empty());
        let _ = fs::remove_dir_all(&save);
        let _ = fs::remove_dir_all(&empty);
    }

    #[test]
    fn style_validation_rejects_illegal_values_without_touching_the_save() {
        let root = std::env::temp_dir().join(format!("talkdnd-style-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let save = root.join("Saves/2030-07-01-01");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::write(save.join("Sessions/2030-07-01-session-00.md"), "# log\n").unwrap();
        crate::migrate::init(&save).unwrap();
        let before = fs::read_to_string(migrate::path(&save)).unwrap();

        assert!(set_style(&root, &save, "tone", "warm", "r").is_err());
        assert!(set_style(&root, &save, "narration", "epic", "r").is_err());
        assert!(set_style(&root, &save, "pace", "fast", "  ").is_err());
        assert_eq!(
            fs::read_to_string(migrate::path(&save)).unwrap(),
            before,
            "illegal values must not change save-meta.yaml"
        );

        let out = set_style(&root, &save, "narration", "short", "table vote").unwrap();
        assert!(
            out[0].contains("standard → short") && out[0].contains("table vote"),
            "{out:?}"
        );
        let meta = migrate::load(&save).unwrap().unwrap();
        assert_eq!(meta.settings.style.narration, "short");
        assert_eq!(meta.settings.style.pace, "standard");
        let log = fs::read_to_string(save.join("Sessions/2030-07-01-session-00.md")).unwrap();
        assert!(log.contains("叙事风格 narration"), "{log}");
        assert!(log.contains("table vote"), "{log}");

        // Setting the same value again changes and logs nothing.
        let log_lines = log.lines().count();
        let again = set_style(&root, &save, "narration", "short", "again").unwrap();
        assert!(again[0].contains("已是 short"), "{again:?}");
        let log = fs::read_to_string(save.join("Sessions/2030-07-01-session-00.md")).unwrap();
        assert_eq!(log.lines().count(), log_lines);

        let lines = style_lines(&save).join("\n");
        assert!(lines.contains("narration：short"), "{lines}");
        assert!(lines.contains("npc_voice：distinct"), "{lines}");
        assert!(lines.contains("options：2-4"), "{lines}");
        let _ = fs::remove_dir_all(&root);
    }
}
