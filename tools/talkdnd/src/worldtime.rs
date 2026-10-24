//! World time (P06; LetsPlay/docs/campaign-continuity.md §2.4, §4.5, §5.4).
//!
//! `talkdnd time ...` keeps an optional in-world calendar with due-dated
//! events in `Saves/<slot>/world-time.yaml` (version 1). It is off by
//! default, never advances on its own — only `time set` / `time advance`
//! move it, and every write is refused while disabled. Due checking is
//! label equality only: `due == date` with `status: pending` means "due
//! today"; no calendar arithmetic (month lengths, leap years) is done.
//!
//! Visibility (contract §6.1, owner decision 2026-10-25): `time event
//! list` defaults to the player view (dm_only events hidden); `--all` is
//! the DM view with a `[DM]` marker, and the `start` digest shows only
//! player-visible due events. The digest here is read-only: `start` never
//! writes world-time.yaml.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Show enabled/calendar/date/phase/elapsed_days and the event count
    Show,
    /// Enable world time
    Enable,
    /// Disable world time (date/phase/events/elapsed_days are kept)
    Disable,
    /// Set the date label (and optional phase); refused while disabled
    Set {
        /// New date label
        #[arg(long)]
        date: String,
        /// New phase label (--phase "" clears it)
        #[arg(long)]
        phase: Option<String>,
    },
    /// Advance the clock (pure addition; at least one of --days/--hours)
    Advance {
        /// Days to add
        #[arg(long, value_name = "N")]
        days: Option<String>,
        /// Hours to add (h/24 days)
        #[arg(long, value_name = "N")]
        hours: Option<String>,
    },
    /// Due-dated events
    Event {
        #[command(subcommand)]
        cmd: EventCmd,
    },
}

#[derive(Subcommand, Clone)]
pub enum EventCmd {
    /// Add a due-dated event
    Add {
        /// Due label (the date label it comes due on)
        #[arg(long)]
        due: String,
        #[arg(long)]
        title: String,
        /// DM-only event (never in the player view)
        #[arg(long)]
        dm_only: bool,
        /// Note (may reference entity ids)
        #[arg(long)]
        note: Option<String>,
    },
    /// List events (default: player view; --all is the DM view with [DM])
    List {
        /// Also list dm_only events (DM view)
        #[arg(long)]
        all: bool,
    },
    /// Mark an event done
    Done { id: String },
    /// Skip an event
    Skip { id: String },
}

pub const FILE: &str = "world-time.yaml";
pub const VERSION: u32 = 1;

/// The event statuses (contract §4.5): pending until done or skipped;
/// terminal states refuse further done/skip.
pub const PENDING: &str = "pending";
pub const DONE: &str = "done";
pub const SKIPPED: &str = "skipped";

/// One scheduled event (contract §4.5; field names frozen).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    /// Sequential, never reused: EV-001, EV-002, ...
    pub id: String,
    /// Free-text date label the event comes due on.
    pub due: String,
    pub title: String,
    /// pending | done | skipped
    pub status: String,
    #[serde(default)]
    pub dm_only: bool,
    #[serde(default)]
    pub notes: String,
}

/// `world-time.yaml`, version 1. A missing file is the default
/// (disabled, empty); a newer version is refused like every sidecar.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldTime {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub calendar: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub phase: String,
    /// The only day counter; `time advance` adds to it, nothing else does.
    #[serde(default)]
    pub elapsed_days: f64,
    #[serde(default)]
    pub events: Vec<Event>,
}

fn path(save: &Path) -> PathBuf {
    save.join(FILE)
}

/// Load `world-time.yaml`; a missing file is the disabled default, an
/// invalid file or a newer version is refused naming file and version.
pub fn load(save: &Path) -> Result<WorldTime> {
    let p = path(save);
    if !p.is_file() {
        return Ok(WorldTime::default());
    }
    let wt: WorldTime = serde_norway::from_str(
        &fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?,
    )
    .with_context(|| format!("invalid {}", p.display()))?;
    if wt.version > VERSION {
        bail!(
            "{}: unsupported world-time schema version {} (this talkdnd supports {VERSION}); update talkdnd",
            p.display(),
            wt.version
        );
    }
    Ok(wt)
}

fn store(save: &Path, wt: &WorldTime) -> Result<()> {
    let mut current = wt.clone();
    current.version = VERSION;
    let text = format!(
        "# TalkDND world time: an optional in-world calendar with due-dated events.\n# Off by default; only `talkdnd time ...` advances it; hand edits are reported by `talkdnd check`.\n{}",
        serde_norway::to_string(&current)?
    );
    atomic_write(path(save), text.as_bytes())
}

