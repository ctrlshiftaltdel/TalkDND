//! Synthetic tests for the cooperative advancement lock (v1).
//!
//! Every fixture is created here; no test reads a real save, a book or a
//! secret. Multi-process and path-replacement injection are Unix-only because
//! Windows file-sharing semantics for replacing an open file are unverified
//! (see `docs/ADVANCEMENT-LOCK-DESIGN.md` §2.8 and the task's
//! `unverified_assumptions`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use talkdnd::advancement_lock as adv;

const SAVE_PATH: &str = "Saves/slot";

fn fast() -> adv::AcquireOptions {
    adv::AcquireOptions {
        timeout: Duration::from_millis(200),
        retry: Duration::from_millis(10),
    }
}

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("talkdnd-advlock-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_ok(dir: &Path) -> adv::InitOutcome {
    let mut opts = adv::InitOptions::new(SAVE_PATH);
    opts.acquire = fast();
    adv::init(dir, &opts).unwrap()
}

fn open_opts() -> adv::OpenOptions {
    let mut opts = adv::OpenOptions::new(SAVE_PATH);
    opts.acquire = fast();
    opts
}

fn open_ok(dir: &Path) -> adv::AdvancementLock {
    adv::open(dir, &open_opts()).unwrap()
}

/// Every file and directory under `dir`, keyed by a POSIX-style relative path.
/// Directory keys end in `/` so creating or removing one is visible.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type().unwrap().is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(root, &path, out);
            } else {
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn read_state(dir: &Path) -> adv::StateRecord {
    serde_json::from_slice(&fs::read(adv::state_path(dir)).unwrap()).unwrap()
}

fn write_state(dir: &Path, record: &adv::StateRecord) {
    fs::write(
        adv::state_path(dir),
        serde_json::to_vec_pretty(record).unwrap(),
    )
    .unwrap();
}

fn evidence_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "talkdnd-advlock-evidence-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

// ---------------------------------------------------------------- init ----

#[test]
fn init_creates_only_the_lock_and_state_and_a_second_init_is_refused() {
    let dir = root("init-basic");
    let outcome = init_ok(&dir);

    assert_eq!(
        snapshot(&dir).keys().cloned().collect::<Vec<_>>(),
        vec![adv::LOCK_FILE.to_string(), adv::STATE_FILE.to_string()]
    );
    assert_eq!(outcome.header.schema, adv::SCHEMA);
    assert_eq!(outcome.state.schema, adv::SCHEMA);
    assert_eq!(outcome.state.state, adv::State::Ready);
    assert_eq!(outcome.state.stage, adv::Stage::Init);
    assert_eq!(outcome.state.revision, 1);
    assert_eq!(outcome.state.save_id, outcome.header.save_id);
    assert_eq!(outcome.state.save_path, SAVE_PATH);
    assert!(!outcome.state.save_id.is_empty());

    let before = snapshot(&dir);
    let mut opts = adv::InitOptions::new(SAVE_PATH);
    opts.acquire = fast();
    let err = adv::init(&dir, &opts).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::AlreadyInitialized);
    assert_eq!(snapshot(&dir), before, "init must never take over");

    // What init wrote is exactly what a normal open reads back.
    let lock = open_ok(&dir);
    assert_eq!(lock.header(), &outcome.header);
    assert_eq!(lock.state(), &outcome.state);
}

#[test]
fn init_refuses_every_partial_protocol_state() {
    // lock without state
    let dir = root("init-lock-only");
    fs::write(adv::lock_path(&dir), b"").unwrap();
    let before = snapshot(&dir);
    let mut opts = adv::InitOptions::new(SAVE_PATH);
    opts.acquire = fast();
    assert_eq!(
        adv::init(&dir, &opts).unwrap_err().kind(),
        adv::ErrorKind::AlreadyInitialized
    );
    assert_eq!(snapshot(&dir), before);

    // state without lock
    let dir = root("init-state-only");
    fs::write(adv::state_path(&dir), b"{}").unwrap();
    let before = snapshot(&dir);
    assert_eq!(
        adv::init(&dir, &opts).unwrap_err().kind(),
        adv::ErrorKind::AlreadyInitialized
    );
    assert_eq!(snapshot(&dir), before);

    // a zero-byte lock is never overwritten
    let dir = root("init-zero-lock");
    fs::write(adv::lock_path(&dir), b"").unwrap();
    let before = snapshot(&dir);
    assert_eq!(
        adv::init(&dir, &opts).unwrap_err().kind(),
        adv::ErrorKind::AlreadyInitialized
    );
    assert_eq!(snapshot(&dir), before);
}

