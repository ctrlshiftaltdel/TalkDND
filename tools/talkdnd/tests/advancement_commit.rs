//! ADV-P1-2: admit → backup → candidate, stopping at a verified unpublished
//! candidate. Synthetic fixtures only; no real saves, books or secrets.
//!
//! Directory-sync-dependent runs are Unix-only: Windows cannot fsync a
//! directory, so `check_writable` / `begin_operation` fail closed (already
//! covered by the lock crate). That platform's directory durability is
//! NOT_RUN / UNKNOWN here, not a pass.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use talkdnd::advancement_commit::{
    copy_user_data, devices_match, protected_target_bytes, run_to_verified_candidate,
    targets_unchanged, DeviceKey, Fault, InterruptStage, Request,
};
use talkdnd::advancement_lock as adv;

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "talkdnd-p2-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Saves")).unwrap();
    dir
}

fn save_of(root: &Path) -> PathBuf {
    root.join("Saves").join("2030-01-02-01")
}

fn init_protocol(root: &Path, save: &Path) {
    fs::create_dir_all(save).unwrap();
    match adv::init(root, save, &adv::InitOptions::new()) {
        Ok(_) => {}
        Err(err) if err.kind() == adv::ErrorKind::StatePublishedNotDurable => {}
        Err(err) => panic!("init protocol: {err}"),
    }
}

fn write_synthetic_save(save: &Path) {
    fs::create_dir_all(save.join("sheets")).unwrap();
    fs::create_dir_all(save.join("Characters")).unwrap();
    fs::create_dir_all(save.join("combat")).unwrap();
    fs::create_dir_all(save.join("Sessions")).unwrap();
    fs::create_dir_all(save.join("checkpoints/cp-1/Characters")).unwrap();
    fs::create_dir_all(save.join("combat/nested")).unwrap();
    fs::create_dir_all(save.join(".cache")).unwrap();

    fs::write(
        save.join("sheets/aria.yaml"),
        "version: 1\nid: aria\nname: Aria\nfile: Characters/Aria.md\nruleset: \"2024\"\n",
    )
    .unwrap();
    fs::write(save.join("Characters/Aria.md"), "# Aria\n").unwrap();
    fs::write(
        save.join("combat/party-state.yaml"),
        "version: 1\nmembers:\n  aria:\n    name: Aria\n    hp: 12\n    max_hp: 12\n",
    )
    .unwrap();
    fs::write(
        save.join("progress.yaml"),
        "version: 1\nxp:\n  aria: 0\npurses:\n  aria:\n    cp: 0\n",
    )
    .unwrap();
    fs::write(save.join("Sessions/2030-01-02.md"), "- session start\n").unwrap();
    fs::write(save.join("rolls.log"), "1d20=7\n").unwrap();
    fs::write(save.join("integrity.yaml"), "version: 1\nfiles: {}\n").unwrap();
    fs::write(save.join("save-meta.yaml"), "schema: 10\ntool: talkdnd\n").unwrap();
    fs::write(
        save.join("checkpoints/cp-1/Characters/Aria.md"),
        "# Aria checkpoint\n",
    )
    .unwrap();
    fs::write(save.join("combat/nested/digest.md"), "nested digest\n").unwrap();
    fs::write(save.join(".cache/digest.md"), "cache\n").unwrap();
    fs::write(save.join("digest.md"), "root digest\n").unwrap();
    fs::write(save.join("secret-terms.cache.json"), "{\"terms\":[]}\n").unwrap();
}

fn parents(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let evidence_root = root.join("Saves").join("_backups").join(".operations");
    let evidence_dir = evidence_root.join("2030-01-02-01");
    let backup_parent = root.join("Saves").join("_backups");
    let candidate_parent = root.join("Saves").join("_backups");
    fs::create_dir_all(&evidence_root).unwrap();
    fs::create_dir_all(&backup_parent).unwrap();
    (evidence_root, evidence_dir, backup_parent, candidate_parent)
}

