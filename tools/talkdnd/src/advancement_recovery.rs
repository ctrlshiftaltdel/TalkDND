//! Explicit recovery for an unfinished D-protocol operation (ADV-P1-5).
//!
//! Before any target write, recovery establishes work evidence and a
//! protected snapshot of the current tree, then waits for an explicit human
//! choice (`回滚到前像` / `继续到目标`). It never auto-resumes or replays.
//! A choice stored in an existing work record is evidence, not authorization
//! to continue: re-entry without a freshly confirmed choice only inspects.
//!
//! This is a library API. There is no player recovery CLI, no schema 11,
//! and no sidecar path. Passing synthetic tests is not a safety proof for
//! real saves; #264 is still unsolved. Recoverability after evidence is
//! cleaned is not guaranteed.

use crate::advancement_backup_metadata::Inventory;
use crate::advancement_commit::acquire_inner_integrity_lock;
use crate::advancement_commit::{
    append_log_line, copy_user_data, digest_bytes, exclusive_create_dir, is_root_excluded,
    list_dir_files, log_contains_marker, op_marker, posix_rel, read_or_empty,
    registered_replace_targets, replace_file_atomically, replace_file_atomically_verified,
    sync_dir, sync_tree, write_synced, Fault,
};
use crate::advancement_inner_lock::InnerLockOptions;
use crate::advancement_lock::{
    self, open, state_path, AdvancementLock, CompletionEvidence, ErrorKind, OpenOptions, Pending,
    Published, Stage, State,
};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Work-record schema (not the save schema; CURRENT_SCHEMA stays 10).
const WORK_SCHEMA: u32 = 1;

/// Human choice required before any recovery target write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryChoice {
    /// 回滚到前像
    RollbackToPreimage,
    /// 继续到目标
    ContinueToTarget,
}

impl RecoveryChoice {
    pub fn as_label(self) -> &'static str {
        match self {
            RecoveryChoice::RollbackToPreimage => "回滚到前像",
            RecoveryChoice::ContinueToTarget => "继续到目标",
        }
    }
}

/// Synthetic faults for T9a. These are injected error paths, not power-loss
/// or real process-kill tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryFault {
    WorkCreate,
    WorkWrite,
    WorkSync,
    WorkReread,
    WorkDirSync,
    SnapshotCreate,
    SnapshotWrite,
    SnapshotSync,
    SnapshotReread,
    SnapshotDirSync,
}

/// Parameters for one explicit recovery attempt or inspection.
pub struct RecoveryRequest<'a> {
    pub root: &'a Path,
    pub save_dir: &'a Path,
    pub recovery_id: &'a str,
    pub evidence_root: &'a Path,
    pub evidence_dir: &'a Path,
    /// Explicit human choice. Required to establish new work evidence and
    /// to perform target writes. Inspection may leave this `None`.
    pub choice: Option<RecoveryChoice>,
    /// Fresh confirmation of the choice for this call. A stored work-record
    /// choice is not treated as confirmation. Target writes require both a
    /// choice and `confirm_choice == true`.
    pub confirm_choice: bool,
    /// Registered synthetic or future extension targets (POSIX relative
    /// paths). They are included in file-by-file comparison. This block
    /// does not invent sidecar fields or paths.
    pub extra_targets: Vec<String>,
    pub fault: Option<RecoveryFault>,
    pub hooks: RecoveryHooks<'a>,
    pub inner_lock: InnerLockOptions,
}

/// Hook after work evidence exists, or after a file intent/done is recorded.
pub type RecoveryHook<'a> = dyn Fn(&str, &Path) -> std::result::Result<(), String> + 'a;
pub type RecoveryEvidenceHook<'a> = dyn Fn(&Path) -> std::result::Result<(), String> + 'a;

/// Test and fault-injection hooks. Production callers leave every field `None`.
#[derive(Default)]
pub struct RecoveryHooks<'a> {
    /// After work record and protected snapshot are established, before any
    /// target write.
    pub after_evidence: Option<&'a RecoveryEvidenceHook<'a>>,
    /// After the work record has recorded intent for this POSIX path, before
    /// the file is written.
    pub after_intent: Option<&'a RecoveryHook<'a>>,
    /// After the file was written (or skipped as already applied) and done
    /// was recorded.
    pub after_done: Option<&'a RecoveryHook<'a>>,
}

/// Outcome of an inspect or recover call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    pub protocol_state: State,
    pub stage: Stage,
    pub original_operation_id: Option<String>,
    pub recovery_id: String,
    pub choice: Option<RecoveryChoice>,
    pub work_path: Option<PathBuf>,
    pub snapshot_dir: Option<PathBuf>,
    pub backup_dir: Option<PathBuf>,
    pub candidate_dir: Option<PathBuf>,
    /// True when work evidence (record + snapshot) exists and verified.
    pub evidence_established: bool,
    /// True when this call was inspection-only (no confirmed choice).
    pub inspect_only: bool,
    pub determined: bool,
    pub in_doubt: bool,
    pub written_paths: Vec<String>,
    pub already_done_paths: Vec<String>,
    pub pending_paths: Vec<String>,
    /// Registered-target progress reconstructed from the work record.
    pub file_progress: Vec<FileProgress>,
    /// Save files outside the registered recovery target set. Reported so
    /// they are not silently ignored; they are never overwritten.
    pub scope_limited_paths: Vec<String>,
    pub needs_human_merge: bool,
    pub error: Option<String>,
}

/// Per-file recovery progress taken from the work record and live tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileProgress {
    pub path: String,
    pub intent: bool,
    pub done: bool,
    pub class: FileClass,
}

/// How the live file compares to the preimage and the chosen/available target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClass {
    MatchesPreimage,
    MatchesTarget,
    MatchesBoth,
    LaterLogAppends,
    Neither,
    MissingEvidence,
}

/// A stopped recovery still carries the report so tests can inspect
/// evidence, progress, and that targets were not written when required.
#[derive(Debug)]
pub struct FailedRecovery {
    pub report: RecoveryReport,
}

impl std::fmt::Display for FailedRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.report.error {
            Some(err) => write!(f, "{err}"),
            None => write!(f, "explicit recovery stopped"),
        }
    }
}

