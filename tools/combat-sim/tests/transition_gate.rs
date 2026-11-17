//! ADV-P1-1pre cross-CLI transition gate, exercised on the **real**
//! `combat-sim` call path.
//!
//! `combat-sim` must never depend on `talkdnd`; it depends on the shared
//! `advancement-lock` crate. Every `combat-sim` row of the shared writer
//! registry is run as a real subprocess against a synthetic save that has the
//! advancement control files.

use advancement_lock::{Class, Cli, WRITERS};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const LOCK: &str = ".talkdnd.advancement.lock";
const STATE: &str = ".talkdnd.advancement.state";

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "combat-sim-gate-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

struct Fixture {
    root: PathBuf,
    campaign: PathBuf,
    state: PathBuf,
}

/// A temp copy of the engine fixture campaign plus one real encounter state.
fn fixture(tag: &str) -> Fixture {
    let root = temp_root(tag);
    let campaign = root.join("campaign");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/engine"),
        &campaign,
    );
    let manifest = campaign.join("manifest.yaml");
    let state = root.join("state/e.state.json");
    fs::create_dir_all(state.parent().unwrap()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--state")
        .arg(&state)
        .args([
            "new",
            "--encounter",
            "test-fight",
            "--party",
            "striker-a,warden-b",
            "--campaign",
        ])
        .arg(&manifest)
        .output()
        .expect("failed to run combat-sim");
    assert!(
        out.status.success(),
        "fixture state failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Fixture {
        root,
        campaign: manifest,
        state,
    }
}

fn run(fixture: &Fixture, save: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--state")
        .arg(&fixture.state)
        .arg("--save")
        .arg(save)
        .arg("--campaign")
        .arg(&fixture.campaign)
        .args(args)
        .output()
        .expect("failed to run combat-sim")
}

fn output_text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
}

fn argv(row: &advancement_lock::Writer, save: &Path) -> Vec<String> {
    let save = save.to_string_lossy();
    row.argv
        .iter()
        .map(|arg| arg.replace("{save}", &save))
        .collect()
}

fn argv_refs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

/// Rows that need content or an index the synthetic campaign does not carry.
/// They may fail for their own reason; the gate must not be the reason.
const CONTENT_DEPENDENT: &[&str] = &[
    "combat-sim:simulate",
    "combat-sim:pack-build-dry-run",
    "combat-sim:pack-release-dry-run",
];

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .map(|entry| entry.unwrap())
            .collect();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap().to_path_buf();
            if entry.file_type().unwrap().is_dir() {
                out.insert(rel, None);
                walk(base, &path, out);
            } else {
                out.insert(rel, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn adopted_save(fixture: &Fixture) -> PathBuf {
    let save = fixture.root.join("Saves/x");
    fs::create_dir_all(save.join("combat")).unwrap();
    fs::write(save.join(LOCK), b"probe lock\n").unwrap();
    fs::write(save.join(STATE), b"probe state\n").unwrap();
    save
}

#[test]
fn unadopted_writers_exit_one_and_leave_the_save_untouched() {
    let fixture = fixture("refuse");
    let save = adopted_save(&fixture);
    let before = snapshot(&save);
    let rows: Vec<_> = WRITERS
        .iter()
        .filter(|row| row.cli == Cli::CombatSim && row.class == Class::UnadoptedWrite)
        .collect();
    assert!(rows.len() > 8, "the combat-sim registry looks too small");
    for row in rows {
        let args = argv(row, &save);
        let out = run(&fixture, &save, &argv_refs(&args));
        let text = output_text(&out);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{} must exit 1, got {:?}\n{text}",
            row.entry,
            out.status.code()
        );
        assert!(
            text.contains("已启用成长协议"),
            "{}: refusal must name the protocol\n{text}",
            row.entry
        );
        assert!(
            text.contains("尚未接入成长协议"),
            "{}: refusal must say the entry is not wired\n{text}",
            row.entry
        );
        assert!(
            text.contains("未写入任何目标文件"),
            "{}: refusal must say nothing was written\n{text}",
            row.entry
        );
        assert_eq!(
            snapshot(&save),
            before,
            "{} changed the save tree while refusing",
            row.entry
        );
    }
}

#[test]
fn read_only_entries_stay_available_under_the_protocol() {
    let fixture = fixture("readonly");
    let save = adopted_save(&fixture);
    let before = snapshot(&save);
    let rows: Vec<_> = WRITERS
        .iter()
        .filter(|row| row.cli == Cli::CombatSim && row.class == Class::ReadOnly)
        .collect();
    assert!(rows.len() > 5, "the combat-sim registry looks too small");
    for row in rows {
        let args = argv(row, &save);
        let out = run(&fixture, &save, &argv_refs(&args));
        let text = output_text(&out);
        assert!(
            !text.contains("尚未接入成长协议"),
            "{} must not be refused\n{text}",
            row.entry
        );
        if !CONTENT_DEPENDENT.contains(&row.entry) {
            assert!(
                out.status.success(),
                "{} must still succeed under the protocol\n{text}",
                row.entry
            );
        }
        assert_eq!(
            snapshot(&save),
            before,
            "{} wrote the save while declared read-only",
            row.entry
        );
    }
}

#[test]
fn without_the_protocol_state_commands_keep_working() {
    let fixture = fixture("noprotocol");
    let save = fixture.root.join("Saves/y");
    fs::create_dir_all(save.join("combat")).unwrap();
    assert!(!save.join(LOCK).exists());
    let init = run(&fixture, &save, &["init", "--seed", "7"]);
    assert!(
        init.status.success(),
        "init without the protocol failed: {}",
        output_text(&init)
    );
    let status = run(&fixture, &save, &["status"]);
    assert!(status.status.success(), "{}", output_text(&status));
    // The gate created nothing.
    assert!(!save.join(LOCK).exists());
    assert!(!save.join(STATE).exists());
}

#[test]
fn help_declares_the_refusal_and_its_exit_code() {
    let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--help")
        .output()
        .expect("failed to run combat-sim --help");
    let top = output_text(&out);
    assert!(top.contains("成长协议"), "{top}");
    assert!(top.contains("退出码 1"), "{top}");
    for command in ["attack", "party", "content"] {
        let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args([command, "--help"])
            .output()
            .expect("failed to run combat-sim --help");
        let text = output_text(&out);
        assert!(
            text.contains("退出码 1"),
            "`combat-sim {command} --help` must declare the refusal\n{text}"
        );
    }
    for args in [
        vec!["party", "rest", "--help"],
        vec!["content", "pack", "build", "--help"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(&args)
            .output()
            .expect("failed to run combat-sim --help");
        let text = output_text(&out);
        assert!(
            text.contains("退出码 1"),
            "`combat-sim {}` must declare the refusal\n{text}",
            args.join(" ")
        );
    }
}
