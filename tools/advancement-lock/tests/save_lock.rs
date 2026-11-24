//! Synthetic tests for the cooperative advancement lock (v1).
//!
//! Every fixture is created here; no test reads a real save, a book or a
//! secret. Path-replacement injection, the read-only permission tests and the
//! tests that need a commit (`begin_operation`) are Unix-only: the first two
//! because Windows file-sharing and mode semantics for them are unverified,
//! the last because a commit requires directory fsync, which Windows does not
//! provide. The Windows-only test asserts that refusal, and every test that
//! does not need a commit — including the multi-process cases — runs on every
//! platform. See `docs/ADVANCEMENT-LOCK-DESIGN.md` §2.6 and the task's
//! `unverified_assumptions`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use advancement_lock::save_lock as adv;

fn fast() -> adv::AcquireOptions {
    adv::AcquireOptions {
        timeout: Duration::from_millis(200),
        retry: Duration::from_millis(10),
    }
}

/// The repository root of one synthetic fixture.
fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("advlock-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The save slot path below `root`, without creating it.
fn slot(root: &Path) -> PathBuf {
    root.join("Saves").join("slot")
}

/// The save slot path below `root`, created.
fn make_slot(root: &Path) -> PathBuf {
    let dir = slot(root);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_opts() -> adv::InitOptions<'static> {
    let mut opts = adv::InitOptions::new();
    opts.acquire = fast();
    opts
}

/// `init` on Unix publishes durably. On a platform without directory fsync the
/// state is still published, but `init` must report
/// `state_published_not_durable` instead of `Ok`, so the helper accepts exactly
/// that and reads the outcome back through a normal open.
#[cfg(unix)]
fn init_at(root: &Path, slot: &Path) -> adv::InitOutcome {
    adv::init(root, slot, &init_opts()).unwrap()
}

#[cfg(not(unix))]
fn init_at(root: &Path, slot: &Path) -> adv::InitOutcome {
    let err = adv::init(root, slot, &init_opts()).unwrap_err();
    assert_eq!(
        err.kind(),
        adv::ErrorKind::StatePublishedNotDurable,
        "{err}"
    );
    let lock = adv::open(root, slot, &open_opts()).unwrap();
    adv::InitOutcome {
        header: lock.header().clone(),
        state: lock.state().clone(),
    }
}

fn init_ok(root: &Path) -> adv::InitOutcome {
    init_at(root, &make_slot(root))
}

fn open_opts() -> adv::OpenOptions {
    let mut opts = adv::OpenOptions::new();
    opts.acquire = fast();
    opts
}

fn open_ok(root: &Path) -> adv::AdvancementLock {
    adv::open(root, &make_slot(root), &open_opts()).unwrap()
}

fn open_at(root: &Path, save: &Path) -> adv::Result<adv::AdvancementLock> {
    adv::open(root, save, &open_opts())
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

/// Names and sizes only. Safe while this process holds the lock: a Windows
/// byte-range lock blocks *reading* the locked file from another handle but not
/// a metadata query, so a byte snapshot cannot be taken while the lock is held.
fn footprint(dir: &Path) -> BTreeMap<String, u64> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, u64>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                out.insert(format!("{rel}/"), 0);
                walk(root, &path, out);
            } else {
                out.insert(rel, meta.len());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn read_state(root: &Path) -> adv::StateRecord {
    serde_json::from_slice(&fs::read(adv::state_path(&slot(root))).unwrap()).unwrap()
}

fn write_state(root: &Path, record: &adv::StateRecord) {
    fs::write(
        adv::state_path(&slot(root)),
        serde_json::to_vec_pretty(record).unwrap(),
    )
    .unwrap();
}

/// A trusted, existing evidence root plus the directory below it that the
/// pre-state snapshot will create.
fn evidence_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("advlock-evidence-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn evidence_dir(tag: &str) -> PathBuf {
    evidence_root(tag).join("ops")
}

// ---------------------------------------------------------------- init ----

#[test]
fn init_creates_only_the_lock_and_state_and_a_second_init_is_refused() {
    let dir = root("init-basic");
    let outcome = init_ok(&dir);

    assert_eq!(
        snapshot(&slot(&dir)).keys().cloned().collect::<Vec<_>>(),
        vec![adv::LOCK_FILE.to_string(), adv::STATE_FILE.to_string()]
    );
    assert_eq!(outcome.header.schema, adv::SCHEMA);
    assert_eq!(outcome.state.schema, adv::SCHEMA);
    assert_eq!(outcome.state.state, adv::State::Ready);
    assert_eq!(outcome.state.stage, adv::Stage::Init);
    assert_eq!(outcome.state.revision, 1);
    assert_eq!(outcome.state.save_id, outcome.header.save_id);
    assert_eq!(outcome.state.save_path, "Saves/slot");
    assert!(!outcome.state.save_id.is_empty());

    let before = snapshot(&slot(&dir));
    let err = adv::init(&dir, &slot(&dir), &init_opts()).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::AlreadyInitialized);
    assert_eq!(snapshot(&slot(&dir)), before, "init must never take over");

    // What init wrote is exactly what a normal open reads back.
    let lock = open_ok(&dir);
    assert_eq!(lock.header(), &outcome.header);
    assert_eq!(lock.state(), &outcome.state);
    assert_eq!(lock.save_path(), "Saves/slot");
}

#[test]
fn init_refuses_every_partial_protocol_state() {
    let dir = root("init-lock-only");
    fs::write(adv::lock_path(&make_slot(&dir)), b"").unwrap();
    let before = snapshot(&slot(&dir));
    assert_eq!(
        adv::init(&dir, &slot(&dir), &init_opts())
            .unwrap_err()
            .kind(),
        adv::ErrorKind::AlreadyInitialized
    );
    assert_eq!(snapshot(&slot(&dir)), before);

    let dir = root("init-state-only");
    fs::write(adv::state_path(&make_slot(&dir)), b"{}").unwrap();
    let before = snapshot(&slot(&dir));
    assert_eq!(
        adv::init(&dir, &slot(&dir), &init_opts())
            .unwrap_err()
            .kind(),
        adv::ErrorKind::AlreadyInitialized
    );
    assert_eq!(snapshot(&slot(&dir)), before);

    let dir = root("init-zero-lock");
    fs::write(adv::lock_path(&make_slot(&dir)), b"").unwrap();
    let before = snapshot(&slot(&dir));
    assert_eq!(
        adv::init(&dir, &slot(&dir), &init_opts())
            .unwrap_err()
            .kind(),
        adv::ErrorKind::AlreadyInitialized
    );
    assert_eq!(snapshot(&slot(&dir)), before);
}

#[test]
fn init_refuses_a_path_it_cannot_trust() {
    let dir = root("init-bad-path");
    let save = make_slot(&dir);
    let before = snapshot(&dir);

    // Relative root.
    assert_eq!(
        adv::init(Path::new("relative"), &save, &init_opts())
            .unwrap_err()
            .kind(),
        adv::ErrorKind::InvalidRequest
    );
    // The save is the root itself.
    assert_eq!(
        adv::init(&dir, &dir, &init_opts()).unwrap_err().kind(),
        adv::ErrorKind::InvalidRequest
    );
    // A `..` component is refused instead of resolved.
    assert_eq!(
        adv::init(&dir, &dir.join("Saves/../Saves/slot"), &init_opts())
            .unwrap_err()
            .kind(),
        adv::ErrorKind::InvalidRequest
    );
    // A trailing separator is an ambiguous second spelling.
    let trailing = PathBuf::from(format!("{}/", save.display()));
    assert_eq!(
        adv::init(&dir, &trailing, &init_opts()).unwrap_err().kind(),
        adv::ErrorKind::InvalidRequest
    );
    // A repeated separator is ambiguous too.
    assert_eq!(
        adv::init(&dir, &dir.join("Saves//slot"), &init_opts())
            .unwrap_err()
            .kind(),
        adv::ErrorKind::InvalidRequest
    );
    // A save outside the root.
    let outside = root("init-bad-path-outside");
    let outside_save = make_slot(&outside);
    assert_eq!(
        adv::init(&dir, &outside_save, &init_opts())
            .unwrap_err()
            .kind(),
        adv::ErrorKind::InvalidRequest
    );

    assert_eq!(snapshot(&dir), before, "a rejected init creates nothing");
}

#[cfg(unix)]
#[test]
fn init_refuses_a_symlinked_save_slot() {
    use std::os::unix::fs::symlink;

    let dir = root("init-symlink");
    let real = make_slot(&dir);
    let link = dir.join("Saves").join("link");
    symlink(&real, &link).unwrap();

    let err = adv::init(&dir, &link, &init_opts()).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::InvalidRequest);
    assert!(err.detail().contains("symlink"), "{}", err.detail());
}

