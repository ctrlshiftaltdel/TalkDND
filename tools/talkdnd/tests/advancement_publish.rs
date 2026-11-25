//! ADV-P1-4: publish, log append, integrity update, and submit idempotency
//! on synthetic fixtures. No real saves, books, or secrets.
//!
//! Directory-sync-dependent runs are Unix-only. Windows stays fail-closed
//! (unsupported directory sync). Real process-kill, power-loss, directory
//! durability, and network FS are NOT_RUN / UNKNOWN here, not a pass.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use talkdnd::advancement_commit::{
    binding_preview_id, protected_target_bytes, run_to_complete, run_to_verified_candidate,
    targets_unchanged, Fault, InterruptStage, PublishHooks, Request,
};
use talkdnd::advancement_inner_lock::InnerLockOptions;
use talkdnd::advancement_lock as adv;
use talkdnd::advancement_lock::{CompletionEvidence, Stage, State};

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "talkdnd-p4-{tag}-{}-{}",
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
        b"version: 1\nid: aria\nname: Aria\nfile: Characters/Aria.md\nruleset: \"2024\"\nnotes:\n  - leveled\n".to_vec(),
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
    m
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
        operation_id: "op-p1-4",
        evidence_root,
        evidence_dir,
        backup_parent,
        candidate_parent,
        overlays,
        fault,
        extra_targets: Vec::new(),
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
    let mut n = 0;
    for name in ["rolls.log", "Sessions/2030-01-02.md"] {
        if let Ok(text) = fs::read_to_string(save.join(name)) {
            n += text.matches(&marker).count();
        }
    }
    n
}

fn count_line(text: &str, needle: &str) -> usize {
    text.lines().filter(|l| l.contains(needle)).count()
}

#[cfg(not(unix))]
#[test]
fn windows_publish_fails_closed_without_writing_targets() {
    let root = root("win-pub");
    let save = save_of(&root);
    write_synthetic_save(&save);
    init_protocol(&root, &save);
    write_baseline_manifest(&save);
    let before = protected_target_bytes(&save).unwrap();
    let rolls = fs::read(save.join("rolls.log")).unwrap();
    let (evr, evd, bp, cp) = parents(&root);
    let preview = binding_preview_id(&save, &[]).unwrap();
    let mut req = request(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), None);
    req.preview_id = Some(preview);
    let err = run_to_complete(&req).expect_err("Windows cannot sync directories");
    assert_eq!(err.report.recovery.interrupt_stage, InterruptStage::Admit);
    assert!(!err.report.published);
    assert!(err.report.published_paths.is_empty());
    let after = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&before, &after));
    assert_eq!(fs::read(save.join("rolls.log")).unwrap(), rolls);
}

#[cfg(unix)]
fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let root = root(tag);
    let save = save_of(&root);
    write_synthetic_save(&save);
    init_protocol(&root, &save);
    write_baseline_manifest(&save);
    let (evr, evd, bp, cp) = parents(&root);
    (root, save, evr, evd, bp, cp)
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn complete_req<'a>(
    root: &'a Path,
    save: &'a Path,
    evr: &'a Path,
    evd: &'a Path,
    bp: &'a Path,
    cp: &'a Path,
    overlays: BTreeMap<String, Vec<u8>>,
    extra: &'a [String],
) -> Request<'a> {
    let preview = binding_preview_id(save, extra).unwrap();
    let mut req = request(root, save, evr, evd, bp, cp, overlays, None);
    req.preview_id = Some(preview);
    req.extra_targets = extra.to_vec();
    req
}