/// Write commands need the feature on; the file is never created for them.
fn require_enabled(wt: &WorldTime) -> Result<()> {
    if !wt.enabled {
        bail!("世界时间未启用：先 `talkdnd time enable`（未启用时写命令被拒绝，存档未改动）");
    }
    Ok(())
}

/// `EV-1`, `ev001` → `EV-001`; other shapes are not event ids.
fn canonical_id(input: &str) -> Option<String> {
    let s = input.trim();
    let rest = s
        .strip_prefix("EV")
        .or_else(|| s.strip_prefix("ev"))
        .or_else(|| s.strip_prefix("Ev"))?;
    let rest = rest.strip_prefix('-').unwrap_or(rest);
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = rest.parse().ok()?;
    Some(format!("EV-{n:03}"))
}

/// The next sequential event id. Ids are never reused: done/skipped events
/// stay in the list, so the counter keeps rising.
fn next_id(events: &[Event]) -> String {
    let max = events
        .iter()
        .filter_map(|e| e.id.strip_prefix("EV-").and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("EV-{:03}", max + 1)
}

/// Round to 9 decimals so repeated --hours additions cannot drift.
fn round9(v: f64) -> f64 {
    (v * 1e9).round() / 1e9
}

/// At most 3 decimals, trailing zeros and dot trimmed: 1 → "1", 1/24 → "0.042".
fn fmt_days(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn label(s: &str) -> &str {
    if s.is_empty() {
        "（未设置）"
    } else {
        s
    }
}

/// Parse --days/--hours: negative or non-numeric input is a usage error
/// (exit 2, like clap), a value above u32::MAX overflows (exit 1), and a
/// missing flag contributes zero.
fn parse_amount(name: &str, raw: Option<String>) -> Result<u32> {
    let Some(raw) = raw else {
        return Ok(0);
    };
    let s = raw.trim();
    let body = s.strip_prefix('+').unwrap_or(s);
    let (neg, digits) = match body.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, body),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        eprintln!("error: --{name} 需要一个非负整数（收到 `{s}`）");
        std::process::exit(2);
    }
    if neg {
        eprintln!("error: --{name} 不能是负数（收到 `{s}`）");
        std::process::exit(2);
    }
    digits
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("--{name} 的值 {s} 超出 32 位整数范围；存档未改动"))
}

