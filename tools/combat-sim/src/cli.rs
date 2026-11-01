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
use crate::content::schema::{Light, Obscurement};
use crate::content::{self, ContentSources, ItemKind, ShownItem};
use crate::dice::{self, DiceRng, RollMode};
use crate::engine::{
    self, AddConditionOptions, AttackOptions, FallOptions, GrappleOptions, ReadyOptions, RestKind,
    ShoveMode, ShoveOptions, UnarmedOptions, UseOptions,
};
use crate::sim::{self, SimulationConfig};
use crate::space::FallAbility;
use crate::state::{
    resolve_state_path, AddSpec, CombatantState, EncounterState, LookupError, NewEncounter,
    ReadyResponse, ReadyState, ReadyTrigger,
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

    /// Player-facing view: unidentified monsters show an appearance
    /// description instead of their real name (status/log/simulate).
    #[arg(long, global = true)]
    pub player: bool,

    /// Save folder (identified.yaml, rolls.log). Defaults to TALKDND_SAVE or
    /// the folder above the campaign's combat/ directory.
    #[arg(long, global = true, value_name = "DIR")]
    pub save: Option<PathBuf>,

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

    /// Start an ad-hoc encounter without writing scene/encounter YAML:
    /// a 5-row grid, the party in zone `near`, foes in zone `far`, `--distance` ft apart.
    Quick {
        /// Foes: `id[:count][@zone]` (repeatable; zone `near` or `far`, default far).
        #[arg(long = "foe", value_name = "ID[:N][@ZONE]", required = true)]
        foes: Vec<String>,
        /// Comma-separated party creature ids (default: the campaign party).
        #[arg(long, value_name = "A,B,C")]
        party: Option<String>,
        /// Feet between the two zones.
        #[arg(long, default_value_t = 30)]
        distance: i32,
        /// Light: bright, dim or dark.
        #[arg(long, default_value = "bright")]
        light: String,
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

    /// Resolve one attack or multiattack. Extra Attack data resolves every
    /// attack in the Attack action; use `--unarmed` or `--offhand` for the
    /// special attack forms.
    Attack {
        attacker: String,
        target: String,
        /// Attack id or name (omit with --unarmed).
        #[arg(required_unless_present = "unarmed")]
        attack: Option<String>,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
        #[arg(long, conflicts_with_all = ["unarmed", "offhand"])]
        multi: bool,
        #[arg(long)]
        no_reactions: bool,
        /// Knock out instead of kill if a melee attack drops the target to 0 HP (2024 PHB).
        #[arg(long)]
        nonlethal: bool,
        /// Unarmed Strike: 1 + STR (2014) or 1d3 + STR (2024), bludgeoning,
        /// without weapon properties or masteries.
        #[arg(long, conflicts_with = "offhand")]
        unarmed: bool,
        /// Two-Weapon Fighting: attack with a different Light melee weapon
        /// using the bonus action after a Light main-hand Attack action.
        #[arg(long)]
        offhand: bool,
        /// Resolve exactly one attack from the Attack action and leave the
        /// remaining Extra Attack slots open for a later attack/grapple/shove.
        #[arg(long, conflicts_with_all = ["multi", "offhand"])]
        one: bool,
        /// Attack outside your turn (readied attack, DM ruling).
        #[arg(long)]
        force: bool,
    },

    /// Grapple a creature in reach (2014 Athletics contest, 2024 STR/DEX save).
    Grapple {
        attacker: String,
        target: String,
        /// Target's defense: 2014 `athletics`/`acrobatics`, 2024 `str`/`dex`.
        /// Omit to let the target use its best legal defense (logged).
        #[arg(long, value_name = "DEFENSE")]
        defense: Option<String>,
        /// Act outside your turn (readied action, DM ruling).
        #[arg(long)]
        force: bool,
    },

    /// Shove a creature in reach: knock it down or move it 5 ft away.
    Shove {
        attacker: String,
        target: String,
        /// `knock-down` or `away` (default: knock-down).
        #[arg(long, value_enum, default_value_t = ShoveModeArg::KnockDown)]
        mode: ShoveModeArg,
        /// Target's defense: 2014 `athletics`/`acrobatics`, 2024 `str`/`dex`.
        #[arg(long, value_name = "DEFENSE")]
        defense: Option<String>,
        /// Act outside your turn (readied action, DM ruling).
        #[arg(long)]
        force: bool,
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
        /// Critical hit: double every damage die (modifiers once). Same in 2014 and 2024.
        #[arg(long)]
        crit: bool,
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
        /// Declare a material component verbatim (required for costly or
        /// consumed materials; logged, never matched against inventory).
        #[arg(long, value_name = "TEXT")]
        material: Option<String>,
        /// DM override: skip component declarations (logged as COMPONENTS IGNORED).
        #[arg(long)]
        ignore_components: bool,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
        #[arg(long)]
        no_reactions: bool,
        /// Knock out instead of kill if a melee attack drops the target to 0 HP (2024 PHB).
        #[arg(long)]
        nonlethal: bool,
        /// Act outside your turn with a non-reaction ability (readied action, DM ruling).
        #[arg(long)]
        force: bool,
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
        /// Declare a material component verbatim (required for costly or
        /// consumed materials; logged, never matched against inventory).
        #[arg(long, value_name = "TEXT")]
        material: Option<String>,
        /// DM override: skip component declarations (logged as COMPONENTS IGNORED).
        #[arg(long)]
        ignore_components: bool,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
        #[arg(long)]
        no_reactions: bool,
        /// Knock out instead of kill if a melee attack drops the target to 0 HP (2024 PHB).
        #[arg(long)]
        nonlethal: bool,
        /// Act outside your turn with a non-reaction ability (readied action, DM ruling).
        #[arg(long)]
        force: bool,
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

    /// Prepare one response to a perceivable trigger (Ready action).
    Ready {
        who: String,
        /// Perceivable circumstance, recorded in the combat log.
        #[arg(long)]
        trigger: String,
        /// Structured trigger kind used for automatic matching.
        #[arg(long = "on", value_enum, default_value_t = ReadyTriggerArg::Manual)]
        on: ReadyTriggerArg,
        /// Weapon attack id for the response.
        #[arg(long, conflicts_with_all = ["unarmed", "ability", "zone", "to"])]
        attack: Option<String>,
        /// Unarmed Strike response.
        #[arg(long, conflicts_with_all = ["ability", "zone", "to"])]
        unarmed: bool,
        /// Owned action ability for the response.
        #[arg(long, conflicts_with_all = ["zone", "to"])]
        ability: Option<String>,
        /// Response target; targeted responses default to the triggering creature.
        #[arg(long, conflicts_with_all = ["zone", "to"])]
        target: Option<String>,
        /// Movement response destination zone.
        #[arg(long, conflicts_with = "to")]
        zone: Option<String>,
        /// Movement response exact grid cell `X,Y`.
        #[arg(long, value_name = "X,Y")]
        to: Option<String>,
    },

    /// Fire a prepared response explicitly (DM trigger).
    Trigger {
        who: String,
        /// Creature that caused the trigger; used when the response has no stored target.
        #[arg(long)]
        source: Option<String>,
    },

    /// Clear a prepared response without triggering it.
    ReadyClear { who: String },

    /// Take off or change altitude in a grid scene.
    Fly {
        who: String,
        /// Height above the local floor in feet (0 lands).
        altitude_ft: i32,
    },

    /// Resolve a fall, rolling the R-09 landing check and the 2024 liquid
    /// landing reaction. Needs a save that has explicit spatial state.
    Fall {
        who: String,
        /// Pre-rolled Acrobatics total for the R-09 check (DM override).
        #[arg(long)]
        landing_check: Option<i32>,
        /// Pre-rolled liquid-landing total (DM override).
        #[arg(long)]
        liquid_check: Option<i32>,
        /// Ability used for the 2024 liquid-landing reaction.
        #[arg(long, value_enum, default_value_t = LiquidAbilityArg::Dex)]
        liquid: LiquidAbilityArg,
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
        /// Combatant name or `all` (living party members only).
        who: String,
        /// Short rest: spend this many Hit Dice (die + Con each).
        #[arg(long)]
        hd: Option<u32>,
    },

    /// Persistent party state between encounters (`party-state.yaml` in the save).
    Party {
        #[command(subcommand)]
        command: PartyCommand,
    },

    /// Wake dormant creatures matching a name, creature id or tag.
    Activate { name: String },

    /// Add new combatants to an encounter that already exists. Mid-fight
    /// additions enter the initiative order by total (rolled per combatant,
    /// or `--initiative`); one inserted at or before the current turn
    /// position does not act in the running round. Added combatants join
    /// awake, and the party side writes back to party-state as usual.
    Add {
        /// Creatures to add: `id[:count][@zone]` (repeatable). The zone
        /// defaults to the encounter's party zone.
        #[arg(long = "creature", value_name = "ID[:N][@ZONE]", required = true)]
        creatures: Vec<String>,
        /// Side for the new combatants, for example `party` or `enemy`
        /// (default: each creature's own side).
        #[arg(long, value_name = "SIDE")]
        side: Option<String>,
        /// Fixed initiative total for every added combatant (omit to roll
        /// d20 + initiative bonus per combatant). Needs an initiative order;
        /// before `init` the added combatants are simply added to the fight.
        #[arg(long, value_name = "N")]
        initiative: Option<i32>,
    },

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
    /// List, show or verify loaded content packs.
    Pack {
        #[command(subcommand)]
        command: PackCommand,
    },
    /// Extract the SRD rules prose that 5e-database does not carry.
    ImportSrdProse {
        /// Currently only `2024` has prose gaps.
        #[arg(long, value_name = "EDITION")]
        edition: String,
        /// A pdf2md `index/` directory containing `manifest.json` and pages.
        #[arg(long, value_name = "DIR")]
        index: PathBuf,
        /// Output directory for `<pack-id>.pack.json`.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Import SRD 5.1 / 5.2.1 JSON (5e-database) into content packs.
    ImportSrdPack {
        /// `2014` or `2024`.
        #[arg(long, value_name = "EDITION")]
        edition: String,
        /// Directory holding `5e-SRD-*.json`.
        #[arg(long, value_name = "DIR")]
        src: PathBuf,
        /// Output directory for `<pack-id>.pack.json`.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Convert SRD 5.1 JSON (5e-database) into library YAML.
    ImportSrd {
        /// `5e-SRD-Monsters.json`.
        #[arg(long, value_name = "FILE")]
        monsters: Option<PathBuf>,
        /// `5e-SRD-Spells.json`.
        #[arg(long, value_name = "FILE")]
        spells: Option<PathBuf>,
        /// Output directory (writes srd-monsters.yaml / srd-spells.yaml).
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Library dirs whose hand-made ids win (repeatable).
        #[arg(long = "skip-ids-from", value_name = "DIR")]
        skip_ids_from: Vec<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PackCommand {
    /// List loaded packs and their metadata.
    List,
    /// Print one pack's metadata as JSON.
    Show { id: String },
    /// Verify loaded packs and run the content check.
    Verify,
    /// Build a full-book pack from a local pdf2md index.
    Build {
        /// `2014` or `2024`.
        #[arg(long, value_name = "EDITION")]
        edition: String,
        /// `phb`, `dmg` or `mm`.
        #[arg(long, value_name = "BOOK")]
        book: String,
        /// A pdf2md `index/` directory containing `manifest.json` and pages.
        #[arg(long, value_name = "DIR")]
        index: PathBuf,
        /// Output directory for `<pack-id>.pack.json` (outside Git).
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Validate and report without writing any pack file.
        #[arg(long)]
        dry_run: bool,
    },
    /// Validate full packs and stage a release bundle with a manifest.
    Release {
        /// Directory holding locally built `*.pack.json` full packs.
        #[arg(long, value_name = "DIR")]
        packs: PathBuf,
        /// Output directory for the packs and `full-packs-manifest.json`.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Validate and print the manifest without writing files.
        #[arg(long)]
        dry_run: bool,
    },
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
        /// Exhaustion level (1 through the ruleset maximum).
        #[arg(long)]
        level: Option<u32>,
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
pub enum LiquidAbilityArg {
    Str,
    Dex,
}

impl From<LiquidAbilityArg> for FallAbility {
    fn from(value: LiquidAbilityArg) -> Self {
        match value {
            LiquidAbilityArg::Str => Self::Strength,
            LiquidAbilityArg::Dex => Self::Dexterity,
        }
    }
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
pub enum ShoveModeArg {
    KnockDown,
    Away,
}

impl From<ShoveModeArg> for ShoveMode {
    fn from(value: ShoveModeArg) -> Self {
        match value {
            ShoveModeArg::KnockDown => Self::KnockDown,
            ShoveModeArg::Away => Self::Away,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReadyTriggerArg {
    Manual,
    EnterReach,
    Attacked,
    Ability,
    TurnStart,
}

impl From<ReadyTriggerArg> for ReadyTrigger {
    fn from(value: ReadyTriggerArg) -> Self {
        match value {
            ReadyTriggerArg::Manual => Self::Manual,
            ReadyTriggerArg::EnterReach => Self::EnterReach,
            ReadyTriggerArg::Attacked => Self::Attacked,
            ReadyTriggerArg::Ability => Self::Ability,
            ReadyTriggerArg::TurnStart => Self::TurnStart,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum PartyCommand {
    /// Print current HP, resources and conditions of every party member.
    Show,
    /// Rest outside combat (living members of the manifest party).
    Rest {
        #[arg(value_enum)]
        kind: RestKindArg,
        /// Party creature id or `all`.
        who: String,
        /// Short rest: spend this many Hit Dice.
        #[arg(long)]
        hd: Option<u32>,
    },
    /// Set a member's current HP (DM correction, healing outside combat).
    SetHp { who: String, hp: i32 },
    /// Copy the party from an encounter state into party-state.yaml.
    Sync {
        #[arg(long = "from-state", value_name = "NAME")]
        from_state: Option<PathBuf>,
    },
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
    // The clap command tree is large in debug builds and the Windows main
    // thread has a 1 MiB stack. Parse and dispatch on a thread with headroom so
    // small subcommands also start reliably on every platform.
    let handle = std::thread::Builder::new()
        .name("combat-sim-cli".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(run_cli)
        .context("start combat-sim CLI thread")?;
    match handle.join() {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn run_cli() -> Result<()> {
    let Cli {
        state,
        seed,
        json,
        lib,
        adventure,
        chars,
        campaign,
        player,
        save,
        command,
    } = Cli::parse();

    let sources = ContentSources {
        libraries: lib,
        adventures: adventure,
        characters: chars,
        campaign,
    };
    let save = crate::player::save_dir(save.as_deref(), sources.campaign_path().as_deref());
    let mask = crate::player::PlayerMask::load(save.as_deref());

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
        Command::Quick {
            foes,
            party,
            distance,
            light,
        } => quick_state(&sources, state.as_deref(), &foes, party, distance, &light),
        Command::Init {
            set,
            surprised,
            adv,
            dis,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let surprise = state.content.rules.surprise.clone().unwrap_or_default();
                let surprise_dis = surprise
                    .initiative_disadvantage
                    .unwrap_or(surprise.condition.is_none());
                let no_names: Vec<String> = Vec::new();
                let mut modes = Vec::new();
                for (names, mode) in [
                    (
                        if surprise_dis { &surprised } else { &no_names },
                        RollMode::Disadvantage,
                    ),
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
        Command::Status => {
            print_status(&state_path(&sources, state.as_deref()), json, player, &mask)
        }
        Command::Attack {
            attacker,
            target,
            attack,
            adv,
            dis,
            multi,
            no_reactions,
            nonlethal,
            unarmed,
            offhand,
            one,
            force,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let attacker = canonical_name(state, &attacker)?;
                check_action_turn(state, &attacker, force, lines)?;
                let target = canonical_name(state, &target)?;
                if unarmed {
                    let options = UnarmedOptions {
                        advantage: adv,
                        disadvantage: dis,
                        no_reactions,
                        nonlethal,
                        one,
                    };
                    return engine::attack_unarmed(state, &attacker, &target, &options, rng, lines);
                }
                let attack = attack.ok_or_else(|| {
                    anyhow!("attack requires an attack id/name unless --unarmed is used")
                })?;
                if offhand {
                    let options = AttackOptions {
                        advantage: adv,
                        disadvantage: dis,
                        multi: false,
                        no_reactions,
                        nonlethal,
                    };
                    return engine::attack_offhand_with_options(
                        state, &attacker, &target, &attack, &options, rng, lines,
                    );
                }
                let options = AttackOptions {
                    advantage: adv,
                    disadvantage: dis,
                    multi,
                    no_reactions,
                    nonlethal,
                };
                if one {
                    return engine::attack_one(
                        state, &attacker, &target, &attack, &options, rng, lines,
                    );
                }
                engine::attack(state, &attacker, &target, &attack, &options, rng, lines)
            })
        }
        Command::Grapple {
            attacker,
            target,
            defense,
            force,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let attacker = canonical_name(state, &attacker)?;
                check_action_turn(state, &attacker, force, lines)?;
                let target = canonical_name(state, &target)?;
                let options = GrappleOptions { defense };
                engine::grapple(state, &attacker, &target, &options, rng, lines)
            })
        }
        Command::Shove {
            attacker,
            target,
            mode,
            defense,
            force,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let attacker = canonical_name(state, &attacker)?;
                check_action_turn(state, &attacker, force, lines)?;
                let target = canonical_name(state, &target)?;
                let options = ShoveOptions {
                    defense,
                    mode: mode.into(),
                };
                engine::shove(state, &attacker, &target, &options, rng, lines)
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
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
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
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
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
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
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
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
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
            crit,
        } => {
            // Stateless unless --state names an existing file (then it is logged there too).
            let path = state
                .as_deref()
                .map(|_| state_path(&sources, state.as_deref()));
            let mode = RollMode::from_flags(adv, dis);
            let mut outcome = None;
            let mut output_lines = Vec::new();
            let state_file = path.filter(|p| p.exists());
            let rolled_in_state = state_file.is_some();
            if let Some(path) = state_file {
                run_state_command_quiet(&path, save.as_deref(), seed, |_state, rng, lines| {
                    outcome = Some(engine::roll_expression(
                        &expression,
                        mode,
                        dc,
                        crit,
                        rng,
                        lines,
                    )?);
                    output_lines = lines.clone();
                    Ok(())
                })?;
            } else {
                let mut rng = make_rng(seed);
                outcome = Some(engine::roll_expression(
                    &expression,
                    mode,
                    dc,
                    crit,
                    rng.as_mut(),
                    &mut output_lines,
                )?);
            }
            if json {
                if let Some(outcome) = &outcome {
                    println!("{}", serde_json::to_string_pretty(outcome)?);
                }
            } else {
                print_lines(&output_lines);
            }
            let text = outcome
                .as_ref()
                .map(|o| o.text.clone())
                .unwrap_or_else(|| output_lines.join(" "));
            if !rolled_in_state {
                crate::player::append_roll(save.as_deref(), &expression, &text);
            }
            Ok(())
        }
        Command::Cast {
            who,
            ability,
            targets,
            option,
            slot,
            material,
            ignore_components,
            adv,
            dis,
            no_reactions,
            nonlethal,
            force,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                check_turn(state, &who, &ability, force, lines)?;
                let options = UseOptions {
                    targets,
                    option,
                    slot_level: slot,
                    material,
                    ignore_components,
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
            material,
            ignore_components,
            adv,
            dis,
            no_reactions,
            nonlethal,
            force,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                check_turn(state, &who, &ability, force, lines)?;
                let options = UseOptions {
                    targets: targets.clone(),
                    option,
                    slot_level: slot,
                    material,
                    ignore_components,
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
                level,
                save_ends,
            } => {
                let path = state_path(&sources, state.as_deref());
                run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
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
                        level,
                    };
                    engine::add_condition(state, &who, &condition, &options, rng, lines)
                })
            }
            ConditionCommand::Remove { who, condition } => {
                let path = state_path(&sources, state.as_deref());
                run_state_command(&path, save.as_deref(), seed, |state, _rng, lines| {
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
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                warn_out_of_turn(state, &who, lines);
                engine::move_to(state, &who, &target, no_oa, rng, lines)
            })
        }
        Command::Ready {
            who,
            trigger,
            on,
            attack,
            unarmed,
            ability,
            target,
            zone,
            to,
        } => {
            if (zone.is_some() || to.is_some()) && target.is_some() {
                return Err(anyhow!("--target does not apply to a readied move"));
            }
            let response = if let Some(attack) = attack {
                ReadyResponse::Attack { attack, target }
            } else if unarmed {
                ReadyResponse::Unarmed { target }
            } else if let Some(ability) = ability {
                ReadyResponse::Ability { ability, target }
            } else if let Some(zone) = zone {
                ReadyResponse::Move {
                    zone: Some(zone),
                    cell: None,
                }
            } else if let Some(to) = to {
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
                ReadyResponse::Move {
                    zone: None,
                    cell: Some([x, y]),
                }
            } else {
                return Err(anyhow!(
                    "ready needs one response: --attack, --unarmed, --ability, --zone or --to"
                ));
            };
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, _rng, lines| {
                let who = canonical_name(state, &who)?;
                check_own_turn(state, &who, "take the Ready action")?;
                let options = ReadyOptions {
                    trigger: on.into(),
                    description: trigger,
                    response,
                };
                engine::ready(state, &who, &options, lines)
            })
        }
        Command::Trigger { who, source } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                let source = source
                    .as_deref()
                    .map(|name| canonical_name(state, name))
                    .transpose()?;
                engine::trigger_ready(state, &who, source.as_deref(), rng, lines)?;
                Ok(())
            })
        }
        Command::ReadyClear { who } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, _rng, lines| {
                let who = canonical_name(state, &who)?;
                if !engine::clear_ready(state, &who, "cleared by command", lines)? {
                    bail!("`{who}` has no readied action");
                }
                Ok(())
            })
        }
        Command::Fly { who, altitude_ft } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, _rng, lines| {
                let who = canonical_name(state, &who)?;
                warn_out_of_turn(state, &who, lines);
                engine::fly(state, &who, altitude_ft, lines)
            })
        }
        Command::Fall {
            who,
            landing_check,
            liquid_check,
            liquid,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                let options = FallOptions {
                    landing_check_total: landing_check,
                    liquid_landing_total: liquid_check,
                };
                let report =
                    engine::fall_with_checks(state, &who, options, liquid.into(), rng, lines)?;
                if report.is_none() {
                    return Err(anyhow!(
                        "this state has no spatial fall map; take off with `fly` or add explicit spatial state first"
                    ));
                }
                Ok(())
            })
        }
        Command::Interact {
            who,
            object,
            option,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                warn_out_of_turn(state, &who, lines);
                engine::interact(state, &who, &object, option, rng, lines)
            })
        }
        Command::Scene => print_scene(&state_path(&sources, state.as_deref())),
        Command::Deathsave { who } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let who = canonical_name(state, &who)?;
                engine::death_save(state, &who, rng, lines)
            })
        }
        Command::Next => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                engine::next_turn(state, rng, lines)
            })
        }
        Command::Log { tail } => {
            print_log(&state_path(&sources, state.as_deref()), tail, player, &mask)
        }
        Command::Rest { kind, who, hd } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let who = if who.eq_ignore_ascii_case("all") {
                    "all".to_string()
                } else {
                    canonical_name(state, &who)?
                };
                engine::rest(state, kind.into(), Some(&who), hd, rng, lines)
            })
        }
        Command::Activate { name } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                engine::activate(state, &name, rng, lines)
            })
        }
        Command::Add {
            creatures,
            side,
            initiative,
        } => {
            let path = state_path(&sources, state.as_deref());
            run_state_command(&path, save.as_deref(), seed, |state, rng, lines| {
                let adds = creatures
                    .iter()
                    .map(|spec| AddSpec::parse(spec))
                    .collect::<Result<Vec<_>>>()?;
                state.add_combatants(&adds, side.as_deref(), initiative, rng, lines)?;
                Ok(())
            })
        }
        Command::Content { command } => run_content(&sources, command),
        Command::Party { command } => run_party(&sources, state.as_deref(), seed, command),
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
            player,
            &mask,
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
        ContentCommand::ImportSrdProse {
            edition,
            index,
            out,
        } => {
            let report = crate::srd_prose::import(&edition, &index, &out)?;
            for chapter in &report.chapters {
                println!(
                    "{}\tp.{}-{}\t{} sections",
                    chapter.name, chapter.start_page, chapter.end_page, chapter.sections
                );
            }
            for duplicate in &report.duplicates {
                println!("duplicate: {duplicate}");
            }
            for gap in &report.gaps {
                println!("gap: {gap}");
            }
            println!(
                "{}: {} entities, {} short headings skipped",
                report.pack_id, report.entities, report.skipped
            );
            Ok(())
        }
        ContentCommand::ImportSrdPack { edition, src, out } => {
            let report = crate::srd_pack::import(&edition, &src, &out)?;
            for pack in &report.packs {
                println!(
                    "{}\t{}\t{}\t{}",
                    pack.id, pack.entities, pack.file, pack.summary
                );
            }
            for gap in &report.gaps {
                println!("gap: {gap}");
            }
            println!(
                "imported {} pack(s) for SRD {}",
                report.packs.len(),
                report.edition
            );
            Ok(())
        }
        ContentCommand::ImportSrd {
            monsters,
            spells,
            out,
            skip_ids_from,
        } => {
            let skip = crate::srd::existing_ids(&skip_ids_from)?;
            std::fs::create_dir_all(&out)?;
            let read = |p: &PathBuf| -> Result<serde_json::Value> {
                let t = std::fs::read_to_string(p).with_context(|| p.display().to_string())?;
                serde_json::from_str(&t).with_context(|| p.display().to_string())
            };
            for (src, name, conv) in [
                (
                    monsters,
                    "srd-monsters.yaml",
                    crate::srd::convert_monsters
                        as fn(
                            &serde_json::Value,
                            &std::collections::BTreeSet<String>,
                        ) -> (serde_json::Value, usize),
                ),
                (spells, "srd-spells.yaml", crate::srd::convert_spells),
            ] {
                let Some(src) = src else { continue };
                let (v, n) = conv(&read(&src)?, &skip);
                let path = out.join(name);
                let text = format!(
                    "{}{}",
                    crate::srd::ATTRIBUTION,
                    serde_norway::to_string(&v)?
                );
                crate::fsutil::atomic_write(&path, text.as_bytes())
                    .with_context(|| path.display().to_string())?;
                println!("wrote {} ({n} item(s))", path.display());
            }
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
        ContentCommand::Pack { command } => {
            match &command {
                PackCommand::Build {
                    edition,
                    book,
                    index,
                    out,
                    dry_run,
                } => {
                    let report =
                        crate::content::full_pack::build(edition, book, index, out, *dry_run)?;
                    for gap in &report.gaps {
                        println!("gap: {gap}");
                    }
                    println!(
                        "{}: {} sections, {} chapters, {} images, {} pages, {} skipped heading(s)",
                        report.pack_id,
                        report.sections,
                        report.chapters,
                        report.images,
                        report.pages,
                        report.skipped_headings
                    );
                    if *dry_run {
                        println!("dry run: would write {}", out.join(&report.file).display());
                    } else {
                        println!("wrote {}", out.join(&report.file).display());
                    }
                    return Ok(());
                }
                PackCommand::Release {
                    packs,
                    out,
                    dry_run,
                } => {
                    if !dry_run && out.is_none() {
                        bail!(
                            "--out is required for `content pack release` unless --dry-run is used"
                        );
                    }
                    let report = crate::content::full_pack::release(
                        packs,
                        if *dry_run { None } else { out.as_deref() },
                    )?;
                    if *dry_run {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&report.to_value())
                                .context("serialize full-pack release manifest")?
                        );
                        println!(
                            "dry run: {} full pack(s), {} bytes; nothing written",
                            report.packs.len(),
                            report.total_bytes
                        );
                    } else {
                        for pack in &report.packs {
                            println!(
                                "{}\t{}\t{} bytes\t{}",
                                pack.id, pack.file, pack.bytes, pack.content_hash
                            );
                        }
                        println!(
                            "{} full pack(s), {} bytes written to {}",
                            report.packs.len(),
                            report.total_bytes,
                            out.as_deref().unwrap_or_else(|| Path::new(".")).display()
                        );
                    }
                    return Ok(());
                }
                _ => {}
            }
            let registry = load_content(sources)?;
            match command {
                PackCommand::List => {
                    if registry.packs.is_empty() {
                        println!("no content packs loaded");
                    } else {
                        for (id, meta) in &registry.packs {
                            println!(
                                "{}\t{}\t{}\t{}\t{}",
                                id,
                                meta.edition,
                                meta.distribution.as_str(),
                                meta.source.as_deref().unwrap_or("(none)"),
                                meta.content_hash.as_deref().unwrap_or("(none)")
                            );
                        }
                    }
                }
                PackCommand::Show { id } => {
                    let meta = registry
                        .packs
                        .get(&id)
                        .ok_or_else(|| anyhow!("no content pack with id `{id}`"))?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(meta)
                            .context("failed to serialize content pack metadata")?
                    );
                }
                PackCommand::Verify => {
                    if registry.packs.is_empty() {
                        println!("no content packs loaded");
                    } else {
                        for (id, meta) in &registry.packs {
                            println!(
                                "{id}: verified (edition {}, distribution {}, hash {})",
                                meta.edition,
                                meta.distribution.as_str(),
                                meta.content_hash.as_deref().unwrap_or("(none)")
                            );
                        }
                    }
                    let report = content::check(&registry);
                    for issue in &report.issues {
                        println!("{issue}");
                    }
                    println!(
                        "content pack verify: {} error(s), {} warning(s)",
                        report.error_count(),
                        report.warning_count()
                    );
                    if report.has_errors() {
                        bail!(
                            "content pack verify failed with {} error(s)",
                            report.error_count()
                        );
                    }
                }
                PackCommand::Build { .. } | PackCommand::Release { .. } => {
                    unreachable!("build and release are handled before loading a library")
                }
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