#[cfg(unix)]
#[test]
fn a_non_utf8_save_component_is_refused_without_touching_its_target() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;

    let dir = root("non-utf8");
    let real = make_slot(&dir);
    // A symlink whose *own name* is not valid UTF-8 must not be able to
    // masquerade as the replacement-character spelling of another name.
    let link = dir
        .join("Saves")
        .join(OsString::from_vec(b"bad-\xff".to_vec()));
    if let Err(e) = symlink(&real, &link) {
        // The filesystem refuses the name itself (APFS does), so the bypass is
        // impossible there. A creation error is the expected "unsupported";
        // anything after a successful creation is an unexpected failure and
        // falls through to the assertions below.
        eprintln!("skipped: this filesystem refuses a non-UTF-8 symlink name ({e})");
        return;
    }

    let err = adv::init(&dir, &link, &init_opts()).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::InvalidRequest);
    assert!(err.detail().contains("not valid UTF-8"), "{}", err.detail());
    // The link is untouched and still points at the real slot.
    let meta = fs::symlink_metadata(&link).unwrap();
    assert!(meta.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), real);
    assert!(
        !adv::lock_path(&real).exists(),
        "nothing may be created through the link"
    );

    // A real directory with a non-UTF-8 name is refused too, not recorded
    // lossily.
    let plain = dir
        .join("Saves")
        .join(OsString::from_vec(b"plain-\xfe".to_vec()));
    if let Err(e) = fs::create_dir(&plain) {
        // Same distinction: the filesystem refusing the name is expected.
        eprintln!("skipped: this filesystem refuses a non-UTF-8 directory name ({e})");
        return;
    }
    let err = adv::init(&dir, &plain, &init_opts()).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::InvalidRequest);
    assert!(!adv::lock_path(&plain).exists());
}

#[test]
fn a_directory_named_with_the_replacement_character_round_trips_losslessly() {
    let dir = root("replacement-char");
    let slot = dir.join("Saves").join("slot\u{FFFD}");
    fs::create_dir_all(&slot).unwrap();

    let outcome = init_at(&dir, &slot);
    assert_eq!(outcome.state.save_path, "Saves/slot\u{FFFD}");
    let lock = adv::open(&dir, &slot, &open_opts()).unwrap();
    assert_eq!(lock.save_path(), "Saves/slot\u{FFFD}");
}

#[test]
fn init_reports_a_missing_save_directory_instead_of_taking_over() {
    let dir = root("init-missing-dir");
    let missing = dir.join("Saves").join("absent");
    let err = adv::init(&dir, &missing, &init_opts()).unwrap_err();
    assert_ne!(
        err.kind(),
        adv::ErrorKind::AlreadyInitialized,
        "a missing save directory is not an existing protocol file"
    );
    assert!(!missing.exists(), "a failed init creates no directory");
}

#[test]
fn init_never_overwrites_a_state_created_before_publish() {
    let dir = root("init-no-overwrite");
    make_slot(&dir);
    let sentinel = b"state created by someone else".to_vec();
    let hook = {
        let target = adv::state_path(&slot(&dir));
        let sentinel = sentinel.clone();
        move |_lock: &Path| fs::write(&target, &sentinel).unwrap()
    };
    let mut opts = adv::InitOptions::new();
    opts.acquire = fast();
    opts.hooks = adv::InitHooks {
        after_create: None,
        before_publish: Some(&hook),
    };

    let err = adv::init(&dir, &slot(&dir), &opts).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::AlreadyInitialized);
    assert_eq!(
        fs::read(adv::state_path(&slot(&dir))).unwrap(),
        sentinel,
        "the existing state must survive byte-identically"
    );
    // The lock exists (with its header), so a normal open must fail closed
    // instead of treating the sentinel as a first initialization.
    assert_eq!(
        open_at(&dir, &slot(&dir)).unwrap_err().kind(),
        adv::ErrorKind::CorruptState
    );
}