#[cfg(unix)]
#[test]
fn happy_path_publishes_once_in_order_and_completes() {
    let (root, save, evr, evd, bp, cp) = setup("happy");
    let overlays = growth_overlays();
    let req = complete_req(&root, &save, &evr, &evd, &bp, &cp, overlays.clone(), &[]);
    let report = run_to_complete(&req).expect("complete");
    assert_eq!(report.protocol_state, State::Complete);
    assert_eq!(report.stage, Stage::Done);
    assert!(report.published);
    assert!(report.logs_recorded);
    assert!(report.integrity_recorded);
    assert!(report.pending_paths.is_empty());
    assert!(!report.overwrite_cannot_be_ruled_out);
    assert_eq!(
        report.published_paths,
        vec![
            "sheets/aria.yaml".to_string(),
            "combat/party-state.yaml".to_string(),
            "Characters/Aria.md".to_string(),
            "progress.yaml".to_string(),
            "rolls.log".to_string(),
            "Sessions/2030-01-02.md".to_string(),
            "integrity.yaml".to_string(),
        ]
    );
    assert_eq!(
        fs::read(save.join("sheets/aria.yaml")).unwrap(),
        overlays["sheets/aria.yaml"]
    );
    assert_eq!(
        fs::read(save.join("progress.yaml")).unwrap(),
        overlays["progress.yaml"]
    );
    assert!(fs::read_to_string(save.join("progress.yaml"))
        .unwrap()
        .contains("aria: 300"));
    assert_eq!(marker_count(&save, "op-p1-4"), 2);
    assert_eq!(
        count_line(
            &fs::read_to_string(save.join("rolls.log")).unwrap(),
            "<!-- talkdnd-op:op-p1-4 -->"
        ),
        1
    );
    let lock = open_fast(&root, &save);
    assert_eq!(lock.state().state, State::Complete);
    assert!(lock.state().pending.is_empty());
    lock.check_ordinary_write().unwrap();
}

#[cfg(unix)]
#[test]
fn intent_write_failure_does_not_publish_and_is_failed() {
    let (root, save, evr, evd, bp, cp) = setup("intent-fail");
    let before = protected_target_bytes(&save).unwrap();
    let rolls = fs::read(save.join("rolls.log")).unwrap();
    let preview = binding_preview_id(&save, &[]).unwrap();
    let mut req = request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        growth_overlays(),
        Some(Fault::IntentWrite),
    );
    req.preview_id = Some(preview);
    let err = run_to_complete(&req).expect_err("intent fault");
    assert_eq!(err.report.protocol_state, State::Failed);
    assert_eq!(err.report.stage, Stage::StoppedBeforeWrite);
    assert!(!err.report.published);
    assert!(err.report.published_paths.is_empty());
    assert!(err.report.pending_paths.is_empty());
    assert!(err.report.error.as_ref().unwrap().contains("intent"));
    assert!(targets_unchanged(
        &before,
        &protected_target_bytes(&save).unwrap()
    ));
    assert_eq!(fs::read(save.join("rolls.log")).unwrap(), rolls);
}

#[cfg(unix)]
#[test]
fn publish_write_failure_after_intent_is_in_doubt_and_does_not_retry() {
    let (root, save, evr, evd, bp, cp) = setup("write-fail");
    let before = protected_target_bytes(&save).unwrap();
    let preview = binding_preview_id(&save, &[]).unwrap();
    let mut req = request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        growth_overlays(),
        Some(Fault::PublishWrite),
    );
    req.preview_id = Some(preview);
    let err = run_to_complete(&req).expect_err("write fault");
    assert_eq!(err.report.protocol_state, State::InDoubt);
    assert_eq!(err.report.recovery.interrupt_stage, InterruptStage::Publish);
    assert!(!err.report.published_paths.is_empty() || !err.report.pending_paths.is_empty());
    assert!(err.report.error.as_ref().unwrap().contains("不自动重试"));
    // The first target was intended but not successfully marked published.
    // Its live bytes must still equal the preimage (no completed publish).
    assert_eq!(
        fs::read(save.join("sheets/aria.yaml")).unwrap(),
        before["sheets/aria.yaml"]
    );
    assert_eq!(
        fs::read(save.join("progress.yaml")).unwrap(),
        before["progress.yaml"]
    );

    let lock = open_fast(&root, &save);
    assert_eq!(lock.state().state, State::InDoubt);
    assert!(lock.check_ordinary_write().is_err());
    drop(lock);

    // A new orchestrator run must not auto-resume or retry publish.
    let evd2 = evr.join("retry-no-resume");
    let mut retry = request(&root, &save, &evr, &evd2, &bp, &cp, growth_overlays(), None);
    retry.preview_id = Some(binding_preview_id(&save, &[]).unwrap());
    retry.operation_id = "op-p1-4-b";
    retry.instance_tag = "b";
    let again = run_to_complete(&retry).expect_err("must not auto-resume in_doubt");
    assert_eq!(again.report.recovery.interrupt_stage, InterruptStage::Admit);
    assert_eq!(
        fs::read(save.join("sheets/aria.yaml")).unwrap(),
        before["sheets/aria.yaml"]
    );
}

