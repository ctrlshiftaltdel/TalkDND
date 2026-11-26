//! D-protocol orchestrator: `admit` → `backup` → `candidate` → `publish`
//! → `logs` → `complete`.
//!
//! [`run_to_verified_candidate`] still stops at a verified unpublished
//! candidate and does not publish. [`run_to_complete`] publishes registered
//! targets (intent, preimage recheck, temp + rename + sync, done), appends
//! logs, updates the integrity manifest under the ADV-P1-3 bounded inner
//! lock, and writes `complete` only after candidate comparison, logs, and
//! integrity are done with no pending intent.
//!
//! Admit reuses [`AdvancementLock::begin_operation`] (and its pre-state
//! snapshot). The D path uses [`AdvancementLock::check_writable`], never
//! [`AdvancementLock::check_ordinary_write`].
//!
//! The inner integrity lock is acquired only through
//! [`acquire_inner_integrity_lock`]. This does not make the whole commit
//! path bounded: the old `integrity::lock()` API remains unbounded. There
//! is still no player commit CLI and no schema 11 / sidecar. Explicit
//! recovery is a separate library API in [`crate::advancement_recovery`].

use crate::advancement_backup_metadata::Inventory;
pub use crate::advancement_identity::{
    prepare_first_enrollment, EnrollmentApproval, EnrollmentCandidate, EnrollmentRequest,
};
use crate::advancement_lock::{
    self, open, AdvancementLock, CompletionEvidence, Error as LockError, OpenOptions, Stage, State,
};
use crate::charsheet::CharFile;
use crate::migrate::CURRENT_SCHEMA;
use crate::progress::Progress;
use anyhow::{anyhow, bail, Context, Result};
use combat_sim::integrity::Manifest;
use combat_sim::party::PartyState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// Root-only control files that must not enter the user-data backup (OD-7).
const CONTROL_FILES: [&str; 4] = [
    ".talkdnd.lock",
    ".talkdnd.integrity.lock",
    advancement_lock::LOCK_FILE,
    advancement_lock::STATE_FILE,
];

/// Root-only cache / derived files (#292 / ADV-P1-1e). Nested same names are
/// user data.
const CACHE_FILES: [&str; 2] = ["digest.md", "secret-terms.cache.json"];
pub(crate) const BACKUP_MANIFEST: &str = "d-backup.manifest.json";

/// Injected failures for AC-5. Ordinary error paths are not power-loss tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    BackupCopy,
    BackupManifest,
    BackupSync,
    CandidateBuild,
    CandidateValidate,
    IntentWrite,
    PublishWrite,
    PublishSync,
    LogAppend,
    IntegrityUpdate,
}

/// Where the orchestrator stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptStage {
    Admit,
    Backup,
    Candidate,
    Publish,
    Logs,
    Complete,
}

/// Materials a human needs after a stop. This block does not implement
/// recovery or auto-resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryMaterials {
    pub interrupt_stage: InterruptStage,
    pub prestate: Option<PathBuf>,
    pub backup_dir: Option<PathBuf>,
    pub backup_verified: bool,
    pub candidate_dir: Option<PathBuf>,
    pub candidate_verified: bool,
    pub note: String,
}

/// Outcome of a run that stopped before publish. `published` is always false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub protocol_state: State,
    pub stage: Stage,
    pub candidate_verified: bool,
    pub published: bool,
    pub backup_dir: Option<PathBuf>,
    pub candidate_dir: Option<PathBuf>,
    pub recovery: RecoveryMaterials,
    pub error: Option<String>,
    pub published_paths: Vec<String>,
    pub pending_paths: Vec<String>,
    pub logs_recorded: bool,
    pub integrity_recorded: bool,
    /// After a rename, a non-cooperative write in the recheck→rename window
    /// cannot be ruled out. This is a report, not a claim that one was
    /// detected. Digests are in [`Report::observed_preimage`] and
    /// [`Report::observed_target`]; injected bytes are never included.
    pub overwrite_cannot_be_ruled_out: bool,
    pub observed_preimage: Vec<(String, String)>,
    pub observed_target: Vec<(String, String)>,
}

impl Report {
    pub fn is_verified_unpublished(&self) -> bool {
        self.candidate_verified && !self.published && self.error.is_none()
    }
}

/// A failed run still carries the report so tests can inspect interrupt
/// location, leftover materials, and that no targets were published.
#[derive(Debug)]
pub struct FailedRun {
    pub report: Report,
}

impl std::fmt::Display for FailedRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.report.error {
            Some(err) => write!(f, "{err}"),
            None => write!(f, "the D commit stopped before publish"),
        }
    }
}

impl std::error::Error for FailedRun {}

/// Parameters for one admit → backup → candidate attempt.
pub struct Request<'a> {
    pub root: &'a Path,
    pub save_dir: &'a Path,
    pub operation_id: &'a str,
    pub evidence_root: &'a Path,
    pub evidence_dir: &'a Path,
    pub backup_parent: &'a Path,
    pub candidate_parent: &'a Path,
    pub overlays: BTreeMap<String, Vec<u8>>,
    pub fault: Option<Fault>,
    /// Registered synthetic or future extension targets (POSIX relative
    /// paths). They follow the same intent / preimage / publish / done
    /// protocol. This block does not invent sidecar fields or paths.
    pub extra_targets: Vec<String>,
    /// Required by [`run_to_complete`]. A mismatch is reported only as
    /// "绑定变化，原因未知" with no category list. [`run_to_verified_candidate`]
    /// ignores this field.
    pub preview_id: Option<String>,
    /// When non-empty, included in backup/candidate directory names so a
    /// repeated submit with the same `operation_id` can use a new exclusive
    /// directory. Empty keeps the ADV-P1-2 names.
    pub instance_tag: &'a str,
    pub hooks: PublishHooks<'a>,
    pub inner_lock: crate::advancement_inner_lock::InnerLockOptions,
}

/// Hook after a specific target's intent or recheck.
pub type PublishTargetHook<'a> = dyn Fn(&str, &Path) + 'a;
/// Hook before any target is published.
pub type PublishStartHook<'a> = dyn Fn(&Path) + 'a;

/// Test and fault-injection hooks for publish. Production callers leave
/// every field `None`.
#[derive(Default)]
pub struct PublishHooks<'a> {
    /// After preview_id has been accepted, before the first intent.
    pub before_any_publish: Option<&'a PublishStartHook<'a>>,
    /// After `record_intent` succeeds for this POSIX relative path.
    pub after_intent: Option<&'a PublishTargetHook<'a>>,
    /// After the preimage recheck, immediately before temp write / append.
    /// T7b injects here; the product must not print injected bytes.
    pub after_recheck_before_publish: Option<&'a PublishTargetHook<'a>>,
}

/// Run the D state machine through a verified candidate and stop. Never
/// publishes, never auto-resumes an `active` operation.
pub fn run_to_verified_candidate(req: &Request<'_>) -> std::result::Result<Report, Box<FailedRun>> {
    run_inner(req).map(|(_, report)| report)
}

/// Bounded inner integrity-lock acquire for the new advancement path
/// (ADV-P1-3 / OD-9). [`run_to_complete`] uses this at the integrity
/// stage. [`run_to_verified_candidate`] does not call this function.
/// After this wiring, the whole commit path is still not promised bounded
/// (the old integrity `lock()` remains unbounded).
///
/// The acquire fails closed on timeout, writes no targets, and does not
/// change SAVE or advancement state. A retry must call this again; each
/// call recomputes a fresh budget from `opts`. This is not a bound on a
/// whole commit path.
pub fn acquire_inner_integrity_lock(
    save_dir: &Path,
    opts: &crate::advancement_inner_lock::InnerLockOptions,
) -> std::result::Result<
    crate::advancement_inner_lock::HeldInnerLock,
    crate::advancement_inner_lock::InnerLockError,
> {
    crate::advancement_inner_lock::acquire_integrity(save_dir, opts)
}

