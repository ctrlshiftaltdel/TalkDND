//! TASK-0111 / v1.3 automation switches: save defaults, scene and encounter
//! overrides, the fixed category/level set, schema 8 -> 9 migration, the CLI,
//! the start digest and the session-log audit.
//!
//! Fixtures are synthetic: no real save contents, DM secrets or book text.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("talkdnd-automation-{name}-{}", std::process::id()));
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
    ok(&r, &["new", "--date", "2030-02-01"]);
    let save = r.join("Saves/2030-02-01-01");
    (r, save)
}

fn meta(save: &Path) -> String {
    fs::read_to_string(save.join("save-meta.yaml")).unwrap()
}

fn latest_log(save: &Path) -> String {
    fs::read_to_string(save.join("Sessions/2030-02-01-session-00-setup.md")).unwrap()
}

/// A synthetic adventure with one scene and one encounter, linked to the save.
fn add_adventure(root: &Path, save: &Path) {
    let combat = root.join("Adventures/test-adventure/combat");
    fs::create_dir_all(combat.join("scenes")).unwrap();
    fs::write(
        combat.join("scenes/crypt.yaml"),
        "scenes:\n  - id: crypt-entry\n    name: Crypt Entry\n    mode: theater\n",
    )
    .unwrap();
    fs::write(
        combat.join("encounters.yaml"),
        "encounters:\n  - id: goblin-ambush\n    scene: crypt-entry\n    members:\n      - creature: goblin\n",
    )
    .unwrap();
    let manifest = save.join("combat/campaign.yaml");
    let text = fs::read_to_string(&manifest).unwrap();
    let text = text.replace(
        "adventures: []",
        "adventures: [../../../Adventures/test-adventure/combat]",
    );
    fs::write(manifest, text).unwrap();
}

const CATEGORIES: [&str; 6] = [
    "resolution",
    "combat",
    "npc",
    "player_choice",
    "reaction",
    "near_limit",
];