#[cfg(unix)]
#[test]
fn leftover_in_doubt_refuses_ordinary_writes_and_is_not_auto_resumed() {
    // T4a/T4e leftover in_doubt (synthetic; not a process-kill or power-loss).
    let (root, save, evr, evd, bp, cp) = setup("t4e-indoubt");
    let before = protected_target_bytes(&save).unwrap();
    let preview = binding_preview_id(&save, &[]).unwrap();
    let mut req = request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        growth_overlays(),
        Some(Fault::PublishSync),
    );
    req.preview_id = Some(preview);
    let err = run_to_complete(&req).expect_err("sync fault");
    assert_eq!(err.report.protocol_state, State::InDoubt);

    let lock = open_fast(&root, &save);
    let ordinary = lock.check_ordinary_write();
    assert!(ordinary.is_err(), "in_doubt must refuse ordinary writes");
    assert!(
        ordinary.unwrap_err().detail().contains("in_doubt"),
        "error should name in_doubt"
    );
    drop(lock);

    let evd2 = evr.join("no-resume");
    let mut retry = complete_req(&root, &save, &evr, &evd2, &bp, &cp, growth_overlays(), &[]);
    retry.operation_id = "op-new";
    retry.instance_tag = "new";
    let again = run_to_complete(&retry).expect_err("must not auto-resume");
    assert_eq!(again.report.recovery.interrupt_stage, InterruptStage::Admit);
    assert!(targets_unchanged(
        &before,
        &protected_target_bytes(&save).unwrap()
    ));
    let lock = open_fast(&root, &save);
    assert_eq!(lock.state().state, State::InDoubt);
}

#[cfg(unix)]
#[test]
fn recheck_mismatch_stops_without_publishing_that_file_t7a() {
    let (root, save, evr, evd, bp, cp) = setup("t7a");
    let injected = b"INJECTED-T7A-SHOULD-NEVER-APPEAR-IN-ERRORS";
    let hook = |rel: &str, save: &Path| {
        if rel == "sheets/aria.yaml" {
            fs::write(save.join("sheets/aria.yaml"), injected).unwrap();
        }
    };
    let preview = binding_preview_id(&save, &[]).unwrap();
    let mut req = request(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), None);
    req.preview_id = Some(preview);
    req.hooks.after_intent = Some(&hook);
    let err = run_to_complete(&req).expect_err("recheck must fail");
    let message = err.report.error.as_ref().unwrap().clone();
    assert!(message.contains("复核"), "{message}");
    assert!(
        !message.contains("INJECTED-T7A-SHOULD-NEVER-APPEAR-IN-ERRORS"),
        "T7a must not print injected bytes: {message}"
    );
    assert!(
        message.contains("已观察前像") || message.contains("sha256:"),
        "{message}"
    );
    // The injected file was written by the hook, not by publish. The
    // orchestrator must have stopped without marking it published.
    assert!(!err
        .report
        .published_paths
        .contains(&"sheets/aria.yaml".to_string()));
    assert_eq!(err.report.protocol_state, State::InDoubt);
}

