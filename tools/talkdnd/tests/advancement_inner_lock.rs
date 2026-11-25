//! ADV-P1-3: bounded inner-lock acquire for the new advancement path.
//!
//! Synthetic fixtures only. These tests prove the new path's acquire, not
//! that a whole commit path is bounded. The old integrity `lock()` API is
//! not called against a live holder here (that would hang). Publish is not
//! wired; directory sync / `check_writable` is not required, so this file
//! should run on Windows as well as Unix.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use talkdnd::advancement_commit::{acquire_inner_integrity_lock, run_to_verified_candidate};
use talkdnd::advancement_inner_lock::{
    acquire_integrity, acquire_integrity_with, InnerLockOptions, DEFAULT_INNER_TIMEOUT,
    INTEGRITY_STAGE,
};
use talkdnd::advancement_lock as adv;

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "talkdnd-p3-{tag}-{}-{}",
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

fn write_synthetic_save(save: &Path) {
    fs::create_dir_all(save.join("sheets")).unwrap();
    fs::create_dir_all(save.join("Characters")).unwrap();
    fs::write(
        save.join("sheets/aria.yaml"),
        "version: 1\nid: aria\nname: Aria\nfile: Characters/Aria.md\nruleset: \"2024\"\n",
    )
    .unwrap();
    fs::write(save.join("Characters/Aria.md"), "# Aria\n").unwrap();
    fs::write(save.join("progress.yaml"), "version: 1\nxp:\n  aria: 0\n").unwrap();
    fs::write(save.join("integrity.yaml"), "version: 1\nfiles: {}\n").unwrap();
    fs::write(save.join("save-meta.yaml"), "schema: 10\ntool: talkdnd\n").unwrap();
}

fn init_protocol(root: &Path, save: &Path) {
    fs::create_dir_all(save).unwrap();
    match adv::init(root, save, &adv::InitOptions::new()) {
        Ok(_) => {}
        Err(err) if err.kind() == adv::ErrorKind::StatePublishedNotDurable => {}
        Err(err) => panic!("init protocol: {err}"),
    }
}

fn snapshot_targets(save: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for rel in [
        "sheets/aria.yaml",
        "Characters/Aria.md",
        "progress.yaml",
        "integrity.yaml",
        "save-meta.yaml",
    ] {
        let path = save.join(rel);
        out.push((path.clone(), fs::read(&path).unwrap()));
    }
    out
}

fn assert_targets_unchanged(before: &[(PathBuf, Vec<u8>)]) {
    for (path, bytes) in before {
        assert_eq!(
            fs::read(path).unwrap(),
            *bytes,
            "{} changed",
            path.display()
        );
    }
}

/// Names and sizes only. Do not read lock file contents while a handle may
/// hold a Windows mandatory lock.
fn state_identity(save: &Path) -> (u64, u64) {
    let lock_path = adv::lock_path(save);
    let state_path = adv::state_path(save);
    let lock_len = fs::metadata(&lock_path).map(|m| m.len()).unwrap_or(0);
    let state_len = fs::metadata(&state_path).map(|m| m.len()).unwrap_or(0);
    (lock_len, state_len)
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
        .env("ADV_INNER_LOCK_CHILD_ROLE", "hold")
        .env("ADV_INNER_LOCK_CHILD_DIR", save)
        .env("ADV_INNER_LOCK_CHILD_HOLD_MS", hold_ms.to_string())
        .env("ADV_INNER_LOCK_CHILD_READY", ready)
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

fn finish_child(child: Child) {
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "child failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("RESULT:OK"),
        "child printed no RESULT:OK: {stdout}"
    );
}

#[test]
fn default_inner_deadline_is_five_seconds_and_is_overridable() {
    let opts = InnerLockOptions::default();
    assert_eq!(opts.d_save, Duration::from_secs(5));
    assert_eq!(DEFAULT_INNER_TIMEOUT, Duration::from_secs(5));
    let integrity = opts
        .stages
        .iter()
        .find(|s| s.name == INTEGRITY_STAGE)
        .expect("integrity stage");
    assert_eq!(integrity.d_inner, Duration::from_secs(5));
    let budget = opts.budget();
    assert_eq!(budget.d_save, Duration::from_secs(5));
    assert_eq!(
        budget.d_inner(INTEGRITY_STAGE),
        Some(Duration::from_secs(5))
    );

    let overridden = InnerLockOptions::with_integrity_timeout(Duration::from_millis(200));
    assert_eq!(overridden.d_save, Duration::from_secs(5));
    assert_eq!(
        overridden.budget().d_inner(INTEGRITY_STAGE),
        Some(Duration::from_millis(200))
    );
}