impl std::error::Error for FailedRecovery {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkRecord {
    schema: u32,
    recovery_id: String,
    original_operation_id: String,
    original_state_bytes: String,
    original_revision: u64,
    original_checksum: String,
    original_state: State,
    original_stage: Stage,
    original_pending: Vec<Pending>,
    original_published: Vec<Published>,
    backup_dir: Option<String>,
    candidate_dir: Option<String>,
    choice: RecoveryChoice,
    created_at: String,
    snapshot_dir: String,
    files: Vec<WorkFile>,
    scope_limited: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkFile {
    path: String,
    intent: bool,
    done: bool,
    before_digest: Option<String>,
    after_digest: Option<String>,
}

/// Inspect leftover state and, if present, existing work evidence. Never
/// writes targets and never creates work evidence.
pub fn inspect_recovery(req: &RecoveryRequest<'_>) -> RecoveryReport {
    let mut inspect = RecoveryRequest {
        confirm_choice: false,
        choice: None,
        fault: None,
        hooks: RecoveryHooks::default(),
        extra_targets: req.extra_targets.clone(),
        root: req.root,
        save_dir: req.save_dir,
        recovery_id: req.recovery_id,
        evidence_root: req.evidence_root,
        evidence_dir: req.evidence_dir,
        inner_lock: req.inner_lock.clone(),
    };
    // Drop hooks so inspect cannot be used to sneak writes.
    inspect.hooks = RecoveryHooks::default();
    match recover(&inspect) {
        Ok(report) => report,
        Err(err) => err.report,
    }
}

/// Establish work evidence (when a confirmed choice is supplied and none
/// exists yet), protect the current tree, then — only with a freshly
/// confirmed human choice — recover file by file. Re-entry without
/// `confirm_choice` only determines the original operation and progress.
pub fn recover(
    req: &RecoveryRequest<'_>,
) -> std::result::Result<RecoveryReport, Box<FailedRecovery>> {
    if req.recovery_id.trim().is_empty() {
        return Err(fail_without_lock(
            req,
            "recovery_id 不能为空；未建立工作证据，未写任何目标",
        ));
    }

    let mut lock = match open_held(req) {
        Ok(lock) => lock,
        Err(err) => {
            return Err(fail_without_lock(
                req,
                &format!("打开恢复锁失败：{err}。未写任何目标。身份不明或锁不可用时只允许人工处理"),
            ));
        }
    };

    let leftover = lock.state().clone();
    let work_path = work_path(req);
    let snapshot_dir = snapshot_dir(req);

    let inspect_only = !req.confirm_choice || req.choice.is_none();

    // Unknown / unusable leftover identity is already handled by open().
    let can_determine_op = !leftover.operation_id.trim().is_empty()
        && matches!(leftover.state, State::Active | State::InDoubt)
        || work_path.is_file();

    let existing = if work_path.is_file() {
        match load_work(&work_path) {
            Ok(work) => Some(work),
            Err(err) => {
                let error = format!(
                    "工作记录无法判定（{err}）。保留现有证据，进入 in_doubt，只允许人工处理，不自动续跑或重放，未写任何目标"
                );
                let in_doubt_err = maybe_mark_in_doubt(&mut lock, &error);
                let error = match in_doubt_err {
                    Ok(()) => error,
                    Err(extra) => format!("{error}（另外，写入 in_doubt 失败：{extra}）"),
                };
                return Err(stop_held(
                    &lock,
                    req,
                    ReportDraft {
                        original_operation_id: nonempty(leftover.operation_id.clone()),
                        work_path: Some(work_path),
                        snapshot_dir: snapshot_dir.is_dir().then_some(snapshot_dir),
                        backup_dir: leftover.backup_dir.as_ref().map(PathBuf::from),
                        candidate_dir: leftover.candidate_dir.as_ref().map(PathBuf::from),
                        evidence_established: false,
                        inspect_only,
                        determined: false,
                        in_doubt: true,
                        needs_human_merge: true,
                        error: Some(error),
                        ..ReportDraft::default()
                    },
                ));
            }
        }
    } else {
        None
    };

    if existing.is_none() && !matches!(leftover.state, State::Active | State::InDoubt) {
        // Already resolved or never started: do not write targets.
        let error = format!(
            "没有未完成的成长操作可恢复（当前协议状态为 {:?}）。未写任何目标。证据被清理后的可恢复性不作保证",
            leftover.state
        );
        return Ok(report_held(
            &lock,
            req,
            ReportDraft {
                original_operation_id: nonempty(leftover.operation_id.clone()),
                backup_dir: leftover.backup_dir.as_ref().map(PathBuf::from),
                candidate_dir: leftover.candidate_dir.as_ref().map(PathBuf::from),
                inspect_only,
                determined: true,
                error: Some(error),
                ..ReportDraft::default()
            },
        ));
    }

    if existing.is_none() && leftover.operation_id.trim().is_empty() {
        let error =
            "无法判定原操作（state 中缺少 operation_id）。停止，只允许人工处理，未写任何目标"
                .to_string();
        let _ = maybe_mark_in_doubt(&mut lock, &error);
        return Err(stop_held(
            &lock,
            req,
            ReportDraft {
                inspect_only,
                determined: false,
                in_doubt: true,
                needs_human_merge: true,
                error: Some(error),
                backup_dir: leftover.backup_dir.as_ref().map(PathBuf::from),
                candidate_dir: leftover.candidate_dir.as_ref().map(PathBuf::from),
                ..ReportDraft::default()
            },
        ));
    }

    let work = if let Some(work) = existing {
        work
    } else if inspect_only {
        // No work evidence yet, and no confirmed choice: inspect leftover only.
        let (backup, candidate) = locations(&leftover);
        let plan = match recovery_plan(req, &leftover, backup.as_deref(), candidate.as_deref()) {
            Ok(plan) => plan,
            Err(err) => {
                let error = format!("{err}。未建立工作证据，未写任何目标");
                return Err(stop_held(
                    &lock,
                    req,
                    ReportDraft {
                        original_operation_id: nonempty(leftover.operation_id.clone()),
                        backup_dir: backup,
                        candidate_dir: candidate,
                        inspect_only: true,
                        determined: false,
                        in_doubt: true,
                        needs_human_merge: true,
                        error: Some(error),
                        ..ReportDraft::default()
                    },
                ));
            }
        };
        return Ok(report_held(
            &lock,
            req,
            ReportDraft {
                original_operation_id: nonempty(leftover.operation_id.clone()),
                backup_dir: backup,
                candidate_dir: candidate,
                inspect_only: true,
                determined: can_determine_op,
                scope_limited_paths: plan.scope_limited.clone(),
                file_progress: plan
                    .files
                    .iter()
                    .map(|f| FileProgress {
                        path: f.rel.clone(),
                        intent: false,
                        done: false,
                        class: f.class,
                    })
                    .collect(),
                error: if plan.conflicts.is_empty() {
                    Some(
                        "仅检查：尚未建立恢复工作证据，也没有经确认的人工选择，未写任何目标".into(),
                    )
                } else {
                    Some(format!(
                        "仅检查：发现需要人工合并的冲突（{}）。未写任何目标，未删除后续日志",
                        plan.conflicts.join("；")
                    ))
                },
                needs_human_merge: !plan.conflicts.is_empty(),
                ..ReportDraft::default()
            },
        ));
    } else {
        // Confirmed choice and no work evidence yet: establish evidence first.
        let choice = req
            .choice
            .expect("confirm_choice with a choice is required to create evidence");
        match establish_evidence(req, &lock, &leftover, choice) {
            Ok(work) => work,
            Err(err) => {
                let error = format!(
                    "恢复工作证据创建失败：{err}。停止，不做任何目标写入；原未完成 state、备份位置与 intent 保持"
                );
                return Err(stop_held(
                    &lock,
                    req,
                    ReportDraft {
                        original_operation_id: nonempty(leftover.operation_id.clone()),
                        backup_dir: leftover.backup_dir.as_ref().map(PathBuf::from),
                        candidate_dir: leftover.candidate_dir.as_ref().map(PathBuf::from),
                        work_path: work_path.is_file().then_some(work_path),
                        snapshot_dir: snapshot_dir.is_dir().then_some(snapshot_dir),
                        evidence_established: false,
                        inspect_only: false,
                        determined: true,
                        error: Some(error),
                        ..ReportDraft::default()
                    },
                ));
            }
        }
    };

    if let Some(hook) = req.hooks.after_evidence {
        if let Err(msg) = hook(req.save_dir) {
            return Err(stop_held(
                &lock,
                req,
                draft_from_work(req, &work, leftover.state, leftover.stage, true).with_error(
                    format!("工作证据已建立后中断：{msg}。未写任何目标，不自动续跑"),
                ),
            ));
        }
    }

    let (backup, candidate) = (
        work.backup_dir.as_ref().map(PathBuf::from),
        work.candidate_dir.as_ref().map(PathBuf::from),
    );
    let plan = match recovery_plan(req, &leftover, backup.as_deref(), candidate.as_deref()) {
        Ok(plan) => plan,
        Err(err) => {
            let error =
                format!("{err}。保留工作证据与受保护快照，停止，只允许人工处理，未写任何目标");
            let _ = maybe_mark_in_doubt(&mut lock, &error);
            return Err(stop_held(
                &lock,
                req,
                draft_from_work(req, &work, lock.state().state, lock.state().stage, true)
                    .with_error(error)
                    .in_doubt_merge(),
            ));
        }
    };

    if inspect_only {
        return Ok(report_held(
            &lock,
            req,
            draft_from_work(req, &work, leftover.state, leftover.stage, true)
                .with_plan(&plan, &work)
                .with_error(
                    "仅检查：已凭工作记录与受保护快照判定原操作与恢复进度。没有经确认的人工选择，不自动续跑或重放，未写任何目标"
                        .into(),
                ),
        ));
    }

    let choice = req
        .choice
        .expect("confirmed choice is required for target writes");
    if choice != work.choice && work.files.iter().any(|f| f.intent || f.done) {
        let error = format!(
            "本次确认的选择（{}）与已开始的恢复工作记录选择（{}）不一致。停止，保留两份证据并要求人工合并，未写任何目标",
            choice.as_label(),
            work.choice.as_label()
        );
        let _ = maybe_mark_in_doubt(&mut lock, &error);
        return Err(stop_held(
            &lock,
            req,
            draft_from_work(req, &work, lock.state().state, lock.state().stage, true)
                .with_plan(&plan, &work)
                .with_error(error)
                .in_doubt_merge(),
        ));
    }

    if !plan.conflicts.is_empty() {
        let error = format!(
            "当前内容既非前像也非目标，或日志已有后续追加，或无法判定：{}。保留工作证据与受保护快照，要求人工合并，不删除后续日志，未覆盖冲突文件",
            plan.conflicts.join("；")
        );
        let _ = maybe_mark_in_doubt(&mut lock, &error);
        return Err(stop_held(
            &lock,
            req,
            draft_from_work(req, &work, lock.state().state, lock.state().stage, true)
                .with_plan(&plan, &work)
                .with_error(error)
                .in_doubt_merge(),
        ));
    }

    let mut work = work;
    if choice != work.choice {
        work.choice = choice;
        if let Err(err) = save_work_atomic(req, &work) {
            let error = format!("更新工作记录中的人工选择失败：{err}。未写任何目标");
            return Err(stop_held(
                &lock,
                req,
                draft_from_work(req, &work, leftover.state, leftover.stage, true).with_error(error),
            ));
        }
    }

    match apply_recovery(req, &mut lock, &mut work, &plan, choice) {
        Ok(outcome) => Ok(report_held(
            &lock,
            req,
            draft_from_work(req, &work, lock.state().state, lock.state().stage, true)
                .with_plan(&plan, &work)
                .with_writes(outcome.written, outcome.already_done)
                .with_optional_error(outcome.note),
        )),
        Err(ApplyStop::Hook { message, written, already_done }) => {
            Err(stop_held(
                &lock,
                req,
                draft_from_work(req, &work, lock.state().state, lock.state().stage, true)
                    .with_plan(&plan, &work)
                    .with_writes(written, already_done)
                    .with_error(format!(
                        "恢复在文件进度点中断：{message}。凭工作记录与快照可判定原操作与进度，不自动续跑或重放"
                    )),
            ))
        }
        Err(ApplyStop::Failed { error, written, already_done, merge }) => {
            if merge {
                let _ = maybe_mark_in_doubt(&mut lock, &error);
            }
            Err(stop_held(
                &lock,
                req,
                draft_from_work(req, &work, lock.state().state, lock.state().stage, true)
                    .with_plan(&plan, &work)
                    .with_writes(written, already_done)
                    .with_error(error)
                    .maybe_merge(merge),
            ))
        }
    }
}

// ---------------------------------------------------------------- evidence ----

fn establish_evidence(
    req: &RecoveryRequest<'_>,
    _lock: &AdvancementLock,
    leftover: &advancement_lock::StateRecord,
    choice: RecoveryChoice,
) -> Result<WorkRecord> {
    advancement_lock::ensure_evidence_dir(req.evidence_root, req.evidence_dir)
        .map_err(|err| anyhow!("{err}"))?;

    let backup = leftover.backup_dir.clone();
    let candidate = leftover.candidate_dir.clone();
    let plan = recovery_plan(
        req,
        leftover,
        backup.as_deref().map(Path::new),
        candidate.as_deref().map(Path::new),
    )?;

    let raw = fs::read(state_path(req.save_dir)).with_context(|| {
        format!(
            "读取原未完成 state 原始字节 {}",
            state_path(req.save_dir).display()
        )
    })?;
    let state_bytes =
        String::from_utf8(raw).map_err(|e| anyhow!("原未完成 state 不是有效 UTF-8：{e}"))?;

    let snap = snapshot_dir(req);
    let work = WorkRecord {
        schema: WORK_SCHEMA,
        recovery_id: req.recovery_id.to_string(),
        original_operation_id: leftover.operation_id.clone(),
        original_state_bytes: state_bytes,
        original_revision: leftover.revision,
        original_checksum: leftover.checksum.clone(),
        original_state: leftover.state,
        original_stage: leftover.stage,
        original_pending: leftover.pending.clone(),
        original_published: leftover.published.clone(),
        backup_dir: backup,
        candidate_dir: candidate,
        choice,
        created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        snapshot_dir: snap.display().to_string(),
        files: plan
            .files
            .iter()
            .map(|f| WorkFile {
                path: f.rel.clone(),
                intent: false,
                done: false,
                before_digest: Some(digest_bytes(&f.current)),
                after_digest: None,
            })
            .collect(),
        scope_limited: plan.scope_limited.clone(),
    };

    create_work_record(req, &work)?;
    create_protected_snapshot(req, &snap)?;
    Ok(work)
}

fn create_work_record(req: &RecoveryRequest<'_>, work: &WorkRecord) -> Result<()> {
    let path = work_path(req);
    if req.fault == Some(RecoveryFault::WorkCreate) {
        bail!("注入失败：工作记录独占创建失败 {}", path.display());
    }
    let bytes = serde_norway::to_string(work).context("encode recovery work record")?;
    let written = (|| -> Result<()> {
        if req.fault == Some(RecoveryFault::WorkWrite) {
            bail!("注入失败：工作记录写入失败 {}", path.display());
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("独占创建 {}", path.display()))?;
        file.write_all(bytes.as_bytes())
            .with_context(|| format!("write {}", path.display()))?;
        file.flush()
            .with_context(|| format!("flush {}", path.display()))?;
        if req.fault == Some(RecoveryFault::WorkSync) {
            bail!("注入失败：工作记录文件 sync 失败 {}", path.display());
        }
        file.sync_all()
            .with_context(|| format!("sync {}", path.display()))?;
        Ok(())
    })();
    if let Err(err) = written {
        let _ = fs::remove_file(&path);
        return Err(err);
    }
    let readback = fs::read(&path).with_context(|| format!("重读 {}", path.display()))?;
    if req.fault == Some(RecoveryFault::WorkReread) || readback != bytes.as_bytes() {
        bail!(
            "工作记录重读摘要不符或注入重读失败 {}（已写文件不作为已成立证据）",
            path.display()
        );
    }
    if req.fault == Some(RecoveryFault::WorkDirSync) {
        bail!(
            "注入失败：工作记录目录同步失败 {}",
            req.evidence_dir.display()
        );
    }
    sync_dir(req.evidence_dir)?;
    Ok(())
}

fn create_protected_snapshot(req: &RecoveryRequest<'_>, snap: &Path) -> Result<()> {
    if req.fault == Some(RecoveryFault::SnapshotCreate) {
        bail!("注入失败：受保护快照独占创建失败 {}", snap.display());
    }
    let parent = snap.parent().ok_or_else(|| anyhow!("快照路径没有父目录"))?;
    let name = snap
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("快照目录名不是 UTF-8"))?;
    exclusive_create_dir(parent, name)
        .with_context(|| format!("独占创建受保护快照 {}", snap.display()))?;

