//! Checkpoints (P01; LetsPlay/docs/campaign-continuity.md §5.1).
//!
//! `talkdnd checkpoint <text>` (text as arguments or on stdin) appends a
//! `## 存档点` section to the newest session log — the original behavior,
//! kept here so that lib.rs only registers and forwards.
//! `checkpoint snapshot|list|show|restore` manage whole-save checkpoint
//! snapshots: `checkpoints/<id>/manifest.yaml` (version 1, sha256 per covered
//! file) plus data copies of the covered files. Snapshot keeps the newest 10
//! and reports deletions; restore backs the whole save up first, replaces the
//! covered files atomically (temp file + rename), deletes covered files that
//! appeared after the snapshot, migrates when the checkpoint schema is older,
//! logs the restore and re-baselines the integrity manifest. A failed
//! commit rolls back by mirroring the pre-restore backup over the save —
//! copying the backup back *and* removing save entries the backup does not
//! have — and only reports "回滚到恢复前状态" once the whole tree matches.
//! Snapshot and restore are serialized per slot with `Saves/<slot>/.talkdnd.lock`;
//! restore's final integrity re-baseline (and the migration it may run) then
//! takes the per-save integrity manifest lock `.talkdnd.integrity.lock`
//! inside it — always checkpoint lock first, integrity lock second, never
//! the reverse.

use crate::{audit, migrate, save};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use combat_sim::fsutil::atomic_write;
use combat_sim::integrity;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Checkpoint directory inside a save.
pub const DIR: &str = "checkpoints";
const MANIFEST: &str = "manifest.yaml";
const VERSION: u32 = 1;
/// Checkpoints kept after every snapshot; older ones are deleted and reported.
const KEEP: usize = 10;
const LOCK: &str = ".talkdnd.lock";
/// A lock older than this counts as abandoned (mirrors the pdf2md worker lock).
const LOCK_STALE_SECS: u64 = 600;
const BACKUP_DIR: &str = "_backups";
/// Top-level files covered by a checkpoint (each one when it exists).
const COVERED_FILES: [&str; 7] = [
    "progress.yaml",
    "reveals.yaml",
    "save-meta.yaml",
    "original.yaml",
    "identified.yaml",
    "canon.md",
    "house-rules.md",
];
/// Directories covered by a checkpoint, recursively.
const COVERED_DIRS: [&str; 4] = ["Characters", "NPCs", "sheets", "combat"];

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// New checkpoint: snapshot the whole save and write a manifest
    Snapshot {
        /// Why this checkpoint is taken (logged)
        #[arg(long)]
        reason: Option<String>,
        /// Single-line JSON to stdout (the manifest content)
        #[arg(long)]
        json: bool,
    },
    /// List checkpoints: id, created, reason, log anchor, usable
    List {
        /// Single-line JSON to stdout
        #[arg(long)]
        json: bool,
    },
    /// Show one checkpoint: manifest fields, file count, total bytes
    Show {
        /// Checkpoint id
        id: String,
        /// Single-line JSON to stdout
        #[arg(long)]
        json: bool,
    },
    /// Restore a checkpoint (the whole save is backed up first); needs --yes
    Restore {
        /// Checkpoint id
        id: String,
        /// Confirm the restore (required)
        #[arg(long)]
        yes: bool,
        /// Single-line JSON to stdout
        #[arg(long)]
        json: bool,
    },
}

/// `checkpoints/<id>/manifest.yaml`, version 1 (contract §4.1).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub schema: u32,
    pub log_file: String,
    pub log_lines: usize,
    /// Relative path -> `sha256:<hex>` for every covered data copy.
    pub files: BTreeMap<String, String>,
}

/// One checkpoint as reported by `list` / `show` (never fatal for the others).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub schema: u32,
    pub log_file: String,
    pub log_lines: usize,
    pub files: usize,
    pub bytes: u64,
    pub usable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// `checkpoint snapshot|list|show|restore` (P01, TASK-0059).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Snapshot { reason, json } => snapshot(root, save, reason, json),
        Cmd::List { json } => list(save, json),
        Cmd::Show { id, json } => show(save, &id, json),
        Cmd::Restore { id, yes, json } => restore(root, save, &id, yes, json),
    }
}

