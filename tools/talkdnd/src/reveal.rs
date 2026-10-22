//! Revealed-knowledge ledger (`Saves/<slot>/reveals.yaml`): what the party
//! has actually learned. The DM checks it before naming a fact or a proper
//! noun in narration (see LetsPlay/docs/anti-spoiler.md). Only player-known
//! facts go here; secrets stay in dm-secrets.md.

use crate::save;
use anyhow::{bail, Context, Result};
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "reveals.yaml";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Fact {
    pub n: usize,
    pub date: String,
    pub fact: String,
    /// How the party learned it (check, NPC, document, rumour...).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Ledger {
    #[serde(default)]
    pub facts: Vec<Fact>,
}

fn path(save: &Path) -> PathBuf {
    save.join(FILE)
}

pub fn load(save: &Path) -> Result<Ledger> {
    let p = path(save);
    if !p.is_file() {
        return Ok(Ledger::default());
    }
    let t = fs::read_to_string(&p)?;
    serde_norway::from_str(&t).with_context(|| format!("invalid {}", p.display()))
}

fn store(save: &Path, l: &Ledger) -> Result<()> {
    let text = format!(
        "# TalkDND revealed knowledge: facts the party has learned (player-safe; secrets stay in dm-secrets.md).\n# Edit with `talkdnd reveal add`.\n{}",
        serde_norway::to_string(l)?
    );
    atomic_write(path(save), text.as_bytes())?;
    Ok(())
}

pub fn add(save: &Path, fact: &str, source: &str, tags: &[String]) -> Result<String> {
    let fact = fact.trim();
    if fact.is_empty() {
        bail!("fact is empty");
    }
    let mut l = load(save)?;
    if let Some(f) = l
        .facts
        .iter()
        .find(|f| f.fact.to_lowercase() == fact.to_lowercase())
    {
        bail!("already revealed as #{}: {}", f.n, f.fact);
    }
    let n = l.facts.iter().map(|f| f.n).max().unwrap_or(0) + 1;
    l.facts.push(Fact {
        n,
        date: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        fact: fact.to_string(),
        source: source.trim().to_string(),
        tags: tags
            .iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
    });
    store(save, &l)?;
    let line = format!(
        "揭示 #{n}：{fact}{}",
        if source.trim().is_empty() {
            String::new()
        } else {
            format!("（来源：{}）", source.trim())
        }
    );
    if let Some(log) = save::latest_log(save) {
        save::append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(line)
}

fn fmt(f: &Fact) -> String {
    let mut s = format!("#{} {}", f.n, f.fact);
    if !f.source.is_empty() {
        s += &format!("（来源：{}）", f.source);
    }
    if !f.tags.is_empty() {
        s += &format!(" [{}]", f.tags.join(", "));
    }
    s
}

pub fn list(save: &Path, tag: Option<&str>) -> Result<Vec<String>> {
    let l = load(save)?;
    let v: Vec<String> = l
        .facts
        .iter()
        .filter(|f| tag.is_none_or(|t| f.tags.iter().any(|x| x.eq_ignore_ascii_case(t))))
        .map(fmt)
        .collect();
    Ok(if v.is_empty() {
        vec!["(nothing revealed yet)".into()]
    } else {
        v
    })
}

/// Facts, sources or tags containing `term` (case-insensitive).
pub fn check(save: &Path, term: &str) -> Result<Vec<String>> {
    let t = term.trim().to_lowercase();
    if t.is_empty() {
        bail!("term is empty");
    }
    let l = load(save)?;
    let hits: Vec<String> = l
        .facts
        .iter()
        .filter(|f| {
            f.fact.to_lowercase().contains(&t)
                || f.tags.iter().any(|x| x.to_lowercase().contains(&t))
        })
        .map(fmt)
        .collect();
    Ok(if hits.is_empty() {
        vec![format!("NOT REVEALED: no fact mentions \"{}\" — the characters do not know it yet; describe only what they perceive", term.trim())]
    } else {
        hits
    })
}

/// Last `n` facts for the resume digest.
pub fn recent(save: &Path, n: usize) -> Vec<String> {
    let l = load(save).unwrap_or_default();
    let k = l.facts.len().saturating_sub(n);
    l.facts[k..].iter().map(fmt).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_list_check() {
        let d = std::env::temp_dir().join(format!("talkdnd-reveal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Sessions")).unwrap();
        fs::write(d.join("Sessions/s1.md"), "# s\n").unwrap();
        add(
            &d,
            "The curator keeps a spare key",
            "Investigation 17",
            &["npc:curator".into()],
        )
        .unwrap();
        assert!(add(&d, "the curator keeps a spare key", "", &[]).is_err());
        add(&d, "Vault on level 3", "", &[]).unwrap();
        assert_eq!(list(&d, None).unwrap().len(), 2);
        assert_eq!(list(&d, Some("NPC:curator")).unwrap().len(), 1);
        assert!(check(&d, "CURATOR").unwrap()[0].starts_with("#1"));
        assert!(check(&d, "dragon").unwrap()[0].starts_with("NOT REVEALED"));
        assert_eq!(recent(&d, 1), vec!["#2 Vault on level 3".to_string()]);
        assert!(fs::read_to_string(d.join("Sessions/s1.md"))
            .unwrap()
            .contains("揭示 #1"));
        let _ = fs::remove_dir_all(&d);
    }
}
