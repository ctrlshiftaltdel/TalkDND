//! Command-line interface for the v2 engine.
//!
//! Commands load the self-contained state file, run one engine operation,
//! append the emitted lines, and save. `new`, `content` and `simulate` are the
//! only commands that need content sources.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::ai::Ai;
use crate::content::{self, ContentSources, ItemKind, ShownItem};
use crate::dice::{self, DiceRng, RollMode};
use crate::engine::{self, AddConditionOptions, AttackOptions, RestKind, UseOptions};
use crate::sim::{self, SimulationConfig};
use crate::state::{
    resolve_state_path, AddSpec, CombatantState, EncounterState, LookupError, NewEncounter,
};

/// Global options shared by every command.
#[derive(Debug, Parser)]
#[command(
    name = "combat-sim",
    version,
    about = "Headless D&D 5e (2014 / 2024 rules, selected per campaign) combat helper"
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
        /// Surprised combatant (2024: rolls initiative with Disadvantage), repeatable.
        #[arg(long = "surprised", value_name = "NAME")]
        surprised: Vec<String>,
        /// Combatant rolling initiative with Advantage (e.g. Invisible), repeatable.
        #[arg(long = "adv", value_name = "NAME")]
        adv: Vec<String>,
        /// Combatant rolling initiative with Disadvantage, repeatable.
        #[arg(long = "dis", value_name = "NAME")]
        dis: Vec<String>,
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
        /// Knock out instead of kill if a melee attack drops the target to 0 HP (2024 PHB).
        #[arg(long)]
        nonlethal: bool,
    },

    /// Apply damage from an expression or flat number.
    Damage {
        target: String,
        expression: String,
        #[arg(long = "type", value_name = "T")]
        damage_type: Option<String>,
        #[arg(long)]
        crit: bool,
        /// The damage is a melee blow that knocks out at 0 HP instead of killing.
        #[arg(long)]
        nonlethal: bool,
    },

    /// Heal a creature.
    Heal {
        target: String,
        expression: String,
        /// Creature whose healing is being applied.
        #[arg(long = "from", value_name = "NAME")]
        from: Option<String>,
        /// Spend one spell slot of this level from --from (prefer `cast`).
        #[arg(long, value_name = "N", requires = "from")]
        slot: Option<i32>,
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
        /// Knock out instead of kill if a melee attack drops the target to 0 HP (2024 PHB).
        #[arg(long)]
        nonlethal: bool,
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
        /// Knock out instead of kill if a melee attack drops the target to 0 HP (2024 PHB).
        #[arg(long)]
        nonlethal: bool,
    },

    /// Add or remove a condition.
    Condition {
        #[command(subcommand)]
        command: ConditionCommand,
    },

    /// Move a combatant to another zone or grid cell.
    Move {
        who: String,
        /// Destination zone (grid: any free cell with that zone tag).
        #[arg(conflicts_with = "to")]
        zone: Option<String>,
        /// Exact grid destination, for example `--to 3,2`.
        #[arg(long, value_name = "X,Y")]
        to: Option<String>,
        /// Skip opportunity attacks from creatures being left.
        #[arg(long)]
        no_oa: bool,
    },

    /// Interact with a scene object.
    Interact {
        who: String,
        object: String,
        /// 1-based option number for an object with an options list.
        #[arg(long)]
        option: Option<usize>,
    },

    /// Print zones, edges, objects, triggers and positions.
    Scene,

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

    /// Play an encounter once or run a Monte Carlo batch.
    Simulate {
        /// Build this encounter, like `new`.
        #[arg(long, value_name = "ID", conflicts_with_all = ["from_state", "state"])]
        encounter: Option<String>,
        /// Comma-separated party creature ids (used with --encounter).
        #[arg(long, value_name = "A,B,C")]
        party: Option<String>,
        /// Continue from a saved state name or path.
        #[arg(long = "from-state", value_name = "NAME", conflicts_with = "state")]
        from_state: Option<PathBuf>,
        /// Number of runs (1 = play-by-play).
        #[arg(long, default_value_t = 1)]
        runs: u32,
        /// Round limit; a run that reaches it is a timeout.
        #[arg(long = "max-rounds", default_value_t = 100)]
        max_rounds: u32,
        /// Worker threads for a Monte Carlo run (default: all cores).
        #[arg(long)]
        threads: Option<usize>,
    },
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
        #[arg(long, value_enum, conflicts_with = "until_end_of_turn")]
        until: Option<UntilArg>,
        /// Backwards-compatible shorthand: ends at the named creature's next turn.
        #[arg(long = "until-end-of-turn", value_name = "NAME")]
        until_end_of_turn: Option<String>,
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
        } => new_state(
            &sources,
            state.as_deref(),
            encounter,
            party,
            party_zone,
            add,
        ),
        Command::Init {
            set,
            surprised,
            adv,
            dis,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let surprise = state.content.rules.surprise.clone().unwrap_or_default();
                let surprise_dis = surprise.initiative_disadvantage.unwrap_or(surprise.condition.is_none());
                let no_names: Vec<String> = Vec::new();
                let mut modes = Vec::new();
                for (names, mode) in [
                    (if surprise_dis { &surprised } else { &no_names }, RollMode::Disadvantage),
                    (&dis, RollMode::Disadvantage),
                    (&adv, RollMode::Advantage),
                ] {
                    for name in names {
                        modes.push((canonical_index(state, name.trim())?, mode));
                    }
                }
                for name in &surprised {
                    let index = canonical_index(state, name.trim())?;
                    if surprise_dis {
                        lines.push(format!(
                            "{} is Surprised: initiative rolled with Disadvantage",
                            state.combatants[index].name
                        ));
                    }
                }
                let surprise_condition = surprise.condition.clone();
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
                engine::init_with_modes(state, &overrides, &modes, rng, lines)?;
                if let Some(condition) = surprise_condition {
                    for name in &surprised {
                        let who = canonical_name(state, name.trim())?;
                        let options = engine::AddConditionOptions {
                            until: Some(crate::content::schema::DurationUntil::EndOfTargetNextTurn),
                            until_subject: Some(who.clone()),
                            ..Default::default()
                        };
                        engine::add_condition(state, &who, &condition, &options, rng, lines)?;
                        lines.push(format!(
                            "{who} is Surprised (2014): no move/action on its first turn, no reactions until that turn ends"
                        ));
                    }
                }
                Ok(())
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
            nonlethal,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let attacker = canonical_name(state, &attacker)?;
                warn_out_of_turn(state, &attacker, lines);
                let target = canonical_name(state, &target)?;
                let options = AttackOptions {
                    advantage: adv,
                    disadvantage: dis,
                    multi,
                    no_reactions,
                    nonlethal,
                };
                engine::attack(state, &attacker, &target, &attack, &options, rng, lines)
            })
        }
        Command::Damage {
            target,
            expression,
            damage_type,
            crit,
            nonlethal,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let target = canonical_name(state, &target)?;
                state.nonlethal_active = nonlethal;
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
            slot,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let target = canonical_name(state, &target)?;
                let from = from
                    .as_deref()
                    .map(|name| canonical_name(state, name))
                    .transpose()?;
                if let (Some(level), Some(caster)) = (slot, from.as_deref()) {
                    engine::spend_spell_slot(state, caster, level, lines)?;
                }
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
                engine::save(
                    state,
                    &who,
                    &ability,
                    dc,
                    RollMode::from_flags(adv, dis),
                    rng,
                    lines,
                )
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
            // Stateless unless --state names an existing file (then it is logged there too).
            let path = state.as_deref().map(|_| state_path(&sources, state.as_deref()));
            match path {
                Some(path) if path.exists() => run_state_command(&path, seed, |_state, rng, lines| {
                    engine::roll_expression(&expression, RollMode::from_flags(adv, dis), dc, rng, lines)
                }),
                _ => {
                    let mut lines = Vec::new();
                    let mut rng = make_rng(seed);
                    engine::roll_expression(
                        &expression,
                        RollMode::from_flags(adv, dis),
                        dc,
                        rng.as_mut(),
                        &mut lines,
                    )?;
                    print_lines(&lines);
                    Ok(())
                }
            }
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
            nonlethal,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                warn_out_of_turn(state, &who, lines);
                let options = UseOptions {
                    targets,
                    option,
                    slot_level: slot,
                    advantage: adv,
                    disadvantage: dis,
                    no_reactions,
                    nonlethal,
                };
                state.nonlethal_request = nonlethal;
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
            nonlethal,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                warn_out_of_turn(state, &who, lines);
                let options = UseOptions {
                    targets: targets.clone(),
                    option,
                    slot_level: slot,
                    advantage: adv,
                    disadvantage: dis,
                    no_reactions,
                    nonlethal,
                };
                state.nonlethal_request = nonlethal;
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
                until_end_of_turn,
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
                    let (until, until_subject) = match until_end_of_turn {
                        Some(name) => (
                            Some(crate::content::schema::DurationUntil::EndOfTargetNextTurn),
                            Some(name),
                        ),
                        None => (until.map(Into::into), until_subject),
                    };
                    let options = AddConditionOptions {
                        rounds,
                        minutes,
                        permanent,
                        until,
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
        Command::Move {
            who,
            zone,
            to,
            no_oa,
        } => {
            if zone.is_none() && to.is_none() {
                return Err(anyhow!("move needs a destination zone or --to X,Y"));
            }
            let target = if let Some(to) = to {
                let (x, y) = to
                    .split_once(',')
                    .ok_or_else(|| anyhow!("--to expects X,Y"))?;
                let x: i32 = x
                    .trim()
                    .parse()
                    .with_context(|| format!("invalid x coordinate in `{to}`"))?;
                let y: i32 = y
                    .trim()
                    .parse()
                    .with_context(|| format!("invalid y coordinate in `{to}`"))?;
                engine::MoveTarget::Cell(x, y)
            } else {
                engine::MoveTarget::Zone(zone.unwrap())
            };
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                warn_out_of_turn(state, &who, lines);
                engine::move_to(state, &who, &target, no_oa, rng, lines)
            })
        }
        Command::Interact {
            who,
            object,
            option,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                warn_out_of_turn(state, &who, lines);
                engine::interact(state, &who, &object, option, rng, lines)
            })
        }
        Command::Scene => print_scene(&state_path(&sources, state.as_deref())),
        Command::Deathsave { who } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                engine::death_save(state, &who, rng, lines)
            })
        }
        Command::Next => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, seed, |state, rng, lines| {
                engine::next_turn(state, rng, lines)
            })
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
        Command::Simulate {
            encounter,
            party,
            from_state,
            runs,
            max_rounds,
            threads,
        } => run_simulate(
            &sources,
            state.as_deref(),
            seed,
            json,
            encounter,
            party,
            from_state,
            runs,
            max_rounds,
            threads,
        ),
    }
}