#[allow(clippy::too_many_arguments)]
fn request<'a>(
    root: &'a Path,
    save: &'a Path,
    evidence_root: &'a Path,
    evidence_dir: &'a Path,
    backup_parent: &'a Path,
    candidate_parent: &'a Path,
    overlays: BTreeMap<String, Vec<u8>>,
    fault: Option<Fault>,
) -> Request<'a> {
    Request {
        root,
        save_dir: save,
        operation_id: "op-p2-1",
        evidence_root,
        evidence_dir,
        backup_parent,
        candidate_parent,
        overlays,
        fault,
        extra_targets: Vec::new(),
        preview_id: None,
        instance_tag: "",
        hooks: Default::default(),
        inner_lock: Default::default(),
    }
}

fn valid_overlay() -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    m.insert(
        "sheets/aria.yaml".to_string(),
        b"version: 1\nid: aria\nname: Aria\nfile: Characters/Aria.md\nruleset: \"2024\"\nnotes:\n  - leveled in candidate only\n".to_vec(),
    );
    m
}

fn open_fast(root: &Path, save: &Path) -> adv::AdvancementLock {
    let mut opts = adv::OpenOptions::new();
    opts.acquire = adv::AcquireOptions {
        timeout: Duration::from_millis(200),
        retry: Duration::from_millis(10),
    };
    adv::open(root, save, &opts).unwrap()
}

#[test]
fn devices_match_rejects_injected_cross_volume_keys() {
    // Real cross-volume mounts are NOT_RUN in this CI. The comparison itself
    // is proven with injected keys so a mismatch cannot be silent.
    assert!(devices_match(DeviceKey(1, 0), DeviceKey(1, 0)));
    assert!(!devices_match(DeviceKey(1, 0), DeviceKey(2, 0)));
}

