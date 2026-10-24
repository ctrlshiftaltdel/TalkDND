//! Relations between entities (P04, TASK-0062; LetsPlay/docs/campaign-continuity.md §4.4/§5.3).
//!
//! Covers the acceptance addendum: relation add/end/query, historical --at
//! filtering, ended relations staying out of later sessions, the correction
//! audit trail, dm_only filtering and add/end idempotency.

use std::fs;
use std::path::{Path, PathBuf};
use talkdnd::entity::{self};
use talkdnd::graph::{self};

/// A minimal save with two session logs (current = session-01).
fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("talkdnd-rel-{tag}-{}", std::process::id()));
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

/// Two entities: E-001（Mira）and E-002（贤者商会）.
fn two_entities(save: &Path) {
    entity::add(save, "npc", "Mira", &[], None, false).unwrap();
    entity::add(save, "faction", "贤者商会", &[], None, false).unwrap();
}

fn relations(save: &Path) -> Vec<talkdnd::graph::Relation> {
    graph::load(save).unwrap().relations
}

fn log(save: &Path) -> String {
    fs::read_to_string(save.join("Sessions/2030-01-02-session-01.md")).unwrap()
}

fn add(
    save: &Path,
    kind: &str,
    from: &str,
    to: &str,
    since: Option<&str>,
    dm_only: bool,
) -> anyhow::Result<String> {
    graph::add(save, kind, from, to, since, Some("manual"), None, dm_only)
}

