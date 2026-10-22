//! Overwrite-update safety: the shipped-file manifest (`.talkdnd-files.txt`)
//! and the shipped defaults under `Rules/defaults/`.
//!
//! Release archives are extracted over an existing `LetsPlay/`, so an update
//! must not silently keep files the new version no longer ships, and it must
//! never overwrite the DM's own `Rules/rulings*.md`. Both are handled here.

use anyhow::{Context, Result};
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// File written into the release stage: one `<sha256>  <path>` line per
/// shipped file, paths relative to `LetsPlay/`.
pub const SHIPPED_LIST: &str = ".talkdnd-files.txt";
/// Local tool state (`Rules/defaults` hashes, previous install list, obsolete files).
pub const STATE_DIR: &str = ".talkdnd";

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// Default file name -> sha256 of the shipped copy at the last start.
    #[serde(default)]
    defaults: BTreeMap<String, String>,
}

fn state_path(root: &Path) -> PathBuf {
    root.join(STATE_DIR).join("state.yaml")
}

fn load_state(root: &Path) -> State {
    fs::read_to_string(state_path(root))
        .ok()
        .and_then(|text| serde_norway::from_str(&text).ok())
        .unwrap_or_default()
}

fn store_state(root: &Path, state: &State) -> Result<()> {
    let body = serde_norway::to_string(state)?;
    atomic_write(
        state_path(root),
        format!("# TalkDND local tool state (never commit).\n{body}").as_bytes(),
    )
}

/// Copy shipped `Rules/defaults/rulings*.md` to `Rules/` when the DM has no
/// copy yet. A copy that differs is left alone; when the shipped default has
/// changed since the previous start, a merge hint is returned.
pub fn sync_defaults(root: &Path) -> Result<Vec<String>> {
    let defaults = root.join("Rules").join("defaults");
    let Ok(entries) = fs::read_dir(&defaults) else {
        return Ok(Vec::new());
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("rulings") && n.ends_with(".md")
                })
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let mut state = load_state(root);
    let mut notes = Vec::new();
    for default in files {
        let name = default
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let bytes = fs::read(&default).with_context(|| format!("read {}", default.display()))?;
        let digest = sha256_hex(&bytes);
        let user = root.join("Rules").join(&name);
        if !user.is_file() {
            atomic_write(&user, &bytes)?;
            notes.push(format!(
                "裁定默认已复制到 `Rules/{name}`（出厂默认在 `Rules/defaults/{name}`）"
            ));
        } else {
            let current = fs::read(&user).with_context(|| format!("read {}", user.display()))?;
            if current != bytes {
                let changed = state.defaults.get(&name).is_some_and(|old| *old != digest);
                if changed {
                    notes.push(format!(
                        "`Rules/defaults/{name}` 自上次启动后已更新；你的 `Rules/{name}` 与默认不同，已保留，请手动合并（以你的裁定为准）"
                    ));
                }
            }
        }
        state.defaults.insert(name, digest);
    }
    store_state(root, &state)?;
    Ok(notes)
}

fn parse_list(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((sha, path)) = line.split_once("  ") {
            let path = path.trim();
            if !sha.is_empty() && !path.is_empty() {
                out.insert(path.to_string(), sha.to_ascii_lowercase());
            }
        }
    }
    out
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).is_relative()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Areas the DM owns. An update never moves or rewrites files there.
pub fn is_user_data(path: &str) -> bool {
    let path = path.trim_start_matches("./");
    if path == "Saves"
        || path.starts_with("Saves/")
        || path == "Adventures"
        || path.starts_with("Adventures/")
        || path.starts_with("Rules/2014/")
        || path.starts_with("Rules/2024/")
    {
        return true;
    }
    if let Some(rest) = path.strip_prefix("Rules/") {
        if !rest.contains('/') && rest.starts_with("rulings") && rest.ends_with(".md") {
            return true;
        }
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments
        .iter()
        .any(|s| matches!(*s, "local" | "fixes" | ".md"))
    {
        return true;
    }
    segments.last() == Some(&"handouts.yaml")
}

fn remove_obsolete(
    root: &Path,
    timestamp: &str,
    path: &str,
    message: &mut Vec<String>,
) -> Result<()> {
    let source = root.join(path);
    let dest = root
        .join(STATE_DIR)
        .join("obsolete")
        .join(timestamp)
        .join(path);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        fs::remove_file(&dest)?;
    }
    fs::rename(&source, &dest)
        .with_context(|| format!("move {} to {}", source.display(), dest.display()))?;
    message.push(format!(
        "旧版本文件已移走：`{path}` → `.talkdnd/obsolete/{timestamp}/{path}`"
    ));
    Ok(())
}

