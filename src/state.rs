//! Runtime encounter state and its persisted JSON form.
//!
//! A state file is self-contained: it embeds the loaded content registry, the
//! scene it uses and every combatant's resolved creature. Step commands only
//! need `--state`, never a campaign manifest.

#![allow(clippy::too_many_arguments)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::content::schema::{
    AbilityName, AiAssignment, ConditionDef, Creature, CreatureKind, Duration, DurationUntil,
    EndsOn, ResourceDef, Rules, SaveEndsAt, Scene, SceneMode, SceneTrigger, Side, Speed,
};
use crate::content::{ContentRegistry, ResolvedAbility};
use crate::space::{GridMap, GridSpace, Position, SpaceCache, SpaceView, ZoneGraph};

/// State file used when no `--state` is supplied.
pub const DEFAULT_STATE_FILE: &str = "combat-state.json";

/// Suffix appended to bare state names.
pub const STATE_SUFFIX: &str = ".state.json";

/// A persisted encounter.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncounterState {
    /// Self-contained content snapshot used by every later command.
    #[serde(default)]
    pub content: Arc<ContentRegistry>,
    /// The chosen scene, including any trigger `enabled` flags toggled at runtime.
    #[serde(default)]
    pub scene: Option<Scene>,
    #[serde(default)]
    pub encounter_name: Option<String>,
    #[serde(default)]
    pub party_zone: Option<String>,
    #[serde(default)]
    pub combatants: Vec<CombatantState>,
    /// Indices into `combatants`, in initiative order.
    #[serde(default)]
    pub initiative: Vec<usize>,
    /// Index into `initiative` for the active combatant.
    #[serde(default)]
    pub turn: usize,
    /// Current round number, starting at 0 before initiative is rolled.
    #[serde(default)]
    pub round: u32,
    #[serde(default)]
    pub log: Vec<String>,
    /// Concentration per combatant instance name.
    #[serde(default)]
    pub concentration: BTreeMap<String, ConcentrationState>,
    /// Scene-trigger runtime (`once`, `enabled`), keyed by trigger id.
    #[serde(default)]
    pub triggers: BTreeMap<String, SceneTriggerRuntime>,
    /// Once a side has won or the fight is a draw, the recorded outcome.
    #[serde(default)]
    pub outcome: Option<CombatOutcome>,
    /// Spatial indices derived from `scene`. Shared by every clone of the
    /// state; rebuilt after loading a state file.
    #[serde(skip)]
    space_cache: Arc<SpaceCache>,
    /// Lowercased combatant names for fast `find_index` lookups. Rebuilt
    /// whenever the combatant list or its names change.
    #[serde(skip)]
    name_index: Arc<NameIndex>,
}

impl PartialEq for EncounterState {
    fn eq(&self, other: &Self) -> bool {
        self.content == other.content
            && self.scene == other.scene
            && self.encounter_name == other.encounter_name
            && self.party_zone == other.party_zone
            && self.combatants == other.combatants
            && self.initiative == other.initiative
            && self.turn == other.turn
            && self.round == other.round
            && self.log == other.log
            && self.concentration == other.concentration
            && self.triggers == other.triggers
            && self.outcome == other.outcome
    }
}

/// Cached lowercase combatant names, parallel to `combatants`.
#[derive(Debug, Default)]
struct NameIndex {
    names: Vec<String>,
}

impl NameIndex {
    fn build(combatants: &[CombatantState]) -> Self {
        Self {
            names: combatants
                .iter()
                .map(|combatant| combatant.name.to_lowercase())
                .collect(),
        }
    }
}

/// Concentration bookkeeping for one caster.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConcentrationState {
    #[serde(default)]
    pub ability: Option<String>,
    #[serde(default)]
    pub round: u32,
}

/// Runtime state for a scene trigger.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SceneTriggerRuntime {
    #[serde(default)]
    pub fired: bool,
    #[serde(default)]
    pub enabled: bool,
}

/// Combat result once at most one side remains active.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CombatOutcome {
    Winner(String),
    Draw,
}

impl fmt::Display for CombatOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Winner(side) => write!(f, "{side} wins"),
            Self::Draw => f.write_str("draw"),
        }
    }
}

/// A combatant: the resolved creature plus mutable runtime state.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CombatantState {
    /// Resolved creature data, embedded so the state file is self-contained.
    pub creature: Arc<Creature>,
    /// Instance name, unique within the encounter.
    pub name: String,
    pub side: Side,
    pub max_hp: i32,
    pub hp: i32,
    #[serde(default)]
    pub temp_hp: i32,
    pub zone: String,
    /// Exact position. `None` in older state files means the display zone alone.
    #[serde(default)]
    pub pos: Option<Position>,
    #[serde(default)]
    pub conditions: Vec<ActiveCondition>,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceState>,
    #[serde(default)]
    pub death_saves: DeathSaves,
    #[serde(default)]
    pub action_used: bool,
    #[serde(default)]
    pub bonus_action_used: bool,
    /// Round in which the reaction was spent.
    #[serde(default)]
    pub reaction_round: Option<u32>,
    #[serde(default)]
    pub speed_left: i32,
    /// Once-per-turn/round keys mapped to the round in which they were used.
    #[serde(default)]
    pub once_used: BTreeMap<String, u32>,
    #[serde(default)]
    pub ai: Option<AiAssignment>,
    #[serde(default)]
    pub initiative_total: Option<i32>,
    #[serde(default)]
    pub dormant: bool,
    /// Whether an AI opening ability has already been considered on turn one.
    #[serde(default)]
    pub opening_used: bool,
    /// Dead flag used for creatures without a configured dead condition.
    #[serde(default)]
    pub dead: bool,
}