// ---------------------------------------------------------------- open ----

#[test]
fn open_requires_both_files_and_never_creates_them() {
    let dir = root("open-empty");
    make_slot(&dir);
    let before = snapshot(&slot(&dir));
    let err = open_at(&dir, &slot(&dir)).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::NotInitialized);
    assert_eq!(
        snapshot(&slot(&dir)),
        before,
        "a normal open never creates a file"
    );
}

#[test]
fn open_fails_closed_on_zero_byte_missing_corrupt_and_forged_state() {
    fn case(tag: &str, mutate: impl Fn(&Path)) -> adv::ErrorKind {
        let dir = root(tag);
        init_ok(&dir);
        mutate(&slot(&dir));
        let before = snapshot(&slot(&dir));
        let err = open_at(&dir, &slot(&dir)).unwrap_err();
        assert_eq!(
            snapshot(&slot(&dir)),
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
            let mut record: adv::StateRecord =
                serde_json::from_slice(&fs::read(adv::state_path(d)).unwrap()).unwrap();
            record.revision = 9;
            fs::write(
                adv::state_path(d),
                serde_json::to_vec_pretty(&record).unwrap(),
            )
            .unwrap();
        }),
        adv::ErrorKind::CorruptState
    );
    // A self-consistent record with a different save_id is an identity mismatch.
    assert_eq!(
        case("state-save-id", |d| {
            let mut record: adv::StateRecord =
                serde_json::from_slice(&fs::read(adv::state_path(d)).unwrap()).unwrap();
            record.save_id = "other".into();
            adv::reseal(&mut record).unwrap();
            fs::write(
                adv::state_path(d),
                serde_json::to_vec_pretty(&record).unwrap(),
            )
            .unwrap();
        }),
        adv::ErrorKind::IdentityMismatch
    );
    // A self-consistent record naming another lock file is an identity mismatch.
    assert_eq!(
        case("state-lock-identity", |d| {
            let mut record: adv::StateRecord =
                serde_json::from_slice(&fs::read(adv::state_path(d)).unwrap()).unwrap();
            record.lock_identity = "unix:0:0".into();
            adv::reseal(&mut record).unwrap();
            fs::write(
                adv::state_path(d),
                serde_json::to_vec_pretty(&record).unwrap(),
            )
            .unwrap();
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
fn open_rejects_a_ready_state_that_contradicts_itself() {
    fn forged(tag: &str, mutate: impl Fn(&mut adv::StateRecord)) -> adv::ErrorKind {
        let dir = root(tag);
        init_ok(&dir);
        let mut record = read_state(&dir);
        mutate(&mut record);
        adv::reseal(&mut record).unwrap();
        write_state(&dir, &record);
        let before = snapshot(&slot(&dir));
        let err = open_at(&dir, &slot(&dir)).unwrap_err();
        assert_eq!(snapshot(&slot(&dir)), before);
        err.kind()
    }

    // `ready` with a pending intent, even with a self-consistent checksum.
    assert_eq!(
        forged("ready-pending", |record| {
            record.pending.push(adv::Pending {
                path: "sheets/a.yaml".into(),
                before: "a".into(),
                after: "b".into(),
            });
        }),
        adv::ErrorKind::CorruptState
    );
    // `ready` with a published entry.
    assert_eq!(
        forged("ready-published", |record| {
            record.published.push(adv::Published {
                path: "sheets/a.yaml".into(),
                before: "a".into(),
                after: "b".into(),
                done: true,
            });
        }),
        adv::ErrorKind::CorruptState
    );
    // `ready` at a stage other than init.
    assert_eq!(
        forged("ready-stage", |record| {
            record.stage = adv::Stage::Publish;
        }),
        adv::ErrorKind::CorruptState
    );
}

#[cfg(unix)]
#[test]
fn open_fails_closed_when_the_directory_cannot_be_enumerated() {
    use std::os::unix::fs::PermissionsExt;

    let dir = root("enumerate");
    init_ok(&dir);
    let save = slot(&dir);
    let before = snapshot(&dir.join("Saves"));
    let mut perms = fs::metadata(&save).unwrap().permissions();
    let mode = perms.mode();
    // Execute-only: opening a known file by name still works, listing does not.
    perms.set_mode(0o111);
    fs::set_permissions(&save, perms).unwrap();

    if fs::read_dir(&save).is_ok() {
        let mut perms = fs::metadata(&save).unwrap().permissions();
        perms.set_mode(mode);
        fs::set_permissions(&save, perms).unwrap();
        eprintln!("skipped: this user can list a 0o111 directory");
        return;
    }

    let result = open_at(&dir, &save);
    let mut perms = fs::metadata(&save).unwrap().permissions();
    perms.set_mode(mode);
    fs::set_permissions(&save, perms).unwrap();

    let err = result.unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::CorruptState);
    assert!(
        err.detail().contains("enumerate"),
        "the error must name the enumeration step: {}",
        err.detail()
    );
    assert_eq!(snapshot(&dir.join("Saves")), before);
}

#[test]
fn open_rejects_a_save_outside_the_root_or_a_moved_path() {
    let dir = root("outside");
    init_ok(&dir);
    let other = root("outside-other");
    make_slot(&other);

    let err = open_at(&dir, &slot(&other)).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::InvalidRequest);
    assert!(err.detail().contains("outside"), "{}", err.detail());

    // A same-volume rename keeps the files but changes the derived path.
    let old = slot(&dir);
    let moved = dir.join("Saves").join("renamed");
    fs::rename(&old, &moved).unwrap();
    let before = snapshot(&dir.join("Saves"));
    let err = open_at(&dir, &moved).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::SavePathMismatch);
    assert!(
        err.detail().contains("does not support"),
        "{}",
        err.detail()
    );
    assert_eq!(snapshot(&dir.join("Saves")), before);
}