/// `checkpoint <text>` (arguments or stdin): append a `## 存档点` section to
/// the newest session log. Behavior unchanged from before the checkpoint
/// command group split; it lives here so lib.rs stays forwarding-only.
pub fn append(root: &Path, save: &Path, text: Vec<String>) -> Result<()> {
    let mut body = text.join(" ");
    if body.trim().is_empty() {
        std::io::stdin().read_to_string(&mut body)?;
    }
    if body.trim().is_empty() {
        bail!("checkpoint text is empty (pass it as arguments or on stdin)");
    }
    let log = save::log_path(save)?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M");
    save::append(&log, &format!("\n## 存档点（{now}）\n\n{}\n", body.trim()))?;
    println!("checkpoint appended to {}", rel(root, &log));
    Ok(())
}

// ---------------------------------------------------------------- snapshot

fn snapshot(root: &Path, save: &Path, reason: Option<String>, json: bool) -> Result<()> {
    let _lock = Lock::acquire(save)?;
    let Some(log) = save::latest_log(save) else {
        bail!("没有会话日志（Sessions/）；先 `talkdnd new` 建一个存档，再打检查点");
    };
    let log_file = file_name(&log);
    let log_lines = fs::read_to_string(&log)
        .map(|t| t.lines().count())
        .unwrap_or(0);
    let schema = migrate::detect(save)?;
    let files = covered_hashes(save)?;
    let id = new_id(save, reason.as_deref());
    let dir = checkpoint_dir(save, &id);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    for rel_path in files.keys() {
        let dst = dir.join(rel_path);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(save.join(rel_path), &dst)
            .with_context(|| format!("copy {rel_path} into the checkpoint"))?;
    }
    let manifest = Manifest {
        version: VERSION,
        id: id.clone(),
        created: stamp(),
        reason,
        schema,
        log_file,
        log_lines,
        files,
    };
    let manifest_path = dir.join(MANIFEST);
    atomic_write(
        &manifest_path,
        serde_norway::to_string(&manifest)?.as_bytes(),
    )
    .with_context(|| format!("write {}", manifest_path.display()))?;
    let deleted = retain(save);
    if json {
        println!("{}", serde_json::to_string(&manifest)?);
    } else {
        println!(
            "checkpoint {id} 已创建：{} 个文件 → {}",
            manifest.files.len(),
            rel(root, &dir)
        );
        if deleted.is_empty() {
            println!(
                "保留策略：共 {} 个检查点（上限 {KEEP}）",
                checkpoint_ids(save).len()
            );
        } else {
            println!(
                "保留策略：删除 {} 个更早的检查点：{}",
                deleted.len(),
                deleted.join(", ")
            );
        }
    }
    Ok(())
}

/// Keep only the newest `KEEP` checkpoints; returns the deleted ids.
fn retain(save: &Path) -> Vec<String> {
    let mut ids = checkpoint_ids(save);
    let mut deleted = Vec::new();
    while ids.len() > KEEP {
        let id = ids.remove(0);
        let _ = fs::remove_dir_all(checkpoint_dir(save, &id));
        deleted.push(id);
    }
    deleted
}

// -------------------------------------------------------------- list / show

fn list(save: &Path, json: bool) -> Result<()> {
    let all = entries(save);
    if json {
        println!("{}", serde_json::to_string(&all)?);
        return Ok(());
    }
    if all.is_empty() {
        println!("（还没有检查点；`talkdnd checkpoint snapshot [--reason ...]` 建一个）");
        return Ok(());
    }
    for e in &all {
        let reason = e.reason.as_deref().unwrap_or("-");
        let status = match &e.problem {
            Some(p) => format!("不可用（{p}）"),
            None => "可用".to_string(),
        };
        println!(
            "{}  {}  {}  日志 {}@{}  {}",
            e.id, e.created, reason, e.log_file, e.log_lines, status
        );
    }
    Ok(())
}

