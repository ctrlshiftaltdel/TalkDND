//! Cross-reference audit for `talkdnd check` (P07, TASK-0071;
//! LetsPlay/docs/campaign-continuity.md §3.4): dangling relation endpoints,
//! clashing entity names/aliases, world-time events due today and unusable
//! checkpoints — warnings only, `check` still exits 0.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use talkdnd::audit;

fn root(tag: &str) -> PathBuf {
    let r = std::env::temp_dir().join(format!("talkdnd-xref-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&r);
    let save = r.join("Saves/2030-01-02-01");
    fs::create_dir_all(save.join("Sessions")).unwrap();
    fs::write(
        save.join("Sessions/2030-01-02-session-00-setup.md"),
        "# 会话\n",
    )
    .unwrap();
    talkdnd::migrate::init(&save).unwrap();
    r
}

fn save_of(r: &Path) -> PathBuf {
    r.join("Saves/2030-01-02-01")
}

fn bin(root: &Path, args: &[&str]) -> (bool, String) {
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

/// A save exhibiting all four contract §3.4 findings (plus a name that
/// equals its own alias, which only hand edits can produce).
fn plant_all_four(save: &Path) {
    fs::write(
        save.join("entities.yaml"),
        "version: 1\nentities:\n  - {id: E-001, type: npc, name: Mira, first_seen: s.md, status: active}\n  - {id: E-002, type: npc, name: Miran, aliases: [mira], first_seen: s.md, status: active}\n  - {id: E-003, type: item, name: Key, aliases: [Key], first_seen: s.md, status: active}\n",
    )
    .unwrap();
    fs::write(
        save.join("relations.yaml"),
        "version: 1\nrelations:\n  - {id: R-001, type: ally, from: E-001, to: E-999, since_session: s.md}\n  - {id: R-002, type: knows, from: E-777, to: E-001, since_session: s.md}\n",
    )
    .unwrap();
    fs::write(
        save.join("world-time.yaml"),
        "version: 1\nenabled: true\ndate: '2030-01-01'\nevents:\n  - {id: EV-001, due: '2030-01-01', title: festival, status: pending}\n  - {id: EV-002, due: '2030-02-01', title: later, status: pending}\n  - {id: EV-003, due: '2030-01-01', title: past, status: done}\n",
    )
    .unwrap();
    fs::create_dir_all(save.join("checkpoints/20200101-000000")).unwrap();
    fs::create_dir_all(save.join("checkpoints/20200101-000001")).unwrap();
    fs::write(
        save.join("checkpoints/20200101-000001/manifest.yaml"),
        "version: 1\nid: [broken\n",
    )
    .unwrap();
}

#[test]
fn clean_save_has_no_cross_ref_warnings() {
    let r = root("clean");
    let save = save_of(&r);
    let report = audit::audit(&save).unwrap();
    assert!(report.clean(), "{:?}", report.lines());
    assert!(report.cross_ref.is_empty(), "{:?}", report.cross_ref);
    let (good, out) = bin(&r, &["check"]);
    assert!(good, "{out}");
    assert!(out.contains("integrity: OK"), "{out}");
    assert!(!out.contains("cross-ref:"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn check_reports_the_four_contract_cross_refs_and_exits_zero() {
    let r = root("four");
    let save = save_of(&r);
    plant_all_four(&save);
    let (good, out) = bin(&r, &["check"]);
    assert!(good, "cross-ref findings are warnings, never errors: {out}");
    // 1. dangling relation endpoints, one line each, with the relation id
    assert!(
        out.contains("cross-ref: 关系 R-001（ally）：to 指向不存在的实体 E-999"),
        "{out}"
    );
    assert!(
        out.contains("cross-ref: 关系 R-002（knows）：from 指向不存在的实体 E-777"),
        "{out}"
    );
    // 2. clashing names/aliases, case-insensitive, one line per shared key
    assert!(out.contains("cross-ref: 名称/别名冲突「mira」"), "{out}");
    assert!(
        out.contains("cross-ref: 实体 E-003（Key）的主名与别名相同"),
        "{out}"
    );
    // 3. the world-time event due today (pending only, future/done ignored)
    assert!(
        out.contains("cross-ref: 世界时间事件 EV-001 今日到期"),
        "{out}"
    );
    assert!(!out.contains("EV-002"), "{out}");
    // 4. unusable checkpoints, with the same problem as checkpoint list
    assert!(
        out.contains("cross-ref: 检查点 20200101-000000 不可用：manifest.yaml 缺失"),
        "{out}"
    );
    assert!(
        out.contains("cross-ref: 检查点 20200101-000001 不可用：manifest.yaml 解析失败"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn fixing_the_data_clears_the_warnings() {
    let r = root("fix");
    let save = save_of(&r);
    plant_all_four(&save);
    let (good, out) = bin(&r, &["check"]);
    assert!(good && out.contains("cross-ref:"), "{out}");
    // Fix everything (by hand here; integrity flags the hand edits, but the
    // cross-ref findings must disappear all the same).
    fs::write(
        save.join("entities.yaml"),
        "version: 1\nentities:\n  - {id: E-001, type: npc, name: Mira, first_seen: s.md, status: active}\n",
    )
    .unwrap();
    fs::write(save.join("relations.yaml"), "version: 1\nrelations: []\n").unwrap();
    fs::write(
        save.join("world-time.yaml"),
        "version: 1\nenabled: true\ndate: '2030-01-01'\nevents:\n  - {id: EV-001, due: '2030-01-01', title: festival, status: done}\n",
    )
    .unwrap();
    fs::write(
        save.join("checkpoints/20200101-000000/manifest.yaml"),
        "version: 1\nid: 20200101-000000\ncreated: '2020-01-01 00:00'\nschema: 8\nlog_file: s.md\nlog_lines: 1\nfiles: {}\n",
    )
    .unwrap();
    fs::remove_dir_all(save.join("checkpoints/20200101-000001")).unwrap();
    let (good, out) = bin(&r, &["check"]);
    assert!(good, "{out}");
    assert!(!out.contains("cross-ref:"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn unloadable_files_become_warnings_not_failures() {
    let r = root("unloadable");
    let save = save_of(&r);
    fs::write(save.join("entities.yaml"), "version: 9\nentities: []\n").unwrap();
    fs::write(save.join("world-time.yaml"), "enabled: [unclosed\n").unwrap();
    let (good, out) = bin(&r, &["check"]);
    assert!(good, "check must not abort on unloadable sidecars: {out}");
    assert!(
        out.contains("cross-ref: entities.yaml 无法读取")
            && out.contains("unsupported entity schema version 9"),
        "{out}"
    );
    assert!(out.contains("cross-ref: world-time.yaml 无法读取"), "{out}");
    // The dependent checks were skipped, not invented.
    assert!(!out.contains("今日到期"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn disabled_world_time_due_events_are_reported_with_a_note() {
    let r = root("disabled");
    let save = save_of(&r);
    fs::write(
        save.join("world-time.yaml"),
        "version: 1\nenabled: false\ndate: '2030-01-01'\nevents:\n  - {id: EV-001, due: '2030-01-01', title: festival, status: pending}\n",
    )
    .unwrap();
    let report = audit::audit(&save).unwrap();
    assert_eq!(
        report.cross_ref,
        vec!["世界时间事件 EV-001 今日到期（due 2030-01-01，pending）；世界时间未启用"]
    );
    assert!(!report.clean());
    let _ = fs::remove_dir_all(&r);
}
