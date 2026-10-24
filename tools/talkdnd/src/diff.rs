//! Checkpoint snapshots (P01; LetsPlay/docs/campaign-continuity.md §5.1).
//!
//! `talkdnd snapshot diff` is the structured difference between two
//! checkpoints (or a checkpoint and `current`): HP, temporary HP, level,
//! resources, conditions, coins, items, XP and party members. Missing or
//! renamed fields are reported as 未知, never guessed; the same input always
//! produces the same output (sorted keys, no timestamps, no randomness).

use crate::checkpoint;
use anyhow::{bail, Result};
use clap::Subcommand;
use combat_sim::party::{self, PartyState};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Structured diff of two checkpoints (or `current`); same input, same output
    Diff {
        /// First checkpoint id
        #[arg(value_name = "ID_A")]
        a: String,
        /// Second checkpoint id, or `current` for the live save
        #[arg(value_name = "ID_B|CURRENT")]
        b: String,
    },
}

/// `snapshot diff <idA> <idB|current>` (P01, TASK-0059).
pub fn run(_root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let Cmd::Diff { a, b } = cmd;
    println!("snapshot diff {a} → {b}");
    let lines = diff_lines(save, &a, &b)?;
    if lines.is_empty() {
        println!("（无变化）");
    } else {
        for line in &lines {
            println!("{line}");
        }
    }
    Ok(())
}

/// The diff lines for two checkpoint ids (or `current`). Deterministic.
pub fn diff_lines(save: &Path, a: &str, b: &str) -> Result<Vec<String>> {
    let side_a = Side::load(save, a)?;
    let side_b = Side::load(save, b)?;
    Ok(side_a.diff(&side_b))
}

/// One side of the diff: a checkpoint's data copies or the live save.
struct Side {
    hp: BTreeMap<String, i64>,
    temp_hp: BTreeMap<String, i64>,
    level: BTreeMap<String, i64>,
    resources: BTreeMap<String, BTreeMap<String, (i32, i32)>>,
    conditions: BTreeMap<String, Vec<String>>,
    cp: BTreeMap<String, i64>,
    items: BTreeMap<String, Vec<String>>,
    xp: BTreeMap<String, i64>,
    party: Option<Vec<String>>,
}

impl Side {
    fn load(save: &Path, id: &str) -> Result<Side> {
        let dir = if id == "current" {
            save.to_path_buf()
        } else {
            let dir = checkpoint::resolve(save, id)?;
            if let Some(problem) = &checkpoint::entry(save, id).problem {
                bail!("检查点 {id} 不可用（{problem}）");
            }
            dir
        };
        Ok(Side::read(&dir))
    }

    fn read(dir: &Path) -> Side {
        let mut side = Side {
            hp: BTreeMap::new(),
            temp_hp: BTreeMap::new(),
            level: BTreeMap::new(),
            resources: BTreeMap::new(),
            conditions: BTreeMap::new(),
            cp: BTreeMap::new(),
            items: BTreeMap::new(),
            xp: BTreeMap::new(),
            party: party_list(dir),
        };
        if let Ok(ps) = PartyState::load(&dir.join("combat").join(party::FILE_NAME)) {
            for (id, m) in &ps.members {
                side.hp.insert(id.clone(), i64::from(m.hp));
                side.temp_hp.insert(id.clone(), i64::from(m.temp_hp));
                let mut res: BTreeMap<String, (i32, i32)> = BTreeMap::new();
                for (name, r) in &m.resources {
                    res.insert(name.clone(), (r.current, r.max));
                }
                if !res.is_empty() {
                    side.resources.insert(id.clone(), res);
                }
                let mut conds: Vec<String> =
                    m.conditions.iter().map(|c| c.condition.clone()).collect();
                conds.sort();
                side.conditions.insert(id.clone(), conds);
            }
        }
        for sheet in crate::check::sheets(dir) {
            if sheet.id.is_empty() {
                continue;
            }
            if let Some(level) = sheet.block.get("level").and_then(|v| v.as_i64()) {
                side.level.insert(sheet.id.clone(), level);
            }
        }
        if let Ok(p) = crate::progress::Progress::load(dir) {
            side.xp = p.xp.clone();
            for (key, purse) in &p.purses {
                side.cp.insert(key.clone(), purse.cp);
                let mut items = purse.items.clone();
                items.sort();
                side.items.insert(key.clone(), items);
            }
        }
        side
    }