#[test]
fn init_refuses_an_invalid_save_path() {
    let dir = root("init-bad-path");
    for bad in [
        "",
        " Saves/slot",
        "/abs",
        "Saves/../x",
        "Saves\\slot",
        "Saves//x",
    ] {
        let mut opts = adv::InitOptions::new(bad);
        opts.acquire = fast();
        assert_eq!(
            adv::init(&dir, &opts).unwrap_err().kind(),
            adv::ErrorKind::InvalidRequest,
            "{bad:?} must be rejected"
        );
    }
    assert!(snapshot(&dir).is_empty(), "a rejected init creates nothing");
}

// ---------------------------------------------------------------- open ----

#[test]
fn open_requires_both_files_and_never_creates_them() {
    let dir = root("open-empty");
    let before = snapshot(&dir);
    let err = adv::open(&dir, &open_opts()).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::NotInitialized);
    assert_eq!(snapshot(&dir), before, "a normal open never creates a file");
}

#[test]
fn open_fails_closed_on_zero_byte_missing_corrupt_and_forged_state() {
    fn case(tag: &str, mutate: impl Fn(&Path)) -> adv::ErrorKind {
        let dir = root(tag);
        init_ok(&dir);
        mutate(&dir);
        let before = snapshot(&dir);
        let err = adv::open(&dir, &open_opts()).unwrap_err();
        assert_eq!(
            snapshot(&dir),
            before,
            "{tag}: a failed open must not repair, rewrite or create anything"
        );
        err.kind()
    }

    assert_eq!(
        case("state-zero", |d| fs::write(adv::state_path(d), b"")
            .unwrap()),
        adv::ErrorKind::CorruptState
    );
    assert_eq!(
        case("state-missing", |d| {
            fs::remove_file(adv::state_path(d)).unwrap()
        }),
        adv::ErrorKind::CorruptState
    );
    assert_eq!(
        case("state-garbage", |d| {
            fs::write(adv::state_path(d), b"{not json").unwrap()
        }),
        adv::ErrorKind::CorruptState
    );
    // Unknown fields are a future schema, not something to ignore.
    assert_eq!(
        case("state-unknown-field", |d| {
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(adv::state_path(d)).unwrap()).unwrap();
            value["future"] = serde_json::json!(true);
            fs::write(
                adv::state_path(d),
                serde_json::to_vec_pretty(&value).unwrap(),
            )
            .unwrap();
        }),
        adv::ErrorKind::CorruptState
    );
    // A changed body without a new checksum is corruption.
    assert_eq!(
        case("state-checksum", |d| {
            let mut record = read_state(d);
            record.revision = 9;
            write_state(d, &record);
        }),
        adv::ErrorKind::CorruptState
    );
    // A self-consistent record with a different save_id is an identity mismatch.
    assert_eq!(
        case("state-save-id", |d| {
            let mut record = read_state(d);
            record.save_id = "other".into();
            adv::reseal(&mut record).unwrap();
            write_state(d, &record);
        }),
        adv::ErrorKind::IdentityMismatch
    );
    // A self-consistent record naming another lock file is an identity mismatch.
    assert_eq!(
        case("state-lock-identity", |d| {
            let mut record = read_state(d);
            record.lock_identity = "unix:0:0".into();
            adv::reseal(&mut record).unwrap();
            write_state(d, &record);
        }),
        adv::ErrorKind::IdentityMismatch
    );
    // The lock header itself is parsed and its schema checked.
    assert_eq!(
        case("header-garbage", |d| {
            fs::write(adv::lock_path(d), b"{}").unwrap()
        }),
        adv::ErrorKind::CorruptState
    );
    assert_eq!(
        case("header-schema", |d| {
            let mut header: adv::LockHeader =
                serde_json::from_slice(&fs::read(adv::lock_path(d)).unwrap()).unwrap();
            header.schema = adv::SCHEMA + 1;
            fs::write(
                adv::lock_path(d),
                serde_json::to_vec_pretty(&header).unwrap(),
            )
            .unwrap();
        }),
        adv::ErrorKind::CorruptState
    );
}

