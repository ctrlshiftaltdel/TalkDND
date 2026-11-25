//! ADV-P1-5 explicit recovery: synthetic fixtures only.
//!
//! These tests measure T9a/T9b under injected faults and hooks. They are
//! not proof of safety for real saves, real process kills, power loss,
//! directory durability, or network filesystems. #264 remains unsolved.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use talkdnd::advancement_commit::{
    binding_preview_id, protected_target_bytes, run_to_complete, targets_unchanged, Fault,
    PublishHooks, Request,
};
use talkdnd::advancement_inner_lock::InnerLockOptions;
use talkdnd::advancement_lock as adv;
use talkdnd::advancement_lock::{Stage, State};
use talkdnd::advancement_recovery::{
    inspect_recovery, recover, RecoveryChoice, RecoveryFault, RecoveryHooks, RecoveryRequest,
};

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "talkdnd-p5-{tag}-{}-{}",
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
    fs::write(save.join("synthetic-ext.txt"), "v1\n").unwrap();
}

fn write_baseline_manifest(save: &Path) {
    let mut m = combat_sim::integrity::baseline(save);
    m.accepted.push(combat_sim::integrity::Accepted {
        date: "2030-01-02".to_string(),
        reason: "player accepted a prior hand edit".to_string(),
        files: vec!["save-meta.yaml".to_string()],
    });
    combat_sim::integrity::store(save, &m).unwrap();
}

fn growth_overlays() -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    m.insert(
        "sheets/aria.yaml".to_string(),
        b"version: 1\nid: aria\nname: Aria\nfile: Characters/Aria.md\nruleset: \"2024\"\nlevel: 2\n"
            .to_vec(),
    );
    m.insert(
        "Characters/Aria.md".to_string(),
        b"# Aria\n\nLevel 2\n".to_vec(),
    );
    m.insert(
        "combat/party-state.yaml".to_string(),
        b"version: 1\nmembers:\n  aria:\n    name: Aria\n    hp: 20\n    max_hp: 20\n".to_vec(),
    );
    m.insert(
        "progress.yaml".to_string(),
        b"version: 1\nxp:\n  aria: 300\npurses:\n  aria:\n    cp: 0\n".to_vec(),
    );
    m.insert("synthetic-ext.txt".to_string(), b"v2-registered\n".to_vec());
    m
}

fn extra_targets() -> Vec<String> {
    vec!["synthetic-ext.txt".to_string()]
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

fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let root = root(tag);
    let save = save_of(&root);
    write_synthetic_save(&save);
    init_protocol(&root, &save);
    write_baseline_manifest(&save);
    let (evr, evd, bp, cp) = parents(&root);
    (root, save, evr, evd, bp, cp)
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
        operation_id: "op-p1-5",
        evidence_root,
        evidence_dir,
        backup_parent,
        candidate_parent,
        overlays,
        fault,
        extra_targets: extra_targets(),
        preview_id: None,
        instance_tag: "",
        hooks: PublishHooks::default(),
        inner_lock: InnerLockOptions::default(),
    }
}

fn open_fast(root: &Path, save: &Path) -> adv::AdvancementLock {
    let mut opts = adv::OpenOptions::new();
    opts.acquire = adv::AcquireOptions {
        timeout: Duration::from_millis(200),
        retry: Duration::from_millis(10),
    };
    adv::open(root, save, &opts).unwrap()
}

fn marker_count(save: &Path, operation_id: &str) -> usize {
    let marker = format!("<!-- talkdnd-op:{operation_id} -->");
    let rolls = fs::read_to_string(save.join("rolls.log")).unwrap_or_default();
    rolls.lines().filter(|l| l.contains(&marker)).count()
}

fn rec_dir(evr: &Path, tag: &str) -> PathBuf {
    evr.join("2030-01-02-01").join(tag)
}