fn run_inner(req: &Request<'_>) -> std::result::Result<(AdvancementLock, Report), Box<FailedRun>> {
    let prestate = req
        .evidence_dir
        .join(format!("{}-prestate.json", req.operation_id));

    let mut lock = match open_writable(req) {
        Ok(lock) => lock,
        Err(err) => {
            return Err(fail(Failure {
                interrupt: InterruptStage::Admit,
                protocol_state: State::Ready,
                stage: Stage::Init,
                backup_dir: None,
                candidate_dir: None,
                backup_verified: false,
                candidate_verified: false,
                error: format!(
                    "admit 失败：{err}。未写 active，未写任何目标文件。恢复所需材料：若操作前快照已创建则见 {}。",
                    prestate.display()
                ),
                prestate: prestate_if_exists(&prestate),
            }));
        }
    };

    if let Err(err) = combat_sim::encounter_lifecycle::require_between_encounters(req.save_dir) {
        return Err(fail(Failure {
            interrupt: InterruptStage::Admit,
            protocol_state: lock.state().state,
            stage: lock.state().stage,
            backup_dir: None,
            candidate_dir: None,
            backup_verified: false,
            candidate_verified: false,
            error: format!("当前只允许预览：{err:#}。未写任何目标文件。"),
            prestate: None,
        }));
    }

    if let Err(err) = lock.begin_operation(req.operation_id, req.evidence_root, req.evidence_dir) {
        let state = lock.state().state;
        let stage = lock.state().stage;
        drop(lock);
        return Err(fail(Failure {
                interrupt: InterruptStage::Admit,
                protocol_state: state,
                stage,
                backup_dir: None,
                candidate_dir: None,
                backup_verified: false,
                candidate_verified: false,
                error: format!(
                "admit 失败：{}。未写 active（或 active 的持久性未知），未写任何目标文件。恢复所需材料：操作前快照（若已创建）{}。",
                err,
                prestate.display()
            ),
                prestate: prestate_if_exists(&prestate),
            }));
    }

    let backup = match build_backup(&mut lock, req) {
        Ok(path) => path,
        Err(err) => {
            return Err(stop_failed(
                &mut lock,
                InterruptStage::Backup,
                err,
                None,
                false,
                prestate_if_exists(&prestate),
            ));
        }
    };

    let candidate = match build_candidate(&mut lock, req, &backup) {
        Ok(path) => path,
        Err(err) => {
            return Err(stop_failed(
                &mut lock,
                InterruptStage::Candidate,
                err,
                Some(backup.clone()),
                true,
                prestate_if_exists(&prestate),
            ));
        }
    };

    let report = Report {
        protocol_state: lock.state().state,
        stage: lock.state().stage,
        candidate_verified: true,
        published: false,
        backup_dir: Some(backup.clone()),
        candidate_dir: Some(candidate.clone()),
        recovery: RecoveryMaterials {
            interrupt_stage: InterruptStage::Candidate,
            prestate: prestate_if_exists(&prestate),
            backup_dir: Some(backup),
            backup_verified: true,
            candidate_dir: Some(candidate),
            candidate_verified: true,
            note: "候选已验证、未发布。本块不发布任何目标文件，也不自动续跑。".to_string(),
        },
        error: None,
        published_paths: Vec::new(),
        pending_paths: Vec::new(),
        logs_recorded: false,
        integrity_recorded: false,
        overwrite_cannot_be_ruled_out: false,
        observed_preimage: Vec::new(),
        observed_target: Vec::new(),
    };
    Ok((lock, report))
}

fn open_writable(req: &Request<'_>) -> Result<AdvancementLock> {
    let mut opts = OpenOptions::new();
    opts.acquire = advancement_lock::AcquireOptions {
        timeout: advancement_lock::DEFAULT_ACQUIRE_TIMEOUT,
        retry: std::time::Duration::from_millis(20),
    };
    let lock = open(req.root, req.save_dir, &opts).map_err(lock_err)?;
    lock.check_writable().map_err(lock_err)?;
    Ok(lock)
}

fn lock_err(err: LockError) -> anyhow::Error {
    anyhow!("{err}")
}

fn prestate_if_exists(path: &Path) -> Option<PathBuf> {
    path.exists().then(|| path.to_path_buf())
}

struct Failure {
    interrupt: InterruptStage,
    protocol_state: State,
    stage: Stage,
    backup_dir: Option<PathBuf>,
    candidate_dir: Option<PathBuf>,
    backup_verified: bool,
    candidate_verified: bool,
    error: String,
    prestate: Option<PathBuf>,
}

fn fail(failure: Failure) -> Box<FailedRun> {
    Box::new(FailedRun {
        report: Report {
            protocol_state: failure.protocol_state,
            stage: failure.stage,
            candidate_verified: failure.candidate_verified,
            published: false,
            backup_dir: failure.backup_dir.clone(),
            candidate_dir: failure.candidate_dir.clone(),
            recovery: RecoveryMaterials {
                interrupt_stage: failure.interrupt,
                prestate: failure.prestate,
                backup_dir: failure.backup_dir,
                backup_verified: failure.backup_verified,
                candidate_dir: failure.candidate_dir,
                candidate_verified: failure.candidate_verified,
                note: failure.error.clone(),
            },
            error: Some(failure.error),
            published_paths: Vec::new(),
            pending_paths: Vec::new(),
            logs_recorded: false,
            integrity_recorded: false,
            overwrite_cannot_be_ruled_out: false,
            observed_preimage: Vec::new(),
            observed_target: Vec::new(),
        },
    })
}

fn format_error(err: &anyhow::Error) -> String {
    let mut parts = Vec::new();
    for (i, cause) in err.chain().enumerate() {
        let piece = cause.to_string();
        if i == 0 || !parts.iter().any(|p: &String| p.contains(&piece)) {
            parts.push(piece);
        }
    }
    parts.join("：")
}

fn stop_failed(
    lock: &mut AdvancementLock,
    interrupt: InterruptStage,
    err: anyhow::Error,
    backup_dir: Option<PathBuf>,
    backup_verified: bool,
    prestate: Option<PathBuf>,
) -> Box<FailedRun> {
    let candidate_dir = None;
    let reason = format!(
        "{} 阶段停止：{}。未发布任何目标文件。恢复所需材料：操作前快照{}{}{}。本块不自动续跑或重放。",
        match interrupt {
            InterruptStage::Admit => "admit",
            InterruptStage::Backup => "backup",
            InterruptStage::Candidate => "candidate",
            InterruptStage::Publish => "publish",
            InterruptStage::Logs => "logs",
            InterruptStage::Complete => "complete",
        },
        format_error(&err),
        prestate
            .as_ref()
            .map(|p| format!(" {}", p.display()))
            .unwrap_or_else(|| "（无）".to_string()),
        backup_dir
            .as_ref()
            .map(|p| format!("；{}备份 {}", if backup_verified { "已验证" } else { "不完整" }, p.display()))
            .unwrap_or_else(|| "；无已验证备份".to_string()),
        "；候选未标记为已验证"
    );
    let finish = lock.finish_failed(&reason);
    let protocol_state = lock.state().state;
    let stage = lock.state().stage;
    let error = match finish {
        Ok(()) => reason,
        Err(finish_err) => format!(
            "{reason}（另外，写入 failed 状态失败：{finish_err}；当前协议状态可能仍为 active）"
        ),
    };
    fail(Failure {
        interrupt,
        protocol_state,
        stage,
        backup_dir,
        candidate_dir,
        backup_verified,
        candidate_verified: false,
        error,
        prestate,
    })
}