/// Name of the active combatant, or `None` when turn tracking does not apply
/// (no initiative rolled yet, or the fight has already ended).
fn active_combatant(state: &EncounterState) -> Option<&str> {
    if state.initiative.is_empty() || state.outcome.is_some() {
        return None;
    }
    let &active = state.initiative.get(state.turn)?;
    Some(state.combatants[active].name.as_str())
}

/// Warns (does not block) when someone other than the active combatant moves
/// or interacts: neither consumes an action in this engine, so out-of-turn
/// use stays a bookkeeping warning for the DM.
fn warn_out_of_turn(state: &EncounterState, actor: &str, lines: &mut Vec<String>) {
    let Some(active_name) = active_combatant(state) else {
        return;
    };
    if !active_name.eq_ignore_ascii_case(actor) {
        lines.push(format!(
            "WARNING: {actor} is acting out of turn (round {}, active: {active_name}). OK for reactions/readied actions; otherwise run `next` first.",
            state.round
        ));
    }
}

/// Shared tail of the turn gates: `--force` logs an audit WARNING and lets
/// the action proceed; without it the command fails here, before any roll,
/// log line, action spend or other state change. Callers must have checked
/// that `actor` is not the active combatant.
fn force_or_deny(
    state: &EncounterState,
    actor: &str,
    action: &str,
    exception: &str,
    force: bool,
    lines: &mut Vec<String>,
) -> Result<()> {
    let active_name = active_combatant(state).expect("caller checked the active combatant");
    if force {
        lines.push(format!(
            "WARNING: {actor} acts out of turn to {action} (round {}, active: {active_name}) with --force",
            state.round
        ));
        return Ok(());
    }
    bail!(
        "{actor} cannot {action} now: it is {active_name}'s turn (round {}). {exception}Run `next` first, or add --force for a readied action or DM ruling",
        state.round
    )
}

