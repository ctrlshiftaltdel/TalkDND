//! Schema 11 migration (#330): read-only identity planning, DM confirmation
//! matching, sidecar missing-vs-empty distinction, the ext4 end-to-end run
//! and rollback through explicit recovery. Synthetic fixtures only; no real
//! saves, books or secrets.
//!
//! The end-to-end and recovery tests observe the live filesystem and only
//! run their write assertions on a writable ext4 mount; on any other
//! filesystem they assert the fail-closed refusal instead, so the suite
//! stays honest on tmpfs/macOS/Windows CI. The ext4 publish path is expected
//! to preserve the verified metadata of the file it actually replaces, not
//! merely to keep it in the backup, so those tests compare the replaced
//! sheet before and after.

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
const SHEET: &[u8] = b"\xef\xbb\xbf# DM note\r\nversion: 1\r\nid: fixture\r\nname: Fixture\r\nfile: Characters/Fixture.md\r\nruleset: \"2024\"\r\ncustom: {old: [4, 8], present: true}";

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
                        "name: fixture\nruleset: 5.5e-srd\nparty: [fixture]\nstate_dir: .\n",
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
            .flat_map(|pending| pending.confirm_args())
            .map(|arg| {
                let spec = arg
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
    // no silent overwrite of what may be an interrupted run's evidence. The
    // record reader refuses it (an unparsable record), which is the same
    // fail-closed signal; the pending list must be empty either way.
    fs::write(fixture.save.join(SIDECAR), b"").unwrap();
    let plan = fixture.plan(&[]);
    assert!(!plan.ready_for_schema_11());
    assert!(
        plan.pending.is_empty(),
        "an existing (empty) sidecar must not read as pending: {plan:?}"
    );
    assert!(
        plan.refused.iter().any(|reason| reason.contains(SIDECAR)),
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

/// The authoritative gate is #344's own admission inspection, not a
/// re-implementation: the e2e takes its success branch exactly when the
/// migration's own pre-write check would admit the environment.
#[cfg(target_os = "linux")]
fn writable_ext4(save: &Path, root: &Path) -> bool {
    use talkdnd::advancement_admission::{Admission, ApprovedRoots};
    let saves_root = root.join("Saves");
    let backups_root = saves_root.join("advancement-backups-probe");
    let inspected = if backups_root.is_dir() {
        &backups_root
    } else {
        &saves_root
    };
    let roots = ApprovedRoots {
        saves: &saves_root,
        backups: &saves_root,
        synchronized: &[],
    };
    matches!(
        talkdnd::advancement_admission::inspect(save, inspected, &roots),
        Admission::Admitted
    )
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

#[cfg(target_os = "linux")]
fn remove_fixture_xattr(path: &Path, name: &[u8]) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(name).unwrap();
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let rc = unsafe { libc::removexattr(c_path.as_ptr(), name.as_ptr()) };
    assert_eq!(rc, 0, "removing xattr on {}", path.display());
}

#[cfg(target_os = "linux")]
fn set_fixture_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// The live mode, ownership and recoverable modification time of a fixture
/// path, in the order the migration must preserve them.
#[cfg(target_os = "linux")]
fn fixture_metadata(path: &Path) -> (u32, u32, u32, std::time::SystemTime) {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path).unwrap();
    (
        meta.mode() & 0o7777,
        meta.uid(),
        meta.gid(),
        meta.modified().unwrap(),
    )
}

/// A nontrivial Linux POSIX ACL (version 2 header, owner, named user 65534,
/// group owner, mask, other) that the kernel cannot collapse to the minimal
/// owner/group/other form. The owner, group/mask and other permissions come
/// from `mode`, so applying the ACL yields exactly that `st_mode` (the kernel
/// derives the group permission bits from the ACL mask). The named user entry
/// keeps the ACL genuinely nontrivial even when the mask is restrictive.
#[cfg(target_os = "linux")]
fn fixture_acl(mode: u32) -> Vec<u8> {
    let owner = ((mode >> 6) & 0o7) as u16;
    let group = ((mode >> 3) & 0o7) as u16;
    let other = (mode & 0o7) as u16;
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, perm, id) in [
        (0x01u16, owner, u32::MAX), // owner
        (0x02, 4u16, 65534),        // named user, read-only
        (0x04, group, u32::MAX),    // group owner
        (0x10, group, u32::MAX),    // mask
        (0x20, other, u32::MAX),    // other
    ] {
        acl.extend_from_slice(&tag.to_le_bytes());
        acl.extend_from_slice(&perm.to_le_bytes());
        acl.extend_from_slice(&id.to_le_bytes());
    }
    acl
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

    if !writable_ext4(&fixture.save, &fixture.root) {
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

    // Real POSIX ACLs alongside the user xattrs, seeded only now that the
    // admitted branch is selected and before the migration writes. The sheet
    // is the file the migration actually replaces: its live mode, owner/group,
    // user xattr, POSIX ACL and recoverable modification time must survive
    // the staged temp + rename, not merely live on in the backup.
    let card_acl = fixture_acl(0o644);
    // Keep the pre-existing card ACL/xattr coverage as well.
    set_fixture_xattr(
        &fixture.save.join(CARD_PATH),
        b"system.posix_acl_access",
        &card_acl,
    );
    let sheet_path = fixture.save.join(SHEET_PATH);
    let sheet_acl = fixture_acl(0o600);
    set_fixture_mode(&sheet_path, 0o600);
    set_fixture_xattr(&sheet_path, b"user.talkdnd-330-sheet", b"replaced-sheet");
    set_fixture_xattr(&sheet_path, b"system.posix_acl_access", &sheet_acl);
    let sheet_before = fixture_metadata(&sheet_path);
    assert_eq!(
        sheet_before.0 & 0o777,
        0o600,
        "seeding the restrictive ACL must leave the sheet at 0600: {:o}",
        sheet_before.0
    );
    assert!(
        sheet_before.0 & 0o077 != 0o077,
        "fixture sheet must start restrictive: {:o}",
        sheet_before.0
    );

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
    let sidecar_path = fixture.save.join(SIDECAR);
    let sidecar_text = fs::read_to_string(&sidecar_path).unwrap();
    assert!(
        sidecar_text.contains("version: 1")
            && sidecar_text.contains("history: unknown")
            && sidecar_text.contains(&token),
        "{sidecar_text}"
    );
    // A new sidecar had no verified preimage: it keeps the owner-only
    // staging permissions and must not inherit an invented ACL/xattr.
    let sidecar_mode = fixture_metadata(&sidecar_path).0;
    assert_eq!(
        sidecar_mode & 0o077,
        0,
        "a new sidecar must keep restrictive staging access: {sidecar_mode:o}"
    );
    assert_eq!(
        sidecar_mode & 0o600,
        0o600,
        "a new sidecar must not be created inaccessible to its owner: {sidecar_mode:o}"
    );
    assert!(
        get_fixture_xattr(&sidecar_path, b"system.posix_acl_access").is_none(),
        "no ACL may be invented for a path the preimage did not contain"
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
        get_fixture_xattr(&backed_card, b"system.posix_acl_access"),
        Some(card_acl.clone()),
        "the ext4 backup inventory must preserve the card POSIX ACL byte for byte"
    );
    // The replaced sheet's user xattr and POSIX ACL travel through the same
    // inventory path; assert the exact parsed bytes, independently of the
    // card above.
    let backed_sheet = backup.join(SHEET_PATH);
    assert_eq!(
        get_fixture_xattr(&backed_sheet, b"user.talkdnd-330-sheet"),
        Some(b"replaced-sheet".to_vec()),
        "the ext4 backup inventory must preserve the sheet xattr"
    );
    assert_eq!(
        get_fixture_xattr(&backed_sheet, b"system.posix_acl_access"),
        Some(sheet_acl.clone()),
        "the ext4 backup inventory must preserve the POSIX ACL byte for byte"
    );
    assert_eq!(
        fs::read_to_string(backup.join("checkpoints/cp-1/Characters/Fixture.md")).unwrap(),
        "# old checkpoint card\n"
    );
    assert!(fixture
        .save
        .join("checkpoints/cp-1/Characters/Fixture.md")
        .is_file());

    // The live target the D publish actually replaced keeps the verified
    // metadata: mode, owner/group, user xattr, POSIX ACL and the recoverable
    // modification time (ctime is audit-only and is not asserted).
    let sheet_after = fixture_metadata(&sheet_path);
    assert_eq!(
        (sheet_after.0, sheet_after.1, sheet_after.2),
        (sheet_before.0, sheet_before.1, sheet_before.2),
        "live replaced sheet must keep mode and owner/group"
    );
    assert_eq!(
        sheet_after.3, sheet_before.3,
        "live replaced sheet must keep the recoverable modification time"
    );
    assert_eq!(
        get_fixture_xattr(&sheet_path, b"user.talkdnd-330-sheet"),
        Some(b"replaced-sheet".to_vec()),
        "live replaced sheet must keep its user xattr"
    );
    assert_eq!(
        get_fixture_xattr(&sheet_path, b"system.posix_acl_access"),
        Some(sheet_acl.clone()),
        "live replaced sheet must keep its POSIX ACL byte for byte"
    );

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
    use talkdnd::advancement_commit::{
        binding_preview_id, digest_bytes, run_to_complete, Fault, Request,
    };
    use talkdnd::advancement_lock as adv;
    use talkdnd::advancement_recovery::{self, RecoveryChoice, RecoveryHooks, RecoveryRequest};

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
        // Deterministic stop after every registered replace target (including
        // the sidecar and save-meta) has been published, and after the logs.
        fault: Some(Fault::IntegrityUpdate),
        extra_targets,
        preview_id: Some(preview),
        instance_tag: "",
        hooks: Default::default(),
        inner_lock: Default::default(),
        ext4_metadata: false,
    };
    fn confirmed<'a>(
        choice: RecoveryChoice,
        root: &'a Path,
        save: &'a Path,
        recovery_id: &'a str,
        evidence_root: &'a Path,
        evidence_dir: &'a Path,
    ) -> RecoveryRequest<'a> {
        RecoveryRequest {
            root,
            save_dir: save,
            recovery_id,
            evidence_root,
            evidence_dir,
            choice: Some(choice),
            confirm_choice: true,
            extra_targets: Vec::new(),
            fault: None,
            hooks: RecoveryHooks::default(),
            inner_lock: Default::default(),
        }
    }

    let failed = run_to_complete(&request).unwrap_err();
    // The stop lands after every registered replace target (sidecar and
    // save-meta included) was published, so publication did happen: the
    // report must say so and name the sidecar, not claim nothing was written.
    assert!(failed.report.published, "{failed}");
    assert!(
        failed
            .report
            .published_paths
            .iter()
            .any(|path| path == SIDECAR),
        "{failed:?}"
    );
    assert_eq!(
        failed.report.recovery.interrupt_stage,
        talkdnd::advancement_commit::InterruptStage::Logs,
        "{failed}"
    );

    // The sidecar really existed and was published by the operation before
    // the stop; the state tables record its absent preimage (digest of empty
    // bytes) and the nonempty target.
    let published_sidecar = fs::read(fixture.save.join(SIDECAR)).unwrap();
    assert!(!published_sidecar.is_empty());
    assert_eq!(published_sidecar, candidates[0].sidecar_bytes);
    let state = adv::open(&fixture.root, &fixture.save, &adv::OpenOptions::new())
        .unwrap()
        .state()
        .clone();
    let published = state
        .published
        .iter()
        .find(|entry| entry.path == SIDECAR)
        .expect("sidecar was published");
    assert!(published.done, "{published:?}");
    assert_eq!(published.before, digest_bytes(b""));
    assert_eq!(published.after, digest_bytes(&published_sidecar));
    assert!(state.pending.iter().all(|pending| pending.path != SIDECAR));

    // A later hand edit lands on a target while the operation is stopped.
    // Explicit recovery must not silently clobber it: without a confirmed
    // human choice nothing is written, and the bytes stay.
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
    // Restore the card: the refusal above already proved protection, and the
    // rollback below exercises ordinary preimage restoration.
    fs::write(&hand_edit, CARD).unwrap();

    // An unrelated EXISTING EMPTY file is not the missing preimage: rollback
    // refuses and leaves it exactly where it is.
    fs::write(fixture.save.join(SIDECAR), b"").unwrap();
    let empty = advancement_recovery::recover(&confirmed(
        RecoveryChoice::RollbackToPreimage,
        &fixture.root,
        &fixture.save,
        "schema11-interrupt",
        &evidence_root,
        &evidence_dir,
    ))
    .unwrap_err();
    assert!(
        empty
            .report
            .error
            .as_deref()
            .is_some_and(|err| err.contains(SIDECAR)),
        "{empty}"
    );
    assert_eq!(fs::read(fixture.save.join(SIDECAR)).unwrap(), b"");

    // A later hand-edited sidecar is likewise not removed.
    fs::write(fixture.save.join(SIDECAR), b"# later sidecar hand edit\n").unwrap();
    let edited = advancement_recovery::recover(&confirmed(
        RecoveryChoice::RollbackToPreimage,
        &fixture.root,
        &fixture.save,
        "schema11-interrupt",
        &evidence_root,
        &evidence_dir,
    ))
    .unwrap_err();
    assert!(
        edited
            .report
            .error
            .as_deref()
            .is_some_and(|err| err.contains(SIDECAR)),
        "{edited}"
    );
    assert_eq!(
        fs::read(fixture.save.join(SIDECAR)).unwrap(),
        b"# later sidecar hand edit\n"
    );

    // Restore the recorded target so this scenario reaches the after-intent hook.
    fs::write(fixture.save.join(SIDECAR), &published_sidecar).unwrap();
    // The intent is persisted before the after-intent hook, so a hook that
    // rewrites the sidecar is caught by the final type/byte re-check: the
    // replaced entry is left untouched and the conflict is reported.
    let hook_save = fixture.save.clone();
    let rewrite_sidecar = move |rel: &str, _save: &Path| -> std::result::Result<(), String> {
        if rel == SIDECAR {
            fs::write(hook_save.join(SIDECAR), b"# hook rewrote the sidecar\n")
                .map_err(|err| err.to_string())?;
        }
        Ok(())
    };
    let mut hooked = confirmed(
        RecoveryChoice::RollbackToPreimage,
        &fixture.root,
        &fixture.save,
        "schema11-interrupt",
        &evidence_root,
        &evidence_dir,
    );
    hooked.hooks.after_intent = Some(&rewrite_sidecar);
    let raced = advancement_recovery::recover(&hooked).unwrap_err();
    assert!(
        raced
            .report
            .error
            .as_deref()
            .is_some_and(|err| err.contains(SIDECAR)),
        "{raced}"
    );
    assert_eq!(
        fs::read(fixture.save.join(SIDECAR)).unwrap(),
        b"# hook rewrote the sidecar\n",
        "the post-intent re-check must leave the hook's replacement untouched"
    );
    assert!(
        !raced
            .report
            .written_paths
            .iter()
            .any(|path| path == SIDECAR),
        "{raced:?}"
    );

    // Only when live bytes still equal the operation's recorded target does
    // the confirmed rollback restore true path absence.
    fs::write(fixture.save.join(SIDECAR), &published_sidecar).unwrap();
    let report = advancement_recovery::recover(&confirmed(
        RecoveryChoice::RollbackToPreimage,
        &fixture.root,
        &fixture.save,
        "schema11-interrupt",
        &evidence_root,
        &evidence_dir,
    ))
    .unwrap();
    // A successful rollback returns its positive note through the report's
    // legacy `error` field (see advancement_recovery::finish_terminal), so
    // `report.error` is not an error signal here. Assert the protocol outcome
    // the rollback must reach instead.
    assert_eq!(report.protocol_state, adv::State::Failed, "{report:?}");
    assert_eq!(report.stage, adv::Stage::StoppedBeforeWrite, "{report:?}");
    assert!(!report.needs_human_merge, "{report:?}");
    assert!(
        report.written_paths.iter().any(|path| path == SIDECAR),
        "{report:?}"
    );
    assert!(
        !fixture.save.join(SIDECAR).exists(),
        "rollback must restore true path absence, not write empty bytes"
    );
    assert_eq!(fs::read(&hand_edit).unwrap(), CARD);
    assert_eq!(fs::read(fixture.save.join(SHEET_PATH)).unwrap(), SHEET);
    let meta = fs::read_to_string(fixture.save.join("save-meta.yaml")).unwrap();
    assert!(meta.contains("schema: 10"), "{meta}");
    assert!(!meta.contains("schema: 11"), "{meta}");
}

