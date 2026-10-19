//! Command-line interface for SPEC §4.
//!
//! Commands load the persisted encounter, call the rules engine with a capture
//! log, then save the resulting state and print the new log lines. `status` and
//! `log` are read-only; `roll` does not need an encounter at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::ai::Ai;
use crate::dice::{self, DiceRng, RollMode};
use crate::engine::{self, AttackOptions, CheckOptions, ConditionOptions};
use crate::model::{Kind, Resource, Side};
use crate::sim::{self, MonteCarloReport, RunOutcome, SimOptions, SingleRun};
use crate::state::{ActiveCondition, CombatantState, DeathSaves, EncounterState, LookupError};

/// Global options shared by every command.
#[derive(Debug, Parser)]
#[command(
    name = "combat-sim",
    version,
    about = "Headless D&D 5e (2024 rules) combat helper"
)]
pub struct Cli {
    /// Path to the persisted encounter state.
    #[arg(
        long,
        global = true,
        value_name = "FILE",
        default_value = "./combat-state.json"
    )]
    pub state: PathBuf,

    /// Seed for deterministic rolls (tests and replays only).
    #[arg(long, global = true, value_name = "N")]
    pub seed: Option<u64>,

    /// Emit machine-readable JSON where supported.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// One CLI action.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new encounter state.
    New {
        /// Encounter JSON file (mutually exclusive with positional specs).
        #[arg(long, value_name = "FILE", conflicts_with = "specs")]
        encounter: Option<PathBuf>,

        /// Combatant specs: `file.json[:count][@zone]`.
        #[arg(value_name = "SPEC")]
        specs: Vec<String>,
    },

    /// Roll initiative.
    Init {
        /// Fixed initiative total for one combatant, repeatable.
        #[arg(long = "set", value_name = "NAME=TOTAL")]
        set: Vec<String>,
    },

    /// Show the current encounter.
    Status,

    /// Resolve one attack (or the attacker's multiattack sequence).
    Attack {
        attacker: String,
        target: String,
        attack: String,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
        #[arg(long)]
        no_sneak: bool,
        #[arg(long)]
        multi: bool,
        #[arg(long)]
        no_reactions: bool,
    },

    /// Apply damage from an expression or flat number.
    Damage {
        target: String,
        expression: String,
        #[arg(long = "type", value_name = "T")]
        damage_type: Option<String>,
        #[arg(long)]
        crit: bool,
    },

    /// Heal a creature.
    Heal {
        target: String,
        expression: String,
        /// Creature whose healing is being applied (logging only).
        #[arg(long = "from", value_name = "NAME")]
        from: Option<String>,
    },

    /// Roll a saving throw.
    Save {
        who: String,
        ability: String,
        dc: i32,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
    },

    /// Roll a skill or ability check.
    Check {
        who: String,
        skill_or_ability: String,
        dc: i32,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
        #[arg(long, default_value_t = 0)]
        bonus: i32,
    },

    /// Roll an arbitrary dice expression.
    Roll {
        expression: String,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
        #[arg(long)]
        dc: Option<i32>,
    },

    /// Cast a named attack, save or heal entry.
    Cast {
        who: String,
        name: String,
        targets: Vec<String>,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
    },

    /// Spend a resource, or run a self-heal entry.
    Use {
        who: String,
        name: String,
        #[arg(long)]
        amount: Option<u32>,
    },

    /// Add or remove a condition.
    Condition {
        #[command(subcommand)]
        command: ConditionCommand,
    },

    /// Move a combatant to another zone.
    Move { who: String, zone: String },

    /// Roll a death saving throw.
    Deathsave { who: String },

    /// End the current turn and start the next one.
    Next,

    /// Print the encounter log.
    Log {
        /// Print only the last N lines.
        #[arg(long)]
        tail: Option<usize>,
    },

    /// Apply a short or long rest.
    Rest {
        #[arg(value_enum)]
        kind: RestKindArg,
        /// Combatant name or `all`.
        who: String,
    },

    /// Batch simulation: play-by-play with one run, Monte Carlo with more.
    Simulate {
        /// Combatant specs: `file.json[:count][@zone]`.
        #[arg(value_name = "SPEC")]
        specs: Vec<String>,

        /// Encounter JSON file.
        #[arg(long, value_name = "FILE", conflicts_with = "specs")]
        encounter: Option<PathBuf>,

        /// Continue from the persisted state file.
        #[arg(long, conflicts_with_all = ["specs", "encounter"])]
        from_state: bool,

        /// Number of simulated runs.
        #[arg(long, default_value_t = 1)]
        runs: u64,

        /// Abort a run after this many rounds.
        #[arg(long, default_value_t = 100)]
        max_rounds: u32,

        /// Worker threads for the Monte Carlo path.
        #[arg(long)]
        threads: Option<usize>,
    },
}