impl CombatantState {
    pub fn display_name(&self) -> &str {
        &self.name
    }

    /// Exact position, falling back to the display zone for older state files.
    pub fn position(&self) -> Position {
        self.pos.clone().unwrap_or_else(|| Position::Zone {
            zone: self.zone.clone(),
        })
    }

    pub fn cell(&self) -> Option<[i32; 2]> {
        match &self.pos {
            Some(Position::Cell { x, y }) => Some([*x, *y]),
            _ => None,
        }
    }

    pub fn set_cell(&mut self, x: i32, y: i32, zone: impl Into<String>) {
        self.pos = Some(Position::Cell { x, y });
        self.zone = zone.into();
    }

    pub fn set_zone(&mut self, zone: impl Into<String>) {
        let zone = zone.into();
        self.pos = Some(Position::Zone { zone: zone.clone() });
        self.zone = zone;
    }

    pub fn creature_id(&self) -> &str {
        &self.creature.id
    }

    pub fn tags(&self) -> &[String] {
        self.creature.tags.as_deref().unwrap_or(&[])
    }

    pub fn kind(&self) -> CreatureKind {
        self.creature.kind()
    }

    pub fn is_pc(&self) -> bool {
        self.kind() == CreatureKind::Pc
    }

    pub fn is_dead(&self) -> bool {
        self.dead || self.death_saves.dead
    }

    /// A member counts as active when it is awake, alive and not dormant.
    pub fn is_active(&self) -> bool {
        !self.dormant && self.hp > 0 && !self.is_dead()
    }

    pub fn is_dying(&self) -> bool {
        self.is_pc() && self.hp <= 0 && !self.is_dead() && !self.death_saves.stable
    }

    pub fn has_condition(&self, content: &ContentRegistry, condition: &str) -> bool {
        active_condition_ids(content, self)
            .iter()
            .any(|id| id.eq_ignore_ascii_case(condition))
    }

    pub fn is_immune_to_condition(&self, condition: &str) -> bool {
        self.creature
            .condition_immunities
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .any(|immune| immune.eq_ignore_ascii_case(condition))
    }

    pub fn ability_modifier(&self, ability: AbilityName) -> i32 {
        self.creature.ability_modifier(ability)
    }

    /// Saving throw bonus: explicit save value when present, otherwise the
    /// ability modifier.
    pub fn save_bonus(&self, ability: AbilityName) -> i32 {
        self.creature
            .saves
            .as_ref()
            .and_then(|saves| saves.get(ability.as_str()))
            .copied()
            .unwrap_or_else(|| self.creature.ability_modifier(ability))
    }

