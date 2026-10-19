//! Command-line interface for the v2 engine.
//!
//! Commands load the self-contained state file, run one engine operation,
//! append the emitted lines, and save. `new`, `content` and `simulate` are the
//! only commands that need content sources.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::content::{self, ContentSources, ItemKind, ShownItem};
use crate::dice::{self, DiceRng, RollMode};
use crate::engine::{
    self, AddConditionOptions, AttackOptions, RestKind, UseOptions,
};
use crate::state::{
    resolve_state_path, AddSpec, CombatantState, EncounterState, LookupError, NewEncounter,
};

/// Global options shared by every command.
#[derive(Debug, Parser)]
#[command(
    name = "combat-sim",
    version,
    about = "Headless D&D 5e (2024 rules) combat helper"
)]
pub struct Cli {
    /// Persisted encounter state. A bare name resolves to
    /// `<manifest state_dir>/NAME.state.json`, or `./NAME.state.json`.
    #[arg(long, global = true, value_name = "NAME")]
    pub state: Option<PathBuf>,

    /// Seed for deterministic rolls (tests and replays only).
    #[arg(long, global = true, value_name = "N")]
    pub seed: Option<u64>,

    /// Emit machine-readable JSON where supported.
    #[arg(long, global = true)]
    pub json: bool,

    /// Additional content library directory (repeatable).
    #[arg(long = "lib", global = true, value_name = "DIR")]
    pub lib: Vec<PathBuf>,

    /// Additional adventure content directory (repeatable).
    #[arg(long = "adventure", global = true, value_name = "DIR")]
    pub adventure: Vec<PathBuf>,

    /// Additional character file or directory (repeatable).
    #[arg(long = "chars", global = true, value_name = "PATH")]
    pub chars: Vec<PathBuf>,

    /// Campaign manifest; falls back to COMBAT_SIM_CAMPAIGN.
    #[arg(long = "campaign", global = true, value_name = "FILE")]
    pub campaign: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

/// One CLI action.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new encounter state from an encounter id.
    New {
        #[arg(long, value_name = "ID")]
        encounter: String,
        /// Comma-separated party creature ids.
        #[arg(long, value_name = "A,B,C")]
        party: Option<String>,
        /// Default party zone.
        #[arg(long, value_name = "ZONE")]
        party_zone: Option<String>,
        /// Extra members: `id[:count][@zone]` (repeatable).
        #[arg(long = "add", value_name = "ID[:N][@ZONE]")]
        add: Vec<String>,
    },

    /// Roll initiative.
    Init {
        /// Fixed initiative total for one combatant, repeatable.
        #[arg(long = "set", value_name = "NAME=TOTAL")]
        set: Vec<String>,
    },

    /// Show the current encounter.
    Status,