#[test]
fn a_replaced_lock_file_is_an_identity_mismatch_before_open() {
    let dir = root("identity-before-open");
    init_ok(&dir);
    let bytes = fs::read(adv::lock_path(&slot(&dir))).unwrap();
    // Write a sibling copy and rename it over the lock: the source file exists
    // while the target does, so the two cannot share an inode. Deleting first
    // and rewriting would let the filesystem reuse the inode.
    let replacement = slot(&dir).join("replacement");
    fs::write(&replacement, &bytes).unwrap();
    fs::rename(&replacement, adv::lock_path(&slot(&dir))).unwrap();

    let err = open_at(&dir, &slot(&dir)).unwrap_err();
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
    let before = snapshot(&slot(&dir));

    let lock = open_ok(&dir);
    assert_eq!(lock.state().state, adv::State::Ready);
    #[cfg(unix)]
    lock.check_writable().unwrap();
    #[cfg(not(unix))]
    assert_eq!(
        lock.check_writable().unwrap_err().kind(),
        adv::ErrorKind::UnwritableState,
        "a platform without directory sync must never answer writable"
    );
    drop(lock);

    assert_eq!(
        snapshot(&slot(&dir)),
        before,
        "opening and reading must not write"
    );
}

#[test]
fn ordinary_write_allows_ready_state_without_directory_sync() {
    let dir = root("ordinary-ready");
    init_ok(&dir);
    let before = snapshot(&slot(&dir));
    let lock = open_ok(&dir);
    lock.check_ordinary_write().unwrap();
    drop(lock);
    assert_eq!(
        snapshot(&slot(&dir)),
        before,
        "check_ordinary_write must not write"
    );

    // Planted active / in_doubt states fail closed on every platform,
    // including those that cannot fsync a directory.
    let path = adv::state_path(&slot(&dir));
    let mut record: adv::StateRecord = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record.state = adv::State::Active;
    record.stage = adv::Stage::Admit;
    record.operation_id = "plant-active".into();
    adv::reseal(&mut record).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let err = open_ok(&dir).check_ordinary_write().unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::UnwritableState);
    assert!(err.detail().contains("active"), "{}", err.detail());

    record.state = adv::State::InDoubt;
    record.stage = adv::Stage::Publish;
    record.last_error = Some("planted".into());
    adv::reseal(&mut record).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let err = open_ok(&dir).check_ordinary_write().unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::UnwritableState);
    assert!(err.detail().contains("in_doubt"), "{}", err.detail());
}

// ------------------------------------------------------------ predicate ----

#[cfg(unix)]
#[test]
fn only_ready_complete_or_zero_write_failed_is_writable() {
    let dir = root("predicate");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    assert_eq!(lock.state().state, adv::State::Ready);
    lock.check_writable().unwrap();

    let evidence_root = evidence_root("predicate");
    lock.begin_operation("op-1", &evidence_root, &evidence_dir("predicate"))
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

#[cfg(unix)]
#[test]
fn failed_is_only_allowed_before_any_target_write() {
    let dir = root("failed");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    let evidence_root = evidence_root("failed");
    let ops = evidence_dir("failed");

    lock.begin_operation("op-1", &evidence_root, &ops).unwrap();
    lock.finish_failed("backup was not verified").unwrap();
    assert_eq!(lock.state().state, adv::State::Failed);
    assert_eq!(lock.state().stage, adv::Stage::StoppedBeforeWrite);
    lock.check_writable().unwrap();
    // A stop before any target write may be reopened with a new operation.
    lock.begin_operation("op-2", &evidence_root, &ops).unwrap();
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

#[cfg(unix)]
#[test]
fn complete_needs_all_verification_and_no_pending_intent() {
    let dir = root("complete");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    let evidence_root = evidence_root("complete");
    lock.begin_operation("op-1", &evidence_root, &evidence_dir("complete"))
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
fn a_forged_self_consistent_complete_with_an_unfinished_publish_is_refused() {
    let dir = root("forged-unfinished");
    init_ok(&dir);
    let mut record = read_state(&dir);
    record.state = adv::State::Complete;
    record.stage = adv::Stage::Done;
    record.published.push(adv::Published {
        path: "sheets/a.yaml".into(),
        before: "before".into(),
        after: "after".into(),
        done: false,
    });
    adv::reseal(&mut record).unwrap();
    write_state(&dir, &record);

    let lock = open_ok(&dir);
    assert_eq!(
        lock.check_writable().unwrap_err().kind(),
        adv::ErrorKind::UnwritableState
    );
}

#[cfg(unix)]
#[test]
fn state_updates_replace_atomically_and_never_truncate_in_place() {
    let dir = root("revisions");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    let evidence_root = evidence_root("revisions");
    let ops = evidence_dir("revisions");
    lock.begin_operation("op-1", &evidence_root, &ops).unwrap();
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
        snapshot(&slot(&dir)).keys().cloned().collect::<Vec<_>>(),
        vec![adv::LOCK_FILE.to_string(), adv::STATE_FILE.to_string()],
        "no temp file may survive a successful write"
    );
    let record = read_state(&dir);
    assert_eq!(record.revision, 5);
    assert_eq!(record.state, adv::State::Complete);
}

// --------------------------------------------------- on-disk state recheck ----

#[cfg(unix)]
#[test]
fn a_state_deleted_after_open_is_not_rebuilt() {
    let dir = root("disk-delete");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    let evidence_root = evidence_root("disk-delete");
    lock.begin_operation("op-1", &evidence_root, &evidence_dir("disk-delete"))
        .unwrap();
    let state = adv::state_path(&slot(&dir));
    fs::remove_file(&state).unwrap();

    let err = lock
        .record_intent("sheets/a.yaml", "before", "after")
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::CorruptState);
    assert!(!state.exists(), "a missing state must never be rebuilt");
    assert_eq!(lock.state().state, adv::State::Active);

    assert_eq!(
        lock.finish_failed("stop").unwrap_err().kind(),
        adv::ErrorKind::CorruptState
    );
    assert!(!state.exists());
}

#[cfg(unix)]
#[test]
fn a_state_zeroed_corrupted_or_replaced_after_open_stops_every_update() {
    fn case(tag: &str, mutate: impl Fn(&Path)) {
        let dir = root(tag);
        init_ok(&dir);
        let mut lock = open_ok(&dir);
        lock.begin_operation("op-1", &evidence_root(tag), &evidence_dir(tag))
            .unwrap();
        let state = adv::state_path(&slot(&dir));
        mutate(&state);
        let before = fs::read(&state).unwrap();

        let err = lock
            .record_intent("sheets/a.yaml", "before", "after")
            .unwrap_err();
        assert_eq!(err.kind(), adv::ErrorKind::CorruptState, "{tag}");
        assert_eq!(
            fs::read(&state).unwrap(),
            before,
            "{tag}: must not overwrite"
        );
        assert_eq!(lock.state().state, adv::State::Active);
    }

    case("disk-zero", |state| fs::write(state, b"").unwrap());
    case("disk-garbage", |state| {
        fs::write(state, b"{not json").unwrap()
    });
    case("disk-replaced", |state| {
        // A self-consistent but different revision must not be accepted.
        let mut record: adv::StateRecord =
            serde_json::from_slice(&fs::read(state).unwrap()).unwrap();
        record.revision = 99;
        adv::reseal(&mut record).unwrap();
        fs::write(state, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    });
}

#[cfg(unix)]
#[test]
fn check_writable_rechecks_the_disk_state() {
    let dir = root("predicate-disk");
    init_ok(&dir);
    let lock = open_ok(&dir);
    lock.check_writable().unwrap();
    fs::remove_file(adv::state_path(&slot(&dir))).unwrap();
    let err = lock.check_writable().unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::CorruptState);
}

#[cfg(unix)]
#[test]
fn a_state_changed_between_open_and_begin_operation_is_refused() {
    let dir = root("pre-begin-change");
    init_ok(&dir);
    let mut lock = open_ok(&dir);
    // A self-consistent replacement between `open` and the operation start:
    // the pre-state step must read and validate it, not trust the cached copy.
    let mut record = read_state(&dir);
    record.revision = 99;
    adv::reseal(&mut record).unwrap();
    write_state(&dir, &record);
    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();

    let err = lock
        .begin_operation(
            "op-1",
            &evidence_root("pre-begin-change"),
            &evidence_dir("pre-begin-change"),
        )
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::CorruptState);
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
    let on_disk: adv::StateRecord = serde_json::from_slice(&before).unwrap();
    assert_eq!(on_disk.state, adv::State::Ready);
    assert_eq!(on_disk.revision, 99);
    assert_eq!(lock.state().state, adv::State::Ready);
}