/// One line into the newest session log (skipped when there is none).
fn log_line(save: &Path, line: &str) -> Result<()> {
    if let Some(log) = crate::save::latest_log(save) {
        crate::save::append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(())
}

/// `time show|enable|disable|set|advance|event ...` (P06, TASK-0063).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    let _ = root;
    match cmd {
        Cmd::Show => {
            let wt = load(save)?;
            if wt.enabled {
                println!("世界时间：已启用（`talkdnd time disable` 关闭；关闭保留数据）");
                println!("- 历法：{}", label(&wt.calendar));
                println!("- 日期：{}", label(&wt.date));
                println!("- 时段：{}", label(&wt.phase));
                println!("- 已过天数：{}", fmt_days(wt.elapsed_days));
                let pending = wt.events.iter().filter(|e| e.status == PENDING).count();
                println!(
                    "- 事件：{} 条（待发生 {pending}；`talkdnd time event list` 查看）",
                    wt.events.len()
                );
            } else {
                println!("世界时间：未启用（`talkdnd time enable` 开启；未启用时写命令被拒绝）");
                if wt.version > 0 {
                    println!(
                        "- 已保留的数据：日期 {}，已过天数 {}，事件 {} 条（启用后继续有效）",
                        label(&wt.date),
                        fmt_days(wt.elapsed_days),
                        wt.events.len()
                    );
                }
            }
        }
        Cmd::Enable => {
            let mut wt = load(save)?;
            if wt.enabled {
                println!("世界时间已是启用状态；未改动、未记日志");
                return Ok(());
            }
            wt.enabled = true;
            store(save, &wt)?;
            log_line(save, "世界时间：启用")?;
            println!(
                "世界时间已启用（{}）；`talkdnd time set --date <日期>` 设置当前日期，`time advance` 推进",
                path(save).display()
            );
        }
        Cmd::Disable => {
            let mut wt = load(save)?;
            if !wt.enabled {
                println!("世界时间已是未启用状态；未改动、未记日志");
                return Ok(());
            }
            wt.enabled = false;
            store(save, &wt)?;
            log_line(save, "世界时间：停用（日期与事件数据保留）")?;
            println!(
                "世界时间已停用；日期/事件数据保留在 {}",
                path(save).display()
            );
        }
        Cmd::Set { date, phase } => {
            let mut wt = load(save)?;
            require_enabled(&wt)?;
            let date = date.trim();
            if date.is_empty() {
                bail!("--date 不能为空（去空白后）；存档未改动");
            }
            // No --phase keeps the current one; --phase "" clears it.
            let new_phase = phase.as_deref().map(|p| p.trim().to_string());
            let old_date = wt.date.clone();
            let old_phase = wt.phase.clone();
            let phase_changed = new_phase.as_deref().is_some_and(|p| p != old_phase);
            if old_date == date && !phase_changed {
                println!("日期/时段未变化；未改动、未记日志");
                return Ok(());
            }
            wt.date = date.to_string();
            if let Some(p) = &new_phase {
                wt.phase = p.clone();
            }
            store(save, &wt)?;
            let mut line = format!("世界时间：日期 {} → {}", label(&old_date), label(&wt.date));
            if phase_changed {
                line += &format!("；时段 {} → {}", label(&old_phase), label(&wt.phase));
            }
            log_line(save, &line)?;
            println!("{line}");
        }
        Cmd::Advance { days, hours } => {
            let mut wt = load(save)?;
            require_enabled(&wt)?;
            let d = parse_amount("days", days)?;
            let h = parse_amount("hours", hours)?;
            if d == 0 && h == 0 {
                bail!("--days/--hours 至少一个且合计必须大于 0；存档未改动");
            }
            let add = round9(d as f64 + h as f64 / 24.0);
            let new = round9(wt.elapsed_days + add);
            if new == round9(wt.elapsed_days) {
                println!("已过天数未变化；未改动、未记日志");
                return Ok(());
            }
            let old = wt.elapsed_days;
            wt.elapsed_days = new;
            store(save, &wt)?;
            let line = format!(
                "世界时间：已过天数 {} → {}（+{}）",
                fmt_days(old),
                fmt_days(new),
                fmt_days(add)
            );
            log_line(save, &line)?;
            println!("{line}");
        }
        Cmd::Event { cmd } => match cmd {
            EventCmd::Add {
                due,
                title,
                dm_only,
                note,
            } => {
                let mut wt = load(save)?;
                require_enabled(&wt)?;
                let due = due.trim();
                let title = title.trim();
                if due.is_empty() {
                    bail!("--due 不能为空（去空白后）；存档未改动");
                }
                if title.is_empty() {
                    bail!("--title 不能为空（去空白后）；存档未改动");
                }
                let e = Event {
                    id: next_id(&wt.events),
                    due: due.to_string(),
                    title: title.to_string(),
                    status: PENDING.to_string(),
                    dm_only,
                    notes: note.as_deref().map(str::trim).unwrap_or("").to_string(),
                };
                wt.events.push(e.clone());
                store(save, &wt)?;
                let mark = if dm_only { "［DM］" } else { "" };
                let line = format!(
                    "世界时间：新增事件 {}{mark}（{} 到期：{}）",
                    e.id, e.due, e.title
                );
                log_line(save, &line)?;
                println!("已添加事件 {}{mark}：{}（到期：{}）", e.id, e.title, e.due);
            }
            EventCmd::List { all } => {
                let wt = load(save)?;
                let shown: Vec<&Event> = wt.events.iter().filter(|e| all || !e.dm_only).collect();
                if all {
                    println!(
                        "预定事件（{} 条；DM 视图，[DM] 开头 = 仅 DM 可见）：",
                        shown.len()
                    );
                } else {
                    println!(
                        "预定事件（{} 条；玩家可见的条目，`--all` 看 DM 视图）：",
                        shown.len()
                    );
                }
                if shown.is_empty() {
                    println!("- （还没有事件；`talkdnd time event add --due <日期> --title <标题>` 添加）");
                }
                for e in shown {
                    let mut line = String::new();
                    if all && e.dm_only {
                        line += "[DM] ";
                    }
                    line += &format!("{}：{}（到期：{}；{}）", e.id, e.title, e.due, e.status);
                    if e.due == wt.date && e.status == PENDING {
                        line += " —— 今天到期";
                    }
                    if !e.notes.is_empty() {
                        line += &format!("；备注：{}", e.notes);
                    }
                    println!("- {line}");
                }
            }
            EventCmd::Done { id } => {
                let e = set_status(save, &id, DONE)?;
                let line = format!("世界时间：事件 {} 标记完成（{}）", e.id, e.title);
                log_line(save, &line)?;
                println!("事件 {} 已标记 done：{}", e.id, e.title);
            }
            EventCmd::Skip { id } => {
                let e = set_status(save, &id, SKIPPED)?;
                let line = format!("世界时间：事件 {} 跳过（{}）", e.id, e.title);
                log_line(save, &line)?;
                println!("事件 {} 已标记 skipped：{}", e.id, e.title);
            }
        },
    }
    Ok(())
}

