//! ADV-P1-1a: adopted talkdnd writers take the SAVE lock, exercised on the
//! **real** `talkdnd` binary.
//!
//! A save without protocol files keeps its existing behaviour and is never
//! auto-initialised. On a protocol-enabled save, group A writers acquire the
//! SAVE lock before writing; if the lock or state is missing, unwritable,
//! `active` or `in_doubt`, they fail closed and leave the tree unchanged.
//! Missing SAVE state on a protocol-enabled save is the plan's conservative
//! temporary handling, documented in the refusal text. Group C (checkpoint
//! snapshot/restore and migrate) is adopted in this file's 1c tests. Leftover
//! 1d–1e, `checkpoint <text>`, and `handout build|mark|list --dm` writers
//! still hit the transition gate.

use advancement_lock::{Class, Cli, WRITERS};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use talkdnd::advancement_lock as adv;

const SAVE: &str = "Saves/2030-01-02-01";
const SLOT: &str = "2030-01-02-01";

fn play_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "talkdnd-p1a-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("Saves")).unwrap();
    fs::write(root.join("AGENTS.md"), "x").unwrap();
    root
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .arg("--root")
        .arg(root)
        .args(args)
        .env("TALKDND_NO_PDF2MD", "1")
        .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("failed to run talkdnd")
}

fn run_slot(root: &Path, args: &[&str]) -> Output {
    let mut full = vec!["--save", SLOT];
    full.extend_from_slice(args);
    run(root, &full)
}

fn output_text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
}

fn ok(root: &Path, args: &[&str]) -> String {
    let out = run(root, args);
    assert!(
        out.status.success(),
        "`{}` failed: {}",
        args.join(" "),
        output_text(&out)
    );
    output_text(&out)
}

fn ok_slot(root: &Path, args: &[&str]) -> String {
    let out = run_slot(root, args);
    assert!(
        out.status.success(),
        "`--save {SLOT} {}` failed: {}",
        args.join(" "),
        output_text(&out)
    );
    output_text(&out)
}

/// A synthetic save with one valid level-1 character (`aria`), built through
/// the real CLI. No real player save is read.
fn playable(tag: &str) -> PathBuf {
    let root = play_root(tag);
    ok(&root, &["new", "--date", "2030-01-02"]);
    ok(&root, &["chargen", "set", "ruleset", "2024"]);
    ok(&root, &["chargen", "set", "adventure", "Gate Probe"]);
    ok(&root, &["chargen", "set", "species", "human"]);
    ok(&root, &["chargen", "set", "class", "fighter"]);
    ok(&root, &["chargen", "set", "level", "1"]);
    ok(&root, &["chargen", "set", "background", "soldier"]);
    ok(&root, &["chargen", "array"]);
    ok(
        &root,
        &[
            "chargen", "assign", "str=15", "con=14", "dex=13", "wis=12", "int=10", "cha=8",
        ],
    );
    ok(&root, &["chargen", "asi", "str=2", "con=1"]);
    ok(
        &root,
        &[
            "chargen",
            "set",
            "skills",
            "athletics,intimidation,perception,survival,history",
        ],
    );
    ok(&root, &["chargen", "set", "armor", "chain-mail"]);
    ok(&root, &["chargen", "set", "shield", "yes"]);
    ok(&root, &["chargen", "set", "spells", "none"]);
    ok(&root, &["chargen", "set", "name", "Aria"]);
    ok(&root, &["chargen", "set", "fighting-style", "defense"]);
    ok(&root, &["chargen", "set", "background-equipment", "B"]);
    ok(&root, &["chargen", "set", "languages", "none"]);
    for field in ["traits", "ideals", "bonds", "flaws"] {
        ok(&root, &["chargen", "set", field, "由玩家决定"]);
    }
    ok(&root, &["chargen", "validate"]);
    ok(&root, &["chargen", "sheet"]);
    ok(&root, &["chargen", "confirm"]);
    let save = root.join(SAVE);
    fs::write(save.join("gate-probe-terms.txt"), b"probe-term\n").unwrap();
    root
}