/// Blocks an attack outside the attacker's turn unless `--force` (readied
/// attack or DM ruling). Unlike [`check_turn`] there is no reaction
/// exception: an attack always spends the attacker's action.
fn check_action_turn(
    state: &EncounterState,
    attacker: &str,
    force: bool,
    lines: &mut Vec<String>,
) -> Result<()> {
    let Some(active_name) = active_combatant(state) else {
        return Ok(());
    };
    if active_name.eq_ignore_ascii_case(attacker) {
        return Ok(());
    }
    force_or_deny(state, attacker, "attack", "", force, lines)
}

/// Blocks an action that is only legal on the actor's own turn (Ready). It has
/// no `--force` escape hatch: a readied response is the out-of-turn action.
fn check_own_turn(state: &EncounterState, actor: &str, action: &str) -> Result<()> {
    let Some(active_name) = active_combatant(state) else {
        return Ok(());
    };
    if active_name.eq_ignore_ascii_case(actor) {
        return Ok(());
    }
    bail!(
        "{actor} cannot {action} now: it is {active_name}'s turn (round {}). Run `next` first",
        state.round
    )
}

/// Blocks a non-reaction ability outside the actor's turn unless `--force`
/// (readied action or DM ruling); reactions stay allowed.
fn check_turn(
    state: &EncounterState,
    actor: &str,
    ability: &str,
    force: bool,
    lines: &mut Vec<String>,
) -> Result<()> {
    let Some(active_name) = active_combatant(state) else {
        return Ok(());
    };
    if active_name.eq_ignore_ascii_case(actor) {
        return Ok(());
    }
    let reaction = state
        .find_index(actor)
        .ok()
        .and_then(|i| {
            state.combatants[i]
                .find_owned_ability(&state.content, ability)
                .ok()
        })
        .is_some_and(|r| r.ability.activation == crate::content::schema::Activation::Reaction);
    if reaction {
        return Ok(());
    }
    force_or_deny(
        state,
        actor,
        &format!("use {ability}"),
        "Only reactions work out of turn. ",
        force,
        lines,
    )
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

fn run_state_command<F>(
    state_path: &Path,
    save: Option<&Path>,
    seed: Option<u64>,
    action: F,
) -> Result<()>
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
    write_party_state(&state)?;
    crate::player::append_combat_lines(save, &lines);
    print_lines(&lines);
    Ok(())
}