/// Shared D-request inputs for the ext4 publish/recovery tests below: the
/// reviewed confirmations, candidate overlays, the schema-11 metadata
/// overlay and the registered extra targets.
#[cfg(target_os = "linux")]
fn ext4_request_inputs(
    fixture: &Fixture,
) -> (
    String,
    BTreeMap<String, Vec<u8>>,
    Vec<String>,
    Vec<talkdnd::advancement_commit::EnrollmentCandidate>,
) {
    let confirms = fixture.confirm_args();
    let plan = fixture.plan(&confirms);
    let candidates = advancement_sidecar::prepare(&plan, &fixture.save).unwrap();
    let mut overlays = advancement_sidecar::overlays(&candidates);
    let mut meta_text = fs::read_to_string(fixture.save.join("save-meta.yaml")).unwrap();
    meta_text = meta_text.replace("schema: 10", "schema: 11");
    meta_text.push_str(
        "migrations:\n  - from: 10\n    to: 11\n    date: synthetic\n    backup: synthetic\n",
    );
    overlays.insert("save-meta.yaml".to_string(), meta_text.into_bytes());
    let extra_targets = plan.extra_targets();
    let preview =
        talkdnd::advancement_commit::binding_preview_id(&fixture.save, &extra_targets).unwrap();
    (preview, overlays, extra_targets, candidates)
}