fn init_protocol(root: &Path, save: &Path) {
    match adv::init(root, save, &adv::InitOptions::new()) {
        Ok(_) => {}
        Err(err) if err.kind() == adv::ErrorKind::StatePublishedNotDurable => {}
        Err(err) => panic!("init protocol: {err}"),
    }
    assert!(
        save.join(adv::LOCK_FILE).is_file() && save.join(adv::STATE_FILE).is_file(),
        "init must create both protocol files"
    );
}

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

fn adopted_rows() -> Vec<&'static advancement_lock::Writer> {
    WRITERS
        .iter()
        .filter(|row| row.cli == Cli::Talkdnd && row.class == Class::AdoptedWrite)
        .collect()
}

fn plant_state(save: &Path, mutate: impl FnOnce(&mut adv::StateRecord)) {
    let path = adv::state_path(save);
    let mut record: adv::StateRecord = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    mutate(&mut record);
    adv::reseal(&mut record).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
}

fn assert_refused_unwritten(
    out: &Output,
    save: &Path,
    before: &BTreeMap<PathBuf, Option<Vec<u8>>>,
    entry: &str,
    expect: &[&str],
) {
    let text = output_text(out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{entry} must exit 1, got {:?}\n{text}",
        out.status.code()
    );
    assert!(
        text.contains("已启用成长协议"),
        "{entry}: refusal must name the protocol\n{text}"
    );
    assert!(
        text.contains("未写入任何目标文件"),
        "{entry}: refusal must say nothing was written (not maybe-written)\n{text}"
    );
    assert!(
        !text.contains("可能已被写入"),
        "{entry}: lock-unavailable refusal must not use the maybe-written stage\n{text}"
    );
    for needle in expect {
        assert!(
            text.contains(needle),
            "{entry}: expected `{needle}` in\n{text}"
        );
    }
    assert_eq!(
        snapshot(save),
        *before,
        "{entry} wrote the save while refusing"
    );
}

fn restore_protocol_files(save: &Path, lock: &[u8], state: &[u8]) {
    fs::write(save.join(adv::LOCK_FILE), lock).unwrap();
    fs::write(save.join(adv::STATE_FILE), state).unwrap();
}

#[test]
fn adopted_writers_run_under_a_ready_protocol_and_do_not_auto_init() {
    let root = playable("ready");
    let save = root.join(SAVE);
    assert!(!save.join(adv::LOCK_FILE).exists());
    assert!(!save.join(adv::STATE_FILE).exists());
    init_protocol(&root, &save);

    let xp = ok_slot(&root, &["xp", "add", "10", "--reason", "p1a-ready"]);
    assert!(!xp.contains("尚未接入成长协议"), "{xp}");
    assert!(!xp.contains("无法取得 SAVE 锁"), "{xp}");

    ok_slot(&root, &["scene", "p1a-ready-scene"]);
    ok_slot(&root, &["char", "aria", "damage", "1"]);
    ok_slot(
        &root,
        &["loot", "add", "--gp", "1", "probe", "--reason", "p1a-ready"],
    );
    ok_slot(&root, &["memory", "add", "p1a ready fact"]);
    ok_slot(&root, &["secret", "add", "p1a-ready-secret"]);
    ok_slot(&root, &["time", "enable"]);

    // Existing structured level-up, including `--hp roll`, stays available
    // under the protocol (AC-4). Milestone avoids depending on XP thresholds.
    let level = ok_slot(
        &root,
        &[
            "level-up",
            "aria",
            "--hp",
            "roll",
            "--milestone",
            "--reason",
            "p1a structured level-up",
        ],
    );
    assert!(!level.contains("尚未接入成长协议"), "{level}");
    let yaml = fs::read_to_string(save.join("sheets/aria.yaml")).unwrap();
    assert!(
        yaml.contains("level: 2"),
        "structured level-up should persist level 2\n{yaml}"
    );

    // The protocol files still exist; nothing auto-created a sidecar or schema 11.
    assert!(save.join(adv::LOCK_FILE).is_file());
    assert!(save.join(adv::STATE_FILE).is_file());
    assert!(!save.join("advancement-choice.yaml").exists());
    assert!(!save.join(".talkdnd.advancement.sidecar").exists());
}

