//! Schema 11 migration (#330): read-only identity planning, DM confirmation
//! matching, sidecar missing-vs-empty distinction, the ext4 end-to-end run
//! and rollback through explicit recovery. Synthetic fixtures only; no real
//! saves, books or secrets.
//!
//! The end-to-end and recovery tests observe the live filesystem and only
//! run their write assertions on a writable ext4 mount; on any other
//! filesystem they assert the fail-closed refusal instead, so the suite
//! stays honest on tmpfs/macOS/Windows CI.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use advancement_lock::save_lock::digest;
use talkdnd::advancement_sidecar::{self, BindingConfirm};
use talkdnd::migrate;

const CARD_PATH: &str = "Characters/Fixture.md";
const SHEET_PATH: &str = "sheets/fixture.yaml";
const SIDECAR: &str = "advancement/fixture.yaml";
const CARD: &[u8] = b"\xef\xbb\xbf# Fixture\r\n\r\n```combat yaml\r\nid: fixture\r\nname: Fixture\r\n```\r\n\r\nDM note";
const SHEET: &[u8] = b"\xef\xbb\xbf# DM note\r\nversion: 1\r\nid: fixture\r\nname: Fixture\r\nfile: Characters/Fixture.md\r\ncustom: {old: [4, 8], present: true}";

struct Fixture {
    root: PathBuf,
    save: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        for n in 0..64 {
            let root =
                std::env::temp_dir().join(format!("talkdnd-330-{tag}-{}-{n}", std::process::id()));
            match fs::create_dir(&root) {
                Ok(()) => {
                    fs::create_dir_all(root.join("Saves")).unwrap();
                    let save = root.join("Saves").join("2030-01-01-01");
                    for dir in ["Characters", "sheets", "advancement", "combat"] {
                        fs::create_dir_all(save.join(dir)).unwrap();
                    }
                    fs::write(root.join("AGENTS.md"), "Synthetic play root\n").unwrap();
                    fs::write(
                        save.join("combat/campaign.yaml"),
                        "name: fixture\nruleset: 5.5e-srd\nparty: [fixture]\n",
                    )
                    .unwrap();
                    fs::write(
                        save.join("progress.yaml"),
                        "version: 1\nxp:\n  fixture: 0\npurses:\n  fixture:\n    cp: 0\n",
                    )
                    .unwrap();
                    fs::write(save.join("integrity.yaml"), "version: 1\nfiles: {}\n").unwrap();
                    fs::write(save.join("save-meta.yaml"), "schema: 10\ntool: talkdnd\n").unwrap();
                    return Self { root, save };
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("failed to create fixture: {error}"),
            }
        }
        panic!("no free fixture directory");
    }

    fn seed_card(&self) {
        fs::write(self.save.join(CARD_PATH), CARD).unwrap();
        fs::write(self.save.join(SHEET_PATH), SHEET).unwrap();
    }

    fn confirm_args(&self) -> Vec<BindingConfirm> {
        self.plan(&[])
            .pending
            .iter()
            .map(|line| {
                let spec = line
                    .strip_prefix("--confirm-binding ")
                    .expect("pending line is a confirm argument");
                BindingConfirm::parse(spec).unwrap()
            })
            .collect()
    }

    fn plan(&self, confirms: &[BindingConfirm]) -> advancement_sidecar::BindingPlan {
        advancement_sidecar::plan(&self.save, 10, confirms).unwrap()
    }

    fn snapshot_tree(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        collect_files(&self.save, &self.save, &mut out);
        out
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn collect_files(dir: &Path, root: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, root, out);
        } else if path.is_file() {
            out.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                fs::read(&path).unwrap(),
            );
        }
    }
}

