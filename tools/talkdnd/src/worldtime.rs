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
//! the DM view with a `[DM]` marker, the `start` digest shows only
//! player-visible due events, and `time show` counts only player-visible
//! events (dm_only never changes its totals). The digest here is
//! read-only: `start` never writes world-time.yaml.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use combat_sim::campaign_time::{SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE};
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Show enabled/calendar/date/phase/elapsed_days and the player-visible
    /// event count (dm_only events are never counted)
    Show,
    /// Enable world time
    ///
    /// 存档启用成长协议后，本命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写任何目标文件。没有协议文件的存档保持既有行为，不自动初始化。
    Enable,
    /// Disable world time (date/phase/events/elapsed_days are kept)
    ///
    /// 存档启用成长协议后，本命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写任何目标文件。没有协议文件的存档保持既有行为，不自动初始化。
    Disable,
    /// Set the date label (and optional phase); refused while disabled
    ///
    /// 存档启用成长协议后，本命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写任何目标文件。没有协议文件的存档保持既有行为，不自动初始化。
    Set {
        /// New date label
        #[arg(long)]
        date: String,
        /// New phase label (--phase "" clears it)
        #[arg(long)]
        phase: Option<String>,
    },
    /// Advance the clock by days, hours, minutes or seconds.
    ///
    /// 存档启用成长协议后，本命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写任何目标文件。没有协议文件的存档保持既有行为，不自动初始化。
    Advance {
        /// Days to add
        #[arg(long, value_name = "N")]
        days: Option<String>,
        /// Hours to add (h/24 days)
        #[arg(long, value_name = "N")]
        hours: Option<String>,
        /// Minutes to add, using the exact campaign clock
        #[arg(long, value_name = "N")]
        minutes: Option<String>,
        /// Seconds to add, using the exact campaign clock
        #[arg(long, value_name = "N")]
        seconds: Option<String>,
    },
    /// Track a minute- or hour-long spell across exploration and combat.
    Casting {
        #[command(subcommand)]
        cmd: CastingCmd,
    },
    /// Due-dated events
    Event {
        #[command(subcommand)]
        cmd: EventCmd,
    },
}

#[derive(Subcommand, Clone)]
pub enum CastingCmd {
    /// Begin a long casting during exploration.
    Start {
        caster: String,
        spell: String,
        #[arg(long)]
        material: Option<String>,
    },
    /// Show casting progress without changing the save.
    Status,
    /// Attach active castings to an encounter that has rolled initiative.
    Enter { encounter: String },
    /// Reconcile the encounter clock and resume exploration.
    Leave,
    /// Pay once for a completed exploration casting and confirm its effects.
    Finish {
        id: String,
        #[arg(long)]
        confirm_manual: bool,
    },
    /// Interrupt an exploration casting without spending a slot.
    Interrupt { id: String },
}

#[derive(Subcommand, Clone)]
pub enum EventCmd {
    /// Add a due-dated event
    ///
    /// 存档启用成长协议后，本命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写任何目标文件。没有协议文件的存档保持既有行为，不自动初始化。
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
    ///
    /// 存档启用成长协议后，本命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写任何目标文件。没有协议文件的存档保持既有行为，不自动初始化。
    Done { id: String },
    /// Skip an event
    ///
    /// 存档启用成长协议后，本命令在写目标前取得 SAVE 锁；SAVE state 缺失、不可写、active 或 in_doubt 时以退出码 1 拒绝且不写任何目标文件。没有协议文件的存档保持既有行为，不自动初始化。
    Skip { id: String },
}

pub use combat_sim::campaign_time::{Event, WorldTime, FILE, VERSION};

pub const PENDING: &str = "pending";
pub const DONE: &str = "done";
pub const SKIPPED: &str = "skipped";

fn path(save: &Path) -> PathBuf {
    save.join(FILE)
}

pub fn load(save: &Path) -> Result<WorldTime> {
    WorldTime::load(save)
}

fn store(save: &Path, wt: &WorldTime) -> Result<()> {
    wt.store(save)
}

/// Write commands need the feature on; the file is never created for them.
fn require_enabled(wt: &WorldTime) -> Result<()> {
    if !wt.enabled {
        bail!("世界时间未启用：先 `talkdnd time enable`（未启用时写命令被拒绝，存档未改动）");
    }
    Ok(())
}

