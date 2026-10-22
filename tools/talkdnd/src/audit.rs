//! Save integrity (manifest) and log reconciliation for `talkdnd check`
//! (no arguments), `talkdnd start` and `talkdnd integrity`.
//!
//! The manifest itself lives in `combat_sim::integrity`. This module adds the
//! reconciliation baseline: key values (HP, XP, coins, items) and the session
//! log position at the last clean point. Value changes since then must be
//! explained by new session-log lines or new `xp_log` entries; otherwise they
//! are reported as warnings. Matching is plain keyword search, so results are
//! deterministic.

use crate::check;
use crate::progress::Progress;
use anyhow::Result;
use combat_sim::integrity::{self, Change};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Baseline {
    pub date: String,
    #[serde(default)]
    pub hp: BTreeMap<String, i64>,
    #[serde(default)]
    pub xp: BTreeMap<String, i64>,
    #[serde(default)]
    pub cp: BTreeMap<String, i64>,
    #[serde(default)]
    pub items: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub xp_log_len: usize,
    /// Newest session log file name and its line count.
    #[serde(default)]
    pub log: String,
    #[serde(default)]
    pub log_lines: usize,
}

fn logs(save: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(save.join("Sessions"))
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension().is_some_and(|x| x == "md")
                        && !p
                            .file_name()
                            .is_some_and(|n| n.eq_ignore_ascii_case("README.md"))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn fname(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

pub fn current(save: &Path) -> Baseline {
    let mut b = Baseline {
        date: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        ..Default::default()
    };
    for s in check::sheets(save) {
        if let Some(hp) = s.block.get("hp").and_then(|v| v.as_i64()) {
            if !s.id.is_empty() {
                b.hp.insert(s.id.clone(), hp);
            }
        }
    }
    if let Ok(p) = Progress::load(save) {
        b.xp = p.xp.clone();
        for (k, purse) in &p.purses {
            b.cp.insert(k.clone(), purse.cp);
            let mut items = purse.items.clone();
            items.sort();
            b.items.insert(k.clone(), items);
        }
        b.xp_log_len = p.xp_log.len();
    }
    if let Some(l) = logs(save).pop() {
        b.log = fname(&l);
        b.log_lines = fs::read_to_string(&l)
            .map(|t| t.lines().count())
            .unwrap_or(0);
    }
    b
}

/// Session-log lines written after the baseline position.
fn new_lines(save: &Path, b: &Baseline) -> Vec<String> {
    let mut out = Vec::new();
    for l in logs(save) {
        let n = fname(&l);
        if n.as_str() < b.log.as_str() {
            continue;
        }
        let text = fs::read_to_string(&l).unwrap_or_default();
        let skip = if n == b.log { b.log_lines } else { 0 };
        out.extend(text.lines().skip(skip).map(|s| s.to_lowercase()));
    }
    out
}

const HP_WORDS: [&str; 7] = ["hp", "生命", "伤害", "治疗", "休息", "受伤", "恢复"];
const COIN_WORDS: [&str; 11] = [
    "gp", "sp", "cp", "pp", "ep", "金", "银", "铜", "钱", "花费", "获得",
];
const XP_WORDS: [&str; 2] = ["xp", "经验"];

fn mentions(lines: &[String], names: &[String], words: &[&str]) -> bool {
    lines.iter().any(|l| {
        names
            .iter()
            .any(|n| !n.is_empty() && l.contains(n.as_str()))
            && words.iter().any(|w| l.contains(w))
    })
}

fn names_for(key: &str, sheets: &[check::Sheet]) -> Vec<String> {
    let mut v = vec![key.to_lowercase()];
    if key == crate::progress::PARTY {
        v.extend(["队伍", "公共"].map(String::from));
    }
    if let Some(s) = sheets.iter().find(|s| s.id == key) {
        v.push(s.name.to_lowercase());
    }
    v
}

fn multiset_diff(old: &[String], new: &[String]) -> (Vec<String>, Vec<String>) {
    let mut gone = old.to_vec();
    let mut added = Vec::new();
    for i in new {
        if let Some(pos) = gone.iter().position(|x| x == i) {
            gone.remove(pos);
        } else {
            added.push(i.clone());
        }
    }
    (added, gone)
}

/// Unexplained value changes since the baseline.
pub fn reconcile(save: &Path, b: &Baseline) -> Vec<String> {
    let now = current(save);
    let lines = new_lines(save, b);
    let sheets = check::sheets(save);
    let progress = Progress::load(save).unwrap_or_default();
    let mut w = Vec::new();
    for (id, hp) in &now.hp {
        if let Some(old) = b.hp.get(id) {
            if old != hp && !mentions(&lines, &names_for(id, &sheets), &HP_WORDS) {
                w.push(format!(
                    "HP of `{id}` {old} → {hp}: no session-log line since {} mentions it",
                    b.date
                ));
            }
        }
    }
    for (id, xp) in &now.xp {
        let old = b.xp.get(id).copied().unwrap_or(0);
        if old == *xp {
            continue;
        }
        let logged: i64 = progress
            .xp_log
            .iter()
            .skip(b.xp_log_len)
            .filter(|e| e.to.contains(id))
            .map(|e| e.each)
            .sum();
        if logged != xp - old && !mentions(&lines, &names_for(id, &sheets), &XP_WORDS) {
            w.push(format!(
                "XP of `{id}` {old} → {xp} ({:+}); xp_log since {} explains {logged:+}",
                xp - old,
                b.date
            ));
        }
    }
    let empty = Vec::new();
    let keys: std::collections::BTreeSet<&String> = now.cp.keys().chain(b.cp.keys()).collect();
    for k in keys {
        let old = b.cp.get(k).copied().unwrap_or(0);
        let cur = now.cp.get(k).copied().unwrap_or(0);
        if old != cur && !mentions(&lines, &names_for(k, &sheets), &COIN_WORDS) {
            w.push(format!(
                "coins of `{k}` {} → {}: no session-log line since {} mentions it",
                crate::progress::coins(old),
                crate::progress::coins(cur),
                b.date
            ));
        }
        let (added, gone) = multiset_diff(
            b.items.get(k).unwrap_or(&empty),
            now.items.get(k).unwrap_or(&empty),
        );
        for (verb, list) in [("appeared in", added), ("disappeared from", gone)] {
            for item in list {
                let needle = item.to_lowercase();
                if !lines.iter().any(|l| l.contains(&needle)) {
                    w.push(format!(
                        "item \"{item}\" {verb} `{k}` with no session-log line naming it"
                    ));
                }
            }
        }
    }
    w
}

#[derive(Debug, Default)]
pub struct Report {
    pub created: bool,
    pub changes: Vec<Change>,
    pub unexplained: Vec<String>,
}

impl Report {
    pub fn clean(&self) -> bool {
        self.changes.is_empty() && self.unexplained.is_empty()
    }

    /// Warning lines plus DM guidance (empty when clean).
    pub fn lines(&self) -> Vec<String> {
        let mut v = Vec::new();
        for c in &self.changes {
            v.push(format!("integrity: {}", c.describe()));
        }
        for u in &self.unexplained {
            v.push(format!("reconcile: {u}"));
        }
        if !v.is_empty() {
            v.push("DM：若是你自己按桌规手改并已记日志 → `talkdnd integrity accept --reason \"…\"`；否则跳出角色问玩家，`talkdnd log` 记下处理结果，不合理的改动按日志改回后再 accept（见 docs/table-integrity.md）".into());
        }
        v
    }
}

fn baseline_of(m: &integrity::Manifest) -> Option<Baseline> {
    m.baseline
        .as_ref()
        .and_then(|v| serde_norway::from_value(v.clone()).ok())
}

fn set_baseline(save: &Path, m: &mut integrity::Manifest) -> Result<()> {
    m.baseline = Some(serde_norway::to_value(current(save))?);
    integrity::store(save, m)
}

/// Full audit; creates manifest and baseline for saves that predate them.
pub fn audit(save: &Path) -> Result<Report> {
    let (mut m, created) = integrity::ensure(save)?;
    let mut r = Report {
        created,
        ..Default::default()
    };
    r.changes = integrity::changes_against(&m, &integrity::snapshot(save));
    match baseline_of(&m) {
        Some(b) => r.unexplained = reconcile(save, &b),
        None => set_baseline(save, &mut m)?,
    }
    Ok(r)
}

/// Call after a tool command: record the files it wrote; refresh the
/// baseline when everything is clean. Takes one save snapshot and reuses it
/// for both the manifest update and the reconciliation check.
pub fn after_command(save: &Path, before: &BTreeMap<String, String>) -> Result<()> {
    let now = integrity::snapshot(save);
    integrity::after_write_with(save, before, &now)?;
    let Some(mut m) = integrity::load(save)? else {
        return Ok(());
    };
    let clean_files = integrity::changes_against(&m, &now).is_empty();
    let clean_values = baseline_of(&m).is_none_or(|b| reconcile(save, &b).is_empty());
    if clean_files && clean_values {
        set_baseline(save, &mut m)?;
    }
    Ok(())
}

/// `talkdnd integrity accept`: take the current files and values as the new
/// baseline, and log why.
pub fn accept(save: &Path, reason: &str) -> Result<Vec<String>> {
    let (mut m, _) = integrity::ensure(save)?;
    let changes = integrity::changes_against(&m, &integrity::snapshot(save));
    let unexplained = baseline_of(&m)
        .map(|b| reconcile(save, &b))
        .unwrap_or_default();
    let files: Vec<String> = changes.iter().map(|c| c.path().to_string()).collect();
    let date = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    m.files = integrity::snapshot(save);
    m.accepted.push(integrity::Accepted {
        date,
        reason: reason.trim().to_string(),
        files: files.clone(),
    });
    // Log first so the baseline log position includes this line.
    if let Some(l) = crate::save::latest_log(save) {
        crate::save::append(
            &l,
            &format!(
                "- [{}] 存档校验：认可手改（{}）：{}",
                chrono::Local::now().format("%H:%M"),
                if files.is_empty() {
                    "无文件差异".to_string()
                } else {
                    files.join(", ")
                },
                reason.trim()
            ),
        )?;
    }
    set_baseline(save, &mut m)?;
    let mut out = vec![format!(
        "accepted {} file change(s) and {} unexplained value change(s): {}",
        files.len(),
        unexplained.len(),
        reason.trim()
    )];
    out.extend(files.iter().map(|f| format!("- {f}")));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("talkdnd-audit-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for s in ["Characters", "Sessions", "combat"] {
            fs::create_dir_all(d.join(s)).unwrap();
        }
        fs::write(
            d.join("Characters/Tess.md"),
            "# Tess\n\n```combat yaml\nid: tess\nname: Tess\nhp: 10\n```\n",
        )
        .unwrap();
        fs::write(
            d.join("progress.yaml"),
            "xp: {tess: 0}\npurses:\n  tess: {cp: 500}\n  party: {cp: 0, items: [rope]}\n",
        )
        .unwrap();
        fs::write(d.join("Sessions/2030-01-01-session-01.md"), "# s1\n").unwrap();
        d
    }

    fn set(d: &Path, from: &str, to: &str) {
        for f in ["Characters/Tess.md", "progress.yaml"] {
            let p = d.join(f);
            let t = fs::read_to_string(&p).unwrap();
            if t.contains(from) {
                fs::write(&p, t.replace(from, to)).unwrap();
            }
        }
    }

    fn log(d: &Path, line: &str) {
        crate::save::append(&d.join("Sessions/2030-01-01-session-01.md"), line).unwrap();
    }

    #[test]
    fn old_save_gets_a_clean_baseline() {
        let d = save_dir("old");
        let r = audit(&d).unwrap();
        assert!(r.created && r.clean(), "{:?}", r.lines());
        assert!(audit(&d).unwrap().clean());
        assert!(!audit(&d).unwrap().created);
    }

    #[test]
    fn backup_and_hidden_directories_are_not_audited() {
        let d = save_dir("backups");
        fs::create_dir_all(d.join("_backups/Characters")).unwrap();
        fs::write(d.join("_backups/Characters/evil.md"), "hp: 999\n").unwrap();
        fs::create_dir_all(d.join(".cache")).unwrap();
        fs::write(d.join(".cache/progress.yaml"), "xp: {tess: 999}\n").unwrap();
        let r = audit(&d).unwrap();
        assert!(r.clean(), "{:?}", r.lines());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn unexplained_changes_are_reported_until_accepted() {
        let d = save_dir("hand");
        audit(&d).unwrap();
        set(&d, "hp: 10", "hp: 30");
        set(&d, "cp: 500", "cp: 99500");
        set(&d, "items: [rope]", "items: [rope, vorpal sword]");
        set(&d, "xp: {tess: 0}", "xp: {tess: 900}");
        let r = audit(&d).unwrap();
        assert_eq!(r.changes.len(), 2, "{:?}", r.changes);
        assert_eq!(r.unexplained.len(), 4, "{:?}", r.unexplained);
        assert!(r.unexplained.iter().any(|w| w.contains("vorpal sword")));
        // deterministic
        assert_eq!(audit(&d).unwrap().unexplained, r.unexplained);
        // a tool write elsewhere does not refresh a dirty baseline
        let before = integrity::snapshot(&d);
        fs::write(d.join("reveals.yaml"), "facts: []\n").unwrap();
        after_command(&d, &before).unwrap();
        assert_eq!(audit(&d).unwrap().unexplained.len(), 4);
        accept(&d, "test").unwrap();
        assert!(audit(&d).unwrap().clean());
        let t = fs::read_to_string(d.join("Sessions/2030-01-01-session-01.md")).unwrap();
        assert!(t.contains("存档校验"));
    }

    #[test]
    fn logged_changes_are_explained() {
        let d = save_dir("logged");
        audit(&d).unwrap();
        set(&d, "hp: 10", "hp: 4");
        log(&d, "- [20:00] Tess 受到 6 点伤害，HP 10→4");
        set(&d, "cp: 500", "cp: 300");
        log(&d, "- [20:01] Tess 花费 2 gp 买绳子");
        set(&d, "items: [rope]", "items: []");
        log(&d, "- [20:02] 队伍的 rope 被烧断");
        let r = audit(&d).unwrap();
        assert!(r.unexplained.is_empty(), "{:?}", r.unexplained);
        // the files were still hand-edited
        assert_eq!(r.changes.len(), 2);
    }

    #[test]
    fn xp_log_entries_explain_xp() {
        let d = save_dir("xp");
        audit(&d).unwrap();
        let before = integrity::snapshot(&d);
        fs::write(
            d.join("progress.yaml"),
            "xp: {tess: 50}\npurses:\n  tess: {cp: 500}\n  party: {cp: 0, items: [rope]}\nxp_log:\n- {date: x, category: quest, reason: r, each: 50, to: [tess]}\n",
        )
        .unwrap();
        after_command(&d, &before).unwrap();
        let r = audit(&d).unwrap();
        assert!(r.clean(), "{:?}", r.lines());
    }
}