// -------------------------------------------------------- pre-state (D) ----

#[cfg(unix)]
#[test]
fn begin_operation_snapshots_the_previous_state_before_becoming_active() {
    let dir = root("prestate");
    init_ok(&dir);
    let evidence_root = evidence_root("prestate");
    let ops = evidence_dir("prestate");
    let mut lock = open_ok(&dir);
    lock.begin_operation("op-1", &evidence_root, &ops).unwrap();

    assert_eq!(lock.state().state, adv::State::Active);
    assert_eq!(lock.state().stage, adv::Stage::Admit);
    assert_eq!(lock.state().operation_id, "op-1");

    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(ops.join("op-1-prestate.json")).unwrap()).unwrap();
    assert_eq!(doc["prior_state"], "ready");
    assert_eq!(doc["prior_stage"], "init");
    assert_eq!(doc["state_revision"], 1);
    assert_eq!(doc["save_path"], "Saves/slot");
    assert!(doc["state_bytes"].as_str().unwrap().contains("\"ready\""));
    // The recorded bytes are the ones that were validated: they re-parse to the
    // same revision and checksum, with no second read in between.
    let prior: adv::StateRecord =
        serde_json::from_str(doc["state_bytes"].as_str().unwrap()).unwrap();
    assert_eq!(prior.state, adv::State::Ready);
    assert_eq!(prior.revision, 1);
    assert_eq!(doc["state_checksum"], prior.checksum);
}

#[cfg(unix)]
#[test]
fn a_failed_prestate_snapshot_leaves_the_state_untouched() {
    let dir = root("prestate-fail");
    init_ok(&dir);
    let evidence_root = evidence_root("prestate-fail");
    let ops = evidence_dir("prestate-fail");
    fs::create_dir_all(&ops).unwrap();
    // The exclusive create must fail: this is the injectable failure.
    fs::write(ops.join("op-1-prestate.json"), b"stale evidence").unwrap();

    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();
    let mut lock = open_ok(&dir);
    let err = lock
        .begin_operation("op-1", &evidence_root, &ops)
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::EvidenceFailed);
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
    assert_eq!(lock.state().state, adv::State::Ready);
    lock.check_writable().unwrap();
}

#[cfg(unix)]
#[test]
fn begin_operation_creates_and_syncs_nested_evidence_directories() {
    let dir = root("prestate-nested");
    init_ok(&dir);
    let evidence_root = dir.join("ops");
    fs::create_dir(&evidence_root).unwrap();
    let ops = evidence_root.join("a").join("b").join("c");
    assert!(!ops.exists());
    let mut lock = open_ok(&dir);
    lock.begin_operation("op-1", &evidence_root, &ops).unwrap();

    assert!(ops.is_dir());
    assert!(ops.join("op-1-prestate.json").is_file());
    assert_eq!(lock.state().state, adv::State::Active);
}

#[cfg(unix)]
#[test]
fn begin_operation_fails_when_a_directory_cannot_be_created() {
    let dir = root("prestate-dirfail");
    init_ok(&dir);
    let evidence_root = dir.join("ops");
    fs::create_dir(&evidence_root).unwrap();
    // A regular file in the path makes the level below it impossible.
    let blocker = evidence_root.join("a");
    fs::write(&blocker, b"not a directory").unwrap();
    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();

    let mut lock = open_ok(&dir);
    let err = lock
        .begin_operation("op-1", &evidence_root, &blocker.join("sub"))
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::EvidenceFailed);
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
    assert_eq!(lock.state().state, adv::State::Ready);
}