    /// Resolve one attack or multiattack.
    Attack {
        attacker: String,
        target: String,
        attack: String,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
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
        /// Creature whose healing is being applied.
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

    /// Cast an owned ability.
    Cast {
        who: String,
        ability: String,
        targets: Vec<String>,
        #[arg(long)]
        option: Option<usize>,
        #[arg(long)]
        slot: Option<i32>,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
        #[arg(long)]
        no_reactions: bool,
    },

    /// Use an owned ability or a creature attack.
    Use {
        who: String,
        ability: String,
        targets: Vec<String>,
        #[arg(long)]
        option: Option<usize>,
        #[arg(long)]
        slot: Option<i32>,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
        #[arg(long)]
        no_reactions: bool,
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

    /// Wake dormant creatures matching a name, creature id or tag.
    Activate { name: String },

    /// Inspect and validate campaign content without loading an encounter.
    Content {
        #[command(subcommand)]
        command: ContentCommand,
    },

    /// Batch simulation (arrives in v2 step 4).
    Simulate,
}

#[derive(Debug, Subcommand)]
pub enum ContentCommand {
    /// List loaded content items and their source files.
    List {
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
    },
    /// Print one item as JSON, including its kind and source file.
    Show { id: String },
    /// Validate references, expressions and TODO notes.
    Check,
}

#[derive(Debug, Subcommand)]
pub enum ConditionCommand {
    Add {
        who: String,
        condition: String,
        #[arg(long, conflicts_with_all = ["minutes", "permanent", "until"])]
        rounds: Option<u32>,
        #[arg(long, conflicts_with_all = ["rounds", "permanent", "until"])]
        minutes: Option<u32>,
        #[arg(long, conflicts_with_all = ["rounds", "minutes", "until"])]
        permanent: bool,
        #[arg(long, value_enum)]
        until: Option<UntilArg>,
        /// Creature whose next turn ends the condition (defaults to the target).
        #[arg(long = "until-subject", value_name = "NAME")]
        until_subject: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        linked: Option<String>,
        /// Repeated save: `ABILITY:DC`, for example `wis:13`.
        #[arg(long = "save-ends", value_name = "ABILITY:DC")]
        save_ends: Option<String>,
    },
    Remove {
        who: String,
        condition: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum UntilArg {
    StartOfSourceNextTurn,
    EndOfSourceNextTurn,
    EndOfTargetNextTurn,
}

impl From<UntilArg> for crate::content::schema::DurationUntil {
    fn from(value: UntilArg) -> Self {
        match value {
            UntilArg::StartOfSourceNextTurn => Self::StartOfSourceNextTurn,
            UntilArg::EndOfSourceNextTurn => Self::EndOfSourceNextTurn,
            UntilArg::EndOfTargetNextTurn => Self::EndOfTargetNextTurn,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RestKindArg {
    Short,
    Long,
}

impl From<RestKindArg> for RestKind {
    fn from(kind: RestKindArg) -> Self {
        match kind {
            RestKindArg::Short => RestKind::Short,
            RestKindArg::Long => RestKind::Long,
        }
    }
}

pub fn run() -> Result<()> {
    let Cli {
        state,
        seed,
        json,
        lib,
        adventure,
        chars,
        campaign,
        command,
    } = Cli::parse();

    let sources = ContentSources {
        libraries: lib,
        adventures: adventure,
        characters: chars,
        campaign,
    };

    match command {
        Command::New {
            encounter,
            party,
            party_zone,
            add,
        } => new_state(&sources, state.as_deref(), encounter, party, party_zone, add),
        Command::Init { set } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let mut overrides = Vec::new();
                for token in &set {
                    let (name, total) = token.split_once('=').ok_or_else(|| {
                        anyhow!("--set expects NAME=TOTAL, for example --set A=30")
                    })?;
                    let total: i32 = total
                        .trim()
                        .parse()
                        .with_context(|| format!("invalid initiative total in `{token}`"))?;
                    let index = canonical_index(state, name.trim())?;
                    overrides.push((index, total));
                }
                engine::init(state, &overrides, rng, lines)
            })
        }
        Command::Status => print_status(&state_path(&sources, state.as_deref()), json),
        Command::Attack {
            attacker,
            target,
            attack,
            adv,
            dis,
            multi,
            no_reactions,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let attacker = canonical_name(state, &attacker)?;
                let target = canonical_name(state, &target)?;
                let options = AttackOptions {
                    advantage: adv,
                    disadvantage: dis,
                    multi,
                    no_reactions,
                };
                engine::attack(state, &attacker, &target, &attack, &options, rng, lines)
            })
        }
        Command::Damage {
            target,
            expression,
            damage_type,
            crit,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let target = canonical_name(state, &target)?;
                engine::damage(
                    state,
                    &target,
                    &expression,
                    damage_type.as_deref(),
                    crit,
                    rng,
                    lines,
                )
            })
        }
        Command::Heal {
            target,
            expression,
            from,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let target = canonical_name(state, &target)?;
                let from = from
                    .as_deref()
                    .map(|name| canonical_name(state, name))
                    .transpose()?;
                engine::heal(state, &target, &expression, from.as_deref(), rng, lines)
            })
        }
        Command::Save {
            who,
            ability,
            dc,
            adv,
            dis,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                engine::save(state, &who, &ability, dc, RollMode::from_flags(adv, dis), rng, lines)
            })
        }
        Command::Check {
            who,
            skill_or_ability,
            dc,
            adv,
            dis,
            bonus,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                engine::check(
                    state,
                    &who,
                    &skill_or_ability,
                    dc,
                    RollMode::from_flags(adv, dis),
                    bonus,
                    rng,
                    lines,
                )
            })
        }
        Command::Roll {
            expression,
            adv,
            dis,
            dc,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |_state, rng, lines| {
                engine::roll_expression(&expression, RollMode::from_flags(adv, dis), dc, rng, lines)
            })
        }
        Command::Cast {
            who,
            ability,
            targets,
            option,
            slot,
            adv,
            dis,
            no_reactions,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                let targets = canonical_names(state, &targets)?;
                let options = UseOptions {
                    targets,
                    option,
                    slot_level: slot,
                    advantage: adv,
                    disadvantage: dis,
                    no_reactions,
                };
                engine::use_ability(state, &who, &ability, &options, rng, lines)
            })
        }
        Command::Use {
            who,
            ability,
            targets,
            option,
            slot,
            adv,
            dis,
            no_reactions,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                let targets = canonical_names(state, &targets)?;
                let options = UseOptions {
                    targets: targets.clone(),
                    option,
                    slot_level: slot,
                    advantage: adv,
                    disadvantage: dis,
                    no_reactions,
                };
                engine::use_entry(state, &who, &ability, &targets, &options, rng, lines)
            })
        }
        Command::Condition { command } => match command {
            ConditionCommand::Add {
                who,
                condition,
                rounds,
                minutes,
                permanent,
                until,
                until_subject,
                source,
                linked,
                save_ends,
            } => {
                let path = state_path(&sources, state.as_deref());
                run_state_command(&path, seed, |state, rng, lines| {
                    let who = canonical_name(state, &who)?;
                    let save_ends = match save_ends {
                        Some(text) => {
                            let (ability, dc) = text.split_once(':').ok_or_else(|| {
                                anyhow!("--save-ends expects ABILITY:DC, for example wis:13")
                            })?;
                            let ability = crate::state::ability_from_text(ability)
                                .ok_or_else(|| anyhow!("unknown save ability `{ability}`"))?;
                            let dc: i32 = dc
                                .trim()
                                .parse()
                                .with_context(|| format!("invalid save DC in `{text}`"))?;
                            Some((ability, dc))
                        }
                        None => None,
                    };
                    let options = AddConditionOptions {
                        rounds,
                        minutes,
                        permanent,
                        until: until.map(Into::into),
                        until_subject,
                        source,
                        linked,
                        save_ends,
                    };
                    engine::add_condition(state, &who, &condition, &options, rng, lines)
                })
            }
            ConditionCommand::Remove { who, condition } => {
                let path = state_path(&sources, state.as_deref());
                run_state_command(&path, seed, |state, _rng, lines| {
                    let who = canonical_name(state, &who)?;
                    engine::remove_condition(state, &who, &condition, lines)
                })
            }
        },
        Command::Move { who, zone } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, _rng, lines| {
                let who = canonical_name(state, &who)?;
                engine::move_to(state, &who, &zone, lines)
            })
        }
        Command::Deathsave { who } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                engine::death_save(state, &who, rng, lines)
            })
        }
        Command::Next => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| engine::next_turn(state, rng, lines))
        }
        Command::Log { tail } => print_log(&state_path(&sources, state.as_deref()), tail),
        Command::Rest { kind, who } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, _rng, lines| {
                let who = if who.eq_ignore_ascii_case("all") {
                    "all".to_string()
                } else {
                    canonical_name(state, &who)?
                };
                engine::rest(state, kind.into(), Some(&who), lines)
            })
        }
        Command::Activate { name } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                engine::activate(state, &name, rng, lines)
            })
        }
        Command::Content { command } => run_content(&sources, command),
        Command::Simulate => {
            println!("not yet (v2 step 4)");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// State path and content loading
// ---------------------------------------------------------------------------

fn manifest_state_dir(sources: &ContentSources) -> Option<PathBuf> {
    let path = sources.campaign_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        serde_json::from_str(&text).ok()?
    } else {
        let value: serde_norway::Value = serde_norway::from_str(&text).ok()?;
        serde_json::to_value(value).ok()?
    };
    let manifest: crate::content::schema::CampaignManifest =
        serde_json::from_value(value).ok()?;
    let base = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    manifest
        .state_dir
        .map(|dir| if dir.is_absolute() { dir } else { base.join(dir) })
}