#[test]
fn markdown_level_up_stays_available_under_the_protocol() {
    let root = playable("md-level");
    let save = root.join(SAVE);
    // A Markdown-only character has no sheets/<id>.yaml, so level-up takes
    // the inventory's Markdown path rather than the structured commit.
    let md = save.join("Characters/quill.md");
    fs::write(
        &md,
        r#"# Quill

- **等级**：fighter 1

```combat yaml
id: quill
name: Quill
kind: pc
level: 1
hp: 12
prof: 2
abilities: {str: 16, dex: 14, con: 14, int: 10, wis: 12, cha: 8}
saves: {str: 5, con: 4}
skills: {athletics: 5}
resources:
  hit-dice: {max: 1, recharge: long, die: 10}
attacks:
  - {id: blade, name: Blade, kind: melee, ability: str, to_hit: 5, damage: [{dice: "1d8+3", type: slashing}], reach: 5}
notes: "fighter 1. synthetic"
```
"#,
    )
    .unwrap();
    assert!(!save.join("sheets/quill.yaml").exists());
    init_protocol(&root, &save);
    let text = ok_slot(
        &root,
        &[
            "level-up",
            "quill",
            "--hp",
            "5",
            "--milestone",
            "--reason",
            "p1a markdown level-up",
        ],
    );
    assert!(!text.contains("尚未接入成长协议"), "{text}");
    assert!(
        !save.join("sheets/quill.yaml").exists(),
        "Markdown path must not create a structured sheet"
    );
    let body = fs::read_to_string(&md).unwrap();
    assert!(
        body.contains("level: 2"),
        "Markdown level-up should change the sheet\n{body}"
    );
}

#[test]
fn missing_state_or_lock_refuses_every_adopted_writer_and_writes_nothing() {
    let root = playable("missing");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let lock_bytes = fs::read(save.join(adv::LOCK_FILE)).unwrap();
    let state_bytes = fs::read(save.join(adv::STATE_FILE)).unwrap();
    let rows = adopted_rows();
    assert_eq!(
        rows.len(),
        30,
        "group A + group C adopted talkdnd registry size"
    );

    // Lock exists, SAVE state is missing: conservative temporary handling.
    fs::remove_file(save.join(adv::STATE_FILE)).unwrap();
    let before = snapshot(&save);
    for row in &rows {
        let args = argv(row, &save);
        let out = run_slot(&root, &argv_refs(&args));
        assert_refused_unwritten(
            &out,
            &save,
            &before,
            row.entry,
            &["SAVE state 缺失", "不自动初始化"],
        );
    }

    // State exists, lock file is missing: still protocol-enabled, still refused.
    restore_protocol_files(&save, &lock_bytes, &state_bytes);
    fs::remove_file(save.join(adv::LOCK_FILE)).unwrap();
    let before = snapshot(&save);
    for row in &rows {
        let args = argv(row, &save);
        let out = run_slot(&root, &argv_refs(&args));
        assert_refused_unwritten(
            &out,
            &save,
            &before,
            row.entry,
            &["SAVE 锁文件缺失", "不自动初始化"],
        );
    }
    restore_protocol_files(&save, &lock_bytes, &state_bytes);
}

#[test]
fn active_and_in_doubt_state_refuse_adopted_writers() {
    let root = playable("states");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let state_bytes = fs::read(save.join(adv::STATE_FILE)).unwrap();
    let rows = adopted_rows();

    plant_state(&save, |record| {
        record.state = adv::State::Active;
        record.stage = adv::Stage::Admit;
        record.operation_id = "p1a-plant-active".into();
    });
    let before = snapshot(&save);
    for row in &rows {
        let args = argv(row, &save);
        let out = run_slot(&root, &argv_refs(&args));
        assert_refused_unwritten(&out, &save, &before, row.entry, &["active"]);
    }

    fs::write(save.join(adv::STATE_FILE), &state_bytes).unwrap();
    plant_state(&save, |record| {
        record.state = adv::State::InDoubt;
        record.stage = adv::Stage::Publish;
        record.operation_id = "p1a-plant-in-doubt".into();
        record.last_error = Some("planted".into());
    });
    let before = snapshot(&save);
    for row in &rows {
        let args = argv(row, &save);
        let out = run_slot(&root, &argv_refs(&args));
        assert_refused_unwritten(&out, &save, &before, row.entry, &["in_doubt"]);
    }
}

