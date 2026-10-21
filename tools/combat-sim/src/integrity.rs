//! Save integrity manifest (`Saves/<slot>/integrity.yaml`), shared by
//! talkdnd and combat-sim.
//!
//! After every tool write the fingerprints of the key save files are
//! recorded. A file whose content no longer matches its fingerprint was
//! changed outside the tools. A file that was already changed by hand stays
//! flagged even if a tool writes it later, until the DM accepts it with
//! `talkdnd integrity accept --reason`.
//!
//! The fingerprint is FNV-1a 64 over the bytes. It catches casual hand edits;
//! it is not a security boundary (anyone with file access can also edit the
//! manifest).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "integrity.yaml";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Accepted {
    pub date: String,
    pub reason: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    #[serde(default)]
    pub version: u32,
    /// Relative path -> fingerprint.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// Reconciliation baseline maintained by talkdnd (values + log position).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<serde_norway::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted: Vec<Accepted>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Modified(String),
    Added(String),
    Removed(String),
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Change::Modified(p) | Change::Added(p) | Change::Removed(p) => p,
        }
    }
    pub fn describe(&self) -> String {
        match self {
            Change::Modified(p) => format!("{p}: changed outside the tools"),
            Change::Added(p) => format!("{p}: new file not written by the tools"),
            Change::Removed(p) => format!("{p}: removed outside the tools"),
        }
    }
}

pub fn fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{h:016x}:{}", bytes.len())
}

fn list(dir: &Path, ext: &str, skip_underscore: bool) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == ext))
        .filter(|p| {
            !skip_underscore
                || !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('_'))
        })
        .collect();
    v.sort();
    v
}

/// Current fingerprints of the tracked files.
pub fn snapshot(save: &Path) -> BTreeMap<String, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    for f in ["progress.yaml", "reveals.yaml", "house-rules.md"] {
        let p = save.join(f);
        if p.is_file() {
            files.push(p);
        }
    }
    files.extend(list(&save.join("Characters"), "md", true));
    files.extend(list(&save.join("NPCs"), "md", true));
    files.extend(list(&save.join("combat"), "yaml", false));
    let mut out = BTreeMap::new();
    for f in files {
        if let Ok(bytes) = fs::read(&f) {
            let rel = f
                .strip_prefix(save)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, fingerprint(&bytes));
        }
    }
    out
}

pub fn path(save: &Path) -> PathBuf {
    save.join(FILE)
}

pub fn load(save: &Path) -> Result<Option<Manifest>> {
    let p = path(save);
    if !p.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    Ok(Some(
        serde_norway::from_str(&text).with_context(|| format!("invalid {}", p.display()))?,
    ))
}

pub fn store(save: &Path, m: &Manifest) -> Result<()> {
    let text = format!(
        "# TalkDND save integrity manifest (written by talkdnd/combat-sim; do not edit).\n# Accept hand edits with `talkdnd integrity accept --reason \"...\"`.\n{}",
        serde_norway::to_string(m)?
    );
    fs::write(path(save), text).with_context(|| format!("write {}", path(save).display()))
}

/// Fresh manifest from the current state.
pub fn baseline(save: &Path) -> Manifest {
    Manifest {
        version: 1,
        files: snapshot(save),
        baseline: None,
        accepted: Vec::new(),
    }
}

/// Load the manifest, creating a baseline for saves that predate it.
/// Returns (manifest, created).
pub fn ensure(save: &Path) -> Result<(Manifest, bool)> {
    if let Some(m) = load(save)? {
        return Ok((m, false));
    }
    let m = baseline(save);
    store(save, &m)?;
    Ok((m, true))
}

pub fn changes_against(m: &Manifest, now: &BTreeMap<String, String>) -> Vec<Change> {
    let mut v = Vec::new();
    for (p, h) in now {
        match m.files.get(p) {
            Some(old) if old == h => {}
            Some(_) => v.push(Change::Modified(p.clone())),
            None => v.push(Change::Added(p.clone())),
        }
    }
    for p in m.files.keys() {
        if !now.contains_key(p) {
            v.push(Change::Removed(p.clone()));
        }
    }
    v
}

pub fn changes(save: &Path) -> Result<Vec<Change>> {
    let (m, _) = ensure(save)?;
    Ok(changes_against(&m, &snapshot(save)))
}

/// Record the files a tool changed between `before` and now. Files that
/// were already out of step with the manifest before the write stay flagged.
pub fn after_write(save: &Path, before: &BTreeMap<String, String>) -> Result<()> {
    let Some(mut m) = load(save)? else {
        // First tool write on an old save: the current state is the baseline.
        store(save, &baseline(save))?;
        return Ok(());
    };
    let now = snapshot(save);
    let clean = |p: &String| before.get(p) == m.files.get(p);
    let mut updates = Vec::new();
    for (p, h) in &now {
        if before.get(p) != Some(h) && clean(p) {
            updates.push((p.clone(), Some(h.clone())));
        }
    }
    for p in before.keys() {
        if !now.contains_key(p) && clean(p) {
            updates.push((p.clone(), None));
        }
    }
    if updates.is_empty() {
        return Ok(());
    }
    for (p, h) in updates {
        match h {
            Some(h) => m.files.insert(p, h),
            None => m.files.remove(&p),
        };
    }
    store(save, &m)
}

/// The save directory for a file inside `Saves/<slot>/combat/`, when that
/// save already has a manifest.
pub fn save_of_combat_file(file: &Path) -> Option<PathBuf> {
    let save = file.parent()?.parent()?;
    path(save).is_file().then(|| save.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("integrity-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Characters")).unwrap();
        fs::create_dir_all(d.join("combat")).unwrap();
        d
    }

    #[test]
    fn tool_writes_are_recorded_hand_edits_stay_flagged() {
        let d = dir("flow");
        fs::write(d.join("progress.yaml"), "xp: {a: 0}\n").unwrap();
        fs::write(d.join("Characters/A.md"), "hp: 10\n").unwrap();
        fs::write(d.join("Characters/_notes.md"), "x").unwrap();
        let (_, created) = ensure(&d).unwrap();
        assert!(created);
        assert!(changes(&d).unwrap().is_empty());
        // tool write
        let before = snapshot(&d);
        fs::write(d.join("progress.yaml"), "xp: {a: 100}\n").unwrap();
        after_write(&d, &before).unwrap();
        assert!(changes(&d).unwrap().is_empty());
        // hand edit
        fs::write(d.join("Characters/A.md"), "hp: 99\n").unwrap();
        fs::write(d.join("Characters/_notes.md"), "y").unwrap();
        assert_eq!(
            changes(&d).unwrap(),
            vec![Change::Modified("Characters/A.md".into())]
        );
        // a later tool write to that file does not launder the edit
        let before = snapshot(&d);
        fs::write(d.join("Characters/A.md"), "hp: 98\n").unwrap();
        fs::write(d.join("combat/party-state.yaml"), "members: {}\n").unwrap();
        after_write(&d, &before).unwrap();
        assert_eq!(
            changes(&d).unwrap(),
            vec![Change::Modified("Characters/A.md".into())]
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn fingerprint_is_stable() {
        assert_eq!(fingerprint(b""), "fnv1a64:cbf29ce484222325:0");
        assert_ne!(fingerprint(b"a"), fingerprint(b"b"));
    }
}