fn show(save: &Path, id: &str, json: bool) -> Result<()> {
    if bad_id(id) {
        bail!("检查点 id `{id}` 不合法");
    }
    if !checkpoint_dir(save, id).is_dir() {
        bail!("检查点 {id} 不存在；`checkpoint list` 看全部");
    }
    let e = entry(save, id);
    if json {
        println!("{}", serde_json::to_string(&e)?);
    } else {
        println!("检查点 {}", e.id);
        println!("- 创建：{}", e.created);
        println!("- 原因：{}", e.reason.as_deref().unwrap_or("-"));
        println!("- 快照时 schema：{}", e.schema);
        println!("- 日志锚点：{}@{}", e.log_file, e.log_lines);
        println!("- 文件：{} 个，{} 字节", e.files, e.bytes);
        match &e.problem {
            Some(p) => println!("- 状态：不可用（{p}）"),
            None => println!("- 状态：可用（{} 个副本 sha256 与清单一致）", e.files),
        }
    }
    if let Some(problem) = &e.problem {
        bail!("检查点 {id} 不可用（{problem}）");
    }
    Ok(())
}

// ---------------------------------------------------------------- restore

fn restore(root: &Path, save: &Path, id: &str, yes: bool, json: bool) -> Result<()> {
    restore_inner(root, save, id, yes, json, None, None)
}

/// Test-only fault injection for `restore` (production always passes
/// `None`): `fail_rename_at` makes the n-th commit rename (0-based,
/// commit order) fail with a synthetic error after real staging, and
/// `fail_rollback` makes the whole-save rollback fail the same way. This
/// keeps the partial-commit and rollback-failure paths deterministically
/// testable on every platform — no permission tricks, and deliberately no
/// production switch.
#[doc(hidden)]
#[derive(Debug, Default, Clone, Copy)]
pub struct RestoreFault {
    pub fail_rename_at: Option<usize>,
    pub fail_rollback: bool,
}

/// Test-only entry: `restore` with injected faults (`--yes`, plain output).
#[doc(hidden)]
pub fn restore_injecting(root: &Path, save: &Path, id: &str, fault: RestoreFault) -> Result<()> {
    restore_inner(root, save, id, true, false, Some(fault), None)
}

/// Test-only entry: `restore` with a closure injected just before the final
/// integrity manifest re-baseline (`--yes`, plain output). The hook plays a
/// concurrent manifest writer committing at that point; production always
/// passes `None`. See the regression test
/// `restore_rebaseline_keeps_a_concurrent_writers_fingerprints` in
/// `tests/checkpoint.rs`.
#[doc(hidden)]
pub fn restore_injecting_manifest_hook(
    root: &Path,
    save: &Path,
    id: &str,
    manifest_hook: &dyn Fn(),
) -> Result<()> {
    restore_inner(root, save, id, true, false, None, Some(manifest_hook))
}

