//! ADV-P1-1pre cross-CLI transition gate, exercised on the **real** `talkdnd`
//! call path.
//!
//! Every row of the shared writer registry that belongs to `talkdnd` is run as
//! a real subprocess: unadopted writers must exit 1 with the shared refusal
//! text and leave the save tree byte-identical, while read-only and
//! control-metadata rows must stay available. A save without the control files
//! keeps its old behaviour.

use advancement_lock::{Class, Cli, WRITERS};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SAVE: &str = "Saves/2030-01-02-01";
const SLOT: &str = "2030-01-02-01";
const LOCK: &str = ".talkdnd.advancement.lock";
const STATE: &str = ".talkdnd.advancement.state";

fn play_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "talkdnd-gate-{tag}-{}-{}",
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
/// the real CLI so every read-only row has something to read.
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
    root
}

fn adopt_protocol(save: &Path) {
    fs::write(save.join(LOCK), b"probe lock\n").unwrap();
    fs::write(save.join(STATE), b"probe state\n").unwrap();
    // The import rows name this file; it is part of the synthetic save so the
    // tree comparison covers it too.
    fs::write(save.join("gate-probe-terms.txt"), b"probe-term\n").unwrap();
}

/// Every directory and file below `dir`, with file bytes.
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

/// Read-only rows that legitimately fail on a save without the referenced book
/// content or caster. The gate must not be the reason; the tree must stay
/// identical either way.
const CONTENT_DEPENDENT: &[&str] = &[
    "talkdnd:snapshot-diff",
    "talkdnd:adventure-search",
    "talkdnd:adventure-section-read",
    "talkdnd:adventure-index-read",
    "talkdnd:char-spells-read",
    // Writes a rules data file outside the save and needs converted books.
    "talkdnd:data-import",
];

#[test]
fn unadopted_writers_exit_one_and_leave_the_save_untouched() {
    let root = playable("refuse");
    let save = root.join(SAVE);
    adopt_protocol(&save);
    let before = snapshot(&save);
    let rows: Vec<_> = WRITERS
        .iter()
        .filter(|row| row.cli == Cli::Talkdnd && row.class == Class::UnadoptedWrite)
        .collect();
    assert!(rows.len() > 30, "the talkdnd registry looks too small");
    for row in rows {
        let args = argv(row, &save);
        let out = run_slot(&root, &argv_refs(&args));
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
    let root = playable("readonly");
    let save = root.join(SAVE);
    adopt_protocol(&save);
    let before = snapshot(&save);
    let rows: Vec<_> = WRITERS
        .iter()
        .filter(|row| row.cli == Cli::Talkdnd && row.class == Class::ReadOnly)
        .collect();
    assert!(rows.len() > 20, "the talkdnd registry looks too small");
    for row in rows {
        let args = argv(row, &save);
        let out = run_slot(&root, &argv_refs(&args));
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
fn cache_writes_are_control_metadata_only() {
    let root = playable("cache");
    let save = root.join(SAVE);
    adopt_protocol(&save);
    for row in WRITERS
        .iter()
        .filter(|row| row.cli == Cli::Talkdnd && row.class == Class::ControlMetadata)
    {
        let before = snapshot(&save);
        let args = argv(row, &save);
        let out = run_slot(&root, &argv_refs(&args));
        let text = output_text(&out);
        assert!(
            out.status.success(),
            "{} must stay available under the protocol\n{text}",
            row.entry
        );
        assert!(
            !text.contains("尚未接入成长协议"),
            "{} must not be refused\n{text}",
            row.entry
        );
        let after = snapshot(&save);
        for (path, bytes) in &after {
            if path.ends_with("secret-terms.cache.json") {
                continue;
            }
            assert_eq!(
                before.get(path),
                Some(bytes),
                "{} changed {} beyond the control-metadata cache",
                row.entry,
                path.display()
            );
        }
        for path in before.keys() {
            assert!(
                after.contains_key(path),
                "{} removed {}",
                row.entry,
                path.display()
            );
        }
    }
}

#[test]
fn without_the_protocol_existing_writes_keep_working() {
    let root = playable("noprotocol");
    let save = root.join(SAVE);
    assert!(!save.join(LOCK).exists());
    assert!(!save.join(STATE).exists());
    // The same entries that are refused under the protocol work as before.
    ok_slot(&root, &["xp", "add", "10", "--reason", "gate-probe"]);
    ok_slot(&root, &["log", "gate probe without the protocol"]);
    ok_slot(&root, &["scene", "gate-probe-scene"]);
    ok_slot(&root, &["guard", "gate probe text"]);
    ok_slot(&root, &["char", "aria", "damage", "1"]);
    // The gate never creates a control file on its own.
    assert!(!save.join(LOCK).exists());
    assert!(!save.join(STATE).exists());
}

#[test]
fn a_new_save_is_still_allowed_under_the_protocol() {
    let root = playable("newsave");
    let save = root.join(SAVE);
    adopt_protocol(&save);
    let before = snapshot(&save);
    let text = ok(&root, &["new", "--date", "2031-01-01"]);
    assert!(text.contains("created Saves/2031-01-01-01"), "{text}");
    assert!(root.join("Saves/2031-01-01-01/chargen.json").is_file());
    assert_eq!(snapshot(&save), before, "`new` touched an adopted save");
}

#[test]
fn a_read_only_run_does_not_rewrite_the_audit_baseline() {
    let root = playable("baseline");
    let save = root.join(SAVE);
    adopt_protocol(&save);
    let baseline = fs::read(save.join("integrity.yaml")).unwrap();
    // `xp show` is audited (`tracked`) without the protocol; under it the gate
    // must suppress the post-command audit write.
    ok_slot(&root, &["xp", "show"]);
    assert_eq!(
        fs::read(save.join("integrity.yaml")).unwrap(),
        baseline,
        "a read-only command rewrote integrity.yaml"
    );
}

#[test]
fn help_declares_the_refusal_and_its_exit_code() {
    let root = play_root("help");
    let top = ok(&root, &["--help"]);
    assert!(top.contains("成长协议"), "{top}");
    assert!(top.contains("退出码 1"), "{top}");
    for args in [
        vec!["xp", "--help"],
        vec!["xp", "add", "--help"],
        vec!["char", "--help"],
        vec!["chargen", "sheet", "--help"],
        vec!["checkpoint", "--help"],
        vec!["secret", "--help"],
        vec!["secret", "import", "--help"],
    ] {
        let text = ok(&root, &args);
        assert!(
            text.contains("退出码 1"),
            "`{}` help must declare the refusal\n{text}",
            args.join(" ")
        );
    }
    // A control-metadata command is not refused, so its help does not promise
    // a refusal.
    let guard = ok(&root, &["guard", "--help"]);
    assert!(!guard.contains("退出码 1"), "{guard}");
}