#[test]
fn defaults_follow_the_charter_and_show_their_source() {
    let (r, save) = new_save("defaults");
    let text = meta(&save);
    assert!(text.contains("schema: 9"), "{text}");
    for want in [
        "automation:",
        "version: 1",
        "resolution: auto",
        "combat: auto",
        "npc: assisted",
        "player_choice: assisted",
        "reaction: assisted",
        "near_limit: assisted",
    ] {
        assert!(text.contains(want), "missing {want} in {text}");
    }
    // `settings.dice` compatibility: the old switch still works on schema 9.
    assert!(text.contains("dice_display: true"), "{text}");

    let out = ok(&r, &["settings", "automation", "show"]);
    for cat in CATEGORIES {
        assert!(out.contains(cat), "missing {cat}: {out}");
    }
    assert!(out.contains("来源：存档默认"), "{out}");
    assert!(out.contains("遭遇 > 场景 > 存档默认"), "{out}");
    assert!(out.contains("场景覆盖：无"), "{out}");
    assert!(out.contains("遭遇覆盖：无"), "{out}");

    // Bare `settings` keeps the dice line and adds the automation summary.
    let out = ok(&r, &["settings"]);
    assert!(out.contains("骰子显示：开"), "{out}");
    assert!(out.contains("自动化") && out.contains("resolution"), "{out}");

    // Every level is a fixed, documented value.
    for level in ["manual", "assisted", "auto"] {
        ok(
            &r,
            &[
                "settings",
                "automation",
                "set",
                "npc",
                level,
                "--reason",
                "level check",
            ],
        );
    }
    let out = ok(&r, &["settings", "dice", "off"]);
    assert!(out.contains("骰子显示：关"), "{out}");
    let out = ok(&r, &["settings"]);
    assert!(out.contains("骰子显示：关") && out.contains("npc"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn precedence_is_encounter_then_scene_then_save_default() {
    let (r, save) = new_save("precedence");
    add_adventure(&r, &save);

    let out = ok(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "npc",
            "manual",
            "--scene",
            "crypt-entry",
            "--reason",
            "players want to speak for the NPCs",
        ],
    );
    assert!(out.contains("场景 crypt-entry") && out.contains("未覆盖 → manual"), "{out}");
    ok(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "combat",
            "assisted",
            "--encounter",
            "goblin-ambush",
            "--reason",
            "ask before monster turns",
        ],
    );
    ok(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "near_limit",
            "auto",
            "--scene",
            "crypt-entry",
            "--reason",
            "warn automatically",
        ],
    );

    // Scene only: encounter level does not apply, scene overrides do.
    let out = ok(
        &r,
        &["settings", "automation", "show", "--scene", "crypt-entry"],
    );
    assert!(out.contains("npc（NPC 对话）：manual — 来源：场景 crypt-entry"), "{out}");
    assert!(out.contains("near_limit（接近上限）：auto — 来源：场景 crypt-entry"), "{out}");
    assert!(out.contains("combat（战斗流程）：auto — 来源：存档默认"), "{out}");
    assert!(out.contains("reaction（玩家反应）：assisted — 来源：存档默认"), "{out}");

    // Encounter + its scene: encounter > scene > save default, per category.
    let out = ok(
        &r,
        &[
            "settings",
            "automation",
            "show",
            "--encounter",
            "goblin-ambush",
        ],
    );
    assert!(out.contains("combat（战斗流程）：assisted — 来源：遭遇 goblin-ambush"), "{out}");
    assert!(out.contains("npc（NPC 对话）：manual — 来源：场景 crypt-entry"), "{out}");
    assert!(out.contains("resolution（结算）：auto — 来源：存档默认"), "{out}");

    // The audit line records old value, new value, scope and reason.
    let log = latest_log(&save);
    assert!(log.contains("自动化 npc（场景 crypt-entry）：未覆盖 → manual"), "{log}");
    assert!(
        log.contains("players want to speak for the NPCs"),
        "{log}"
    );
    assert!(log.contains("自动化 combat（遭遇 goblin-ambush）"), "{log}");
    assert!(log.contains("自动化 near_limit（场景 crypt-entry）"), "{log}");

    // Reset one override: the value falls back to the save default.
    let out = ok(
        &r,
        &[
            "settings",
            "automation",
            "reset",
            "npc",
            "--scene",
            "crypt-entry",
            "--reason",
            "table changed its mind",
        ],
    );
    assert!(out.contains("manual → 未覆盖"), "{out}");
    let out = ok(
        &r,
        &["settings", "automation", "show", "--scene", "crypt-entry"],
    );
    assert!(out.contains("npc（NPC 对话）：assisted — 来源：存档默认"), "{out}");

    // Reset all encounter overrides.
    let out = ok(
        &r,
        &[
            "settings",
            "automation",
            "reset",
            "all",
            "--encounter",
            "goblin-ambush",
            "--reason",
            "back to defaults",
        ],
    );
    assert!(out.contains("遭遇 goblin-ambush"), "{out}");
    let out = ok(
        &r,
        &[
            "settings",
            "automation",
            "show",
            "--encounter",
            "goblin-ambush",
        ],
    );
    assert!(out.contains("combat（战斗流程）：auto — 来源：存档默认"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn the_current_scene_label_is_a_known_scene_id() {
    let (r, save) = new_save("current-scene");
    fs::write(save.join("progress.yaml"), "xp: {}\nscene: tavern\n").unwrap();
    ok(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "npc",
            "manual",
            "--scene",
            "tavern",
            "--reason",
            "players narrate here",
        ],
    );
    let out = ok(&r, &["settings", "automation", "show", "--scene", "tavern"]);
    assert!(out.contains("npc（NPC 对话）：manual — 来源：场景 tavern"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn illegal_values_and_unknown_ids_are_rejected_without_writing() {
    let (r, save) = new_save("reject");
    add_adventure(&r, &save);
    let before = meta(&save);
    let log_before = latest_log(&save);

    // Unknown category.
    let (good, out) = run(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "tone",
            "auto",
            "--reason",
            "r",
        ],
    );
    assert!(!good && out.contains("自动化类别只能是"), "{out}");
    // Unknown level.
    let (good, out) = run(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "npc",
            "sometimes",
            "--reason",
            "r",
        ],
    );
    assert!(!good && out.contains("manual、assisted、auto"), "{out}");
    // Player choices and reactions may never be automated.
    for category in ["player_choice", "reaction"] {
        let (good, out) = run(
            &r,
            &[
                "settings",
                "automation",
                "set",
                category,
                "auto",
                "--reason",
                "r",
            ],
        );
        assert!(
            !good && out.contains("不能设为 auto") && out.contains(category),
            "{out}"
        );
    }
    // Unknown scene / encounter ids.
    let (good, out) = run(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "npc",
            "manual",
            "--scene",
            "no-such-scene",
            "--reason",
            "r",
        ],
    );
    assert!(!good && out.contains("未知场景 id"), "{out}");
    let (good, out) = run(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "npc",
            "manual",
            "--encounter",
            "no-such-encounter",
            "--reason",
            "r",
        ],
    );
    assert!(!good && out.contains("未知遭遇 id"), "{out}");
    // Both scopes at once is ambiguous.
    let (good, out) = run(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "npc",
            "manual",
            "--scene",
            "crypt-entry",
            "--encounter",
            "goblin-ambush",
            "--reason",
            "r",
        ],
    );
    assert!(!good && out.contains("不能同时使用"), "{out}");
    // Empty reason.
    let (good, out) = run(
        &r,
        &[
            "settings", "automation", "set", "npc", "manual", "--reason", "  ",
        ],
    );
    assert!(!good && out.contains("--reason"), "{out}");
    // Missing reason is a usage error.
    let (good, out) = run(&r, &["settings", "automation", "set", "npc", "manual"]);
    assert!(!good && out.contains("--reason"), "{out}");
    // Unknown reset scope value.
    let (good, out) = run(
        &r,
        &[
            "settings",
            "automation",
            "reset",
            "tone",
            "--reason",
            "r",
        ],
    );
    assert!(!good && out.contains("自动化类别只能是"), "{out}");

    assert_eq!(meta(&save), before, "failed commands must not write");
    assert_eq!(latest_log(&save), log_before, "failed commands must not log");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn hand_edited_automation_fails_closed() {
    let (r, save) = new_save("hand-edit");
    let good = meta(&save);

    // Unknown category, level and field are refused when the save loads.
    let bad = good.replace("  npc: assisted", "  tone: assisted");
    fs::write(save.join("save-meta.yaml"), &bad).unwrap();
    let (okay, out) = run(&r, &["settings", "automation", "show"]);
    assert!(!okay && out.contains("自动化类别只能是"), "{out}");

    let bad = good.replace("  npc: assisted", "  npc: sometimes");
    fs::write(save.join("save-meta.yaml"), &bad).unwrap();
    let (okay, out) = run(&r, &["settings", "automation", "show"]);
    assert!(!okay && out.contains("manual、assisted、auto"), "{out}");

    let bad = good.replace("automation:\n", "automation:\n  mystery: 1\n");
    fs::write(save.join("save-meta.yaml"), &bad).unwrap();
    let (okay, out) = run(&r, &["settings", "automation", "show"]);
    assert!(!okay && out.to_lowercase().contains("unknown field"), "{out}");

    // A duplicate scene id in the YAML mapping is refused, not silently last-wins.
    let bad = good.replace(
        "  encounters:",
        "  scenes:\n    crypt-entry:\n      npc: manual\n    crypt-entry:\n      npc: auto\n  encounters:",
    );
    fs::write(save.join("save-meta.yaml"), &bad).unwrap();
    let (okay, out) = run(&r, &["settings", "automation", "show"]);
    assert!(!okay && out.contains("重复"), "{out}");

    // Restoring the original file makes the save usable again.
    fs::write(save.join("save-meta.yaml"), &good).unwrap();
    ok(&r, &["settings", "automation", "show"]);
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn migration_8_to_9_backs_up_logs_and_keeps_old_saves_equivalent() {
    let (r, save) = new_save("migrate");
    let old_meta = "# old save\nschema: 8\ntool: 1.2.0\ndeviation: {scope: 1, ratio: 10}\nsettings:\n  dice_display: false\n  style:\n    narration: short\n    detail: high\n    pace: fast\n    npc_voice: neutral\n    options: none\nmigrations: []\n";
    fs::write(save.join("save-meta.yaml"), old_meta).unwrap();

    // A schema-8 save without automation still behaves like the charter defaults.
    let before = ok(&r, &["settings", "automation", "show"]);
    assert!(before.contains("npc（NPC 对话）：assisted — 来源：存档默认"), "{before}");
    assert!(meta(&save).contains("schema: 8"));

    let dry = ok(&r, &["migrate", "--dry-run"]);
    assert!(dry.contains("dry-run") && dry.contains("8 → 9"), "{dry}");
    assert!(dry.contains("自动化"), "{dry}");
    assert!(meta(&save).contains("schema: 8"));
    assert!(!r.join("Saves/_backups").exists());

    let out = ok(&r, &["migrate"]);
    assert!(out.contains("迁移完成") && out.contains("8 → 9"), "{out}");
    assert!(out.contains("Saves/_backups/"), "{out}");
    let text = meta(&save);
    assert!(text.contains("schema: 9"), "{text}");
    // The old dice switch and style survive unchanged.
    assert!(text.contains("dice_display: false"), "{text}");
    assert!(text.contains("narration: short"), "{text}");
    assert!(text.contains("npc_voice: neutral"), "{text}");
    // The automation defaults arrive explicitly.
    assert!(text.contains("automation:") && text.contains("near_limit: assisted"), "{text}");
    assert!(
        text.contains("from: 8") && text.contains("to: 9"),
        "8->9 migration recorded: {text}"
    );

    // The whole-save backup still says schema 8 and has no automation block:
    // restoring it is the documented rollback.
    let backups: Vec<PathBuf> = fs::read_dir(r.join("Saves/_backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    let backup_meta = fs::read_to_string(backups[0].join("save-meta.yaml")).unwrap();
    assert!(backup_meta.contains("schema: 8"), "{backup_meta}");
    assert!(!backup_meta.contains("automation:"), "{backup_meta}");

    // Migration is logged, and the effective settings are unchanged.
    let log = latest_log(&save);
    assert!(log.contains("schema 8→9"), "{log}");
    assert!(log.contains("Saves/_backups/"), "{log}");
    assert!(log.contains("自动化设置默认"), "{log}");
    let after = ok(&r, &["settings", "automation", "show"]);
    assert_eq!(
        before
            .lines()
            .filter(|l| l.contains("来源：存档默认"))
            .count(),
        after
            .lines()
            .filter(|l| l.contains("来源：存档默认"))
            .count()
    );
    assert!(after.contains("npc（NPC 对话）：assisted — 来源：存档默认"), "{after}");

    // Integrity stays clean and a second run changes nothing.
    assert!(ok(&r, &["check"]).contains("integrity: OK"));
    let backup_count = fs::read_dir(r.join("Saves/_backups")).unwrap().count();
    assert!(ok(&r, &["migrate"]).contains("无需迁移"));
    assert_eq!(
        fs::read_dir(r.join("Saves/_backups")).unwrap().count(),
        backup_count
    );
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn a_newer_schema_is_still_refused() {
    let (r, save) = new_save("newer");
    fs::write(
        save.join("save-meta.yaml"),
        "schema: 10\ntool: future\nsettings: {automation: {version: 1}}\n",
    )
    .unwrap();
    let (good, out) = run(&r, &["settings", "automation", "show"]);
    assert!(
        !good && out.contains("10") && out.contains("update talkdnd"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn start_digest_shows_the_effective_automation_summary() {
    let (r, save) = new_save("digest");
    add_adventure(&r, &save);
    ok(
        &r,
        &[
            "settings",
            "automation",
            "set",
            "npc",
            "manual",
            "--scene",
            "crypt-entry",
            "--reason",
            "players narrate this scene",
        ],
    );
    fs::write(
        save.join("progress.yaml"),
        "xp: {}\nscene: crypt-entry\n",
    )
    .unwrap();

    let out = ok(&r, &["start"]);
    assert!(out.contains("- 自动化："), "{out}");
    assert!(out.contains("NPC 对话 manual"), "{out}");
    assert!(out.contains("结算 auto"), "{out}");
    assert!(out.contains("场景 crypt-entry"), "{out}");
    let digest = fs::read_to_string(save.join("digest.md")).unwrap();
    assert!(digest.contains("NPC 对话 manual"), "{digest}");
    assert!(digest.contains("`talkdnd settings automation`"), "{digest}");
    let _ = fs::remove_dir_all(&r);
}