#[allow(clippy::too_many_arguments)]
fn rec_req<'a>(
    root: &'a Path,
    save: &'a Path,
    evr: &'a Path,
    evd: &'a Path,
    recovery_id: &'a str,
    choice: Option<RecoveryChoice>,
    confirm: bool,
    fault: Option<RecoveryFault>,
) -> RecoveryRequest<'a> {
    RecoveryRequest {
        root,
        save_dir: save,
        recovery_id,
        evidence_root: evr,
        evidence_dir: evd,
        choice,
        confirm_choice: confirm,
        extra_targets: extra_targets(),
        fault,
        hooks: RecoveryHooks::default(),
        inner_lock: InnerLockOptions::default(),
    }
}

struct Leftover {
    root: PathBuf,
    save: PathBuf,
    evr: PathBuf,
    #[allow(dead_code)]
    evd: PathBuf,
    backup: PathBuf,
    candidate: PathBuf,
    #[allow(dead_code)]
    preimage: BTreeMap<String, Vec<u8>>,
    #[allow(dead_code)]
    ext_before: Vec<u8>,
    leftover_state: State,
    backup_dir: Option<String>,
    pending: Vec<String>,
    published: Vec<String>,
    revision: u64,
}

fn leftover_with_publish_fault(tag: &str, fault: Fault) -> Leftover {
    let (root, save, evr, evd, bp, cp) = setup(tag);
    let preimage = protected_target_bytes(&save).unwrap();
    let ext_before = fs::read(save.join("synthetic-ext.txt")).unwrap();
    let preview = binding_preview_id(&save, &extra_targets()).unwrap();
    let mut req = request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        growth_overlays(),
        Some(fault),
    );
    req.preview_id = Some(preview);
    let err = run_to_complete(&req).expect_err("expected leftover");
    let lock = open_fast(&root, &save);
    let leftover_state = lock.state().state;
    let backup_dir = lock.state().backup_dir.clone();
    let pending: Vec<String> = lock
        .state()
        .pending
        .iter()
        .map(|p| p.path.clone())
        .collect();
    let published: Vec<String> = lock
        .state()
        .published
        .iter()
        .map(|p| p.path.clone())
        .collect();
    let revision = lock.state().revision;
    let backup = PathBuf::from(lock.state().backup_dir.as_ref().expect("backup"));
    let candidate = PathBuf::from(lock.state().candidate_dir.as_ref().expect("candidate"));
    drop(lock);
    assert!(
        leftover_state == State::InDoubt || leftover_state == State::Failed,
        "leftover should be unfinished, got {leftover_state:?}: {:?}",
        err.report.error
    );
    Leftover {
        root,
        save,
        evr,
        evd,
        backup,
        candidate,
        preimage,
        ext_before,
        leftover_state,
        backup_dir,
        pending,
        published,
        revision,
    }
}

fn snapshot_tree(save: &Path) -> (BTreeMap<String, Vec<u8>>, Vec<u8>, Vec<u8>) {
    (
        protected_target_bytes(save).unwrap(),
        fs::read(save.join("synthetic-ext.txt")).unwrap_or_default(),
        fs::read(save.join("rolls.log")).unwrap_or_default(),
    )
}

fn tree_unchanged(save: &Path, before: &(BTreeMap<String, Vec<u8>>, Vec<u8>, Vec<u8>)) -> bool {
    let after = snapshot_tree(save);
    targets_unchanged(&before.0, &after.0) && before.1 == after.1 && before.2 == after.2
}

fn assert_scope_reported(paths: &[String]) {
    assert!(
        paths.iter().any(|p| p == "save-meta.yaml"),
        "save-meta.yaml is outside the registered target set and must be reported, got {paths:?}"
    );
    assert!(
        paths
            .iter()
            .any(|p| p == "checkpoints/cp-1/Characters/Aria.md"),
        "checkpoint file must be reported as scope-limited, got {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p == "digest.md" || p == "secret-terms.cache.json"),
        "root cache files are excluded from user data, should not appear as scope-limited: {paths:?}"
    );
}

