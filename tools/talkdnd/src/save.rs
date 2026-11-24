//! Locating the play folder, saves and session logs.

use anyhow::{bail, Context, Result};
use combat_sim::content::loader::Ruleset;
use combat_sim::fsutil::atomic_write;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Marker file at the top of every play folder (shipped in the release).
pub const ROOT_MARKER: &str = ".talkdnd-root";

/// A play folder: has the marker, or the classic `AGENTS.md` + `Saves/` pair.
pub fn is_play_root(p: &Path) -> bool {
    p.join(ROOT_MARKER).is_file() || (p.join("AGENTS.md").is_file() && p.join("Saves").is_dir())
}

/// The play folder, independent of its name and location. Order:
/// `--root`, then `TALKDND_ROOT`, then the parent of the executable's `bin/`
/// (the normal case for an extracted release, whatever the cwd), then the
/// nearest ancestor of the current directory that is a play folder or has a
/// `LetsPlay/` child (development repository).
pub fn find_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(absolute(p));
    }
    if let Some(p) = std::env::var_os("TALKDND_ROOT").filter(|v| !v.is_empty()) {
        let p = absolute(Path::new(&p));
        if !is_play_root(&p) {
            bail!(
                "TALKDND_ROOT={} is not a play folder (no {ROOT_MARKER} or AGENTS.md + Saves/)",
                p.display()
            );
        }
        return Ok(p);
    }
    if let Some(root) = std::env::current_exe()
        .ok()
        .and_then(|e| e.canonicalize().ok().or(Some(e)))
        .and_then(|e| e.parent().and_then(|b| b.parent()).map(Path::to_path_buf))
    {
        if is_play_root(&root) {
            return Ok(root);
        }
    }
    let cwd = std::env::current_dir()?;
    for dir in cwd.ancestors() {
        if is_play_root(dir) {
            return Ok(dir.to_path_buf());
        }
        if is_play_root(&dir.join("LetsPlay")) {
            return Ok(dir.join("LetsPlay"));
        }
    }
    bail!("play folder not found: run the program from its bin/ folder inside the extracted package, run inside that folder, or pass --root / set TALKDND_ROOT")
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
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

/// The save-visible ruleset from `combat/campaign.yaml`. Missing file/value
/// defaults to `5.5e-srd`; the old `2014` / `2024` values map to the SRD
/// variants. Any other value is an actionable error (fail closed).
pub fn ruleset(save: &Path) -> Result<Ruleset> {
    let path = save.join("combat/campaign.yaml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Ruleset::FivePointFiveESrd)
        }
        Err(error) => {
            return Err(error).with_context(|| format!("read {}", path.display()));
        }
    };
    let Some(value) = manifest_ruleset_value(&text) else {
        return Ok(Ruleset::FivePointFiveESrd);
    };
    Ruleset::parse(&value).with_context(|| format!("in {}", path.display()))
}

/// Engine edition (`2014` / `2024`) for callers that select the on-disk layer.
pub fn edition(save: &Path) -> Result<String> {
    Ok(ruleset(save)?.edition().to_string())
}

/// Rewrites an old `ruleset` value in place, preserving every other line and
/// the player's comments. The migration is idempotent and records itself in a
/// comment next to the field. Returns a digest line when it wrote something.
pub fn migrate_ruleset(save: &Path) -> Result<Option<String>> {
    let path = save.join("combat/campaign.yaml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let Some(old) = manifest_ruleset_value(&text) else {
        return Ok(None);
    };
    let canonical = Ruleset::parse(&old)
        .with_context(|| format!("in {}", path.display()))?
        .value();
    if old == canonical {
        return Ok(None);
    }

    let mut out = String::with_capacity(text.len() + 96);
    let mut replaced = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !replaced && trimmed.starts_with("ruleset:") {
            replaced = true;
            let indent = &line[..line.len() - trimmed.len()];
            let comment = trimmed
                .find('#')
                .map(|index| trimmed[index..].trim_end().to_string());
            let eol = if line.ends_with("\r\n") { "\r\n" } else { "\n" };
            let comment = match comment.as_deref() {
                Some("# 2014 | 2024") => " # 5.5e-full | 5e-full | 5.5e-srd | 5e-srd".to_string(),
                Some(comment) => format!(" {comment}"),
                None => String::new(),
            };
            out.push_str(&format!(
                "{indent}# ruleset migrated from {old} to {canonical}{eol}"
            ));
            out.push_str(&format!("{indent}ruleset: \"{canonical}\"{comment}{eol}"));
            continue;
        }
        out.push_str(line);
    }
    if !replaced {
        return Ok(None);
    }
    let before = combat_sim::integrity::snapshot(save);
    atomic_write(&path, out.as_bytes())?;
    combat_sim::integrity::after_write(save, &before)?;
    Ok(Some(format!("ruleset：{old} -> {canonical}（已迁移）")))
}