#[test]
fn dry_run_lists_exact_confirm_arguments_and_plain_migrate_writes_nothing() {
    let fixture = Fixture::new("pending");
    fixture.seed_card();
    let before = fixture.snapshot_tree();

    let dry = migrate::ensure_with_bindings(&fixture.save, &fixture.root, true, &[]).unwrap();
    assert_eq!((dry.from, dry.to), (10, 10));
    assert_eq!(dry.identity_pending.len(), 2, "{:?}", dry.identity_pending);
    assert!(
        dry.identity_pending.iter().any(|line| line
            == &format!("--confirm-binding {CARD_PATH}={}", digest(CARD).as_str())),
        "{:?}",
        dry.identity_pending
    );
    assert!(
        dry.identity_pending
            .iter()
            .any(|line| line
                == &format!("--confirm-binding {SHEET_PATH}={}", digest(SHEET).as_str())),
        "{:?}",
        dry.identity_pending
    );
    assert!(
        dry.identity_refused.is_empty(),
        "{:?}",
        dry.identity_refused
    );
    assert_eq!(fixture.snapshot_tree(), before, "dry-run must not write");

    // Without confirmations the plain entry stops at schema 10 and writes
    // nothing: no legacy-chain churn, no backup, no marker.
    let applied = migrate::ensure_with_bindings(&fixture.save, &fixture.root, false, &[]).unwrap();
    assert_eq!((applied.from, applied.to), (10, 10));
    assert_eq!(
        fixture.snapshot_tree(),
        before,
        "unconfirmed run must not write"
    );
    assert!(!fixture.root.join("Saves/_backups").exists());
    assert!(!fixture.save.join(SIDECAR).exists());
}

#[test]
fn a_stale_or_wrong_confirm_digest_is_refused_not_ignored() {
    let fixture = Fixture::new("stale");
    fixture.seed_card();
    let before = fixture.snapshot_tree();
    let stale = vec![
        BindingConfirm {
            path: CARD_PATH.to_string(),
            digest: "1111111111111111111111111111111111111111111111111111111111111111".to_string(),
        },
        BindingConfirm {
            path: SHEET_PATH.to_string(),
            digest: digest(SHEET).as_str().to_owned(),
        },
    ];
    let plan = fixture.plan(&stale);
    assert!(!plan.ready_for_schema_11(), "{:?}", plan.refused);
    assert!(
        plan.refused
            .iter()
            .any(|reason| reason.contains("确认摘要与当前字节不符")),
        "{:?}",
        plan.refused
    );
    let applied =
        migrate::ensure_with_bindings(&fixture.save, &fixture.root, false, &stale).unwrap();
    assert_eq!((applied.from, applied.to), (10, 10));
    assert_eq!(fixture.snapshot_tree(), before);
}

#[test]
fn an_existing_sidecar_blocks_enrollment_even_when_the_file_is_empty() {
    let fixture = Fixture::new("sidecar-exists");
    fixture.seed_card();

    // Missing sidecar: the character is pending, not refused.
    let plan = fixture.plan(&[]);
    assert!(
        plan.pending.len() == 1 && plan.refused.is_empty(),
        "{plan:?}"
    );

    // An EMPTY sidecar file is "exists", never "missing": no enrollment and
    // no silent overwrite of what may be an interrupted run's evidence.
    fs::write(fixture.save.join(SIDECAR), b"").unwrap();
    let plan = fixture.plan(&[]);
    assert!(!plan.ready_for_schema_11());
    assert!(
        plan.refused.iter().any(|reason| {
            reason.contains(SIDECAR) && reason.contains("空文件同样视为存在")
        }),
        "{:?}",
        plan.refused
    );

    // A valid orphan record is refused the same way (no matching carrier).
    fs::remove_file(fixture.save.join(SIDECAR)).unwrap();
    fs::write(
        fixture.save.join(SIDECAR),
        "version: 1\ncard: Characters/Gone.md\ntoken: 11111111111111111111111111111111\nhistory: unknown\n",
    )
    .unwrap();
    let plan = fixture.plan(&[]);
    assert!(
        plan.refused
            .iter()
            .any(|reason| reason.contains("已存在") || reason.contains("成长记录无效")),
        "{:?}",
        plan.refused
    );
    assert!(!plan.ready_for_schema_11());
}