    if req.fault == Some(RecoveryFault::SnapshotWrite) {
        // Leave the empty exclusive directory so it is an incomplete snapshot.
        bail!("注入失败：受保护快照写入失败 {}", snap.display());
    }
    copy_user_data(req.save_dir, snap)
        .with_context(|| format!("复制当前树到受保护快照 {}", snap.display()))?;
    let mut manifest = BTreeMap::new();
    collect_tree_digests(snap, snap, &mut manifest)?;
    let text = serde_norway::to_string(&manifest).context("encode snapshot manifest")?;
    write_synced(&snap.join("manifest.yaml"), text.as_bytes())?;

    if req.fault == Some(RecoveryFault::SnapshotSync) {
        bail!("注入失败：受保护快照文件 sync 失败 {}", snap.display());
    }
    // copy_user_data / write_synced already sync files they write; still walk
    // the snapshot so a missing sync is surfaced.
    sync_tree(snap)?;

    let mut readback = BTreeMap::new();
    collect_tree_digests(snap, snap, &mut readback)?;
    // manifest.yaml itself is part of the snapshot; exclude it from the
    // comparison against the just-written content map by comparing file
    // digests of the copied user data only.
    if req.fault == Some(RecoveryFault::SnapshotReread) {
        bail!("注入失败：受保护快照重读摘要不符 {}", snap.display());
    }
    for (rel, digest) in &manifest {
        if rel == "manifest.yaml" {
            continue;
        }
        match readback.get(rel) {
            Some(got) if got == digest => {}
            _ => bail!("受保护快照重读摘要不符：{rel}"),
        }
    }
    if req.fault == Some(RecoveryFault::SnapshotDirSync) {
        bail!("注入失败：受保护快照目录同步失败 {}", snap.display());
    }
    sync_dir(snap)?;
    if let Some(parent) = snap.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

fn collect_tree_digests(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_tree_digests(&path, root, out)?;
        } else if ft.is_file() {
            let rel = posix_rel(root, &path)?;
            out.insert(rel, digest_bytes(&fs::read(&path)?));
        }
    }
    Ok(())
}

