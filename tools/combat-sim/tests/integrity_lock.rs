//! ADV-P1-3: bounded integrity-lock acquire. These tests cover the new
//! `lock_with_timeout` path only. The existing blocking `lock()` API is
//! deliberately not called against a live holder here: that would hang if
//! the unbounded semantics are intact (AC-2 is proven by source inspection
//! in `integrity.rs` and by keeping the existing integrity tests).
//!
//! Contention uses a real child process. Same-process threads are not a
//! valid proof for flock/fcntl locks. T6b does not prove that a whole
//! advancement commit path is bounded, and publish is not wired.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use combat_sim::integrity::{self, lock_with_timeout};

fn save_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "integrity-lock-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
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
        .env("INTEGRITY_LOCK_CHILD_ROLE", "hold")
        .env("INTEGRITY_LOCK_CHILD_DIR", save)
        .env("INTEGRITY_LOCK_CHILD_HOLD_MS", hold_ms.to_string())
        .env("INTEGRITY_LOCK_CHILD_READY", ready)
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
fn lock_with_timeout_fails_within_budget_when_another_process_holds_the_lock() {
    let save = save_dir("t6b");
    let ready = save.parent().unwrap().join(format!(
        "integrity-lock-ready-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut holder = spawn_holder(&save, 2500, &ready);
    wait_ready(&ready, &mut holder, Duration::from_secs(5));

    let timeout = Duration::from_millis(300);
    let start = Instant::now();
    let err = match lock_with_timeout(&save, timeout) {
        Err(err) => err,
        Ok(_) => panic!("should time out"),
    };
    let elapsed = start.elapsed();
    let msg = format!("{err:#}");
    assert!(msg.contains("内层锁"), "timeout must report 内层锁: {msg}");
    assert!(
        elapsed >= timeout,
        "must wait the inner budget, waited {elapsed:?} budget {timeout:?}"
    );
    assert!(
        elapsed < timeout + Duration::from_secs(2),
        "must fail inside the budget rather than hang: {elapsed:?}"
    );

    // The waiter must not have left a half-held lock. After the holder
    // exits, a later bounded acquire must succeed.
    finish_child(holder);
    let held = lock_with_timeout(&save, Duration::from_secs(1)).unwrap();
    drop(held);
    let _ = fs::remove_file(&ready);
    let _ = fs::remove_dir_all(&save);
}

/// Not a test: child body for the contention cases above.
#[test]
#[ignore]
fn child_helper() {
    let Ok(role) = std::env::var("INTEGRITY_LOCK_CHILD_ROLE") else {
        return;
    };
    assert_eq!(role, "hold");
    let save = PathBuf::from(std::env::var("INTEGRITY_LOCK_CHILD_DIR").unwrap());
    let hold_ms: u64 = std::env::var("INTEGRITY_LOCK_CHILD_HOLD_MS")
        .unwrap()
        .parse()
        .unwrap();
    let ready = PathBuf::from(std::env::var("INTEGRITY_LOCK_CHILD_READY").unwrap());
    let _held = integrity::lock(&save).expect("holder takes the blocking integrity lock");
    fs::write(&ready, b"ready").unwrap();
    std::thread::sleep(Duration::from_millis(hold_ms));
    println!("RESULT:OK");
}