/// Raw `ruleset:` value without its inline comment or quotes.
fn manifest_ruleset_value(text: &str) -> Option<String> {
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("ruleset:") else {
            continue;
        };
        let value = rest
            .split('#')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(['"', '\'']);
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// Refuse a log append on a protocol-enabled save unless this process already
/// holds SAVE. `Ok(None)` from the enclosing-dir walk is an uninitialized
/// save and is allowed. An unresolvable path fails closed.
fn require_save_for_log_append(path: &Path) -> Result<()> {
    match ::advancement_lock::enclosing_protocol_dir(path)? {
        Some(save)
            if ::advancement_lock::has_protocol(&save)
                && !::advancement_lock::is_save_lock_held(&save) =>
        {
            bail!(
                "协议违规：在取得 SAVE 锁之前追加日志 {}；总序必须是先持有 SAVE 再写日志。未写入任何目标文件",
                path.display()
            );
        }
        Some(_) | None => Ok(()),
    }
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
///
/// Session logs, `rolls.log`, checkpoint restore notes and migrate session
/// lines all come through here. On a protocol-enabled save the caller must
/// already hold SAVE. An uninitialized save (no control file on the path or
/// its ancestors) appends as before. A refusal writes nothing and leaves
/// existing lines unchanged.
pub fn append(path: &Path, text: &str) -> Result<()> {
    require_save_for_log_append(path)?;
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

    #[test]
    fn a_protocol_append_requires_save_and_does_not_truncate() {
        let root = std::env::temp_dir().join(format!(
            "talkdnd-append-proto-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        let save = root.join("Saves/slot");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        let log = save.join("Sessions/log.md");
        fs::write(&log, "kept line\n").unwrap();
        match ::advancement_lock::init(&root, &save, &::advancement_lock::InitOptions::new()) {
            Ok(_) => {}
            Err(err) if err.kind() == ::advancement_lock::ErrorKind::StatePublishedNotDurable => {}
            Err(err) => panic!("init protocol: {err}"),
        }
        let lock_path = ::advancement_lock::lock_path(&save);
        let before_lock = fs::read(&lock_path).unwrap();
        let before_state = fs::read(::advancement_lock::state_path(&save)).unwrap();
        let before_log = fs::read(&log).unwrap();

        let err = append(&log, "must-not-land").unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("协议违规") && text.contains("未写入任何目标文件"),
            "{text}"
        );
        assert_eq!(fs::read(&log).unwrap(), before_log);

        let held = ::advancement_lock::open(&root, &save, &::advancement_lock::OpenOptions::new())
            .unwrap();
        held.check_ordinary_write().unwrap();
        let meta = fs::symlink_metadata(&lock_path).unwrap();
        assert!(meta.is_file());
        append(&log, "added").unwrap();
        drop(held);

        let after = fs::read(&log).unwrap();
        assert!(after.starts_with(&before_log));
        assert!(String::from_utf8_lossy(&after).contains("added"));
        assert_eq!(fs::read(&lock_path).unwrap(), before_lock);
        assert_eq!(
            fs::read(::advancement_lock::state_path(&save)).unwrap(),
            before_state
        );

        let plain = root.join("plain");
        fs::create_dir_all(&plain).unwrap();
        let plain_log = plain.join("log.md");
        append(&plain_log, "free").unwrap();
        assert_eq!(fs::read_to_string(&plain_log).unwrap(), "free\n");
        assert!(!plain.join(::advancement_lock::LOCK_FILE).exists());
        let _ = fs::remove_dir_all(&root);
    }
}