/// Condition subcommands.
#[derive(Debug, Subcommand)]
pub enum ConditionCommand {
    Add {
        who: String,
        condition: String,
        #[arg(long, conflicts_with = "until_end_of_turn")]
        rounds: Option<u32>,
        #[arg(long = "until-end-of-turn", value_name = "NAME")]
        until_end_of_turn: Option<String>,
        #[arg(long)]
        source: Option<String>,
    },
    Remove {
        who: String,
        condition: String,
    },
}

/// Rest length accepted by clap.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RestKindArg {
    Short,
    Long,
}

impl From<RestKindArg> for engine::RestKind {
    fn from(kind: RestKindArg) -> Self {
        match kind {
            RestKindArg::Short => engine::RestKind::Short,
            RestKindArg::Long => engine::RestKind::Long,
        }
    }
}

/// Parse the command line and dispatch the requested command.
pub fn run() -> Result<()> {
    let Cli {
        state,
        seed,
        json,
        command,
    } = Cli::parse();

    match command {
        Command::New { encounter, specs } => new_state(&state, encounter.as_deref(), &specs),
        Command::Init { set } => run_init(&state, seed, &set),
        Command::Status => print_status(&state, json),
        Command::Attack {
            attacker,
            target,
            attack,
            adv,
            dis,
            no_sneak,
            multi,
            no_reactions,
        } => run_state_command(&state, seed, |state, rng, lines| {
            let attacker = canonical_name(state, &attacker)?;
            let target = canonical_name(state, &target)?;
            let options = AttackOptions {
                advantage: adv,
                disadvantage: dis,
                no_sneak,
                no_reactions,
                multi,
            };
            engine::attack(state, &attacker, &target, &attack, &options, rng, lines)?;
            Ok(())
        }),
        Command::Damage {
            target,
            expression,
            damage_type,
            crit,
        } => run_state_command(&state, seed, |state, rng, lines| {
            let target = canonical_name(state, &target)?;
            engine::damage(
                state,
                &target,
                &expression,
                damage_type.as_deref(),
                crit,
                rng,
                lines,
            )?;
            Ok(())
        }),
        Command::Heal {
            target,
            expression,
            from,
        } => run_state_command(&state, seed, |state, rng, lines| {
            let target = canonical_name(state, &target)?;
            let from = from
                .as_deref()
                .map(|name| canonical_name(state, name))
                .transpose()?;
            engine::heal(state, &target, &expression, from.as_deref(), rng, lines)?;
            Ok(())
        }),
        Command::Save {
            who,
            ability,
            dc,
            adv,
            dis,
        } => run_state_command(&state, seed, |state, rng, lines| {
            let who = canonical_name(state, &who)?;
            let mode = RollMode::from_flags(adv, dis);
            engine::save(state, &who, &ability, dc, mode, rng, lines)?;
            Ok(())
        }),
        Command::Check {
            who,
            skill_or_ability,
            dc,
            adv,
            dis,
            bonus,
        } => run_state_command(&state, seed, |state, rng, lines| {
            let who = canonical_name(state, &who)?;
            let options = CheckOptions {
                mode: RollMode::from_flags(adv, dis),
                bonus,
            };
            engine::check(state, &who, &skill_or_ability, dc, &options, rng, lines)?;
            Ok(())
        }),
        Command::Roll {
            expression,
            adv,
            dis,
            dc,
        } => run_state_command(&state, seed, |_state, rng, lines| {
            engine::roll_expression(&expression, RollMode::from_flags(adv, dis), dc, rng, lines)?;
            Ok(())
        }),
        Command::Cast {
            who,
            name,
            targets,
            adv,
            dis,
        } => run_state_command(&state, seed, |state, rng, lines| {
            let who = canonical_name(state, &who)?;
            let targets = targets
                .iter()
                .map(|target| canonical_name(state, target))
                .collect::<Result<Vec<_>>>()?;
            let options = AttackOptions {
                advantage: adv,
                disadvantage: dis,
                ..AttackOptions::default()
            };
            engine::cast(state, &who, &name, &targets, &options, rng, lines)?;
            Ok(())
        }),
        Command::Use { who, name, amount } => {
            run_state_command(&state, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                engine::use_entry(state, &who, &name, amount, rng, lines)?;
                Ok(())
            })
        }
        Command::Condition { command } => match command {
            ConditionCommand::Add {
                who,
                condition,
                rounds,
                until_end_of_turn,
                source,
            } => run_state_command(&state, seed, |state, _rng, lines| {
                let who = canonical_name(state, &who)?;
                let source = source
                    .as_deref()
                    .map(|name| canonical_name(state, name))
                    .transpose()?;
                let until_end_of_turn = until_end_of_turn
                    .as_deref()
                    .map(|name| canonical_name(state, name))
                    .transpose()?;
                let options = ConditionOptions {
                    rounds,
                    until_end_of_turn,
                    source,
                };
                engine::add_condition(state, &who, &condition, &options, lines)?;
                Ok(())
            }),
            ConditionCommand::Remove { who, condition } => {
                run_state_command(&state, seed, |state, _rng, lines| {
                    let who = canonical_name(state, &who)?;
                    engine::remove_condition(state, &who, &condition, lines)?;
                    Ok(())
                })
            }
        },
        Command::Move { who, zone } => run_state_command(&state, seed, |state, _rng, lines| {
            let who = canonical_name(state, &who)?;
            engine::move_to(state, &who, &zone, lines)?;
            Ok(())
        }),
        Command::Deathsave { who } => run_state_command(&state, seed, |state, rng, lines| {
            let who = canonical_name(state, &who)?;
            engine::death_save(state, &who, rng, lines)?;
            Ok(())
        }),
        Command::Next => run_state_command(&state, seed, |state, rng, lines| {
            engine::next_turn(state, rng, lines)?;
            if let Some(index) = state.current_index() {
                lines.push(combatant_line(state, index, true));
            }
            Ok(())
        }),
        Command::Log { tail } => print_log(&state, tail),
        Command::Rest { kind, who } => run_state_command(&state, seed, |state, _rng, lines| {
            let who = if who.eq_ignore_ascii_case("all") {
                "all".to_string()
            } else {
                canonical_name(state, &who)?
            };
            engine::rest(state, kind.into(), Some(&who), lines)?;
            Ok(())
        }),
        Command::Simulate {
            specs,
            encounter,
            runs,
            max_rounds,
            threads,
            ..
        } => simulate(
            &state,
            json,
            &specs,
            encounter.as_deref(),
            SimOptions {
                runs,
                max_rounds,
                seed,
                threads,
            },
        ),
    }
}