    /// Skill bonus: explicit skill value when present, otherwise the plain
    /// ability modifier for an ability name.
    pub fn skill_bonus(&self, skill: &str) -> i32 {
        self.creature
            .skills
            .as_ref()
            .and_then(|skills| {
                skills
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(skill))
                    .map(|(_, bonus)| *bonus)
            })
            .unwrap_or_else(|| {
                ability_from_text(skill)
                    .map(|ability| self.creature.ability_modifier(ability))
                    .unwrap_or(0)
            })
    }

    /// Base walking speed before condition modifiers.
    pub fn base_speed(&self) -> i32 {
        let Speed { walk, .. } = self.creature.speed();
        walk.max(0)
    }

    pub fn has_reaction(&self, round: u32) -> bool {
        !self.is_dead() && self.reaction_round != Some(round)
    }

    pub fn resource(&self, id: &str) -> Option<&ResourceState> {
        self.resources.get(id)
    }

    pub fn resource_mut(&mut self, id: &str) -> Option<&mut ResourceState> {
        self.resources.get_mut(id)
    }

    /// Resolves an owned ability reference (`actions` then `features`).
    pub fn find_owned_ability(
        &self,
        content: &ContentRegistry,
        query: &str,
    ) -> Result<ResolvedAbility> {
        let mut entries = Vec::new();
        if let Some(actions) = &self.creature.actions {
            entries.extend(actions.iter());
        }
        if let Some(features) = &self.creature.features {
            entries.extend(features.iter());
        }
        let mut matches = Vec::new();
        for entry in entries {
            let resolved = content.resolve_ability(entry)?;
            if ability_matches(&resolved, query) {
                matches.push(resolved);
            }
        }
        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => bail!(
                "creature `{}` has no ability matching `{query}`",
                self.creature.display_name()
            ),
            _ => bail!(
                "ability `{query}` is ambiguous on `{}`; matches: {}",
                self.creature.display_name(),
                matches
                    .iter()
                    .map(|resolved| resolved.ability.id.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// All owned abilities, resolved with their entry params.
    pub fn owned_abilities(&self, content: &ContentRegistry) -> Result<Vec<ResolvedAbility>> {
        let mut entries = Vec::new();
        if let Some(actions) = &self.creature.actions {
            entries.extend(actions.iter());
        }
        if let Some(features) = &self.creature.features {
            entries.extend(features.iter());
        }
        entries
            .into_iter()
            .map(|entry| content.resolve_ability(entry))
            .collect()
    }

    pub fn find_attack(&self, query: &str) -> Result<&crate::content::schema::Attack> {
        let needle = query.trim().to_ascii_lowercase();
        let attacks = self.creature.attacks.as_deref().unwrap_or(&[]);
        if let Some(attack) = attacks.iter().find(|attack| {
            attack.id.eq_ignore_ascii_case(&needle)
                || attack.display_name().eq_ignore_ascii_case(&needle)
        }) {
            return Ok(attack);
        }
        let matches: Vec<&crate::content::schema::Attack> = attacks
            .iter()
            .filter(|attack| {
                attack.id.to_ascii_lowercase().starts_with(&needle)
                    || attack
                        .display_name()
                        .to_ascii_lowercase()
                        .starts_with(&needle)
            })
            .collect();
        match matches.as_slice() {
            [attack] => Ok(attack),
            [] => bail!("creature `{}` has no attack matching `{query}`", self.name),
            _ => bail!(
                "attack `{query}` is ambiguous on `{}`; matches: {}",
                self.name,
                matches
                    .iter()
                    .map(|attack| attack.id.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

fn name_matches(lower_name: &str, needle: &str, prefix: bool) -> bool {
    if needle.is_ascii() {
        if prefix {
            lower_name
                .as_bytes()
                .get(..needle.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(needle.as_bytes()))
        } else {
            lower_name.len() == needle.len() && lower_name.eq_ignore_ascii_case(needle)
        }
    } else {
        let needle = needle.to_lowercase();
        if prefix {
            lower_name.starts_with(&needle)
        } else {
            lower_name == needle
        }
    }
}

fn ability_matches(resolved: &ResolvedAbility, query: &str) -> bool {
    let needle = query.trim().to_ascii_lowercase();
    let id = &resolved.ability.id;
    let name = resolved.ability.display_name();
    id.eq_ignore_ascii_case(&needle)
        || name.eq_ignore_ascii_case(&needle)
        || id.to_ascii_lowercase().starts_with(&needle)
        || name.to_ascii_lowercase().starts_with(&needle)
}

pub fn ability_from_text(text: &str) -> Option<AbilityName> {
    match text.trim().to_ascii_lowercase().as_str() {
        "str" | "strength" => Some(AbilityName::Str),
        "dex" | "dexterity" => Some(AbilityName::Dex),
        "con" | "constitution" => Some(AbilityName::Con),
        "int" | "intelligence" => Some(AbilityName::Int),
        "wis" | "wisdom" => Some(AbilityName::Wis),
        "cha" | "charisma" => Some(AbilityName::Cha),
        _ => None,
    }
}

/// A named resource pool.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ResourceState {
    pub current: i32,
    pub max: i32,
    pub recharge: crate::content::schema::Recharge,
    #[serde(default)]
    pub short_rest_regain: Option<i32>,
}

impl ResourceState {
    pub fn from_def(def: &ResourceDef) -> Self {
        Self {
            current: def.max,
            max: def.max,
            recharge: def.recharge.clone(),
            short_rest_regain: def.short_rest_regain,
        }
    }

    pub fn is_full(&self) -> bool {
        self.current >= self.max
    }

    pub fn restore(&mut self, amount: i32) {
        self.current = (self.current + amount.max(0)).min(self.max);
    }
}

/// Why a condition is currently active.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ActiveCondition {
    pub condition: String,
    #[serde(default)]
    pub source: Option<String>,
    /// Creature this instance is linked to (for `linked_*` predicates).
    #[serde(default)]
    pub linked: Option<String>,
    #[serde(default)]
    pub duration: Option<ActiveDuration>,
    #[serde(default)]
    pub save_ends: Option<ActiveSaveEnds>,
    #[serde(default)]
    pub ends_on: Vec<EndsOn>,
    /// Ability/attack id that created this instance.
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub concentration: bool,
    #[serde(default)]
    pub applied_round: u32,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Runtime form of a content duration.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActiveDuration {
    /// Ends after `remaining` end-of-turn events of the affected creature.
    Rounds {
        remaining: u32,
    },
    /// Same as rounds, but measured in 10-round minutes.
    Minutes {
        remaining: u32,
    },
    Permanent,
    /// Ends at the named phase of `subject`'s next turn.
    Until {
        until: DurationUntil,
        subject: String,
        end_round: u32,
    },
}

impl ActiveDuration {
    pub fn describe(&self) -> String {
        match self {
            Self::Rounds { remaining } => format!("{remaining}r"),
            Self::Minutes { remaining } => format!("{}m", remaining.div_ceil(10)),
            Self::Permanent => "permanent".to_string(),
            Self::Until { until, subject, .. } => match until {
                DurationUntil::StartOfSourceNextTurn => {
                    format!("until {subject} starts next turn")
                }
                DurationUntil::EndOfSourceNextTurn => {
                    format!("until {subject} ends next turn")
                }
                DurationUntil::EndOfTargetNextTurn => {
                    format!("until {subject} ends next turn")
                }
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ActiveSaveEnds {
    pub ability: AbilityName,
    pub dc: i32,
    pub at: SaveEndsAt,
}

/// Death-saving-throw progress.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeathSaves {
    #[serde(default)]
    pub successes: u8,
    #[serde(default)]
    pub failures: u8,
    #[serde(default)]
    pub stable: bool,
    #[serde(default)]
    pub dead: bool,
}

/// Description of an `--add` member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddSpec {
    pub creature: String,
    pub count: u32,
    pub zone: Option<String>,
}

impl AddSpec {
    /// Parses `id`, `id:count`, `id@zone` or `id:count@zone`.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.is_empty() {
            bail!("empty --add value");
        }
        let (head, zone) = match text.rsplit_once('@') {
            Some((head, zone)) => {
                if head.is_empty() {
                    bail!("`{text}` has no creature id before `@`");
                }
                if zone.trim().is_empty() {
                    bail!("`{text}` has an empty zone after `@`");
                }
                (head, Some(zone.trim().to_string()))
            }
            None => (text, None),
        };
        let (creature, count) = match head.rsplit_once(':') {
            Some((creature, suffix))
                if !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                let count: u32 = suffix
                    .parse()
                    .with_context(|| format!("invalid count `{suffix}` in `{text}`"))?;
                if count == 0 {
                    bail!("count must be at least 1 in `{text}`");
                }
                (creature, count)
            }
            _ => (head, 1),
        };
        if creature.is_empty() {
            bail!("`{text}` has no creature id");
        }
        Ok(Self {
            creature: creature.to_string(),
            count,
            zone,
        })
    }
}

/// Failure to resolve a combatant name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    NotFound {
        query: String,
    },
    Ambiguous {
        query: String,
        candidates: Vec<String>,
    },
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { query } => write!(f, "no combatant matches `{query}`"),
            Self::Ambiguous { query, candidates } => write!(
                f,
                "combatant name `{query}` is ambiguous; matches: {}",
                candidates.join(", ")
            ),
        }
    }
}

impl std::error::Error for LookupError {}

/// Options for `new --encounter`.
#[derive(Debug, Clone, Default)]
pub struct NewEncounter {
    pub encounter: String,
    pub party: Vec<String>,
    pub party_zone: Option<String>,
    pub adds: Vec<AddSpec>,
}

impl EncounterState {
    /// Builds a state from an encounter id in `content`.
    pub fn from_encounter(content: ContentRegistry, options: &NewEncounter) -> Result<Self> {
        Self::from_encounter_shared(Arc::new(content), options)
    }

    /// Builds a state from an encounter using content shared with a simulation
    /// template. Clones of the state then only clone the `Arc`.
    pub fn from_encounter_shared(
        content: Arc<ContentRegistry>,
        options: &NewEncounter,
    ) -> Result<Self> {
        let encounter = content
            .encounters
            .get(&options.encounter)
            .cloned()
            .ok_or_else(|| anyhow!("unknown encounter `{}`", options.encounter))?;
        let scene = content
            .scenes
            .get(&encounter.scene)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "encounter `{}` references unknown scene `{}`",
                    encounter.id,
                    encounter.scene
                )
            })?;
        let grid = if scene.mode == SceneMode::Grid {
            Some(
                GridMap::parse(&scene)
                    .ok_or_else(|| anyhow!("scene `{}` has no usable grid", scene.id))?,
            )
        } else {
            None
        };
        let mut placement = Placement {
            mode: scene.mode,
            grid,
            occupied: BTreeSet::new(),
        };
        let party_zone = options
            .party_zone
            .clone()
            .or_else(|| encounter.party_zone.clone())
            .or_else(|| scene.zones.first().map(|zone| zone.id.clone()))
            .or_else(|| {
                placement
                    .grid
                    .as_ref()
                    .and_then(|grid| grid.zone_ids().into_iter().next())
            })
            .unwrap_or_default();
        if !placement.known_zone(&scene, &party_zone) {
            bail!(
                "unknown party zone `{party_zone}` (known zones: {})",
                zone_list(&scene, placement.grid.as_ref())
            );
        }

        let mut placement_log = Vec::new();
        let mut combatants = Vec::new();
        for member in &encounter.members {
            if member.count() == 0 {
                bail!("encounter member `{}` has count 0", member.creature);
            }
            let creature = content
                .resolve_creature(&member.creature)
                .with_context(|| format!("encounter member `{}`", member.creature))?;
            let request = member_request(scene.mode, member.pos, member.zone.clone(), &party_zone);
            if !placement.request_known(&scene, &request) {
                bail!(
                    "encounter member `{}` uses unknown zone `{}` (known zones: {})",
                    creature.display_name(),
                    request.describe(),
                    zone_list(&scene, placement.grid.as_ref())
                );
            }
            let side = member.side.clone().unwrap_or_else(|| creature.side());
            let dormant = member
                .dormant
                .unwrap_or_else(|| creature.dormant.unwrap_or(false));
            let ai = member.ai.clone().or_else(|| creature.ai.clone());
            let base_name = member
                .name
                .clone()
                .unwrap_or_else(|| creature.display_name().to_string());
            append_instances(
                &mut combatants,
                &mut placement,
                &scene,
                creature,
                &base_name,
                member.count(),
                &request,
                side,
                dormant,
                ai,
                &mut placement_log,
                "encounter member",
            )?;
        }

        let mut party_creatures = Vec::new();
        for id in &options.party {
            let creature = content
                .resolve_creature(id)
                .with_context(|| format!("party member `{id}`"))?;
            party_creatures.push(creature);
        }
        let party_request = PlaceRequest::Zone(party_zone.clone());
        for creature in party_creatures {
            let base_name = creature.display_name().to_string();
            let ai = creature.ai.clone();
            append_instances(
                &mut combatants,
                &mut placement,
                &scene,
                creature,
                &base_name,
                1,
                &party_request,
                Side::new("party"),
                false,
                ai,
                &mut placement_log,
                "party member",
            )?;
        }

        for add in &options.adds {
            let creature = content
                .resolve_creature(&add.creature)
                .with_context(|| format!("--add `{}`", add.creature))?;
            let zone = add.zone.clone().unwrap_or_else(|| party_zone.clone());
            let request = PlaceRequest::Zone(zone);
            if !placement.request_known(&scene, &request) {
                bail!(
                    "--add `{}` uses unknown zone `{}` (known zones: {})",
                    add.creature,
                    request.describe(),
                    zone_list(&scene, placement.grid.as_ref())
                );
            }
            let base_name = creature.display_name().to_string();
            let side = creature.side();
            let dormant = creature.dormant.unwrap_or(false);
            let ai = creature.ai.clone();
            append_instances(
                &mut combatants,
                &mut placement,
                &scene,
                creature,
                &base_name,
                add.count,
                &request,
                side,
                dormant,
                ai,
                &mut placement_log,
                "--add",
            )?;
        }

        if combatants.is_empty() {
            bail!("encounter `{}` has no combatants", encounter.id);
        }
        uniquify_names(&mut combatants);

        let space_cache = Arc::new(SpaceCache::build(Some(&scene)));
        let name_index = Arc::new(NameIndex::build(&combatants));
        let mut state = Self {
            content,
            scene: Some(scene.clone()),
            encounter_name: Some(
                encounter
                    .name
                    .clone()
                    .unwrap_or_else(|| encounter.id.clone()),
            ),
            party_zone: Some(party_zone),
            combatants,
            initiative: Vec::new(),
            turn: 0,
            round: 0,
            log: placement_log,
            concentration: BTreeMap::new(),
            triggers: scene_trigger_runtime(&scene),
            outcome: None,
            space_cache,
            name_index,
        };
        state.refresh_initial_conditions()?;
        state.log.push(format!(
            "Created {} with {} combatants",
            state.encounter_name.as_deref().unwrap_or("encounter"),
            state.combatants.len()
        ));
        Ok(state)
    }

    /// Applies each creature's initial condition ids (after content is known).
    fn refresh_initial_conditions(&mut self) -> Result<()> {
        for combatant in &mut self.combatants {
            let ids = combatant.creature.conditions.clone().unwrap_or_default();
            for id in ids {
                if combatant.is_immune_to_condition(&id) {
                    continue;
                }
                let duration = self
                    .content
                    .conditions
                    .get(&id)
                    .and_then(|condition| condition.duration_default.clone());
                combatant.conditions.push(ActiveCondition {
                    condition: id,
                    source: None,
                    linked: None,
                    duration: duration
                        .map(|duration| active_duration(&duration, &combatant.name, 0)),
                    save_ends: None,
                    ends_on: Vec::new(),
                    created_by: None,
                    concentration: false,
                    applied_round: 0,
                    notes: None,
                });
            }
        }
        Ok(())
    }

    pub fn space(&self) -> Option<SpaceView<'_>> {
        let scene = self.scene.as_ref()?;
        if let Some(view) = self.space_cache.view(scene) {
            return Some(view);
        }
        // A state deserialized outside `load` has no cache; build a temporary
        // one so correctness never depends on the cache being present.
        match scene.mode {
            SceneMode::Zones => Some(SpaceView::Zones(ZoneGraph::new(scene))),
            SceneMode::Grid => GridSpace::new(scene).map(SpaceView::Grid),
        }
    }

    /// Rebuilds the derived spatial and name indices from the current state.
    pub fn rebuild_caches(&mut self) {
        self.space_cache = Arc::new(SpaceCache::build(self.scene.as_ref()));
        self.name_index = Arc::new(NameIndex::build(&self.combatants));
    }

    /// Refreshes only the name index (combatants added or renamed).
    pub fn refresh_name_index(&mut self) {
        self.name_index = Arc::new(NameIndex::build(&self.combatants));
    }

    pub fn rules(&self) -> &Rules {
        &self.content.rules
    }

    pub fn condition_def(&self, id: &str) -> Option<&ConditionDef> {
        self.content.conditions.get(id)
    }

    pub fn find_index(&self, query: &str) -> Result<usize, LookupError> {
        let needle = query.trim();
        if needle.is_empty() {
            return Err(LookupError::NotFound {
                query: query.to_string(),
            });
        }
        let indexed = self.name_index.names.len() == self.combatants.len();
        let matches = |prefix: bool| -> Vec<usize> {
            if indexed {
                (0..self.combatants.len())
                    .filter(|index| name_matches(&self.name_index.names[*index], needle, prefix))
                    .collect()
            } else {
                self.combatants
                    .iter()
                    .enumerate()
                    .filter(|(_, combatant)| {
                        name_matches(&combatant.name.to_lowercase(), needle, prefix)
                    })
                    .map(|(index, _)| index)
                    .collect()
            }
        };
        let exact = matches(false);
        match exact.as_slice() {
            [index] => return Ok(*index),
            [] => {}
            _ => return Err(self.ambiguous(query, &exact)),
        }

        let prefix = matches(true);
        match prefix.as_slice() {
            [index] => Ok(*index),
            [] => Err(LookupError::NotFound {
                query: query.to_string(),
            }),
            _ => Err(self.ambiguous(query, &prefix)),
        }
    }

    pub fn get(&self, query: &str) -> Result<&CombatantState, LookupError> {
        let index = self.find_index(query)?;
        Ok(&self.combatants[index])
    }

    pub fn get_mut(&mut self, query: &str) -> Result<&mut CombatantState, LookupError> {
        let index = self.find_index(query)?;
        Ok(&mut self.combatants[index])
    }

    /// All instances matching an instance name, creature id, or tag. Used by
    /// `activate`.
    pub fn find_activatable(&self, query: &str) -> Vec<usize> {
        let needle = query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.combatants
            .iter()
            .enumerate()
            .filter(|(_, combatant)| {
                combatant.name.eq_ignore_ascii_case(&needle)
                    || combatant.name.to_ascii_lowercase().starts_with(&needle)
                    || combatant.creature.id.eq_ignore_ascii_case(&needle)
                    || combatant
                        .creature
                        .id
                        .to_ascii_lowercase()
                        .starts_with(&needle)
                    || combatant.tags().iter().any(|tag| {
                        tag.eq_ignore_ascii_case(&needle)
                            || tag.to_ascii_lowercase().starts_with(&needle)
                    })
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn ambiguous(&self, query: &str, indices: &[usize]) -> LookupError {
        let mut candidates: Vec<String> = indices
            .iter()
            .map(|&index| self.combatants[index].name.clone())
            .collect();
        candidates.dedup();
        LookupError::Ambiguous {
            query: query.to_string(),
            candidates,
        }
    }

    pub fn current_index(&self) -> Option<usize> {
        self.initiative.get(self.turn).copied()
    }

    pub fn current(&self) -> Option<&CombatantState> {
        self.current_index()
            .and_then(|index| self.combatants.get(index))
    }

    pub fn current_mut(&mut self) -> Option<&mut CombatantState> {
        let index = self.current_index()?;
        self.combatants.get_mut(index)
    }

    /// Resolves the scene trigger key used in [`Self::triggers`].
    pub fn trigger_key(index: usize, trigger: &SceneTrigger) -> String {
        trigger
            .id
            .clone()
            .unwrap_or_else(|| format!("trigger-{index}"))
    }

    pub fn trigger_enabled(&self, key: &str) -> bool {
        self.triggers
            .get(key)
            .map(|state| state.enabled)
            .unwrap_or(true)
    }

    pub fn set_trigger_enabled(&mut self, key: &str, enabled: bool) {
        if let Some(scene) = &mut self.scene {
            for (index, trigger) in scene.triggers.iter_mut().enumerate() {
                if Self::trigger_key(index, trigger) == key {
                    trigger.enabled = enabled;
                }
            }
        }
        let entry = self
            .triggers
            .entry(key.to_string())
            .or_insert(SceneTriggerRuntime {
                fired: false,
                enabled: true,
            });
        entry.enabled = enabled;
    }

    pub fn set_trigger_fired(&mut self, key: &str) {
        let entry = self
            .triggers
            .entry(key.to_string())
            .or_insert(SceneTriggerRuntime {
                fired: false,
                enabled: true,
            });
        entry.fired = true;
    }

    /// Paths in state files are not implicitly relative to the campaign.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read state file {}", path.display()))?;
        let mut state: Self = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse state file {}", path.display()))?;
        state.rebuild_caches();
        Ok(state)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create state directory {}", parent.display())
                })?;
            }
        }
        let json =
            serde_json::to_string_pretty(self).context("failed to serialize encounter state")?;
        fs::write(path, json)
            .with_context(|| format!("failed to write state file {}", path.display()))
    }
}