fn save_work_atomic(req: &RecoveryRequest<'_>, work: &WorkRecord) -> Result<()> {
    let path = work_path(req);
    let bytes = serde_norway::to_string(work).context("encode recovery work record")?;
    let tmp = path.with_extension("yaml.adv-rec-tmp");
    let write = (|| -> Result<()> {
        let mut file = File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(bytes.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    fs::rename(&tmp, &path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    let published = File::open(&path)?;
    published.sync_all()?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

fn load_work(path: &Path) -> Result<WorkRecord> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let work: WorkRecord =
        serde_norway::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    if work.schema != WORK_SCHEMA {
        bail!(
            "工作记录 schema {} 不被接受（本块工作记录 schema 为 {WORK_SCHEMA}）",
            work.schema
        );
    }
    if work.recovery_id.trim().is_empty() || work.original_operation_id.trim().is_empty() {
        bail!("工作记录缺少 recovery_id 或 original_operation_id");
    }
    Ok(work)
}

fn work_path(req: &RecoveryRequest<'_>) -> PathBuf {
    req.evidence_dir
        .join(format!("{}-work.yaml", req.recovery_id))
}

fn snapshot_dir(req: &RecoveryRequest<'_>) -> PathBuf {
    req.evidence_dir
        .join(format!("{}-snapshot", req.recovery_id))
}

// ---------------------------------------------------------------- planning ----

struct PlannedFile {
    rel: String,
    kind: RecoverKind,
    current: Vec<u8>,
    preimage: Vec<u8>,
    target: Vec<u8>,
    class: FileClass,
    /// True only when rollback must delete this path instead of writing empty
    /// bytes. Presence comes from the verified backup, not from bytes: a
    /// missing preimage and an existing empty file are different things.
    remove_on_rollback: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecoverKind {
    Replace,
    Log,
    Integrity,
}

struct RecoveryPlan {
    files: Vec<PlannedFile>,
    scope_limited: Vec<String>,
    conflicts: Vec<String>,
    /// The verified ext4 inventory from the original backup, when that
    /// operation captured one. Recovery applies it through the same staged
    /// temp file + verify-before-rename path as the original publish.
    verified_metadata: Option<Inventory>,
}

fn recovery_plan(
    req: &RecoveryRequest<'_>,
    leftover: &advancement_lock::StateRecord,
    backup: Option<&Path>,
    candidate: Option<&Path>,
) -> Result<RecoveryPlan> {
    let backup = backup.ok_or_else(|| anyhow!("缺少原操作备份位置，无法取得前像，停止"))?;
    // An ext4-inventory-backed operation restores its verified metadata with
    // the bytes. An older backup has no such inventory and keeps the legacy
    // replace semantics exactly as before. The verified call decodes the
    // manifest once; its result is the inventory used below, so there is no
    // second, unverified read.
    let verified_metadata = crate::advancement_commit::verify_recovery_backup(
        backup,
        req.save_dir,
        &leftover.operation_id,
    )
    .map_err(|err| anyhow!("重新验收原操作备份 {}：{err:#}", backup.display()))?;

    // Registered replace targets are defined from the candidate if present,
    // otherwise from the backup (rollback still has to cover every registered
    // publish target, including synthetic extras).
    let source = candidate.filter(|p| p.is_dir()).unwrap_or(backup);
    let mut replace = registered_replace_targets(source, &req.extra_targets)?;
    // Also include registered extras that exist only on the live save / backup
    // so a missing candidate copy cannot silently drop them from comparison.
    for rel in &req.extra_targets {
        if rel.contains('\\') || rel.is_empty() {
            bail!("扩展目标路径必须是 POSIX 相对路径：{rel}");
        }
        if !replace.iter().any(|t| t == rel) {
            replace.push(rel.clone());
        }
    }

    let mut files = Vec::new();
    for rel in replace {
        files.push(plan_file(
            req.save_dir,
            backup,
            candidate,
            leftover,
            &rel,
            RecoverKind::Replace,
        )?);
    }
    for rel in registered_log_targets(req.save_dir, backup, candidate)? {
        files.push(plan_file(
            req.save_dir,
            backup,
            candidate,
            leftover,
            &rel,
            RecoverKind::Log,
        )?);
    }
    files.push(plan_file(
        req.save_dir,
        backup,
        candidate,
        leftover,
        "integrity.yaml",
        RecoverKind::Integrity,
    )?);

    let registered: Vec<String> = files.iter().map(|f| f.rel.clone()).collect();
    let scope_limited = unregistered_user_files(req.save_dir, &registered)?;

    let mut conflicts = Vec::new();
    for f in &files {
        match f.class {
            FileClass::Neither => {
                conflicts.push(format!("{} 的当前内容既非前像也非目标，不自动覆盖", f.rel))
            }
            FileClass::LaterLogAppends => conflicts.push(format!(
                "{} 已有后续日志追加，不删除后续日志，不自动覆盖",
                f.rel
            )),
            FileClass::MissingEvidence => {
                conflicts.push(format!("{} 缺少前像或目标证据，无法判定", f.rel))
            }
            FileClass::MatchesPreimage | FileClass::MatchesTarget | FileClass::MatchesBoth => {}
        }
    }
    if !scope_limited.is_empty() {
        // Report, do not silently ignore; not a conflict by itself.
    }
    Ok(RecoveryPlan {
        files,
        scope_limited,
        conflicts,
        verified_metadata,
    })
}

fn plan_file(
    save: &Path,
    backup: &Path,
    candidate: Option<&Path>,
    leftover: &advancement_lock::StateRecord,
    rel: &str,
    kind: RecoverKind,
) -> Result<PlannedFile> {
    let (current, current_present, current_regular) = read_rel_presence(save, rel)?;
    let preimage = read_rel(backup, rel)?;
    let preimage_present = path_exists(backup, rel)?;
    let target = match kind {
        RecoverKind::Log => log_target(&preimage, &leftover.operation_id, &current),
        RecoverKind::Integrity | RecoverKind::Replace => match candidate {
            Some(dir) => read_rel(dir, rel)?,
            None => Vec::new(),
        },
    };
    let target_present = match (kind, candidate) {
        (RecoverKind::Replace, Some(dir)) => path_is_regular_file(dir, rel)?,
        _ => false,
    };
    let mut class = classify(kind, &current, &preimage, &target, candidate.is_some());
    let sidecar = kind == RecoverKind::Replace && is_advancement_sidecar(rel);
    let remove_on_rollback = sidecar
        && !preimage_present
        && target_present
        && !target.is_empty()
        && current_present
        && current_regular
        && current == target
        && operation_created_target(leftover, rel, &preimage, &target);
    if sidecar && !preimage_present && current_present && !remove_on_rollback {
        // Bytes alone cannot tell "was absent" from "was empty": the verified
        // backup says this sidecar had no preimage, so an existing file is
        // unproven until every removal-proof gate above passes. Writing the
        // empty preimage over it would overwrite bytes the operation cannot
        // be shown to have created, so this is a conflict, not MatchesTarget.
        class = FileClass::Neither;
    }
    Ok(PlannedFile {
        rel: rel.to_string(),
        kind,
        current,
        preimage,
        target,
        class,
        remove_on_rollback,
    })
}

/// A strictly recognized advancement sidecar: `advancement/<id>.yaml`, one
/// normal path component, no nested directories or backslashes.
fn is_advancement_sidecar(rel: &str) -> bool {
    let Some(name) = rel.strip_prefix("advancement/") else {
        return false;
    };
    name.len() > ".yaml".len()
        && name.ends_with(".yaml")
        && !name.contains('/')
        && !name.contains('\\')
        && name != ".yaml"
}

/// Presence of the preimage path, whatever its type: absence must be a real
/// `NotFound`, never the result of treating a non-regular entry as missing.
fn path_exists(root: &Path, rel: &str) -> Result<bool> {
    let path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    match fs::symlink_metadata(&path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("stat {}", path.display())),
    }
}

fn path_is_regular_file(root: &Path, rel: &str) -> Result<bool> {
    let path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    match fs::symlink_metadata(&path) {
        Ok(meta) => Ok(meta.file_type().is_file()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("stat {}", path.display())),
    }
}

fn read_rel_presence(root: &Path, rel: &str) -> Result<(Vec<u8>, bool, bool)> {
    let path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => Ok((
            fs::read(&path).with_context(|| format!("read {}", path.display()))?,
            true,
            true,
        )),
        Ok(_) => Ok((Vec::new(), true, false)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok((Vec::new(), false, false)),
        Err(err) => Err(err).with_context(|| format!("stat {}", path.display())),
    }
}

/// The original operation's own state must prove it wrote this exact target
/// over this exact preimage before rollback may un-create the path.
fn operation_created_target(
    leftover: &advancement_lock::StateRecord,
    rel: &str,
    preimage: &[u8],
    target: &[u8],
) -> bool {
    let before = digest_bytes(preimage);
    let after = digest_bytes(target);
    if leftover.pending.iter().any(|pending| pending.path == rel) {
        return false;
    }
    leftover.published.iter().any(|entry| {
        entry.path == rel && entry.done && entry.before == before && entry.after == after
    })
}

fn log_target(preimage: &[u8], operation_id: &str, current: &[u8]) -> Vec<u8> {
    let marker = op_marker(operation_id);
    if log_contains_marker(preimage, &marker) {
        return preimage.to_vec();
    }
    // If the live file already has exactly preimage + the original marker
    // (the intended target), use that. Otherwise the intended target is
    // preimage plus one marker line.
    let mut target = preimage.to_vec();
    if !target.is_empty() && !target.ends_with(b"\n") {
        // Tail anomaly is classified later; do not invent a clean target.
        return current.to_vec();
    }
    target.extend_from_slice(format!("{marker} advancement applied\n").as_bytes());
    target
}

fn classify(
    kind: RecoverKind,
    current: &[u8],
    preimage: &[u8],
    target: &[u8],
    have_candidate: bool,
) -> FileClass {
    if kind == RecoverKind::Replace && !have_candidate && current != preimage {
        // Rollback can still proceed when current == preimage; otherwise we
        // cannot name a target and must not guess.
        if current == preimage {
            return FileClass::MatchesPreimage;
        }
        return FileClass::MissingEvidence;
    }
    if kind == RecoverKind::Integrity && !have_candidate && current != preimage {
        if current == preimage {
            return FileClass::MatchesPreimage;
        }
        return FileClass::MissingEvidence;
    }
    if current == preimage && current == target {
        return FileClass::MatchesBoth;
    }
    if current == preimage {
        return FileClass::MatchesPreimage;
    }
    if current == target {
        return FileClass::MatchesTarget;
    }
    if kind == RecoverKind::Log
        && (current.starts_with(preimage) || current.starts_with(target))
        && current.len() > preimage.len()
        && current.len() > target.len()
    {
        return FileClass::LaterLogAppends;
    }
    if kind == RecoverKind::Log && !current.is_empty() && !current.ends_with(b"\n") {
        // Same rule as publish: a trailing-log anomaly cannot prove whether
        // the operation was appended. Do not replay or truncate.
        return FileClass::LaterLogAppends;
    }
    FileClass::Neither
}

fn registered_log_targets(
    save: &Path,
    backup: &Path,
    candidate: Option<&Path>,
) -> Result<Vec<String>> {
    let mut logs = vec!["rolls.log".to_string()];
    let mut seen = logs.clone();
    for root in [Some(save), Some(backup), candidate] {
        let Some(root) = root else { continue };
        for rel in list_dir_files(root, "Sessions", "md")? {
            if !seen.iter().any(|t| t == &rel) {
                seen.push(rel.clone());
                logs.push(rel);
            }
        }
    }
    Ok(logs)
}

fn unregistered_user_files(save: &Path, registered: &[String]) -> Result<Vec<String>> {
    let mut all = BTreeMap::new();
    collect_user_paths(save, save, true, &mut all)?;
    let mut extra = Vec::new();
    for path in all.keys() {
        if !registered.iter().any(|r| r == path) {
            extra.push(path.clone());
        }
    }
    extra.sort();
    Ok(extra)
}

fn collect_user_paths(
    dir: &Path,
    root: &Path,
    at_root: bool,
    out: &mut BTreeMap<String, ()>,
) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if at_root && is_root_excluded(&name) {
            continue;
        }
        if at_root {
            if let Some(text) = name.to_str() {
                if text == "_backups" {
                    continue;
                }
            }
        }
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_user_paths(&path, root, false, out)?;
        } else if ft.is_file() {
            out.insert(posix_rel(root, &path)?, ());
        }
    }
    Ok(())
}

fn read_rel(root: &Path, rel: &str) -> Result<Vec<u8>> {
    read_or_empty(&root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

fn locations(leftover: &advancement_lock::StateRecord) -> (Option<PathBuf>, Option<PathBuf>) {
    (
        leftover.backup_dir.as_ref().map(PathBuf::from),
        leftover.candidate_dir.as_ref().map(PathBuf::from),
    )
}

// ---------------------------------------------------------------- apply ----

struct ApplyOutcome {
    written: Vec<String>,
    already_done: Vec<String>,
    note: Option<String>,
}

enum ApplyStop {
    Hook {
        message: String,
        written: Vec<String>,
        already_done: Vec<String>,
    },
    Failed {
        error: String,
        written: Vec<String>,
        already_done: Vec<String>,
        merge: bool,
    },
}

fn apply_recovery(
    req: &RecoveryRequest<'_>,
    lock: &mut AdvancementLock,
    work: &mut WorkRecord,
    plan: &RecoveryPlan,
    choice: RecoveryChoice,
) -> std::result::Result<ApplyOutcome, ApplyStop> {
    let mut written = Vec::new();
    let mut already_done = Vec::new();

    for file in &plan.files {
        if file.kind == RecoverKind::Integrity {
            // Integrity is handled after replace + log files so a continue
            // can recompute it from the recovered tree.
            continue;
        }
        match apply_one(
            req,
            work,
            file,
            choice,
            &mut written,
            &mut already_done,
            plan.verified_metadata.as_ref(),
        ) {
            Ok(()) => {}
            Err(err) => return Err(err),
        }
    }

    match apply_integrity(
        req,
        work,
        plan,
        choice,
        &mut written,
        &mut already_done,
        plan.verified_metadata.as_ref(),
    ) {
        Ok(()) => {}
        Err(err) => return Err(err),
    }

    let terminal = match finish_terminal(lock, plan, work, choice) {
        Ok(note) => note,
        Err(err) => {
            return Err(ApplyStop::Failed {
                error: format!("{err}。文件已按选择处理，但终态写入失败；请人工检查"),
                written,
                already_done,
                merge: true,
            });
        }
    };

    let note = if !plan.scope_limited.is_empty() {
        Some(format!(
            "{}。范围限制：以下存档文件不在已注册恢复目标内，未覆盖、未静默忽略：{}",
            terminal.unwrap_or_else(|| "恢复完成".into()),
            plan.scope_limited.join(", ")
        ))
    } else {
        terminal
    };
    Ok(ApplyOutcome {
        written,
        already_done,
        note,
    })
}

fn apply_one(
    req: &RecoveryRequest<'_>,
    work: &mut WorkRecord,
    file: &PlannedFile,
    choice: RecoveryChoice,
    written: &mut Vec<String>,
    already_done: &mut Vec<String>,
    verified_metadata: Option<&Inventory>,
) -> std::result::Result<(), ApplyStop> {
    if work_file(work, &file.rel).is_some_and(|f| f.done) {
        already_done.push(file.rel.clone());
        return Ok(());
    }

    if choice == RecoveryChoice::RollbackToPreimage && file.remove_on_rollback {
        return remove_created_sidecar(req, work, file, written, already_done);
    }

    let desired = match choice {
        RecoveryChoice::RollbackToPreimage => file.preimage.as_slice(),
        RecoveryChoice::ContinueToTarget => file.target.as_slice(),
    };
    let already_matches = file.current == desired;

    if already_matches {
        // Record done without rewriting (AC-5). Still go through intent/done
        // in the work record so re-entry can see progress.
        if let Err(err) = record_work_intent(req, work, file, desired) {
            return Err(ApplyStop::Failed {
                error: format!("{err}"),
                written: written.clone(),
                already_done: already_done.clone(),
                merge: false,
            });
        }
        if let Some(hook) = req.hooks.after_intent {
            if let Err(msg) = hook(&file.rel, req.save_dir) {
                return Err(ApplyStop::Hook {
                    message: msg,
                    written: written.clone(),
                    already_done: already_done.clone(),
                });
            }
        }
        if let Err(err) = record_work_done(req, work, &file.rel) {
            return Err(ApplyStop::Failed {
                error: format!("{err}"),
                written: written.clone(),
                already_done: already_done.clone(),
                merge: false,
            });
        }
        already_done.push(file.rel.clone());
        if let Some(hook) = req.hooks.after_done {
            if let Err(msg) = hook(&file.rel, req.save_dir) {
                return Err(ApplyStop::Hook {
                    message: msg,
                    written: written.clone(),
                    already_done: already_done.clone(),
                });
            }
        }
        return Ok(());
    }

    // Current must be the other side (preimage or target). Conflicts were
    // already rejected in the plan.
    if let Err(err) = record_work_intent(req, work, file, desired) {
        return Err(ApplyStop::Failed {
            error: format!("{err}"),
            written: written.clone(),
            already_done: already_done.clone(),
            merge: false,
        });
    }
    if let Some(hook) = req.hooks.after_intent {
        if let Err(msg) = hook(&file.rel, req.save_dir) {
            return Err(ApplyStop::Hook {
                message: msg,
                written: written.clone(),
                already_done: already_done.clone(),
            });
        }
    }

    let dest = req
        .save_dir
        .join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    let live = read_or_empty(&dest).unwrap_or_default();
    if live != file.current {
        return Err(ApplyStop::Failed {
            error: format!(
                "恢复复核不符：{} 在 intent 之后已变化，停止且不覆盖，要求人工合并",
                file.rel
            ),
            written: written.clone(),
            already_done: already_done.clone(),
            merge: true,
        });
    }

    let write_err = match file.kind {
        RecoverKind::Log
            if choice == RecoveryChoice::ContinueToTarget
                && live == file.preimage
                && desired.starts_with(&live) =>
        {
            append_log_line(&dest, &desired[live.len()..], None::<Fault>).err()
        }
        _ => match verified_metadata {
            Some(inventory) => {
                replace_file_atomically_verified(&dest, desired, None, inventory, &file.rel).err()
            }
            None => replace_file_atomically(&dest, desired, None).err(),
        },
    };
    if let Some(err) = write_err {
        return Err(ApplyStop::Failed {
            error: format!("恢复写入 {} 失败：{err}。停止，不自动重试", file.rel),
            written: written.clone(),
            already_done: already_done.clone(),
            merge: true,
        });
    }

    if let Err(err) = record_work_done(req, work, &file.rel) {
        return Err(ApplyStop::Failed {
            error: format!("{err}"),
            written: written.clone(),
            already_done: already_done.clone(),
            merge: true,
        });
    }
    written.push(file.rel.clone());
    if let Some(hook) = req.hooks.after_done {
        if let Err(msg) = hook(&file.rel, req.save_dir) {
            return Err(ApplyStop::Hook {
                message: msg,
                written: written.clone(),
                already_done: already_done.clone(),
            });
        }
    }
    Ok(())
}

/// Roll back a sidecar the interrupted operation created: the verified
/// backup has no such path, so restore absence (delete the file) rather than
/// writing an empty file. Only reached for a present regular file whose live
/// bytes still equal the operation's recorded target; every mismatch stops
/// without deleting anything.
fn remove_created_sidecar(
    req: &RecoveryRequest<'_>,
    work: &mut WorkRecord,
    file: &PlannedFile,
    written: &mut Vec<String>,
    already_done: &mut Vec<String>,
) -> std::result::Result<(), ApplyStop> {
    let dest = req
        .save_dir
        .join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    let fail = |error: String, merge: bool| ApplyStop::Failed {
        error,
        written: written.clone(),
        already_done: already_done.clone(),
        merge,
    };

    match fs::symlink_metadata(&dest) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // Absence is the desired rollback state; record progress again
            // (idempotent re-entry) without recreating the file.
            if let Err(err) = record_work_intent(req, work, file, &file.preimage) {
                return Err(fail(format!("{err}"), false));
            }
            if let Some(hook) = req.hooks.after_intent {
                if let Err(msg) = hook(&file.rel, req.save_dir) {
                    return Err(ApplyStop::Hook {
                        message: msg,
                        written: written.clone(),
                        already_done: already_done.clone(),
                    });
                }
            }
            if let Err(err) = record_work_done(req, work, &file.rel) {
                return Err(fail(format!("{err}"), false));
            }
            already_done.push(file.rel.clone());
            if let Some(hook) = req.hooks.after_done {
                if let Err(msg) = hook(&file.rel, req.save_dir) {
                    return Err(ApplyStop::Hook {
                        message: msg,
                        written: written.clone(),
                        already_done: already_done.clone(),
                    });
                }
            }
            return Ok(());
        }
        Err(err) => {
            return Err(fail(
                format!("恢复复核 {} 失败：{err}。停止且不删除", file.rel),
                true,
            ))
        }
    }

    // Persist the intent first, then let the after-intent hook run. The
    // type and byte checks are deliberately last: the hook (or anything
    // else) may replace the entry after the plan was made, and only a file
    // that still is the same regular target may be deleted.
    if let Err(err) = record_work_intent(req, work, file, &file.preimage) {
        return Err(fail(format!("{err}"), false));
    }
    if let Some(hook) = req.hooks.after_intent {
        if let Err(msg) = hook(&file.rel, req.save_dir) {
            return Err(ApplyStop::Hook {
                message: msg,
                written: written.clone(),
                already_done: already_done.clone(),
            });
        }
    }

    let meta = match fs::symlink_metadata(&dest) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // Someone else restored absence; that is the rollback result, so
            // record it without recreating the file.
            if let Err(err) = record_work_done(req, work, &file.rel) {
                return Err(fail(format!("{err}"), false));
            }
            already_done.push(file.rel.clone());
            if let Some(hook) = req.hooks.after_done {
                if let Err(msg) = hook(&file.rel, req.save_dir) {
                    return Err(ApplyStop::Hook {
                        message: msg,
                        written: written.clone(),
                        already_done: already_done.clone(),
                    });
                }
            }
            return Ok(());
        }
        Err(err) => {
            return Err(fail(
                format!("恢复复核 {} 失败：{err}。停止且不删除", file.rel),
                true,
            ))
        }
    };
    if !meta.file_type().is_file() {
        return Err(fail(
            format!(
                "恢复复核不符：{} 不再是普通文件，拒绝删除，要求人工合并",
                file.rel
            ),
            true,
        ));
    }
    let live = match fs::read(&dest) {
        Ok(bytes) => bytes,
        Err(err) => {
            return Err(fail(
                format!("恢复复核读取 {} 失败：{err}。停止且不删除", file.rel),
                true,
            ))
        }
    };
    if live != file.target {
        return Err(fail(
            format!(
                "恢复复核不符：{} 在意图之后已变化，停止且不删除，要求人工合并",
                file.rel
            ),
            true,
        ));
    }
    if let Err(err) = fs::remove_file(&dest) {
        return Err(fail(
            format!("删除 {} 失败：{err}。停止，不自动重试", file.rel),
            true,
        ));
    }
    // Only the sidecar itself is removed; no sibling or directory cleanup.
    if let Some(parent) = dest.parent() {
        if let Err(err) = sync_dir(parent) {
            return Err(fail(format!("同步 {} 所在目录失败：{err}", file.rel), true));
        }
    }
    if let Err(err) = record_work_done(req, work, &file.rel) {
        return Err(fail(format!("{err}"), true));
    }
    written.push(file.rel.clone());
    if let Some(hook) = req.hooks.after_done {
        if let Err(msg) = hook(&file.rel, req.save_dir) {
            return Err(ApplyStop::Hook {
                message: msg,
                written: written.clone(),
                already_done: already_done.clone(),
            });
        }
    }
    Ok(())
}

