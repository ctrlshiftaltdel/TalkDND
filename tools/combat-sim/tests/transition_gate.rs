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
    for &args in WRITE_HELP {
        let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(args)
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

/// Every write leaf of `combat-sim`, as a `--help` path.
const WRITE_HELP: &[&[&str]] = &[
    &["new", "--help"],
    &["quick", "--help"],
    &["init", "--help"],
    &["attack", "--help"],
    &["grapple", "--help"],
    &["shove", "--help"],
    &["damage", "--help"],
    &["heal", "--help"],
    &["save", "--help"],
    &["check", "--help"],
    &["roll", "--help"],
    &["cast", "--help"],
    &["use", "--help"],
    &["condition", "add", "--help"],
    &["condition", "remove", "--help"],
    &["move", "--help"],
    &["ready", "--help"],
    &["trigger", "--help"],
    &["ready-clear", "--help"],
    &["fly", "--help"],
    &["fall", "--help"],
    &["interact", "--help"],
    &["deathsave", "--help"],
    &["next", "--help"],
    &["rest", "--help"],
    &["activate", "--help"],
    &["add", "--help"],
    &["party", "rest", "--help"],
    &["party", "set-hp", "--help"],
    &["party", "sync", "--help"],
    &["content", "import-srd", "--help"],
    &["content", "import-srd-prose", "--help"],
    &["content", "import-srd-pack", "--help"],
    &["content", "pack", "build", "--help"],
    &["content", "pack", "release", "--help"],
];

/// A relative `--state` that points into a protected save is refused even
/// though `--save` names an unprotected directory.
#[test]
fn a_relative_state_path_inside_a_protected_save_is_refused() {
    let fixture = fixture("relative-state");
    let protected = fixture.root.join("Saves/p");
    fs::create_dir_all(protected.join("combat")).unwrap();
    fs::write(protected.join(LOCK), b"probe lock\n").unwrap();
    let plain = fixture.root.join("Saves/plain");
    fs::create_dir_all(&plain).unwrap();
    let before = snapshot(&protected);
    let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(fixture.root.join("Saves"))
        .args(["--state", "p/combat/e.state.json", "--save"])
        .arg(&plain)
        .args(["init", "--seed", "1"])
        .output()
        .expect("failed to run combat-sim");
    let text = output_text(&out);
    assert_eq!(out.status.code(), Some(1), "must exit 1\n{text}");
    assert!(text.contains("已启用成长协议"), "{text}");
    assert_eq!(snapshot(&protected), before);
}

/// A symlink that reaches a protected save is resolved, not walked lexically.
#[cfg(unix)]
#[test]
fn a_state_path_through_a_symlink_to_a_protected_save_is_refused() {
    let fixture = fixture("symlink-state");
    let protected = fixture.root.join("Saves/p");
    fs::create_dir_all(protected.join("combat")).unwrap();
    fs::write(protected.join(STATE), b"probe state\n").unwrap();
    let plain = fixture.root.join("Saves/plain");
    fs::create_dir_all(&plain).unwrap();
    let alias = fixture.root.join("alias");
    std::os::unix::fs::symlink(&protected, &alias).unwrap();
    let before = snapshot(&protected);
    let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .args(["--state"])
        .arg(alias.join("combat/e.state.json"))
        .arg("--save")
        .arg(&plain)
        .args(["init", "--seed", "1"])
        .output()
        .expect("failed to run combat-sim");
    let text = output_text(&out);
    assert_eq!(out.status.code(), Some(1), "must exit 1\n{text}");
    assert!(text.contains("已启用成长协议"), "{text}");
    assert_eq!(snapshot(&protected), before);
}

/// A campaign outside the save whose `state_dir` points into a protected save
/// is refused: the party state target is a write target.
#[test]
fn a_campaign_state_dir_inside_a_protected_save_is_refused() {
    let fixture = fixture("manifest-state-dir");
    let protected = fixture.root.join("Saves/p");
    let state_dir = protected.join("combat");
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(protected.join(LOCK), b"probe lock\n").unwrap();
    let plain = fixture.root.join("Saves/plain");
    fs::create_dir_all(&plain).unwrap();
    let outside = fixture.root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let campaign = outside.join("campaign.yaml");
    fs::write(
        &campaign,
        format!(
            "name: Outside\nparty: [striker-a]\nstate_dir: {}\n",
            state_dir.display()
        ),
    )
    .unwrap();
    let before = snapshot(&protected);
    let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .arg("--save")
        .arg(&plain)
        .arg("--campaign")
        .arg(&campaign)
        .args(["party", "set-hp", "striker-a", "1"])
        .output()
        .expect("failed to run combat-sim");
    let text = output_text(&out);
    assert_eq!(out.status.code(), Some(1), "must exit 1\n{text}");
    assert!(text.contains("已启用成长协议"), "{text}");
    assert_eq!(snapshot(&protected), before);
}

/// A state file outside the save whose recorded `party_state` points into a
/// protected save is refused.
#[test]
fn a_state_party_state_inside_a_protected_save_is_refused() {
    let fixture = fixture("state-party-state");
    let protected = fixture.root.join("Saves/p");
    let party = protected.join("combat/party-state.yaml");
    fs::create_dir_all(party.parent().unwrap()).unwrap();
    fs::write(protected.join(LOCK), b"probe lock\n").unwrap();
    let plain = fixture.root.join("Saves/plain");
    fs::create_dir_all(&plain).unwrap();
    let outside = fixture.root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    // Start from the valid fixture state and only rewrite `party_state`.
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fixture.state).unwrap()).unwrap();
    value["party_state"] = serde_json::Value::String(party.to_string_lossy().into_owned());
    let state = outside.join("e.state.json");
    fs::write(&state, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    let before = snapshot(&protected);
    let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .arg("--state")
        .arg(&state)
        .arg("--save")
        .arg(&plain)
        .args(["init", "--seed", "1"])
        .output()
        .expect("failed to run combat-sim");
    let text = output_text(&out);
    assert_eq!(out.status.code(), Some(1), "must exit 1\n{text}");
    assert!(text.contains("已启用成长协议"), "{text}");
    assert_eq!(snapshot(&protected), before);
}

/// A state file whose `party_state` is outside every save keeps its old
/// behaviour.
#[test]
fn a_state_party_state_outside_the_saves_is_not_refused() {
    let fixture = fixture("state-party-state-outside");
    let plain = fixture.root.join("Saves/plain");
    fs::create_dir_all(&plain).unwrap();
    let outside = fixture.root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fixture.state).unwrap()).unwrap();
    value["party_state"] = serde_json::Value::String(
        outside
            .join("party-state.yaml")
            .to_string_lossy()
            .into_owned(),
    );
    let state = outside.join("e.state.json");
    fs::write(&state, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .arg("--state")
        .arg(&state)
        .arg("--save")
        .arg(&plain)
        .args(["status"])
        .output()
        .expect("failed to run combat-sim");
    let text = output_text(&out);
    assert!(!text.contains("尚未接入成长协议"), "{text}");
    assert!(
        out.status.success(),
        "status on an external state must keep working\n{text}"
    );
}