/// Turns a content [`Duration`] into runtime form.
pub fn active_duration(duration: &Duration, subject: &str, round: u32) -> ActiveDuration {
    if let Some(rounds) = duration.rounds {
        ActiveDuration::Rounds { remaining: rounds }
    } else if let Some(minutes) = duration.minutes {
        ActiveDuration::Minutes {
            remaining: minutes.saturating_mul(10),
        }
    } else if let Some(until) = duration.until {
        ActiveDuration::Until {
            until,
            subject: subject.to_string(),
            end_round: next_named_turn_round_for_duration(round, until),
        }
    } else {
        ActiveDuration::Permanent
    }
}

fn next_named_turn_round_for_duration(round: u32, _until: DurationUntil) -> u32 {
    round.saturating_add(1)
}

/// Where an encounter member should be placed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PlaceRequest {
    Zone(String),
    Cell(i32, i32),
}

impl PlaceRequest {
    fn describe(&self) -> String {
        match self {
            Self::Zone(zone) => zone.clone(),
            Self::Cell(x, y) => format!("({x},{y})"),
        }
    }
}

fn member_request(
    mode: SceneMode,
    pos: Option<[i32; 2]>,
    zone: Option<String>,
    party_zone: &str,
) -> PlaceRequest {
    match (mode, pos) {
        (SceneMode::Grid, Some([x, y])) => PlaceRequest::Cell(x, y),
        _ => PlaceRequest::Zone(zone.unwrap_or_else(|| party_zone.to_string())),
    }
}