fn build_backup(lock: &mut AdvancementLock, req: &Request<'_>) -> Result<PathBuf> {
    require_existing_parent(req.backup_parent, "backup_parent")?;
    require_outside(req.save_dir, req.backup_parent, "备份目录")?;
    require_same_device(req.save_dir, req.backup_parent, "备份目录")?;
    let name = instance_dir_name(req, "d-backup");
    let dest = exclusive_create_dir(req.backup_parent, &name)
        .with_context(|| "独占创建用户数据备份目录")?;
    let metadata = Inventory::capture(req.save_dir)
        .with_context(|| format!("检查原始元数据；不完整备份保留在 {}", dest.display()))?;

    if req.fault == Some(Fault::BackupCopy) {
        // Start the copy so an incomplete backup exists, then stop.
        let _ = copy_user_data(req.save_dir, &dest);
        bail!(
            "注入失败：backup 复制中途停止；不完整备份保留在 {}。不得把目录存在当成完整备份",
            dest.display()
        );
    }

    copy_user_data(req.save_dir, &dest).with_context(|| {
        format!(
            "复制用户数据到 {} 失败；不完整备份已保留，不得静默跳过",
            dest.display()
        )
    })?;

    let entries = verify_copy(req.save_dir, &dest)?;
    metadata.verify_source(req.save_dir)?;

    if req.fault == Some(Fault::BackupSync) {
        bail!(
            "注入失败：backup 目录同步失败或结果不明；不完整备份保留在 {}，未标记为已验证",
            dest.display()
        );
    }
    sync_tree(&dest)?;

    if req.fault == Some(Fault::BackupManifest) {
        bail!(
            "注入失败：backup 清单写入/校验失败；不完整备份保留在 {}，未标记为已验证",
            dest.display()
        );
    }

    // Reserve the manifest before restoring the root directory's timestamps.
    // Writing data to this same entry later leaves those timestamps intact.
    let mut manifest = File::create_new(dest.join(BACKUP_MANIFEST))?;
    metadata.restore_and_verify_copy(&dest)?;
    sync_dir(req.backup_parent)?;
    write_backup_manifest(
        &mut manifest,
        req.operation_id,
        req.save_dir,
        entries,
        metadata,
    )?;
    lock.record_backup(
        dest.to_str()
            .ok_or_else(|| anyhow!("backup_dir {} 不能无损表示为 UTF-8", dest.display()))?,
    )
    .map_err(lock_err)?;
    Ok(dest)
}

fn build_candidate(
    lock: &mut AdvancementLock,
    req: &Request<'_>,
    backup: &Path,
) -> Result<PathBuf> {
    require_existing_parent(req.candidate_parent, "candidate_parent")?;
    require_outside(req.save_dir, req.candidate_parent, "候选目录")?;
    require_outside(backup, req.candidate_parent, "候选目录")?;
    require_same_device(req.save_dir, req.candidate_parent, "候选目录")?;
    require_same_device(backup, req.candidate_parent, "候选目录")?;
    let name = instance_dir_name(req, "d-candidate");
    let dest =
        exclusive_create_dir(req.candidate_parent, &name).with_context(|| "独占创建候选目录")?;

    if req.fault == Some(Fault::CandidateBuild) {
        let _ = copy_verified_preimage(backup, &dest);
        bail!(
            "注入失败：candidate 构建中途停止；不完整候选保留在 {}，不得标记为已验证",
            dest.display()
        );
    }

    copy_verified_preimage(backup, &dest)
        .with_context(|| format!("从已验证前像构建候选 {}", dest.display()))?;
    apply_overlays(&dest, &req.overlays)?;

    if req.fault == Some(Fault::CandidateValidate) {
        bail!(
            "注入失败：candidate 校验失败；候选保留在 {}，不得标记为已验证",
            dest.display()
        );
    }

    let manifest = validate_candidate(&dest, req.operation_id)?;
    sync_tree(&dest)?;
    write_candidate_manifest(&dest, req.operation_id, &manifest)?;
    sync_tree(&dest)?;
    lock.record_candidate_verified(
        dest.to_str()
            .ok_or_else(|| anyhow!("candidate_dir {} 不能无损表示为 UTF-8", dest.display()))?,
    )
    .map_err(lock_err)?;
    Ok(dest)
}

fn require_existing_parent(path: &Path, what: &str) -> Result<()> {
    let meta = fs::symlink_metadata(path).with_context(|| {
        format!(
            "{what} {} 必须由调用方提供且已存在，库不会自动创建",
            path.display()
        )
    })?;
    if meta.file_type().is_symlink() {
        bail!("{what} {} 是符号链接，必须是真实目录", path.display());
    }
    if !meta.is_dir() {
        bail!("{what} {} 不是目录", path.display());
    }
    Ok(())
}

fn require_outside(source: &Path, proposed: &Path, label: &str) -> Result<()> {
    let source = fs::canonicalize(source)?;
    let proposed = fs::canonicalize(proposed)?;
    if proposed.starts_with(source) {
        bail!("{label}必须位于受保护目录之外");
    }
    Ok(())
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("save")
        .to_string()
}

fn instance_dir_name(req: &Request<'_>, kind: &str) -> String {
    if req.instance_tag.is_empty() {
        format!("{}-{}-{kind}", dir_name(req.save_dir), req.operation_id)
    } else {
        format!(
            "{}-{}-{}-{kind}",
            dir_name(req.save_dir),
            req.operation_id,
            req.instance_tag
        )
    }
}

pub fn exclusive_create_dir(parent: &Path, name: &str) -> Result<PathBuf> {
    let dest = parent.join(name);
    fs::create_dir(&dest).with_context(|| {
        format!(
            "独占创建 {} 失败（目录已存在不构成完整备份/候选；必须使用新的独占目录）",
            dest.display()
        )
    })?;
    Ok(dest)
}

/// Device identity used to refuse a cross-device / cross-volume candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceKey(pub u64, pub u64);

pub fn device_key(path: &Path) -> Result<DeviceKey> {
    let meta = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    Ok(device_key_from_metadata(&meta))
}

fn device_key_from_metadata(meta: &fs::Metadata) -> DeviceKey {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        DeviceKey(meta.dev(), 0)
    }
    #[cfg(windows)]
    {
        // Stable std does not expose a volume serial here. Two paths on
        // different volumes cannot be proven to match, so the caller must
        // treat an inability to confirm as a rejection at a higher level
        // if a stronger probe is unavailable. Same-machine tests still
        // compare whatever we can (file index is unstable), so we use 0
        // and rely on `require_same_device` only when both keys are known.
        let _ = meta;
        DeviceKey(0, 0)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        DeviceKey(0, 0)
    }
}

/// Compare two already-computed device keys. Tests inject mismatched keys
/// rather than pretending a second volume exists in CI.
pub fn devices_match(a: DeviceKey, b: DeviceKey) -> bool {
    a == b
}

fn require_same_device(a: &Path, b: &Path, what: &str) -> Result<()> {
    let ka = device_key(a)?;
    let kb = device_key(b)?;
    if !devices_match(ka, kb) {
        bail!(
            "跨设备 / 跨卷拒绝：{what} {} 与 {} 不在同一文件系统",
            a.display(),
            b.display()
        );
    }
    Ok(())
}

pub fn is_root_excluded(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|n| {
        CONTROL_FILES.contains(&n) || CACHE_FILES.contains(&n) || n == ".cache" || n == ".talkdnd"
    })
}

