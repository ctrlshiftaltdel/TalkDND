//! Checkpoint snapshot / list / show / restore (P01, TASK-0059;
//! LetsPlay/docs/campaign-continuity.md §4.1 / §5.1).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use talkdnd::checkpoint::{self, Cmd};
use talkdnd::migrate;

/// A minimal but complete save: covered files, out-of-scope files, one log.
fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("talkdnd-cp-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let save = root.join("Saves/2030-01-02-01");
    for d in ["Sessions", "combat", "Characters", "NPCs", "sheets"] {
        fs::create_dir_all(save.join(d)).unwrap();
    }
    fs::write(root.join("AGENTS.md"), "x\n").unwrap();
    fs::write(
        save.join("Sessions/2030-01-02-session-00-setup.md"),
        "# 会话\n\n- [10:00] 开局\n",
    )
    .unwrap();
    fs::write(
        save.join("combat/campaign.yaml"),
        "name: 测试战役\nruleset: \"2024\"\nparty: [tess]\n",
    )
    .unwrap();
    fs::write(
        save.join("progress.yaml"),
        "xp: {tess: 300}\npurses:\n  tess: {cp: 500}\n  party: {cp: 0, items: [rope]}\n",
    )
    .unwrap();
    fs::write(save.join("reveals.yaml"), "version: 1\nfacts: []\n").unwrap();
    fs::write(save.join("identified.yaml"), "version: 1\nentries: []\n").unwrap();
    fs::write(save.join("original.yaml"), "version: 1\nentries: []\n").unwrap();
    fs::write(save.join("canon.md"), "# 定论\n").unwrap();
    fs::write(
        save.join("Characters/tess.md"),
        "# Tess\n\n```combat yaml\nid: tess\nname: Tess\nlevel: 3\nhp: 12\nmax_hp: 20\n```\n",
    )
    .unwrap();
    fs::write(save.join("sheets/tess.yaml"), "id: tess\nname: Tess\n").unwrap();
    fs::write(
        save.join("combat/party-state.yaml"),
        "version: 1\nmembers:\n  tess: {name: Tess, hp: 12, max_hp: 20, temp_hp: 0, resources: {}, conditions: []}\n",
    )
    .unwrap();
    // Out of the copy set: never copied, never touched by restore.
    fs::write(save.join("dm-secrets.md"), "secret\n").unwrap();
    fs::write(save.join("rolls.log"), "1d20=17\n").unwrap();
    fs::write(save.join("digest.md"), "old digest\n").unwrap();
    fs::write(save.join("chargen.json"), "{}\n").unwrap();
    fs::write(save.join("secret-terms.yaml"), "terms: []\n").unwrap();
    fs::write(save.join("integrity.yaml"), "version: 1\nfiles: {}\n").unwrap();
    migrate::init(&save).unwrap();
    (root, save)
}

fn root_of(save: &Path) -> PathBuf {
    save.parent().unwrap().parent().unwrap().to_path_buf()
}

fn snapshot(save: &Path, reason: Option<&str>) -> String {
    checkpoint::run(
        &root_of(save),
        save,
        Cmd::Snapshot {
            reason: reason.map(str::to_string),
            json: false,
        },
    )
    .unwrap();
    checkpoint::latest(save).unwrap().id
}

fn restore(save: &Path, id: &str, yes: bool) -> anyhow::Result<()> {
    checkpoint::run(
        &root_of(save),
        save,
        Cmd::Restore {
            id: id.to_string(),
            yes,
            json: false,
        },
    )
}

fn show(save: &Path, id: &str) -> anyhow::Result<()> {
    checkpoint::run(
        &root_of(save),
        save,
        Cmd::Show {
            id: id.to_string(),
            json: false,
        },
    )
}

/// Everything under the save except checkpoints/, the lock and the cache.
/// Files are `(path, content)`; directories are `(path/, empty)` — so a
/// full-tree comparison also catches directories that were created or
/// removed, not just file contents.
fn fingerprint(save: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for p in entries {
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "checkpoints" || rel == ".talkdnd.lock" || rel == ".cache" {
                continue;
            }
            if p.is_dir() {
                out.push((format!("{rel}/"), Vec::new()));
                walk(&p, base, out);
            } else {
                out.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    let mut v = Vec::new();
    walk(save, save, &mut v);
    v.sort();
    v
}

fn bin(root: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .arg("--root")
        .arg(root)
        .args(args)
        .env("TALKDND_NO_PDF2MD", "1")
        .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
        .output()
        .unwrap();
    let s =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), s)
}