#[cfg(unix)]
#[test]
fn t7b_reports_overwrite_cannot_be_ruled_out_without_injected_bytes() {
    let (root, save, evr, evd, bp, cp) = setup("t7b");
    let injected = b"INJECTED-T7B-VALUE-MUST-NOT-BE-PRINTED";
    let hook = |rel: &str, save: &Path| {
        if rel == "sheets/aria.yaml" {
            fs::write(save.join("sheets/aria.yaml"), injected).unwrap();
        }
    };
    let preview = binding_preview_id(&save, &[]).unwrap();
    let mut req = request(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), None);
    req.preview_id = Some(preview);
    req.hooks.after_recheck_before_publish = Some(&hook);
    let report = run_to_complete(&req).expect("rename may still complete");
    assert!(
        report.overwrite_cannot_be_ruled_out,
        "T7b must honestly report that overwrite cannot be ruled out"
    );
    assert!(!report.observed_preimage.is_empty());
    assert!(!report.observed_target.is_empty());
    let note = &report.recovery.note;
    assert!(note.contains("无法排除"), "{note}");
    assert!(
        note.contains("不宣称检测成功"),
        "must not claim detection succeeded: {note}"
    );
    assert!(
        !note.contains("已检测到覆盖") && !note.contains("检测已成功"),
        "{note}"
    );
    assert!(!note.contains("INJECTED-T7B-VALUE-MUST-NOT-BE-PRINTED"));
    if let Some(err) = &report.error {
        assert!(!err.contains("INJECTED-T7B-VALUE-MUST-NOT-BE-PRINTED"));
    }
    // The published bytes are the candidate, which may have overwritten the
    // injection; we do not claim to have detected or preserved it.
    assert_eq!(
        fs::read(save.join("sheets/aria.yaml")).unwrap(),
        growth_overlays()["sheets/aria.yaml"]
    );
}

#[cfg(unix)]
#[test]
fn t2a_state_replacement_during_publish_stops_and_does_not_continue() {
    let (root, save, evr, evd, bp, cp) = setup("t2a-pub");
    let hook = |_rel: &str, save_dir: &Path| {
        let path = adv::state_path(save_dir);
        let mut record: adv::StateRecord =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record.revision = record.revision.saturating_add(9);
        adv::reseal(&mut record).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    };
    let preview = binding_preview_id(&save, &[]).unwrap();
    let mut req = request(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), None);
    req.preview_id = Some(preview);
    req.hooks.after_intent = Some(&hook);
    let err = run_to_complete(&req).expect_err("replaced state");
    assert_ne!(err.report.protocol_state, State::Complete);
    // After the first file may already have been written when record_published
    // failed, the result is unclear: in_doubt, never complete or a forged failed.
    assert_ne!(err.report.protocol_state, State::Failed);
    let evd2 = evr.join("t2a-retry");
    let mut retry = complete_req(&root, &save, &evr, &evd2, &bp, &cp, growth_overlays(), &[]);
    retry.operation_id = "op-t2a-2";
    retry.instance_tag = "t2a2";
    // If the first stop left in_doubt or a poisoned/replaced state, a new
    // run must not silently continue publishing.
    let _ = run_to_complete(&retry);
    let lock = open_fast(&root, &save);
    assert_ne!(lock.state().state, State::Complete);
}

#[cfg(unix)]
#[test]
fn t8_preview_mismatch_rejects_before_target_writes() {
    let (root, save, evr, evd, bp, cp) = setup("t8");
    let before = protected_target_bytes(&save).unwrap();
    let rolls = fs::read(save.join("rolls.log")).unwrap();
    let mut req = complete_req(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), &[]);
    req.preview_id = Some("sha256:deadbeef".to_string());
    let err = run_to_complete(&req).expect_err("mismatch");
    let message = err.report.error.as_ref().unwrap().clone();
    assert!(message.contains("绑定变化，原因未知"), "{message}");
    assert!(!message.contains("sheets/"));
    assert!(!message.contains("progress.yaml"));
    assert!(!message.contains("aria"));
    assert_eq!(err.report.protocol_state, State::Ready);
    assert!(!err.report.published);
    assert!(targets_unchanged(
        &before,
        &protected_target_bytes(&save).unwrap()
    ));
    assert_eq!(fs::read(save.join("rolls.log")).unwrap(), rolls);
    // The lock must not have become active for a missing/mismatched binding.
    let lock = open_fast(&root, &save);
    assert_eq!(lock.state().state, State::Ready);
}

#[cfg(unix)]
#[test]
fn t8_missing_preview_id_rejects_before_target_writes() {
    let (root, save, evr, evd, bp, cp) = setup("t8-missing");
    let before = protected_target_bytes(&save).unwrap();
    let req = request(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), None);
    let err = run_to_complete(&req).expect_err("missing preview");
    assert!(err
        .report
        .error
        .as_ref()
        .unwrap()
        .contains("未提供 preview_id"));
    assert!(targets_unchanged(
        &before,
        &protected_target_bytes(&save).unwrap()
    ));
}