// ---------------------------------------------------------------------------
// State loading/saving and name resolution
// ---------------------------------------------------------------------------

fn make_rng(seed: Option<u64>) -> Box<dyn DiceRng> {
    match seed {
        Some(seed) => Box::new(dice::seeded_rng(seed)),
        None => Box::new(dice::step_rng()),
    }
}

fn run_state_command<F>(state_path: &Path, seed: Option<u64>, action: F) -> Result<()>
where
    F: FnOnce(&mut EncounterState, &mut dyn DiceRng, &mut Vec<String>) -> Result<()>,
{
    let mut state = EncounterState::load(state_path)
        .with_context(|| format!("failed to load state {}", state_path.display()))?;
    let mut lines = Vec::new();
    let mut rng = make_rng(seed);
    action(&mut state, rng.as_mut(), &mut lines)?;

    state.log.extend(lines.iter().cloned());
    state
        .save(state_path)
        .with_context(|| format!("failed to save state {}", state_path.display()))?;
    print_lines(&lines);
    Ok(())
}

fn new_state(state_path: &Path, encounter: Option<&Path>, specs: &[String]) -> Result<()> {
    let mut state = match encounter {
        Some(path) => EncounterState::from_encounter_file(path)
            .with_context(|| format!("failed to load encounter {}", path.display()))?,
        None => {
            if specs.is_empty() {
                bail!("provide at least one combatant spec or --encounter");
            }
            EncounterState::from_specs(specs)?
        }
    };

    let name = state.name.as_deref().unwrap_or("encounter");
    let line = format!("Created {name} with {} combatants", state.combatants.len());
    state.log.push(line.clone());
    state
        .save(state_path)
        .with_context(|| format!("failed to save state {}", state_path.display()))?;
    println!("{line}");
    Ok(())
}

