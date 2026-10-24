//! P03 long-term memory (TASK-0061): `memory add|list|remove` and the
//! player-view filter in `known`/`start` (LetsPlay/docs/campaign-continuity.md
//! §4.2, §6.1).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("talkdnd-mem-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(p.join("Saves")).unwrap();
    fs::write(p.join("AGENTS.md"), "x").unwrap();
    p
}

fn run(root: &Path, args: &[&str]) -> (bool, String) {
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

fn ok(root: &Path, args: &[&str]) -> String {
    let (good, s) = run(root, args);
    assert!(good, "{args:?} failed: {s}");
    s
}

fn new_save(name: &str) -> (PathBuf, PathBuf) {
    let r = root(name);
    ok(&r, &["new", "--date", "2030-01-02"]);
    let save = r.join("Saves/2030-01-02-01");
    (r, save)
}

#[test]
fn memory_crud_duplicates_and_empty_text() {
    let (r, save) = new_save("crud");
    let out = ok(
        &r,
        &[
            "memory", "add", "the", "bard", "owes", "the", "guard", "a", "favor",
        ],
    );
    assert!(
        out.contains("M-001") && out.contains("the bard owes the guard a favor"),
        "{out}"
    );
    assert!(out.contains("player-confirmed"), "{out}");
    let out = ok(
        &r,
        &[
            "memory",
            "add",
            "guard captain is secret",
            "--source",
            "manual",
        ],
    );
    assert!(out.contains("M-002") && out.contains("manual"), "{out}");
    let out = ok(
        &r,
        &[
            "memory",
            "add",
            "brought over from the old campaign",
            "--source",
            "import",
            "--tag",
            "npc:guard",
        ],
    );
    assert!(out.contains("M-003"), "{out}");

    // The ledger is version 1 with sequential, unique ids.
    let yaml = fs::read_to_string(save.join("memory.yaml")).unwrap();
    assert!(yaml.contains("version: 1"), "{yaml}");
    assert!(yaml.contains("M-001") && yaml.contains("M-002") && yaml.contains("M-003"));

    // Empty text and unknown sources are rejected, file untouched.
    let before = fs::read_to_string(save.join("memory.yaml")).unwrap();
    let (good, out) = run(&r, &["memory", "add"]);
    assert!(!good && out.contains("记忆文本为空"), "{out}");
    let (good, out) = run(&r, &["memory", "add", "   "]);
    assert!(!good && out.contains("记忆文本为空"), "{out}");
    let (good, out) = run(&r, &["memory", "add", "x", "--source", "diary"]);
    assert!(
        !good && out.contains("player-confirmed、import、manual"),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(save.join("memory.yaml")).unwrap(),
        before
    );

    // Default list: player-confirmed only; --all is the DM view with sources.
    let out = ok(&r, &["memory", "list"]);
    assert!(
        out.contains("M-001") && out.contains("the bard owes"),
        "{out}"
    );
    assert!(!out.contains("secret"), "{out}");
    let out = ok(&r, &["memory", "list", "--all"]);
    for want in [
        "M-001",
        "M-002",
        "M-003",
        "player-confirmed",
        "manual",
        "import",
    ] {
        assert!(out.contains(want), "missing {want}: {out}");
    }
    let out = ok(&r, &["memory", "list", "--all", "--tag", "npc:guard"]);
    assert!(out.contains("M-003"), "{out}");
    assert!(!out.contains("M-001"), "{out}");
    let out = ok(&r, &["memory", "list", "--all", "--tag", "nope"]);
    assert!(out.contains("0 条"), "{out}");

    // Remove works; removing an unknown id is an error.
    let out = ok(&r, &["memory", "remove", "M-002"]);
    assert!(out.contains("已删除"), "{out}");
    let yaml = fs::read_to_string(save.join("memory.yaml")).unwrap();
    assert!(!yaml.contains("secret"), "{yaml}");
    let (good, out) = run(&r, &["memory", "remove", "M-009"]);
    assert!(!good && out.contains("M-009 不存在"), "{out}");

    // Ids are never reused: the next add continues after the highest seen.
    let out = ok(&r, &["memory", "add", "another fact"]);
    assert!(out.contains("M-004"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn player_view_shows_confirmed_memory_only_and_never_secrets() {
    let (r, save) = new_save("player-view");
    ok(
        &r,
        &[
            "memory", "add", "The", "bard", "owes", "the", "guard", "a", "favor",
        ],
    );
    ok(
        &r,
        &[
            "memory", "add", "the", "guard", "secretly", "serves", "the", "duke", "--source",
            "manual",
        ],
    );
    ok(
        &r,
        &["memory", "add", "imported whisper", "--source", "import"],
    );
    fs::write(save.join("dm-secrets.md"), "- term: The Silent Duke\n").unwrap();

    let out = ok(&r, &["start"]);
    assert!(out.contains("## 关键记忆"), "{out}");
    assert!(
        out.contains("M-001：The bard owes the guard a favor"),
        "{out}"
    );
    assert!(!out.contains("secretly serves"), "{out}");
    assert!(!out.contains("imported whisper"), "{out}");
    assert!(!out.contains("Silent Duke"), "{out}");

    // The digest written to disk agrees with the printed one.
    let digest = fs::read_to_string(save.join("digest.md")).unwrap();
    assert!(
        digest.contains("M-001：The bard owes the guard a favor"),
        "{digest}"
    );
    assert!(!digest.contains("secretly serves"), "{digest}");

    // `known` is a player view too and stays free of DM-only memory.
    let out = ok(&r, &["known"]);
    assert!(
        !out.contains("secretly serves") && !out.contains("imported whisper"),
        "{out}"
    );

    // Removing the confirmed entry empties the digest section.
    ok(&r, &["memory", "remove", "M-001"]);
    let out = ok(&r, &["start"]);
    assert!(out.contains("还没有"), "{out}");
    assert!(!out.contains("M-001"), "{out}");
    let _ = fs::remove_dir_all(&r);
}