#[test]
fn acquire_succeeds_when_the_inner_lock_is_free() {
    let root = root("free");
    let save = save_of(&root);
    write_synthetic_save(&save);
    let held = acquire_inner_integrity_lock(&save, &InnerLockOptions::default()).unwrap();
    assert!(!held.diagnostics.timed_out);
    assert_eq!(held.diagnostics.d_save, Duration::from_secs(5));
    assert!(held
        .diagnostics
        .d_inner
        .iter()
        .any(|(name, d)| name == INTEGRITY_STAGE && *d == Duration::from_secs(5)));
    drop(held);
    let held = acquire_integrity(&save, &InnerLockOptions::default()).unwrap();
    drop(held);
    let _ = fs::remove_dir_all(&root);
}

/// AC-3 T6b: another process holds the integrity lock; the new path fails
/// within its inner budget and reports 「内层锁」. Diagnostics record
/// `D_save` and each `D_inner`. This does not prove a whole commit path is
/// bounded.
#[test]
fn t6b_new_path_fails_within_budget_when_another_process_holds_the_inner_lock() {
    let root = root("t6b");
    let save = save_of(&root);
    write_synthetic_save(&save);
    init_protocol(&root, &save);
    let before = snapshot_targets(&save);
    let state_before = state_identity(&save);

    let ready = root.join("holder-ready");
    let mut holder = spawn_holder(&save, 4000, &ready);
    wait_ready(&ready, &mut holder, Duration::from_secs(5));

    let d_inner = Duration::from_millis(300);
    let mut opts = InnerLockOptions::with_integrity_timeout(d_inner);
    opts.d_save = Duration::from_secs(7);
    let start = Instant::now();
    let err = acquire_inner_integrity_lock(&save, &opts).expect_err("should time out");
    let elapsed = start.elapsed();

    assert!(
        err.to_string().contains("内层锁"),
        "timeout must report 内层锁: {}",
        err
    );
    assert!(err.diagnostics.timed_out);
    assert_eq!(err.diagnostics.d_save, Duration::from_secs(7));
    assert_eq!(
        err.diagnostics.d_inner,
        vec![(INTEGRITY_STAGE.to_string(), d_inner)]
    );
    assert!(
        elapsed >= d_inner,
        "must wait the inner budget, waited {elapsed:?} d_inner {d_inner:?}"
    );
    assert!(
        elapsed < d_inner + Duration::from_secs(2),
        "must fail inside the inner budget, not hang and not wait D_save: {elapsed:?}"
    );
    // D_save is recorded, not used as the wait for this acquire.
    assert!(
        elapsed < opts.d_save,
        "must not wait D_save on an inner acquire: {elapsed:?} d_save {:?}",
        opts.d_save
    );

    assert_targets_unchanged(&before);
    assert_eq!(
        state_identity(&save),
        state_before,
        "SAVE/advancement lock and state must be unchanged (size only; contents not read while a lock may be held)"
    );

    finish_child(holder);
    // No half-held inner lock left by the waiter.
    let held = acquire_inner_integrity_lock(&save, &opts).unwrap();
    drop(held);
    assert_targets_unchanged(&before);
    let _ = fs::remove_dir_all(&root);
}

