//! P03 DM style settings (TASK-0061): `settings style show|set`, the
//! mandatory `--reason`, session-log lines, and rejection of illegal values
//! without touching the save (LetsPlay/docs/campaign-continuity.md §5.2).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("talkdnd-style-{name}-{}", std::process::id()));
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
fn style_show_defaults_and_set_writes_meta_and_log() {
    let (r, save) = new_save("show-set");

    // Defaults after `new` (schema 8): standard/standard/standard/distinct/2-4.
    let out = ok(&r, &["settings", "style", "show"]);
    for want in [
        "narration：standard",
        "detail：standard",
        "pace：standard",
        "npc_voice：distinct",
        "options：2-4",
    ] {
        assert!(out.contains(want), "missing {want}: {out}");
    }
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    assert!(meta.contains("schema: 8"), "{meta}");

    // A set writes the field, keeps the others, and logs the reason.
    let out = ok(
        &r,
        &[
            "settings",
            "style",
            "set",
            "narration",
            "short",
            "--reason",
            "table vote",
        ],
    );
    assert!(
        out.contains("standard → short") && out.contains("table vote"),
        "{out}"
    );
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    assert!(meta.contains("narration: short"), "{meta}");
    assert!(meta.contains("npc_voice: distinct"), "{meta}");
    let log = fs::read_to_string(save.join("Sessions/2030-01-02-session-00-setup.md")).unwrap();
    assert!(log.contains("叙事风格 narration"), "{log}");
    assert!(log.contains("table vote"), "{log}");

    // show reflects the change; the other fields are untouched.
    let out = ok(&r, &["settings", "style", "show"]);
    assert!(out.contains("narration：short"), "{out}");

    // The dice setting and bare `settings` behave exactly as before.
    let out = ok(&r, &["settings"]);
    assert!(out.contains("骰子显示：开"), "{out}");
    let out = ok(&r, &["settings", "dice", "off"]);
    assert!(out.contains("骰子显示：关"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn style_set_rejects_illegal_values_and_missing_reason() {
    let (r, save) = new_save("reject");
    let before = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    let log_before =
        fs::read_to_string(save.join("Sessions/2030-01-02-session-00-setup.md")).unwrap();

    // Unknown field, unknown value, empty reason: all refused, file unchanged.
    let (good, out) = run(
        &r,
        &["settings", "style", "set", "tone", "warm", "--reason", "r"],
    );
    assert!(!good && out.contains("风格字段只能是"), "{out}");
    let (good, out) = run(
        &r,
        &[
            "settings",
            "style",
            "set",
            "narration",
            "epic",
            "--reason",
            "r",
        ],
    );
    assert!(!good && out.contains("short/standard/long"), "{out}");
    let (good, out) = run(
        &r,
        &[
            "settings", "style", "set", "options", "none", "--reason", "  ",
        ],
    );
    assert!(!good && out.contains("--reason"), "{out}");
    assert_eq!(
        fs::read_to_string(save.join("save-meta.yaml")).unwrap(),
        before
    );

    // --reason is required by the CLI (usage error, exit 2).
    let (good, out) = run(&r, &["settings", "style", "set", "pace", "fast"]);
    assert!(!good, "{out}");
    assert!(out.contains("--reason"), "{out}");
    assert_eq!(
        fs::read_to_string(save.join("save-meta.yaml")).unwrap(),
        before
    );
    let log = fs::read_to_string(save.join("Sessions/2030-01-02-session-00-setup.md")).unwrap();
    assert_eq!(log, log_before, "no log line without a real change");

    // Legal value of a different field still works afterwards.
    ok(
        &r,
        &[
            "settings",
            "style",
            "set",
            "pace",
            "fast",
            "--reason",
            "players asked",
        ],
    );
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    assert!(meta.contains("pace: fast"), "{meta}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn an_old_save_migrates_on_first_style_use_and_starts() {
    let (r, save) = new_save("old-save");
    // Downgrade the save to schema 7 (pre-style) with an old tool stamp.
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    fs::write(
        save.join("save-meta.yaml"),
        meta.replace("schema: 8", "schema: 7"),
    )
    .unwrap();
    let out = ok(&r, &["settings", "style", "show"]);
    assert!(out.contains("narration：standard"), "{out}");
    // show is read-only: the save stays at 7 until a write or start migrates it.
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    assert!(meta.contains("schema: 7"), "{meta}");

    // First write migrates with the usual whole-save backup, then applies.
    let out = ok(
        &r,
        &[
            "settings",
            "style",
            "set",
            "detail",
            "high",
            "--reason",
            "player asked for more",
        ],
    );
    assert!(out.contains("standard → high"), "{out}");
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    assert!(
        meta.contains("schema: 8") && meta.contains("detail: high"),
        "{meta}"
    );
    assert!(meta.contains("from: 7"), "migration recorded: {meta}");
    let backups: Vec<PathBuf> = fs::read_dir(r.join("Saves/_backups"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    // The backup still holds the old schema (the documented restore path).
    let backup_meta = fs::read_to_string(backups[0].join("save-meta.yaml")).unwrap();
    assert!(backup_meta.contains("schema: 7"), "{backup_meta}");

    // And `start` on the migrated save is clean.
    let out = ok(&r, &["start"]);
    assert!(
        out.contains("schema 8") || out.contains("无需迁移"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&r);
}
