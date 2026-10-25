//! P06 world time (TASK-0063): `time show|enable|disable|set|advance|event`
//! persistence, idempotence, boundaries, the `start` digest section and the
//! dm_only visibility rules (LetsPlay/docs/campaign-continuity.md §4.5/§5.4).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("talkdnd-time-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(p.join("Saves")).unwrap();
    fs::write(p.join("AGENTS.md"), "x").unwrap();
    p
}

/// Exit code plus combined stdout+stderr, for the exact 0/1/2 matrix.
fn code(root: &Path, args: &[&str]) -> (i32, String) {
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
    (out.status.code().unwrap_or(-1), s)
}

fn ok(root: &Path, args: &[&str]) -> String {
    let (c, s) = code(root, args);
    assert_eq!(c, 0, "{args:?} exited {c}: {s}");
    s
}

fn new_save(name: &str) -> (PathBuf, PathBuf) {
    let r = root(name);
    ok(&r, &["new", "--date", "2030-01-02"]);
    let save = r.join("Saves/2030-01-02-01");
    (r, save)
}

fn wt_path(save: &Path) -> PathBuf {
    save.join("world-time.yaml")
}

fn elapsed_days(save: &Path) -> f64 {
    let text = fs::read_to_string(wt_path(save)).unwrap();
    let v: serde_norway::Value = serde_norway::from_str(&text).unwrap();
    v["elapsed_days"].as_f64().unwrap()
}