fn apply_integrity(
    req: &RecoveryRequest<'_>,
    work: &mut WorkRecord,
    plan: &RecoveryPlan,
    choice: RecoveryChoice,
    written: &mut Vec<String>,
    already_done: &mut Vec<String>,
    verified_metadata: Option<&Inventory>,
) -> std::result::Result<(), ApplyStop> {
    let Some(file) = plan.files.iter().find(|f| f.kind == RecoverKind::Integrity) else {
        return Ok(());
    };
    if work_file(work, &file.rel).is_some_and(|entry| entry.done) {
        already_done.push(file.rel.clone());
        return Ok(());
    }
    let stop = |error: String| ApplyStop::Failed {
        error,
        written: written.clone(),
        already_done: already_done.clone(),
        merge: true,
    };
    let _held = acquire_inner_integrity_lock(req.save_dir, &req.inner_lock)
        .map_err(|err| stop(format!("{err}。完整性更新停止")))?;
    let current = read_rel(req.save_dir, &file.rel).map_err(|err| stop(format!("{err}")))?;
    if current != file.current {
        return Err(stop(
            "integrity.yaml 在恢复计划生成后已变化，停止且不覆盖，要求人工合并".into(),
        ));
    }
    let mut file = PlannedFile {
        rel: file.rel.clone(),
        kind: file.kind,
        current,
        preimage: file.preimage.clone(),
        target: file.target.clone(),
        class: file.class,
        remove_on_rollback: file.remove_on_rollback,
    };
    if choice == RecoveryChoice::ContinueToTarget {
        file.target = compute_integrity_target(req, work).map_err(|err| stop(format!("{err}")))?;
        file.class = classify(
            RecoverKind::Integrity,
            &file.current,
            &file.preimage,
            &file.target,
            true,
        );
        if matches!(file.class, FileClass::Neither | FileClass::LaterLogAppends) {
            return Err(stop(
                "integrity.yaml 的当前内容既非前像也非继续目标，要求人工合并，未覆盖".into(),
            ));
        }
    }
    apply_one(
        req,
        work,
        &file,
        choice,
        written,
        already_done,
        verified_metadata,
    )
}