fn assert_leftover_intent_unchanged(fx: &Leftover) {
    let lock = open_fast(&fx.root, &fx.save);
    assert_eq!(lock.state().state, fx.leftover_state);
    assert_eq!(lock.state().revision, fx.revision);
    assert_eq!(lock.state().backup_dir, fx.backup_dir);
    let pending: Vec<String> = lock
        .state()
        .pending
        .iter()
        .map(|p| p.path.clone())
        .collect();
    let published: Vec<String> = lock
        .state()
        .published
        .iter()
        .map(|p| p.path.clone())
        .collect();
    assert_eq!(pending, fx.pending);
    assert_eq!(published, fx.published);
}

fn bytes_at(root: &Path, rel: &str) -> Vec<u8> {
    fs::read(root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))).unwrap_or_default()
}

#[cfg(not(unix))]
#[test]
fn windows_recovery_fails_closed_without_writing_targets() {
    // Admit itself fails closed on this platform (no directory sync), so a
    // leftover `active`/`in_doubt` cannot be created here. Recovery still
    // must fail closed at evidence-directory sync and must not write targets.
    // `ensure_evidence_dir` is the first step of work-evidence creation.
    let (root, save, evr, evd, bp, cp) = setup("win-rec");
    let before = snapshot_tree(&save);
    let ev_err = adv::ensure_evidence_dir(&evr, &evd).expect_err("directory sync must fail closed");
    let detail = format!("{ev_err}");
    assert!(
        ev_err.detail().contains("cannot sync") || detail.contains("sync"),
        "error should name directory sync: {detail}"
    );

    let preview = binding_preview_id(&save, &extra_targets()).unwrap();
    let mut pub_req = request(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), None);
    pub_req.preview_id = Some(preview);
    let pub_err = run_to_complete(&pub_req).expect_err("Windows cannot publish");
    assert!(pub_err.report.published_paths.is_empty());

    let rec_evd = rec_dir(&evr, "win");
    let req = rec_req(
        &root,
        &save,
        &evr,
        &rec_evd,
        "rec-win",
        Some(RecoveryChoice::RollbackToPreimage),
        true,
        None,
    );
    let result = recover(&req);
    match result {
        Ok(report) => {
            assert!(report.written_paths.is_empty());
            assert!(!report.evidence_established);
        }
        Err(err) => {
            assert!(err.report.written_paths.is_empty());
            assert!(!err.report.evidence_established);
        }
    }
    assert!(tree_unchanged(&save, &before));
}

#[cfg(unix)]
#[test]
fn t9a_work_evidence_faults_stop_with_zero_target_writes() {
    for (tag, fault) in [
        ("t9a-wcreate", RecoveryFault::WorkCreate),
        ("t9a-wwrite", RecoveryFault::WorkWrite),
        ("t9a-wsync", RecoveryFault::WorkSync),
        ("t9a-wreread", RecoveryFault::WorkReread),
        ("t9a-wdir", RecoveryFault::WorkDirSync),
        ("t9a-screate", RecoveryFault::SnapshotCreate),
        ("t9a-swrite", RecoveryFault::SnapshotWrite),
        ("t9a-ssync", RecoveryFault::SnapshotSync),
        ("t9a-sreread", RecoveryFault::SnapshotReread),
        ("t9a-sdir", RecoveryFault::SnapshotDirSync),
    ] {
        let fx = leftover_with_publish_fault(tag, Fault::LogAppend);
        let before = snapshot_tree(&fx.save);
        let evd = rec_dir(&fx.evr, tag);
        let rec_id = format!("rec-{tag}");
        let req = rec_req(
            &fx.root,
            &fx.save,
            &fx.evr,
            &evd,
            &rec_id,
            Some(RecoveryChoice::RollbackToPreimage),
            true,
            Some(fault),
        );
        let err = recover(&req).expect_err(&format!("{fault:?} must stop"));
        assert!(
            err.report.written_paths.is_empty(),
            "{fault:?} must not write targets: {:?}",
            err.report.written_paths
        );
        assert!(
            !err.report.evidence_established,
            "{fault:?} must not claim evidence is established"
        );
        assert!(
            err.report
                .error
                .as_ref()
                .is_some_and(|e| e.contains("工作证据")
                    || e.contains("注入失败")
                    || e.contains("目录")),
            "{fault:?} error should name the evidence failure: {:?}",
            err.report.error
        );
        assert!(
            tree_unchanged(&fx.save, &before),
            "{fault:?} changed the tree"
        );
        assert_leftover_intent_unchanged(&fx);
    }
}

