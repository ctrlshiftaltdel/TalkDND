//! Move a pre-LetsPlay checkout into `LetsPlay/`. Same rules as the former
//! `scripts/migrate_local_layout.sh`: dry-run unless `apply` is set, directories
//! merge, an existing destination is left untouched, and the summary text keeps
//! the old script name.

use std::fs;
use std::path::{Path, PathBuf};

pub struct Outcome {
    pub lines: Vec<String>,
    pub moves: u64,
    pub skips: u64,
    pub apply: bool,
}

impl Outcome {
    pub fn summary_line(&self) -> String {
        if self.apply {
            format!(
                "migrate_local_layout: applied, {} move(s), {} skip(s)",
                self.moves, self.skips
            )
        } else {
            format!(
                "migrate_local_layout: dry-run, {} move(s), {} skip(s) (pass --apply to move)",
                self.moves, self.skips
            )
        }
    }

    pub fn stdout(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&self.summary_line());
        out.push('\n');
        out
    }
}

struct Migrator {
    root: PathBuf,
    apply: bool,
    lines: Vec<String>,
    moves: u64,
    skips: u64,
}

pub fn run(root: &Path, apply: bool) -> Result<Outcome, String> {
    let mut migrator = Migrator {
        root: root.to_path_buf(),
        apply,
        lines: Vec::new(),
        moves: 0,
        skips: 0,
    };
    migrator.move_children("Rules/2014", "LetsPlay/Rules/2014")?;
    migrator.move_children("Rules/2024", "LetsPlay/Rules/2024")?;
    migrator.move_loose_rules()?;
    migrator.move_entry("Rules/combat/local", "LetsPlay/Rules/combat/local")?;
    migrator.move_entry(
        "Rules/combat/2024/monsters/scarecrow.yaml",
        "LetsPlay/Rules/combat/2024/monsters/scarecrow.yaml",
    )?;
    migrator.move_children("Adventures", "LetsPlay/Adventures")?;
    migrator.move_children("Saves", "LetsPlay/Saves")?;
    if apply {
        // Independent removes: a non-empty directory is left in place.
        // Nested paths come first, matching the shell's rmdir order.
        for rel in [
            "Rules/combat/2024/monsters",
            "Rules/combat/2024",
            "Rules/combat",
            "Rules/2014",
            "Rules/2024",
            "Rules",
            "Adventures",
            "Saves",
        ] {
            let _ = fs::remove_dir(migrator.root.join(rel));
        }
    }
    Ok(Outcome {
        lines: migrator.lines,
        moves: migrator.moves,
        skips: migrator.skips,
        apply,
    })
}

impl Migrator {
    fn move_loose_rules(&mut self) -> Result<(), String> {
        let rules = self.root.join("Rules");
        if !rules.is_dir() {
            return Ok(());
        }
        for name in child_names(&rules)? {
            if name == "2014" || name == "2024" || name == "combat" {
                continue;
            }
            self.move_entry(&format!("Rules/{name}"), &format!("LetsPlay/Rules/{name}"))?;
        }
        Ok(())
    }

    fn move_children(&mut self, src_dir: &str, dest_dir: &str) -> Result<(), String> {
        let src_path = self.root.join(src_dir);
        // `[ -d ]` follows symlinks. Missing paths and non-directories are no-ops.
        if !src_path.is_dir() {
            return Ok(());
        }
        let names = child_names(&src_path)?;
        for name in names {
            self.move_entry(
                &format!("{src_dir}/{name}"),
                &format!("{dest_dir}/{name}"),
            )?;
        }
        Ok(())
    }

    fn move_entry(&mut self, src: &str, dest: &str) -> Result<(), String> {
        let src_path = self.root.join(src);
        let dest_path = self.root.join(dest);
        if !present(&src_path) {
            return Ok(());
        }
        if real_dir(&src_path) && real_dir(&dest_path) {
            return self.move_children(src, dest);
        }
        if present(&dest_path) {
            self.skips += 1;
            self.lines
                .push(format!("skip (exists): {src} -> {dest}"));
            return Ok(());
        }
        self.moves += 1;
        self.lines.push(format!("move: {src} -> {dest}"));
        if self.apply {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("mkdir {}: {err}", parent.display()))?;
            }
            fs::rename(&src_path, &dest_path)
                .map_err(|err| format!("mv {src} -> {dest}: {err}"))?;
        }
        Ok(())
    }
}

fn present(path: &Path) -> bool {
    // True for a broken symlink, matching `[ -e ] || [ -L ]`.
    path.symlink_metadata().is_ok()
}

fn real_dir(path: &Path) -> bool {
    match path.symlink_metadata() {
        Ok(meta) => meta.file_type().is_dir(),
        Err(_) => false,
    }
}

/// Child names in the same three classes as `dir/*`, `dir/.[!.]*`, `dir/..?*`.
/// Within a class, names sort by raw bytes (POSIX glob order in the C locale).
fn child_names(dir: &Path) -> Result<Vec<String>, String> {
    let mut normal = Vec::new();
    let mut hidden = Vec::new();
    let mut dotdot = Vec::new();
    let entries = fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read {}: {err}", dir.display()))?;
        let name = entry.file_name().into_string().map_err(|_| {
            format!("non-utf8 file name under {}", dir.display())
        })?;
        match glob_class(&name) {
            Some(0) => normal.push(name),
            Some(1) => hidden.push(name),
            Some(2) => dotdot.push(name),
            _ => {}
        }
    }
    normal.sort();
    hidden.sort();
    dotdot.sort();
    normal.append(&mut hidden);
    normal.append(&mut dotdot);
    Ok(normal)
}

fn glob_class(name: &str) -> Option<u8> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." {
        return None;
    }
    if bytes[0] != b'.' {
        return Some(0);
    }
    if bytes.len() >= 2 && bytes[1] != b'.' {
        return Some(1);
    }
    if bytes.len() > 2 {
        Some(2)
    } else {
        None
    }
}