#[cfg(unix)]
#[test]
fn every_retry_rechecks_all_evidence_levels() {
    use std::os::unix::fs::PermissionsExt;

    let dir = root("retry-sync");
    init_ok(&dir);
    // `ops` is the trusted, syncable root; `ops/a` is the level that cannot be
    // opened for fsync, so `mkdir` below it works while syncing its entry fails.
    let evidence_root = dir.join("ops");
    fs::create_dir(&evidence_root).unwrap();
    let broken = evidence_root.join("a");
    fs::create_dir(&broken).unwrap();
    let mut perms = fs::metadata(&broken).unwrap().permissions();
    let mode = perms.mode();
    perms.set_mode(0o333);
    fs::set_permissions(&broken, perms).unwrap();

    if fs::File::open(&broken).is_ok() {
        let mut perms = fs::metadata(&broken).unwrap().permissions();
        perms.set_mode(mode);
        fs::set_permissions(&broken, perms).unwrap();
        eprintln!("skipped: this user can open a 0o333 directory");
        return;
    }

    let evidence = broken.join("b").join("c");
    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();
    let mut lock = open_ok(&dir);
    // Every attempt walks all levels from the root again. The first failure
    // leaves `b` behind and the second leaves `c`, but `a`'s own entry can
    // still not be synced, so every retry must fail instead of skipping the
    // level that failed first.
    for (index, operation) in ["op-1", "op-2", "op-3"].iter().enumerate() {
        let err = lock
            .begin_operation(operation, &evidence_root, &evidence)
            .unwrap_err();
        assert_eq!(
            err.kind(),
            adv::ErrorKind::EvidenceFailed,
            "attempt {}",
            index + 1
        );
        assert_eq!(
            fs::read(adv::state_path(&slot(&dir))).unwrap(),
            before,
            "attempt {} must not touch the state",
            index + 1
        );
        assert_eq!(lock.state().state, adv::State::Ready);
    }

    // Restoring the level lets the same path succeed.
    let mut perms = fs::metadata(&broken).unwrap().permissions();
    perms.set_mode(mode);
    fs::set_permissions(&broken, perms).unwrap();
    lock.begin_operation("op-4", &evidence_root, &evidence)
        .unwrap();
    assert_eq!(lock.state().state, adv::State::Active);
}

#[cfg(unix)]
#[test]
fn begin_operation_requires_an_existing_evidence_root() {
    let dir = root("evidence-root-missing");
    init_ok(&dir);
    let evidence_root = dir.join("absent-root");
    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();
    let mut lock = open_ok(&dir);
    let err = lock
        .begin_operation("op-1", &evidence_root, &evidence_root.join("ops"))
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::EvidenceFailed);
    assert!(!evidence_root.exists(), "the root must never be created");
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
    assert_eq!(lock.state().state, adv::State::Ready);
}

#[cfg(unix)]
#[test]
fn begin_operation_rejects_an_evidence_root_that_is_not_a_directory() {
    let dir = root("evidence-root-file");
    init_ok(&dir);
    let evidence_root = dir.join("root-file");
    fs::write(&evidence_root, b"not a directory").unwrap();
    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();
    let mut lock = open_ok(&dir);
    let err = lock
        .begin_operation("op-1", &evidence_root, &evidence_root.join("ops"))
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::EvidenceFailed);
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
    assert_eq!(lock.state().state, adv::State::Ready);
}

#[cfg(unix)]
#[test]
fn begin_operation_rejects_an_evidence_dir_outside_its_root() {
    let dir = root("evidence-outside");
    init_ok(&dir);
    let evidence_root = dir.join("ops");
    fs::create_dir(&evidence_root).unwrap();
    let other = dir.join("other");
    fs::create_dir(&other).unwrap();
    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();
    let mut lock = open_ok(&dir);
    let err = lock
        .begin_operation("op-1", &evidence_root, &other.join("ops"))
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::InvalidRequest);
    assert!(err.detail().contains("outside"), "{}", err.detail());
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
    assert_eq!(lock.state().state, adv::State::Ready);
}

#[cfg(unix)]
#[test]
fn begin_operation_fails_when_the_evidence_root_cannot_be_synced() {
    use std::os::unix::fs::PermissionsExt;

    let dir = root("evidence-root-nosync");
    init_ok(&dir);
    let evidence_root = dir.join("ops");
    fs::create_dir(&evidence_root).unwrap();
    let mut perms = fs::metadata(&evidence_root).unwrap().permissions();
    let mode = perms.mode();
    perms.set_mode(0o333);
    fs::set_permissions(&evidence_root, perms).unwrap();
    if fs::File::open(&evidence_root).is_ok() {
        let mut perms = fs::metadata(&evidence_root).unwrap().permissions();
        perms.set_mode(mode);
        fs::set_permissions(&evidence_root, perms).unwrap();
        eprintln!("skipped: this user can open a 0o333 directory");
        return;
    }

    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();
    let mut lock = open_ok(&dir);
    let err = lock
        .begin_operation("op-1", &evidence_root, &evidence_root.join("ops"))
        .unwrap_err();
    let mut perms = fs::metadata(&evidence_root).unwrap().permissions();
    perms.set_mode(mode);
    fs::set_permissions(&evidence_root, perms).unwrap();

    assert_eq!(err.kind(), adv::ErrorKind::EvidenceFailed);
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
    assert_eq!(lock.state().state, adv::State::Ready);
}

#[cfg(unix)]
#[test]
fn begin_operation_rejects_a_symlinked_evidence_level() {
    use std::os::unix::fs::symlink;

    let dir = root("evidence-symlink");
    init_ok(&dir);
    let evidence_root = dir.join("ops");
    fs::create_dir(&evidence_root).unwrap();
    let target = dir.join("target");
    fs::create_dir(&target).unwrap();
    let link = evidence_root.join("link");
    symlink(&target, &link).unwrap();

    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();
    let mut lock = open_ok(&dir);
    let err = lock
        .begin_operation("op-1", &evidence_root, &link.join("ops"))
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::EvidenceFailed);
    assert!(
        !target.join("ops").exists(),
        "nothing may be created through the link"
    );
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
    assert_eq!(lock.state().state, adv::State::Ready);
}

