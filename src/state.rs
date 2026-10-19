//! Runtime encounter state and its persisted JSON form.
//!
//! Step 2 mutates this state; step 1 provides construction, name lookup and a
//! lossless JSON round trip.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{self, CombatantSpec, ConditionApplication, DurationSpec, Resource};

/// Zone used when no encounter or data file supplies one.
pub const DEFAULT_ZONE: &str = "main";

/// A persisted encounter.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EncounterState {
    /// Encounter name, when the state was built from an encounter file.
    #[serde(default)]
    pub name: Option<String>,
    /// Abstract zone positions in feet.
    #[serde(default)]
    pub zones: BTreeMap<String, i32>,
    /// Combatants in creation order.
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
    /// Human-readable log lines.
    #[serde(default)]
    pub log: Vec<String>,
}

/// A combatant's full static data plus mutable runtime state.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatantState {
    /// Full data-file contents, embedded so a saved state can be reloaded
    /// without access to the original data files.
    pub spec: CombatantSpec,
    pub hp: i32,
    #[serde(default)]
    pub temp_hp: i32,
    pub zone: String,
    #[serde(default)]
    pub conditions: Vec<ActiveCondition>,
    /// Runtime copy of the spec's resource pools; `current` changes as they are spent.
    #[serde(default)]
    pub resources: BTreeMap<String, Resource>,
    #[serde(default)]
    pub death_saves: DeathSaves,
    #[serde(default)]
    pub reaction_used: bool,
    #[serde(default)]
    pub sneak_used_this_turn: bool,
    #[serde(default)]
    pub action_used: bool,
    #[serde(default)]
    pub bonus_action_used: bool,
    #[serde(default)]
    pub speed_left: i32,
    /// Weapon-mastery and similar markers (Sap, Vex, Slow) with their expiry.
    #[serde(default)]
    pub markers: Vec<EffectMarker>,
}

impl CombatantState {
    /// Builds runtime state from static data.
    pub fn from_spec(spec: CombatantSpec, zone: impl Into<String>) -> Self {
        let hp = spec.hp;
        let temp_hp = spec.temp_hp;
        let speed_left = spec.speed;
        let conditions = spec
            .conditions
            .iter()
            .cloned()
            .map(ActiveCondition::from)
            .collect();
        let resources = spec.resources.clone();
        Self {
            spec,
            hp,
            temp_hp,
            zone: zone.into(),
            conditions,
            resources,
            death_saves: DeathSaves::default(),
            reaction_used: false,
            sneak_used_this_turn: false,
            action_used: false,
            bonus_action_used: false,
            speed_left,
            markers: Vec::new(),
        }
    }

    /// The combatant's display name, including any duplicate suffix.
    pub fn name(&self) -> &str {
        &self.spec.name
    }

    /// Which side the combatant is on.
    pub fn side(&self) -> model::Side {
        self.spec.side
    }

    /// Whether the combatant is a PC or monster.
    pub fn kind(&self) -> model::Kind {
        self.spec.kind
    }

    /// Base armour class from the data file.
    pub fn ac(&self) -> i32 {
        self.spec.ac
    }

    /// Ability modifier using the 5e floor formula.
    pub fn ability_modifier(&self, ability: &str) -> i32 {
        let score = self
            .spec
            .abilities
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(ability))
            .map(|(_, score)| *score)
            .unwrap_or(10);
        (score - 10).div_euclid(2)
    }

    /// Saving-throw bonus: an explicit save value when present, otherwise the
    /// plain ability modifier.
    pub fn save_bonus(&self, ability: &str) -> i32 {
        self.spec
            .saves
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(ability))
            .map(|(_, bonus)| *bonus)
            .unwrap_or_else(|| self.ability_modifier(ability))
    }

    /// Skill bonus: an explicit skill value when present, otherwise the plain
    /// ability modifier for an ability name.
    pub fn skill_bonus(&self, skill: &str) -> i32 {
        self.spec
            .skills
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(skill))
            .map(|(_, bonus)| *bonus)
            .unwrap_or_else(|| self.ability_modifier(skill))
    }

    /// Checks active conditions by name, case-insensitively.
    pub fn has_condition(&self, condition: &str) -> bool {
        self.conditions
            .iter()
            .any(|active| active.condition.eq_ignore_ascii_case(condition))
    }

    /// Checks condition immunities by name, case-insensitively.
    pub fn is_immune_to_condition(&self, condition: &str) -> bool {
        self.spec
            .condition_immunities
            .iter()
            .any(|immune| immune.eq_ignore_ascii_case(condition))
    }

    /// Whether the combatant has a marker of the given kind.
    pub fn has_marker(&self, kind: MarkerKind) -> bool {
        self.markers.iter().any(|marker| marker.kind == kind)
    }

    /// Current speed after Slow markers and the grappled condition.
    pub fn effective_speed(&self) -> i32 {
        if self.has_condition("grappled") {
            return 0;
        }
        let speed = self.spec.speed;
        if self.has_marker(MarkerKind::Slow) {
            (speed - 10).max(0)
        } else {
            speed.max(0)
        }
    }

    /// Whether the combatant can act: positive HP, not dead, not unconscious.
    pub fn is_conscious(&self) -> bool {
        self.hp > 0 && !self.death_saves.dead && !self.has_condition("unconscious")
    }
}