#[cfg(unix)]
#[test]
fn repeated_submit_does_not_reapply_xp_or_logs() {
    let (root, save, evr, evd, bp, cp) = setup("idempotent");
    let req = complete_req(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), &[]);
    let first = run_to_complete(&req).expect("first complete");
    assert_eq!(first.protocol_state, State::Complete);
    let after_first = protected_target_bytes(&save).unwrap();
    let rolls_first = fs::read_to_string(save.join("rolls.log")).unwrap();
    let session_first = fs::read_to_string(save.join("Sessions/2030-01-02.md")).unwrap();
    let xp = fs::read_to_string(save.join("progress.yaml")).unwrap();
    assert!(xp.contains("aria: 300"));
    assert_eq!(marker_count(&save, "op-p1-4"), 2);

    let evd2 = evr.join("2030-01-02-01-retry");
    let mut second = complete_req(&root, &save, &evr, &evd2, &bp, &cp, growth_overlays(), &[]);
    second.instance_tag = "retry";
    let report = run_to_complete(&second).expect("second complete");
    assert_eq!(report.protocol_state, State::Complete);
    let after_second = protected_target_bytes(&save).unwrap();
    assert!(targets_unchanged(&after_first, &after_second));
    assert_eq!(fs::read_to_string(save.join("progress.yaml")).unwrap(), xp);
    assert_eq!(
        fs::read_to_string(save.join("Characters/Aria.md")).unwrap(),
        "# Aria\n\nLevel 2\n"
    );
    assert_eq!(marker_count(&save, "op-p1-4"), 2);
    assert_eq!(
        fs::read_to_string(save.join("rolls.log")).unwrap(),
        rolls_first
    );
    assert_eq!(
        fs::read_to_string(save.join("Sessions/2030-01-02.md")).unwrap(),
        session_first
    );
}

#[cfg(unix)]
#[test]
fn trailing_log_anomaly_stops_instead_of_replaying() {
    let (root, save, evr, evd, bp, cp) = setup("log-tail");
    fs::write(save.join("rolls.log"), "1d20=7").unwrap(); // no trailing newline
    write_baseline_manifest(&save);
    let req = complete_req(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), &[]);
    let err = run_to_complete(&req).expect_err("trailing anomaly");
    let message = err.report.error.as_ref().unwrap().clone();
    assert!(
        message.contains("日志尾部异常") || message.contains("无法证明"),
        "{message}"
    );
    assert!(
        message.contains("日志可能已追加") || message.contains("停止"),
        "{message}"
    );
    assert!(!fs::read_to_string(save.join("rolls.log"))
        .unwrap()
        .contains("<!-- talkdnd-op:op-p1-4 -->"));
    // Replace targets were published first, so this is in_doubt, not failed.
    assert_eq!(err.report.protocol_state, State::InDoubt);
}

#[cfg(unix)]
#[test]
fn integrity_update_preserves_dirty_and_accepted_marks() {
    let (root, save, evr, evd, bp, cp) = setup("dirty");
    fs::write(
        save.join("save-meta.yaml"),
        "schema: 10\ntool: talkdnd\nnote: hand-edit\n",
    )
    .unwrap();
    let dirty = combat_sim::integrity::changes(&save).unwrap();
    assert!(
        dirty.iter().any(|c| c.path() == "save-meta.yaml"),
        "precondition: hand edit is dirty: {dirty:?}"
    );
    let accepted_before = combat_sim::integrity::load(&save)
        .unwrap()
        .unwrap()
        .accepted
        .clone();
    assert!(!accepted_before.is_empty());

    let req = complete_req(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), &[]);
    let report = run_to_complete(&req).expect("complete");
    assert_eq!(report.protocol_state, State::Complete);
    let dirty_after = combat_sim::integrity::changes(&save).unwrap();
    assert!(
        dirty_after.iter().any(|c| c.path() == "save-meta.yaml"),
        "hand-edit dirty flag must not be washed away: {dirty_after:?}"
    );
    let manifest = combat_sim::integrity::load(&save).unwrap().unwrap();
    assert_eq!(manifest.accepted, accepted_before);
    assert_eq!(
        fs::read_to_string(save.join("save-meta.yaml")).unwrap(),
        "schema: 10\ntool: talkdnd\nnote: hand-edit\n"
    );
}