pub fn posix_rel(root: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(root)
        .with_context(|| format!("{} 不在 {} 下", path.display(), root.display()))?;
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(name) => {
                let text = name.to_str().ok_or_else(|| {
                    anyhow!("{} 含有非 UTF-8 路径分量，无法无损记录", path.display())
                })?;
                if text.contains('\\') {
                    bail!("{} 的路径分量含有反斜杠，拒绝", path.display());
                }
                parts.push(text);
            }
            _ => bail!("{} 不是根下的普通相对路径", path.display()),
        }
    }
    Ok(parts.join("/"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEntry {
    pub path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

pub fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn copy_user_data(src: &Path, dest: &Path) -> Result<Vec<BackupEntry>> {
    require_outside(src, dest, "复制目标")?;
    require_outside(dest, src, "复制来源")?;
    let mut entries = Vec::new();
    copy_tree_at(src, dest, src, true, &mut entries)?;
    Ok(entries)
}

fn copy_tree_at(
    src: &Path,
    dest: &Path,
    src_root: &Path,
    at_root: bool,
    entries: &mut Vec<BackupEntry>,
) -> Result<()> {
    if !at_root {
        // dest already created by the caller for the exclusive root.
        if !dest.exists() {
            fs::create_dir(dest).with_context(|| format!("create {}", dest.display()))?;
        }
    }
    let mut dir_entries: Vec<_> = fs::read_dir(src)
        .with_context(|| format!("read {}", src.display()))?
        .collect();
    dir_entries.sort_by_key(|e| e.as_ref().map(|d| d.file_name()).unwrap_or_default());
    for entry in dir_entries {
        let entry = entry.with_context(|| format!("iterate {}", src.display()))?;
        let name = entry.file_name();
        if at_root && is_root_excluded(&name) {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        let ft = entry
            .file_type()
            .with_context(|| format!("type {}", from.display()))?;
        if ft.is_symlink() {
            bail!(
                "无法复制 {}：是符号链接；不得跟随链接逃出存档根，也不静默跳过",
                from.display()
            );
        }
        if ft.is_dir() {
            if from.canonicalize().ok().is_some_and(|canon| {
                !canon.starts_with(
                    src_root
                        .canonicalize()
                        .unwrap_or_else(|_| src_root.to_path_buf()),
                )
            }) {
                bail!("拒绝跟随 {} 逃出存档根", from.display());
            }
            fs::create_dir(&to).with_context(|| format!("create {}", to.display()))?;
            entries.push(BackupEntry {
                path: posix_rel(src_root, &from)?,
                kind: "dir".to_string(),
                size: None,
                digest: None,
            });
            copy_tree_at(&from, &to, src_root, false, entries)?;
        } else if ft.is_file() {
            let meta =
                fs::symlink_metadata(&from).with_context(|| format!("stat {}", from.display()))?;
            refuse_special_file(&from, &meta)?;
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
            let bytes = fs::read(&to).with_context(|| format!("read {}", to.display()))?;
            entries.push(BackupEntry {
                path: posix_rel(src_root, &from)?,
                kind: "file".to_string(),
                size: Some(bytes.len() as u64),
                digest: Some(digest_bytes(&bytes)),
            });
        } else {
            bail!(
                "无法复制 {}：不是普通文件或目录（符号链接和特殊文件无法纳入整份备份）",
                from.display()
            );
        }
    }
    Ok(())
}

fn refuse_special_file(path: &Path, meta: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() > 1 {
            bail!(
                "无法安全复制 {}：硬链接（nlink={}）无法在本协议下验证，停止并保留不完整备份",
                path.display(),
                meta.nlink()
            );
        }
    }
    let _ = (path, meta);
    Ok(())
}

/// Copy the verified backup into a candidate, skipping the backup manifest
/// (it is a backup record, not user data to publish).
fn copy_verified_preimage(backup: &Path, dest: &Path) -> Result<()> {
    copy_user_data(backup, dest)?;
    // The backup manifest lives in the backup dir; drop it from the candidate
    // if the copy included it (it is not a save file).
    let manifest = dest.join(BACKUP_MANIFEST);
    if manifest.exists() {
        fs::remove_file(&manifest).with_context(|| format!("strip {}", manifest.display()))?;
    }
    Ok(())
}

fn apply_overlays(dest: &Path, overlays: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    for (rel, bytes) in overlays {
        validate_overlay_path(rel)?;
        let path = dest.join(rel.split('/').collect::<PathBuf>());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        if path.exists() {
            let meta =
                fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
            if meta.file_type().is_symlink() || !meta.is_file() {
                bail!("候选覆盖 {} 不是普通文件，拒绝", rel);
            }
        }
        fs::write(&path, bytes).with_context(|| format!("write overlay {rel}"))?;
    }
    Ok(())
}

fn validate_overlay_path(path: &str) -> Result<()> {
    let bad = path.is_empty()
        || path.trim() != path
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if bad {
        bail!("候选覆盖路径 {path:?} 必须是相对于候选根的普通正斜杠路径");
    }
    Ok(())
}

fn verify_copy(src: &Path, dest: &Path) -> Result<Vec<BackupEntry>> {
    let mut expected = Vec::new();
    collect_user_entries(src, src, true, &mut expected)?;
    let mut found: BTreeMap<String, BackupEntry> = BTreeMap::new();
    collect_backup_entries(dest, dest, true, &mut found)?;
    if expected.len() != found.len() {
        bail!(
            "备份校验失败：源有 {} 项，副本有 {} 项；不得把目录存在当成完整备份",
            expected.len(),
            found.len()
        );
    }
    for entry in &expected {
        let some = found
            .get(&entry.path)
            .ok_or_else(|| anyhow!("备份缺项 {}；不得把目录存在当成完整备份", entry.path))?;
        if some != entry {
            bail!(
                "备份 {} 与源不一致（类型/大小/摘要）；不得标记为已验证",
                entry.path
            );
        }
    }
    Ok(expected)
}

fn collect_user_entries(
    dir: &Path,
    src_root: &Path,
    at_root: bool,
    out: &mut Vec<BackupEntry>,
) -> Result<()> {
    let mut dir_entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .collect();
    dir_entries.sort_by_key(|e| e.as_ref().map(|d| d.file_name()).unwrap_or_default());
    for entry in dir_entries {
        let entry = entry?;
        let name = entry.file_name();
        if at_root && is_root_excluded(&name) {
            continue;
        }
        let from = entry.path();
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            bail!("源中出现符号链接 {}，停止", from.display());
        }
        if ft.is_dir() {
            out.push(BackupEntry {
                path: posix_rel(src_root, &from)?,
                kind: "dir".to_string(),
                size: None,
                digest: None,
            });
            collect_user_entries(&from, src_root, false, out)?;
        } else if ft.is_file() {
            let meta = fs::symlink_metadata(&from)?;
            refuse_special_file(&from, &meta)?;
            let bytes = fs::read(&from)?;
            out.push(BackupEntry {
                path: posix_rel(src_root, &from)?,
                kind: "file".to_string(),
                size: Some(bytes.len() as u64),
                digest: Some(digest_bytes(&bytes)),
            });
        } else {
            bail!("源中出现特殊文件 {}，停止", from.display());
        }
    }
    Ok(())
}

fn collect_backup_entries(
    dir: &Path,
    dest_root: &Path,
    at_root: bool,
    out: &mut BTreeMap<String, BackupEntry>,
) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        if at_root && name == BACKUP_MANIFEST {
            continue;
        }
        let path = entry.path();
        let rel = posix_rel(dest_root, &path)?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            out.insert(
                rel.clone(),
                BackupEntry {
                    path: rel,
                    kind: "dir".to_string(),
                    size: None,
                    digest: None,
                },
            );
            collect_backup_entries(&path, dest_root, false, out)?;
        } else if ft.is_file() {
            refuse_special_file(&path, &fs::symlink_metadata(&path)?)?;
            let bytes = fs::read(&path)?;
            out.insert(
                rel.clone(),
                BackupEntry {
                    path: rel,
                    kind: "file".to_string(),
                    size: Some(bytes.len() as u64),
                    digest: Some(digest_bytes(&bytes)),
                },
            );
        } else {
            bail!("副本中出现特殊文件 {}，停止", path.display());
        }
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
struct BackupManifestFile {
    schema: u32,
    operation_id: String,
    source: String,
    verified: bool,
    entries: Vec<BackupEntry>,
    source_metadata: Option<Inventory>,
}

fn write_backup_manifest(
    file: &mut File,
    operation_id: &str,
    source: &Path,
    entries: Vec<BackupEntry>,
    source_metadata: Inventory,
) -> Result<()> {
    let doc = BackupManifestFile {
        schema: 2,
        operation_id: operation_id.to_string(),
        source: backup_source_name(source)?,
        verified: true,
        entries,
        source_metadata: Some(source_metadata),
    };
    serde_json::to_writer_pretty(&mut *file, &doc).context("写入备份清单")?;
    file.sync_all().context("同步备份清单")?;
    Ok(())
}

fn backup_source_name(source: &Path) -> Result<String> {
    Ok(source
        .to_str()
        .ok_or_else(|| anyhow!("save path is not UTF-8"))?
        .replace('\\', "/"))
}

pub(crate) fn verify_recovery_backup(backup: &Path, source: &Path, operation: &str) -> Result<()> {
    require_outside(source, backup, "恢复备份目录")?;
    require_outside(backup, source, "存档目录")?;
    if !fs::symlink_metadata(backup)?.is_dir() {
        bail!("恢复备份必须是普通目录");
    }
    let path = backup.join(BACKUP_MANIFEST);
    let manifest_metadata = fs::symlink_metadata(&path).context("检查恢复备份清单")?;
    if !manifest_metadata.is_file() {
        bail!("恢复备份清单必须是普通文件");
    }
    refuse_special_file(&path, &manifest_metadata)?;
    let manifest: BackupManifestFile =
        serde_json::from_reader(File::open(path)?).context("读取恢复备份清单")?;
    if !manifest.verified
        || manifest.operation_id != operation
        || manifest.source != backup_source_name(source)?
    {
        bail!("备份清单与当前存档或未完成操作不匹配");
    }

    let mut found = BTreeMap::new();
    collect_backup_entries(backup, backup, true, &mut found)?;
    for entry in manifest.entries {
        if found.remove(&entry.path).as_ref() != Some(&entry) {
            bail!("恢复备份条目与清单不一致：{}", entry.path);
        }
    }
    if !found.is_empty() {
        bail!("恢复备份存在清单未列出的条目");
    }
    match (manifest.schema, manifest.source_metadata) {
        (1, None) => Ok(()),
        (2, Some(metadata)) => metadata.verify_backup(backup),
        _ => bail!("恢复备份清单版本不受支持或元数据缺失"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CandidateManifestFile {
    schema: u32,
    operation_id: String,
    verified: bool,
    entries: Vec<BackupEntry>,
    identity_missing: Vec<String>,
}

fn write_candidate_manifest(
    dest: &Path,
    operation_id: &str,
    manifest: &CandidateManifestFile,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    write_synced(&dest.join("d-candidate.manifest.json"), &bytes)?;
    let _ = operation_id;
    Ok(())
}

fn validate_candidate(dest: &Path, operation_id: &str) -> Result<CandidateManifestFile> {
    let mut entries_map = BTreeMap::new();
    collect_backup_entries(dest, dest, true, &mut entries_map)?;
    // Ignore the candidate manifest if a previous attempt left one.
    entries_map.remove("d-candidate.manifest.json");
    let entries: Vec<BackupEntry> = entries_map.values().cloned().collect();

    let mut sheets: BTreeMap<String, CharFile> = BTreeMap::new();
    let mut missing: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    // Phase 1: schema-load every known file. Party/progress identity checks
    // run in phase 2 so they see the complete sheet set (directory order is
    // not a contract).
    for entry in &entries {
        if entry.kind != "file" {
            continue;
        }
        let path = dest.join(entry.path.split('/').collect::<PathBuf>());
        if entry.path == "save-meta.yaml" {
            validate_save_meta(&path, &mut errors)?;
            continue;
        }
        if let Some(id) = entry
            .path
            .strip_prefix("sheets/")
            .and_then(|s| s.strip_suffix(".yaml").or_else(|| s.strip_suffix(".yml")))
        {
            if id.contains('/') {
                continue;
            }
            match load_sheet(&path) {
                Ok(cf) => {
                    if cf.version > 1 {
                        errors.push(format!(
                            "sheets/{id}.yaml: unsupported character schema version {}",
                            cf.version
                        ));
                    }
                    if cf.id != id {
                        errors.push(format!(
                            "sheets/{id}.yaml: id {:?} 与文件名不符（只使用已有身份，不伪造）",
                            cf.id
                        ));
                    }
                    if !entries_map.contains_key(&cf.file.replace('\\', "/")) {
                        missing.push(format!("sheet `{id}` 引用的 {} 在候选中不存在", cf.file));
                    }
                    sheets.insert(id.to_string(), cf);
                }
                Err(err) => errors.push(format!("sheets/{id}.yaml: schema 校验失败：{err}")),
            }
        }
        if entry.path == "integrity.yaml" {
            if let Err(err) = load_integrity(&path) {
                errors.push(format!("integrity.yaml: schema 校验失败：{err}"));
            }
        }
    }

    for entry in &entries {
        if entry.kind != "file" {
            continue;
        }
        let path = dest.join(entry.path.split('/').collect::<PathBuf>());
        if entry.path == "combat/party-state.yaml" {
            match PartyState::load(&path) {
                Ok(party) => {
                    if party.version > 1 {
                        errors.push(format!(
                            "combat/party-state.yaml: unsupported schema version {}",
                            party.version
                        ));
                    }
                    for (member_id, member) in &party.members {
                        if !sheets.contains_key(member_id) {
                            missing.push(format!(
                                "party 成员 `{member_id}`（{}）没有对应的已有角色表，不伪造绑定",
                                member.name
                            ));
                        }
                    }
                }
                Err(err) => errors.push(format!("combat/party-state.yaml: schema 校验失败：{err}")),
            }
            continue;
        }
        if entry.path == "progress.yaml" {
            match load_progress(&path) {
                Ok(progress) => {
                    if progress.version > 1 {
                        errors.push(format!(
                            "progress.yaml: unsupported schema version {}",
                            progress.version
                        ));
                    }
                    let mut keys = BTreeSet::new();
                    keys.extend(progress.xp.keys().cloned());
                    keys.extend(progress.purses.keys().cloned());
                    for key in keys {
                        if key == crate::progress::PARTY {
                            continue;
                        }
                        if !sheets.contains_key(&key) {
                            missing.push(format!(
                                "progress 引用 `{key}` 但候选中没有对应角色表，不伪造绑定"
                            ));
                        }
                    }
                }
                Err(err) => errors.push(format!("progress.yaml: schema 校验失败：{err}")),
            }
        }
    }

    // Characters/*.md that no sheet points at are reported, not rebound.
    for entry in &entries {
        if entry.kind == "file" && entry.path.starts_with("Characters/") {
            let referenced = sheets
                .values()
                .any(|cf| cf.file.replace('\\', "/") == entry.path);
            if !referenced && !sheets.is_empty() {
                missing.push(format!(
                    "候选中的 {} 没有已被批准的角色表引用，不按文件名伪造绑定",
                    entry.path
                ));
            }
        }
    }

    if !missing.is_empty() || !errors.is_empty() {
        let mut parts = errors;
        parts.extend(missing.iter().cloned());
        bail!(
            "候选校验未通过（缺项已报告、未伪造身份）：{}",
            parts.join("；")
        );
    }

    Ok(CandidateManifestFile {
        schema: 1,
        operation_id: operation_id.to_string(),
        verified: true,
        entries,
        identity_missing: Vec::new(),
    })
}

fn load_sheet(path: &Path) -> Result<CharFile> {
    let text = fs::read_to_string(path)?;
    let cf: CharFile =
        serde_norway::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(cf)
}

fn load_progress(path: &Path) -> Result<Progress> {
    let text = fs::read_to_string(path)?;
    let progress: Progress =
        serde_norway::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(progress)
}

fn load_integrity(path: &Path) -> Result<Manifest> {
    let text = fs::read_to_string(path)?;
    let manifest: Manifest =
        serde_norway::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(manifest)
}

fn validate_save_meta(path: &Path, errors: &mut Vec<String>) -> Result<()> {
    let text = fs::read_to_string(path)?;
    let value: serde_norway::Value =
        serde_norway::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    let schema = value
        .as_mapping()
        .and_then(|m| m.get(serde_norway::Value::from("schema")))
        .and_then(|v| v.as_u64());
    match schema {
        Some(n) if n == u64::from(CURRENT_SCHEMA) => Ok(()),
        Some(n) => {
            errors.push(format!(
                "save-meta.yaml schema {n} 不被本块接受（CURRENT_SCHEMA 仍为 {CURRENT_SCHEMA}；不新增 schema 11 或 sidecar）"
            ));
            Ok(())
        }
        None => {
            errors.push("save-meta.yaml 缺少 schema 字段，不伪造".to_string());
            Ok(())
        }
    }
}

pub fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write {}", path.display()))?;
    file.flush()
        .with_context(|| format!("flush {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", path.display()))?;
    Ok(())
}

pub fn sync_tree(dir: &Path) -> Result<()> {
    fn walk(dir: &Path) -> Result<()> {
        for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type()?;
            if ft.is_dir() {
                walk(&path)?;
            } else if ft.is_file() {
                let file = File::open(&path).with_context(|| format!("open {}", path.display()))?;
                file.sync_all()
                    .with_context(|| format!("sync {}", path.display()))?;
            }
        }
        sync_dir(dir)?;
        Ok(())
    }
    walk(dir)?;
    if let Some(parent) = dir.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

pub fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let handle = File::open(dir).with_context(|| format!("open {}", dir.display()))?;
        handle
            .sync_all()
            .with_context(|| format!("sync {}", dir.display()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        bail!(
            "this platform cannot sync the directory {}；directory durability is unverified, so the backup/candidate is not marked verified",
            dir.display()
        );
    }
}

/// Byte snapshot of the protected target files this block must not publish.
/// Missing files are recorded as absent so a later comparison can prove they
/// were not created either.
pub fn protected_target_bytes(save: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    const FILES: &[&str] = &[
        "progress.yaml",
        "combat/party-state.yaml",
        "rolls.log",
        "integrity.yaml",
        "save-meta.yaml",
    ];
    for rel in FILES {
        let path = save.join(PathBuf::from(
            rel.replace('/', std::path::MAIN_SEPARATOR_STR),
        ));
        if path.is_file() {
            out.insert((*rel).to_string(), fs::read(&path)?);
        } else {
            out.insert((*rel).to_string(), Vec::new());
        }
    }
    for dir in ["sheets", "Characters", "Sessions"] {
        let root = save.join(dir);
        if !root.is_dir() {
            continue;
        }
        collect_all_files(&root, save, &mut out)?;
    }
    Ok(out)
}

pub fn collect_all_files(
    dir: &Path,
    root: &Path,
    out: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_all_files(&path, root, out)?;
        } else if ft.is_file() {
            out.insert(posix_rel(root, &path)?, fs::read(&path)?);
        }
    }
    Ok(())
}

/// Compare two protected-target snapshots. Empty value means the path was
/// absent at snapshot time.
pub fn targets_unchanged(
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
) -> bool {
    before == after
}

// ---------------------------------------------------------------- publish ----

const OP_MARKER_PREFIX: &str = "<!-- talkdnd-op:";
const OP_MARKER_SUFFIX: &str = " -->";

pub fn op_marker(operation_id: &str) -> String {
    format!("{OP_MARKER_PREFIX}{operation_id}{OP_MARKER_SUFFIX}")
}

/// Compute the v1 binding digest used as `preview_id` for this block.
/// This is not a full preview implementation (ADV-P1-7). A mismatch may
/// only be reported as "绑定变化，原因未知".
pub fn binding_preview_id(save: &Path, extra_targets: &[String]) -> Result<String> {
    let mut listing = protected_target_bytes(save)?;
    for rel in extra_targets {
        if listing.contains_key(rel) {
            continue;
        }
        let path = save.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if path.is_file() {
            listing.insert(rel.clone(), fs::read(&path)?);
        } else {
            listing.insert(rel.clone(), Vec::new());
        }
    }
    let mut acc = Vec::new();
    for (path, bytes) in listing {
        acc.extend_from_slice(path.as_bytes());
        acc.push(0);
        acc.extend_from_slice(digest_bytes(&bytes).as_bytes());
        acc.push(0);
    }
    Ok(digest_bytes(&acc))
}

/// Run admit → backup → candidate → publish → logs → integrity → complete.
///
/// Publish acquires the inner integrity lock only through
/// [`acquire_inner_integrity_lock`]. The whole commit path is still not
/// promised bounded. This function does not auto-resume an `active` or
/// `in_doubt` leftover, and it is not a player CLI.
pub fn run_to_complete(req: &Request<'_>) -> std::result::Result<Report, Box<FailedRun>> {
    // Reject before admit when the caller omitted or cannot compute a
    // binding id, so a missing preview does not leave an active operation.
    match req.preview_id.as_deref() {
        None => {
            return Err(reject_before_admit(
                "未提供 preview_id，拒绝提交。未写任何目标文件。",
            ));
        }
        Some(given) => match binding_preview_id(req.save_dir, &req.extra_targets) {
            Ok(expected) if given == expected => {}
            Ok(_) => {
                return Err(reject_before_admit(
                    "绑定变化，原因未知。未写任何目标文件。",
                ));
            }
            Err(err) => {
                return Err(reject_before_admit(&format!(
                    "无法计算绑定摘要：{err}。未写任何目标文件。"
                )));
            }
        },
    }

    let (lock, prepared) = run_inner(req)?;
    // Recheck after candidate build. A change here is still before any
    // target write; the report may only say "绑定变化，原因未知".
    publish_verified_candidate(req, lock, prepared)
}

fn reject_before_admit(message: &str) -> Box<FailedRun> {
    fail(Failure {
        interrupt: InterruptStage::Publish,
        protocol_state: State::Ready,
        stage: Stage::Init,
        backup_dir: None,
        candidate_dir: None,
        backup_verified: false,
        candidate_verified: false,
        error: message.to_string(),
        prestate: None,
    })
}

fn publish_verified_candidate(
    req: &Request<'_>,
    mut lock: AdvancementLock,
    prepared: Report,
) -> std::result::Result<Report, Box<FailedRun>> {
    let prestate = prepared.recovery.prestate.clone();
    let backup = prepared.backup_dir.clone();
    let candidate = match prepared.candidate_dir.clone() {
        Some(path) => path,
        None => {
            let err = "候选目录缺失，无法发布".to_string();
            return Err(stop_before_publish_held(&mut lock, req, &err, &prepared));
        }
    };

    let preview = req
        .preview_id
        .as_deref()
        .expect("run_to_complete already required preview_id");
    match binding_preview_id(req.save_dir, &req.extra_targets) {
        Ok(expected) if preview == expected => {}
        Ok(_) => {
            return Err(stop_before_publish_held(
                &mut lock,
                req,
                "绑定变化，原因未知。未写任何目标文件。",
                &prepared,
            ));
        }
        Err(err) => {
            return Err(stop_before_publish_held(
                &mut lock,
                req,
                &format!("无法计算绑定摘要：{err}。未写任何目标文件。"),
                &prepared,
            ));
        }
    }

    if lock.state().state != State::Active || lock.state().candidate_dir.is_none() {
        let err = format!(
            "发布拒绝：当前状态不是已验证候选（state={:?} stage={:?}）。不自动续跑。",
            lock.state().state,
            lock.state().stage
        );
        return Err(stop_before_publish_held(&mut lock, req, &err, &prepared));
    }

    if let Some(hook) = req.hooks.before_any_publish {
        hook(req.save_dir);
    }
    if let Err(err) = combat_sim::encounter_lifecycle::require_between_encounters(req.save_dir) {
        return Err(stop_before_publish_held(
            &mut lock,
            req,
            &format!("当前只允许预览：{err:#}。未写任何目标文件。"),
            &prepared,
        ));
    }

    let mut ctx = PublishCtx {
        published: Vec::new(),
        pending: Vec::new(),
        observed_preimage: Vec::new(),
        observed_target: Vec::new(),
        overwrite_cannot_be_ruled_out: false,
        logs_recorded: false,
        integrity_recorded: false,
    };

    let snapshot_before = combat_sim::integrity::snapshot(req.save_dir);

    if let Err(err) = publish_registered_targets(req, &mut lock, &candidate, &mut ctx) {
        return Err(stop_after_publish_started(
            &mut lock,
            req,
            InterruptStage::Publish,
            err,
            &prepared,
            &ctx,
        ));
    }

    if let Err(err) = publish_logs(req, &mut lock, &mut ctx) {
        return Err(stop_after_publish_started(
            &mut lock,
            req,
            InterruptStage::Logs,
            err,
            &prepared,
            &ctx,
        ));
    }
    ctx.logs_recorded = true;

    if let Err(err) = publish_integrity(req, &mut lock, &snapshot_before, &mut ctx) {
        return Err(stop_after_publish_started(
            &mut lock,
            req,
            InterruptStage::Logs,
            err,
            &prepared,
            &ctx,
        ));
    }
    ctx.integrity_recorded = true;

    if let Err(err) = compare_candidate_and_complete(&mut lock, req, &candidate, &ctx) {
        return Err(stop_after_publish_started(
            &mut lock,
            req,
            InterruptStage::Complete,
            err,
            &prepared,
            &ctx,
        ));
    }

    Ok(Report {
        protocol_state: lock.state().state,
        stage: lock.state().stage,
        candidate_verified: true,
        published: true,
        backup_dir: backup,
        candidate_dir: Some(candidate.clone()),
        recovery: RecoveryMaterials {
            interrupt_stage: InterruptStage::Complete,
            prestate,
            backup_dir: prepared.backup_dir.clone(),
            backup_verified: true,
            candidate_dir: Some(candidate),
            candidate_verified: true,
            note: complete_note(&ctx),
        },
        error: None,
        published_paths: ctx.published,
        pending_paths: lock
            .state()
            .pending
            .iter()
            .map(|p| p.path.clone())
            .collect(),
        logs_recorded: ctx.logs_recorded,
        integrity_recorded: ctx.integrity_recorded,
        overwrite_cannot_be_ruled_out: ctx.overwrite_cannot_be_ruled_out,
        observed_preimage: ctx.observed_preimage,
        observed_target: ctx.observed_target,
    })
}

struct PublishCtx {
    published: Vec<String>,
    pending: Vec<String>,
    observed_preimage: Vec<(String, String)>,
    observed_target: Vec<(String, String)>,
    overwrite_cannot_be_ruled_out: bool,
    logs_recorded: bool,
    integrity_recorded: bool,
}

fn complete_note(ctx: &PublishCtx) -> String {
    if ctx.overwrite_cannot_be_ruled_out {
        format!(
            "发布完成，但无法排除复核与 rename 之间的覆盖。已观察前像：{:?}；目标：{:?}。不宣称检测成功。",
            ctx.observed_preimage, ctx.observed_target
        )
    } else {
        "发布、日志与完整性已完成。整条提交路径的总等待仍不承诺有界。".to_string()
    }
}

fn publish_registered_targets(
    req: &Request<'_>,
    lock: &mut AdvancementLock,
    candidate: &Path,
    ctx: &mut PublishCtx,
) -> Result<()> {
    for rel in registered_replace_targets(candidate, &req.extra_targets)? {
        let dest = candidate.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !dest.is_file() {
            bail!("候选缺少发布目标 {rel}，停止且不跳过");
        }
        let after = fs::read(&dest)?;
        publish_replace(req, lock, &rel, &after, ctx, TargetKind::Replace)?;
    }
    Ok(())
}

pub fn registered_replace_targets(candidate: &Path, extra: &[String]) -> Result<Vec<String>> {
    let mut targets = Vec::new();
    targets.extend(list_dir_files(candidate, "sheets", "yaml")?);
    if candidate.join("combat").join("party-state.yaml").is_file() {
        targets.push("combat/party-state.yaml".to_string());
    }
    targets.extend(list_dir_files(candidate, "Characters", "md")?);
    if candidate.join("progress.yaml").is_file() {
        targets.push("progress.yaml".to_string());
    }
    for rel in extra {
        if rel.contains('\\') || rel.is_empty() {
            bail!("扩展目标路径必须是 POSIX 相对路径：{rel}");
        }
        if !targets.iter().any(|t| t == rel) {
            targets.push(rel.clone());
        }
    }
    Ok(targets)
}

pub fn list_dir_files(root: &Path, dir: &str, ext: &str) -> Result<Vec<String>> {
    let folder = root.join(dir);
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&folder)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(text) = name.to_str() else {
            bail!("{} 含有非 UTF-8 文件名，拒绝", entry.path().display());
        };
        if text.ends_with(&format!(".{ext}")) && !text.starts_with('.') {
            names.push(format!("{dir}/{text}"));
        }
    }
    names.sort();
    Ok(names)
}

