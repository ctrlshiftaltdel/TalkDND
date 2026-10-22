//! Save schema metadata (`Saves/<slot>/save-meta.yaml`) and migrations.
//!
//! The schema number describes the on-disk layout of a save:
//! 1 = Markdown sheets only, 2 = structured `sheets/<id>.yaml`, 3 = versioned
//! YAML files plus `save-meta.yaml`. `talkdnd start` and `talkdnd migrate`
//! back the whole save up into `Saves/_backups/` before writing, then run the
//! migrations in order. A save from a newer talkdnd is refused: this binary
//! cannot know its format.
//!
//! Migrations only update the integrity manifest for files that were clean
//! before; a file the DM had already hand-edited stays flagged.

use crate::classdata::Book;
use crate::progress::Progress;
use crate::reveal;
use crate::{charsheet, check, save};
use anyhow::{bail, Context, Result};
use combat_sim::fsutil::atomic_write;
use combat_sim::party::PartyState;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "save-meta.yaml";
pub const CURRENT_SCHEMA: u32 = 3;
pub const BACKUP_DIR: &str = "_backups";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub date: String,
    /// Path relative to the play folder, e.g. `Saves/_backups/<name>`.
    #[serde(default)]
    pub backup: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SaveMeta {
    pub schema: u32,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub migrations: Vec<Migration>,
}

pub fn path(save: &Path) -> PathBuf {
    save.join(FILE)
}

/// Load `save-meta.yaml`; a schema from a newer talkdnd is an error.
pub fn load(save: &Path) -> Result<Option<SaveMeta>> {
    let p = path(save);
    if !p.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let meta: SaveMeta =
        serde_norway::from_str(&text).with_context(|| format!("invalid {}", p.display()))?;
    if meta.schema > CURRENT_SCHEMA {
        bail!(
            "{}: 存档 schema {} 比这个 talkdnd 支持的版本（{}）新；请更新 talkdnd 后再打开（update talkdnd）",
            p.display(),
            meta.schema,
            CURRENT_SCHEMA
        );
    }
    Ok(Some(meta))
}

/// Current schema of a save. Missing metadata: 1 when a Markdown sheet still
/// has no structured file, else 2 (the pre-`save-meta.yaml` layout).
pub fn detect(save: &Path) -> Result<u32> {
    if let Some(meta) = load(save)? {
        return Ok(meta.schema.max(1));
    }
    for sheet in check::sheets(save) {
        if !charsheet::path(save, &sheet.id).is_file() {
            return Ok(1);
        }
    }
    Ok(2)
}

/// Write the current schema for a brand-new save.
pub fn init(save: &Path) -> Result<()> {
    if path(save).is_file() {
        return Ok(());
    }
    let meta = SaveMeta {
        schema: CURRENT_SCHEMA,
        tool: env!("CARGO_PKG_VERSION").to_string(),
        migrations: Vec::new(),
    };
    write_meta(save, &meta)
}

fn write_meta(save: &Path, meta: &SaveMeta) -> Result<()> {
    let text = format!(
        "# TalkDND save schema metadata (written by talkdnd; do not edit).\n# Backups from migrations live in Saves/_backups/.\n{}",
        serde_norway::to_string(meta)?
    );
    atomic_write(path(save), text.as_bytes())
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub from: u32,
    pub to: u32,
    pub dry_run: bool,
    /// Backup path relative to the play folder.
    pub backup: Option<String>,
    pub sheets: Vec<String>,
    pub log_text: Option<String>,
}

impl Report {
    pub fn already_current(&self) -> bool {
        self.from == self.to
    }

    pub fn lines(&self) -> Vec<String> {
        if self.already_current() {
            return vec![format!("存档 schema {}：无需迁移", self.to)];
        }
        if self.dry_run {
            let mut v = vec![format!(
                "dry-run：存档 schema {} → {}；会先把整份存档备份到 Saves/{BACKUP_DIR}/，再迁移（未写入任何文件）",
                self.from, self.to
            )];
            if !self.sheets.is_empty() {
                v.push(format!("会迁移角色卡：{}", self.sheets.join(", ")));
            }
            return v;
        }
        let mut v = vec![format!("存档 schema {} → {}：迁移完成", self.from, self.to)];
        if let Some(b) = &self.backup {
            v.push(format!("迁移前整份备份：`{b}`"));
        }
        if !self.sheets.is_empty() {
            v.push(format!(
                "已迁移角色卡 {} 张：{}",
                self.sheets.len(),
                self.sheets.join(", ")
            ));
        }
        if let Some(log) = &self.log_text {
            v.push(format!("已写入会话日志：{log}"));
        }
        v
    }
}