struct Placement {
    mode: SceneMode,
    grid: Option<GridMap>,
    occupied: BTreeSet<(i32, i32)>,
}

impl Placement {
    fn known_zone(&self, scene: &Scene, zone: &str) -> bool {
        scene.zones.iter().any(|known| known.id == zone)
            || self
                .grid
                .as_ref()
                .is_some_and(|grid| grid.zone_ids().iter().any(|known| known == zone))
    }

    fn request_known(&self, scene: &Scene, request: &PlaceRequest) -> bool {
        match request {
            PlaceRequest::Zone(zone) => self.known_zone(scene, zone),
            PlaceRequest::Cell(x, y) => self
                .grid
                .as_ref()
                .is_some_and(|grid| grid.in_bounds(*x, *y) && !grid.is_wall(*x, *y)),
        }
    }

    fn place(
        &mut self,
        scene: &Scene,
        request: &PlaceRequest,
        log: &mut Vec<String>,
        label: &str,
    ) -> Result<(String, Position)> {
        match self.mode {
            SceneMode::Zones => {
                let PlaceRequest::Zone(zone) = request else {
                    bail!(
                        "scene `{}` uses zones, so grid positions are not valid",
                        scene.id
                    );
                };
                Ok((zone.clone(), Position::Zone { zone: zone.clone() }))
            }
            SceneMode::Grid => {
                let grid = self
                    .grid
                    .as_ref()
                    .ok_or_else(|| anyhow!("scene `{}` has no grid", scene.id))?;
                let requested = match request {
                    PlaceRequest::Cell(x, y) => {
                        if grid.is_wall(*x, *y) {
                            bail!("cell ({x},{y}) is a wall or outside the grid");
                        }
                        Some((*x, *y))
                    }
                    PlaceRequest::Zone(_) => None,
                };
                let anchor = match request {
                    PlaceRequest::Zone(zone) => grid.cells_in_zone(zone).first().copied(),
                    PlaceRequest::Cell(x, y) => Some((*x, *y)),
                };
                let cell = match requested {
                    Some(cell) if !self.occupied.contains(&cell) => cell,
                    Some(cell) => {
                        let fallback = self.nearest_free(grid, cell)?;
                        log.push(format!(
                            "{label} requested ({},{}) but the cell is occupied; placed at ({},{})",
                            cell.0, cell.1, fallback.0, fallback.1
                        ));
                        fallback
                    }
                    None => match anchor {
                        Some(anchor) => {
                            let in_zone: Vec<(i32, i32)> = match request {
                                PlaceRequest::Zone(zone) => grid.cells_in_zone(zone),
                                PlaceRequest::Cell(..) => Vec::new(),
                            };
                            match in_zone
                                .into_iter()
                                .find(|cell| !self.occupied.contains(cell))
                            {
                                Some(cell) => cell,
                                None => {
                                    let fallback = self.nearest_free(grid, anchor)?;
                                    log.push(format!(
                                        "{label} zone `{}` has no free cell; placed at ({},{})",
                                        request.describe(),
                                        fallback.0,
                                        fallback.1
                                    ));
                                    fallback
                                }
                            }
                        }
                        None => {
                            let fallback =
                                self.nearest_free(grid, (grid.width / 2, grid.height / 2))?;
                            log.push(format!(
                                "{label} zone `{}` has no cell; placed at ({},{})",
                                request.describe(),
                                fallback.0,
                                fallback.1
                            ));
                            fallback
                        }
                    },
                };
                self.occupied.insert(cell);
                Ok((
                    grid.display_zone(cell.0, cell.1),
                    Position::Cell {
                        x: cell.0,
                        y: cell.1,
                    },
                ))
            }
        }
    }