#[test]
fn snapshot_writes_manifest_and_covered_copies() {
    let (root, save) = fixture("manifest");
    let id = snapshot(&save, Some("Before the boss"));
    assert!(id.starts_with("20"), "{id}");
    assert!(id.ends_with("-before-the-boss"), "{id}");
    let dir = checkpoint::checkpoint_dir(&save, &id);
    let m = checkpoint::manifest(&save, &id).unwrap();
    assert_eq!(m.version, 1);
    assert_eq!(m.id, id);
    assert_eq!(m.reason.as_deref(), Some("Before the boss"));
    assert_eq!(m.schema, migrate::CURRENT_SCHEMA);
    assert_eq!(m.log_file, "2030-01-02-session-00-setup.md");
    assert_eq!(m.log_lines, 3);
    // Every covered file is copied and hashed.
    for rel in [
        "progress.yaml",
        "reveals.yaml",
        "save-meta.yaml",
        "original.yaml",
        "identified.yaml",
        "canon.md",
        "Characters/tess.md",
        "sheets/tess.yaml",
        "combat/party-state.yaml",
        "combat/campaign.yaml",
    ] {
        assert!(dir.join(rel).is_file(), "{rel} not copied");
        assert!(m.files.contains_key(rel), "{rel} not in the manifest");
        assert!(m.files[rel].starts_with("sha256:"), "{}", m.files[rel]);
    }
    assert!(!m.files.contains_key("house-rules.md"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn snapshot_excludes_sessions_integrity_and_dm_files() {
    let (root, save) = fixture("excludes");
    let id = snapshot(&save, None);
    let dir = checkpoint::checkpoint_dir(&save, &id);
    let m = checkpoint::manifest(&save, &id).unwrap();
    for rel in [
        "Sessions",
        "checkpoints",
        "integrity.yaml",
        "dm-secrets.md",
        "dm-plan.md",
        "secret-terms.yaml",
        "rolls.log",
        "digest.md",
        "chargen.json",
        "handouts",
        "npc-drafts",
        ".talkdnd.lock",
    ] {
        assert!(!dir.join(rel).exists(), "{rel} must not be copied");
        assert!(
            !m.files
                .keys()
                .any(|k| k == rel || k.starts_with(&format!("{rel}/"))),
            "{rel} must not be in the manifest"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn snapshot_works_on_an_old_save_without_checkpoints() {
    let (root, save) = fixture("old-save");
    assert!(checkpoint::latest(&save).is_none());
    let id = snapshot(&save, None);
    let all = checkpoint::entries(&save);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, id);
    assert!(all[0].usable, "{:?}", all[0].problem);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn snapshot_requires_a_session_log() {
    let (root, save) = fixture("no-log");
    fs::remove_dir_all(save.join("Sessions")).unwrap();
    let err = checkpoint::run(
        &root_of(&save),
        &save,
        Cmd::Snapshot {
            reason: None,
            json: false,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("会话日志"), "{err}");
    assert!(checkpoint::latest(&save).is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn retention_keeps_the_newest_ten_and_reports_deletions() {
    let (root, save) = fixture("retention");
    // Twelve checkpoints with distinct timestamps (lexicographic = time order).
    let mut ids = Vec::new();
    for i in 0..12 {
        let id = format!("20260101-{:06}", i * 3600);
        let dir = checkpoint::checkpoint_dir(&save, &id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("manifest.yaml"), "version: 1\n").unwrap();
        ids.push(id);
    }
    let fresh = snapshot(&save, None);
    let left = checkpoint::entries(&save);
    assert_eq!(left.len(), 10, "retention must keep the newest 10");
    let left_ids: Vec<&str> = left.iter().map(|e| e.id.as_str()).collect();
    assert!(
        !left_ids.contains(&ids[0].as_str()),
        "oldest must be deleted"
    );
    assert!(
        !left_ids.contains(&ids[1].as_str()),
        "2nd oldest must be deleted"
    );
    assert!(
        !left_ids.contains(&ids[2].as_str()),
        "3rd oldest must be deleted"
    );
    assert!(left_ids.contains(&ids[3].as_str()));
    assert!(left_ids.contains(&fresh.as_str()));
    // The deletion is reported by the snapshot that triggered it.
    let (good, out) = bin(&root, &["checkpoint", "snapshot"]);
    assert!(good, "{out}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn list_marks_a_corrupt_manifest_unavailable_without_failing_the_rest() {
    let (root, save) = fixture("corrupt");
    let broken = snapshot(&save, Some("broken one"));
    let good = snapshot(&save, Some("good one"));
    let path = checkpoint::checkpoint_dir(&save, &broken).join("manifest.yaml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, &text[..text.len() / 2]).unwrap();
    let all = checkpoint::entries(&save);
    assert_eq!(all.len(), 2, "a broken checkpoint never hides the others");
    let bad = all.iter().find(|e| e.id == broken).unwrap();
    assert!(!bad.usable);
    assert!(
        bad.problem.as_deref().unwrap().contains("解析失败"),
        "{bad:?}"
    );
    let ok = all.iter().find(|e| e.id == good).unwrap();
    assert!(ok.usable, "{:?}", ok.problem);
    // show and restore refuse the broken one, not the good one.
    assert!(show(&save, &broken).is_err());
    assert!(restore(&save, &broken, true).is_err());
    restore(&save, &good, true).unwrap();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_tampered_copy_is_unavailable_and_restore_refuses() {
    let (root, save) = fixture("tamper");
    let tampered = snapshot(&save, None);
    let good = snapshot(&save, None);
    let copy = checkpoint::checkpoint_dir(&save, &tampered).join("progress.yaml");
    let mut text = fs::read_to_string(&copy).unwrap();
    text.push_str("\n# tampered\n");
    fs::write(&copy, text).unwrap();
    let all = checkpoint::entries(&save);
    let bad = all.iter().find(|e| e.id == tampered).unwrap();
    assert!(!bad.usable, "{:?}", bad.problem);
    assert!(
        bad.problem.as_deref().unwrap().contains("sha256"),
        "{:?}",
        bad.problem
    );
    let ok = all.iter().find(|e| e.id == good).unwrap();
    assert!(ok.usable, "{:?}", ok.problem);
    let before = fingerprint(&save);
    assert!(restore(&save, &tampered, true).is_err());
    assert_eq!(
        fingerprint(&save),
        before,
        "a refused restore changes nothing"
    );
    restore(&save, &good, true).unwrap();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_missing_checkpoint_is_an_error() {
    let (root, save) = fixture("missing");
    snapshot(&save, None);
    let err = show(&save, "20990101-000000").unwrap_err();
    assert!(err.to_string().contains("不存在"), "{err}");
    let err = restore(&save, "20990101-000000", true).unwrap_err();
    assert!(err.to_string().contains("不存在"), "{err}");
    let err = talkdnd::diff::diff_lines(&save, "20990101-000000", "current").unwrap_err();
    assert!(err.to_string().contains("不存在"), "{err}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn restore_requires_yes_and_changes_nothing() {
    let (root, save) = fixture("no-yes");
    let id = snapshot(&save, None);
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    let before = fingerprint(&save);
    let err = restore(&save, &id, false).unwrap_err();
    assert!(err.to_string().contains("--yes"), "{err}");
    assert_eq!(fingerprint(&save), before);
    assert!(
        !root.join("Saves/_backups").exists(),
        "a refused restore must not even back up"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn restore_replaces_covered_files_deletes_new_ones_and_backs_up() {
    let (root, save) = fixture("restore");
    let id = snapshot(&save, None);
    let original_progress = fs::read_to_string(save.join("progress.yaml")).unwrap();
    // Changes after the snapshot: an edit, a new covered file, a new out-of-scope file.
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    fs::write(save.join("Characters/evil.md"), "# evil\n").unwrap();
    fs::write(save.join("canon.md"), "# 新定论\n").unwrap();
    fs::write(save.join("dm-secrets.md"), "changed secret\n").unwrap();
    restore(&save, &id, true).unwrap();
    // Covered files are back to the snapshot state.
    assert_eq!(
        fs::read_to_string(save.join("progress.yaml")).unwrap(),
        original_progress
    );
    assert_eq!(
        fs::read_to_string(save.join("canon.md")).unwrap(),
        "# 定论\n"
    );
    // A covered file that appeared after the snapshot is gone.
    assert!(!save.join("Characters/evil.md").exists());
    // Out-of-scope files are untouched.
    assert_eq!(
        fs::read_to_string(save.join("dm-secrets.md")).unwrap(),
        "changed secret\n"
    );
    // The whole save was backed up before the restore.
    let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves/_backups"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    let backup = &backups[0];
    let name = backup.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("2030-01-02-01-"), "{name}");
    assert!(name.contains("-restore-"), "{name}");
    assert!(backup.join("save-meta.yaml").is_file());
    assert!(backup.join("progress.yaml").is_file());
    assert!(backup.join("combat").is_dir());
    assert!(backup.join("Characters/evil.md").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn restore_of_a_newer_schema_is_refused() {
    let (root, save) = fixture("newer");
    let dir = checkpoint::checkpoint_dir(&save, "20990101-000000-future");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("manifest.yaml"),
        "version: 1\nid: 20990101-000000-future\ncreated: \"2099-01-01 00:00\"\nschema: 99\nlog_file: x.md\nlog_lines: 0\nfiles: {}\n",
    )
    .unwrap();
    let before = fingerprint(&save);
    let err = restore(&save, "20990101-000000-future", true).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("20990101-000000-future") && msg.contains("99"),
        "{msg}"
    );
    assert!(msg.contains("更新 talkdnd"), "{msg}");
    assert_eq!(fingerprint(&save), before);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn restore_of_an_older_schema_runs_the_normal_migration() {
    let (root, save) = fixture("older");
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    fs::write(
        save.join("save-meta.yaml"),
        meta.replace("schema: 8", "schema: 6"),
    )
    .unwrap();
    let id = snapshot(&save, None);
    let m = checkpoint::manifest(&save, &id).unwrap();
    assert_eq!(m.schema, 6);
    // The save moves on to schema 8 after the snapshot.
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    fs::write(
        save.join("save-meta.yaml"),
        meta.replace("schema: 6", "schema: 8"),
    )
    .unwrap();
    restore(&save, &id, true).unwrap();
    // Restore first, then the normal migration: the save is current again.
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    assert!(meta.contains("schema: 8"), "{meta}");
    assert!(
        meta.contains("from: 6"),
        "migration must be recorded: {meta}"
    );
    let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves/_backups"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    assert_eq!(
        backups.len(),
        2,
        "restore backup + migration backup: {backups:?}"
    );
    let log = fs::read_to_string(save.join("Sessions/2030-01-02-session-00-setup.md")).unwrap();
    assert!(log.contains("恢复检查点"), "{log}");
    assert!(log.contains("schema 6→8"), "{log}");
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn an_injected_write_failure_leaves_the_save_unchanged_and_recoverable() {
    use std::os::unix::fs::PermissionsExt;
    let (root, save) = fixture("stage-fail");
    let id = snapshot(&save, None);
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    let before = fingerprint(&save);
    // A read-only covered directory makes the staged temp write fail.
    fs::set_permissions(save.join("Characters"), fs::Permissions::from_mode(0o500)).unwrap();
    let err = restore(&save, &id, true).unwrap_err();
    fs::set_permissions(save.join("Characters"), fs::Permissions::from_mode(0o755)).unwrap();
    let msg = err.to_string();
    assert!(msg.contains("未改动"), "{msg}");
    // No half-restored state: the old file is still readable and unchanged.
    assert_eq!(fingerprint(&save), before);
    assert_eq!(
        fs::read_to_string(save.join("progress.yaml")).unwrap(),
        "xp: {tess: 999}\n"
    );
    // A clearly documented backup exists as the recovery path.
    let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves/_backups"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    // After the failure is removed, the retry succeeds.
    restore(&save, &id, true).unwrap();
    assert_eq!(
        fs::read_to_string(save.join("progress.yaml")).unwrap(),
        "xp: {tess: 300}\npurses:\n  tess: {cp: 500}\n  party: {cp: 0, items: [rope]}\n"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn restore_rebaselines_integrity_and_logs_the_restore() {
    let (root, save) = fixture("rebaseline");
    let id = snapshot(&save, None);
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    fs::write(
        save.join("Characters/tess.md"),
        "# Tess\n\n```combat yaml\nid: tess\nname: Tess\nlevel: 3\nhp: 4\nmax_hp: 20\n```\n",
    )
    .unwrap();
    restore(&save, &id, true).unwrap();
    // talkdnd check (no arguments) reports no unexplained changes.
    let report = talkdnd::audit::audit(&save).unwrap();
    assert!(report.clean(), "{:?}", report.lines());
    // integrity.yaml and the audit baseline reflect the restored state.
    let m = combat_sim::integrity::load(&save).unwrap().unwrap();
    let now = combat_sim::integrity::snapshot(&save);
    assert_eq!(m.files, now, "the manifest must match the restored save");
    // The log has the restore line and keeps its history.
    let log = fs::read_to_string(save.join("Sessions/2030-01-02-session-00-setup.md")).unwrap();
    assert!(log.contains("# 会话"), "history must not be truncated");
    assert!(
        log.contains("- [10:00] 开局"),
        "history must not be truncated"
    );
    let line = log.lines().find(|l| l.contains("恢复检查点")).unwrap();
    assert!(line.contains(&id), "{line}");
    assert!(line.contains("Saves/_backups/"), "{line}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_held_lock_fails_the_second_call_and_a_retry_after_release_works() {
    let (root, save) = fixture("lock");
    let id = snapshot(&save, None);
    // A fresh lock file = another snapshot/restore is running right now.
    fs::write(save.join(".talkdnd.lock"), "12345\n").unwrap();
    let err = checkpoint::run(
        &root_of(&save),
        &save,
        Cmd::Snapshot {
            reason: None,
            json: false,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("锁"), "{err}");
    let before = fingerprint(&save);
    let err = restore(&save, &id, true).unwrap_err();
    assert!(err.to_string().contains("锁"), "{err}");
    assert_eq!(fingerprint(&save), before);
    // Released: both work again.
    fs::remove_file(save.join(".talkdnd.lock")).unwrap();
    snapshot(&save, None);
    restore(&save, &id, true).unwrap();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn concurrent_snapshots_exactly_one_wins() {
    let (root, save) = fixture("concurrent");
    // Enough files that the winner's snapshot outlasts the loser's lock attempt.
    for i in 0..600 {
        fs::write(
            save.join(format!("Characters/extra-{i:03}.md")),
            "# extra\n",
        )
        .unwrap();
    }
    let barrier = std::sync::Barrier::new(2);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| {
                barrier.wait();
                let r = checkpoint::run(
                    &root_of(&save),
                    &save,
                    Cmd::Snapshot {
                        reason: None,
                        json: false,
                    },
                );
                results.lock().unwrap().push(r);
            });
        }
    });
    let results = results.into_inner().unwrap();
    let ok = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(ok, 1, "exactly one snapshot may hold the lock: {results:?}");
    let err = results.iter().find(|r| r.is_err()).unwrap();
    assert!(
        err.as_ref().unwrap_err().to_string().contains("锁"),
        "{err:?}"
    );
    assert_eq!(checkpoint::entries(&save).len(), 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn cli_snapshot_list_show_restore_flow() {
    let (root, save) = fixture("cli");
    let (good, out) = bin(
        &root,
        &["checkpoint", "snapshot", "--reason", "before boss"],
    );
    assert!(good, "{out}");
    assert!(out.contains("已创建"), "{out}");
    assert!(out.contains("保留策略"), "{out}");
    let id = checkpoint::latest(&save).unwrap().id;

    let (good, out) = bin(&root, &["checkpoint", "snapshot", "--json"]);
    assert!(good, "{out}");
    let json: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(json["version"], 1);
    let json_id = json["id"].as_str().unwrap().to_string();
    assert!(json_id.starts_with("20"), "{json_id}");
    assert_ne!(json_id, id);
    assert!(checkpoint::checkpoint_dir(&save, &json_id).is_dir());

    let (good, out) = bin(&root, &["checkpoint", "list"]);
    assert!(good, "{out}");
    assert!(out.contains(&id) && out.contains("可用"), "{out}");
    let (good, out) = bin(&root, &["checkpoint", "list", "--json"]);
    assert!(good, "{out}");
    let json: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 2);

    let (good, out) = bin(&root, &["checkpoint", "show", &id]);
    assert!(good, "{out}");
    assert!(out.contains("创建") && out.contains("字节"), "{out}");
    let (good, out) = bin(&root, &["checkpoint", "show", "20990101-000000"]);
    assert!(!good && out.contains("不存在"), "{out}");

    // Without --yes the restore is refused and the save is untouched.
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    let before = fingerprint(&save);
    let (good, out) = bin(&root, &["checkpoint", "restore", &id]);
    assert!(!good && out.contains("--yes"), "{out}");
    assert_eq!(fingerprint(&save), before);

    let (good, out) = bin(&root, &["checkpoint", "restore", &id, "--yes"]);
    assert!(good, "{out}");
    assert!(out.contains("备份当前存档"), "{out}");
    assert_eq!(
        fs::read_to_string(save.join("progress.yaml")).unwrap(),
        "xp: {tess: 300}\npurses:\n  tess: {cp: 500}\n  party: {cp: 0, items: [rope]}\n"
    );

    // talkdnd check (no arguments) is clean after the restore.
    let (good, out) = bin(&root, &["check"]);
    assert!(good, "{out}");
    assert!(out.contains("integrity: OK"), "{out}");

    // The diff against the restored state shows no changes.
    let (good, out) = bin(&root, &["snapshot", "diff", &id, "current"]);
    assert!(good, "{out}");
    assert!(
        out.contains("snapshot diff") && out.contains("无变化"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn staging_failure_leaves_the_save_tree_unchanged() {
    let (root, save) = fixture("stage-fail");
    let id = snapshot(&save, None);
    // Portable staging failure: a file sits where the covered Characters/
    // directory must be created, so staging aborts before anything is
    // replaced (no permission tricks, works on every platform).
    fs::remove_dir_all(save.join("Characters")).unwrap();
    fs::write(save.join("Characters"), "not a directory\n").unwrap();
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    let before = fingerprint(&save);
    let err = restore(&save, &id, true).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("恢复已中止"), "{msg}");
    assert!(msg.contains("未改动"), "{msg}");
    assert!(msg.contains("Saves/_backups/"), "{msg}");
    assert_eq!(
        fingerprint(&save),
        before,
        "a staging failure must not touch the tree"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn partial_commit_failure_rolls_back_the_whole_tree() {
    let (root, save) = fixture("commit-fail");
    let id = snapshot(&save, None);
    // Diverge the save from the snapshot and add a covered file that only
    // exists after it: a successful restore would delete it, so the full-tree
    // comparison (this file included) proves the rollback was complete —
    // not just a few spot checks.
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    fs::write(save.join("canon.md"), "# 新定论\n").unwrap();
    fs::write(save.join("Characters/nick.md"), "# Nick\n").unwrap();
    let before = fingerprint(&save);
    // Injected commit failure at the 4th rename (0-based): three files are
    // already replaced when the commit breaks. The rollback itself must run
    // for real and restore every file.
    let err = checkpoint::restore_injecting(
        &root,
        &save,
        &id,
        checkpoint::RestoreFault {
            fail_rename_at: Some(3),
            fail_rollback: false,
        },
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("恢复失败"), "{msg}");
    assert!(msg.contains("已从"), "{msg}");
    assert!(msg.contains("回滚到恢复前状态"), "{msg}");
    assert!(!msg.contains("回滚失败"), "{msg}");
    assert!(msg.contains("Saves/_backups/"), "{msg}");
    assert_eq!(
        fingerprint(&save),
        before,
        "rollback must restore the whole tree, post-snapshot files included"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn failed_rollback_is_reported_not_claimed() {
    let (root, save) = fixture("rollback-fail");
    let id = snapshot(&save, None);
    // The commit fails partway and the rollback itself fails too (both
    // injected; a real rollback obstruction such as ENOSPC is not portable
    // to reproduce). The error must say the rollback failed, name the
    // backup and both errors, and never claim "已回滚".
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    let err = checkpoint::restore_injecting(
        &root,
        &save,
        &id,
        checkpoint::RestoreFault {
            fail_rename_at: Some(3),
            fail_rollback: true,
        },
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("恢复失败"), "{msg}");
    assert!(msg.contains("回滚失败"), "{msg}");
    assert!(msg.contains("可能不完整"), "{msg}");
    assert!(msg.contains("injected commit failure"), "{msg}");
    assert!(msg.contains("injected rollback failure"), "{msg}");
    assert!(msg.contains("Saves/_backups/"), "{msg}");
    assert!(!msg.contains("已从"), "{msg}");
    let _ = fs::remove_dir_all(&root);
}

/// The issue #148 sequence: a covered file (and its whole directory) existed
/// at snapshot time but is gone before the restore, so the pre-restore
/// backup does not contain it — while the commit re-creates it before
/// failing. A rollback that only copies the backup back would leave the
/// re-created file behind and still claim success; the mirror rollback must
/// remove it (and the directory staging re-created), leaving the tree
/// exactly as it was before the restore.
#[test]
fn rollback_after_partial_commit_removes_entries_the_backup_lacks() {
    let (root, save) = fixture("rollback-extras");
    let id = snapshot(&save, None);
    // Delete the whole covered directory: its file must be re-created by
    // the commit (and the directory by staging), yet neither is in the
    // pre-restore backup.
    fs::remove_dir_all(save.join("Characters")).unwrap();
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    let before = fingerprint(&save);
    assert!(
        before.iter().all(|(p, _)| !p.starts_with("Characters/")),
        "the fixture change must remove every Characters entry: {before:?}"
    );
    // Commit order is the manifest's sorted key order and `Characters/tess.md`
    // sorts first (uppercase C), so its rename re-creates the file before the
    // injected failure hits the second rename.
    let err = checkpoint::restore_injecting(
        &root,
        &save,
        &id,
        checkpoint::RestoreFault {
            fail_rename_at: Some(1),
            fail_rollback: false,
        },
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("恢复失败"), "{msg}");
    assert!(msg.contains("已从"), "{msg}");
    assert!(msg.contains("回滚到恢复前状态"), "{msg}");
    assert!(!msg.contains("回滚失败"), "{msg}");
    assert_eq!(
        fingerprint(&save),
        before,
        "rollback must also remove entries the backup does not have"
    );
    let _ = fs::remove_dir_all(&root);
}

/// Staging creates directories for covered paths that do not exist in the
/// pre-restore save; when staging then aborts, those directories must be
/// removed again — the "当前存档未改动" claim must be true for the whole
/// tree, directory structure included.
#[test]
fn staging_abort_removes_the_directories_staging_created() {
    let (root, save) = fixture("stage-dirs");
    let id = snapshot(&save, None);
    // Staging order is the manifest's sorted key order: staging
    // `Characters/tess.md` re-creates the deleted `Characters/` directory,
    // then `combat/campaign.yaml` finds a file where its directory must be
    // and staging aborts with the `Characters/` directory already created.
    fs::remove_dir_all(save.join("Characters")).unwrap();
    fs::remove_dir_all(save.join("combat")).unwrap();
    fs::write(save.join("combat"), "not a directory\n").unwrap();
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    let before = fingerprint(&save);
    let err = restore(&save, &id, true).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("恢复已中止"), "{msg}");
    assert!(msg.contains("未改动"), "{msg}");
    assert!(msg.contains("Saves/_backups/"), "{msg}");
    assert_eq!(
        fingerprint(&save),
        before,
        "a staging abort must not leave the directories staging created"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn real_commit_obstruction_still_rolls_back_cleanly() {
    let (root, save) = fixture("real-eisdir");
    let id = snapshot(&save, None);
    // Real-world partial-commit trigger (no injection): sheets/tess.yaml is
    // the last covered file in commit order, so every earlier rename has
    // already committed when the rename onto the directory fails. The
    // rollback must still bring the whole tree back.
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();
    fs::write(save.join("Characters/nick.md"), "# Nick\n").unwrap();
    fs::remove_file(save.join("sheets/tess.yaml")).unwrap();
    fs::create_dir_all(save.join("sheets/tess.yaml")).unwrap();
    let before = fingerprint(&save);
    let err = restore(&save, &id, true).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("恢复失败"), "{msg}");
    assert!(msg.contains("已从"), "{msg}");
    assert!(!msg.contains("回滚失败"), "{msg}");
    assert_eq!(
        fingerprint(&save),
        before,
        "a real commit failure must still roll back the whole tree"
    );
    let _ = fs::remove_dir_all(&root);
}

/// Regression test for issue #154: restore's final integrity manifest
/// read-modify-write runs under the save's integrity manifest lock, so a
/// concurrent tool command that commits right before the re-baseline keeps
/// its fingerprints: the in-lock load picks the committed update up instead
/// of being overwritten by a manifest that was read before it committed.
/// The concurrent writer is fired through the test-only
/// `restore_injecting_manifest_hook` at exactly that point — inside
/// `restore_rebaseline`, after the integrity manifest lock is taken and
/// before the manifest is read. The hook first proves the lock is held by
/// opening a second handle on `.talkdnd.integrity.lock` and asserting
/// `try_lock()` returns `WouldBlock`: without the lock this assertion
/// fails instead of the whole test silently passing on pre-lock code. It
/// then commits the writer's update with a raw manifest read-modify-write —
/// a real writer reaches the same state through the lock, but the hook must
/// not re-acquire the lock the code under test already holds. On pre-lock
/// code restore's store dropped the writer's fingerprint — the manifest kept
/// the checkpoint's stale fingerprint for the file the writer had
/// rewritten, reported forever as a hand edit.
#[test]
fn restore_rebaseline_keeps_a_concurrent_writers_fingerprints() {
    let (root, save) = fixture("rebaseline-race");
    let id = snapshot(&save, None);
    // Diverge the save from the checkpoint, so the restore really replaces
    // covered files before it re-baselines the manifest.
    fs::write(save.join("progress.yaml"), "xp: {tess: 999}\n").unwrap();

    const TESS_REWRITTEN: &str =
        "# Tess\n\n```combat yaml\nid: tess\nname: Tess\nlevel: 3\nhp: 15\nmax_hp: 20\n```\n";
    let b_save = save.clone();
    let writer_b = move || {
        // The hook runs with the integrity manifest lock held: a second
        // opener must be locked out. This is the assertion that fails on
        // pre-lock code (issue #154).
        let probe = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(b_save.join(combat_sim::integrity::LOCK))
            .unwrap();
        match probe.try_lock() {
            Err(std::fs::TryLockError::WouldBlock) => {}
            other => panic!(
                "the integrity manifest lock must be held at the hook point; try_lock returned {other:?}"
            ),
        }
        // The concurrent tool command rewrites a protected file and records
        // it with a raw manifest read-modify-write: a real writer reaches
        // the same state through the lock, but the hook must not re-acquire
        // the lock the code under test already holds.
        fs::write(b_save.join("Characters/tess.md"), TESS_REWRITTEN).unwrap();
        let mut m = combat_sim::integrity::load(&b_save).unwrap().unwrap();
        m.files.insert(
            "Characters/tess.md".to_string(),
            combat_sim::integrity::fingerprint(TESS_REWRITTEN.as_bytes()),
        );
        combat_sim::integrity::store(&b_save, &m).unwrap();
    };
    checkpoint::restore_injecting_manifest_hook(&root, &save, &id, &writer_b).unwrap();

    // The manifest matches the disk exactly — every fingerprint, the
    // concurrent writer's included — so `check` reports no changes.
    let m = combat_sim::integrity::load(&save).unwrap().unwrap();
    let now = combat_sim::integrity::snapshot(&save);
    assert_eq!(m.files, now, "manifest and disk diverged after restore");
    assert!(
        combat_sim::integrity::changes_against(&m, &now).is_empty(),
        "{:?}",
        combat_sim::integrity::changes_against(&m, &now)
    );
    assert_eq!(
        m.files.get("Characters/tess.md").map(String::as_str),
        Some(combat_sim::integrity::fingerprint(TESS_REWRITTEN.as_bytes()).as_str()),
        "the concurrent writer's fingerprint was lost to the restore"
    );
    let _ = fs::remove_dir_all(&root);
}