#[derive(Clone, Copy)]
enum TargetKind {
    Replace,
    Log,
    Integrity,
}

fn publish_replace(
    req: &Request<'_>,
    lock: &mut AdvancementLock,
    rel: &str,
    after: &[u8],
    ctx: &mut PublishCtx,
    kind: TargetKind,
) -> Result<()> {
    let path = req
        .save_dir
        .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    let before = if path.is_file() {
        fs::read(&path)?
    } else {
        Vec::new()
    };
    let before_d = digest_bytes(&before);
    let after_d = digest_bytes(after);
    ctx.observed_preimage
        .push((rel.to_string(), before_d.clone()));
    ctx.observed_target.push((rel.to_string(), after_d.clone()));

    if req.fault == Some(Fault::IntentWrite) {
        bail!("注入失败：intent 写入失败，未发布 {rel}");
    }
    lock.record_intent(rel, &before_d, &after_d)
        .map_err(lock_err)?;
    ctx.pending.push(rel.to_string());

    if let Some(hook) = req.hooks.after_intent {
        hook(rel, req.save_dir);
    }

    let current = if path.is_file() {
        fs::read(&path)?
    } else {
        Vec::new()
    };
    if current != before {
        bail!(
            "发布复核不符：{rel} 的当前内容与前像不一致（已观察前像 {before_d}，目标 {after_d}）"
        );
    }

    if let Some(hook) = req.hooks.after_recheck_before_publish {
        hook(rel, req.save_dir);
    }

    match kind {
        TargetKind::Log => {
            if !after.starts_with(&before) {
                bail!("日志目标字节不是现有内容的后缀，拒绝改写既有行");
            }
            append_log_line(&path, &after[before.len()..], req.fault)?;
        }
        TargetKind::Replace | TargetKind::Integrity => {
            replace_file_atomically(&path, after, req.fault)?;
        }
    }
    if req.hooks.after_recheck_before_publish.is_some() {
        // A hook ran in the recheck→rename window. We can observe our
        // preimage and target digests, but we cannot rule out an overwrite.
        // This is a report, not a claim that one was detected.
        ctx.overwrite_cannot_be_ruled_out = true;
    }

    lock.record_published(rel).map_err(lock_err)?;
    ctx.pending.retain(|p| p != rel);
    ctx.published.push(rel.to_string());
    Ok(())
}