fn compute_integrity_target(req: &RecoveryRequest<'_>, work: &WorkRecord) -> Result<Vec<u8>> {
    let backup = work
        .backup_dir
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("继续到目标需要原备份以计算完整性前像"))?;
    // `before` is the original preimage tree (the D backup), same as
    // ADV-P1-4's snapshot taken before publish. `now` is the live tree
    // after replace and log files have been recovered. Dirty / accepted
    // marks on the live manifest are preserved.
    let snapshot_before = combat_sim::integrity::snapshot(&backup);
    let now = combat_sim::integrity::snapshot(req.save_dir);
    let planned = combat_sim::integrity::planned_after_write(req.save_dir, &snapshot_before, &now)?;
    combat_sim::integrity::render(&planned).map_err(|e| anyhow!("{e}"))
}

fn finish_terminal(
    lock: &mut AdvancementLock,
    plan: &RecoveryPlan,
    work: &WorkRecord,
    choice: RecoveryChoice,
) -> Result<Option<String>> {
    let all_done = work.files.iter().all(|f| f.done)
        || plan
            .files
            .iter()
            .all(|f| work_file(work, &f.rel).is_some_and(|w| w.done));
    if !all_done {
        return Ok(Some(
            "恢复尚未对全部注册目标记为 done；不写 complete，不自动续跑".into(),
        ));
    }
    match choice {
        RecoveryChoice::RollbackToPreimage => {
            lock.finish_recovered_rollback(&format!(
                "显式恢复 {}：已回滚到前像（原操作 {}）",
                work.recovery_id, work.original_operation_id
            ))
            .map_err(|e| anyhow!("{e}"))?;
            Ok(Some(
                "显式恢复完成：已回滚到前像。#264 未解决前不声称恢复安全".into(),
            ))
        }
        RecoveryChoice::ContinueToTarget => {
            lock.finish_recovered_complete(&CompletionEvidence {
                candidate_matches: true,
                logs_recorded: true,
                integrity_recorded: true,
            })
            .map_err(|e| anyhow!("{e}"))?;
            Ok(Some(
                "显式恢复完成：已继续到目标。#264 未解决前不声称恢复安全".into(),
            ))
        }
    }
}