#[test]
fn a_copy_cannot_use_a_nested_or_ancestor_destination() {
    let root = root("nested-copy-target");
    let save = save_of(&root);
    write_synthetic_save(&save);
    let nested = save.join(".cache");
    assert!(copy_user_data(&save, &nested)
        .unwrap_err()
        .to_string()
        .contains("受保护目录之外"));
    assert!(copy_user_data(&nested, &save)
        .unwrap_err()
        .to_string()
        .contains("受保护目录之外"));
    assert!(!nested.join("progress.yaml").exists());
    assert_eq!(fs::read(nested.join("digest.md")).unwrap(), b"cache\n");
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_candidate_cannot_be_created_inside_the_verified_backup() {
    let (root, save, evidence_root, evidence_dir, backup_parent, _, before) =
        setup("nested-candidate-parent");
    let backup = backup_parent.join("2030-01-02-01-op-p2-1-d-backup");
    let error = run_to_verified_candidate(&request(
        &root,
        &save,
        &evidence_root,
        &evidence_dir,
        &backup_parent,
        &backup,
        valid_overlay(),
        None,
    ))
    .expect_err("candidate would copy its own output");
    assert_eq!(
        error.report.recovery.interrupt_stage,
        InterruptStage::Candidate
    );
    assert!(error.report.recovery.backup_verified);
    assert!(error
        .report
        .error
        .as_deref()
        .unwrap()
        .contains("受保护目录之外"));
    assert!(backup.join("d-backup.manifest.json").is_file());
    assert!(!backup.join("2030-01-02-01-op-p2-1-d-candidate").exists());
    assert_eq!(protected_target_bytes(&save).unwrap(), before);
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn ambiguous_encounter_stops_advancement_before_creating_operation_evidence() {
    let root = root("ambiguous-encounter");
    fs::write(root.join("AGENTS.md"), "Synthetic play root\n").unwrap();
    let save = save_of(&root);
    write_synthetic_save(&save);
    fs::write(save.join("combat/fight.state.json"), "[invalid").unwrap();
    init_protocol(&root, &save);
    let before = protected_target_bytes(&save).unwrap();
    let (evidence_root, evidence_dir, backup_parent, candidate_parent) = parents(&root);
    let error = run_to_verified_candidate(&request(
        &root,
        &save,
        &evidence_root,
        &evidence_dir,
        &backup_parent,
        &candidate_parent,
        valid_overlay(),
        None,
    ))
    .expect_err("an ambiguous encounter must prevent preparing to publish");
    assert_eq!(error.report.recovery.interrupt_stage, InterruptStage::Admit);
    let message = error.report.error.as_deref().unwrap();
    assert!(
        message.contains("当前只允许预览") && message.contains("fight.state.json"),
        "{message}"
    );
    assert!(error.report.recovery.prestate.is_none());
    assert!(error.report.backup_dir.is_none());
    assert_eq!(protected_target_bytes(&save).unwrap(), before);
    assert_eq!(open_fast(&root, &save).state().state, adv::State::Ready);
    let _ = fs::remove_dir_all(&root);
}

#[cfg(not(unix))]
#[test]
fn windows_d_commit_fails_closed_without_writing_targets() {
    let root = root("win-closed");
    let save = save_of(&root);
    write_synthetic_save(&save);
    init_protocol(&root, &save);
    let before = protected_target_bytes(&save).unwrap();
    let (evidence_root, evidence_dir, backup_parent, candidate_parent) = parents(&root);
    let err = run_to_verified_candidate(&request(
        &root,
        &save,
        &evidence_root,
        &evidence_dir,
        &backup_parent,
        &candidate_parent,
        valid_overlay(),
        None,
    ))
    .expect_err("Windows cannot sync directories");
    assert_eq!(err.report.recovery.interrupt_stage, InterruptStage::Admit);
    assert!(!err.report.candidate_verified);
    assert!(!err.report.published);
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
}

#[cfg(unix)]
fn setup(
    tag: &str,
) -> (
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    BTreeMap<String, Vec<u8>>,
) {
    let root = root(tag);
    let save = save_of(&root);
    write_synthetic_save(&save);
    init_protocol(&root, &save);
    let before = protected_target_bytes(&save).unwrap();
    let (evidence_root, evidence_dir, backup_parent, candidate_parent) = parents(&root);
    (
        root,
        save,
        evidence_root,
        evidence_dir,
        backup_parent,
        candidate_parent,
        before,
    )
}

#[cfg(unix)]
#[test]
fn happy_path_marks_candidate_verified_and_does_not_publish() {
    let (root, save, evr, evd, bp, cp, before) = setup("happy");
    let report = run_to_verified_candidate(&request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        valid_overlay(),
        None,
    ))
    .expect("verified unpublished candidate");
    assert!(report.is_verified_unpublished());
    assert_eq!(report.protocol_state, adv::State::Active);
    assert_eq!(report.stage, adv::Stage::Candidate);
    assert!(!report.published);

    let after = protected_target_bytes(&save).unwrap();
    assert!(
        targets_unchanged(&before, &after),
        "live targets changed despite unpublished candidate"
    );
    // The overlay must exist only in the candidate, not in the save.
    let live = String::from_utf8(fs::read(save.join("sheets/aria.yaml")).unwrap()).unwrap();
    assert!(!live.contains("leveled in candidate only"), "{live}");
    let cand = report.candidate_dir.as_ref().unwrap();
    let cand_sheet = String::from_utf8(fs::read(cand.join("sheets/aria.yaml")).unwrap()).unwrap();
    assert!(
        cand_sheet.contains("leveled in candidate only"),
        "{cand_sheet}"
    );

    // Backup includes user checkpoints and nested same-named files, excludes
    // root control / cache / evidence.
    let backup = report.backup_dir.as_ref().unwrap();
    assert!(backup.join("checkpoints/cp-1/Characters/Aria.md").is_file());
    assert!(backup.join("combat/nested/digest.md").is_file());
    assert!(!backup.join(".cache").exists());
    assert!(!backup.join("digest.md").exists());
    assert!(!backup.join("secret-terms.cache.json").exists());
    assert!(!backup.join(adv::LOCK_FILE).exists());
    assert!(!backup.join(adv::STATE_FILE).exists());
    assert!(!backup.join(".talkdnd.lock").exists());
    let manifest = fs::read_to_string(backup.join("d-backup.manifest.json")).unwrap();
    assert!(manifest.contains("\"verified\": true"));
    assert!(manifest.contains("checkpoints/cp-1/Characters/Aria.md"));

    // Prestate snapshot exists under the caller-provided evidence dir.
    assert!(evd.join("op-p2-1-prestate.json").is_file());
}

#[cfg(unix)]
fn assert_failure_leaves_targets(
    tag: &str,
    fault: Fault,
    expected: InterruptStage,
    backup_verified: bool,
    needle: &str,
) {
    let (root, save, evr, evd, bp, cp, before) = setup(tag);
    let err = run_to_verified_candidate(&request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        valid_overlay(),
        Some(fault),
    ))
    .expect_err("injected fault must stop");
    assert_eq!(err.report.recovery.interrupt_stage, expected);
    assert!(!err.report.candidate_verified);
    assert!(!err.report.published);
    assert_eq!(err.report.recovery.backup_verified, backup_verified);
    let msg = err.to_string();
    assert!(msg.contains(needle), "{msg}");
    assert!(
        msg.contains("未发布") || msg.contains("未写任何目标"),
        "{msg}"
    );
    assert!(
        msg.contains("恢复所需材料") || err.report.recovery.note.contains("恢复所需材料"),
        "{msg}"
    );
    let after = protected_target_bytes(&save).unwrap();
    assert!(
        targets_unchanged(&before, &after),
        "targets changed after {tag}"
    );
    // State must not claim a verified candidate.
    let lock = open_fast(&root, &save);
    assert!(lock.state().candidate_dir.is_none());
    if !backup_verified {
        // Either never recorded, or recorded only when verified.
        assert!(lock.state().backup_dir.is_none() || lock.state().stage != adv::Stage::Candidate);
        assert_ne!(lock.state().stage, adv::Stage::Candidate);
    }
}