#[cfg(unix)]
#[test]
fn both_recovery_choices_hold_the_integrity_lock_from_intent_through_done() {
    use std::{cell::Cell, time::Duration};

    for (tag, choice) in [
        ("lock-continue", RecoveryChoice::ContinueToTarget),
        ("lock-rollback", RecoveryChoice::RollbackToPreimage),
    ] {
        let fx = leftover_with_publish_fault(tag, Fault::LogAppend);
        let evd = rec_dir(&fx.evr, tag);
        let checks = Cell::new(0);
        let compete = |rel: &str, save: &Path| {
            if rel == "integrity.yaml" {
                assert!(
                    combat_sim::integrity::lock_with_timeout(save, Duration::ZERO).is_err(),
                    "another writer acquired the integrity lock during {tag}"
                );
                checks.set(checks.get() + 1);
            }
            Ok(())
        };
        let mut req = rec_req(
            &fx.root,
            &fx.save,
            &fx.evr,
            &evd,
            tag,
            Some(choice),
            true,
            None,
        );
        req.hooks.after_intent = Some(&compete);
        req.hooks.after_done = Some(&compete);
        recover(&req).unwrap();
        assert_eq!(checks.get(), 2, "{tag}");
        assert!(combat_sim::integrity::lock_with_timeout(&fx.save, Duration::ZERO).is_ok());
    }
}

#[cfg(unix)]
#[test]
fn continue_to_target_finishes_partial_publish_including_extension() {
    let fx = leftover_with_publish_fault("cont", Fault::LogAppend);
    // Replace targets (including the synthetic extension) are already at the
    // candidate; logs and integrity are not.
    assert_eq!(bytes_at(&fx.save, "synthetic-ext.txt"), b"v2-registered\n");
    assert_eq!(marker_count(&fx.save, "op-p1-5"), 0);
    let evd = rec_dir(&fx.evr, "cont");
    let req = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-cont",
        Some(RecoveryChoice::ContinueToTarget),
        true,
        None,
    );
    let report = recover(&req).expect("continue");
    assert_eq!(report.protocol_state, State::Complete);
    assert!(report.evidence_established);
    assert_eq!(report.original_operation_id.as_deref(), Some("op-p1-5"));
    assert_scope_reported(&report.scope_limited_paths);
    assert!(
        report
            .file_progress
            .iter()
            .any(|f| f.path == "synthetic-ext.txt" && f.done),
        "extension target must be in file-by-file recovery, got {:?}",
        report.file_progress
    );
    assert!(!report
        .written_paths
        .contains(&"synthetic-ext.txt".to_string()));
    assert!(report
        .already_done_paths
        .contains(&"synthetic-ext.txt".to_string()));
    assert_eq!(
        bytes_at(&fx.save, "sheets/aria.yaml"),
        bytes_at(&fx.candidate, "sheets/aria.yaml")
    );
    assert_eq!(bytes_at(&fx.save, "synthetic-ext.txt"), b"v2-registered\n");
    assert_eq!(marker_count(&fx.save, "op-p1-5"), 1);
    assert_eq!(
        bytes_at(&fx.save, "checkpoints/cp-1/Characters/Aria.md"),
        b"# Aria checkpoint\n",
        "unregistered files must not be overwritten"
    );
    let lock = open_fast(&fx.root, &fx.save);
    assert!(lock.check_ordinary_write().is_ok());
    drop(lock);

    // Starting recovery again must not re-apply (log marker stays 1).
    let evd2 = rec_dir(&fx.evr, "cont-again");
    let again = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd2,
        "rec-cont-again",
        Some(RecoveryChoice::ContinueToTarget),
        true,
        None,
    );
    let report = recover(&again);
    // The leftover is already complete, so recovery must not write targets
    // or append another log marker.
    match report {
        Ok(r) => {
            assert!(
                r.written_paths.is_empty(),
                "second recover wrote {:?}",
                r.written_paths
            );
        }
        Err(err) => {
            assert!(
                err.report.written_paths.is_empty(),
                "second recover wrote {:?}",
                err.report.written_paths
            );
        }
    }
    assert_eq!(marker_count(&fx.save, "op-p1-5"), 1);
}

