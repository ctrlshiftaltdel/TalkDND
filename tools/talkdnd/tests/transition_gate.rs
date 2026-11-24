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

fn latest_session_log(save: &Path) -> PathBuf {
    let mut logs: Vec<PathBuf> = fs::read_dir(save.join("Sessions"))
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "md")
                && !path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("README.md"))
        })
        .collect();
    logs.sort();
    logs.pop().expect("playable save has a session log")
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
    for &args in WRITE_HELP {
        let text = ok(&root, args);
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

/// Every write leaf of `talkdnd`, as a `--help` path. Kept explicit so a new
/// write command has to add itself here.
const WRITE_HELP: &[&[&str]] = &[
    &["start", "--help"],
    &["migrate", "--help"],
    &["level-up", "--help"],
    &["xp", "add", "--help"],
    &["loot", "add", "--help"],
    &["loot", "spend", "--help"],
    &["loot", "give", "--help"],
    &["loot", "split", "--help"],
    &["scene", "--help"],
    &["char", "--help"],
    &["char", "aria", "show", "--help"],
    &["char", "aria", "init", "--help"],
    &["char", "aria", "damage", "--help"],
    &["char", "aria", "heal", "--help"],
    &["char", "aria", "temp", "--help"],
    &["char", "aria", "use", "--help"],
    &["char", "aria", "restore", "--help"],
    &["char", "aria", "rest", "--help"],
    &["char", "aria", "condition", "--help"],
    &["char", "aria", "exhaustion", "--help"],
    &["char", "aria", "inspiration", "--help"],
    &["char", "aria", "death-save", "--help"],
    &["char", "aria", "gold", "--help"],
    &["char", "aria", "item", "--help"],
    &["char", "aria", "ammo", "--help"],
    &["char", "aria", "note", "--help"],
    &["char", "aria", "dm-note", "--help"],
    &["char", "aria", "set", "--help"],
    &["char", "aria", "batch", "--help"],
    &["char", "aria", "spells", "--help"],
    &["char", "aria", "spells", "adopt", "--help"],
    &["char", "aria", "spells", "sync", "--help"],
    &["char", "aria", "spells", "terrain", "--help"],
    &["char", "aria", "prepare", "--help"],
    &["char", "aria", "unprepare", "--help"],
    &["char", "aria", "learn", "--help"],
    &["char", "aria", "scribe", "--help"],
    &["char", "aria", "ritual", "--help"],
    &["chargen", "roll", "--help"],
    &["chargen", "array", "--help"],
    &["chargen", "pointbuy", "--help"],
    &["chargen", "assign", "--help"],
    &["chargen", "asi", "--help"],
    &["chargen", "set", "--help"],
    &["chargen", "sheet", "--help"],
    &["chargen", "confirm", "--help"],
    &["companion", "random", "--help"],
    &["companion", "sheet", "--help"],
    &["npc", "random", "--help"],
    &["npc", "sheet", "--help"],
    &["check", "--help"],
    &["integrity", "status", "--help"],
    &["integrity", "accept", "--help"],
    &["checkpoint", "--help"],
    &["checkpoint", "snapshot", "--help"],
    &["checkpoint", "restore", "--help"],
    &["log", "--help"],
    &["memory", "add", "--help"],
    &["memory", "remove", "--help"],
    &["entity", "add", "--help"],
    &["entity", "rename", "--help"],
    &["entity", "retire", "--help"],
    &["relation", "add", "--help"],
    &["relation", "end", "--help"],
    &["time", "enable", "--help"],
    &["time", "disable", "--help"],
    &["time", "set", "--help"],
    &["time", "advance", "--help"],
    &["time", "event", "add", "--help"],
    &["time", "event", "done", "--help"],
    &["time", "event", "skip", "--help"],
    &["settings", "dice", "--help"],
    &["settings", "style", "set", "--help"],
    &["settings", "automation", "set", "--help"],
    &["settings", "automation", "reset", "--help"],
    &["secret", "add", "--help"],
    &["secret", "remove", "--help"],
    &["secret", "import", "--help"],
    &["identify", "--help"],
    &["reveal", "add", "--help"],
    &["original", "add", "--help"],
    &["original", "plan", "--help"],
    &["handout", "show", "--help"],
    &["handout", "build", "--help"],
    &["handout", "mark", "--help"],
    &["handout", "list", "--dm", "--help"],
    &["adventure", "section", "--help"],
    &["adventure", "index", "build", "--help"],
];

/// A book folder inside another, adopted save must be refused even when the
/// selected save is uninitialised.
#[test]
fn a_book_folder_in_another_save_is_refused() {
    let root = playable("book-other-save");
    let selected = root.join(SAVE);
    assert!(!selected.join(LOCK).exists());
    let other = root.join("Saves/2030-05-05-01");
    let book = other.join("Adventures/probe/index");
    fs::create_dir_all(&book).unwrap();
    fs::write(book.join("index.json"), b"{}\n").unwrap();
    fs::write(book.join("Probe.pages.md"), b"# Chapter 1\n").unwrap();
    fs::write(other.join(LOCK), b"probe lock\n").unwrap();
    let before = snapshot(&other);
    let absolute = book.to_string_lossy().into_owned();
    let relative = "Saves/2030-05-05-01/Adventures/probe/index".to_string();
    for args in [
        vec!["adventure", "index", "build", "--book", &absolute],
        vec!["adventure", "index", "build", "--book", &relative],
        vec!["handout", "--book", &absolute, "build"],
        vec![
            "handout", "--book", &absolute, "mark", "p0001-00", "--player",
        ],
    ] {
        let out = run_slot(&root, &args);
        let text = output_text(&out);
        assert_eq!(
            out.status.code(),
            Some(1),
            "`{}` must exit 1\n{text}",
            args.join(" ")
        );
        assert!(
            text.contains("已启用成长协议"),
            "`{}` must name the protocol\n{text}",
            args.join(" ")
        );
        assert_eq!(
            snapshot(&other),
            before,
            "`{}` changed the other save",
            args.join(" ")
        );
    }
}

/// A book folder outside every save keeps its old behaviour: no protocol, no
/// refusal.
#[test]
fn a_book_folder_outside_the_saves_is_not_refused() {
    let root = playable("book-outside");
    let book = root.join("Adventures/probe/index");
    fs::create_dir_all(&book).unwrap();
    fs::write(book.join("index.json"), b"{}\n").unwrap();
    fs::write(book.join("Probe.pages.md"), b"# Chapter 1\n").unwrap();
    let absolute = book.to_string_lossy().into_owned();
    for args in [
        vec!["adventure", "index", "build", "--book", &absolute],
        vec!["handout", "--book", &absolute, "build"],
    ] {
        let out = run_slot(&root, &args);
        let text = output_text(&out);
        assert!(
            !text.contains("尚未接入成长协议"),
            "`{}` must not be refused outside a save\n{text}",
            args.join(" ")
        );
    }
}

/// `--list` prints headings and returns before the log write, so it stays
/// read-only even with `--reason`.
#[test]
fn adventure_section_list_with_reason_is_read_only() {
    let root = playable("section-list");
    let save = root.join(SAVE);
    let book = root.join("Adventures/gate-probe/index");
    fs::create_dir_all(&book).unwrap();
    fs::write(
        book.join("Gate Probe.pages.md"),
        b"<a id=\"p1\"></a>\n# Chapter 1: Probe\n\nSome text.\n\n## Area A\n\nMore text.\n",
    )
    .unwrap();
    adopt_protocol(&save);
    let before = snapshot(&save);
    let text = ok_slot(
        &root,
        &["adventure", "section", "--list", "--reason", "gate probe"],
    );
    assert!(text.contains("Chapter 1"), "{text}");
    assert_eq!(snapshot(&save), before, "`--list --reason` wrote the save");
}

/// `handout list --dm` builds and writes a missing book registry, so it is a
/// writer; the player view stays read-only.
#[test]
fn handout_list_dm_is_a_writer_but_the_player_view_is_not() {
    let root = playable("handout-list-dm");
    let save = root.join(SAVE);
    adopt_protocol(&save);
    let before = snapshot(&save);
    let out = run_slot(&root, &["handout", "list", "--dm"]);
    let text = output_text(&out);
    assert_eq!(out.status.code(), Some(1), "must exit 1\n{text}");
    assert!(text.contains("已启用成长协议"), "{text}");
    assert_eq!(snapshot(&save), before);
    let player = run_slot(&root, &["handout", "list"]);
    assert!(
        player.status.success(),
        "the player view must stay available\n{}",
        output_text(&player)
    );
    assert!(!output_text(&player).contains("尚未接入成长协议"));
    assert_eq!(snapshot(&save), before);
}

/// A `--book` folder in another, adopted save is refused for `handout show`
/// and `handout list --dm` too, even when the registry there does not exist
/// yet (both would build and write it).
#[test]
fn handout_show_and_dm_list_check_the_book_folder_of_another_save() {
    let root = playable("handout-book-other-save");
    let selected = root.join(SAVE);
    assert!(!selected.join(LOCK).exists());
    let other = root.join("Saves/2030-05-05-01");
    let book = other.join("Adventures/probe/index");
    fs::create_dir_all(&book).unwrap();
    fs::write(book.join("index.json"), b"{\"headings\":[]}\n").unwrap();
    fs::write(other.join(LOCK), b"probe lock\n").unwrap();
    assert!(!book.join("handouts.yaml").exists());
    let before = snapshot(&other);
    let absolute = book.to_string_lossy().into_owned();
    for args in [
        vec!["handout", "--book", &absolute, "show", "probe"],
        vec!["handout", "--book", &absolute, "list", "--dm"],
    ] {
        let out = run_slot(&root, &args);
        let text = output_text(&out);
        assert_eq!(
            out.status.code(),
            Some(1),
            "`{}` must exit 1\n{text}",
            args.join(" ")
        );
        assert!(
            text.contains("已启用成长协议"),
            "`{}` must name the protocol\n{text}",
            args.join(" ")
        );
        assert_eq!(
            snapshot(&other),
            before,
            "`{}` wrote the other save",
            args.join(" ")
        );
    }
}

/// `new` audits the save it created, never a later-dated save that happens to
/// be the newest slot.
#[test]
fn new_audits_only_the_save_it_created() {
    for with_integrity in [false, true] {
        let root = playable(if with_integrity {
            "new-audit-with-integrity"
        } else {
            "new-audit-without-integrity"
        });
        let old = root.join("Saves/2030-12-31-01");
        fs::create_dir_all(old.join("Sessions")).unwrap();
        fs::write(old.join("canon.md"), b"# old save\n").unwrap();
        fs::write(
            old.join("Sessions/2030-12-31-session-00-setup.md"),
            b"# setup\n",
        )
        .unwrap();
        if with_integrity {
            fs::write(old.join("integrity.yaml"), b"version: 1\nfiles: {}\n").unwrap();
        }
        fs::write(old.join(LOCK), b"probe lock\n").unwrap();
        let before = snapshot(&old);
        let text = ok(&root, &["new", "--date", "2030-01-03"]);
        assert!(text.contains("created Saves/2030-01-03-01"), "{text}");
        assert_eq!(
            snapshot(&old),
            before,
            "`new` audited or touched the later-dated save"
        );
        assert!(
            root.join("Saves/2030-01-03-01/integrity.yaml").is_file(),
            "the created save was not audited"
        );
    }
}

/// The latest session log is a concrete write target: a symlink into a
/// protected save is refused before `talkdnd log` follows it.
#[cfg(unix)]
#[test]
fn a_session_log_symlink_into_a_protected_save_is_refused() {
    let root = playable("session-log-link");
    let selected = root.join(SAVE);
    assert!(!selected.join(LOCK).exists());
    let other = root.join("Saves/2030-05-05-01");
    fs::create_dir_all(other.join("Sessions")).unwrap();
    fs::write(other.join(LOCK), b"probe lock\n").unwrap();
    let protected_file = other.join("Sessions/protected.md");
    fs::write(&protected_file, b"# protected\n").unwrap();
    let latest = latest_session_log(&selected);
    fs::remove_file(&latest).unwrap();
    std::os::unix::fs::symlink(&protected_file, &latest).unwrap();
    let before = snapshot(&other);
    let out = run_slot(&root, &["log", "gate-probe"]);
    let text = output_text(&out);
    assert_eq!(out.status.code(), Some(1), "must exit 1\n{text}");
    assert!(text.contains("已启用成长协议"), "{text}");
    assert_eq!(snapshot(&other), before, "the protected save changed");
    assert!(
        fs::symlink_metadata(&latest)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the session log symlink was replaced"
    );
}

/// `<save>/rolls.log` is a concrete write target: a symlink into a protected
/// save is refused before a rolls-appending command follows it.
#[cfg(unix)]
#[test]
fn a_rolls_log_symlink_into_a_protected_save_is_refused() {
    let root = playable("rolls-log-link");
    let selected = root.join(SAVE);
    assert!(!selected.join(LOCK).exists());
    let other = root.join("Saves/2030-05-05-01");
    fs::create_dir_all(other.join("Sessions")).unwrap();
    fs::write(other.join(LOCK), b"probe lock\n").unwrap();
    let protected_file = other.join("Sessions/protected.md");
    fs::write(&protected_file, b"# protected\n").unwrap();
    let link = selected.join("rolls.log");
    std::os::unix::fs::symlink(&protected_file, &link).unwrap();
    let before = snapshot(&other);
    let out = run_slot(&root, &["check", "aria", "athletics"]);
    let text = output_text(&out);
    assert_eq!(out.status.code(), Some(1), "must exit 1\n{text}");
    assert!(text.contains("已启用成长协议"), "{text}");
    assert_eq!(snapshot(&other), before, "the protected save changed");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the rolls.log symlink was replaced"
    );
}
