//! Entity ledger (P04, TASK-0062; LetsPlay/docs/campaign-continuity.md §4.3/§5.3).
//!
//! Covers the acceptance addendum: name/alias conflicts, case/ambiguity,
//! rename/retire semantics, dm_only player filtering, idempotency, and
//! `talkdnd check` reporting hand edits to entities.yaml (P00b integrity).

use std::fs;
use std::path::{Path, PathBuf};
use talkdnd::audit;
use talkdnd::entity::{self};

/// A minimal save with two session logs (current = session-01).
fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("talkdnd-ent-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let save = root.join("Saves/2030-01-02-01");
    fs::create_dir_all(save.join("Sessions")).unwrap();
    fs::write(root.join("AGENTS.md"), "x\n").unwrap();
    fs::write(
        save.join("Sessions/2030-01-02-session-00-setup.md"),
        "# 会话\n",
    )
    .unwrap();
    fs::write(save.join("Sessions/2030-01-02-session-01.md"), "# 会话\n").unwrap();
    (root, save)
}

fn add(save: &Path, kind: &str, name: &str, aliases: &[&str]) -> String {
    let alias: Vec<String> = aliases.iter().map(|s| s.to_string()).collect();
    let line = entity::add(save, kind, name, &alias, None, false).unwrap();
    line.split_whitespace().next().unwrap().to_string()
}

fn ledger(save: &Path) -> talkdnd::entity::Ledger {
    entity::load(save).unwrap()
}

fn log(save: &Path) -> String {
    fs::read_to_string(save.join("Sessions/2030-01-02-session-01.md")).unwrap()
}