#[test]
fn add_list_show_and_idempotent_readd() {
    let (_root, save) = fixture("add");
    two_entities(&save);
    let out = add(&save, "ally", "mira", "贤者商会", None, false).unwrap();
    assert!(out.starts_with("R-001"), "{out}");
    assert!(out.contains("E-001（Mira） → E-002（贤者商会）"), "{out}");
    assert!(out.contains("自 2030-01-02-session-01.md"), "{out}");
    assert!(out.contains("来源 manual"), "{out}");

    // Loose ids and names resolve; from/to are stored as ids.
    let r = &relations(&save)[0];
    assert_eq!((r.from.as_str(), r.to.as_str()), ("E-001", "E-002"));

    // Same (type, from, to) while open: idempotent, no second row.
    let again = add(&save, "ally", "E-001", "E-002", None, false).unwrap();
    assert!(
        again.contains("已存在") && again.contains("R-001"),
        "{again}"
    );
    assert_eq!(relations(&save).len(), 1);

    // A different type is a new row.
    let out = add(&save, "knows", "E-001", "E-002", None, false).unwrap();
    assert!(out.starts_with("R-002"), "{out}");

    let listed = graph::list(&save, None, None, false).unwrap();
    assert_eq!(listed.len(), 2, "{listed:?}");
    let by_entity = graph::list(&save, Some("Mira"), None, false).unwrap();
    assert_eq!(by_entity.len(), 2, "{by_entity:?}");
    let shown = graph::show(&save, "r1").unwrap().join("\n");
    assert!(shown.contains("- 自：2030-01-02-session-01.md"), "{shown}");
    assert!(shown.contains("- 至：（仍生效）"), "{shown}");
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn historical_at_filtering_and_ended_relations() {
    let (_root, save) = fixture("history");
    two_entities(&save);
    // Opened in session-00, still open: visible now (session-01) and at S00.
    add(
        &save,
        "ally",
        "E-001",
        "E-002",
        Some("2030-01-02-session-00"),
        false,
    )
    .unwrap();
    assert_eq!(graph::list(&save, None, None, false).unwrap().len(), 1);
    assert_eq!(
        graph::list(&save, None, Some("2030-01-02-session-00-setup.md"), false)
            .unwrap()
            .len(),
        1
    );
    // Not yet in force before any logged session.
    let before = graph::list(&save, None, Some("2030-01-01-session-00.md"), false).unwrap();
    assert!(before[0].contains("没有符合条件"), "{before:?}");
    // A relation scheduled for a later session is invisible now.
    add(
        &save,
        "enemy",
        "E-002",
        "E-001",
        Some("2030-01-02-session-02"),
        false,
    )
    .unwrap();
    let now = graph::list(&save, None, None, false).unwrap().join("\n");
    assert!(now.contains("ally") && !now.contains("enemy"), "{now}");
    let future = graph::list(&save, None, Some("2030-01-02-session-02.md"), false).unwrap();
    // Both the still-open ally and the scheduled enemy are in force there.
    assert_eq!(future.len(), 2, "{future:?}");
    assert!(future.iter().any(|l| l.contains("enemy")));

    // End the ally relation at session-01: gone from now on, but --at S00
    // still shows it and --all marks it ended. Loose input resolves to the
    // real log name (2030-01-02-session-01.md).
    let out = graph::end(&save, "R-001", Some("2030-01-02-session-01"), None).unwrap();
    assert!(out.contains("已结束 2030-01-02-session-01.md"), "{out}");
    assert!(graph::list(&save, None, None, false).unwrap()[0].contains("没有符合条件"));
    let at00 = graph::list(&save, None, Some("2030-01-02-session-00-setup.md"), false).unwrap();
    assert_eq!(at00.len(), 1, "{at00:?}");
    let all = graph::list(&save, None, None, true).unwrap().join("\n");
    assert!(all.contains("[已结束 2030-01-02-session-01.md]"), "{all}");
    assert!(all.contains("enemy"), "{all}");
    // Ending again is a no-op.
    let again = graph::end(&save, "R-001", None, None).unwrap();
    assert!(again.contains("未改动"), "{again}");
    assert_eq!(
        relations(&save)[0].until_session,
        "2030-01-02-session-01.md"
    );
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn corrections_keep_the_audit_trail() {
    let (_root, save) = fixture("corrections");
    two_entities(&save);
    add(&save, "enemy", "E-001", "E-002", None, false).unwrap();
    let out = graph::end(
        &save,
        "R-001",
        None,
        Some("登记错误：应为 ally，另建新关系"),
    )
    .unwrap();
    assert!(out.contains("已结束"), "{out}");

    // The row survives with its end marker and note; nothing was deleted.
    let r = &relations(&save)[0];
    assert_eq!(r.id, "R-001");
    assert_eq!(r.kind, "enemy");
    assert!(!r.until_session.is_empty());
    assert!(
        r.note.contains("登记错误"),
        "{:?}\n{:?}",
        r.note,
        r.until_session
    );
    let text = log(&save);
    assert!(text.contains("关系 R-001 结束于"), "{text}");

    // The corrected relation is a new row with its own id.
    add(&save, "ally", "E-001", "E-002", None, false).unwrap();
    let all = relations(&save);
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].id, "R-002");
    assert_eq!(all[1].kind, "ally");
    assert!(all[1].until_session.is_empty());
    let shown = graph::show(&save, "R-001").unwrap().join("\n");
    assert!(shown.contains("登记错误"), "{shown}");
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn dm_only_relations_stay_out_of_the_player_view() {
    let (_root, save) = fixture("dmonly");
    two_entities(&save);
    add(&save, "ally", "E-001", "E-002", None, false).unwrap();
    add(&save, "knows", "E-001", "E-002", None, true).unwrap();

    let now = graph::list(&save, None, None, false).unwrap().join("\n");
    assert!(now.contains("ally") && !now.contains("knows"), "{now}");
    let all = graph::list(&save, None, None, true).unwrap().join("\n");
    assert!(all.contains("knows") && all.contains("[DM-only]"), "{all}");
    assert!(graph::show(&save, "R-002").is_err());

    // context is the player view: the dm_only relation is hidden.
    let ctx = graph::context(&save, "mira").unwrap().join("\n");
    assert!(ctx.contains("E-001（Mira）"), "{ctx}");
    assert!(ctx.contains("当前生效关系（1 条）"), "{ctx}");
    assert!(ctx.contains("ally") && !ctx.contains("knows"), "{ctx}");

    // An entity that is itself dm_only is refused in list/context/show paths.
    entity::add(&save, "item", "密钥", &[], None, true).unwrap();
    assert!(graph::list(&save, Some("密钥"), None, false).is_err());
    assert!(graph::context(&save, "密钥").is_err());
    assert!(entity::show(&save, "密钥").is_err());
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn context_refuses_retired_entities_but_relations_stay_queryable() {
    let (_root, save) = fixture("retired");
    two_entities(&save);
    add(&save, "ally", "E-001", "E-002", None, false).unwrap();
    entity::retire(&save, "E-002").unwrap();
    let err = graph::context(&save, "E-002").unwrap_err().to_string();
    assert!(err.contains("已退役"), "{err}");
    // relation list on a retired entity still works (explicit query).
    let listed = graph::list(&save, Some("E-002"), None, false).unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    // show of the retired entity still works.
    let shown = entity::show(&save, "E-002").unwrap().join("\n");
    assert!(shown.contains("retired"), "{shown}");
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn invalid_input_is_refused() {
    let (_root, save) = fixture("invalid");
    two_entities(&save);
    assert!(add(&save, "rival", "E-001", "E-002", None, false).is_err());
    assert!(add(&save, "ally", "E-009", "E-002", None, false).is_err());
    assert!(add(&save, "ally", "E-001", "不存在的人", None, false).is_err());
    assert!(graph::end(&save, "R-099", None, None).is_err());
    assert!(graph::show(&save, "R-099").is_err());
    // end before since is refused.
    add(
        &save,
        "ally",
        "E-001",
        "E-002",
        Some("2030-01-02-session-01"),
        false,
    )
    .unwrap();
    let err = graph::end(&save, "R-001", Some("2030-01-02-session-00-setup.md"), None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("早于"), "{err}");
    // A newer schema version is refused.
    fs::write(save.join("relations.yaml"), "version: 2\nrelations: []\n").unwrap();
    let err = graph::load(&save).unwrap_err().to_string();
    assert!(err.contains("relations.yaml") && err.contains("2"), "{err}");
    let _ = fs::remove_dir_all(&_root);
}

#[test]
fn end_and_add_write_session_log_lines() {
    let (_root, save) = fixture("logging");
    two_entities(&save);
    add(&save, "ally", "E-001", "E-002", None, false).unwrap();
    graph::end(&save, "R-001", None, Some("破裂")).unwrap();
    let text = log(&save);
    assert!(text.contains("关系 R-001 登记：ally"), "{text}");
    assert!(
        text.contains("关系 R-001 结束于 2030-01-02-session-01.md"),
        "{text}"
    );
    assert!(text.contains("破裂"), "{text}");
    let _ = fs::remove_dir_all(&_root);
}