/// done/skip one event: enabled required, unknown ids refused, terminal
/// states refuse re-marking — every refusal leaves the file untouched.
fn set_status(save: &Path, id: &str, status: &str) -> Result<Event> {
    let mut wt = load(save)?;
    require_enabled(&wt)?;
    let Some(cid) = canonical_id(id) else {
        bail!("事件 id 形如 EV-001（收到 `{id}`）；存档未改动");
    };
    {
        let Some(e) = wt.events.iter_mut().find(|e| e.id == cid) else {
            bail!("事件 {cid} 不存在（`talkdnd time event list --all` 看全部 id）；存档未改动");
        };
        if e.status != PENDING {
            bail!("事件 {cid} 已是 {}，不能再标记；存档未改动", e.status);
        }
        e.status = status.to_string();
    }
    store(save, &wt)?;
    let e = wt
        .events
        .into_iter()
        .find(|e| e.id == cid)
        .expect("event was just found");
    Ok(e)
}

/// The `start` digest's world-time section: only when enabled, player view
/// (dm_only events never appear). Reading never writes; a broken file just
/// yields no section (hand edits are reported by `talkdnd check`).
pub fn digest_section(save: &Path) -> Option<String> {
    let wt = load(save).ok()?;
    if !wt.enabled {
        return None;
    }
    let mut s = String::from("\n## 世界时间\n\n");
    s += &format!("- 日期：{}\n", label(&wt.date));
    if !wt.phase.is_empty() {
        s += &format!("- 时段：{}\n", wt.phase);
    }
    s += &format!("- 已过天数：{}\n", fmt_days(wt.elapsed_days));
    let due: Vec<&Event> = wt
        .events
        .iter()
        .filter(|e| !e.dm_only && e.status == PENDING && e.due == wt.date)
        .collect();
    if !due.is_empty() {
        s += "- 今天到期（玩家可见）：\n";
        for e in due {
            s += &format!("  - {}：{}\n", e.id, e.title);
        }
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_save(tag: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("talkdnd-worldtime-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("Sessions")).unwrap();
        fs::write(p.join("Sessions/s.md"), "# log\n").unwrap();
        p
    }

    #[test]
    fn missing_file_is_default_and_store_roundtrips() {
        let save = temp_save("roundtrip");
        let d = load(&save).unwrap();
        assert!(!d.enabled);
        assert_eq!(d.version, 0, "missing file loads as the zero default");
        assert!(d.events.is_empty());
        // No file is created by load.
        assert!(!path(&save).exists());

        let wt = WorldTime {
            version: 1,
            enabled: true,
            calendar: "Forgotten Realms".into(),
            date: "1 Hammer".into(),
            phase: "morning".into(),
            elapsed_days: 3.5,
            events: vec![Event {
                id: "EV-001".into(),
                due: "3 Hammer".into(),
                title: "festival".into(),
                status: PENDING.into(),
                dm_only: true,
                notes: "npc:E-001".into(),
            }],
        };
        store(&save, &wt).unwrap();
        let back = load(&save).unwrap();
        assert_eq!(back, wt);
        let text = fs::read_to_string(path(&save)).unwrap();
        assert!(text.contains("version: 1"), "{text}");
        assert!(text.contains("dm_only: true"), "{text}");

        // An invalid file is refused naming the file.
        fs::write(path(&save), "enabled: [unclosed").unwrap();
        let err = load(&save).unwrap_err().to_string();
        assert!(err.contains("world-time.yaml"), "{err}");
        // A newer version is refused naming file and version.
        fs::write(path(&save), "version: 2\nenabled: true\n").unwrap();
        let err = load(&save).unwrap_err().to_string();
        assert!(
            err.contains("world-time.yaml") && err.contains("2"),
            "{err}"
        );
        let _ = fs::remove_dir_all(&save);
    }

    #[test]
    fn event_ids_are_sequential_and_never_reused() {
        let events = vec![];
        assert_eq!(next_id(&events), "EV-001");
        let events: Vec<Event> = ["EV-001", "EV-002", "EV-003"]
            .into_iter()
            .map(|id| Event {
                id: id.into(),
                due: "d".into(),
                title: "t".into(),
                status: DONE.into(),
                dm_only: false,
                notes: String::new(),
            })
            .collect();
        assert_eq!(next_id(&events), "EV-004");
        // done/skipped events are never deleted by the CLI, so the highest
        // id stays in the file and the counter never goes backwards.
        let events: Vec<Event> = events.into_iter().filter(|e| e.id != "EV-002").collect();
        assert_eq!(next_id(&events), "EV-004");

        assert_eq!(canonical_id("ev-1").as_deref(), Some("EV-001"));
        assert_eq!(canonical_id("EV-012").as_deref(), Some("EV-012"));
        assert_eq!(canonical_id("E-001"), None);
        assert_eq!(canonical_id("festival"), None);
    }

    #[test]
    fn days_formatting_and_rounding() {
        assert_eq!(fmt_days(0.0), "0");
        assert_eq!(fmt_days(1.0), "1");
        assert_eq!(fmt_days(0.5), "0.5");
        assert_eq!(fmt_days(1.0 / 24.0), "0.042");
        assert_eq!(fmt_days(3.125), "3.125");
        // 24 one-hour steps round to 1 day within 1e-6.
        let mut d = 0.0;
        for _ in 0..24 {
            d = round9(d + 1.0 / 24.0);
        }
        assert!((d - 1.0).abs() < 1e-6, "{d}");
        assert_eq!(fmt_days(d), "1");

        assert_eq!(parse_amount("days", None).unwrap(), 0);
        assert_eq!(parse_amount("days", Some(" 7 ".into())).unwrap(), 7);
        assert_eq!(parse_amount("hours", Some("+3".into())).unwrap(), 3);
        assert!(parse_amount("days", Some("4294967296".into())).is_err());
        assert!(parse_amount("days", Some("99999999999999999999999".into())).is_err());
    }

    #[test]
    fn digest_section_needs_enabled_and_filters_dm_only() {
        let save = temp_save("digest");
        assert!(digest_section(&save).is_none(), "missing file = no section");
        let wt = WorldTime {
            version: 1,
            enabled: false,
            date: "1 Hammer".into(),
            ..WorldTime::default()
        };
        store(&save, &wt).unwrap();
        assert!(digest_section(&save).is_none(), "disabled = no section");

        let wt = WorldTime {
            version: 1,
            enabled: true,
            calendar: String::new(),
            date: "1 Hammer".into(),
            phase: "morning".into(),
            elapsed_days: 2.5,
            events: vec![
                Event {
                    id: "EV-001".into(),
                    due: "1 Hammer".into(),
                    title: "visible parade".into(),
                    status: PENDING.into(),
                    dm_only: false,
                    notes: String::new(),
                },
                Event {
                    id: "EV-002".into(),
                    due: "1 Hammer".into(),
                    title: "secret ambush".into(),
                    status: PENDING.into(),
                    dm_only: true,
                    notes: String::new(),
                },
                Event {
                    id: "EV-003".into(),
                    due: "1 Hammer".into(),
                    title: "finished chore".into(),
                    status: DONE.into(),
                    dm_only: false,
                    notes: String::new(),
                },
                Event {
                    id: "EV-004".into(),
                    due: "2 Hammer".into(),
                    title: "later".into(),
                    status: PENDING.into(),
                    dm_only: false,
                    notes: String::new(),
                },
            ],
        };
        store(&save, &wt).unwrap();
        let sec = digest_section(&save).unwrap();
        assert!(sec.contains("## 世界时间"), "{sec}");
        assert!(sec.contains("日期：1 Hammer"), "{sec}");
        assert!(sec.contains("时段：morning"), "{sec}");
        assert!(sec.contains("已过天数：2.5"), "{sec}");
        assert!(sec.contains("EV-001：visible parade"), "{sec}");
        assert!(!sec.contains("secret ambush"), "{sec}");
        assert!(!sec.contains("EV-002"), "{sec}");
        assert!(!sec.contains("finished chore"), "{sec}");
        assert!(!sec.contains("EV-004"), "{sec}");

        // Without due events only the date/phase/day lines remain.
        let wt2 = WorldTime {
            date: "9 Hammer".into(),
            ..wt.clone()
        };
        store(&save, &wt2).unwrap();
        let sec = digest_section(&save).unwrap();
        assert!(sec.contains("日期：9 Hammer"), "{sec}");
        assert!(!sec.contains("今天到期"), "{sec}");
        let _ = fs::remove_dir_all(&save);
    }
}