pub fn replace_file_atomically(path: &Path, bytes: &[u8], fault: Option<Fault>) -> Result<()> {
    if fault == Some(Fault::PublishWrite) {
        bail!(
            "注入失败：目标文件写入失败，未完成 {} 的发布",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp_name = format!(
        ".{}.adv-pub-tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("target")
    );
    let tmp = path
        .parent()
        .map(|p| p.join(&tmp_name))
        .unwrap_or_else(|| PathBuf::from(&tmp_name));
    let write = (|| -> Result<()> {
        let mut file = File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("write {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", tmp.display()))?;
        Ok(())
    })();
    if let Err(err) = write {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if fault == Some(Fault::PublishSync) {
        let _ = fs::remove_file(&tmp);
        bail!(
            "注入失败：文件或父目录同步失败，停止发布 {}",
            path.display()
        );
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    let published = File::open(path).with_context(|| format!("open {}", path.display()))?;
    published
        .sync_all()
        .with_context(|| format!("sync {}", path.display()))?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

pub fn append_log_line(path: &Path, new_bytes: &[u8], fault: Option<Fault>) -> Result<()> {
    if fault == Some(Fault::LogAppend) {
        bail!("注入失败：日志追加失败 {}", path.display());
    }
    if fault == Some(Fault::PublishWrite) {
        bail!("注入失败：日志写入失败 {}", path.display());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(new_bytes)
        .with_context(|| format!("append {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", path.display()))?;
    if let Some(parent) = path.parent() {
        if fault == Some(Fault::PublishSync) {
            bail!("注入失败：日志父目录同步失败 {}", path.display());
        }
        sync_dir(parent)?;
    }
    Ok(())
}

fn publish_logs(req: &Request<'_>, lock: &mut AdvancementLock, ctx: &mut PublishCtx) -> Result<()> {
    let mut logs = vec!["rolls.log".to_string()];
    logs.extend(list_dir_files(req.save_dir, "Sessions", "md")?);

    let marker = op_marker(req.operation_id);
    let line = format!("{marker} advancement applied\n");

    for rel in logs {
        let path = req
            .save_dir
            .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let existing = if path.is_file() {
            fs::read(&path)?
        } else {
            Vec::new()
        };
        if !existing.is_empty() && !existing.ends_with(b"\n") {
            bail!(
                "日志尾部异常，无法证明 {rel} 是否已追加操作 {}；停止而不是重放。日志可能已追加、清单可能未更新",
                req.operation_id
            );
        }
        if log_contains_marker(&existing, &marker) {
            // Already applied for this operation id: record as done without
            // appending (do not rewrite existing lines).
            let digest = digest_bytes(&existing);
            lock.record_intent(&rel, &digest, &digest)
                .map_err(lock_err)?;
            lock.record_published(&rel).map_err(lock_err)?;
            ctx.published.push(rel);
            continue;
        }
        let mut after = existing;
        after.extend_from_slice(line.as_bytes());
        publish_replace(req, lock, &rel, &after, ctx, TargetKind::Log)?;
        // For logs we pass the full desired file bytes so intent after-digest
        // is correct, but TargetKind::Log appends only the new line (the
        // function writes `after` as if it were the new line...).
    }
    Ok(())
}

pub fn log_contains_marker(bytes: &[u8], marker: &str) -> bool {
    let text = String::from_utf8_lossy(bytes);
    text.lines().any(|line| line.contains(marker))
}

fn publish_integrity(
    req: &Request<'_>,
    lock: &mut AdvancementLock,
    snapshot_before: &BTreeMap<String, String>,
    ctx: &mut PublishCtx,
) -> Result<()> {
    if req.fault == Some(Fault::IntegrityUpdate) {
        bail!("注入失败：完整性更新失败");
    }
    let _held = acquire_inner_integrity_lock(req.save_dir, &req.inner_lock)
        .map_err(|err| anyhow!("{err}。发布已停止；整条提交路径仍不承诺有界"))?;
    let now = combat_sim::integrity::snapshot(req.save_dir);
    let planned = combat_sim::integrity::planned_after_write(req.save_dir, snapshot_before, &now)?;
    let after = combat_sim::integrity::render(&planned)?;
    publish_replace(
        req,
        lock,
        "integrity.yaml",
        &after,
        ctx,
        TargetKind::Integrity,
    )?;
    Ok(())
}

fn compare_candidate_and_complete(
    lock: &mut AdvancementLock,
    req: &Request<'_>,
    candidate: &Path,
    ctx: &PublishCtx,
) -> Result<()> {
    if !lock.state().pending.is_empty() {
        bail!("完成拒绝：仍有未决 intent，只能 in_doubt");
    }
    for rel in registered_replace_targets(candidate, &req.extra_targets)? {
        let live = read_or_empty(
            &req.save_dir
                .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR)),
        )?;
        let want = read_or_empty(&candidate.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR)))?;
        if live != want {
            bail!("完成比对失败：{rel} 与候选不一致，只能 in_doubt");
        }
        if !ctx.published.iter().any(|p| p == &rel) {
            bail!("完成比对失败：{rel} 未出现在 published[] 中，只能 in_doubt");
        }
    }
    if !ctx.logs_recorded || !ctx.integrity_recorded {
        bail!("完成拒绝：日志或完整性尚未完成，只能 in_doubt");
    }
    lock.finish_complete(&CompletionEvidence {
        candidate_matches: true,
        logs_recorded: true,
        integrity_recorded: true,
    })
    .map_err(lock_err)?;
    Ok(())
}

pub fn read_or_empty(path: &Path) -> Result<Vec<u8>> {
    if path.is_file() {
        Ok(fs::read(path)?)
    } else {
        Ok(Vec::new())
    }
}

fn stop_before_publish_held(
    lock: &mut AdvancementLock,
    _req: &Request<'_>,
    message: &str,
    prepared: &Report,
) -> Box<FailedRun> {
    let mut error = message.to_string();
    if lock.state().state == State::Active
        && lock.state().published.is_empty()
        && lock.state().pending.is_empty()
    {
        if let Err(err) = lock.finish_failed(&error) {
            error = format!("{error}（写入 failed 失败：{err}）");
        }
    }
    fail(Failure {
        interrupt: InterruptStage::Publish,
        protocol_state: lock.state().state,
        stage: lock.state().stage,
        backup_dir: prepared.backup_dir.clone(),
        candidate_dir: prepared.candidate_dir.clone(),
        backup_verified: true,
        candidate_verified: true,
        error,
        prestate: prepared.recovery.prestate.clone(),
    })
}

fn stop_after_publish_started(
    lock: &mut AdvancementLock,
    _req: &Request<'_>,
    interrupt: InterruptStage,
    err: anyhow::Error,
    prepared: &Report,
    ctx: &PublishCtx,
) -> Box<FailedRun> {
    let published_now = !lock.state().published.is_empty() || !lock.state().pending.is_empty();
    let reason = format!(
        "{} 阶段停止：{}。已发布：{:?}；未决 intent：{:?}。{}整条提交路径仍不承诺有界。本块不自动重试发布，也不自动续跑。",
        match interrupt {
            InterruptStage::Admit => "admit",
            InterruptStage::Backup => "backup",
            InterruptStage::Candidate => "candidate",
            InterruptStage::Publish => "publish",
            InterruptStage::Logs => "logs",
            InterruptStage::Complete => "complete",
        },
        format_error(&err),
        lock.state()
            .published
            .iter()
            .map(|p| p.path.as_str())
            .collect::<Vec<_>>(),
        lock.state()
            .pending
            .iter()
            .map(|p| p.path.as_str())
            .collect::<Vec<_>>(),
        if ctx.overwrite_cannot_be_ruled_out {
            "无法排除覆盖；已观察前像与目标已记录（不含实际注入值）。"
        } else {
            ""
        }
    );
    let finish = if published_now || !ctx.published.is_empty() || !ctx.pending.is_empty() {
        lock.finish_in_doubt(&reason)
    } else {
        lock.finish_failed(&reason)
    };
    let protocol_state = lock.state().state;
    let stage = lock.state().stage;
    let pending_paths: Vec<String> = lock
        .state()
        .pending
        .iter()
        .map(|p| p.path.clone())
        .collect();
    let published_paths: Vec<String> = lock
        .state()
        .published
        .iter()
        .map(|p| p.path.clone())
        .collect();
    let error = match finish {
        Ok(()) => reason,
        Err(finish_err) => format!("{reason}（另外，写入终态失败：{finish_err}）"),
    };
    let mut boxed = fail(Failure {
        interrupt,
        protocol_state,
        stage,
        backup_dir: prepared.backup_dir.clone(),
        candidate_dir: prepared.candidate_dir.clone(),
        backup_verified: true,
        candidate_verified: true,
        error,
        prestate: prepared.recovery.prestate.clone(),
    });
    boxed.report.published = !published_paths.is_empty();
    boxed.report.published_paths = published_paths;
    boxed.report.pending_paths = pending_paths;
    boxed.report.logs_recorded = ctx.logs_recorded;
    boxed.report.integrity_recorded = ctx.integrity_recorded;
    boxed.report.overwrite_cannot_be_ruled_out = ctx.overwrite_cannot_be_ruled_out;
    boxed.report.observed_preimage = ctx.observed_preimage.clone();
    boxed.report.observed_target = ctx.observed_target.clone();
    boxed
}