#[cfg(unix)]
#[test]
fn backup_copy_fault_keeps_incomplete_backup_and_targets() {
    let (root, save, evr, evd, bp, cp, before) = setup("fault-copy");
    let err = run_to_verified_candidate(&request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        valid_overlay(),
        Some(Fault::BackupCopy),
    ))
    .expect_err("copy fault");
    assert_eq!(err.report.recovery.interrupt_stage, InterruptStage::Backup);
    assert!(!err.report.recovery.backup_verified);
    assert!(!err.report.candidate_verified);
    assert!(!err.report.published);
    let msg = err.to_string();
    assert!(msg.contains("backup"), "{msg}");
    assert!(
        msg.contains("不完整备份") || msg.contains("注入失败"),
        "{msg}"
    );
    // The exclusive backup dir was created and left behind.
    let leftovers: Vec<_> = fs::read_dir(&bp)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("op-p2-1-d-backup"))
        .collect();
    assert_eq!(leftovers.len(), 1, "incomplete backup should be kept");
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
    let lock = open_fast(&root, &save);
    assert!(lock.state().backup_dir.is_none());
    assert_eq!(lock.state().state, adv::State::Failed);
    assert_eq!(lock.state().stage, adv::Stage::StoppedBeforeWrite);
}

#[cfg(unix)]
#[test]
fn backup_manifest_fault_does_not_record_a_verified_backup() {
    assert_failure_leaves_targets(
        "fault-manifest",
        Fault::BackupManifest,
        InterruptStage::Backup,
        false,
        "清单",
    );
}

#[cfg(unix)]
#[test]
fn backup_sync_fault_is_not_a_power_loss_claim() {
    assert_failure_leaves_targets(
        "fault-sync",
        Fault::BackupSync,
        InterruptStage::Backup,
        false,
        "同步",
    );
}

#[cfg(unix)]
#[test]
fn candidate_build_fault_does_not_mark_verified() {
    assert_failure_leaves_targets(
        "fault-cand-build",
        Fault::CandidateBuild,
        InterruptStage::Candidate,
        true,
        "candidate 构建",
    );
}

#[cfg(unix)]
#[test]
fn candidate_validate_fault_does_not_mark_verified() {
    assert_failure_leaves_targets(
        "fault-cand-validate",
        Fault::CandidateValidate,
        InterruptStage::Candidate,
        true,
        "candidate 校验",
    );
}

