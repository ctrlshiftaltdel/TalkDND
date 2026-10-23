//! What the players have identified (`Saves/<slot>/identified.yaml`).
//!
//! A monster or item keeps its appearance description in player-facing output
//! until `talkdnd identify <thing>` records that the party learned what it is.
//! An appearance override can be set before identification:
//! `talkdnd identify --appearance <thing> "<description>"`. DM-facing views
//! always show the real name; this file only feeds the player view.

use anyhow::{bail, Context, Result};
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "identified.yaml";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Creature/item name or id as the DM typed it.
    pub name: String,
    #[serde(default)]
    pub identified: bool,
    /// Player-facing description used while unidentified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub how: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default)]
    pub date: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Ledger {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

fn path(save_dir: &Path) -> PathBuf {
    save_dir.join(FILE)
}

pub fn load(save_dir: &Path) -> Result<Ledger> {
    let p = path(save_dir);
    if !p.is_file() {
        return Ok(Ledger::default());
    }
    let ledger: Ledger = serde_norway::from_str(
        &fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?,
    )
    .with_context(|| format!("invalid {}", p.display()))?;
    if ledger.version > 1 {
        bail!(
            "{}: unsupported identified schema version {} (this talkdnd supports 1)",
            p.display(),
            ledger.version
        );
    }
    Ok(ledger)
}

fn store(save_dir: &Path, ledger: &Ledger) -> Result<()> {
    let mut current = ledger.clone();
    current.version = 1;
    let text = format!(
        "# TalkDND identified monsters/items (player-known names; DM-only appearance overrides).\n# Edit with `talkdnd identify`.\n{}",
        serde_norway::to_string(&current)?
    );
    atomic_write(path(save_dir), text.as_bytes())
}

fn get_or_insert<'a>(ledger: &'a mut Ledger, thing: &str) -> &'a mut Entry {
    let key = thing.trim().to_lowercase();
    if let Some(i) = ledger
        .entries
        .iter()
        .position(|e| e.name.to_lowercase() == key)
    {
        return &mut ledger.entries[i];
    }
    ledger.entries.push(Entry {
        name: thing.trim().to_string(),
        date: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        ..Default::default()
    });
    ledger.entries.last_mut().expect("just pushed")
}

/// `talkdnd identify <thing> [--how check|reveal|lore] [--reason …]`.
pub fn identify(save_dir: &Path, thing: &str, how: &str, reason: Option<&str>) -> Result<String> {
    let thing = thing.trim();
    if thing.is_empty() {
        bail!("identify needs a name");
    }
    let how = how.trim().to_lowercase();
    if !["check", "reveal", "lore"].contains(&how.as_str()) {
        bail!("--how must be check, reveal or lore");
    }
    let mut ledger = load(save_dir)?;
    let (reason, how_echo) = {
        let entry = get_or_insert(&mut ledger, thing);
        entry.identified = true;
        entry.how = how.clone();
        entry.reason = reason.unwrap_or("").trim().to_string();
        entry.date = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        (entry.reason.clone(), entry.how.clone())
    };
    store(save_dir, &ledger)?;
    let line = format!(
        "已识别：{}{}{}（玩家现在可以听到真名）",
        thing,
        if reason.is_empty() {
            String::new()
        } else {
            format!("（{reason}）")
        },
        if how_echo == "reveal" {
            String::new()
        } else {
            format!("，方式：{how_echo}")
        }
    );
    if let Some(log) = crate::save::latest_log(save_dir) {
        crate::save::append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(line)
}

/// `talkdnd identify --appearance <thing> "<description>"`: player-facing
/// description until the thing is identified.
pub fn set_appearance(save_dir: &Path, thing: &str, description: &str) -> Result<String> {
    let thing = thing.trim();
    let description = description.trim();
    if thing.is_empty() || description.is_empty() {
        bail!("--appearance needs <thing> \"<description>\"");
    }
    let mut ledger = load(save_dir)?;
    let entry = get_or_insert(&mut ledger, thing);
    entry.appearance = Some(description.to_string());
    store(save_dir, &ledger)?;
    let line = format!("外貌描述：{thing} → {description}（识别前对玩家使用）");
    if let Some(log) = crate::save::latest_log(save_dir) {
        crate::save::append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(line)
}

/// Match an identified entry by any of `keys` (case-insensitive name).
pub fn lookup<'a>(ledger: &'a Ledger, keys: &[&str]) -> Option<&'a Entry> {
    ledger.entries.iter().find(|e| {
        keys.iter()
            .any(|k| !k.trim().is_empty() && e.name.eq_ignore_ascii_case(k.trim()))
    })
}

/// Player-facing display name: real name only when identified, else the
/// appearance override, else `fallback`.
pub fn display(ledger: &Ledger, keys: &[&str], real: &str, fallback: &str) -> String {
    match lookup(ledger, keys) {
        Some(e) if e.identified => real.to_string(),
        Some(e) => e.appearance.as_deref().unwrap_or(fallback).to_string(),
        None => fallback.to_string(),
    }
}

pub fn list_lines(save_dir: &Path) -> Result<Vec<String>> {
    let ledger = load(save_dir)?;
    if ledger.entries.is_empty() {
        return Ok(vec!["（还没有识别记录）".into()]);
    }
    Ok(ledger
        .entries
        .iter()
        .map(|e| {
            let state = if e.identified {
                "已识别".to_string()
            } else {
                "未识别".to_string()
            };
            let mut s = format!("{}  {}  {}", e.name, state, e.date);
            if let Some(a) = &e.appearance {
                s += &format!("  外貌：{a}");
            }
            if !e.reason.is_empty() {
                s += &format!("  （{}）", e.reason);
            }
            s
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identify_and_appearance_round_trip() {
        let d = std::env::temp_dir().join(format!("talkdnd-identify-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Sessions")).unwrap();
        fs::write(d.join("Sessions/s.md"), "# log\n").unwrap();
        set_appearance(&d, "goblin", "一只瘦小的绿色类人生物").unwrap();
        let ledger = load(&d).unwrap();
        assert_eq!(
            display(&ledger, &["goblin", "g1"], "Goblin", "一只小型类人生物"),
            "一只瘦小的绿色类人生物"
        );
        assert_eq!(
            display(&ledger, &["other"], "Other", "一只中型野兽"),
            "一只中型野兽"
        );
        identify(&d, "Goblin", "check", Some("perception 15")).unwrap();
        let ledger = load(&d).unwrap();
        assert_eq!(
            display(&ledger, &["GOBLIN"], "Goblin", "一只小型类人生物"),
            "Goblin"
        );
        let listed = list_lines(&d).unwrap().join("\n");
        assert!(listed.contains("已识别") && listed.contains("perception 15"));
        assert!(identify(&d, "x", "magic", None).is_err());
        let log = fs::read_to_string(d.join("Sessions/s.md")).unwrap();
        assert!(log.contains("已识别：Goblin"), "{log}");
        let _ = fs::remove_dir_all(&d);
    }
}