#[test]
fn a_replaced_lock_file_is_an_identity_mismatch_before_open() {
    let dir = root("identity-before-open");
    init_ok(&dir);
    let bytes = fs::read(adv::lock_path(&dir)).unwrap();
    fs::remove_file(adv::lock_path(&dir)).unwrap();
    fs::write(adv::lock_path(&dir), &bytes).unwrap();

    let err = adv::open(&dir, &open_opts()).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::IdentityMismatch);
    assert!(
        err.detail().contains("not the lock that wrote this state"),
        "{}",
        err.detail()
    );
}

#[test]
fn a_read_only_success_path_writes_nothing() {
    let dir = root("read-only");
    init_ok(&dir);
    let before = snapshot(&dir);

    let lock = open_ok(&dir);
    assert_eq!(lock.state().state, adv::State::Ready);
    lock.check_writable().unwrap();
    drop(lock);

    assert_eq!(snapshot(&dir), before, "opening and reading must not write");
}

// ------------------------------------------------------------ predicate ----

#[test]
fn only_ready_complete_or_zero_write_failed_is_writable() {
    let dir = root("predicate");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    assert_eq!(lock.state().state, adv::State::Ready);
    lock.check_writable().unwrap();

    lock.begin_operation("op-1", &evidence_dir("predicate"))
        .unwrap();
    assert_eq!(lock.state().state, adv::State::Active);
    assert_eq!(
        lock.check_writable().unwrap_err().kind(),
        adv::ErrorKind::UnwritableState
    );

    lock.finish_in_doubt("publish stopped").unwrap();
    assert_eq!(lock.state().state, adv::State::InDoubt);
    assert_eq!(
        lock.check_writable().unwrap_err().kind(),
        adv::ErrorKind::UnwritableState
    );
}

#[test]
fn failed_is_only_allowed_before_any_target_write() {
    let dir = root("failed");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    let ops = evidence_dir("failed");

    lock.begin_operation("op-1", &ops).unwrap();
    lock.finish_failed("backup was not verified").unwrap();
    assert_eq!(lock.state().state, adv::State::Failed);
    assert_eq!(lock.state().stage, adv::Stage::StoppedBeforeWrite);
    lock.check_writable().unwrap();
    // A stop before any target write may be reopened with a new operation.
    lock.begin_operation("op-2", &ops).unwrap();
    assert_eq!(lock.state().operation_id, "op-2");

    // Once an intent or a publish exists, `failed` is a contradiction.
    lock.record_intent("sheets/a.yaml", "before", "after")
        .unwrap();
    assert_eq!(
        lock.finish_failed("stop").unwrap_err().kind(),
        adv::ErrorKind::InvalidRequest
    );
    lock.finish_in_doubt("stop").unwrap();
    assert_eq!(lock.state().state, adv::State::InDoubt);
    assert_eq!(
        lock.check_writable().unwrap_err().kind(),
        adv::ErrorKind::UnwritableState
    );
}

#[test]
fn complete_needs_all_verification_and_no_pending_intent() {
    let dir = root("complete");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    lock.begin_operation("op-1", &evidence_dir("complete"))
        .unwrap();
    lock.record_intent("sheets/a.yaml", "before", "after")
        .unwrap();

    assert_eq!(
        lock.finish_complete(&adv::CompletionEvidence::default())
            .unwrap_err()
            .kind(),
        adv::ErrorKind::InvalidRequest
    );
    let partial = adv::CompletionEvidence {
        candidate_matches: true,
        logs_recorded: true,
        ..Default::default()
    };
    assert_eq!(
        lock.finish_complete(&partial).unwrap_err().kind(),
        adv::ErrorKind::InvalidRequest
    );

    lock.record_published("sheets/a.yaml").unwrap();
    assert!(lock.state().pending.is_empty());
    assert_eq!(lock.state().published.len(), 1);
    assert!(lock.state().published[0].done);

    let all = adv::CompletionEvidence {
        candidate_matches: true,
        logs_recorded: true,
        integrity_recorded: true,
    };
    lock.finish_complete(&all).unwrap();
    assert_eq!(lock.state().state, adv::State::Complete);
    assert_eq!(lock.state().stage, adv::Stage::Done);
    lock.check_writable().unwrap();
}