#[cfg(unix)]
#[test]
fn recovery_rejects_changes_to_backup_contents_or_metadata() {
    use std::os::unix::fs::PermissionsExt;

    for (tag, damage) in [
        ("backup-content", "恢复备份条目与清单不一致"),
        ("backup-mode", "备份条目元数据与清单不一致"),
    ] {
        let fx = leftover_with_publish_fault(tag, Fault::LogAppend);
        let path = fx.backup.join("sheets/aria.yaml");
        if tag == "backup-content" {
            fs::write(path, b"changed backup bytes\n").unwrap();
        } else {
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            fs::set_permissions(path, fs::Permissions::from_mode(mode ^ 0o040)).unwrap();
        }
        let before = snapshot_tree(&fx.save);
        let evd = rec_dir(&fx.evr, tag);
        let req = rec_req(
            &fx.root,
            &fx.save,
            &fx.evr,
            &evd,
            tag,
            Some(RecoveryChoice::RollbackToPreimage),
            true,
            None,
        );
        let failure = recover(&req).expect_err("changed backup evidence must stop recovery");
        let error = failure.report.error.as_deref().unwrap();
        assert!(
            error.contains(damage) && error.contains("sheets/aria.yaml"),
            "{error}"
        );
        assert!(!failure.report.evidence_established);
        assert!(failure.report.written_paths.is_empty());
        assert!(tree_unchanged(&fx.save, &before));
        assert_leftover_intent_unchanged(&fx);
    }
}

#[cfg(unix)]
#[test]
fn recovery_accepts_the_legacy_backup_manifest() {
    let fx = leftover_with_publish_fault("backup-v1", Fault::LogAppend);
    let path = fx.backup.join("d-backup.manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["schema"] = serde_json::json!(1);
    manifest.as_object_mut().unwrap().remove("source_metadata");
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let evd = rec_dir(&fx.evr, "backup-v1");
    let req = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-backup-v1",
        Some(RecoveryChoice::RollbackToPreimage),
        true,
        None,
    );
    let report = recover(&req).expect("version 1 backup still supplies the byte preimage");
    assert_eq!(report.protocol_state, State::Failed);
    assert_eq!(
        bytes_at(&fx.save, "sheets/aria.yaml"),
        fx.preimage["sheets/aria.yaml"]
    );
}