/// AC-5: a retry recomputes the budget from scratch and still validates.
#[test]
fn retry_after_timeout_uses_a_fresh_budget_and_still_validates() {
    let root = root("retry");
    let save = save_of(&root);
    write_synthetic_save(&save);

    let ready = root.join("holder-ready");
    let mut holder = spawn_holder(&save, 5000, &ready);
    wait_ready(&ready, &mut holder, Duration::from_secs(5));

    let d_inner = Duration::from_millis(250);
    let opts = InnerLockOptions::with_integrity_timeout(d_inner);
    let validations = AtomicUsize::new(0);
    let validate = |_: &Path| {
        validations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };

    let start = Instant::now();
    let err = acquire_integrity_with(&save, &opts, validate).expect_err("first attempt times out");
    let first = start.elapsed();
    assert!(err.to_string().contains("内层锁"), "{}", err);
    assert_eq!(validations.load(Ordering::SeqCst), 1);
    assert!(first >= d_inner, "first wait {first:?}");
    assert!(
        first < d_inner + Duration::from_secs(2),
        "first hang {first:?}"
    );

    // Second attempt, same options, holder still present: must wait a fresh
    // full D_inner, not leftover (~0) from the first attempt.
    let start = Instant::now();
    let err = acquire_integrity_with(&save, &opts, validate).expect_err("second attempt times out");
    let second = start.elapsed();
    assert!(err.to_string().contains("内层锁"), "{}", err);
    assert_eq!(validations.load(Ordering::SeqCst), 2);
    assert!(
        second >= d_inner,
        "retry must use a fresh budget, waited {second:?} after first {first:?}"
    );
    assert!(
        second < d_inner + Duration::from_secs(2),
        "retry must not hang: {second:?}"
    );

    // A third attempt still validates; if validation fails it must not skip
    // to "already timed out" or try to acquire anyway.
    let err = acquire_integrity_with(&save, &opts, |_: &Path| {
        validations.fetch_add(1, Ordering::SeqCst);
        Err("forced validation failure after prior timeout".into())
    })
    .expect_err("validation must run and fail");
    assert_eq!(validations.load(Ordering::SeqCst), 3);
    assert!(
        err.to_string().contains("校验失败"),
        "retry must honor a fresh validation failure: {}",
        err
    );
    assert!(err.to_string().contains("内层锁"), "{}", err);
    assert!(
        !err.diagnostics.timed_out,
        "a validation failure is not an inner-lock timeout: {:?}",
        err.diagnostics
    );

    finish_child(holder);
    let _ = fs::remove_dir_all(&root);
}

fn run_inner_source_body(src: &str) -> String {
    // Normalize CRLF checkouts (Windows CI) before scanning, matching the
    // integrity lock() source-scan.
    let src = src.replace("\r\n", "\n");
    assert!(
        src.contains("pub fn acquire_inner_integrity_lock"),
        "the new-path call site must exist"
    );
    let start = src.find("fn run_inner").expect("run_inner still exists");
    let rest = &src[start + 1..];
    let end = rest
        .find("\nfn ")
        .or_else(|| rest.find("\npub fn "))
        .expect("function after run_inner");
    rest[..end].to_string()
}

fn assert_run_inner_is_not_wired(body: &str) {
    assert!(
        !body.contains("acquire_inner_integrity_lock")
            && !body.contains("lock_with_timeout")
            && !body.contains("acquire_integrity"),
        "run_inner must not acquire the inner lock or wire publish: {body}"
    );
    assert!(
        body.contains("published: false"),
        "run_inner must still stop unpublished: {body}"
    );
}

#[test]
fn run_to_verified_candidate_does_not_wire_inner_lock_or_publish() {
    // AC-4: this block exposes a call site but does not hook it into the
    // D orchestrator, and does not implement publish.
    let src = include_str!("../src/advancement_commit.rs");
    let body = run_inner_source_body(src);
    assert_run_inner_is_not_wired(&body);

    let crlf = src.replace("\r\n", "\n").replace('\n', "\r\n");
    assert!(
        crlf.contains("\r\n"),
        "CRLF view must actually contain CRLF"
    );
    let body_crlf = run_inner_source_body(&crlf);
    assert_eq!(body, body_crlf);
    assert_run_inner_is_not_wired(&body_crlf);

    // Keep the type in the file so a later accidental hook is easier to see.
    let _ = run_to_verified_candidate;
}

/// Not a test: child body that holds the old blocking integrity lock.
#[test]
#[ignore]
fn child_helper() {
    let Ok(role) = std::env::var("ADV_INNER_LOCK_CHILD_ROLE") else {
        return;
    };
    assert_eq!(role, "hold");
    let save = PathBuf::from(std::env::var("ADV_INNER_LOCK_CHILD_DIR").unwrap());
    let hold_ms: u64 = std::env::var("ADV_INNER_LOCK_CHILD_HOLD_MS")
        .unwrap()
        .parse()
        .unwrap();
    let ready = PathBuf::from(std::env::var("ADV_INNER_LOCK_CHILD_READY").unwrap());
    let _held =
        combat_sim::integrity::lock(&save).expect("holder takes the blocking integrity lock");
    fs::write(&ready, b"ready").unwrap();
    std::thread::sleep(Duration::from_millis(hold_ms));
    println!("RESULT:OK");
}