fn record_work_intent(
    req: &RecoveryRequest<'_>,
    work: &mut WorkRecord,
    file: &PlannedFile,
    desired: &[u8],
) -> Result<()> {
    let entry = work_file_mut(work, &file.rel);
    entry.intent = true;
    entry.before_digest = Some(digest_bytes(&file.current));
    entry.after_digest = Some(digest_bytes(desired));
    save_work_atomic(req, work)
}

fn record_work_done(req: &RecoveryRequest<'_>, work: &mut WorkRecord, rel: &str) -> Result<()> {
    let entry = work_file_mut(work, rel);
    entry.done = true;
    save_work_atomic(req, work)
}

fn work_file<'a>(work: &'a WorkRecord, rel: &str) -> Option<&'a WorkFile> {
    work.files.iter().find(|f| f.path == rel)
}

fn work_file_mut<'a>(work: &'a mut WorkRecord, rel: &str) -> &'a mut WorkFile {
    if let Some(index) = work.files.iter().position(|f| f.path == rel) {
        return &mut work.files[index];
    }
    work.files.push(WorkFile {
        path: rel.to_string(),
        intent: false,
        done: false,
        before_digest: None,
        after_digest: None,
    });
    work.files.last_mut().expect("just pushed")
}

// ---------------------------------------------------------------- lock / report ----