/// Migrate `save` to [`CURRENT_SCHEMA`]. `dry_run` reports only and writes
/// nothing. Idempotent: a save already at the current schema is left untouched.
pub fn ensure(save: &Path, root: &Path, dry_run: bool) -> Result<Report> {
    let from = detect(save)?;
    if from >= CURRENT_SCHEMA {
        return Ok(Report {
            from,
            to: from,
            dry_run,
            ..Default::default()
        });
    }
    let mut report = Report {
        from,
        to: CURRENT_SCHEMA,
        dry_run,
        ..Default::default()
    };
    if dry_run {
        if from < 2 {
            for sheet in check::sheets(save) {
                if !charsheet::path(save, &sheet.id).is_file() {
                    report.sheets.push(sheet.id);
                }
            }
        }
        return Ok(report);
    }

    // Snapshot before any write: after the migration only files that were
    // clean before are recorded, so pre-existing hand edits stay flagged.
    let before = combat_sim::integrity::snapshot(save);
    let slot = save
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "save".into());
    let backup = backup_save(save, root, &slot, from)?;
    report.backup = Some(backup.clone());

    if from < 2 {
        report.sheets = migrate_sheets(save, root)?;
    }
    add_versions(save)?;

    let mut meta = load(save)?.unwrap_or_default();
    meta.schema = CURRENT_SCHEMA;
    meta.tool = env!("CARGO_PKG_VERSION").to_string();
    let date = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    if from < 2 {
        meta.migrations.push(Migration {
            from: 1,
            to: 2,
            date: date.clone(),
            backup: backup.clone(),
        });
    }
    meta.migrations.push(Migration {
        from: from.max(2),
        to: 3,
        date,
        backup: backup.clone(),
    });
    write_meta(save, &meta)?;

    let text = format!(
        "[{}] 存档结构迁移 schema {from}→{CURRENT_SCHEMA}（备份：{backup}）",
        chrono::Local::now().format("%H:%M")
    );
    if let Some(log) = save::latest_log(save) {
        save::append(&log, &format!("- {text}"))?;
        report.log_text = Some(text);
    }

    combat_sim::integrity::after_write(save, &before)?;
    Ok(report)
}

/// Recursive copy of the save, skipping machine-only directories.
fn backup_save(save: &Path, root: &Path, slot: &str, schema: u32) -> Result<String> {
    let parent = root.join("Saves").join(BACKUP_DIR);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut name = format!("{slot}-{stamp}-schema{schema}");
    let mut n = 2;
    while parent.join(&name).exists() {
        name = format!("{slot}-{stamp}-schema{schema}-{n}");
        n += 1;
    }
    let dest = parent.join(&name);
    copy_tree(save, &dest)?;
    Ok(format!("Saves/{BACKUP_DIR}/{name}"))
}

fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(name.as_str(), BACKUP_DIR | ".cache" | ".talkdnd") {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

fn migrate_sheets(save: &Path, root: &Path) -> Result<Vec<String>> {
    let book = Book::load(&save::ruleset(save), Some(root))?;
    let mut migrated = Vec::new();
    for sheet in check::sheets(save) {
        if charsheet::path(save, &sheet.id).is_file() {
            continue;
        }
        if !crate::chargen::valid_id(&sheet.id) {
            bail!(
                "invalid character id `{}` in {}; cannot migrate it (use lowercase letters, digits and hyphens)",
                sheet.id,
                sheet.file.display()
            );
        }
        let cf = charsheet::migrate(save, &sheet, &book)?;
        charsheet::store(save, &cf)?;
        migrated.push(sheet.id);
    }
    Ok(migrated)
}

/// 2→3: rewrite the YAML files through their stores so they carry `version: 1`
/// (additive; older files still load).
fn add_versions(save: &Path) -> Result<()> {
    if Progress::path(save).is_file() {
        let progress = Progress::load(save)?;
        progress.store(save)?;
    }
    if save.join(reveal::FILE).is_file() {
        let ledger = reveal::load(save)?;
        reveal::store(save, &ledger)?;
    }
    let party = save.join("combat").join(combat_sim::party::FILE_NAME);
    if party.is_file() {
        let state = PartyState::load(&party)?;
        state.save(&party)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = "# Hero\n\n- **等级**：战士 2\n\n```combat yaml\nid: hero\nname: Hero\nkind: pc\nside: party\nlevel: 2\nac: 16\nhp: 20\nspeed: {walk: 30}\nprof: 2\nabilities: {str: 16, dex: 10, con: 14, int: 10, wis: 10, cha: 12}\nnotes: \"fighter 2\"\n```\n";

    fn root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("talkdnd-migrate-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Saves")).unwrap();
        dir
    }

    fn save_with_sheet(root: &Path) -> PathBuf {
        let save = root.join("Saves/2030-01-01-01");
        for dir in ["Characters", "NPCs", "Sessions", "combat"] {
            fs::create_dir_all(save.join(dir)).unwrap();
        }
        fs::write(
            save.join("combat/campaign.yaml"),
            "name: X\nruleset: \"2024\"\nparty: []\n",
        )
        .unwrap();
        fs::write(save.join("Characters/hero.md"), SHEET).unwrap();
        fs::write(save.join("Sessions/2030-01-01-session-00.md"), "# log\n").unwrap();
        fs::write(save.join("progress.yaml"), "xp: {hero: 0}\n").unwrap();
        save
    }

    fn meta(save: &Path) -> SaveMeta {
        load(save).unwrap().expect("save-meta.yaml")
    }

    #[test]
    fn migrates_a_v1_save_and_is_idempotent() {
        let root = root("v1");
        let save = save_with_sheet(&root);
        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (1, 3));
        assert_eq!(report.sheets, vec!["hero".to_string()]);
        assert!(charsheet::path(&save, "hero").is_file());
        assert_eq!(meta(&save).schema, 3);
        assert!(meta(&save)
            .migrations
            .iter()
            .any(|m| m.from == 1 && m.to == 2));

        let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves").join(BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].join("Characters/hero.md").is_file());
        assert!(backups[0].join("progress.yaml").is_file());
        assert!(!backups[0].join("_backups").exists());

        let log = fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md")).unwrap();
        assert!(log.contains("存档结构迁移 schema 1→3"), "{log}");
        assert!(log.contains("Saves/_backups/"), "{log}");
        assert!(combat_sim::integrity::changes(&save).unwrap().is_empty());

        // A second start does nothing.
        let log_lines = log.lines().count();
        let again = ensure(&save, &root, false).unwrap();
        assert!(again.already_current());
        assert!(again.backup.is_none());
        let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves").join(BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        let log = fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md")).unwrap();
        assert_eq!(log.lines().count(), log_lines);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let root = root("dry");
        let save = save_with_sheet(&root);
        let report = ensure(&save, &root, true).unwrap();
        assert_eq!((report.from, report.to), (1, 3));
        assert!(report.dry_run && report.sheets == vec!["hero".to_string()]);
        assert!(!path(&save).is_file());
        assert!(!charsheet::path(&save, "hero").is_file());
        assert!(!root.join("Saves").join(BACKUP_DIR).exists());
        let log = fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md")).unwrap();
        assert!(!log.contains("迁移"), "{log}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pre_edited_file_stays_flagged_after_migration() {
        let root = root("edited");
        let save = root.join("Saves/2030-02-02-01");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(save.join("Sessions/2030-02-02-session-00.md"), "# log\n").unwrap();
        fs::write(
            save.join("progress.yaml"),
            "xp: {hero: 0}\npurses:\n  hero: {cp: 500}\n",
        )
        .unwrap();
        combat_sim::integrity::ensure(&save).unwrap();
        let edited = fs::read_to_string(save.join("progress.yaml"))
            .unwrap()
            .replace("cp: 500", "cp: 900000");
        fs::write(save.join("progress.yaml"), edited).unwrap();

        let report = ensure(&save, &root, false).unwrap();
        assert_eq!((report.from, report.to), (2, 3));
        assert!(meta(&save).schema == 3);
        let changes = combat_sim::integrity::changes(&save).unwrap();
        assert!(
            changes
                .iter()
                .any(|c| c.path() == "progress.yaml" && c.describe().contains("changed")),
            "{changes:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let root = root("newer");
        let save = save_with_sheet(&root);
        fs::write(path(&save), "schema: 99\ntool: future\n").unwrap();
        let err = ensure(&save, &root, false).unwrap_err().to_string();
        assert!(err.contains("99") && err.contains("talkdnd"), "{err}");
        assert!(!root.join("Saves").join(BACKUP_DIR).exists());
        let _ = fs::remove_dir_all(&root);
    }
}