/// A deterministic publish failure must stop before the rename and leave
/// both the live bytes and the restrictive metadata of the target it was
/// about to replace exactly as they were, with no staged temp left behind.
#[cfg(target_os = "linux")]
#[test]
fn ext4_publish_failure_before_rename_leaves_live_target_unmodified() {
    use talkdnd::advancement_commit::{run_to_complete, Fault, Request};
    use talkdnd::advancement_lock as adv;

    let fixture = Fixture::new("ext4-fail");
    fixture.seed_card();
    if !writable_ext4(&fixture.save, &fixture.root) {
        // Keep independent fail-closed coverage instead of silently skipping
        // on a runner that is not the admitted Linux ext4 environment.
        let confirms = fixture.confirm_args();
        let before = fixture.snapshot_tree();
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
    adv::init(&fixture.root, &fixture.save, &adv::InitOptions::new()).unwrap();
    let (preview, overlays, extra_targets, _candidates) = ext4_request_inputs(&fixture);

    let sheet_path = fixture.save.join(SHEET_PATH);
    set_fixture_mode(&sheet_path, 0o600);
    set_fixture_xattr(&sheet_path, b"user.talkdnd-330-fail", b"must-stay");
    let before_bytes = fs::read(&sheet_path).unwrap();
    let before = fixture_metadata(&sheet_path);

    let backup_parent = fixture.root.join("Saves/_backups");
    fs::create_dir_all(&backup_parent).unwrap();
    let evidence_root = backup_parent.join(".operations");
    let evidence_dir = evidence_root.join("2030-01-01-01");
    fs::create_dir_all(&evidence_dir).unwrap();

    // The first registered replace target is `sheets/fixture.yaml`; the
    // injected sync failure lands after the staged write and before the
    // metadata apply and rename.
    let request = Request {
        root: &fixture.root,
        save_dir: &fixture.save,
        operation_id: "ext4-publish-fail",
        evidence_root: &evidence_root,
        evidence_dir: &evidence_dir,
        backup_parent: &backup_parent,
        candidate_parent: &backup_parent,
        overlays,
        fault: Some(Fault::PublishSync),
        extra_targets,
        preview_id: Some(preview),
        instance_tag: "",
        hooks: Default::default(),
        inner_lock: Default::default(),
        ext4_metadata: true,
    };
    let failed = run_to_complete(&request).unwrap_err();
    assert!(!failed.report.published, "{failed}");
    assert!(failed.report.published_paths.is_empty(), "{failed:?}");
    assert_eq!(fs::read(&sheet_path).unwrap(), before_bytes);
    assert_eq!(
        fixture_metadata(&sheet_path),
        before,
        "a pre-rename failure must not alter live mode/ownership/timestamps"
    );
    assert_eq!(
        get_fixture_xattr(&sheet_path, b"user.talkdnd-330-fail"),
        Some(b"must-stay".to_vec())
    );
    assert!(
        !fixture.save.join(".fixture.yaml.adv-pub-tmp").exists(),
        "the failed staged temp file must be removed"
    );
}

/// Explicit rollback of an ext4-inventory-backed operation restores both the
/// verified preimage bytes and that preimage's metadata on the real replaced
/// sheet, even after the live target's metadata was changed after the
/// interrupted publish.
#[cfg(target_os = "linux")]
#[test]
fn explicit_rollback_restores_verified_ext4_metadata_for_a_replaced_sheet() {
    use talkdnd::advancement_commit::{run_to_complete, Fault, Request};
    use talkdnd::advancement_lock as adv;
    use talkdnd::advancement_recovery::{self, RecoveryChoice, RecoveryHooks, RecoveryRequest};

    let fixture = Fixture::new("ext4-recover");
    fixture.seed_card();
    if !writable_ext4(&fixture.save, &fixture.root) {
        // Explicit recovery is only claimed for an ext4-inventory-backed
        // operation on the admitted environment; elsewhere the migration
        // itself must refuse and write nothing (no false-positive skip).
        let confirms = fixture.confirm_args();
        let before = fixture.snapshot_tree();
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
    adv::init(&fixture.root, &fixture.save, &adv::InitOptions::new()).unwrap();
    let (preview, overlays, extra_targets, _candidates) = ext4_request_inputs(&fixture);

    // Metadata seeded before the D operation, so the verified inventory has
    // an original entry for the sheet the operation will replace.
    let sheet_path = fixture.save.join(SHEET_PATH);
    let sheet_acl = fixture_acl(0o600);
    set_fixture_mode(&sheet_path, 0o600);
    set_fixture_xattr(&sheet_path, b"user.talkdnd-330-recover", b"original");
    set_fixture_xattr(&sheet_path, b"system.posix_acl_access", &sheet_acl);
    let original = fixture_metadata(&sheet_path);
    assert_eq!(
        original.0 & 0o777,
        0o600,
        "seeding the restrictive ACL must leave the sheet at 0600: {:o}",
        original.0
    );

    let backup_parent = fixture.root.join("Saves/_backups");
    fs::create_dir_all(&backup_parent).unwrap();
    let evidence_root = backup_parent.join(".operations");
    let evidence_dir = evidence_root.join("2030-01-01-01");
    fs::create_dir_all(&evidence_dir).unwrap();

    let request = Request {
        root: &fixture.root,
        save_dir: &fixture.save,
        operation_id: "ext4-recover",
        evidence_root: &evidence_root,
        evidence_dir: &evidence_dir,
        backup_parent: &backup_parent,
        candidate_parent: &backup_parent,
        overlays,
        // Publish the registered targets (the sheet included) and stop at
        // integrity, leaving a recoverable in_doubt operation.
        fault: Some(Fault::IntegrityUpdate),
        extra_targets,
        preview_id: Some(preview),
        instance_tag: "",
        hooks: Default::default(),
        inner_lock: Default::default(),
        ext4_metadata: true,
    };
    let failed = run_to_complete(&request).unwrap_err();
    assert!(failed.report.published, "{failed}");
    assert!(
        failed
            .report
            .published_paths
            .iter()
            .any(|p| p == SHEET_PATH),
        "{failed:?}"
    );
    // The successful replace already preserved the metadata.
    assert_eq!(fixture_metadata(&sheet_path), original);

    // A later change to the live target's metadata is reversed by the
    // verified rollback (the bytes still equal the operation's target).
    set_fixture_mode(&sheet_path, 0o644);
    remove_fixture_xattr(&sheet_path, b"user.talkdnd-330-recover");
    remove_fixture_xattr(&sheet_path, b"system.posix_acl_access");
    let mutated = fixture_metadata(&sheet_path);
    assert_eq!(
        mutated.0 & 0o777,
        0o644,
        "the mutated sheet mode must be 0644: {:o}",
        mutated.0
    );
    assert!(
        get_fixture_xattr(&sheet_path, b"user.talkdnd-330-recover").is_none(),
        "the mutation must remove the sheet xattr"
    );
    assert!(
        get_fixture_xattr(&sheet_path, b"system.posix_acl_access").is_none(),
        "the mutation must remove the sheet POSIX ACL"
    );
    assert_ne!(mutated, original, "the mutation must be observable");

    let report = advancement_recovery::recover(&RecoveryRequest {
        root: &fixture.root,
        save_dir: &fixture.save,
        recovery_id: "ext4-recover",
        evidence_root: &evidence_root,
        evidence_dir: &evidence_dir,
        choice: Some(RecoveryChoice::RollbackToPreimage),
        confirm_choice: true,
        extra_targets: Vec::new(),
        fault: None,
        hooks: RecoveryHooks::default(),
        inner_lock: Default::default(),
    })
    .unwrap();
    assert_eq!(report.protocol_state, adv::State::Failed, "{report:?}");
    assert!(!report.needs_human_merge, "{report:?}");
    assert!(
        report.written_paths.iter().any(|p| p == SHEET_PATH),
        "{report:?}"
    );

    // Bytes and metadata of the real replaced sheet are the verified
    // preimage again; ctime is audit-only and is not compared.
    assert_eq!(fs::read(&sheet_path).unwrap(), SHEET);
    assert_eq!(
        fixture_metadata(&sheet_path),
        original,
        "rollback must restore mode, owner/group and mtime"
    );
    assert_eq!(
        get_fixture_xattr(&sheet_path, b"user.talkdnd-330-recover"),
        Some(b"original".to_vec())
    );
    assert_eq!(
        get_fixture_xattr(&sheet_path, b"system.posix_acl_access"),
        Some(sheet_acl),
        "rollback must restore the POSIX ACL byte for byte"
    );
}