fn restore_inner(
    root: &Path,
    save: &Path,
    id: &str,
    yes: bool,
    json: bool,
    fault: Option<RestoreFault>,
    manifest_hook: Option<&dyn Fn()>,
) -> Result<()> {
    let fault = fault.unwrap_or_default();
    if !yes {
        bail!("恢复会替换存档里覆盖范围的文件；确认请加 --yes（会先把整份存档备份到 Saves/{BACKUP_DIR}/）");
    }
    let _lock = Lock::acquire(save)?;
    let dir = resolve(save, id)?;
    if let Some(problem) = &entry(save, id).problem {
        bail!("检查点 {id} 不可用（{problem}）；已拒绝恢复");
    }
    let cm = manifest(save, id)?;
    let current = migrate::detect(save)?;
    if cm.schema > current {
        bail!(
            "检查点 {id} 的 schema {} 比当前存档（schema {current}）新；请更新 talkdnd 后再恢复",
            cm.schema
        );
    }

    // 1. Whole-save backup before anything changes.
    let slot = save
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "save".to_string());
    let backup_name = format!("{slot}-{}-restore-{id}", stamp_compact());
    let backup = root.join("Saves").join(BACKUP_DIR).join(&backup_name);
    fs::create_dir_all(root.join("Saves").join(BACKUP_DIR))?;
    copy_tree(save, &backup)?;

    // 2. Stage every replacement next to its target; nothing is touched yet.
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut stage_err = None;
    for rel_path in cm.files.keys() {
        let dst = save.join(rel_path);
        if let Some(parent) = dst.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                stage_err = Some(anyhow::Error::new(err));
                break;
            }
        }
        let tmp = temp_sibling(&dst);
        match fs::copy(dir.join(rel_path), &tmp) {
            Ok(_) => staged.push((tmp, dst)),
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                stage_err = Some(anyhow::Error::new(err));
                break;
            }
        }
    }
    if let Some(err) = stage_err {
        // Remove the staged temp files and whatever staging created
        // (typically directories for covered paths that did not exist
        // pre-restore); failures are reported, never swallowed.
        let mut leftovers = remove_staged(root, &staged, 0);
        leftovers.extend(remove_extras(&backup, save, root));
        if leftovers.is_empty() {
            return Err(err.context(format!(
                "恢复已中止：当前存档未改动（恢复前整份备份在 {}）",
                rel(root, &backup)
            )));
        }
        return Err(err.context(format!(
            "恢复已中止：覆盖文件未改动，但 {} 个临时文件或目录残留：{}（恢复前整份备份在 {}）",
            leftovers.len(),
            leftovers.join("、"),
            rel(root, &backup)
        )));
    }

    // 3. Commit: rename each staged file into place (atomic replacement).
    let mut replaced = 0usize;
    let mut commit_err = None;
    let mut i = 0;
    while i < staged.len() {
        let (tmp, dst) = &staged[i];
        let attempt = match fault.fail_rename_at {
            Some(n) if n == i => Err(std::io::Error::other("injected commit failure")),
            _ => fs::rename(tmp, dst),
        };
        match attempt {
            Ok(()) => replaced += 1,
            Err(err) => {
                commit_err = Some(anyhow::Error::new(err));
                break;
            }
        }
        i += 1;
    }
    if let Some(err) = commit_err {
        // Remove the not-yet-committed temp files; failures are reported so
        // the message never claims a state it did not verify.
        let leftovers = remove_staged(root, &staged, i);
        // Rollback = mirror the backup over the save: copy it back, then
        // remove the entries the commit re-created (or staging made) that
        // the pre-restore backup does not contain. Only when the whole
        // tree matches again may the message claim a full rollback.
        let rollback = if fault.fail_rollback {
            Err(anyhow::anyhow!("injected rollback failure"))
        } else {
            copy_tree(&backup, save).and_then(|()| {
                let extras = remove_extras(&backup, save, root);
                if extras.is_empty() {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "无法删除备份之外的残留：{}",
                        extras.join("、")
                    ))
                }
            })
        };
        match rollback {
            Ok(()) => {
                let detail = if leftovers.is_empty() {
                    format!("已从 {} 回滚到恢复前状态", rel(root, &backup))
                } else {
                    // Temp files could not be removed, so we cannot claim
                    // the whole tree is back to its pre-restore state.
                    format!(
                        "覆盖文件已从 {} 回滚，但 {} 个临时文件残留：{}",
                        rel(root, &backup),
                        leftovers.len(),
                        leftovers.join("、")
                    )
                };
                return Err(anyhow::anyhow!(format!("恢复失败：{err}；{detail}")));
            }
            Err(rb) => {
                let mut msg = format!(
                    "恢复失败：{err}；回滚失败（{rb:#}），当前存档可能不完整。恢复前整份备份在 {}，请手动检查或用备份恢复",
                    rel(root, &backup)
                );
                if !leftovers.is_empty() {
                    msg += &format!(
                        "；另有 {} 个临时文件残留：{}",
                        leftovers.len(),
                        leftovers.join("、")
                    );
                }
                return Err(anyhow::anyhow!(msg));
            }
        }
    }

    // 4. Covered files that appeared after the snapshot are removed.
    let mut deleted: Vec<String> = Vec::new();
    let mut delete_warnings: Vec<String> = Vec::new();
    for (rel_path, path) in covered_files(save) {
        if cm.files.contains_key(&rel_path) {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => deleted.push(rel_path),
            Err(err) => delete_warnings.push(format!("{rel_path}：{err}")),
        }
    }

    // 5. Older checkpoint schema: restore first, then the normal migration.
    let mut migration: Option<String> = None;
    if cm.schema < current {
        let report = migrate::ensure(save, root, false)?;
        migration = Some(report.lines().join("；"));
    }

    // 6. Log the restore (append-only) and re-baseline integrity + audit.
    if let Some(log) = save::latest_log(save) {
        save::append(
            &log,
            &format!(
                "- [{}] 恢复检查点 {id}（备份 {}）",
                chrono::Local::now().format("%H:%M"),
                rel(root, &backup)
            ),
        )?;
    }
    if let Some(hook) = manifest_hook {
        // Test-only (restore_injecting_manifest_hook): fires at the start of
        // the manifest re-baseline — the interleaving point that the
        // integrity lock inside restore_rebaseline closes; production
        // always passes None.
        hook();
    }
    restore_rebaseline(save)?;

    if json {
        let v = serde_json::json!({
            "id": id,
            "backup": rel(root, &backup),
            "replaced": replaced,
            "deleted": deleted,
            "migrated": migration.is_some(),
        });
        println!("{}", serde_json::to_string(&v)?);
    } else {
        println!("恢复检查点 {id}（快照时 schema {}）", cm.schema);
        println!("备份当前存档 → {}", rel(root, &backup));
        println!("替换 {} 个覆盖范围文件", replaced);
        if !deleted.is_empty() {
            println!(
                "删除快照之后新增的 {} 个文件：{}",
                deleted.len(),
                deleted.join(", ")
            );
        }
        for w in &delete_warnings {
            println!("注意：{w} 删不掉（可手动删）");
        }
        if let Some(note) = &migration {
            println!("迁移：{note}");
        }
        println!(
            "已在会话日志记录恢复；integrity 清单与对账基准已更新，`talkdnd check` 应无未解释变化"
        );
    }
    Ok(())
}