/// Like [`run_state_command`] but does not print the log lines.
fn run_state_command_quiet<F>(
    state_path: &Path,
    save: Option<&Path>,
    seed: Option<u64>,
    action: F,
) -> Result<()>
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
    write_party_state(&state)?;
    crate::player::append_combat_lines(save, &lines);
    Ok(())
}

fn write_party_state(state: &EncounterState) -> Result<()> {
    let Some(path) = state.party_state.as_deref() else {
        return Ok(());
    };
    let mut party = crate::party::PartyState::load(path)?;
    party.record(state);
    party.save(path)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let a = if a.as_os_str().is_empty() {
        Path::new(".")
    } else {
        a
    };
    let b = if b.as_os_str().is_empty() {
        Path::new(".")
    } else {
        b
    };
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn party_state_path(sources: &ContentSources) -> Option<PathBuf> {
    manifest_state_dir(sources).map(|dir| dir.join(crate::party::FILE_NAME))
}

fn run_party(
    sources: &ContentSources,
    requested: Option<&Path>,
    seed: Option<u64>,
    command: PartyCommand,
) -> Result<()> {
    use crate::party::{Member, PartyState};
    let path = party_state_path(sources).ok_or_else(|| {
        anyhow!("party state needs a campaign manifest with `state_dir` (run inside a save or pass --campaign)")
    })?;
    let mut party = PartyState::load(&path)?;
    // Members of the manifest party that are not recorded yet start fresh.
    if let (Some(manifest), Ok(content)) = (read_manifest(sources), load_content(sources)) {
        for id in &manifest.party {
            if !party.members.contains_key(id) {
                if let Ok(creature) = content.resolve_creature(id) {
                    party
                        .members
                        .insert(id.clone(), Member::from_creature(&creature));
                }
            }
        }
    }
    let dying_conditions = load_content(sources)
        .ok()
        .and_then(|content| content.rules.dying_conditions.clone())
        .unwrap_or_else(|| vec![crate::party::DEFAULT_DYING_CONDITION.to_string()]);
    let mut lines = Vec::new();
    let pick = |party: &PartyState, who: &str| -> Result<Vec<String>> {
        if who.eq_ignore_ascii_case("all") {
            return Ok(party.members.keys().cloned().collect());
        }
        party
            .members
            .iter()
            .find(|(id, m)| id.eq_ignore_ascii_case(who) || m.name.eq_ignore_ascii_case(who))
            .map(|(id, _)| vec![id.clone()])
            .ok_or_else(|| {
                anyhow!(
                    "no party member `{who}`; known: {}",
                    party.members.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            })
    };
    match command {
        PartyCommand::Show => {
            if party.members.is_empty() {
                println!("party state is empty (no `party:` in the manifest yet)");
            }
            for line in party.summary_lines() {
                println!("{line}");
            }
            return Ok(());
        }
        PartyCommand::Rest { kind, who, hd } => {
            let ids = pick(&party, &who)?;
            let mut rng = make_rng(seed);
            for id in ids {
                let member = party.members.get_mut(&id).expect("picked");
                match kind {
                    RestKindArg::Short => {
                        member.short_rest(hd.unwrap_or(0), rng.as_mut(), &mut lines)?
                    }
                    RestKindArg::Long => {
                        if hd.is_some() {
                            bail!("--hd is only for short rests");
                        }
                        member.long_rest(&dying_conditions, &mut lines)
                    }
                }
            }
        }
        PartyCommand::SetHp { who, hp } => {
            for id in pick(&party, &who)? {
                let member = party.members.get_mut(&id).expect("picked");
                member.set_hp(hp, &dying_conditions);
                lines.push(format!(
                    "{} HP set to {}/{}",
                    member.name, member.hp, member.max_hp
                ));
            }
        }
        PartyCommand::Sync { from_state } => {
            let state_file = state_path(sources, from_state.as_deref().or(requested));
            let state = EncounterState::load(&state_file)
                .with_context(|| format!("failed to load state {}", state_file.display()))?;
            party.record(&state);
            lines.push(format!("party state updated from {}", state_file.display()));
        }
    }
    party.save(&path)?;
    print_lines(&lines);
    for line in party.summary_lines() {
        println!("{line}");
    }
    Ok(())
}

/// YAML for the ad-hoc `quick` scene and encounter.
pub fn quick_yaml(foes: &[String], distance: i32, light: &str) -> Result<String> {
    if !["bright", "dim", "dark"].contains(&light) {
        bail!("--light must be bright, dim or dark");
    }
    if distance < 0 {
        bail!("--distance must be 0 or more");
    }
    let mut members = String::new();
    for spec in foes {
        let (rest, zone) = match spec.split_once('@') {
            Some((r, z)) => (r, z.trim()),
            None => (spec.as_str(), "far"),
        };
        if zone != "near" && zone != "far" {
            bail!("foe zone must be `near` or `far`, got `{zone}`");
        }
        let (id, count) = match rest.split_once(':') {
            Some((i, n)) => (
                i.trim(),
                n.trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| anyhow!("bad count in `{spec}`"))?,
            ),
            None => (rest.trim(), 1),
        };
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("bad creature id in `{spec}`");
        }
        members.push_str(&format!(
            "      - {{creature: {id}, count: {count}, zone: {zone}}}\n"
        ));
    }
    // A grid so creatures really close the distance: 5 rows, the party in
    // the left column (`n`), foes in the right one (`f`).
    let cols = (distance / 5).max(1) as usize + 1;
    let row = format!("n{}f", ".".repeat(cols - 1));
    let rows: String = (0..5).map(|_| format!("        - \"{row}\"\n")).collect();
    Ok(format!(
        "scenes:\n  - id: quick-scene\n    name: Quick encounter\n    mode: grid\n    light: {light}\n    grid:\n      cell: 5\n      legend:\n        \".\": {{}}\n        \"n\": {{zone: near}}\n        \"f\": {{zone: far}}\n      rows:\n{rows}encounters:\n  - id: quick\n    name: Quick encounter\n    scene: quick-scene\n    party_zone: near\n    members:\n{members}"
    ))
}