    fn nearest_free(&self, grid: &GridMap, anchor: (i32, i32)) -> Result<(i32, i32)> {
        let mut free: Vec<(i32, (i32, i32))> = grid
            .open_cells()
            .into_iter()
            .filter(|cell| !self.occupied.contains(cell))
            .map(|cell| {
                (
                    (cell.0 - anchor.0).abs().max((cell.1 - anchor.1).abs()),
                    cell,
                )
            })
            .collect();
        free.sort_unstable();
        free.first()
            .map(|(_, cell)| *cell)
            .ok_or_else(|| anyhow!("the grid has no free cell"))
    }
}

#[allow(clippy::too_many_arguments)]
fn append_instances(
    combatants: &mut Vec<CombatantState>,
    placement: &mut Placement,
    scene: &Scene,
    creature: Creature,
    base_name: &str,
    count: u32,
    request: &PlaceRequest,
    side: Side,
    dormant: bool,
    ai: Option<AiAssignment>,
    log: &mut Vec<String>,
    label: &str,
) -> Result<()> {
    let count = count.max(1);
    let existing: BTreeSet<String> = combatants
        .iter()
        .map(|combatant| combatant.name.to_ascii_lowercase())
        .collect();
    for index in 1..=count {
        let name = if count > 1 {
            format!("{base_name} {index}")
        } else {
            unique_name(base_name, &existing)
        };
        let (zone, pos) = placement.place(scene, request, log, label)?;
        let mut runtime =
            combatant_from_creature(&creature, name, side.clone(), &zone, pos, dormant);
        runtime.ai = ai.clone();
        combatants.push(runtime);
    }
    Ok(())
}