/// Re-baseline the integrity manifest after a restore: fingerprints are set
/// to the current save state and the reconciliation baseline to the current
/// values, so `talkdnd check` is clean right after a restore. The whole
/// read-modify-write runs under the save's integrity manifest lock, and the
/// manifest is read inside the lock, so a concurrent tool process's
/// committed update is picked up by the in-lock load instead of being
/// overwritten by a manifest that was read before it committed
/// (issue #154). Lock order: the caller already holds the checkpoint lock
/// (`.talkdnd.lock`); the integrity lock is always taken after it, never
/// the reverse, and nothing inside (load, snapshot, `audit::current`,
/// store) acquires a lock, so the two never nest.
fn restore_rebaseline(save: &Path) -> Result<()> {
    let _lock = integrity::lock(save)?;
    let mut im = integrity::load(save)?.unwrap_or_else(|| integrity::baseline(save));
    im.files = integrity::snapshot(save);
    im.baseline = Some(serde_norway::to_value(audit::current(save))?);
    integrity::store(save, &im)
}

// ------------------------------------------------------------------ access

/// Checkpoint directory inside a save (no validation).
pub fn checkpoint_dir(save: &Path, id: &str) -> PathBuf {
    save.join(DIR).join(id)
}

/// Validated checkpoint directory for `id` (refuses traversal, must exist).
pub fn resolve(save: &Path, id: &str) -> Result<PathBuf> {
    if bad_id(id) {
        bail!("检查点 id `{id}` 不合法");
    }
    let dir = checkpoint_dir(save, id);
    if !dir.is_dir() {
        bail!("检查点 {id} 不存在；`talkdnd checkpoint list` 看全部");
    }
    Ok(dir)
}

/// `manifest.yaml` of `id`; an error when it is missing or unparsable.
pub fn manifest(save: &Path, id: &str) -> Result<Manifest> {
    let p = checkpoint_dir(save, id).join(MANIFEST);
    if !p.is_file() {
        bail!("检查点 {id} 不存在（没有 {MANIFEST}）");
    }
    let text = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let m: Manifest =
        serde_norway::from_str(&text).with_context(|| format!("invalid {}", p.display()))?;
    for rel in m.files.keys() {
        if unsafe_rel(rel) {
            bail!("检查点 {id} 的清单里有不安全的路径：{rel}");
        }
    }
    Ok(m)
}

/// A manifest path must stay inside the save (no traversal, no absolute).
fn unsafe_rel(rel: &str) -> bool {
    rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('\\')
        || rel.contains(':')
        || rel.split('/').any(|p| p == "..")
}