#[cfg(unix)]
#[test]
fn unreadable_state_refuses_adopted_writers() {
    use std::os::unix::fs::PermissionsExt;
    let root = playable("unwritable");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let state = save.join(adv::STATE_FILE);
    let before = snapshot(&save);
    let original = fs::metadata(&state).unwrap().permissions();
    let mut off = original.clone();
    off.set_mode(0o000);
    fs::set_permissions(&state, off).unwrap();
    let out = run_slot(&root, &["xp", "add", "1", "--reason", "unwritable"]);
    let text = output_text(&out);
    fs::set_permissions(&state, original).unwrap();
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("未写入任何目标文件"), "{text}");
    assert!(text.contains("已启用成长协议"), "{text}");
    assert_eq!(
        snapshot(&save),
        before,
        "unreadable SAVE state must not be repaired or written past"
    );
}

#[test]
fn a_held_save_lock_refuses_the_writer_and_writes_nothing() {
    let root = playable("held");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let before = snapshot(&save);
    let held = adv::open(&root, &save, &adv::OpenOptions::new()).unwrap();
    let out = run_slot(&root, &["xp", "add", "1", "--reason", "held-lock"]);
    let text = output_text(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("未写入任何目标文件"), "{text}");
    assert!(text.contains("占用"), "{text}");
    // Control-metadata commands must not take the SAVE lock, so they still run
    // while another holder has it (AC-3).
    let guard = run_slot(&root, &["guard", "held-lock probe"]);
    assert!(
        guard.status.success(),
        "guard must not take the SAVE lock\n{}",
        output_text(&guard)
    );
    drop(held);
    // Compare after release: reading the lock file while it is held is
    // blocked on Windows. `guard` may have written the secret-term cache.
    let after = snapshot(&save);
    for (path, bytes) in &after {
        if path.ends_with("secret-terms.cache.json") {
            continue;
        }
        assert_eq!(
            before.get(path),
            Some(bytes),
            "held SAVE lock: unexpected change to {}",
            path.display()
        );
    }
}

#[test]
fn char_spells_view_is_read_only_and_other_char_ops_take_save() {
    let root = playable("char-ro");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let before = snapshot(&save);
    let view = run_slot(&root, &["char", "aria", "spells"]);
    let text = output_text(&view);
    assert!(
        !text.contains("尚未接入成长协议") && !text.contains("无法取得 SAVE 锁"),
        "ordinary spells view must stay available\n{text}"
    );
    assert_eq!(
        snapshot(&save),
        before,
        "char spells ordinary view changed the save tree"
    );

    // With SAVE state missing, a writing char op (including `show`) fails closed.
    let state = fs::read(save.join(adv::STATE_FILE)).unwrap();
    fs::remove_file(save.join(adv::STATE_FILE)).unwrap();
    let before = snapshot(&save);
    for args in [
        ["char", "aria", "show"].as_slice(),
        ["char", "aria", "damage", "1"].as_slice(),
    ] {
        let out = run_slot(&root, args);
        assert_refused_unwritten(
            &out,
            &save,
            &before,
            &format!("char {}", args[2]),
            &["SAVE state 缺失"],
        );
    }
    fs::write(save.join(adv::STATE_FILE), state).unwrap();
}

#[test]
fn chargen_status_and_validate_stay_read_only() {
    let root = playable("chargen-ro");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let before = snapshot(&save);
    for args in [
        ["chargen", "status"].as_slice(),
        ["chargen", "validate"].as_slice(),
    ] {
        let out = run_slot(&root, args);
        let text = output_text(&out);
        assert!(
            out.status.success(),
            "`{}` should stay readable\n{text}",
            args.join(" ")
        );
        assert_eq!(
            snapshot(&save),
            before,
            "`{}` wrote the save",
            args.join(" ")
        );
    }
}