#[cfg(unix)]
#[test]
fn rollback_to_preimage_restores_published_files_including_extension() {
    let fx = leftover_with_publish_fault("roll", Fault::LogAppend);
    assert_ne!(
        bytes_at(&fx.save, "sheets/aria.yaml"),
        bytes_at(&fx.backup, "sheets/aria.yaml")
    );
    let evd = rec_dir(&fx.evr, "roll");
    let req = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-roll",
        Some(RecoveryChoice::RollbackToPreimage),
        true,
        None,
    );
    let report = recover(&req).expect("rollback");
    assert_eq!(report.protocol_state, State::Failed);
    assert_eq!(report.stage, Stage::StoppedBeforeWrite);
    assert!(report.evidence_established);
    assert_scope_reported(&report.scope_limited_paths);
    assert!(
        report
            .file_progress
            .iter()
            .any(|f| f.path == "synthetic-ext.txt"),
        "extension must be compared, got {:?}",
        report.file_progress
    );
    assert_eq!(
        bytes_at(&fx.save, "sheets/aria.yaml"),
        bytes_at(&fx.backup, "sheets/aria.yaml")
    );
    assert_eq!(
        bytes_at(&fx.save, "progress.yaml"),
        bytes_at(&fx.backup, "progress.yaml")
    );
    assert_eq!(bytes_at(&fx.save, "synthetic-ext.txt"), b"v1\n");
    assert_eq!(
        bytes_at(&fx.save, "rolls.log"),
        bytes_at(&fx.backup, "rolls.log")
    );
    assert_eq!(marker_count(&fx.save, "op-p1-5"), 0);
    assert_eq!(
        bytes_at(&fx.save, "save-meta.yaml"),
        b"schema: 10\ntool: talkdnd\n"
    );
    let lock = open_fast(&fx.root, &fx.save);
    assert!(
        lock.check_ordinary_write().is_ok(),
        "rollback must allow ordinary writes again: {:?}",
        lock.check_ordinary_write()
    );
    drop(lock);

    // Repeat recovery after success must not re-apply.
    let evd2 = rec_dir(&fx.evr, "roll-again");
    let again = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd2,
        "rec-roll-again",
        Some(RecoveryChoice::RollbackToPreimage),
        true,
        None,
    );
    let before = snapshot_tree(&fx.save);
    let _ = recover(&again);
    assert!(tree_unchanged(&fx.save, &before));
}

#[cfg(unix)]
#[test]
fn inspect_without_confirmed_choice_writes_nothing() {
    let fx = leftover_with_publish_fault("inspect", Fault::LogAppend);
    let before = snapshot_tree(&fx.save);
    let evd = rec_dir(&fx.evr, "inspect");
    let req = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-inspect",
        Some(RecoveryChoice::RollbackToPreimage),
        false,
        None,
    );
    let report = inspect_recovery(&req);
    assert!(report.inspect_only);
    assert!(!report.evidence_established);
    assert_eq!(report.original_operation_id.as_deref(), Some("op-p1-5"));
    assert!(report.written_paths.is_empty());
    assert!(tree_unchanged(&fx.save, &before));
    assert_leftover_intent_unchanged(&fx);
    assert_scope_reported(&report.scope_limited_paths);
}