fn resolve_name(state: &EncounterState, query: &str) -> Result<usize> {
    state.find_index(query).map_err(|error| match error {
        LookupError::NotFound { .. } => anyhow!(
            "no combatant matches `{query}`; known combatants: {}",
            known_combatants(state)
        ),
        LookupError::Ambiguous { candidates, .. } => anyhow!(
            "combatant name `{query}` is ambiguous; matches: {}",
            candidates.join(", ")
        ),
    })
}

fn canonical_name(state: &EncounterState, query: &str) -> Result<String> {
    let index = resolve_name(state, query)?;
    Ok(state.combatants[index].name().to_string())
}

fn known_combatants(state: &EncounterState) -> String {
    if state.combatants.is_empty() {
        "(none)".to_string()
    } else {
        state
            .combatants
            .iter()
            .map(|combatant| combatant.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// ---------------------------------------------------------------------------
// Commands that do not mutate state
// ---------------------------------------------------------------------------

fn print_status(state_path: &Path, json: bool) -> Result<()> {
    let state = EncounterState::load(state_path)
        .with_context(|| format!("failed to load state {}", state_path.display()))?;
    if json {
        let output = StatusOutput::from_state(&state);
        println!(
            "{}",
            serde_json::to_string_pretty(&output).context("failed to serialize status JSON")?
        );
    } else {
        for line in status_lines(&state) {
            println!("{line}");
        }
    }
    Ok(())
}

fn print_log(state_path: &Path, tail: Option<usize>) -> Result<()> {
    let state = EncounterState::load(state_path)
        .with_context(|| format!("failed to load state {}", state_path.display()))?;
    let start = match tail {
        Some(tail) => state.log.len().saturating_sub(tail),
        None => 0,
    };
    for line in &state.log[start..] {
        println!("{line}");
    }
    Ok(())
}

fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

// ---------------------------------------------------------------------------
// Status rendering
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct StatusOutput {
    round: u32,
    turn: usize,
    active: Option<String>,
    combatants: Vec<CombatantStatus>,
}

#[derive(Debug, Serialize)]
struct CombatantStatus {
    name: String,
    side: Side,
    kind: Kind,
    initiative: Option<i32>,
    current: bool,
    hp: i32,
    max_hp: i32,
    temp_hp: i32,
    ac: i32,
    zone: String,
    conditions: Vec<String>,
    resources: BTreeMap<String, Resource>,
    death_saves: Option<DeathSaves>,
}

impl StatusOutput {
    fn from_state(state: &EncounterState) -> Self {
        let current = state.current_index();
        let combatants = status_order(state)
            .into_iter()
            .map(|index| {
                let combatant = &state.combatants[index];
                CombatantStatus {
                    name: combatant.name().to_string(),
                    side: combatant.side(),
                    kind: combatant.kind(),
                    initiative: combatant.initiative_total,
                    current: current == Some(index),
                    hp: combatant.hp,
                    max_hp: combatant.spec.max_hp,
                    temp_hp: combatant.temp_hp,
                    ac: combatant.ac(),
                    zone: combatant.zone.clone(),
                    conditions: combatant.conditions.iter().map(condition_display).collect(),
                    resources: combatant.resources.clone(),
                    death_saves: if combatant.kind() == Kind::Pc {
                        Some(combatant.death_saves)
                    } else {
                        None
                    },
                }
            })
            .collect();
        Self {
            round: state.round,
            turn: state.turn,
            active: state
                .current()
                .map(|combatant| combatant.name().to_string()),
            combatants,
        }
    }
}

fn status_lines(state: &EncounterState) -> Vec<String> {
    let mut lines = Vec::with_capacity(state.combatants.len() + 1);
    match state.current() {
        Some(combatant) => lines.push(format!(
            "Round {} — {}'s turn",
            state.round,
            combatant.name()
        )),
        None => lines.push(format!("Round {} — initiative not rolled", state.round)),
    }
    let current = state.current_index();
    for index in status_order(state) {
        lines.push(combatant_line(state, index, current == Some(index)));
    }
    lines
}

fn status_order(state: &EncounterState) -> Vec<usize> {
    let mut order = Vec::with_capacity(state.combatants.len());
    for &index in &state.initiative {
        if index < state.combatants.len() && !order.contains(&index) {
            order.push(index);
        }
    }
    for index in 0..state.combatants.len() {
        if !order.contains(&index) {
            order.push(index);
        }
    }
    order
}

fn combatant_line(state: &EncounterState, index: usize, current: bool) -> String {
    let combatant = &state.combatants[index];
    let marker = if current { "> " } else { "  " };
    let initiative = combatant
        .initiative_total
        .map_or_else(|| "-".to_string(), |total| total.to_string());
    let hp = if combatant.temp_hp > 0 {
        format!(
            "{}/{} (+{})",
            combatant.hp, combatant.spec.max_hp, combatant.temp_hp
        )
    } else {
        format!("{}/{}", combatant.hp, combatant.spec.max_hp)
    };
    let conditions = condition_list(&combatant.conditions);
    let resources = resource_list(combatant);
    let saves = saves_label(combatant);
    format!(
        "{marker}{initiative} {} {} HP {hp} AC {} {} | conditions: {conditions} | resources: {resources} | saves: {saves}",
        combatant.name(),
        side_label(combatant.side()),
        combatant.ac(),
        combatant.zone
    )
}

fn side_label(side: Side) -> &'static str {
    match side {
        Side::Party => "party",
        Side::Enemy => "enemy",
        Side::Unknown => "unknown",
    }
}

fn condition_list(conditions: &[ActiveCondition]) -> String {
    if conditions.is_empty() {
        "none".to_string()
    } else {
        conditions
            .iter()
            .map(condition_display)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn condition_display(condition: &ActiveCondition) -> String {
    if let Some(rounds) = condition
        .duration
        .as_ref()
        .and_then(|duration| duration.rounds)
    {
        format!("{}({rounds})", condition.condition)
    } else if let Some(ends) = &condition.ends_with_turn_of {
        format!("{}(until {ends}'s turn)", condition.condition)
    } else {
        condition.condition.clone()
    }
}

fn resource_list(combatant: &CombatantState) -> String {
    if combatant.resources.is_empty() {
        "none".to_string()
    } else {
        combatant
            .resources
            .iter()
            .map(|(name, resource)| format!("{name} {}/{}", resource.current, resource.max))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn saves_label(combatant: &CombatantState) -> String {
    if combatant.kind() != Kind::Pc {
        return "-".to_string();
    }
    if combatant.death_saves.dead {
        "dead".to_string()
    } else if combatant.death_saves.stable {
        "stable".to_string()
    } else {
        format!(
            "{}S/{}F",
            combatant.death_saves.successes, combatant.death_saves.failures
        )
    }
}

fn simulate(
    state_path: &Path,
    json: bool,
    specs: &[String],
    encounter: Option<&Path>,
    options: SimOptions,
) -> Result<()> {
    if options.runs == 0 {
        bail!("--runs must be at least 1");
    }
    if options.max_rounds == 0 {
        bail!("--max-rounds must be at least 1");
    }
    if options.threads == Some(0) {
        bail!("--threads must be at least 1");
    }

    let mut template = if !specs.is_empty() {
        EncounterState::from_specs(specs)?
    } else if let Some(path) = encounter {
        EncounterState::from_encounter_file(path)
            .with_context(|| format!("failed to load encounter {}", path.display()))?
    } else {
        EncounterState::load(state_path)
            .with_context(|| format!("failed to load state {}", state_path.display()))?
    };
    template.log.clear();

    if options.runs == 1 {
        let ai = Ai::new(&template)?;
        let mut rng = make_rng(options.seed);
        let mut lines = Vec::new();
        let run = sim::play_run(
            &mut template,
            &ai,
            options.max_rounds,
            rng.as_mut(),
            &mut lines,
        )?;
        if json {
            let output = SingleRunOutput::from_state(&template, &run, lines);
            println!(
                "{}",
                serde_json::to_string_pretty(&output)
                    .context("failed to serialize simulation report")?
            );
        } else {
            print_lines(&lines);
            print_single_run_summary(&template, &run);
        }
        return Ok(());
    }

    let report = sim::monte_carlo(template, options)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .context("failed to serialize simulation report")?
        );
    } else {
        print_monte_carlo_summary(&report);
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct SingleRunOutput {
    runs: u64,
    outcome: RunOutcome,
    rounds: u32,
    log: Vec<String>,
    combatants: Vec<SingleRunCombatant>,
}

#[derive(Debug, Serialize)]
struct SingleRunCombatant {
    name: String,
    side: Side,
    hp: i32,
    max_hp: i32,
    dead: bool,
    down: bool,
}

impl SingleRunOutput {
    fn from_state(state: &EncounterState, run: &SingleRun, log: Vec<String>) -> Self {
        let combatants = state
            .combatants
            .iter()
            .map(|combatant| SingleRunCombatant {
                name: combatant.name().to_string(),
                side: combatant.side(),
                hp: combatant.hp,
                max_hp: combatant.spec.max_hp,
                dead: combatant.death_saves.dead,
                down: combatant.hp <= 0 && !combatant.death_saves.dead,
            })
            .collect();
        Self {
            runs: 1,
            outcome: run.outcome,
            rounds: run.rounds,
            log,
            combatants,
        }
    }
}

fn print_single_run_summary(state: &EncounterState, run: &SingleRun) {
    let outcome = match run.outcome {
        RunOutcome::PartyWins => "party wins",
        RunOutcome::EnemyWins => "enemies win",
        RunOutcome::Draw => "draw",
        RunOutcome::Timeout => "timeout",
    };
    println!("Outcome: {outcome} after {} round(s)", run.rounds);
    for side in [Side::Party, Side::Enemy] {
        let label = side_label(side);
        let members: Vec<&CombatantState> = state
            .combatants
            .iter()
            .filter(|combatant| combatant.side() == side)
            .collect();
        let hp: i32 = members.iter().map(|combatant| combatant.hp.max(0)).sum();
        let details = members
            .iter()
            .map(|combatant| {
                let status = if combatant.death_saves.dead {
                    " dead"
                } else if combatant.hp <= 0 {
                    " down"
                } else {
                    ""
                };
                format!(
                    "{} {}/{}{status}",
                    combatant.name(),
                    combatant.hp.max(0),
                    combatant.spec.max_hp
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!("{label}: {hp} HP ({details})");
    }
}

fn print_monte_carlo_summary(report: &MonteCarloReport) {
    println!(
        "runs: {}  threads: {}  elapsed: {:.3}s  ({:.0} runs/sec)",
        report.runs, report.threads, report.elapsed_seconds, report.runs_per_second
    );
    println!(
        "party wins: {} ({:.2}%)  enemy wins: {} ({:.2}%)  draws: {} ({:.2}%)  timeouts: {} ({:.2}%)",
        report.party_wins,
        report.party_win_rate * 100.0,
        report.enemy_wins,
        report.enemy_win_rate * 100.0,
        report.draws,
        report.draw_rate * 100.0,
        report.timeouts,
        report.timeout_rate * 100.0
    );
    println!(
        "rounds: avg {:.2}  median {:.1}",
        report.avg_rounds, report.median_rounds
    );
    println!(
        "avg HP left: party {:.2}  enemies {:.2}",
        report.avg_party_hp, report.avg_enemy_hp
    );
    for combatant in &report.combatants {
        println!(
            "{} ({}): avg HP {:.2}  down {:.2}%  dead {:.2}%",
            combatant.name,
            side_label(combatant.side),
            combatant.avg_hp,
            combatant.down_rate * 100.0,
            combatant.dead_rate * 100.0
        );
    }
}

fn run_init(state_path: &Path, seed: Option<u64>, sets: &[String]) -> Result<()> {
    run_state_command(state_path, seed, |state, rng, lines| {
        let mut overrides = Vec::with_capacity(sets.len());
        for token in sets {
            let (name, total) = token
                .split_once('=')
                .ok_or_else(|| anyhow!("--set expects NAME=TOTAL, for example --set Will=30"))?;
            let total: i32 = total
                .trim()
                .parse()
                .with_context(|| format!("invalid initiative total in `{token}`"))?;
            let index = resolve_name(state, name.trim())?;
            overrides.push((index, total));
        }
        engine::init_with_overrides(state, &overrides, rng, lines)
    })
}