#[test]
fn time_show_and_event_list_stay_read_only() {
    let root = playable("time-ro");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let before = snapshot(&save);
    for args in [
        ["time", "show"].as_slice(),
        ["time", "event", "list"].as_slice(),
    ] {
        let out = run_slot(&root, args);
        let text = output_text(&out);
        assert!(
            !text.contains("尚未接入成长协议") && !text.contains("无法取得 SAVE 锁"),
            "`{}` must stay available\n{text}",
            args.join(" ")
        );
        assert_eq!(
            snapshot(&save),
            before,
            "`{}` wrote the save",
            args.join(" ")
        );
    }
}

#[test]
fn guard_and_secret_list_do_not_take_the_save_lock() {
    let root = playable("cache");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    plant_state(&save, |record| {
        record.state = adv::State::Active;
        record.stage = adv::Stage::Admit;
        record.operation_id = "p1a-cache".into();
    });
    for args in [
        ["guard", "cache probe"].as_slice(),
        ["secret", "list"].as_slice(),
    ] {
        let out = run_slot(&root, args);
        let text = output_text(&out);
        assert!(
            out.status.success(),
            "`{}` is control metadata and must run even when SAVE is active\n{text}",
            args.join(" ")
        );
        assert!(!text.contains("无法取得 SAVE 锁"), "{text}");
    }
}

#[test]
fn new_does_not_init_and_does_not_write_a_protocol_save() {
    let root = playable("newslot");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let before = snapshot(&save);
    let text = ok(&root, &["new", "--date", "2031-01-01"]);
    assert!(text.contains("created Saves/2031-01-01-01"), "{text}");
    let created = root.join("Saves/2031-01-01-01");
    assert!(created.join("chargen.json").is_file());
    assert!(
        !created.join(adv::LOCK_FILE).exists() && !created.join(adv::STATE_FILE).exists(),
        "`new` must not auto-init the protocol"
    );
    assert_eq!(
        snapshot(&save),
        before,
        "`new` touched a protocol-enabled save"
    );
}

#[test]
fn start_takes_the_save_lock_for_user_data_and_writes_nothing_when_unavailable() {
    let root = playable("start");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let state = fs::read(save.join(adv::STATE_FILE)).unwrap();
    fs::remove_file(save.join(adv::STATE_FILE)).unwrap();
    let before = snapshot(&save);
    let out = run_slot(&root, &["start", "--no-pdf2md"]);
    assert_refused_unwritten(&out, &save, &before, "talkdnd:start", &["SAVE state 缺失"]);
    assert!(
        !save.join("digest.md").exists() || before.contains_key(&PathBuf::from("digest.md")),
        "start must not write digest.md when the SAVE lock is unavailable"
    );
    fs::write(save.join(adv::STATE_FILE), state).unwrap();

    let ready = run_slot(&root, &["start", "--no-pdf2md"]);
    let text = output_text(&ready);
    assert!(
        !text.contains("尚未接入成长协议"),
        "start must be adopted, not transition-gate refused\n{text}"
    );
}

#[test]
fn leftover_unadopted_writers_are_still_refused_by_the_transition_gate() {
    let root = playable("leftover");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    let before = snapshot(&save);
    for (entry, args) in [
        ("talkdnd:log", ["log", "p1c leftover"].as_slice()),
        (
            "talkdnd:checkpoint-text",
            ["checkpoint", "p1c leftover"].as_slice(),
        ),
        ("talkdnd:check", ["check"].as_slice()),
    ] {
        let out = run_slot(&root, args);
        let text = output_text(&out);
        assert_eq!(out.status.code(), Some(1), "{entry}\n{text}");
        assert!(text.contains("尚未接入成长协议"), "{entry}\n{text}");
        assert!(text.contains("未写入任何目标文件"), "{entry}\n{text}");
        assert_eq!(snapshot(&save), before, "{entry} wrote the save");
    }
}