#[cfg(unix)]
#[test]
fn synthetic_extension_target_is_not_skipped_and_has_failure_coverage() {
    let (root, save, evr, evd, bp, cp) = setup("ext-ok");
    fs::write(save.join("synthetic-ext.txt"), "v1\n").unwrap();
    write_baseline_manifest(&save);
    let mut overlays = growth_overlays();
    overlays.insert("synthetic-ext.txt".to_string(), b"v2-registered\n".to_vec());
    let extra = vec!["synthetic-ext.txt".to_string()];
    let req = complete_req(&root, &save, &evr, &evd, &bp, &cp, overlays, &extra);
    let report = run_to_complete(&req).expect("complete with extension");
    assert!(
        report
            .published_paths
            .contains(&"synthetic-ext.txt".to_string()),
        "extension must be published, not skipped: {:?}",
        report.published_paths
    );
    assert_eq!(
        fs::read(save.join("synthetic-ext.txt")).unwrap(),
        b"v2-registered\n"
    );
    let lock = open_fast(&root, &save);
    let entry = lock
        .state()
        .published
        .iter()
        .find(|p| p.path == "synthetic-ext.txt")
        .expect("intent/digest record");
    assert!(entry.before.starts_with("sha256:"));
    assert!(entry.after.starts_with("sha256:"));
    assert_ne!(entry.before, entry.after);
    assert!(entry.done);
    drop(lock);

    // Failure coverage on a fresh fixture: a recheck mismatch on the
    // extension must stop and record the target, not skip it.
    let (root, save, evr, evd, bp, cp) = setup("ext-fail");
    fs::write(save.join("synthetic-ext.txt"), "v1\n").unwrap();
    write_baseline_manifest(&save);
    let mut overlays = growth_overlays();
    overlays.insert("synthetic-ext.txt".to_string(), b"v2-registered\n".to_vec());
    let extra = vec!["synthetic-ext.txt".to_string()];
    let hook = |rel: &str, save: &Path| {
        if rel == "synthetic-ext.txt" {
            fs::write(save.join("synthetic-ext.txt"), b"hook-changed\n").unwrap();
        }
    };
    let mut req = complete_req(&root, &save, &evr, &evd, &bp, &cp, overlays, &extra);
    req.hooks.after_intent = Some(&hook);
    let err = run_to_complete(&req).expect_err("extension recheck");
    assert_eq!(err.report.protocol_state, State::InDoubt);
    assert!(
        err.report
            .pending_paths
            .contains(&"synthetic-ext.txt".to_string())
            || err
                .report
                .error
                .as_ref()
                .unwrap()
                .contains("synthetic-ext.txt"),
        "extension failure must be recorded, not skipped: {:?}",
        err.report
    );
    assert_eq!(
        fs::read(save.join("synthetic-ext.txt")).unwrap(),
        b"hook-changed\n"
    );
    assert_ne!(
        fs::read(save.join("sheets/aria.yaml")).unwrap(),
        b"version: 1\nid: aria\nname: Aria\nfile: Characters/Aria.md\nruleset: \"2024\"\n"
    );
}

#[cfg(unix)]
#[test]
fn complete_is_refused_for_forged_or_incomplete_evidence() {
    let (root, save, evr, evd, bp, cp) = setup("forged");
    let report = run_to_verified_candidate(&request(
        &root,
        &save,
        &evr,
        &evd,
        &bp,
        &cp,
        growth_overlays(),
        None,
    ))
    .expect("candidate");
    assert!(report.is_verified_unpublished());
    let mut lock = open_fast(&root, &save);
    let err = lock
        .finish_complete(&CompletionEvidence {
            candidate_matches: false,
            logs_recorded: false,
            integrity_recorded: false,
        })
        .expect_err("forged complete");
    assert!(err.detail().contains("in_doubt") || err.to_string().contains("in_doubt"));
    assert_eq!(lock.state().state, State::Active);
    // A partial-looking failed is also refused once intent exists.
    lock.record_intent("sheets/aria.yaml", "sha256:a", "sha256:b")
        .unwrap();
    let err = lock
        .finish_failed("should not accept failed after intent")
        .expect_err("failed after intent");
    assert!(err.detail().contains("in_doubt") || err.to_string().contains("in_doubt"));
    assert_eq!(lock.state().state, State::Active);
}