/// Every checkpoint, newest last; unusable ones are marked, never skipped.
pub fn entries(save: &Path) -> Vec<Entry> {
    checkpoint_ids(save)
        .into_iter()
        .map(|id| entry(save, &id))
        .collect()
}

/// Read-only checkpoint audit for `talkdnd check` (contract §3.4): one line
/// per checkpoint whose manifest is missing or damaged or whose copies fail
/// verification — the same `problem` that `list`/`show` display and
/// `restore` refuses on. Never fatal, never writes.
pub fn audit_checkpoints(save: &Path) -> Vec<String> {
    checkpoint_ids(save)
        .into_iter()
        .filter_map(|id| {
            let problem = entry(save, &id).problem?;
            Some(format!("检查点 {id} 不可用：{problem}"))
        })
        .collect()
}

/// The newest checkpoint (usable or not).
pub fn latest(save: &Path) -> Option<Entry> {
    checkpoint_ids(save).pop().map(|id| entry(save, &id))
}

/// The newest usable checkpoint (for `start`'s "较上次的变化").
pub fn latest_usable(save: &Path) -> Option<Entry> {
    checkpoint_ids(save)
        .into_iter()
        .rev()
        .map(|id| entry(save, &id))
        .find(|e| e.usable)
}

/// Manifest + verified copies for one checkpoint; never fails.
pub fn entry(save: &Path, id: &str) -> Entry {
    let dir = checkpoint_dir(save, id);
    let mut e = Entry {
        id: id.to_string(),
        ..Default::default()
    };
    let p = dir.join(MANIFEST);
    if !p.is_file() {
        e.problem = Some(format!("{MANIFEST} 缺失"));
        return e;
    }
    let Ok(text) = fs::read_to_string(&p) else {
        e.problem = Some(format!("{MANIFEST} 读不了"));
        return e;
    };
    let Ok(m) = serde_norway::from_str::<Manifest>(&text) else {
        e.problem = Some(format!("{MANIFEST} 解析失败（截断或损坏）"));
        return e;
    };
    e.created = m.created.clone();
    e.reason = m.reason.clone();
    e.schema = m.schema;
    e.log_file = m.log_file.clone();
    e.log_lines = m.log_lines;
    e.files = m.files.len();
    if m.version > VERSION {
        e.problem = Some(format!(
            "manifest version {} 不支持（本程序支持 {VERSION}）",
            m.version
        ));
        return e;
    }
    let mut bytes = 0u64;
    for (rel_path, want) in &m.files {
        if unsafe_rel(rel_path) {
            e.problem = Some(format!("清单里有不安全的路径：{rel_path}"));
            return e;
        }
        match fs::read(dir.join(rel_path)) {
            Ok(b) => {
                bytes += b.len() as u64;
                if hash(&b) != *want {
                    e.problem = Some(format!(
                        "文件 {rel_path} 的 sha256 与清单不符（副本被改动）"
                    ));
                    return e;
                }
            }
            Err(_) => {
                e.problem = Some(format!("文件 {rel_path} 缺失"));
                return e;
            }
        }
    }
    e.bytes = bytes;
    e.usable = true;
    e
}

fn checkpoint_ids(save: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(save.join(DIR)) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !bad_id(n))
        .collect();
    v.sort();
    v
}

fn bad_id(id: &str) -> bool {
    id.is_empty()
        || id == "."
        || id == ".."
        || id.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'])
}

// ------------------------------------------------------------ covered files

