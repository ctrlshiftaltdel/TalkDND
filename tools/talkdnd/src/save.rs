//! Locating the play folder, saves and session logs.

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The LetsPlay folder: `--root`, else the nearest ancestor of the current
/// directory holding `AGENTS.md` and `Saves/`, else the parent of the
/// executable's `bin/` directory.
pub fn find_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let is_root = |p: &Path| p.join("AGENTS.md").is_file() && p.join("Saves").is_dir();
    let cwd = std::env::current_dir()?;
    for dir in cwd.ancestors() {
        if is_root(dir) {
            return Ok(dir.to_path_buf());
        }
        if is_root(&dir.join("LetsPlay")) {
            return Ok(dir.join("LetsPlay"));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe.parent().and_then(|b| b.parent()) {
            if is_root(root) {
                return Ok(root.to_path_buf());
            }
        }
    }
    bail!("LetsPlay folder not found (run inside LetsPlay/ or pass --root)")
}

pub fn is_slot_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 13
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 | 10 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
}

pub fn slots(root: &Path) -> Result<Vec<String>> {
    let mut v = Vec::new();
    let dir = root.join("Saves");
    if dir.is_dir() {
        for e in fs::read_dir(&dir)? {
            let e = e?;
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with('_') || n.starts_with('.') {
                continue;
            }
            if e.path().is_dir() && is_slot_name(&n) {
                v.push(n);
            }
        }
    }
    v.sort();
    Ok(v)
}

/// Explicit slot, or the newest one.
pub fn resolve_slot(root: &Path, slot: Option<&str>) -> Result<PathBuf> {
    match slot {
        Some(s) => {
            if s.starts_with('_') || s.starts_with('.') {
                bail!("`{s}` is not a save slot (names starting with `_` or `.` are internal)");
            }
            let p = root.join("Saves").join(s);
            if !p.is_dir() {
                bail!("save {s} not found; saves: {}", slots(root)?.join(", "));
            }
            Ok(p)
        }
        None => match slots(root)?.pop() {
            Some(s) => Ok(root.join("Saves").join(s)),
            None => bail!("no saves yet; start one with `talkdnd new`"),
        },
    }
}

pub fn next_slot_name(root: &Path, date: &str) -> Result<String> {
    let n = slots(root)?
        .iter()
        .filter(|s| s.starts_with(date))
        .filter_map(|s| s[11..].parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    Ok(format!("{date}-{:02}", n + 1))
}

/// Ruleset from combat/campaign.yaml; default 2024.
pub fn ruleset(save: &Path) -> String {
    let text = fs::read_to_string(save.join("combat/campaign.yaml")).unwrap_or_default();
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("ruleset:") {
            let v = rest
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['"', '\'']);
            if v == "2014" || v == "2024" {
                return v.to_string();
            }
        }
    }
    "2024".to_string()
}

/// Newest session log (by file name) in Sessions/.
pub fn latest_log(save: &Path) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(save.join("Sessions"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "md")
                && !p
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("README.md"))
        })
        .collect();
    v.sort();
    v.pop()
}

/// Append `text` without reading the whole file: only the last byte is read to
/// decide whether a separating newline is needed. Long campaign logs therefore
/// cost O(1), not O(file size).
pub fn append(path: &Path, text: &str) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let existing = f
        .metadata()
        .map(|m| m.len())
        .with_context(|| format!("stat {}", path.display()))?;
    if existing > 0 {
        f.seek(SeekFrom::End(-1))
            .with_context(|| format!("seek {}", path.display()))?;
        let mut last = [0u8; 1];
        f.read_exact(&mut last)
            .with_context(|| format!("read {}", path.display()))?;
        if last[0] != b'\n' {
            f.write_all(b"\n")?;
        }
    }
    f.write_all(text.as_bytes())?;
    if !text.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    Ok(())
}

pub fn log_path(save: &Path) -> Result<PathBuf> {
    latest_log(save)
        .context("no session log in Sessions/; create one first (talkdnd new does this)")
}

/// Last `## 存档点` section of a log (heading included).
pub fn last_checkpoint(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().rposition(|l| l.starts_with("## 存档点"))?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## ") || l.starts_with("# "))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    Some(lines[start..end].join("\n").trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_and_hidden_dirs_are_not_save_slots() {
        let root = std::env::temp_dir().join(format!("talkdnd-slots-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for slot in [
            "2030-01-02-01",
            "2030-01-02-02",
            "_backups/2030-01-02-01-20300101-120000-schema1",
            ".cache",
            "not-a-slot",
        ] {
            fs::create_dir_all(root.join("Saves").join(slot)).unwrap();
        }
        assert_eq!(
            slots(&root).unwrap(),
            vec!["2030-01-02-01".to_string(), "2030-01-02-02".to_string()]
        );
        assert_eq!(
            resolve_slot(&root, None).unwrap(),
            root.join("Saves/2030-01-02-02")
        );
        assert_eq!(
            next_slot_name(&root, "2030-01-02").unwrap(),
            "2030-01-02-03"
        );
        assert!(resolve_slot(&root, Some("_backups")).is_err());
        assert!(resolve_slot(&root, Some(".cache")).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn append_adds_a_newline_only_when_needed() {
        let root = std::env::temp_dir().join(format!("talkdnd-append-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let log = root.join("log.md");

        append(&log, "first").unwrap();
        assert_eq!(fs::read_to_string(&log).unwrap(), "first\n");

        append(&log, "second\n").unwrap();
        assert_eq!(fs::read_to_string(&log).unwrap(), "first\nsecond\n");

        // A large file without a trailing newline still gets exactly one.
        let big = root.join("big.md");
        let body = "x".repeat(5 * 1024 * 1024);
        fs::write(&big, &body).unwrap();
        append(&big, "tail").unwrap();
        let text = fs::read_to_string(&big).unwrap();
        assert!(text.starts_with(&body));
        assert!(text.ends_with("x\ntail\n"));

        let _ = fs::remove_dir_all(&root);
    }
}