#[cfg(unix)]
#[test]
fn begin_operation_rejects_a_non_utf8_evidence_component() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = root("evidence-non-utf8");
    init_ok(&dir);
    let evidence_root = dir.join("ops");
    fs::create_dir(&evidence_root).unwrap();
    let bad = evidence_root.join(OsString::from_vec(b"bad-\xff".to_vec()));
    if fs::create_dir(&bad).is_err() {
        eprintln!("skipped: this filesystem rejects a non-UTF-8 directory name");
        return;
    }

    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();
    let mut lock = open_ok(&dir);
    let err = lock
        .begin_operation("op-1", &evidence_root, &bad.join("ops"))
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::InvalidRequest);
    assert!(err.detail().contains("not valid UTF-8"), "{}", err.detail());
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn begin_operation_rejects_a_non_utf8_evidence_root() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = root("evidence-non-utf8-root");
    init_ok(&dir);
    let evidence_root = dir.join(OsString::from_vec(b"root-\xff".to_vec()));
    if let Err(e) = fs::create_dir(&evidence_root) {
        // The filesystem refusing the name is the expected "unsupported".
        eprintln!("skipped: this filesystem refuses a non-UTF-8 directory name ({e})");
        return;
    }
    // A plain UTF-8 child: only the root itself is not valid UTF-8.
    let evidence = evidence_root.join("ops");
    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();
    let mut lock = open_ok(&dir);
    let err = lock
        .begin_operation("op-1", &evidence_root, &evidence)
        .unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::InvalidRequest);
    assert!(err.detail().contains("not valid UTF-8"), "{}", err.detail());
    assert_eq!(fs::read(adv::state_path(&slot(&dir))).unwrap(), before);
    assert_eq!(lock.state().state, adv::State::Ready);
    assert!(!evidence.exists(), "nothing may be created");
}

#[cfg(unix)]
#[test]
fn a_backslash_in_a_save_component_is_refused() {
    let dir = root("backslash");
    let slot = dir.join("Saves").join("a\\b");
    fs::create_dir_all(&slot).unwrap();
    let err = adv::init(&dir, &slot, &init_opts()).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::InvalidRequest);
    assert!(err.detail().contains("backslash"), "{}", err.detail());
}

#[cfg(not(unix))]
#[test]
fn init_reports_published_but_not_durable_and_refuses_every_commit() {
    let dir = root("prestate-nosync");
    make_slot(&dir);
    let err = adv::init(&dir, &slot(&dir), &init_opts()).unwrap_err();
    assert_eq!(
        err.kind(),
        adv::ErrorKind::StatePublishedNotDurable,
        "{err}"
    );
    assert!(
        err.detail().contains("durability is unknown"),
        "the message must say the durability is unknown: {}",
        err.detail()
    );
    // Both protocol files stay: this is not a failed creation to retry.
    assert!(adv::lock_path(&slot(&dir)).is_file());
    assert!(adv::state_path(&slot(&dir)).is_file());
    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();

    // The state is readable, but nothing may write.
    let mut lock = open_ok(&dir);
    assert_eq!(lock.state().state, adv::State::Ready);
    assert_eq!(
        lock.check_writable().unwrap_err().kind(),
        adv::ErrorKind::UnwritableState
    );
    assert_eq!(
        lock.begin_operation(
            "op-1",
            &evidence_root("prestate-nosync"),
            &evidence_dir("prestate-nosync"),
        )
        .unwrap_err()
        .kind(),
        adv::ErrorKind::UnwritableState
    );
    assert_eq!(
        fs::read(adv::state_path(&slot(&dir))).unwrap(),
        before,
        "the published state must survive byte-identically"
    );
    assert_eq!(lock.state().state, adv::State::Ready);
}

// ------------------------------------------------------------- copying ----

#[test]
fn a_copied_save_with_another_path_is_refused() {
    let from = root("copy-source");
    init_ok(&from);
    let to = root("copy-target");
    let target = to.join("Saves").join("other");
    copy_dir(&slot(&from), &target);

    let err = open_at(&to, &target).unwrap_err();
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
    assert_eq!(
        snapshot(&target),
        snapshot(&slot(&from)),
        "a refused copy must stay byte-identical"
    );
}

#[cfg(unix)]
#[test]
fn a_copied_save_with_the_same_relative_path_is_refused_by_lock_identity() {
    let from = root("copy-same-source");
    init_ok(&from);
    let to = root("copy-same-target");
    copy_dir(&slot(&from), &slot(&to));

    // The copy keeps `save_path`, so only the file identity can notice it.
    // Unix has an exact `dev`+`ino`; the Windows best-effort identity may be
    // preserved by a copy, which is why this case is Unix-only and the module
    // documents "may not be detectable".
    let err = open_at(&to, &slot(&to)).unwrap_err();
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
    let err = open_at(&dir, &slot(&dir)).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::LockTimeout);
    drop(lock);
    open_ok(&dir);
}

#[test]
fn a_second_process_times_out_and_never_takes_over() {
    let dir = root("timeout");
    init_ok(&dir);
    let lock = open_ok(&dir);
    // A byte snapshot cannot be read while this process holds the lock on
    // Windows, so compare names/sizes and the (unlocked) state bytes.
    let before = footprint(&slot(&dir));
    let state_before = fs::read(adv::state_path(&slot(&dir))).unwrap();

    let result = run_child("open", &dir);
    assert_eq!(result, "ERR:lock_timeout");
    assert_eq!(
        footprint(&slot(&dir)),
        before,
        "the waiter must not write anything"
    );
    assert_eq!(
        fs::read(adv::state_path(&slot(&dir))).unwrap(),
        state_before,
        "the waiter must not touch the state"
    );
    assert_eq!(lock.state().state, adv::State::Ready);
    drop(lock);
}