#[test]
fn without_the_protocol_adopted_writers_keep_working_and_do_not_init() {
    let root = playable("noproto");
    let save = root.join(SAVE);
    assert!(!save.join(adv::LOCK_FILE).exists());
    assert!(!save.join(adv::STATE_FILE).exists());
    ok_slot(&root, &["xp", "add", "10", "--reason", "no-protocol"]);
    ok_slot(&root, &["scene", "no-protocol-scene"]);
    ok_slot(&root, &["char", "aria", "damage", "1"]);
    ok_slot(&root, &["start", "--no-pdf2md"]);
    assert!(!save.join(adv::LOCK_FILE).exists());
    assert!(!save.join(adv::STATE_FILE).exists());
}

fn latest_checkpoint_id(save: &Path) -> String {
    let dir = save.join("checkpoints");
    let mut ids: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    ids.pop()
        .unwrap_or_else(|| panic!("no checkpoint in {}", dir.display()))
}

fn downgrade_schema(save: &Path, from: &str, to: &str) {
    let path = save.join("save-meta.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let updated = text.replace(&format!("schema: {from}"), &format!("schema: {to}"));
    assert_ne!(
        updated, text,
        "save-meta.yaml had no `schema: {from}`:\n{text}"
    );
    fs::write(&path, updated).unwrap();
}

fn git_head() -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn t6a_scope() -> String {
    format!(
        "T6a evidence scope: synthetic held SAVE lock (not a live D commit); platform={} {}; head={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        git_head()
    )
}

fn wait_child_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
    label: &str,
) -> Output {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_end(&mut stdout);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_end(&mut stderr);
                }
                return Output {
                    status,
                    stdout,
                    stderr,
                };
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "{label} hung (child still running after {timeout:?}); {}",
                    t6a_scope()
                );
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(err) => panic!("{label}: try_wait failed: {err}"),
        }
    }
}

fn run_slot_timed(root: &Path, args: &[&str], timeout: Duration, label: &str) -> Output {
    let mut full = vec!["--root".to_string(), root.display().to_string()];
    full.push("--save".into());
    full.push(SLOT.into());
    full.extend(args.iter().map(|s| (*s).to_string()));
    let child = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .args(&full)
        .env("TALKDND_NO_PDF2MD", "1")
        .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn talkdnd");
    wait_child_with_timeout(child, timeout, label)
}

#[test]
fn group_c_writers_succeed_under_an_initialized_protocol() {
    let root = playable("p1c-ready");
    let save = root.join(SAVE);
    init_protocol(&root, &save);

    let snap = ok_slot(&root, &["checkpoint", "snapshot"]);
    assert!(
        !snap.contains("尚未接入成长协议") && !snap.contains("无法取得 SAVE 锁"),
        "snapshot must be adopted\n{snap}"
    );
    let id = latest_checkpoint_id(&save);
    assert!(
        save.join("checkpoints").join(&id).is_dir(),
        "snapshot must create checkpoints/{id}"
    );

    let listed = ok_slot(&root, &["checkpoint", "list"]);
    assert!(
        listed.contains(&id) || !listed.is_empty(),
        "list:\n{listed}"
    );

    downgrade_schema(&save, "10", "8");
    let migrated = ok_slot(&root, &["migrate"]);
    assert!(
        migrated.contains("schema") || migrated.contains("迁移") || migrated.contains("无需迁移"),
        "migrate:\n{migrated}"
    );
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    assert!(
        meta.contains("schema: 10"),
        "migrate must write the current schema:\n{meta}"
    );

    let restored = ok_slot(&root, &["checkpoint", "restore", &id, "--yes"]);
    assert!(
        !restored.contains("尚未接入成长协议"),
        "restore must be adopted\n{restored}"
    );
}