/// The checkpoint copy set: relative path -> absolute path (sorted).
fn covered_files(save: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for name in COVERED_FILES {
        let p = save.join(name);
        if p.is_file() {
            out.push((name.to_string(), p));
        }
    }
    for dir_name in COVERED_DIRS {
        collect(&save.join(dir_name), dir_name, &mut out);
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn collect(dir: &Path, rel_base: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        let rel_path = format!("{rel_base}/{name}");
        if p.is_dir() {
            collect(&p, &rel_path, out);
        } else if p.is_file() {
            out.push((rel_path, p));
        }
    }
}

fn covered_hashes(save: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for (rel_path, p) in covered_files(save) {
        let bytes = fs::read(&p).with_context(|| format!("read {}", p.display()))?;
        files.insert(rel_path, hash(&bytes));
    }
    Ok(files)
}

/// `sha256:<hex>` over the file bytes.
pub fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

// ------------------------------------------------------------------ helpers

fn new_id(save: &Path, reason: Option<&str>) -> String {
    let base = stamp_compact();
    let first = match reason.map(slug).filter(|s| !s.is_empty()) {
        Some(s) => format!("{base}-{s}"),
        None => base.clone(),
    };
    let mut id = first.clone();
    let mut n = 2;
    while checkpoint_dir(save, &id).exists() {
        id = format!("{first}-{n}");
        n += 1;
    }
    id
}

/// ASCII lowercase letters/digits/hyphens digest of a reason (max 24 chars).
fn slug(reason: &str) -> String {
    let mut s = String::new();
    for c in reason.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.is_empty() && !s.ends_with('-') {
            s.push('-');
        }
    }
    let cut: String = s.trim_matches('-').chars().take(24).collect();
    cut.trim_matches('-').to_string()
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

fn stamp_compact() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Remove staged temp files from index `from`; returns human-readable
/// leftovers (`path（error）`) so callers can report what could not be
/// cleaned instead of claiming the save is untouched.
fn remove_staged(root: &Path, staged: &[(PathBuf, PathBuf)], from: usize) -> Vec<String> {
    staged
        .iter()
        .skip(from)
        .filter_map(|(tmp, _)| {
            fs::remove_file(tmp)
                .err()
                .map(|e| format!("{}（{e}）", rel(root, tmp)))
        })
        .collect()
}

fn temp_sibling(dst: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut name = dst
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".tmp-{}-{nanos}", std::process::id()));
    dst.with_file_name(name)
}

/// Recursive copy (backups and rollback); skips the lock and the cache.
fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        if name == LOCK {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        let ft = entry.file_type()?;
        if ft.is_dir() {
            if name == ".cache" {
                continue;
            }
            copy_tree(&from, &to)?;
        } else if ft.is_file() {
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// The removing half of a mirror rollback: delete every entry under `dest`
/// that has no counterpart under `src`, so the tree matches `src` exactly
/// after `copy_tree(src, dest)`. The lock file and `.cache` directories are
/// runtime data deliberately outside the backup scope (contract §4.1) and
/// are never touched. Returns human-readable leftovers
/// (`path（error）`) for entries that could not be removed — the caller
/// must report them instead of claiming a verified rollback.
fn remove_extras(src: &Path, dest: &Path, root: &Path) -> Vec<String> {
    fn walk(src: &Path, dest: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(dest) else {
            out.push(format!("{}（读不了）", rel(root, dest)));
            return;
        };
        let mut entries: Vec<std::fs::DirEntry> = rd.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name();
            if name == LOCK || name == ".cache" {
                continue;
            }
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                out.push(format!("{}（类型读不了）", rel(root, &path)));
                continue;
            };
            let src_path = src.join(&name);
            if !src_path.exists() {
                let res = if ft.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                if let Err(err) = res {
                    out.push(format!("{}（{err}）", rel(root, &path)));
                }
                continue;
            }
            if ft.is_dir() {
                walk(&src_path, &path, root, out);
            }
        }
    }
    let mut leftovers = Vec::new();
    walk(src, dest, root, &mut leftovers);
    leftovers
}

// --------------------------------------------------------------------- lock

/// Per-slot mutex for snapshot/restore; the file is removed on drop.
struct Lock {
    path: PathBuf,
}

impl Lock {
    fn acquire(save: &Path) -> Result<Lock> {
        let path = save.join(LOCK);
        let mut stolen = false;
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(Lock { path });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if stolen || !stale(&path) {
                        bail!(
                            "另一个 checkpoint 操作正在进行（锁 {}）；稍后重试",
                            path.display()
                        );
                    }
                    let _ = fs::remove_file(&path);
                    stolen = true;
                }
                Err(err) => {
                    return Err(anyhow::Error::new(err))
                        .with_context(|| format!("open lock {}", path.display()));
                }
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() >= LOCK_STALE_SECS)
}