#[cfg(unix)]
#[test]
fn t6b_inner_lock_contention_stops_at_integrity_and_is_in_doubt() {
    let (root, save, evr, evd, bp, cp) = setup("t6b");
    let ready = root.join("holder-ready");
    let mut holder = spawn_holder(&save, 4000, &ready);
    wait_ready(&ready, &mut holder, Duration::from_secs(5));

    let preview = binding_preview_id(&save, &[]).unwrap();
    let mut req = request(&root, &save, &evr, &evd, &bp, &cp, growth_overlays(), None);
    req.preview_id = Some(preview);
    req.inner_lock = InnerLockOptions::with_integrity_timeout(Duration::from_millis(400));
    let started = Instant::now();
    let err = run_to_complete(&req).expect_err("inner lock timeout");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "must not hang on the unbounded lock: {elapsed:?}"
    );
    let message = err.report.error.as_ref().unwrap().clone();
    assert!(message.contains("内层锁"), "{message}");
    assert!(
        message.contains("不承诺有界") || message.contains("仍不承诺有界"),
        "{message}"
    );
    assert_eq!(err.report.protocol_state, State::InDoubt);
    assert!(!err.report.published_paths.is_empty());
    assert!(!err.report.integrity_recorded);
    finish_child(holder);
}

#[test]
fn run_to_complete_uses_bounded_inner_lock_only() {
    let src = include_str!("../src/advancement_commit.rs").replace("\r\n", "\n");
    assert!(src.contains("acquire_inner_integrity_lock"));
    // The new publish path must not call the old unbounded lock() API.
    // Comments mentioning the old API are fine; a call is not.
    let publish = {
        let start = src.find("fn publish_integrity").expect("publish_integrity");
        let rest = &src[start..];
        let end = rest.find("\nfn ").unwrap_or(rest.len());
        rest[..end].to_string()
    };
    assert!(publish.contains("acquire_inner_integrity_lock"));
    assert!(
        !publish.contains("integrity::lock(") && !publish.contains("integrity::after_write("),
        "publish_integrity must use the bounded API and planned bytes: {publish}"
    );
}

/// Not a test: child body that holds the old blocking integrity lock.
#[test]
#[ignore]
fn child_helper() {
    let Ok(role) = std::env::var("ADV_PUBLISH_CHILD_ROLE") else {
        return;
    };
    assert_eq!(role, "hold");
    let save = PathBuf::from(std::env::var("ADV_PUBLISH_CHILD_DIR").unwrap());
    let hold_ms: u64 = std::env::var("ADV_PUBLISH_CHILD_HOLD_MS")
        .unwrap()
        .parse()
        .unwrap();
    let ready = PathBuf::from(std::env::var("ADV_PUBLISH_CHILD_READY").unwrap());
    let _held =
        combat_sim::integrity::lock(&save).expect("holder takes the blocking integrity lock");
    fs::write(&ready, b"ready").unwrap();
    std::thread::sleep(Duration::from_millis(hold_ms));
    println!("RESULT:OK");
}

fn spawn_holder(save: &Path, hold_ms: u64, ready: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "child_helper",
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env("ADV_PUBLISH_CHILD_ROLE", "hold")
        .env("ADV_PUBLISH_CHILD_DIR", save)
        .env("ADV_PUBLISH_CHILD_HOLD_MS", hold_ms.to_string())
        .env("ADV_PUBLISH_CHILD_READY", ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_ready(ready: &Path, child: &mut Child, limit: Duration) {
    let start = Instant::now();
    while !ready.is_file() {
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!("holder did not become ready in {limit:?}");
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("holder exited before ready: {status}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn finish_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}