fn state_path(sources: &ContentSources, requested: Option<&Path>) -> PathBuf {
    let state_dir = requested
        .filter(|path| {
            let text = path.to_string_lossy();
            !text.contains('/') && !text.contains('\\')
        })
        .and_then(|_| manifest_state_dir(sources));
    resolve_state_path(requested, state_dir.as_deref())
}

fn load_content(sources: &ContentSources) -> Result<content::ContentRegistry> {
    sources.load().context("failed to load content")
}

fn run_content(sources: &ContentSources, command: ContentCommand) -> Result<()> {
    match command {
        ContentCommand::List { kind } => {
            let registry = load_content(sources)?;
            let filter = match kind {
                Some(kind) => Some(ItemKind::parse(&kind).ok_or_else(|| {
                    anyhow!(
                        "unknown content kind `{kind}`; expected conditions, masteries, abilities, creatures, ai-templates, scenes or encounters"
                    )
                })?),
                None => None,
            };
            let kinds: Vec<ItemKind> = match filter {
                Some(kind) => vec![kind],
                None => ItemKind::ALL.to_vec(),
            };
            for kind in kinds {
                for (id, source) in registry.entries(kind) {
                    println!("{}\t{id}\t{source}", kind.as_str());
                }
            }
            Ok(())
        }
        ContentCommand::Show { id } => {
            let registry = load_content(sources)?;
            let item = registry
                .find_any(&id)
                .ok_or_else(|| anyhow!("no content item with id `{id}`"))?;
            let shown = ShownItem::new(&registry, item);
            println!(
                "{}",
                serde_json::to_string_pretty(&shown).context("failed to serialize content item")?
            );
            Ok(())
        }
        ContentCommand::Check => {
            let registry = load_content(sources)?;
            let report = content::check(&registry);
            for issue in &report.issues {
                println!("{issue}");
            }
            println!(
                "content check: {} error(s), {} warning(s)",
                report.error_count(),
                report.warning_count()
            );
            if report.has_errors() {
                bail!(
                    "content check failed with {} error(s)",
                    report.error_count()
                );
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Command plumbing
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

fn new_state(
    sources: &ContentSources,
    requested: Option<&Path>,
    encounter: String,
    party: Option<String>,
    party_zone: Option<String>,
    add: Vec<String>,
) -> Result<()> {
    let content = load_content(sources)?;
    let party = party
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect();
    let adds = add
        .iter()
        .map(|spec| AddSpec::parse(spec))
        .collect::<Result<Vec<_>>>()?;
    let options = NewEncounter {
        encounter,
        party,
        party_zone,
        adds,
    };
    let state = EncounterState::from_encounter(content, &options)?;
    let state_dir = manifest_state_dir(sources);
    let path = resolve_state_path(requested, state_dir.as_deref());
    state
        .save(&path)
        .with_context(|| format!("failed to save state {}", path.display()))?;
    for line in &state.log {
        println!("{line}");
    }
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

fn canonical_index(state: &EncounterState, query: &str) -> Result<usize> {
    resolve_name(state, query)
}

fn canonical_name(state: &EncounterState, query: &str) -> Result<String> {
    let index = resolve_name(state, query)?;
    Ok(state.combatants[index].name.clone())
}

fn canonical_names(state: &EncounterState, queries: &[String]) -> Result<Vec<String>> {
    queries
        .iter()
        .map(|query| canonical_name(state, query))
        .collect()
}

fn known_combatants(state: &EncounterState) -> String {
    if state.combatants.is_empty() {
        "(none)".to_string()
    } else {
        state
            .combatants
            .iter()
            .map(|combatant| combatant.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

// ---------------------------------------------------------------------------
// Read-only commands and status JSON
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
        for line in engine::status_lines(&state) {
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

#[derive(Debug, Serialize)]
struct StatusOutput {
    round: u32,
    turn: usize,
    active: Option<String>,
    outcome: Option<crate::state::CombatOutcome>,
    combatants: Vec<CombatantStatus>,
}

#[derive(Debug, Serialize)]
struct CombatantStatus {
    name: String,
    side: String,
    kind: crate::content::schema::CreatureKind,
    initiative: Option<i32>,
    current: bool,
    hp: i32,
    max_hp: i32,
    temp_hp: i32,
    ac: i32,
    zone: String,
    conditions: Vec<ConditionStatus>,
    resources: BTreeMap<String, ResourceStatus>,
    death_saves: Option<crate::state::DeathSaves>,
    dormant: bool,
    concentrating: bool,
}

#[derive(Debug, Serialize)]
struct ConditionStatus {
    id: String,
    source: Option<String>,
    linked: Option<String>,
    remaining: Option<String>,
    save_ends: Option<String>,
    concentration: bool,
}

#[derive(Debug, Serialize)]
struct ResourceStatus {
    current: i32,
    max: i32,
    recharge: String,
}

impl StatusOutput {
    fn from_state(state: &EncounterState) -> Self {
        let current = state.current_index();
        let combatants = engine::status_order(state)
            .into_iter()
            .map(|index| {
                let combatant: &CombatantState = &state.combatants[index];
                CombatantStatus {
                    name: combatant.name.clone(),
                    side: combatant.side.as_str().to_string(),
                    kind: combatant.kind(),
                    initiative: combatant.initiative_total,
                    current: current == Some(index),
                    hp: combatant.hp,
                    max_hp: combatant.max_hp,
                    temp_hp: combatant.temp_hp,
                    ac: engine::armor_class(state, index),
                    zone: combatant.zone.clone(),
                    conditions: combatant
                        .conditions
                        .iter()
                        .map(|condition| ConditionStatus {
                            id: condition.condition.clone(),
                            source: condition.source.clone(),
                            linked: condition.linked.clone(),
                            remaining: condition
                                .duration
                                .as_ref()
                                .map(crate::state::ActiveDuration::describe),
                            save_ends: condition
                                .save_ends
                                .as_ref()
                                .map(|save_ends| {
                                    format!(
                                        "{}:{}",
                                        save_ends.ability.as_str(),
                                        save_ends.dc
                                    )
                                }),
                            concentration: condition.concentration,
                        })
                        .collect(),
                    resources: combatant
                        .resources
                        .iter()
                        .map(|(id, resource)| {
                            (
                                id.clone(),
                                ResourceStatus {
                                    current: resource.current,
                                    max: resource.max,
                                    recharge: resource.recharge.to_string(),
                                },
                            )
                        })
                        .collect(),
                    death_saves: if combatant.is_pc() {
                        Some(combatant.death_saves)
                    } else {
                        None
                    },
                    dormant: combatant.dormant,
                    concentrating: state.concentration.contains_key(&combatant.name),
                }
            })
            .collect();
        Self {
            round: state.round,
            turn: state.turn,
            active: state.current().map(|combatant| combatant.name.clone()),
            outcome: state.outcome.clone(),
            combatants,
        }
    }
}