#[test]
fn disabled_by_default_and_writes_are_refused() {
    let (r, save) = new_save("off-default");
    // show works and reports disabled.
    let out = ok(&r, &["time", "show"]);
    assert!(out.contains("未启用"), "{out}");
    // Every write command is refused with exit 1.
    for args in [
        vec!["time", "set", "--date", "1 Hammer"],
        vec!["time", "advance", "--days", "1"],
        vec![
            "time", "event", "add", "--due", "1 Hammer", "--title", "festival",
        ],
        vec!["time", "event", "done", "EV-001"],
        vec!["time", "event", "skip", "EV-001"],
    ] {
        let (c, out) = code(&r, &args);
        assert_eq!(c, 1, "{args:?} exited {c}: {out}");
        assert!(out.contains("未启用"), "{args:?}: {out}");
    }
    // The refused writes never created the sidecar.
    assert!(!wt_path(&save).exists(), "world-time.yaml must not exist");
    // event list is readable while disabled.
    let out = ok(&r, &["time", "event", "list"]);
    assert!(out.contains("0 条"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn enable_disable_set_advance_persist_log_and_are_idempotent() {
    let (r, save) = new_save("crud");
    let log = save.join("Sessions/2030-01-02-session-00-setup.md");

    // enable persists version 1 with enabled: true and logs once.
    ok(&r, &["time", "enable"]);
    let first = fs::read_to_string(wt_path(&save)).unwrap();
    assert!(
        first.contains("version: 1") && first.contains("enabled: true"),
        "{first}"
    );
    let log_text = fs::read_to_string(&log).unwrap();
    assert!(log_text.contains("世界时间：启用"), "{log_text}");
    let log_lines = log_text.lines().count();

    // Re-enable changes nothing: no rewrite, no duplicate log line.
    let out = ok(&r, &["time", "enable"]);
    assert!(out.contains("未改动"), "{out}");
    assert_eq!(fs::read_to_string(wt_path(&save)).unwrap(), first);
    assert_eq!(
        fs::read_to_string(&log).unwrap().lines().count(),
        log_lines,
        "idempotent enable must not log again"
    );

    // set --date persists and logs.
    let out = ok(
        &r,
        &["time", "set", "--date", "1 Hammer", "--phase", "morning"],
    );
    assert!(out.contains("1 Hammer"), "{out}");
    let text = fs::read_to_string(wt_path(&save)).unwrap();
    assert!(
        text.contains("date: 1 Hammer") && text.contains("phase: morning"),
        "{text}"
    );
    let log_text = fs::read_to_string(&log).unwrap();
    assert!(log_text.contains("日期"), "{log_text}");
    let log_lines = log_text.lines().count();

    // Setting the same values changes and logs nothing.
    let out = ok(
        &r,
        &["time", "set", "--date", "1 Hammer", "--phase", "morning"],
    );
    assert!(out.contains("未改动"), "{out}");
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), log_lines);
    // --date " " is empty after trimming: refused, file untouched.
    let before = fs::read_to_string(wt_path(&save)).unwrap();
    let (c, out) = code(&r, &["time", "set", "--date", "   "]);
    assert_eq!(c, 1, "{out}");
    assert_eq!(fs::read_to_string(wt_path(&save)).unwrap(), before);
    // --phase "" clears the phase.
    ok(&r, &["time", "set", "--date", "1 Hammer", "--phase", ""]);
    let text = fs::read_to_string(wt_path(&save)).unwrap();
    assert!(!text.contains("phase: morning"), "{text}");

    // advance persists: --hours 12 adds 0.5 days.
    let out = ok(&r, &["time", "advance", "--hours", "12"]);
    assert!(out.contains("0.5"), "{out}");
    assert!(
        (elapsed_days(&save) - 0.5).abs() < 1e-9,
        "{}",
        elapsed_days(&save)
    );
    let log_text = fs::read_to_string(&log).unwrap();
    assert!(log_text.contains("已过天数"), "{log_text}");

    // disable keeps everything and is idempotent.
    ok(&r, &["time", "disable"]);
    let text = fs::read_to_string(wt_path(&save)).unwrap();
    assert!(text.contains("enabled: false"), "{text}");
    assert!(text.contains("date: 1 Hammer"), "{text}");
    assert!((elapsed_days(&save) - 0.5).abs() < 1e-9);
    let out = ok(&r, &["time", "disable"]);
    assert!(out.contains("未改动"), "{out}");
    // Writes stay refused while disabled (file preserved).
    let (c, _) = code(&r, &["time", "advance", "--days", "1"]);
    assert_eq!(c, 1);
    assert!(elapsed_days(&save) - 0.5 < 1e-9);
    // Re-enable keeps the preserved data.
    ok(&r, &["time", "enable"]);
    let text = fs::read_to_string(wt_path(&save)).unwrap();
    assert!(text.contains("date: 1 Hammer"), "{text}");
    let out = ok(&r, &["time", "show"]);
    assert!(out.contains("1 Hammer") && out.contains("0.5"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn events_add_list_done_skip_with_visibility() {
    let (r, save) = new_save("events");
    ok(&r, &["time", "enable"]);
    ok(&r, &["time", "set", "--date", "1 Hammer"]);

    let out = ok(
        &r,
        &[
            "time", "event", "add", "--due", "1 Hammer", "--title", "festival",
        ],
    );
    assert!(out.contains("EV-001"), "{out}");
    let out = ok(
        &r,
        &[
            "time",
            "event",
            "add",
            "--due",
            "1 Hammer",
            "--title",
            "secret ambush",
            "--dm-only",
            "--note",
            "npc:E-001",
        ],
    );
    assert!(out.contains("EV-002"), "{out}");
    let out = ok(
        &r,
        &[
            "time", "event", "add", "--due", "2 Hammer", "--title", "later",
        ],
    );
    assert!(out.contains("EV-003"), "{out}");

    // Default list is the player view: dm_only hidden, due today marked.
    let out = ok(&r, &["time", "event", "list"]);
    assert!(out.contains("EV-001") && out.contains("festival"), "{out}");
    assert!(out.contains("今天到期"), "{out}");
    assert!(!out.contains("secret ambush"), "{out}");
    assert!(!out.contains("EV-002"), "{out}");
    // DM view shows everything with the [DM] marker.
    let out = ok(&r, &["time", "event", "list", "--all"]);
    assert!(
        out.contains("EV-002") && out.contains("secret ambush"),
        "{out}"
    );
    assert!(out.contains("[DM]"), "{out}");

    // Empty --due/--title are refused; the file is untouched.
    let before = fs::read_to_string(wt_path(&save)).unwrap();
    let (c, out) = code(&r, &["time", "event", "add", "--due", "  ", "--title", "x"]);
    assert_eq!(c, 1, "{out}");
    let (c, out) = code(&r, &["time", "event", "add", "--due", "d", "--title", " "]);
    assert_eq!(c, 1, "{out}");
    assert_eq!(fs::read_to_string(wt_path(&save)).unwrap(), before);

    // done ends the event: it leaves the today-due marks and start digest.
    let out = ok(&r, &["time", "event", "done", "EV-001"]);
    assert!(out.contains("done"), "{out}");
    let text = fs::read_to_string(wt_path(&save)).unwrap();
    assert!(text.contains("status: done"), "{text}");
    let out = ok(&r, &["time", "event", "list"]);
    assert!(!out.contains("今天到期"), "{out}");
    // Terminal states refuse re-marking, file untouched.
    let before = fs::read_to_string(wt_path(&save)).unwrap();
    let (c, out) = code(&r, &["time", "event", "done", "EV-001"]);
    assert_eq!(c, 1, "{out}");
    let (c, out) = code(&r, &["time", "event", "skip", "EV-001"]);
    assert_eq!(c, 1, "{out}");
    assert_eq!(fs::read_to_string(wt_path(&save)).unwrap(), before);
    // Unknown and malformed ids are refused.
    let (c, out) = code(&r, &["time", "event", "done", "EV-099"]);
    assert_eq!(c, 1, "{out}");
    let (c, out) = code(&r, &["time", "event", "skip", "festival"]);
    assert_eq!(c, 1, "{out}");
    assert_eq!(fs::read_to_string(wt_path(&save)).unwrap(), before);

    // skip works on a pending event.
    ok(&r, &["time", "event", "skip", "EV-003"]);
    let text = fs::read_to_string(wt_path(&save)).unwrap();
    assert!(text.contains("status: skipped"), "{text}");
    // Ids are never reused: the next add continues after the highest seen.
    let out = ok(
        &r,
        &[
            "time", "event", "add", "--due", "5 Hammer", "--title", "last",
        ],
    );
    assert!(out.contains("EV-004"), "{out}");
    // The session log carries the event changes.
    let log = fs::read_to_string(save.join("Sessions/2030-01-02-session-00-setup.md")).unwrap();
    assert!(log.contains("新增事件 EV-001"), "{log}");
    assert!(log.contains("标记完成") || log.contains("done"), "{log}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn show_counts_only_player_visible_events() {
    let (r, _save) = new_save("show-counts");
    ok(&r, &["time", "enable"]);
    ok(&r, &["time", "set", "--date", "1 Hammer"]);
    // 2 player-visible + 1 dm_only pending events.
    ok(
        &r,
        &[
            "time", "event", "add", "--due", "1 Hammer", "--title", "festival",
        ],
    );
    ok(
        &r,
        &[
            "time",
            "event",
            "add",
            "--due",
            "1 Hammer",
            "--title",
            "secret ambush",
            "--dm-only",
        ],
    );
    ok(
        &r,
        &[
            "time", "event", "add", "--due", "2 Hammer", "--title", "later",
        ],
    );

    // Enabled show reports only the 2 visible events (contract §6.1): the
    // dm_only event must not leak through either total or pending count.
    let out = ok(&r, &["time", "show"]);
    assert!(out.contains("事件：2 条"), "{out}");
    assert!(out.contains("待发生 2"), "{out}");
    assert!(!out.contains("事件：3 条"), "{out}");
    assert!(!out.contains("secret ambush"), "{out}");

    // The disabled branch keeps the same player-visible totals.
    ok(&r, &["time", "disable"]);
    let out = ok(&r, &["time", "show"]);
    assert!(out.contains("事件 2 条"), "{out}");
    assert!(!out.contains("事件 3 条"), "{out}");

    // Back on, one visible event done: still 2 visible, 1 pending.
    ok(&r, &["time", "enable"]);
    ok(&r, &["time", "event", "done", "EV-001"]);
    let out = ok(&r, &["time", "show"]);
    assert!(out.contains("事件：2 条"), "{out}");
    assert!(out.contains("待发生 1"), "{out}");
    assert!(!out.contains("事件：3 条"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn only_explicit_commands_change_world_time() {
    let (r, save) = new_save("no-auto");
    ok(&r, &["time", "enable"]);
    ok(&r, &["time", "set", "--date", "1 Hammer"]);
    let before = fs::read(wt_path(&save)).unwrap();
    let days_before = elapsed_days(&save);

    // Reads and unrelated commands never touch the sidecar.
    ok(&r, &["start"]);
    ok(&r, &["start"]);
    ok(&r, &["check"]);
    ok(&r, &["time", "show"]);
    ok(&r, &["time", "event", "list"]);
    assert_eq!(
        fs::read(wt_path(&save)).unwrap(),
        before,
        "start/check/show/list must not write"
    );
    assert!((elapsed_days(&save) - days_before).abs() < 1e-12);

    // Only advance/set move the clock, and repeated advance is pure addition.
    ok(&r, &["time", "advance", "--days", "1"]);
    let d1 = elapsed_days(&save);
    ok(&r, &["time", "advance", "--days", "1"]);
    let d2 = elapsed_days(&save);
    assert!((d1 - 1.0).abs() < 1e-9, "{d1}");
    assert!((d2 - 2.0).abs() < 1e-9, "{d2}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn advance_boundaries_and_hour_conversion() {
    let (r, save) = new_save("boundaries");
    ok(&r, &["time", "enable"]);

    // Zero advance is refused (exit 1), file not created.
    let (c, out) = code(&r, &["time", "advance", "--days", "0", "--hours", "0"]);
    assert_eq!(c, 1, "{out}");
    assert!(out.contains("大于 0"), "{out}");
    // Negative values are usage errors (exit 2), both clap-side and ours.
    let (c, _) = code(&r, &["time", "advance", "--days", "-5"]);
    assert_eq!(c, 2, "negative --days must exit 2");
    let (c, out) = code(&r, &["time", "advance", "--days=-5"]);
    assert_eq!(c, 2, "{out}");
    // Values beyond u32 overflow: exit 1, file untouched.
    let before = fs::read(wt_path(&save)).unwrap();
    let (c, out) = code(&r, &["time", "advance", "--days", "99999999999"]);
    assert_eq!(c, 1, "{out}");
    assert_eq!(
        fs::read(wt_path(&save)).unwrap(),
        before,
        "refused writes must not change the file"
    );

    // 24 × --hours 1 displays as 1 and stays within 1e-6 internally.
    for _ in 0..24 {
        ok(&r, &["time", "advance", "--hours", "1"]);
    }
    let out = ok(&r, &["time", "show"]);
    assert!(
        out.contains("已过天数：1\n") || out.contains("已过天数：1（"),
        "{out}"
    );
    let d = elapsed_days(&save);
    assert!((d - 1.0).abs() < 1e-6, "{d}");
    // --days and --hours combine (error still below 1e-6 after 24 steps).
    ok(&r, &["time", "advance", "--days", "1", "--hours", "12"]);
    assert!(
        (elapsed_days(&save) - 2.5).abs() < 1e-6,
        "{}",
        elapsed_days(&save)
    );
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn start_digest_shows_world_time_only_when_enabled() {
    let (r, save) = new_save("digest");

    // Disabled: no section at all.
    let out = ok(&r, &["start"]);
    assert!(!out.contains("世界时间"), "{out}");

    ok(&r, &["time", "enable"]);
    ok(&r, &["time", "set", "--date", "1 Hammer"]);
    ok(
        &r,
        &[
            "time", "event", "add", "--due", "1 Hammer", "--title", "festival",
        ],
    );
    ok(
        &r,
        &[
            "time",
            "event",
            "add",
            "--due",
            "1 Hammer",
            "--title",
            "secret ambush",
            "--dm-only",
        ],
    );
    let out = ok(&r, &["start"]);
    assert!(out.contains("## 世界时间"), "{out}");
    assert!(out.contains("日期：1 Hammer"), "{out}");
    assert!(out.contains("已过天数"), "{out}");
    assert!(out.contains("festival"), "{out}");
    assert!(
        !out.contains("secret ambush"),
        "dm_only events never reach start: {out}"
    );

    // done removes the event from the digest.
    ok(&r, &["time", "event", "done", "EV-001"]);
    let out = ok(&r, &["start"]);
    assert!(out.contains("## 世界时间"), "{out}");
    assert!(!out.contains("festival"), "{out}");

    // Enabled without due events: only the date/day lines.
    ok(
        &r,
        &[
            "time", "event", "add", "--due", "9 Hammer", "--title", "later",
        ],
    );
    let out = ok(&r, &["start"]);
    let section = out.split("## 世界时间").nth(1).unwrap_or("");
    assert!(section.contains("日期：1 Hammer"), "{out}");
    assert!(!section.contains("今天到期"), "{section}");

    // start never wrote the sidecar along the way.
    let text = fs::read_to_string(wt_path(&save)).unwrap();
    assert!(text.contains("enabled: true"), "{text}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn broken_or_newer_world_time_files_are_refused() {
    let (r, save) = new_save("broken");
    fs::write(wt_path(&save), "enabled: [unclosed\n").unwrap();
    let (c, out) = code(&r, &["time", "show"]);
    assert_eq!(c, 1, "{out}");
    assert!(out.contains("world-time.yaml"), "{out}");
    fs::write(wt_path(&save), "version: 2\nenabled: true\n").unwrap();
    let (c, out) = code(&r, &["time", "show"]);
    assert_eq!(c, 1, "{out}");
    assert!(
        out.contains("world-time.yaml") && out.contains("2"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn integrity_tracks_world_time_writes() {
    let (r, save) = new_save("integrity");
    ok(&r, &["time", "enable"]);
    ok(&r, &["time", "set", "--date", "1 Hammer"]);
    ok(
        &r,
        &[
            "time", "event", "add", "--due", "1 Hammer", "--title", "festival",
        ],
    );
    // TASK-0071 (contract §3.4): the tool write itself is still clean —
    // no file-diff warning — and the due-today event is now reported as the
    // contract-mandated cross-ref warning (existing expectation updated,
    // not weakened: the tamper half below still proves write tracking).
    let out = ok(&r, &["check"]);
    assert!(
        out.contains("cross-ref: 世界时间事件 EV-001 今日到期"),
        "{out}"
    );
    assert!(!out.contains("changed outside the tools"), "{out}");
    // A hand edit is reported: the file shows as "changed outside the tools".
    fs::write(
        wt_path(&save),
        "version: 1\nenabled: true\ndate: tampered\n",
    )
    .unwrap();
    let out = ok(&r, &["check"]);
    assert!(out.contains("changed outside the tools"), "{out}");
    assert!(out.contains("world-time.yaml"), "{out}");
    let _ = fs::remove_dir_all(&r);
}