#[test]
fn two_processes_race_init_and_exactly_one_wins() {
    let dir = root("init-race");
    make_slot(&dir);
    let children: Vec<Child> = (0..4).map(|_| spawn_child("init", &dir)).collect();
    let results: Vec<String> = children.into_iter().map(wait_child).collect();

    // One initializer wins; the others lose at the lock's `create_new`. On a
    // platform without directory fsync the winner publishes and then reports
    // that durability is unknown instead of returning `Ok`.
    let winners: Vec<&String> = results
        .iter()
        .filter(|result| *result != "ERR:already_initialized")
        .collect();
    assert_eq!(winners.len(), 1, "exactly one init may win: {results:?}");
    #[cfg(unix)]
    assert_eq!(winners[0], "OK", "{results:?}");
    #[cfg(not(unix))]
    assert_eq!(winners[0], "ERR:state_published_not_durable", "{results:?}");
    for loser in results.iter().filter(|r| *r != "OK") {
        assert!(
            loser == "ERR:already_initialized" || loser == "ERR:state_published_not_durable",
            "{results:?}"
        );
    }
    assert_eq!(
        snapshot(&slot(&dir)).keys().cloned().collect::<Vec<_>>(),
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
    let before = fs::read(adv::state_path(&slot(&dir))).unwrap();

    let replacement = slot(&dir).join("replacement");
    fs::write(&replacement, b"not the locked inode").unwrap();
    fs::rename(&replacement, adv::lock_path(&slot(&dir))).unwrap();

    let err = lock.finish_in_doubt("stop").unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::PathReplaced);
    assert_eq!(
        fs::read(adv::state_path(&slot(&dir))).unwrap(),
        before,
        "a replaced lock aborts before any state write"
    );
}

#[cfg(unix)]
#[test]
fn init_stops_when_the_path_is_replaced_before_the_first_identity_check() {
    let dir = root("t2f1");
    make_slot(&dir);
    let replacement = slot(&dir).join("replacement");
    fs::write(&replacement, b"not the locked inode").unwrap();
    let replace = {
        let replacement = replacement.clone();
        move |path: &Path| fs::rename(&replacement, path).unwrap()
    };
    let mut opts = adv::InitOptions::new();
    opts.acquire = fast();
    opts.hooks = adv::InitHooks {
        after_create: Some(&replace),
        before_publish: None,
    };

    let err = adv::init(&dir, &slot(&dir), &opts).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::PathReplaced);
    assert!(
        !adv::state_path(&slot(&dir)).exists(),
        "no state may be published after a replacement"
    );
    assert_eq!(
        fs::read(adv::lock_path(&slot(&dir))).unwrap(),
        b"not the locked inode",
        "the replacement must not receive a header"
    );
}

#[cfg(unix)]
#[test]
fn init_stops_when_the_path_is_replaced_after_the_header_before_publish() {
    let dir = root("t2f2");
    make_slot(&dir);
    let replacement = slot(&dir).join("replacement");
    fs::write(&replacement, b"not the locked inode").unwrap();
    let replace = {
        let replacement = replacement.clone();
        move |path: &Path| fs::rename(&replacement, path).unwrap()
    };
    let mut opts = adv::InitOptions::new();
    opts.acquire = fast();
    opts.hooks = adv::InitHooks {
        after_create: None,
        before_publish: Some(&replace),
    };

    let err = adv::init(&dir, &slot(&dir), &opts).unwrap_err();
    assert_eq!(err.kind(), adv::ErrorKind::PathReplaced);
    assert!(
        !adv::state_path(&slot(&dir)).exists(),
        "no state may be published after a replacement"
    );
    // The header was written to the original (now unlinked) inode; the
    // replacement file proves the tool cannot claim it skipped the header.
    assert_eq!(
        fs::read(adv::lock_path(&slot(&dir))).unwrap(),
        b"not the locked inode"
    );
    // A later normal open fails closed: lock exists, state does not.
    assert_eq!(
        open_at(&dir, &slot(&dir)).unwrap_err().kind(),
        adv::ErrorKind::CorruptState
    );
}

#[cfg(unix)]
#[test]
fn a_read_only_directory_stops_a_state_write_and_keeps_the_old_state() {
    use std::os::unix::fs::PermissionsExt;

    let dir = root("read-only-write");
    init_ok(&dir);
    let evidence_root = evidence_root("read-only-write");
    let ops = evidence_dir("read-only-write");
    let mut lock = open_ok(&dir);
    lock.begin_operation("op-1", &evidence_root, &ops).unwrap();
    let save = slot(&dir);
    let before = fs::read(adv::state_path(&save)).unwrap();

    let mut perms = fs::metadata(&save).unwrap().permissions();
    let mode = perms.mode();
    perms.set_mode(0o555);
    fs::set_permissions(&save, perms).unwrap();

    // A privileged user ignores the mode; never pretend the injection worked.
    if fs::write(save.join("probe"), b"x").is_ok() {
        let _ = fs::remove_file(save.join("probe"));
        let mut perms = fs::metadata(&save).unwrap().permissions();
        perms.set_mode(mode);
        fs::set_permissions(&save, perms).unwrap();
        eprintln!("skipped: this user can write into a 0o555 directory");
        return;
    }

    let result = lock.finish_in_doubt("stop");
    let mut perms = fs::metadata(&save).unwrap().permissions();
    perms.set_mode(mode);
    fs::set_permissions(&save, perms).unwrap();

    assert_eq!(result.unwrap_err().kind(), adv::ErrorKind::Io);
    assert_eq!(fs::read(adv::state_path(&save)).unwrap(), before);
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
    let save = dir.join("Saves").join("slot");
    let result = match role.as_str() {
        "open" => match adv::open(&dir, &save, &open_opts()) {
            Ok(_) => "OK".to_string(),
            Err(e) => format!("ERR:{}", e.kind().as_str()),
        },
        "init" => match adv::init(&dir, &save, &init_opts()) {
            Ok(_) => "OK".to_string(),
            Err(e) => format!("ERR:{}", e.kind().as_str()),
        },
        other => panic!("unknown child role {other}"),
    };
    println!("RESULT:{result}");
}