fn uniquify_names(combatants: &mut [CombatantState]) {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, combatant) in combatants.iter().enumerate() {
        groups
            .entry(name_stem(&combatant.name))
            .or_default()
            .push(index);
    }
    for indices in groups.into_values().filter(|indices| indices.len() > 1) {
        let base = name_stem(&combatants[indices[0]].name);
        let display = base
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        for (offset, index) in indices.into_iter().enumerate() {
            combatants[index].name = format!("{display} {}", offset + 1);
        }
    }
}

fn name_stem(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    if let Some((prefix, suffix)) = lower.rsplit_once(' ') {
        if !prefix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return prefix.to_string();
        }
    }
    lower
}

fn unique_name(base: &str, existing: &BTreeSet<String>) -> String {
    if !existing.contains(&base.to_ascii_lowercase()) {
        return base.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base} {index}");
        if !existing.contains(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
        index += 1;
    }
}

fn combatant_from_creature(
    creature: &Creature,
    name: String,
    side: Side,
    zone: &str,
    pos: Position,
    dormant: bool,
) -> CombatantState {
    let max_hp = creature.max_hp().max(1);
    let hp = creature.current_hp().clamp(0, max_hp);
    let resources = creature
        .resources
        .as_ref()
        .map(|resources| {
            resources
                .iter()
                .map(|(id, def)| (id.clone(), ResourceState::from_def(def)))
                .collect()
        })
        .unwrap_or_default();
    CombatantState {
        creature: Arc::new(creature.clone()),
        name,
        side,
        max_hp,
        hp,
        temp_hp: 0,
        zone: zone.to_string(),
        pos: Some(pos),
        conditions: Vec::new(),
        resources,
        death_saves: DeathSaves::default(),
        action_used: false,
        bonus_action_used: false,
        reaction_round: None,
        speed_left: creature.speed().walk.max(0),
        once_used: BTreeMap::new(),
        ai: creature.ai.clone(),
        initiative_total: None,
        dormant,
        opening_used: false,
        dead: false,
    }
}