fn open_held(req: &RecoveryRequest<'_>) -> Result<AdvancementLock> {
    let mut opts = OpenOptions::new();
    opts.acquire = crate::advancement_lock::AcquireOptions {
        timeout: std::time::Duration::from_secs(5),
        retry: std::time::Duration::from_millis(20),
    };
    open(req.root, req.save_dir, &opts).map_err(|err| {
        anyhow!(
            "{} ({})",
            err,
            match err.kind() {
                ErrorKind::IdentityMismatch => "身份不符",
                ErrorKind::PathReplaced => "路径被替换",
                ErrorKind::CorruptState => "state 损坏或身份不明",
                _ => err.kind().as_str(),
            }
        )
    })
}

fn maybe_mark_in_doubt(lock: &mut AdvancementLock, reason: &str) -> Result<()> {
    match lock.state().state {
        State::Active => lock.finish_in_doubt(reason).map_err(|e| anyhow!("{e}")),
        State::InDoubt => Ok(()),
        _ => Ok(()),
    }
}

#[derive(Default)]
struct ReportDraft {
    original_operation_id: Option<String>,
    work_path: Option<PathBuf>,
    snapshot_dir: Option<PathBuf>,
    backup_dir: Option<PathBuf>,
    candidate_dir: Option<PathBuf>,
    evidence_established: bool,
    inspect_only: bool,
    determined: bool,
    in_doubt: bool,
    written_paths: Vec<String>,
    already_done_paths: Vec<String>,
    pending_paths: Vec<String>,
    file_progress: Vec<FileProgress>,
    scope_limited_paths: Vec<String>,
    needs_human_merge: bool,
    error: Option<String>,
}

impl ReportDraft {
    fn with_error(mut self, error: String) -> Self {
        self.error = Some(error);
        self
    }
    fn with_optional_error(mut self, error: Option<String>) -> Self {
        self.error = error;
        self
    }
    fn in_doubt_merge(mut self) -> Self {
        self.in_doubt = true;
        self.needs_human_merge = true;
        self.determined = self.original_operation_id.is_some() || self.determined;
        self
    }
    fn maybe_merge(self, merge: bool) -> Self {
        if merge {
            self.in_doubt_merge()
        } else {
            self
        }
    }
    fn with_plan(mut self, plan: &RecoveryPlan, work: &WorkRecord) -> Self {
        self.scope_limited_paths = plan.scope_limited.clone();
        self.file_progress = plan
            .files
            .iter()
            .map(|f| {
                let rec = work_file(work, &f.rel);
                FileProgress {
                    path: f.rel.clone(),
                    intent: rec.is_some_and(|r| r.intent),
                    done: rec.is_some_and(|r| r.done),
                    class: f.class,
                }
            })
            .collect();
        self.pending_paths = self
            .file_progress
            .iter()
            .filter(|f| f.intent && !f.done)
            .map(|f| f.path.clone())
            .collect();
        self
    }
    fn with_writes(mut self, written: Vec<String>, already_done: Vec<String>) -> Self {
        self.written_paths = written;
        self.already_done_paths = already_done;
        self
    }
}

fn draft_from_work(
    req: &RecoveryRequest<'_>,
    work: &WorkRecord,
    _state: State,
    _stage: Stage,
    evidence: bool,
) -> ReportDraft {
    ReportDraft {
        original_operation_id: Some(work.original_operation_id.clone()),
        work_path: Some(work_path(req)),
        snapshot_dir: Some(PathBuf::from(&work.snapshot_dir)),
        backup_dir: work.backup_dir.as_ref().map(PathBuf::from),
        candidate_dir: work.candidate_dir.as_ref().map(PathBuf::from),
        evidence_established: evidence,
        determined: true,
        scope_limited_paths: work.scope_limited.clone(),
        ..ReportDraft::default()
    }
}

fn report_held(
    lock: &AdvancementLock,
    req: &RecoveryRequest<'_>,
    mut draft: ReportDraft,
) -> RecoveryReport {
    if draft.inspect_only {
        draft.inspect_only = true;
    } else {
        draft.inspect_only = !req.confirm_choice || req.choice.is_none();
    }
    RecoveryReport {
        protocol_state: lock.state().state,
        stage: lock.state().stage,
        original_operation_id: draft.original_operation_id,
        recovery_id: req.recovery_id.to_string(),
        choice: req.choice,
        work_path: draft.work_path,
        snapshot_dir: draft.snapshot_dir,
        backup_dir: draft.backup_dir,
        candidate_dir: draft.candidate_dir,
        evidence_established: draft.evidence_established,
        inspect_only: draft.inspect_only,
        determined: draft.determined,
        in_doubt: draft.in_doubt || lock.state().state == State::InDoubt,
        written_paths: draft.written_paths,
        already_done_paths: draft.already_done_paths,
        pending_paths: draft.pending_paths,
        file_progress: draft.file_progress,
        scope_limited_paths: draft.scope_limited_paths,
        needs_human_merge: draft.needs_human_merge,
        error: draft.error,
    }
}

fn stop_held(
    lock: &AdvancementLock,
    req: &RecoveryRequest<'_>,
    draft: ReportDraft,
) -> Box<FailedRecovery> {
    Box::new(FailedRecovery {
        report: report_held(lock, req, draft),
    })
}

fn fail_without_lock(req: &RecoveryRequest<'_>, error: &str) -> Box<FailedRecovery> {
    Box::new(FailedRecovery {
        report: RecoveryReport {
            protocol_state: State::Ready,
            stage: Stage::Init,
            original_operation_id: None,
            recovery_id: req.recovery_id.to_string(),
            choice: req.choice,
            work_path: None,
            snapshot_dir: None,
            backup_dir: None,
            candidate_dir: None,
            evidence_established: false,
            inspect_only: !req.confirm_choice || req.choice.is_none(),
            determined: false,
            in_doubt: true,
            written_paths: Vec::new(),
            already_done_paths: Vec::new(),
            pending_paths: Vec::new(),
            file_progress: Vec::new(),
            scope_limited_paths: Vec::new(),
            needs_human_merge: true,
            error: Some(error.to_string()),
        },
    })
}

fn nonempty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}