// ---------------------------------------------------------------------------
// State path and content loading
// ---------------------------------------------------------------------------

fn read_manifest(sources: &ContentSources) -> Option<crate::content::schema::CampaignManifest> {
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
    serde_json::from_value(value).ok()
}

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
    let manifest: crate::content::schema::CampaignManifest = serde_json::from_value(value).ok()?;
    let base = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    manifest.state_dir.map(|dir| {
        if dir.is_absolute() {
            dir
        } else {
            base.join(dir)
        }
    })
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

/// Warns (does not block) when someone other than the active combatant acts:
/// legitimate for reactions and readied actions, a bookkeeping slip otherwise.
fn warn_out_of_turn(state: &EncounterState, actor: &str, lines: &mut Vec<String>) {
    if state.initiative.is_empty() || state.outcome.is_some() {
        return;
    }
    let Some(&active) = state.initiative.get(state.turn) else {
        return;
    };
    let active_name = &state.combatants[active].name;
    if !active_name.eq_ignore_ascii_case(actor) {
        lines.push(format!(
            "WARNING: {actor} is acting out of turn (round {}, active: {active_name}). OK for reactions/readied actions; otherwise run `next` first.",
            state.round
        ));
    }
}

/// Party ids from `--party`, else the manifest's `party:` list, else an error.
fn resolve_party(sources: &ContentSources, party: Option<String>) -> Result<Vec<String>> {
    let explicit: Vec<String> = party
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect();
    if !explicit.is_empty() {
        return Ok(explicit);
    }
    if let Some(manifest) = read_manifest(sources) {
        if !manifest.party.is_empty() {
            return Ok(manifest.party);
        }
    }
    bail!(
        "no party: pass --party a,b,c or add `party: [a, b, c]` to the campaign manifest (an empty party would make every fight a 100% enemy win)"
    )
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
    let party = resolve_party(sources, party).unwrap_or_else(|error| {
        eprintln!("warning: {error}");
        Vec::new()
    });
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

#[derive(Debug, Serialize)]
struct SimRunOutput {
    log: Vec<String>,
    summary: SimSummary,
}

#[derive(Debug, Serialize)]
struct SimSummary {
    outcome: String,
    winner: Option<String>,
    timed_out: bool,
    rounds: u32,
    hp_per_side: BTreeMap<String, i32>,
    combatants: Vec<SimCombatantSummary>,
}

#[derive(Debug, Serialize)]
struct SimCombatantSummary {
    name: String,
    side: String,
    hp: i32,
    max_hp: i32,
    down: bool,
    dead: bool,
}

#[allow(clippy::too_many_arguments)]
fn run_simulate(
    sources: &ContentSources,
    state_name: Option<&Path>,
    seed: Option<u64>,
    json: bool,
    encounter: Option<String>,
    party: Option<String>,
    from_state: Option<PathBuf>,
    runs: u32,
    max_rounds: u32,
    threads: Option<usize>,
) -> Result<()> {
    if runs == 0 {
        return Err(anyhow!("--runs must be at least 1"));
    }
    if max_rounds == 0 {
        return Err(anyhow!("--max-rounds must be at least 1"));
    }
    let state = if let Some(encounter) = encounter {
        let content = load_content(sources)?;
        let party = resolve_party(sources, party)?;
        let options = NewEncounter {
            encounter,
            party,
            party_zone: None,
            adds: Vec::new(),
        };
        EncounterState::from_encounter_shared(Arc::new(content), &options)?
    } else if let Some(name) = from_state {
        let state_dir = manifest_state_dir(sources);
        let path = resolve_state_path(Some(&name), state_dir.as_deref());
        EncounterState::load(&path)
            .with_context(|| format!("failed to load state {}", path.display()))?
    } else {
        let Some(name) = state_name else {
            return Err(anyhow!(
                "simulate needs --encounter ID, --from-state NAME, or --state NAME"
            ));
        };
        let path = state_path(sources, Some(name));
        EncounterState::load(&path)
            .with_context(|| format!("failed to load state {}", path.display()))?
    };

    let sides: std::collections::BTreeSet<&str> = state
        .combatants
        .iter()
        .map(|combatant| combatant.side.as_str())
        .collect();
    if sides.len() < 2 {
        bail!(
            "simulate: only one side present ({}); pass --party a,b,c or set `party:` in the campaign manifest",
            sides.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    let ai = Ai::prepare(&state);
    let mut rng = make_rng(seed);
    if runs == 1 {
        let meta: Vec<(String, String, i32)> = state
            .combatants
            .iter()
            .map(|combatant| {
                (
                    combatant.name.clone(),
                    combatant.side.as_str().to_string(),
                    combatant.max_hp,
                )
            })
            .collect();
        let mut log = state.log.clone();
        let outcome = sim::play(state, &ai, rng.as_mut(), max_rounds, &mut log)?;
        let mut hp_per_side: BTreeMap<String, i32> = BTreeMap::new();
        let combatants: Vec<SimCombatantSummary> = meta
            .iter()
            .enumerate()
            .map(|(index, (name, side, max_hp))| {
                let hp = outcome.hp.get(index).copied().unwrap_or(0);
                *hp_per_side.entry(side.clone()).or_default() += hp.max(0);
                SimCombatantSummary {
                    name: name.clone(),
                    side: side.clone(),
                    hp,
                    max_hp: *max_hp,
                    down: outcome.down.get(index).copied().unwrap_or(false),
                    dead: outcome.dead.get(index).copied().unwrap_or(false),
                }
            })
            .collect();
        let winner = match &outcome.outcome {
            Some(crate::state::CombatOutcome::Winner(side)) => Some(side.clone()),
            _ => None,
        };
        let summary = SimSummary {
            outcome: if outcome.timed_out && outcome.outcome.is_none() {
                "timeout".to_string()
            } else {
                outcome
                    .outcome
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "draw".to_string())
            },
            winner,
            timed_out: outcome.timed_out,
            rounds: outcome.rounds,
            hp_per_side: hp_per_side.clone(),
            combatants,
        };
        if json {
            let output = SimRunOutput { log, summary };
            println!(
                "{}",
                serde_json::to_string_pretty(&output).context("failed to serialize simulation")?
            );
        } else {
            print_lines(&log);
            println!("Result: {}", summary.outcome);
            println!("Rounds: {}", summary.rounds);
            let hp_text = summary
                .hp_per_side
                .iter()
                .map(|(side, hp)| format!("{side} {hp}"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("HP left: {hp_text}");
            for combatant in &summary.combatants {
                let status = if combatant.dead {
                    "dead".to_string()
                } else if combatant.down {
                    "down".to_string()
                } else {
                    format!("{}/{}", combatant.hp, combatant.max_hp)
                };
                println!("  {} ({}): {}", combatant.name, combatant.side, status);
            }
        }
        return Ok(());
    }

    let threads = threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1)
    });
    let config = SimulationConfig {
        runs,
        max_rounds,
        threads,
        seed,
    };
    let report = sim::monte_carlo(&state, &ai, &config)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .context("failed to serialize simulation report")?
        );
    } else {
        println!(
            "runs: {}  threads: {}  elapsed: {:.3}s  {:.0} runs/s",
            report.runs, report.threads, report.elapsed_secs, report.runs_per_sec
        );
        let wins = report
            .wins
            .iter()
            .map(|(side, count)| format!("{side} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "wins: {wins}  draws: {}  timeouts: {}  errors: {}",
            report.draws, report.timeouts, report.errors
        );
        println!(
            "rounds: avg {:.1}  median {:.0}",
            report.avg_rounds, report.median_rounds
        );
        let hp = report
            .avg_hp_per_side
            .iter()
            .map(|(side, hp)| format!("{side} {hp:.1}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("avg HP left: {hp}");
        for creature in &report.creatures {
            println!(
                "  {} ({}): avg HP {:.1}/{}  down {:.1}%  dead {:.1}%",
                creature.name,
                creature.side,
                creature.avg_hp,
                creature.max_hp,
                creature.down_pct,
                creature.dead_pct
            );
        }
        for example in &report.error_examples {
            println!("  error: {example}");
        }
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

fn print_scene(state_path: &Path) -> Result<()> {
    let state = EncounterState::load(state_path)
        .with_context(|| format!("failed to load state {}", state_path.display()))?;
    for line in engine::scene_text(&state) {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pos: Option<[i32; 2]>,
    conditions: Vec<ConditionStatus>,
    resources: BTreeMap<String, ResourceStatus>,
    death_saves: Option<crate::state::DeathSaves>,
    dormant: bool,
    concentrating: bool,
    dead: bool,
    knocked_out: bool,
    speed_left: i32,
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
                    pos: combatant.cell(),
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
                            save_ends: condition.save_ends.as_ref().map(|save_ends| {
                                format!("{}:{}", save_ends.ability.as_str(), save_ends.dc)
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
                    dead: combatant.is_dead(),
                    knocked_out: combatant.knocked_out,
                    speed_left: combatant.speed_left,
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