    fn diff(&self, other: &Side) -> Vec<String> {
        let mut out = Vec::new();
        for id in union_keys(&self.hp, &other.hp) {
            cmp(
                &mut out,
                "HP",
                &id,
                self.hp.get(&id).copied(),
                other.hp.get(&id).copied(),
            );
        }
        for id in union_keys(&self.temp_hp, &other.temp_hp) {
            cmp(
                &mut out,
                "临时HP",
                &id,
                self.temp_hp.get(&id).copied(),
                other.temp_hp.get(&id).copied(),
            );
        }
        for id in union_keys(&self.level, &other.level) {
            cmp(
                &mut out,
                "等级",
                &id,
                self.level.get(&id).copied(),
                other.level.get(&id).copied(),
            );
        }
        let res_ids: BTreeSet<String> = self
            .resources
            .keys()
            .chain(other.resources.keys())
            .cloned()
            .collect();
        for id in res_ids {
            let a = self.resources.get(&id);
            let b = other.resources.get(&id);
            let names: BTreeSet<String> = a
                .map(|m| m.keys())
                .into_iter()
                .flatten()
                .chain(b.map(|m| m.keys()).into_iter().flatten())
                .cloned()
                .collect();
            for name in names {
                let av = a.and_then(|m| m.get(&name)).copied();
                let bv = b.and_then(|m| m.get(&name)).copied();
                if av == bv {
                    continue;
                }
                out.push(format!(
                    "资源: {id} {name} {} → {}",
                    show_pair(av),
                    show_pair(bv)
                ));
            }
        }
        let cond_ids: BTreeSet<String> = self
            .conditions
            .keys()
            .chain(other.conditions.keys())
            .cloned()
            .collect();
        for id in cond_ids {
            match (self.conditions.get(&id), other.conditions.get(&id)) {
                (Some(a), Some(b)) if a != b => {
                    let mut parts = Vec::new();
                    for c in b.iter().filter(|c| !a.contains(c)) {
                        parts.push(format!("+{c}"));
                    }
                    for c in a.iter().filter(|c| !b.contains(c)) {
                        parts.push(format!("-{c}"));
                    }
                    if !parts.is_empty() {
                        out.push(format!("状态: {id} {}", parts.join(", ")));
                    }
                }
                (Some(a), None) => out.push(format!("状态: {id} {} → 未知", conds(a))),
                (None, Some(b)) => out.push(format!("状态: {id} 未知 → {}", conds(b))),
                _ => {}
            }
        }
        for key in union_keys(&self.cp, &other.cp) {
            cmp(
                &mut out,
                "金币",
                &key,
                self.cp.get(&key).copied(),
                other.cp.get(&key).copied(),
            );
        }
        for key in union_keys(&self.items, &other.items) {
            match (self.items.get(&key), other.items.get(&key)) {
                (Some(a), Some(b)) if a != b => {
                    let (added, gone) = multiset(a, b);
                    let mut parts = Vec::new();
                    for i in &added {
                        parts.push(format!("+{i}"));
                    }
                    for i in &gone {
                        parts.push(format!("-{i}"));
                    }
                    if !parts.is_empty() {
                        out.push(format!("物品: {key} {}", parts.join(", ")));
                    }
                }
                (Some(a), None) => out.push(format!("物品: {key} {} → 未知", a.join(", "))),
                (None, Some(b)) => out.push(format!("物品: {key} 未知 → {}", b.join(", "))),
                _ => {}
            }
        }
        for id in union_keys(&self.xp, &other.xp) {
            cmp(
                &mut out,
                "XP",
                &id,
                self.xp.get(&id).copied(),
                other.xp.get(&id).copied(),
            );
        }
        if self.party != other.party {
            match (&self.party, &other.party) {
                (Some(a), Some(b)) => {
                    let mut parts = Vec::new();
                    for p in b.iter().filter(|p| !a.contains(p)) {
                        parts.push(format!("+{p}"));
                    }
                    for p in a.iter().filter(|p| !b.contains(p)) {
                        parts.push(format!("-{p}"));
                    }
                    if !parts.is_empty() {
                        out.push(format!("队伍成员: {}", parts.join(", ")));
                    }
                }
                (Some(a), None) => out.push(format!("队伍成员: {} → 未知", a.join(", "))),
                (None, Some(b)) => out.push(format!("队伍成员: 未知 → {}", b.join(", "))),
                (None, None) => {}
            }
        }
        out
    }
}

fn cmp(out: &mut Vec<String>, label: &str, id: &str, a: Option<i64>, b: Option<i64>) {
    if a == b {
        return;
    }
    out.push(format!("{label}: {id} {} → {}", show(a), show(b)));
}

fn show(v: Option<i64>) -> String {
    match v {
        Some(x) => x.to_string(),
        None => "未知".to_string(),
    }
}

/// Condition list as text; an empty list is a known "none", not unknown.
fn conds(v: &[String]) -> String {
    if v.is_empty() {
        "（无）".to_string()
    } else {
        v.join(", ")
    }
}

fn show_pair(v: Option<(i32, i32)>) -> String {
    match v {
        Some((c, m)) => format!("{c}/{m}"),
        None => "未知".to_string(),
    }
}

fn union_keys<V>(a: &BTreeMap<String, V>, b: &BTreeMap<String, V>) -> Vec<String> {
    a.keys()
        .chain(b.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Multiset difference: (added, gone).
fn multiset(old: &[String], new: &[String]) -> (Vec<String>, Vec<String>) {
    let mut gone = old.to_vec();
    let mut added = Vec::new();
    for item in new {
        if let Some(pos) = gone.iter().position(|x| x == item) {
            gone.remove(pos);
        } else {
            added.push(item.clone());
        }
    }
    (added, gone)
}

/// The campaign's `party:` list; `None` when there is no campaign manifest.
fn party_list(dir: &Path) -> Option<Vec<String>> {
    let text = fs::read_to_string(dir.join("combat/campaign.yaml")).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("party:") {
            let rest = rest.split('#').next().unwrap_or("");
            return Some(
                rest.trim()
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            );
        }
    }
    Some(Vec::new())
}