#[cfg(unix)]
#[test]
fn t9b_interrupt_at_intent_and_done_then_reenter_without_replay() {
    let fx = leftover_with_publish_fault("t9b", Fault::LogAppend);
    let evd = rec_dir(&fx.evr, "t9b");

    // Interrupt after intent on the first file that still needs a write
    // (sheets/aria.yaml is already at the candidate, so rollback will write it).
    let hook_intent = |rel: &str, _save: &Path| {
        if rel == "sheets/aria.yaml" {
            return Err("inject interrupt after intent".into());
        }
        Ok(())
    };
    let mut req = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-t9b",
        Some(RecoveryChoice::RollbackToPreimage),
        true,
        None,
    );
    req.hooks.after_intent = Some(&hook_intent);
    let err = recover(&req).expect_err("intent interrupt");
    assert!(err.report.evidence_established);
    assert_eq!(err.report.original_operation_id.as_deref(), Some("op-p1-5"));
    assert!(
        err.report
            .pending_paths
            .contains(&"sheets/aria.yaml".to_string()),
        "intent without done should be visible: {:?}",
        err.report
    );
    assert!(err.report.written_paths.is_empty());
    assert_eq!(
        bytes_at(&fx.save, "sheets/aria.yaml"),
        bytes_at(&fx.candidate, "sheets/aria.yaml"),
        "interrupted intent must not have written the file"
    );

    // Re-entry without confirmation: determine progress, write nothing.
    let inspect = rec_req(
        &fx.root, &fx.save, &fx.evr, &evd, "rec-t9b", None, false, None,
    );
    let report = inspect_recovery(&inspect);
    assert!(report.determined);
    assert!(report.inspect_only);
    assert_eq!(report.original_operation_id.as_deref(), Some("op-p1-5"));
    let sheet = report
        .file_progress
        .iter()
        .find(|f| f.path == "sheets/aria.yaml")
        .expect("sheet progress");
    assert!(sheet.intent);
    assert!(!sheet.done);
    assert_eq!(
        bytes_at(&fx.save, "sheets/aria.yaml"),
        bytes_at(&fx.candidate, "sheets/aria.yaml")
    );

    // Interrupt again after the same file is marked done.
    let hook_done = |rel: &str, _save: &Path| {
        if rel == "sheets/aria.yaml" {
            return Err("inject interrupt after done".into());
        }
        Ok(())
    };
    let mut cont = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-t9b",
        Some(RecoveryChoice::RollbackToPreimage),
        true,
        None,
    );
    cont.hooks.after_done = Some(&hook_done);
    let err = recover(&cont).expect_err("done interrupt");
    assert!(
        err.report
            .already_done_paths
            .contains(&"sheets/aria.yaml".to_string())
            || err
                .report
                .written_paths
                .contains(&"sheets/aria.yaml".to_string())
    );
    assert_eq!(
        bytes_at(&fx.save, "sheets/aria.yaml"),
        bytes_at(&fx.backup, "sheets/aria.yaml")
    );
    // The extension should not have been written yet if we stopped at the
    // first file's done hook... unless already-done files before it ran.
    // sheets is the first replace target, so extension should still be v2
    // unless already-done recording of earlier files happened — none earlier.

    // Confirm again: resume without re-applying the done sheet, finish rollback.
    let sheet_before = bytes_at(&fx.save, "sheets/aria.yaml");
    let finish = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-t9b",
        Some(RecoveryChoice::RollbackToPreimage),
        true,
        None,
    );
    let report = recover(&finish).expect("resume after done");
    assert_eq!(report.protocol_state, State::Failed);
    assert!(!report
        .written_paths
        .contains(&"sheets/aria.yaml".to_string()));
    assert!(report
        .already_done_paths
        .contains(&"sheets/aria.yaml".to_string()));
    assert_eq!(bytes_at(&fx.save, "sheets/aria.yaml"), sheet_before);
    assert_eq!(bytes_at(&fx.save, "synthetic-ext.txt"), b"v1\n");
    assert_eq!(marker_count(&fx.save, "op-p1-5"), 0);
}

#[cfg(unix)]
#[test]
fn later_hand_edit_requires_human_merge_and_keeps_both_evidence_sets() {
    let fx = leftover_with_publish_fault("hand", Fault::LogAppend);
    fs::write(
        fx.save.join("sheets/aria.yaml"),
        "hand-edited neither side\n",
    )
    .unwrap();
    let evd = rec_dir(&fx.evr, "hand");
    let req = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-hand",
        Some(RecoveryChoice::RollbackToPreimage),
        true,
        None,
    );
    let err = recover(&req).expect_err("hand edit must stop");
    assert!(err.report.needs_human_merge);
    assert!(err.report.evidence_established);
    assert!(err.report.work_path.as_ref().is_some_and(|p| p.is_file()));
    assert!(err.report.snapshot_dir.as_ref().is_some_and(|p| p.is_dir()));
    assert_eq!(
        bytes_at(&fx.save, "sheets/aria.yaml"),
        b"hand-edited neither side\n",
        "must not overwrite the later hand edit"
    );
    // Published extension still at target; must not have been rolled back
    // after the conflict stop (sheets is first; if we stop at plan time,
    // no file is written).
    assert!(err.report.written_paths.is_empty());
    assert_eq!(bytes_at(&fx.save, "synthetic-ext.txt"), b"v2-registered\n");
}