/// Read-only due audit for `talkdnd check` (contract §3.4): pending events
/// whose `due` label equals the current `date` label. When the feature is
/// disabled but such events exist, they are still reported with a note — a
/// hint only; nothing is changed, rescheduled or created.
pub fn audit_due(wt: &WorldTime) -> Vec<String> {
    if wt.date.is_empty() {
        return Vec::new();
    }
    let note = if wt.enabled {
        String::new()
    } else {
        "；世界时间未启用".to_string()
    };
    wt.events
        .iter()
        .filter(|e| e.status == PENDING && e.due == wt.date)
        .map(|e| {
            format!(
                "世界时间事件 {} 今日到期（due {}，pending）{note}",
                e.id, e.due
            )
        })
        .collect()
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

fn casting_spell(
    save: &Path,
    caster: &str,
    spell: &str,
    material: Option<String>,
) -> Result<(
    combat_sim::content::schema::Creature,
    combat_sim::long_cast::CastRecipe,
)> {
    use combat_sim::content::schema::CreatureKind;
    let content = combat_sim::content::ContentSources {
        campaign: Some(save.join("combat/campaign.yaml")),
        ..Default::default()
    }
    .load()?;
    let creature = content.resolve_creature(caster)?;
    let party = creature
        .side
        .as_ref()
        .map_or(creature.kind() == CreatureKind::Pc, |side| {
            side.as_str() == "party"
        });
    if !party || !matches!(creature.kind(), CreatureKind::Pc | CreatureKind::Npc) {
        bail!("long casting needs an exact party character ID");
    }
    let ability = combat_sim::state::find_creature_ability(&creature, &content, spell)?;
    let recipe = combat_sim::long_cast::CastRecipe::from_ability(
        caster,
        &content.ruleset,
        &ability.ability,
        material,
    )?;
    Ok((creature, recipe))
}

fn attached_progress(
    save: &Path,
    ledger: &combat_sim::long_cast::CastingLedger,
) -> Result<(
    combat_sim::state::EncounterState,
    combat_sim::long_cast::EncounterCasting,
)> {
    use combat_sim::long_cast::Handoff;
    use combat_sim::state::EncounterState;
    let handoff = ledger
        .handoff
        .as_ref()
        .context("no long casting is attached to combat")?;
    let path = Handoff::encounter_path(save, &handoff.encounter_file)?;
    let state = EncounterState::load(&path)?;
    let battle = match state.casting.as_ref() {
        Some(casting) if casting.handoff_id == handoff.id => casting.clone(),
        _ if state.round == handoff.start_round
            && Handoff::fingerprint(&path)? == handoff.before_digest =>
        {
            ledger.encounter_progress()?
        }
        _ => bail!("the encounter no longer matches the recorded long casting handoff"),
    };
    ledger.validate_encounter(&battle, state.round)?;
    Ok((state, battle))
}

fn run_casting(save: &Path, cmd: CastingCmd) -> Result<()> {
    use combat_sim::long_cast::{CastStatus, Handoff};
    use combat_sim::party::{Member, PartyState, FILE_NAME};
    use combat_sim::state::{EncounterState, STATE_SUFFIX};

    let mut world = load(save)?;
    if matches!(cmd, CastingCmd::Status) {
        let Some(clock) = &world.clock else {
            println!("没有长施法记录");
            return Ok(());
        };
        let casts = if clock.casting.handoff.is_some() {
            attached_progress(save, &clock.casting)?
                .1
                .casts
                .into_iter()
                .map(|(id, cast)| (id, cast.progress))
                .collect()
        } else {
            clock.casting.casts.clone()
        };
        for (id, cast) in casts {
            println!(
                "{id} {} {} {}/{} 秒 {}",
                cast.recipe.caster_id,
                cast.recipe.spell_id,
                cast.elapsed_seconds,
                cast.recipe.required_seconds,
                cast.phase_name(),
            );
        }
        return Ok(());
    }

    require_enabled(&world)?;
    world.enable_second_clock()?;
    let clock = world.clock.as_mut().expect("exact clock");
    let party_path = save.join("combat").join(FILE_NAME);
    let line = match cmd {
        CastingCmd::Start {
            caster,
            spell,
            material,
        } => {
            clock.casting.require_exploration()?;
            let (creature, recipe) = casting_spell(save, &caster, &spell, material)?;
            let party = PartyState::load(&party_path)?;
            let member = party
                .members
                .get(&caster)
                .cloned()
                .unwrap_or_else(|| Member::from_creature(&creature));
            if member.dead || member.hp <= 0 {
                bail!("施法者无法开始长施法");
            }
            if let Some(cost) = &recipe.cost {
                let amount = i32::try_from(cost.amount).context("invalid long spell cost")?;
                if member
                    .resources
                    .get(&cost.resource)
                    .is_none_or(|resource| resource.current < amount)
                {
                    bail!("施法者缺少该法术所需的资源");
                }
            }
            let id = combat_sim::long_cast::new_id();
            let line = format!(
                "长施法 {id}：{caster} 开始 {}（{} 秒）",
                recipe.spell_id, recipe.required_seconds
            );
            clock.casting.begin(id, recipe)?;
            line
        }
        CastingCmd::Enter { encounter } => {
            let filename = if encounter == combat_sim::state::DEFAULT_STATE_FILE
                || encounter.ends_with(STATE_SUFFIX)
            {
                encounter
            } else {
                format!("{encounter}{STATE_SUFFIX}")
            };
            let path = Handoff::encounter_path(save, &filename)?;
            let state = EncounterState::load(&path)?;
            if state.round == 0
                || state.current_index().is_none()
                || state.outcome.is_some()
                || state.awaiting_reaction()
            {
                bail!("交接需要已掷先攻、进行中且没有待确认反应的遭遇");
            }
            let saved_party = state
                .party_state
                .as_deref()
                .context("the encounter has no campaign party-state")?;
            if saved_party.file_name() != party_path.file_name()
                || saved_party
                    .parent()
                    .map(std::fs::canonicalize)
                    .transpose()?
                    != party_path.parent().map(std::fs::canonicalize).transpose()?
            {
                bail!("the encounter belongs to a different party state");
            }
            let party = PartyState::load(&party_path)?;
            if clock.casting.casts.iter().any(|(id, cast)| {
                cast.status == CastStatus::Casting && party.completed_casts.contains(id)
            }) {
                bail!("长施法资源已经扣除，请先重试完成以同步战役状态");
            }
            party.verify_encounter_resources(&state)?;
            clock.casting.enter(Handoff {
                id: combat_sim::long_cast::new_id(),
                encounter_file: filename,
                before_digest: Handoff::fingerprint(&path)?,
                start_round: state.round,
            })?;
            let mut trial = state;
            combat_sim::engine::attach_long_casts(
                &mut trial,
                clock.casting.encounter_progress()?,
                &mut Vec::new(),
            )?;
            "长施法进度已交接给遭遇".to_string()
        }
        CastingCmd::Leave => {
            let (state, battle) = attached_progress(save, &clock.casting)?;
            if state.awaiting_reaction() {
                bail!("先处理遭遇中的待确认反应，再回到探索");
            }
            let elapsed = world.reconcile_combat(&battle, state.round)?;
            let mut party = PartyState::load(&party_path)?;
            if battle.casts.iter().any(|(id, cast)| {
                cast.progress.status == CastStatus::Completed && !party.completed_casts.contains(id)
            }) {
                party.record(&state);
                party.save(&party_path)?;
            }
            format!("长施法进度已回到探索，战斗计入 {elapsed} 秒")
        }
        CastingCmd::Finish { id, confirm_manual } => {
            clock.casting.require_exploration()?;
            let cast = clock
                .casting
                .casts
                .get_mut(&id)
                .context("unknown long casting ID")?;
            if !cast.ready_to_complete() {
                bail!("该长施法未完成持续时间，或已经结束");
            }
            if !confirm_manual {
                bail!("先用 --confirm-manual 确认由 DM 结算法术效果");
            }
            let (creature, recipe) = casting_spell(
                save,
                &cast.recipe.caster_id,
                &cast.recipe.spell_id,
                cast.recipe.material.clone(),
            )?;
            if recipe != cast.recipe {
                bail!("施法记录的规则或资源已变更");
            }
            let mut party = PartyState::load(&party_path)?;
            party
                .members
                .entry(recipe.caster_id.clone())
                .or_insert_with(|| Member::from_creature(&creature));
            party.settle_long_cast(&id, &recipe)?;
            party.save(&party_path)?;
            cast.status = CastStatus::Completed;
            format!("MANUAL CONFIRMED：长施法 {id} 完成")
        }
        CastingCmd::Interrupt { id } => {
            clock.casting.require_exploration()?;
            let cast = clock
                .casting
                .casts
                .get_mut(&id)
                .context("unknown long casting ID")?;
            if cast.status != CastStatus::Casting {
                bail!("该长施法已经结束");
            }
            if PartyState::load(&party_path)?.completed_casts.contains(&id) {
                bail!("该长施法已扣除资源，请重试完成以同步状态");
            }
            cast.status = CastStatus::Interrupted;
            format!("长施法 {id} 已中断，未消耗法术资源")
        }
        CastingCmd::Status => unreachable!("status is read-only"),
    };
    store(save, &world)?;
    log_line(save, &line)?;
    println!("{line}");
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
                if let Some(clock) = &wt.clock {
                    let mut seconds = clock.elapsed_seconds;
                    if clock.casting.handoff.is_some() {
                        let (state, battle) = attached_progress(save, &clock.casting)?;
                        seconds = seconds
                            .checked_add(battle.elapsed_seconds(state.round)?)
                            .context("campaign seconds overflow")?;
                    }
                    println!(
                        "- 已过天数：{}",
                        fmt_days(combat_sim::campaign_time::as_days(seconds))
                    );
                    println!("- 已过秒数：{}", seconds);
                } else {
                    println!("- 已过天数：{}", fmt_days(wt.elapsed_days));
                }
                // Player view (contract §6.1): dm_only events stay out of
                // both totals, so the counts cannot leak their existence.
                let visible = wt.events.iter().filter(|e| !e.dm_only).count();
                let pending = wt
                    .events
                    .iter()
                    .filter(|e| !e.dm_only && e.status == PENDING)
                    .count();
                println!(
                    "- 事件：{} 条（待发生 {pending}；`talkdnd time event list` 查看）",
                    visible
                );
            } else {
                println!("世界时间：未启用（`talkdnd time enable` 开启；未启用时写命令被拒绝）");
                if wt.version > 0 {
                    // Same player-view rule: dm_only events are not counted.
                    let visible = wt.events.iter().filter(|e| !e.dm_only).count();
                    println!(
                        "- 已保留的数据：日期 {}，已过天数 {}，事件 {} 条（启用后继续有效）",
                        label(&wt.date),
                        fmt_days(wt.elapsed_days),
                        visible
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
            if wt.clock.as_ref().is_some_and(|clock| {
                clock.casting.handoff.is_some()
                    || clock
                        .casting
                        .casts
                        .values()
                        .any(|cast| cast.status == combat_sim::long_cast::CastStatus::Casting)
            }) {
                bail!("世界时间支撑进行中的长施法，请先结束交接与施法");
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
        Cmd::Advance {
            days,
            hours,
            minutes,
            seconds,
        } => {
            let mut wt = load(save)?;
            require_enabled(&wt)?;
            let exact = wt.clock.is_some() || minutes.is_some() || seconds.is_some();
            let d = u64::from(parse_amount("days", days)?);
            let h = u64::from(parse_amount("hours", hours)?);
            let m = u64::from(parse_amount("minutes", minutes)?);
            let s = u64::from(parse_amount("seconds", seconds)?);
            let elapsed = d * SECONDS_PER_DAY + h * SECONDS_PER_HOUR + m * SECONDS_PER_MINUTE + s;
            if elapsed == 0 {
                bail!("--days/--hours/--minutes/--seconds 至少一个且合计必须大于 0；存档未改动");
            }
            let old = wt.elapsed_days;
            let line = if exact {
                wt.advance_seconds(elapsed)?;
                format!(
                    "世界时间：已过 {} 秒（新增 {} 秒）",
                    wt.clock.as_ref().expect("exact clock").elapsed_seconds,
                    elapsed,
                )
            } else {
                let add = round9(d as f64 + h as f64 / 24.0);
                let new = round9(old + add);
                if new == round9(old) {
                    println!("已过天数未变化；未改动、未记日志");
                    return Ok(());
                }
                wt.elapsed_days = new;
                format!(
                    "世界时间：已过天数 {} → {}（+{}）",
                    fmt_days(old),
                    fmt_days(new),
                    fmt_days(add)
                )
            };
            store(save, &wt)?;
            log_line(save, &line)?;
            println!("{line}");
        }
        Cmd::Casting { cmd } => run_casting(save, cmd)?,
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
            clock: None,
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
            clock: None,
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