#[cfg(unix)]
#[test]
fn candidate_rejects_schema_and_missing_identity_without_forging() {
    let (root, save, evr, evd, bp, cp, before) = setup("bad-identity");
    let mut overlays = BTreeMap::new();
    overlays.insert(
        "combat/party-state.yaml".to_string(),
        b"version: 1\nmembers:\n  aria:\n    name: Aria\n    hp: 12\n    max_hp: 12\n  ghost:\n    name: Ghost\n    hp: 1\n    max_hp: 1\n".to_vec(),
    );
    let err =
        run_to_verified_candidate(&request(&root, &save, &evr, &evd, &bp, &cp, overlays, None))
            .expect_err("missing identity");
    let msg = err.to_string();
    assert!(msg.contains("ghost"), "{msg}");
    assert!(msg.contains("不伪造"), "{msg}");
    assert!(!err.report.candidate_verified);
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));

    // Schema 11 save-meta is refused; CURRENT_SCHEMA stays 10.
    let (root, save, evr, evd, bp, cp, before) = setup("schema11");
    let mut overlays = BTreeMap::new();
    overlays.insert(
        "save-meta.yaml".to_string(),
        b"schema: 11\ntool: talkdnd\n".to_vec(),
    );
    let err =
        run_to_verified_candidate(&request(&root, &save, &evr, &evd, &bp, &cp, overlays, None))
            .expect_err("schema 11");
    let msg = err.to_string();
    assert!(msg.contains("11"), "{msg}");
    assert!(
        msg.contains("CURRENT_SCHEMA") || msg.contains("10"),
        "{msg}"
    );
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
    assert_eq!(
        fs::read_to_string(save.join("save-meta.yaml")).unwrap(),
        "schema: 10\ntool: talkdnd\n"
    );
}

#[cfg(unix)]
#[test]
fn special_file_stops_backup_and_keeps_the_incomplete_directory() {
    let (root, save, evr, evd, bp, cp, before) = setup("special");
    std::os::unix::fs::symlink("/tmp", save.join("escape-link")).unwrap();
    let err = run_to_verified_candidate(&request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        valid_overlay(),
        None,
    ))
    .expect_err("symlink");
    assert_eq!(err.report.recovery.interrupt_stage, InterruptStage::Backup);
    assert!(!err.report.recovery.backup_verified);
    let msg = err.to_string();
    assert!(
        msg.contains("符号链接") || msg.contains("escape-link"),
        "{msg}"
    );
    let leftovers: Vec<_> = fs::read_dir(&bp)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("op-p2-1-d-backup"))
        .collect();
    assert_eq!(leftovers.len(), 1);
    // The leftover must not have followed the link out of the save.
    let backup = leftovers[0].path();
    if backup.join("escape-link").exists() {
        let meta = fs::symlink_metadata(backup.join("escape-link")).unwrap();
        assert!(
            meta.file_type().is_symlink() || !backup.join("escape-link").is_dir(),
            "must not materialize the link target inside the backup"
        );
    }
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
}

#[cfg(unix)]
#[test]
fn existing_backup_directory_is_not_treated_as_complete() {
    let (root, save, evr, evd, bp, cp, before) = setup("exists-not-complete");
    let dest = bp.join("2030-01-02-01-op-p2-1-d-backup");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("stale.txt"), "nope").unwrap();
    let err = run_to_verified_candidate(&request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        valid_overlay(),
        None,
    ))
    .expect_err("existing dir");
    assert_eq!(err.report.recovery.interrupt_stage, InterruptStage::Backup);
    assert!(!err.report.recovery.backup_verified);
    let msg = err.to_string();
    assert!(
        msg.contains("独占") || msg.contains("已存在") || msg.contains("完整"),
        "{msg}"
    );
    assert_eq!(fs::read_to_string(dest.join("stale.txt")).unwrap(), "nope");
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
}