/// Kind of transient marker applied by weapon mastery or a similar effect.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    /// Disadvantage on the marked creature's next attack roll.
    Sap,
    /// Advantage for the source on its next attack against `target`.
    Vex,
    /// Speed reduced by 10 ft while present.
    Slow,
}

/// A Sap, Vex or Slow marker with its source and expiry.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EffectMarker {
    pub kind: MarkerKind,
    /// Creature that applied the marker.
    pub source: String,
    /// Vex only: the creature the advantage applies against.
    #[serde(default)]
    pub target: Option<String>,
    /// Round at whose end the marker expires.
    pub expires_round: u32,
    /// Creature whose turn end removes the marker.
    pub ends_with_turn_of: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A condition currently affecting a combatant.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ActiveCondition {
    pub condition: String,
    /// Creature or effect that applied the condition.
    #[serde(default)]
    pub source: Option<String>,
    /// DC to escape, for grappled/restrained-style conditions.
    #[serde(default)]
    pub escape_dc: Option<i32>,
    #[serde(default)]
    pub duration: Option<DurationSpec>,
    /// Absolute round at whose end the condition expires, when not driven by
    /// `duration.rounds`.
    #[serde(default)]
    pub expires_round: Option<u32>,
    /// Creature whose turn end removes the condition (for
    /// `end_of_source_next_turn` and similar effects).
    #[serde(default)]
    pub ends_with_turn_of: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl From<ConditionApplication> for ActiveCondition {
    fn from(application: ConditionApplication) -> Self {
        Self {
            condition: application.condition,
            source: None,
            escape_dc: application.escape_dc,
            duration: application.duration,
            expires_round: None,
            ends_with_turn_of: None,
            notes: application.notes,
        }
    }
}

/// Death-saving-throw progress for a player character.
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

impl EncounterState {
    /// Builds state from command-line combatant specs.
    ///
    /// Each spec is `file[:count][@zone]`. When no zones exist a `main` zone at
    /// position 0 is created; any other explicitly requested zone is inserted
    /// at position 0 as well.
    pub fn from_specs(specs: &[String]) -> Result<Self> {
        if specs.is_empty() {
            bail!("at least one combatant spec is required");
        }

        let mut zones = BTreeMap::new();
        zones.insert(DEFAULT_ZONE.to_string(), 0);
        let mut combatants = Vec::new();

        for token in specs {
            let parsed = parse_spec_token(token)?;
            let spec = model::load_combatant(&parsed.path)
                .with_context(|| format!("failed to load combatant `{}`", parsed.path.display()))?;
            let zone = parsed
                .zone
                .or_else(|| spec.zone.clone())
                .unwrap_or_else(|| DEFAULT_ZONE.to_string());
            zones.entry(zone.clone()).or_insert(0);
            push_instances(&mut combatants, spec, parsed.count, &zone);
        }

        Ok(Self {
            name: None,
            zones,
            combatants,
            initiative: Vec::new(),
            turn: 0,
            round: 0,
            log: Vec::new(),
        })
    }

    /// Builds state from an encounter file, resolving members relative to it.
    pub fn from_encounter_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let encounter = model::load_encounter(path)?;
        let base = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        Self::from_encounter_value(encounter, base)
    }

    fn from_encounter_value(encounter: model::Encounter, base: &Path) -> Result<Self> {
        let mut zones = encounter.zones;
        if zones.is_empty() {
            zones.insert(DEFAULT_ZONE.to_string(), 0);
        }

        let mut combatants = Vec::new();
        for member in &encounter.members {
            if member.count == 0 {
                bail!("encounter member `{}` has count 0", member.file);
            }
            let spec = model::load_combatant(base.join(&member.file))
                .with_context(|| format!("failed to load encounter member `{}`", member.file))?;
            let zone = resolve_member_zone(member.zone.as_deref(), &spec, &zones)?;
            push_instances(&mut combatants, spec, member.count, &zone);
        }

        Ok(Self {
            name: Some(encounter.name),
            zones,
            combatants,
            initiative: Vec::new(),
            turn: 0,
            round: 0,
            log: Vec::new(),
        })
    }

    /// Resolves a combatant name to its index.
    ///
    /// A case-insensitive exact match wins. Otherwise a case-insensitive unique
    /// prefix is accepted.
    pub fn find_index(&self, query: &str) -> Result<usize, LookupError> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Err(LookupError::NotFound {
                query: query.to_string(),
            });
        }

        let exact: Vec<usize> = self
            .combatants
            .iter()
            .enumerate()
            .filter(|(_, combatant)| combatant.name().to_lowercase() == needle)
            .map(|(index, _)| index)
            .collect();
        match exact.as_slice() {
            [index] => return Ok(*index),
            [] => {}
            _ => return Err(self.ambiguous(query, &exact)),
        }

        let prefix: Vec<usize> = self
            .combatants
            .iter()
            .enumerate()
            .filter(|(_, combatant)| combatant.name().to_lowercase().starts_with(&needle))
            .map(|(index, _)| index)
            .collect();
        match prefix.as_slice() {
            [index] => Ok(*index),
            [] => Err(LookupError::NotFound {
                query: query.to_string(),
            }),
            _ => Err(self.ambiguous(query, &prefix)),
        }
    }

    /// Resolves a combatant name to a reference.
    pub fn get(&self, query: &str) -> Result<&CombatantState, LookupError> {
        let index = self.find_index(query)?;
        Ok(&self.combatants[index])
    }

    /// Resolves a combatant name to a mutable reference.
    pub fn get_mut(&mut self, query: &str) -> Result<&mut CombatantState, LookupError> {
        let index = self.find_index(query)?;
        Ok(&mut self.combatants[index])
    }

    /// Index into `combatants` for the creature whose turn it currently is.
    pub fn current_index(&self) -> Option<usize> {
        self.initiative.get(self.turn).copied()
    }

    /// The creature whose turn it currently is.
    pub fn current(&self) -> Option<&CombatantState> {
        self.current_index()
            .and_then(|index| self.combatants.get(index))
    }

    /// Mutable access to the creature whose turn it currently is.
    pub fn current_mut(&mut self) -> Option<&mut CombatantState> {
        let index = self.current_index()?;
        self.combatants.get_mut(index)
    }

    fn ambiguous(&self, query: &str, indices: &[usize]) -> LookupError {
        let mut candidates: Vec<String> = indices
            .iter()
            .map(|&index| self.combatants[index].name().to_string())
            .collect();
        candidates.dedup();
        LookupError::Ambiguous {
            query: query.to_string(),
            candidates,
        }
    }

    /// Loads state from pretty JSON.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read state file {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("failed to parse state file {}", path.display()))
    }

    /// Saves state as pretty JSON, creating parent directories if needed.
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