#[test]
fn checkpoint_list_show_and_migrate_dry_run_are_tree_identical_under_the_protocol() {
    let root = playable("p1c-ro");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    ok_slot(&root, &["checkpoint", "snapshot"]);
    let id = latest_checkpoint_id(&save);
    downgrade_schema(&save, "10", "8");
    let before = snapshot(&save);
    let protocol_lock = fs::read(save.join(adv::LOCK_FILE)).unwrap();
    let protocol_state = fs::read(save.join(adv::STATE_FILE)).unwrap();

    let list = ok_slot(&root, &["checkpoint", "list"]);
    assert!(
        !list.contains("尚未接入成长协议") && !list.contains("无法取得 SAVE 锁"),
        "list must stay available\n{list}"
    );
    assert_eq!(
        snapshot(&save),
        before,
        "checkpoint list changed the save tree"
    );

    let show = ok_slot(&root, &["checkpoint", "show", &id]);
    assert!(
        !show.contains("尚未接入成长协议") && !show.contains("无法取得 SAVE 锁"),
        "show must stay available\n{show}"
    );
    assert_eq!(
        snapshot(&save),
        before,
        "checkpoint show changed the save tree"
    );

    let dry = ok_slot(&root, &["migrate", "--dry-run"]);
    assert!(
        !dry.contains("尚未接入成长协议") && !dry.contains("无法取得 SAVE 锁"),
        "migrate --dry-run must stay available\n{dry}"
    );
    assert_eq!(
        snapshot(&save),
        before,
        "migrate --dry-run changed the save tree"
    );
    assert_eq!(
        fs::read(save.join(adv::LOCK_FILE)).unwrap(),
        protocol_lock,
        "read-only commands must not rewrite the SAVE lock"
    );
    assert_eq!(
        fs::read(save.join(adv::STATE_FILE)).unwrap(),
        protocol_state,
        "read-only commands must not rewrite SAVE state or auto-init"
    );
}

#[test]
fn group_c_writers_without_the_protocol_keep_working_and_do_not_init() {
    let root = playable("p1c-noproto");
    let save = root.join(SAVE);
    assert!(!save.join(adv::LOCK_FILE).exists());
    assert!(!save.join(adv::STATE_FILE).exists());
    ok_slot(&root, &["checkpoint", "snapshot"]);
    let id = latest_checkpoint_id(&save);
    downgrade_schema(&save, "10", "8");
    let dry_before = snapshot(&save);
    ok_slot(&root, &["migrate", "--dry-run"]);
    assert_eq!(
        snapshot(&save),
        dry_before,
        "migrate --dry-run without the protocol must not write"
    );
    ok_slot(&root, &["migrate"]);
    ok_slot(&root, &["checkpoint", "restore", &id, "--yes"]);
    assert!(!save.join(adv::LOCK_FILE).exists());
    assert!(!save.join(adv::STATE_FILE).exists());
}

#[test]
fn t6a_checkpoint_snapshot_and_restore_time_out_against_a_held_save_lock() {
    let root = playable("p1c-t6a");
    let save = root.join(SAVE);
    init_protocol(&root, &save);
    ok_slot(&root, &["checkpoint", "snapshot"]);
    let id = latest_checkpoint_id(&save);
    let before = snapshot(&save);
    let held = adv::open(&root, &save, &adv::OpenOptions::new()).unwrap();
    let scope = t6a_scope();
    let timeout = adv::DEFAULT_ACQUIRE_TIMEOUT + Duration::from_secs(7);

    let snap = run_slot_timed(
        &root,
        &["checkpoint", "snapshot"],
        timeout,
        "T6a checkpoint snapshot",
    );
    let snap_text = output_text(&snap);
    assert_eq!(
        snap.status.code(),
        Some(1),
        "T6a snapshot must fail closed, not hang\n{scope}\n{snap_text}"
    );
    assert!(
        snap_text.contains("未写入任何目标文件") && snap_text.contains("占用"),
        "T6a snapshot: {scope}\n{snap_text}"
    );

    let restore = run_slot_timed(
        &root,
        &["checkpoint", "restore", &id, "--yes"],
        timeout,
        "T6a checkpoint restore",
    );
    let restore_text = output_text(&restore);
    assert_eq!(
        restore.status.code(),
        Some(1),
        "T6a restore must fail closed, not hang\n{scope}\n{restore_text}"
    );
    assert!(
        restore_text.contains("未写入任何目标文件") && restore_text.contains("占用"),
        "T6a restore: {scope}\n{restore_text}"
    );

    drop(held);
    assert_eq!(
        snapshot(&save),
        before,
        "T6a waiter wrote the save; {scope}"
    );
    eprintln!("T6a snapshot: timeout/fail-closed; T6a restore: timeout/fail-closed; {scope}");
}