#[test]
fn a_forged_self_consistent_complete_with_pending_is_refused() {
    let dir = root("forged-complete");
    init_ok(&dir);
    let mut record = read_state(&dir);
    record.state = adv::State::Complete;
    record.stage = adv::Stage::Done;
    record.pending.push(adv::Pending {
        path: "sheets/a.yaml".into(),
        before: "before".into(),
        after: "after".into(),
    });
    adv::reseal(&mut record).unwrap();
    write_state(&dir, &record);

    let lock = open_ok(&dir);
    assert_eq!(
        lock.check_writable().unwrap_err().kind(),
        adv::ErrorKind::UnwritableState
    );
}

#[test]
fn state_updates_replace_atomically_and_never_truncate_in_place() {
    let dir = root("revisions");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    let ops = evidence_dir("revisions");
    lock.begin_operation("op-1", &ops).unwrap();
    lock.record_intent("sheets/a.yaml", "before", "after")
        .unwrap();
    lock.record_published("sheets/a.yaml").unwrap();
    lock.finish_complete(&adv::CompletionEvidence {
        candidate_matches: true,
        logs_recorded: true,
        integrity_recorded: true,
    })
    .unwrap();
    drop(lock);

    assert_eq!(
        snapshot(&dir).keys().cloned().collect::<Vec<_>>(),
        vec![adv::LOCK_FILE.to_string(), adv::STATE_FILE.to_string()],
        "no temp file may survive a successful write"
    );
    let record = read_state(&dir);
    assert_eq!(record.revision, 5);
    assert_eq!(record.state, adv::State::Complete);
}

// -------------------------------------------------------- pre-state (D) ----

#[test]
fn begin_operation_snapshots_the_previous_state_before_becoming_active() {
    let dir = root("prestate");
    init_ok(&dir);
    let ops = evidence_dir("prestate");
    let mut lock = open_ok(&dir);
    lock.begin_operation("op-1", &ops).unwrap();

    assert_eq!(lock.state().state, adv::State::Active);
    assert_eq!(lock.state().stage, adv::Stage::Admit);
    assert_eq!(lock.state().operation_id, "op-1");

    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(ops.join("op-1-prestate.json")).unwrap()).unwrap();
    assert_eq!(doc["prior_state"], "ready");
    assert_eq!(doc["prior_stage"], "init");
    assert_eq!(doc["state_revision"], 1);
    assert_eq!(doc["save_path"], SAVE_PATH);
    assert!(doc["state_bytes"].as_str().unwrap().contains("\"ready\""));
}

#[test]
fn a_failed_prestate_snapshot_leaves_the_state_untouched() {
    let dir = root("prestate-fail");
    init_ok(&dir);
    let ops = evidence_dir("prestate-fail");
    fs::create_dir_all(&ops).unwrap();
    // The exclusive create must fail: this is the injectable failure.
    fs::write(ops.join("op-1-prestate.json"), b"stale evidence").unwrap();

    let before = fs::read(adv::state_path(&dir)).unwrap();
    let mut lock = open_ok(&dir);
    let err = lock.begin_operation("op-1", &ops).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::EvidenceFailed);
    assert_eq!(fs::read(adv::state_path(&dir)).unwrap(), before);
    assert_eq!(lock.state().state, adv::State::Ready);
    lock.check_writable().unwrap();
}

// ------------------------------------------------------------- copying ----

#[test]
fn a_copied_or_moved_save_with_another_path_is_refused() {
    let from = root("copy-source");
    init_ok(&from);
    let to = root("copy-target");
    copy_dir(&from, &to);

    let mut opts = adv::OpenOptions::new("Saves/moved");
    opts.acquire = fast();
    let err = adv::open(&to, &opts).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::SavePathMismatch);
    assert!(
        err.detail().contains("does not support"),
        "{}",
        err.detail()
    );
    assert!(
        err.detail().contains("may not be detectable"),
        "{}",
        err.detail()
    );
    // Nothing was written into the copy.
    assert_eq!(
        snapshot(&to),
        snapshot(&from),
        "a refused copy must stay byte-identical"
    );
}