/// Compare the shipped file list of the current version with the previous
/// install. Files this version no longer ships are moved out of the play tree
/// when unmodified; modified files are only reported. User data is never
/// touched. Returns human-readable report lines.
pub fn sync_installed_files(root: &Path) -> Result<Vec<String>> {
    let shipped_path = root.join(SHIPPED_LIST);
    if !shipped_path.is_file() {
        return Ok(Vec::new());
    }
    let shipped_text = fs::read_to_string(&shipped_path)
        .with_context(|| format!("read {}", shipped_path.display()))?;
    let shipped = parse_list(&shipped_text);
    if shipped.is_empty() {
        return Ok(Vec::new());
    }
    let installed_path = root.join(STATE_DIR).join("installed-files.txt");
    let previous = fs::read_to_string(&installed_path)
        .ok()
        .map(|text| parse_list(&text))
        .unwrap_or_default();
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut notes = Vec::new();
    if !previous.is_empty() {
        for (path, old_sha) in &previous {
            if shipped.contains_key(path) || !safe_relative(path) || is_user_data(path) {
                continue;
            }
            let current_path = root.join(path);
            if !current_path.is_file() {
                continue;
            }
            let current_sha = sha256_file(&current_path)?;
            if current_sha.eq_ignore_ascii_case(old_sha) {
                remove_obsolete(root, &timestamp, path, &mut notes)?;
            } else {
                notes.push(format!(
                    "旧版本文件 `{path}` 已修改，保留原样（新版本不再附带它；需要时手动删除）"
                ));
            }
        }
    }
    atomic_write(&installed_path, shipped_text.as_bytes())?;
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("talkdnd-update-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn copies_missing_default_and_hints_only_after_a_default_change() {
        let root = temp("defaults");
        write(&root.join("Rules/defaults/rulings.md"), "default one\n");
        let notes = sync_defaults(&root).unwrap();
        assert!(notes.iter().any(|n| n.contains("已复制")), "{notes:?}");
        assert_eq!(
            fs::read_to_string(root.join("Rules/rulings.md")).unwrap(),
            "default one\n"
        );

        // Second start: nothing to do.
        assert!(sync_defaults(&root).unwrap().is_empty());

        // DM edits the copy.
        fs::write(root.join("Rules/rulings.md"), "my table rule\n").unwrap();
        assert!(sync_defaults(&root).unwrap().is_empty());

        // The shipped default changes: hint, but keep the DM's copy.
        fs::write(root.join("Rules/defaults/rulings.md"), "default two\n").unwrap();
        let notes = sync_defaults(&root).unwrap();
        assert!(notes.iter().any(|n| n.contains("请手动合并")), "{notes:?}");
        assert_eq!(
            fs::read_to_string(root.join("Rules/rulings.md")).unwrap(),
            "my table rule\n"
        );
        let state = fs::read_to_string(state_path(&root)).unwrap();
        assert!(state.contains("rulings.md"), "{state}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn moves_unmodified_obsolete_files_and_reports_modified_ones() {
        let root = temp("obsolete");
        for (path, body) in [
            ("old.md", "old body"),
            ("changed.md", "edited after install"),
            ("Saves/keep.md", "save"),
            ("Adventures/keep.md", "adventure"),
            ("Rules/2014/keep.md", "book"),
            ("Rules/rulings.md", "my rulings"),
            ("Rules/chargen/local/x.yaml", "local"),
            ("docs/new.md", "new"),
            ("handouts.yaml", "handout registry"),
            ("X/.md/fixes/y.md", "generated"),
        ] {
            write(&root.join(path), body);
        }
        let old_sha = sha256_hex(b"old body");
        let changed_sha = sha256_hex(b"was different");
        let previous = format!(
            "{old_sha}  old.md\n{changed_sha}  changed.md\n{}  Saves/keep.md\n{}  Adventures/keep.md\n{}  Rules/2014/keep.md\n{}  Rules/rulings.md\n{}  Rules/chargen/local/x.yaml\n{}  handouts.yaml\n{}  X/.md/fixes/y.md\n",
            sha256_hex(b"save"),
            sha256_hex(b"adventure"),
            sha256_hex(b"book"),
            sha256_hex(b"my rulings"),
            sha256_hex(b"local"),
            sha256_hex(b"handout registry"),
            sha256_hex(b"generated"),
        );
        write(&root.join(".talkdnd/installed-files.txt"), &previous);
        let shipped = format!("{}  docs/new.md\n", sha256_hex(b"new"));
        write(&root.join(SHIPPED_LIST), &shipped);

        let notes = sync_installed_files(&root).unwrap();
        assert!(!root.join("old.md").exists());
        assert!(root.join("changed.md").exists());
        for keep in [
            "Saves/keep.md",
            "Adventures/keep.md",
            "Rules/2014/keep.md",
            "Rules/rulings.md",
            "Rules/chargen/local/x.yaml",
            "handouts.yaml",
            "X/.md/fixes/y.md",
            "docs/new.md",
        ] {
            assert!(root.join(keep).is_file(), "{keep} moved");
        }
        let moved_root = root.join(".talkdnd/obsolete");
        let stamp = fs::read_dir(&moved_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(stamp.join("old.md").is_file());
        assert!(notes.iter().any(|n| n.contains("old.md")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("changed.md")), "{notes:?}");
        assert_eq!(
            fs::read_to_string(root.join(".talkdnd/installed-files.txt")).unwrap(),
            shipped
        );

        // The previous list is now current: a second run does nothing.
        assert!(sync_installed_files(&root).unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn user_data_areas_are_recognized() {
        for path in [
            "Saves/a.md",
            "Adventures/a.md",
            "Rules/2014/PHB.md",
            "Rules/2024/PHB.md",
            "Rules/rulings.md",
            "Rules/rulings-2024.md",
            "Rules/chargen/local/x.yaml",
            "some/fixes/y.md",
            "some/.md/z.md",
            "Saves/foo/handouts.yaml",
        ] {
            assert!(is_user_data(path), "{path}");
        }
        for path in [
            "docs/x.md",
            "Rules/defaults/rulings.md",
            "Rules/combat/common/ac.yaml",
        ] {
            assert!(!is_user_data(path), "{path}");
        }
    }

    #[test]
    fn rejects_escaping_paths() {
        let root = temp("escape");
        write(&root.join(SHIPPED_LIST), "abc  ok.md\n");
        write(
            &root.join(".talkdnd/installed-files.txt"),
            &format!("{}  ../../evil.md\n", sha256_hex(b"x")),
        );
        assert!(sync_installed_files(&root).unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