#[cfg(unix)]
#[test]
fn admit_evidence_failure_does_not_write_active_or_targets() {
    let (root, save, evr, _evd, bp, cp, before) = setup("admit-fail");
    // evidence_root is required to exist; pointing it at a missing path fails
    // before active is written.
    let missing = evr.join("missing-root");
    let err = run_to_verified_candidate(&request(
        &root,
        &save,
        &missing,
        &missing.join("ops"),
        &bp,
        &cp,
        valid_overlay(),
        None,
    ))
    .expect_err("missing evidence root");
    assert_eq!(err.report.recovery.interrupt_stage, InterruptStage::Admit);
    assert!(!err.report.published);
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
    let lock = open_fast(&root, &save);
    assert_eq!(lock.state().state, adv::State::Ready);
    assert_ne!(lock.state().state, adv::State::Active);
}

#[cfg(unix)]
#[test]
fn leftover_active_refuses_ordinary_writes_and_is_not_auto_resumed() {
    // T4a/T4e fixture portion (not a process-kill or power-loss test): an
    // operation left `active` must refuse ordinary writes, and a new
    // orchestrator run must not continue it.
    let (root, save, evr, evd, bp, cp, before) = setup("no-resume");
    let mut lock = open_fast(&root, &save);
    lock.check_writable().unwrap();
    lock.begin_operation("op-stuck", &evr, &evd).unwrap();
    assert_eq!(lock.state().state, adv::State::Active);
    drop(lock);

    let lock = open_fast(&root, &save);
    let ordinary = lock.check_ordinary_write();
    assert!(ordinary.is_err(), "active must refuse ordinary writes");
    assert!(
        ordinary.unwrap_err().detail().contains("active"),
        "error should name active"
    );
    drop(lock);

    let err = run_to_verified_candidate(&request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        valid_overlay(),
        None,
    ))
    .expect_err("must not auto-resume");
    assert_eq!(err.report.recovery.interrupt_stage, InterruptStage::Admit);
    assert!(!err.report.candidate_verified);
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
    let lock = open_fast(&root, &save);
    assert_eq!(lock.state().state, adv::State::Active);
    assert_eq!(lock.state().operation_id, "op-stuck");
    assert!(lock.state().backup_dir.is_none());
}

#[cfg(unix)]
#[test]
fn state_replaced_after_admit_stops_before_publish() {
    // T2a fixture portion: a state-file replacement at a stage boundary is
    // detected; this is not a claim about every TOCTOU window or power loss.
    let (root, save, evr, evd, bp, cp, before) = setup("t2a-replace");
    let mut lock = open_fast(&root, &save);
    lock.begin_operation("op-p2-1", &evr, &evd).unwrap();
    // Rewrite the state on disk so the next store (record_backup) fails closed.
    let path = adv::state_path(&save);
    let mut record: adv::StateRecord = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record.revision = record.revision.saturating_add(9);
    adv::reseal(&mut record).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let backup = bp.join("2030-01-02-01-op-p2-1-d-backup");
    fs::create_dir(&backup).unwrap();
    let err = lock
        .record_backup(backup.to_str().unwrap())
        .expect_err("replaced state");
    assert!(
        err.kind() == adv::ErrorKind::CorruptState
            || err.kind() == adv::ErrorKind::UnwritableState
            || err.kind() == adv::ErrorKind::IdentityMismatch,
        "{err}"
    );
    drop(lock);
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
    let _ = (cp,);
}

#[cfg(unix)]
#[test]
fn failed_run_is_reopenable_but_does_not_auto_resume_the_old_backup() {
    let (root, save, evr, evd, bp, cp, before) = setup("reopen-failed");
    let _ = run_to_verified_candidate(&request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        valid_overlay(),
        Some(Fault::BackupManifest),
    ))
    .expect_err("fault");
    let lock = open_fast(&root, &save);
    assert_eq!(lock.state().state, adv::State::Failed);
    lock.check_ordinary_write().unwrap();
    drop(lock);
    // A new operation_id is a new attempt, not a resume of the incomplete backup.
    let mut req = request(&root, &save, &evr, &evd, &bp, &cp, valid_overlay(), None);
    req.operation_id = "op-p2-2";
    let report = run_to_verified_candidate(&req).expect("new operation");
    assert!(report.is_verified_unpublished());
    let backup_name = report
        .backup_dir
        .as_ref()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy();
    assert!(backup_name.contains("op-p2-2-d-backup"), "{backup_name}");
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
}