#[test]
fn add_list_show_and_resolution_round_trip() {
    let (_root, save) = fixture("roundtrip");
    let id = add(&save, "npc", "Mira", &["Bentwood"]);
    assert_eq!(id, "E-001");
    assert_eq!(
        ledger(&save).entities[0].first_seen,
        "2030-01-02-session-01.md"
    );

    let listed = entity::list(&save, None, false).unwrap().join("\n");
    assert!(
        listed.contains("E-001  npc  Mira（别名：Bentwood）"),
        "{listed}"
    );
    let by_type = entity::list(&save, Some("npc"), false).unwrap();
    assert_eq!(by_type.len(), 1);
    assert!(entity::list(&save, Some("quest"), false).unwrap()[0].contains("没有符合条件的实体"));

    // show by id, loose id, name, alias and case-insensitive all resolve.
    for key in ["E-001", "e1", "Mira", "BENTWOOD", "mira"] {
        let shown = entity::show(&save, key).unwrap().join("\n");
        assert!(shown.contains("E-001  npc  Mira"), "{key}: {shown}");
        assert!(
            shown.contains("- 首见：2030-01-02-session-01.md"),
            "{key}: {shown}"
        );
        assert!(shown.contains("- 状态：active"), "{key}: {shown}");
        assert!(shown.contains("- 生效关系：0 条"), "{key}: {shown}");
    }
    assert!(entity::show(&save, "nobody").is_err());
    // A newer schema version is refused.
    fs::write(save.join("entities.yaml"), "version: 2\nentities: []\n").unwrap();
    let err = entity::load(&save).unwrap_err().to_string();
    assert!(err.contains("entities.yaml") && err.contains("2"), "{err}");
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn name_and_alias_conflicts_are_refused_and_never_merged() {
    let (_root, save) = fixture("conflicts");
    add(&save, "npc", "Mira", &["Bentwood"]);
    add(&save, "faction", "贤者商会", &[]);

    // Same primary name, different type: refused with the conflicting entity.
    let err = entity::add(&save, "location", "Mira", &[], None, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("冲突") && err.contains("E-001"), "{err}");

    // New entity whose alias collides with another entity's name or alias.
    for name in ["贤者商会", "Bentwood"] {
        let err = entity::add(&save, "item", name, &[], None, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("冲突"), "{name}: {err}");
    }
    let mira = vec!["Mira".to_string()];
    let err = entity::add(&save, "npc", "Tess", &mira, None, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("冲突") && err.contains("E-001"), "{err}");
    let mira_ci = vec!["mira ".to_string()];
    assert!(entity::add(&save, "npc", "Tess", &mira_ci, None, false).is_err());

    // Re-adding the same name and type is idempotent, not a duplicate.
    let again = entity::add(&save, "npc", "MIRA", &[], None, false).unwrap();
    assert!(
        again.contains("已存在") && again.contains("E-001"),
        "{again}"
    );
    assert_eq!(ledger(&save).entities.len(), 2);
    // Nothing was auto-merged: the alias set is unchanged.
    assert_eq!(ledger(&save).entities[0].aliases, ["Bentwood"]);
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn ambiguous_keys_are_refused_with_candidates() {
    let (_root, save) = fixture("ambiguous");
    add(&save, "npc", "Mira", &[]);
    // Hand-edited duplicate names are refused on resolution (check 兜底).
    let mut l = ledger(&save);
    let mut twin = l.entities[0].clone();
    twin.id = "E-002".into();
    l.entities.push(twin);
    fs::write(
        save.join("entities.yaml"),
        serde_norway::to_string(&l).unwrap(),
    )
    .unwrap();
    let err = entity::show(&save, "Mira").unwrap_err().to_string();
    assert!(
        err.contains("歧义") && err.contains("E-001") && err.contains("E-002"),
        "{err}"
    );
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn rename_keeps_alias_history_and_is_idempotent() {
    let (_root, save) = fixture("rename");
    let id = add(&save, "npc", "Mira", &[]);
    add(&save, "faction", "商会", &[]);

    let out = entity::rename(&save, "mira", "Mira Bentwood").unwrap();
    assert!(out.contains("Mira → Mira Bentwood"), "{out}");
    let e = &ledger(&save).entities[0];
    assert_eq!(e.name, "Mira Bentwood");
    assert_eq!(e.aliases, ["Mira"]);
    assert!(log(&save).contains("实体 E-001 改名：Mira → Mira Bentwood"));

    // Repeating the same rename is a no-op: no duplicate alias, no log line.
    entity::rename(&save, &id, "Mira Bentwood").unwrap();
    let e = &ledger(&save).entities[0];
    assert_eq!(e.aliases, ["Mira"]);
    let log_text = log(&save);
    assert_eq!(log_text.matches("改名").count(), 1, "{log_text}");

    // Renaming to one of its own aliases promotes it back (round trip).
    entity::rename(&save, &id, "Mira").unwrap();
    let e = &ledger(&save).entities[0];
    assert_eq!(e.name, "Mira");
    assert_eq!(e.aliases, ["Mira Bentwood"]);

    // Clashes with other entities are refused, nothing auto-merged.
    let err = entity::rename(&save, &id, "商会").unwrap_err().to_string();
    assert!(err.contains("冲突") && err.contains("E-002"), "{err}");
    assert_eq!(ledger(&save).entities[0].name, "Mira");
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn retire_keeps_the_record_and_filters_the_default_list() {
    let (_root, save) = fixture("retire");
    let id = add(&save, "npc", "Mira", &[]);
    add(&save, "npc", "Tess", &[]);

    let out = entity::retire(&save, &id).unwrap();
    assert!(out.contains("已退役"), "{out}");
    let e = &ledger(&save).entities[0];
    assert_eq!(e.status, "retired");
    assert!(log(&save).contains("退役"));

    // Default list hides retired entities; --all marks them; show still works.
    let listed = entity::list(&save, None, false).unwrap().join("\n");
    assert!(
        listed.contains("Tess") && !listed.contains("E-001"),
        "{listed}"
    );
    let all = entity::list(&save, None, true).unwrap().join("\n");
    assert!(all.contains("E-001") && all.contains("[retired]"), "{all}");
    let shown = entity::show(&save, "Mira").unwrap().join("\n");
    assert!(shown.contains("- 状态：retired"), "{shown}");

    // Retiring again is a no-op.
    let again = entity::retire(&save, "E-001").unwrap();
    assert!(again.contains("未改动"), "{again}");
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn dm_only_entities_stay_out_of_the_player_view() {
    let (_root, save) = fixture("dmonly");
    add(&save, "npc", "Mira", &[]);
    let alias: Vec<String> = vec![];
    entity::add(&save, "quest", "寻找盟友", &alias, None, true).unwrap();

    let listed = entity::list(&save, None, false).unwrap().join("\n");
    assert!(
        listed.contains("Mira") && !listed.contains("寻找盟友"),
        "{listed}"
    );
    assert!(!listed.contains("E-002"), "{listed}");
    let all = entity::list(&save, None, true).unwrap().join("\n");
    assert!(
        all.contains("寻找盟友") && all.contains("[DM-only]"),
        "{all}"
    );
    let quests = entity::list(&save, Some("quest"), false)
        .unwrap()
        .join("\n");
    assert!(quests.contains("没有符合条件的实体"), "{quests}");

    // show and rename by key still resolve, but show refuses (player view).
    assert!(entity::show(&save, "寻找盟友").is_err());
    assert!(entity::rename(&save, "E-002", "新的任务名").is_ok());
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn check_reports_hand_edits_to_entities_yaml() {
    let (_root, save) = fixture("check");
    add(&save, "npc", "Mira", &[]);
    let report = audit::audit(&save).unwrap();
    assert!(
        report.clean(),
        "{:?}\n{:?}",
        report.changes,
        report.unexplained
    );

    // Hand edit behind the tool's back.
    let p = save.join("entities.yaml");
    let text = fs::read_to_string(&p).unwrap();
    fs::write(&p, text.replace("Mira", "Mirabel")).unwrap();
    let report = audit::audit(&save).unwrap();
    assert!(
        report.changes.iter().any(|c| c.path() == "entities.yaml"),
        "{:?}",
        report.changes
    );
    let _ = fs::remove_dir_all(&_root);
}