fn quick_state(
    sources: &ContentSources,
    requested: Option<&Path>,
    foes: &[String],
    party: Option<String>,
    distance: i32,
    light: &str,
) -> Result<()> {
    let yaml = quick_yaml(foes, distance, light)?;
    let dir = std::env::temp_dir().join(format!("combat-sim-quick-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("quick.yaml");
    crate::fsutil::atomic_write(&file, yaml.as_bytes())?;
    let mut sources = sources.clone();
    sources.adventures.push(file);
    let result = new_state(&sources, requested, "quick".into(), party, None, Vec::new());
    let _ = std::fs::remove_dir_all(&dir);
    result
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
    let mut state = EncounterState::from_encounter(content, &options)?;
    let state_dir = manifest_state_dir(sources);
    let path = resolve_state_path(requested, state_dir.as_deref());
    // Party state is used only for encounters saved in the save's state_dir,
    // so scratch states elsewhere never touch it.
    let in_state_dir = match (&state_dir, path.parent()) {
        (Some(dir), Some(parent)) => same_dir(dir, parent),
        _ => false,
    };
    if in_state_dir {
        if let Some(party_path) = party_state_path(sources) {
            let party = crate::party::PartyState::load(&party_path)?;
            let mut applied = Vec::new();
            for combatant in &mut state.combatants {
                if combatant.side.as_str() != "party" {
                    continue;
                }
                if let Some(member) = party.members.get(combatant.creature_id()) {
                    member.apply(combatant);
                    applied.push(format!(
                        "{} HP {}/{}",
                        combatant.name, combatant.hp, combatant.max_hp
                    ));
                }
            }
            if !applied.is_empty() {
                state
                    .log
                    .push(format!("Party state loaded: {}", applied.join(", ")));
            }
            let absolute = party_path
                .parent()
                .and_then(|dir| dir.canonicalize().ok())
                .map(|dir| dir.join(crate::party::FILE_NAME))
                .unwrap_or(party_path);
            state.party_state = Some(absolute);
        }
    }
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
    player: bool,
    mask: &crate::player::PlayerMask,
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
    let replacements = if player {
        mask.replacements(&state)
    } else {
        Vec::new()
    };
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
            let mut value =
                serde_json::to_value(&output).context("failed to serialize simulation")?;
            if player {
                if let Some(lines) = value.get_mut("log").and_then(|v| v.as_array_mut()) {
                    for line in lines {
                        if let Some(text) = line.as_str().map(str::to_string) {
                            *line = serde_json::Value::String(mask.line(&text, &replacements));
                        }
                    }
                }
                if let Some(list) = value
                    .get_mut("summary")
                    .and_then(|v| v.get_mut("combatants"))
                    .and_then(|v| v.as_array_mut())
                {
                    for c in list {
                        if let Some(name) =
                            c.get("name").and_then(|v| v.as_str()).map(str::to_string)
                        {
                            c["name"] = serde_json::Value::String(mask.line(&name, &replacements));
                        }
                    }
                }
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&value).context("failed to serialize simulation")?
            );
        } else {
            for line in &log {
                println!("{}", mask.line(line, &replacements));
            }
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
                println!(
                    "  {} ({}): {}",
                    mask.line(&combatant.name, &replacements),
                    combatant.side,
                    status
                );
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
                mask.line(&creature.name, &replacements),
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

fn print_status(
    state_path: &Path,
    json: bool,
    player: bool,
    mask: &crate::player::PlayerMask,
) -> Result<()> {
    let state = EncounterState::load(state_path)
        .with_context(|| format!("failed to load state {}", state_path.display()))?;
    if json {
        let output = StatusOutput::from_state(&state);
        let mut value = serde_json::to_value(&output).context("failed to serialize status JSON")?;
        if player {
            crate::player::mask_status_json(&mut value, &state, mask);
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&value).context("failed to serialize status JSON")?
        );
    } else {
        let replacements = if player {
            mask.replacements(&state)
        } else {
            Vec::new()
        };
        for line in engine::status_lines(&state) {
            println!("{}", mask.line(&line, &replacements));
        }
    }
    Ok(())
}

fn print_log(
    state_path: &Path,
    tail: Option<usize>,
    player: bool,
    mask: &crate::player::PlayerMask,
) -> Result<()> {
    let state = EncounterState::load(state_path)
        .with_context(|| format!("failed to load state {}", state_path.display()))?;
    let start = match tail {
        Some(tail) => state.log.len().saturating_sub(tail),
        None => 0,
    };
    let replacements = if player {
        mask.replacements(&state)
    } else {
        Vec::new()
    };
    for line in &state.log[start..] {
        println!("{}", mask.line(line, &replacements));
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
    effective_max_hp: i32,
    bloodied: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    ready: Option<ReadyState>,
    /// Raw `Creature.senses` text, preserved exactly as authored.
    #[serde(skip_serializing_if = "Option::is_none")]
    senses: Option<String>,
    /// Parsed vision facts for this combatant.
    vision: VisionStatus,
}

#[derive(Debug, Serialize)]
struct VisionStatus {
    light: Light,
    effective_light: Light,
    obscurement: Obscurement,
    #[serde(skip_serializing_if = "Option::is_none")]
    darkvision_ft: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blindsight_ft: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tremorsense_ft: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truesight_ft: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    passive_perception: Option<i32>,
}

#[derive(Debug, Serialize)]
struct ConditionStatus {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<u32>,
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
                let vision = engine::vision_profile(state, index);
                CombatantStatus {
                    name: combatant.name.clone(),
                    side: combatant.side.as_str().to_string(),
                    kind: combatant.kind(),
                    initiative: combatant.initiative_total,
                    current: current == Some(index),
                    hp: combatant.hp,
                    max_hp: combatant.max_hp,
                    effective_max_hp: engine::effective_max_hp(state, index),
                    bloodied: engine::is_bloodied(state, index),
                    temp_hp: combatant.temp_hp,
                    ac: engine::armor_class(state, index),
                    zone: combatant.zone.clone(),
                    pos: combatant.cell(),
                    conditions: {
                        let exhaustion = engine::exhaustion_condition_id(state);
                        let mut conditions: Vec<ConditionStatus> = combatant
                            .conditions
                            .iter()
                            .map(|condition| ConditionStatus {
                                id: condition.condition.clone(),
                                level: condition
                                    .condition
                                    .eq_ignore_ascii_case(exhaustion)
                                    .then_some(combatant.exhaustion_level),
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
                            .collect();
                        if combatant.exhaustion_level > 0
                            && !conditions
                                .iter()
                                .any(|condition| condition.id.eq_ignore_ascii_case(exhaustion))
                        {
                            conditions.push(ConditionStatus {
                                id: exhaustion.to_string(),
                                level: Some(combatant.exhaustion_level),
                                source: None,
                                linked: None,
                                remaining: None,
                                save_ends: None,
                                concentration: false,
                            });
                        }
                        conditions
                    },
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
                    death_saves: if combatant.makes_death_saves() {
                        Some(combatant.death_saves)
                    } else {
                        None
                    },
                    dormant: combatant.dormant,
                    concentrating: state.concentration.contains_key(&combatant.name),
                    dead: combatant.is_dead(),
                    knocked_out: combatant.knocked_out,
                    speed_left: combatant.speed_left,
                    ready: combatant.ready.clone(),
                    senses: combatant.creature.senses.clone(),
                    vision: VisionStatus {
                        light: vision.light,
                        effective_light: vision.effective_light,
                        obscurement: vision.obscurement,
                        darkvision_ft: vision.senses.darkvision,
                        blindsight_ft: vision.senses.blindsight,
                        tremorsense_ft: vision.senses.tremorsense,
                        truesight_ft: vision.senses.truesight,
                        passive_perception: vision.senses.passive_perception,
                    },
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