#[test]
fn a_copied_save_with_the_same_relative_path_is_refused_by_lock_identity() {
    let from = root("copy-same-source");
    init_ok(&from);
    let to = root("copy-same-target");
    copy_dir(&from, &to);

    // The copy keeps `save_path`, so only the file identity can notice it.
    let err = adv::open(&to, &open_opts()).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::IdentityMismatch);
}

// ------------------------------------------------------ bounded acquire ----

#[test]
fn a_second_handle_in_this_process_cannot_take_the_lock() {
    // The same self-check `init` runs: this records what the platform really
    // does instead of assuming that a lock file exists means mutual exclusion.
    let dir = root("self-check");
    init_ok(&dir);
    let lock = open_ok(&dir);
    let err = adv::open(&dir, &open_opts()).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::LockTimeout);
    drop(lock);
    open_ok(&dir);
}

#[cfg(unix)]
#[test]
fn a_second_process_times_out_and_never_takes_over() {
    let dir = root("timeout");
    init_ok(&dir);
    let lock = open_ok(&dir);
    let before = snapshot(&dir);

    let result = run_child("open", &dir);
    assert_eq!(result, "ERR:lock_timeout");
    assert_eq!(snapshot(&dir), before, "the waiter must not write anything");
    assert_eq!(lock.state().state, adv::State::Ready);
    drop(lock);
}

#[cfg(unix)]
#[test]
fn two_processes_race_init_and_exactly_one_wins() {
    let dir = root("init-race");
    let children: Vec<Child> = (0..4).map(|_| spawn_child("init", &dir)).collect();
    let results: Vec<String> = children.into_iter().map(wait_child).collect();

    let winners = results.iter().filter(|r| *r == "OK").count();
    assert_eq!(winners, 1, "exactly one init may win: {results:?}");
    for loser in results.iter().filter(|r| *r != "OK") {
        assert_eq!(loser, "ERR:already_initialized", "{results:?}");
    }
    assert_eq!(
        snapshot(&dir).keys().cloned().collect::<Vec<_>>(),
        vec![adv::LOCK_FILE.to_string(), adv::STATE_FILE.to_string()]
    );
    let lock = open_ok(&dir);
    assert_eq!(lock.state().state, adv::State::Ready);
}

#[cfg(unix)]
#[test]
fn replacing_the_lock_while_held_is_detected_before_the_next_write() {
    let dir = root("replace-held");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    let before = fs::read(adv::state_path(&dir)).unwrap();

    let replacement = dir.join("replacement");
    fs::write(&replacement, b"not the locked inode").unwrap();
    fs::rename(&replacement, adv::lock_path(&dir)).unwrap();

    let err = lock.finish_in_doubt("stop").unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::PathReplaced);
    assert_eq!(
        fs::read(adv::state_path(&dir)).unwrap(),
        before,
        "a replaced lock aborts before any state write"
    );
}

#[cfg(unix)]
#[test]
fn init_stops_when_the_path_is_replaced_before_the_first_identity_check() {
    let dir = root("t2f1");
    let replacement = dir.join("replacement");
    fs::write(&replacement, b"not the locked inode").unwrap();
    let replace = {
        let replacement = replacement.clone();
        move |path: &Path| fs::rename(&replacement, path).unwrap()
    };
    let mut opts = adv::InitOptions::new(SAVE_PATH);
    opts.acquire = fast();
    opts.hooks = adv::InitHooks {
        after_create: Some(&replace),
        before_publish: None,
    };

    let err = adv::init(&dir, &opts).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::PathReplaced);
    assert!(
        !adv::state_path(&dir).exists(),
        "no state may be published after a replacement"
    );
    assert_eq!(
        fs::read(adv::lock_path(&dir)).unwrap(),
        b"not the locked inode",
        "the replacement must not receive a header"
    );
}