struct SpecToken {
    path: PathBuf,
    count: u32,
    zone: Option<String>,
}

fn parse_spec_token(token: &str) -> Result<SpecToken> {
    let token = token.trim();
    if token.is_empty() {
        bail!("empty combatant spec");
    }

    let (path_part, zone) = match token.rsplit_once('@') {
        Some((path, zone)) => {
            if path.is_empty() {
                bail!("combatant spec `{token}` has no file before `@`");
            }
            if zone.is_empty() {
                bail!("combatant spec `{token}` has an empty zone after `@`");
            }
            (path, Some(zone.to_string()))
        }
        None => (token, None),
    };

    let (path, count) = match path_part.rsplit_once(':') {
        Some((path, suffix))
            if !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            let count = suffix
                .parse::<u32>()
                .with_context(|| format!("invalid count `{suffix}` in combatant spec `{token}`"))?;
            if count == 0 {
                bail!("combatant count must be at least 1 in `{token}`");
            }
            (path, count)
        }
        _ => (path_part, 1),
    };

    if path.is_empty() {
        bail!("combatant spec `{token}` has no file path");
    }

    Ok(SpecToken {
        path: PathBuf::from(path),
        count,
        zone,
    })
}

fn push_instances(
    combatants: &mut Vec<CombatantState>,
    spec: CombatantSpec,
    count: u32,
    zone: &str,
) {
    debug_assert!(count > 0, "instance count must be positive");
    let base_name = spec.name.clone();
    for index in 1..=count {
        let mut instance = spec.clone();
        if count > 1 {
            instance.name = format!("{base_name} {index}");
        }
        combatants.push(CombatantState::from_spec(instance, zone));
    }
}