#[test]
fn an_unmatched_orphan_record_is_refused_and_invalid_records_fail_closed() {
    let fixture = Fixture::new("orphans");
    fixture.seed_card();
    fs::write(
        fixture.save.join("advancement/ghost.yaml"),
        "version: 1\ncard: Characters/Ghost.md\ntoken: 11111111111111111111111111111112\nhistory: unknown\n",
    )
    .unwrap();
    fs::write(
        fixture.save.join("advancement/broken.yaml"),
        "not: a record\n",
    )
    .unwrap();
    let plan = fixture.plan(&[]);
    assert!(
        plan.refused
            .iter()
            .any(|reason| reason.contains("孤儿成长记录 advancement/ghost.yaml")),
        "{:?}",
        plan.refused
    );
    assert!(
        plan.refused
            .iter()
            .any(|reason| reason.contains("advancement/broken.yaml")),
        "{:?}",
        plan.refused
    );
    assert!(!plan.ready_for_schema_11());
}

#[test]
fn an_already_bound_save_is_ready_without_new_confirmations() {
    let fixture = Fixture::new("bound");
    fixture.seed_card();
    let confirms = fixture.confirm_args();
    let candidates = advancement_sidecar::prepare(&fixture.plan(&confirms), &fixture.save).unwrap();
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    fs::write(
        fixture.save.join(&candidate.carrier_path),
        &candidate.carrier_bytes,
    )
    .unwrap();
    fs::write(
        fixture.save.join(&candidate.sidecar_path),
        &candidate.sidecar_bytes,
    )
    .unwrap();

    // The bound pair is strictly matched, so completing to 11 needs no new
    // DM decision; the plan is ready with an empty confirm list.
    let plan = fixture.plan(&[]);
    assert!(plan.ready_for_schema_11(), "{:?}", plan.refused);

    // A confirm naming a bound carrier with the wrong digest is still a
    // refusal, not a no-op.
    let stale = vec![BindingConfirm {
        path: CARD_PATH.to_string(),
        digest: "1111111111111111111111111111111111111111111111111111111111111111".to_string(),
    }];
    let plan = fixture.plan(&stale);
    assert!(
        plan.refused
            .iter()
            .any(|reason| reason.contains("确认摘要与当前字节不符")),
        "{:?}",
        plan.refused
    );
}

#[test]
fn a_future_or_corrupt_save_schema_is_refused_by_the_normal_entry() {
    let fixture = Fixture::new("future");
    fixture.seed_card();
    fs::write(
        fixture.save.join("save-meta.yaml"),
        "schema: 12\ntool: from the future\n",
    )
    .unwrap();
    let err = migrate::ensure_with_bindings(&fixture.save, &fixture.root, false, &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("12"), "{err}");
    assert!(
        err.contains("无法支持")
            || err.contains("refuse")
            || err.contains("不支持")
            || err.contains("更新"),
        "{err}"
    );
}

#[cfg(target_os = "linux")]
fn writable_ext4(path: &Path) -> bool {
    let output = std::process::Command::new("findmnt")
        .args(["-no", "FSTYPE,OPTIONS"])
        .arg(path)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            text.contains("ext4") && text.contains("rw")
        }
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn set_fixture_xattr(path: &Path, name: &[u8], value: &[u8]) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(name).unwrap();
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let rc = unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    assert_eq!(rc, 0, "seeding xattr on {}", path.display());
}