#[cfg(unix)]
#[test]
fn later_log_appends_are_not_deleted() {
    let fx = leftover_with_publish_fault("logtail", Fault::IntegrityUpdate);
    // Logs already have the original marker; a later append must be kept.
    assert_eq!(marker_count(&fx.save, "op-p1-5"), 1);
    let mut rolls = fs::read(fx.save.join("rolls.log")).unwrap();
    rolls.extend_from_slice(b"later-hand-line\n");
    fs::write(fx.save.join("rolls.log"), &rolls).unwrap();
    for choice in [
        RecoveryChoice::RollbackToPreimage,
        RecoveryChoice::ContinueToTarget,
    ] {
        // Distinct evidence dirs / ids so the two choices do not share a work record.
        let evd_choice = rec_dir(&fx.evr, &format!("logtail-{}", choice.as_label()));
        let rec_id = format!("rec-log-{}", choice.as_label());
        let req = rec_req(
            &fx.root,
            &fx.save,
            &fx.evr,
            &evd_choice,
            &rec_id,
            Some(choice),
            true,
            None,
        );
        let err = recover(&req).expect_err("later logs");
        assert!(err.report.needs_human_merge, "{choice:?}");
        let text = String::from_utf8(fs::read(fx.save.join("rolls.log")).unwrap()).unwrap();
        assert!(
            text.contains("later-hand-line"),
            "later log append must not be deleted ({choice:?}): {text}"
        );
        assert_eq!(marker_count(&fx.save, "op-p1-5"), 1);
    }
}

#[cfg(unix)]
#[test]
fn corrupt_work_record_is_in_doubt_and_does_not_auto_resume() {
    let fx = leftover_with_publish_fault("corrupt", Fault::LogAppend);
    let evd = rec_dir(&fx.evr, "corrupt");
    let hook = |_save: &Path| Err("stop after evidence".into());
    let mut req = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-corrupt",
        Some(RecoveryChoice::ContinueToTarget),
        true,
        None,
    );
    req.hooks.after_evidence = Some(&hook);
    let err = recover(&req).expect_err("stopped after evidence");
    assert!(err.report.evidence_established);
    let work = err.report.work_path.as_ref().expect("work path");
    fs::write(work, "not-a-valid-work-record: [\n").unwrap();
    let before = snapshot_tree(&fx.save);
    let inspect = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-corrupt",
        Some(RecoveryChoice::ContinueToTarget),
        true,
        None,
    );
    let err = recover(&inspect).expect_err("corrupt work");
    assert!(err.report.in_doubt);
    assert!(err.report.needs_human_merge);
    assert!(
        !err.report.determined
            || err
                .report
                .error
                .as_ref()
                .is_some_and(|e| e.contains("无法判定") || e.contains("工作记录"))
    );
    assert!(err.report.written_paths.is_empty());
    assert!(tree_unchanged(&fx.save, &before));
}

#[cfg(unix)]
#[test]
fn unregistered_file_is_reported_and_not_silently_ignored() {
    let fx = leftover_with_publish_fault("scope", Fault::LogAppend);
    let evd = rec_dir(&fx.evr, "scope");
    let req = rec_req(
        &fx.root,
        &fx.save,
        &fx.evr,
        &evd,
        "rec-scope",
        Some(RecoveryChoice::ContinueToTarget),
        true,
        None,
    );
    let report = recover(&req).expect("continue");
    assert_scope_reported(&report.scope_limited_paths);
    assert!(
        report
            .error
            .as_ref()
            .is_some_and(|e| e.contains("范围限制") || e.contains("save-meta.yaml"))
            || !report.scope_limited_paths.is_empty()
    );
}