fn resolve_member_zone(
    requested: Option<&str>,
    spec: &CombatantSpec,
    zones: &BTreeMap<String, i32>,
) -> Result<String> {
    match requested {
        Some(zone) if zones.contains_key(zone) => Ok(zone.to_string()),
        Some(zone) => bail!(
            "encounter member `{}` uses undefined zone `{zone}` (known zones: {})",
            spec.name,
            known_zones(zones)
        ),
        None => {
            if let Some(zone) = spec
                .zone
                .as_deref()
                .filter(|zone| zones.contains_key(*zone))
            {
                return Ok(zone.to_string());
            }
            if zones.contains_key(DEFAULT_ZONE) {
                return Ok(DEFAULT_ZONE.to_string());
            }
            bail!(
                "encounter member `{}` has no zone and the encounter has no `main` fallback zone (known zones: {})",
                spec.name,
                known_zones(zones)
            )
        }
    }
}

fn known_zones(zones: &BTreeMap<String, i32>) -> String {
    if zones.is_empty() {
        "(none)".to_string()
    } else {
        zones.keys().cloned().collect::<Vec<_>>().join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Encounter, EncounterMember};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn repo_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "combat-sim-state-{label}-{}-{nanos}-{count}",
            std::process::id()
        ))
    }

    #[test]
    fn builds_from_encounter_file() -> Result<()> {
        let state = EncounterState::from_encounter_file(repo_path(
            "data/encounters/party-vs-2-guards.json",
        ))?;

        assert_eq!(state.name.as_deref(), Some("party vs 2 guards"));
        assert_eq!(state.zones.get("hall"), Some(&40));
        assert_eq!(state.combatants.len(), 6);
        assert_eq!(state.round, 0);
        assert_eq!(state.turn, 0);
        assert!(state.initiative.is_empty());

        let guard_one = state.get("guard 1")?;
        let guard_two = state.get("GUARD 2")?;
        assert_eq!(guard_one.name(), "Guard 1");
        assert_eq!(guard_two.name(), "Guard 2");
        assert_eq!(guard_one.spec.ac, 16);
        assert_eq!(guard_one.hp, 11);
        assert_eq!(guard_one.zone, "hall");
        assert_eq!(guard_one.speed_left, 30);

        let will = state.get("Will")?;
        assert_eq!(will.zone, "front");
        assert_eq!(
            will.resources
                .get("second_wind")
                .map(|resource| resource.current),
            Some(2)
        );

        Ok(())
    }

    #[test]
    fn builds_from_specs_and_renames_duplicates() -> Result<()> {
        let guard_path = repo_path("data/monsters/guard.json");
        let duplicated = format!("{}:2@hall", guard_path.display());
        let state = EncounterState::from_specs(&[duplicated])?;

        assert_eq!(state.zones.get("main"), Some(&0));
        assert_eq!(state.zones.get("hall"), Some(&0));
        assert_eq!(state.zones.len(), 2);
        assert_eq!(state.combatants.len(), 2);
        assert_eq!(state.combatants[0].name(), "Guard 1");
        assert_eq!(state.combatants[1].name(), "Guard 2");
        assert!(state
            .combatants
            .iter()
            .all(|combatant| combatant.zone == "hall"));

        let single = EncounterState::from_specs(&[guard_path.display().to_string()])?;
        assert_eq!(single.combatants.len(), 1);
        assert_eq!(single.combatants[0].name(), "Guard");
        assert_eq!(single.combatants[0].zone, "main");

        Ok(())
    }

    #[test]
    fn lookup_is_case_insensitive_and_accepts_unique_prefixes() -> Result<()> {
        let state = EncounterState::from_encounter_file(repo_path(
            "data/encounters/party-vs-2-guards.json",
        ))?;

        assert_eq!(state.get("mil")?.name(), "Milo");
        assert_eq!(state.get("GUARD 2")?.name(), "Guard 2");
        assert_eq!(state.get("Guard 1")?.name(), "Guard 1");

        match state.find_index("guard") {
            Err(LookupError::Ambiguous { candidates, .. }) => {
                assert_eq!(candidates, vec!["Guard 1", "Guard 2"]);
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }

        assert!(matches!(
            state.find_index("nobody"),
            Err(LookupError::NotFound { .. })
        ));

        let guard_path = repo_path("data/monsters/guard.json");
        let state = EncounterState::from_specs(&[
            guard_path.display().to_string(),
            format!("{}:2", guard_path.display()),
        ])?;
        assert_eq!(state.get("Guard")?.name(), "Guard");

        Ok(())
    }

    #[test]
    fn initial_spec_conditions_become_active() -> Result<()> {
        let spec = model::parse_combatant_json(
            r#"{
                "name": "Sleeper",
                "side": "party",
                "kind": "pc",
                "hp": 5,
                "max_hp": 5,
                "ac": 10,
                "conditions": [
                    {
                        "condition": "prone",
                        "escape_dc": 13,
                        "duration": {"rounds": 2}
                    }
                ]
            }"#,
        )?;
        let combatant = CombatantState::from_spec(spec, "main");

        assert_eq!(combatant.conditions.len(), 1);
        assert_eq!(combatant.conditions[0].condition, "prone");
        assert_eq!(combatant.conditions[0].escape_dc, Some(13));
        assert_eq!(
            combatant.conditions[0]
                .duration
                .as_ref()
                .and_then(|duration| duration.rounds),
            Some(2)
        );
        assert_eq!(combatant.conditions[0].source, None);
        assert!(combatant.is_conscious());

        Ok(())
    }

    #[test]
    fn state_json_round_trip_is_lossless() -> Result<()> {
        let mut state = EncounterState::from_encounter_file(repo_path(
            "data/encounters/party-vs-2-guards.json",
        ))?;
        state.name = Some("modified party vs 2 guards".to_string());
        state.round = 3;
        state.turn = 2;
        state.initiative = vec![3, 0, 5, 1, 4, 2];
        state.log.push("Round 1".to_string());
        state.log.push("Will hits Guard 1".to_string());

        let will = state.get_mut("Will")?;
        will.hp -= 3;
        will.temp_hp = 4;
        will.conditions.push(ActiveCondition {
            condition: "prone".to_string(),
            source: Some("shove".to_string()),
            escape_dc: None,
            duration: Some(DurationSpec {
                until: Some("end_of_source_next_turn".to_string()),
                rounds: None,
                notes: None,
            }),
            expires_round: Some(2),
            ends_with_turn_of: Some("Guard 1".to_string()),
            notes: Some("test condition".to_string()),
        });
        will.resources
            .get_mut("second_wind")
            .expect("Will has second_wind")
            .current = 1;
        will.death_saves.successes = 2;
        will.death_saves.failures = 1;
        will.death_saves.stable = true;
        will.reaction_used = true;
        will.sneak_used_this_turn = true;
        will.action_used = true;
        will.bonus_action_used = true;
        will.speed_left = 15;
        will.markers.push(EffectMarker {
            kind: MarkerKind::Sap,
            source: "Guard 1".to_string(),
            target: None,
            expires_round: 2,
            ends_with_turn_of: "Guard 1".to_string(),
            notes: Some("test marker".to_string()),
        });

        let temp_root = unique_temp_dir("roundtrip");
        let path = temp_root.join("nested").join("state.json");
        state.save(&path)?;
        let loaded = EncounterState::load(&path)?;
        assert_eq!(loaded, state);

        fs::remove_dir_all(&temp_root)?;
        Ok(())
    }

    #[test]
    fn encounter_member_zone_must_exist() -> Result<()> {
        let encounter = Encounter {
            name: "bad zone".to_string(),
            zones: BTreeMap::from([("front".to_string(), 0)]),
            members: vec![EncounterMember {
                file: repo_path("data/monsters/guard.json").display().to_string(),
                count: 1,
                zone: Some("nowhere".to_string()),
                notes: None,
            }],
            notes: None,
        };

        let error = EncounterState::from_encounter_value(encounter, Path::new("."))
            .expect_err("undefined member zone should be rejected");
        assert!(
            error.to_string().contains("nowhere"),
            "unexpected error: {error}"
        );

        Ok(())
    }
}