#[cfg(unix)]
#[test]
fn init_stops_when_the_path_is_replaced_after_the_header_before_publish() {
    let dir = root("t2f2");
    let replacement = dir.join("replacement");
    fs::write(&replacement, b"not the locked inode").unwrap();
    let replace = {
        let replacement = replacement.clone();
        move |path: &Path| fs::rename(&replacement, path).unwrap()
    };
    let mut opts = adv::InitOptions::new(SAVE_PATH);
    opts.acquire = fast();
    opts.hooks = adv::InitHooks {
        after_create: None,
        before_publish: Some(&replace),
    };

    let err = adv::init(&dir, &opts).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::PathReplaced);
    assert!(
        !adv::state_path(&dir).exists(),
        "no state may be published after a replacement"
    );
    // The header was written to the original (now unlinked) inode; the
    // replacement file proves the tool cannot claim it skipped the header.
    assert_eq!(
        fs::read(adv::lock_path(&dir)).unwrap(),
        b"not the locked inode"
    );
    // A later normal open fails closed: lock exists, state does not.
    assert_eq!(
        adv::open(&dir, &open_opts()).unwrap_err().kind(),
        adv::ErrorKind::CorruptState
    );
}

#[cfg(unix)]
#[test]
fn a_read_only_directory_stops_a_state_write_and_keeps_the_old_state() {
    use std::os::unix::fs::PermissionsExt;

    let dir = root("read-only-write");
    init_ok(&dir);
    let ops = evidence_dir("read-only-write");
    let mut lock = open_ok(&dir);
    lock.begin_operation("op-1", &ops).unwrap();
    let before = fs::read(adv::state_path(&dir)).unwrap();

    let mut perms = fs::metadata(&dir).unwrap().permissions();
    let mode = perms.mode();
    perms.set_mode(0o555);
    fs::set_permissions(&dir, perms).unwrap();

    // A privileged user ignores the mode; never pretend the injection worked.
    if fs::write(dir.join("probe"), b"x").is_ok() {
        let _ = fs::remove_file(dir.join("probe"));
        let mut perms = fs::metadata(&dir).unwrap().permissions();
        perms.set_mode(mode);
        fs::set_permissions(&dir, perms).unwrap();
        eprintln!("skipped: this user can write into a 0o555 directory");
        return;
    }

    let result = lock.finish_in_doubt("stop");
    let mut perms = fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(mode);
    fs::set_permissions(&dir, perms).unwrap();

    assert_eq!(result.unwrap_err().kind(), adv::ErrorKind::Io);
    assert_eq!(fs::read(adv::state_path(&dir)).unwrap(), before);
}

// ------------------------------------------------- child process helper ----

fn spawn_child(role: &str, dir: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "child_helper",
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env("ADV_LOCK_CHILD_ROLE", role)
        .env("ADV_LOCK_CHILD_DIR", dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn run_child(role: &str, dir: &Path) -> String {
    wait_child(spawn_child(role, dir))
}

fn wait_child(child: Child) -> String {
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // `--nocapture` keeps libtest's own "test child_helper ... " prefix on the
    // same line, so look for the marker anywhere in it.
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            line.find("RESULT:")
                .map(|at| line[at + 7..].trim().to_string())
        })
        .expect("child printed a RESULT line")
}

/// Not a test: a child process body used by the multi-process tests above.
#[test]
#[ignore]
fn child_helper() {
    let Ok(role) = std::env::var("ADV_LOCK_CHILD_ROLE") else {
        return;
    };
    let dir = PathBuf::from(std::env::var("ADV_LOCK_CHILD_DIR").unwrap());
    let result = match role.as_str() {
        "open" => match adv::open(&dir, &open_opts()) {
            Ok(_) => "OK".to_string(),
            Err(e) => format!("ERR:{}", e.kind().as_str()),
        },
        "init" => {
            let mut opts = adv::InitOptions::new(SAVE_PATH);
            opts.acquire = fast();
            match adv::init(&dir, &opts) {
                Ok(_) => "OK".to_string(),
                Err(e) => format!("ERR:{}", e.kind().as_str()),
            }
        }
        other => panic!("unknown child role {other}"),
    };
    println!("RESULT:{result}");
}