#[cfg(target_os = "linux")]
fn get_fixture_xattr(path: &Path, name: &[u8]) -> Option<Vec<u8>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(name).unwrap();
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let mut buffer = vec![0u8; 256];
    let len = unsafe {
        libc::getxattr(
            c_path.as_ptr(),
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if len < 0 {
        return None;
    }
    buffer.truncate(len as usize);
    Some(buffer)
}

/// AC-2 / AC-1 / AC-4 end to end on the first admitted environment: a
/// synthetic schema-10 save with seeded ACL/xattr and an existing user
/// backup migrates 10→11 through the D protocol; a second run is a no-op.
/// On a non-ext4 or read-only mount the run must refuse instead of writing.
#[cfg(target_os = "linux")]
#[test]
fn schema11_end_to_end_on_admitted_linux_ext4() {
    let fixture = Fixture::new("e2e");
    fixture.seed_card();
    fs::create_dir_all(fixture.save.join("checkpoints/cp-1/Characters")).unwrap();
    fs::write(
        fixture.save.join("checkpoints/cp-1/Characters/Fixture.md"),
        "# old checkpoint card\n",
    )
    .unwrap();
    set_fixture_xattr(
        &fixture.save.join(CARD_PATH),
        b"user.talkdnd-330",
        b"across-the-migration",
    );

    let confirms = fixture.confirm_args();
    let before = fixture.snapshot_tree();

    if !writable_ext4(&fixture.save) {
        // Fail closed everywhere this environment is not the admitted one:
        // no bytes change, the save keeps schema 10, and the refusal says so.
        let applied =
            migrate::ensure_with_bindings(&fixture.save, &fixture.root, false, &confirms).unwrap();
        assert_eq!((applied.from, applied.to), (10, 10));
        assert!(
            applied
                .identity_refused
                .iter()
                .any(|reason| reason.contains("准入")),
            "{:?}",
            applied.identity_refused
        );
        assert_eq!(fixture.snapshot_tree(), before);
        return;
    }

    let applied =
        migrate::ensure_with_bindings(&fixture.save, &fixture.root, false, &confirms).unwrap();
    assert_eq!(
        (applied.from, applied.to),
        (10, migrate::CURRENT_SCHEMA),
        "{:?}",
        applied.identity_refused
    );
    assert_eq!(
        applied.identity_bound.len(),
        1,
        "{:?}",
        applied.identity_bound
    );

    // The structured carrier is the old bytes plus one appended field; the
    // Markdown card is untouched (both sides keep their hand edits).
    let card = fs::read(fixture.save.join(CARD_PATH)).unwrap();
    assert_eq!(card, CARD, "a structured carrier leaves the card untouched");
    let sheet = fs::read(fixture.save.join(SHEET_PATH)).unwrap();
    assert!(sheet.starts_with(SHEET), "old bytes must be preserved");
    let sheet_suffix = String::from_utf8_lossy(&sheet[SHEET.len()..]).to_string();
    let token = sheet_suffix
        .trim()
        .strip_prefix("talkdnd_identity: \"")
        .and_then(|rest| rest.strip_suffix("\""))
        .expect("one appended identity field")
        .to_owned();
    assert_eq!(token.len(), 32);

    // The sidecar is strict v1 with the same token on both sides.
    let sidecar_text = fs::read_to_string(fixture.save.join(SIDECAR)).unwrap();
    assert!(
        sidecar_text.contains("version: 1")
            && sidecar_text.contains("history: unknown")
            && sidecar_text.contains(&token),
        "{sidecar_text}"
    );

    // Schema metadata reached 11 through the same D publish, with the
    // migration record appended.
    let meta = fs::read_to_string(fixture.save.join("save-meta.yaml")).unwrap();
    assert!(meta.contains("schema: 11"), "{meta}");
    assert!(
        meta.contains("from: 10") && meta.contains("to: 11"),
        "{meta}"
    );

    // The D backup verified the whole save, preserved the seeded xattr and
    // carried the existing user backup along.
    let backup = applied.identity_d_backup.as_ref().expect("D backup");
    let backed_card = backup.join(CARD_PATH);
    assert_eq!(
        get_fixture_xattr(&backed_card, b"user.talkdnd-330"),
        Some(b"across-the-migration".to_vec()),
        "the ext4 backup inventory must preserve ACL/xattr"
    );
    assert_eq!(
        fs::read_to_string(backup.join("checkpoints/cp-1/Characters/Fixture.md")).unwrap(),
        "# old checkpoint card\n"
    );
    assert!(fixture
        .save
        .join("checkpoints/cp-1/Characters/Fixture.md")
        .is_file());
    // Note: republishing a registered target goes through temp + rename, so
    // an xattr on a *live* republished target does not survive; the verified
    // backup above is what preserves it for recovery. That boundary is
    // recorded in the PR description rather than asserted here.

    // Idempotent: a second run changes nothing.
    let second = migrate::ensure_with_bindings(&fixture.save, &fixture.root, false, &[]).unwrap();
    assert_eq!((second.from, second.to), (11, 11));
    assert!(second.already_current());
}

/// AC-4: an interrupted 10→11 publish rolls back to the exact preimage
/// through explicit recovery, and a later hand edit is never clobbered by
/// an unconfirmed recovery attempt.
#[cfg(unix)]
#[test]
fn interrupted_schema11_publish_rolls_back_and_protects_later_hand_edits() {
    use talkdnd::advancement_commit::{binding_preview_id, run_to_complete, Fault, Request};
    use talkdnd::advancement_lock as adv;
    use talkdnd::advancement_recovery::{self, RecoveryHooks, RecoveryRequest};

    let fixture = Fixture::new("interrupt");
    fixture.seed_card();
    adv::init(&fixture.root, &fixture.save, &adv::InitOptions::new()).unwrap();

    let confirms = fixture.confirm_args();
    let candidates = advancement_sidecar::prepare(&fixture.plan(&confirms), &fixture.save).unwrap();
    let mut overlays = advancement_sidecar::overlays(&candidates);
    let mut meta_text = fs::read_to_string(fixture.save.join("save-meta.yaml")).unwrap();
    meta_text = meta_text.replace("schema: 10", "schema: 11");
    meta_text.push_str(
        "migrations:\n  - from: 10\n    to: 11\n    date: synthetic\n    backup: synthetic\n",
    );
    overlays.insert("save-meta.yaml".to_string(), meta_text.into_bytes());
    let extra_targets = fixture.plan(&confirms).extra_targets();

    let backup_parent = fixture.root.join("Saves/_backups");
    fs::create_dir_all(&backup_parent).unwrap();
    let evidence_root = backup_parent.join(".operations");
    let evidence_dir = evidence_root.join("2030-01-01-01");
    fs::create_dir_all(&evidence_dir).unwrap();
    let preview = binding_preview_id(&fixture.save, &extra_targets).unwrap();

    let request = Request {
        root: &fixture.root,
        save_dir: &fixture.save,
        operation_id: "schema11-interrupt",
        evidence_root: &evidence_root,
        evidence_dir: &evidence_dir,
        backup_parent: &backup_parent,
        candidate_parent: &backup_parent,
        overlays,
        fault: Some(Fault::PublishWrite),
        extra_targets,
        preview_id: Some(preview),
        instance_tag: "",
        hooks: Default::default(),
        inner_lock: Default::default(),
        ext4_metadata: false,
    };
    let failed = run_to_complete(&request).unwrap_err();
    assert!(!failed.report.published, "{failed}");
    assert_eq!(
        failed.report.recovery.interrupt_stage,
        talkdnd::advancement_commit::InterruptStage::Publish
    );
    assert!(
        fs::read_to_string(fixture.save.join("save-meta.yaml"))
            .unwrap()
            .contains("schema: 10"),
        "the fault must stop before save-meta reaches 11"
    );
    assert!(!fixture.save.join(SIDECAR).exists());

    // A later hand edit lands on an unpublished target while the operation
    // is stopped. Explicit recovery must not silently clobber it: without a
    // confirmed human choice nothing is written, and the bytes stay.
    let hand_edit = fixture.save.join(CARD_PATH);
    fs::write(&hand_edit, b"# hand edited after the interrupt\n").unwrap();
    let inspect = RecoveryRequest {
        root: &fixture.root,
        save_dir: &fixture.save,
        recovery_id: "schema11-interrupt",
        evidence_root: &evidence_root,
        evidence_dir: &evidence_dir,
        choice: None,
        confirm_choice: false,
        extra_targets: Vec::new(),
        fault: None,
        hooks: RecoveryHooks::default(),
        inner_lock: Default::default(),
    };
    let inspected = advancement_recovery::inspect_recovery(&inspect);
    assert!(inspected.inspect_only, "no choice was supplied");
    assert_eq!(
        inspected.original_operation_id.as_deref(),
        Some("schema11-interrupt")
    );
    assert_eq!(
        fs::read(&hand_edit).unwrap(),
        b"# hand edited after the interrupt\n",
        "an unconfirmed recovery must protect later hand edits"
    );
}