fn scene_trigger_runtime(scene: &Scene) -> BTreeMap<String, SceneTriggerRuntime> {
    scene
        .triggers
        .iter()
        .enumerate()
        .map(|(index, trigger)| {
            (
                EncounterState::trigger_key(index, trigger),
                SceneTriggerRuntime {
                    fired: false,
                    enabled: trigger.enabled,
                },
            )
        })
        .collect()
}

fn zone_list(scene: &Scene, grid: Option<&GridMap>) -> String {
    let mut zones: Vec<String> = scene.zones.iter().map(|zone| zone.id.clone()).collect();
    if let Some(grid) = grid {
        for zone in grid.zone_ids() {
            if !zones.contains(&zone) {
                zones.push(zone);
            }
        }
    }
    zones.join(", ")
}

/// Condition ids active on a creature, including recursively implied ones.
pub fn active_condition_ids(content: &ContentRegistry, combatant: &CombatantState) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut stack: Vec<String> = combatant
        .conditions
        .iter()
        .map(|condition| condition.condition.clone())
        .collect();
    while let Some(id) = stack.pop() {
        if ids
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&id))
        {
            continue;
        }
        if let Some(def) = content.conditions.get(&id) {
            stack.extend(def.implies.iter().cloned());
        }
        ids.push(id);
    }
    ids
}

/// Resolves a `--state` argument.
///
/// `None` maps to [`DEFAULT_STATE_FILE`]. A value containing a path separator
/// is literal. A bare name is placed in the manifest `state_dir` when one is
/// known, otherwise in the current directory; `.state.json` is appended unless
/// already present.
pub fn resolve_state_path(requested: Option<&Path>, state_dir: Option<&Path>) -> PathBuf {
    let Some(requested) = requested else {
        return PathBuf::from(DEFAULT_STATE_FILE);
    };
    let text = requested.to_string_lossy();
    if text.is_empty() {
        return PathBuf::from(DEFAULT_STATE_FILE);
    }
    if text.contains('/') || text.contains('\\') {
        return requested.to_path_buf();
    }
    let mut file = text.into_owned();
    if !file.ends_with(STATE_SUFFIX) {
        file.push_str(STATE_SUFFIX);
    }
    match state_dir {
        Some(dir) => dir.join(file),
        None => PathBuf::from(format!("./{file}")),
    }
}
