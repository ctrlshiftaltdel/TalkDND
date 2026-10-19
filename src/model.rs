//! Serde data model for combatants and encounters.
//!
//! The files under `data/` are the authoritative shape. Every struct ignores
//! unknown fields and accepts an optional `notes` string, so new data can be
//! added without breaking existing code.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Which side of the encounter a creature belongs to.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Party,
    Enemy,
    /// Forward-compatibility fallback for data written for a newer schema.
    #[serde(other)]
    Unknown,
}

/// Whether a creature is a player character or a monster.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Pc,
    Monster,
    /// Forward-compatibility fallback for data written for a newer schema.
    #[serde(other)]
    Unknown,
}

/// How an attack resolves.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttackKind {
    Melee,
    Ranged,
    SpellAttack,
    Save,
    /// Automatically hits, e.g. Magic Missile.
    Auto,
    #[serde(other)]
    Unknown,
}

/// 2024 weapon mastery properties.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mastery {
    Sap,
    Slow,
    Nick,
    Vex,
    Topple,
    Graze,
    Push,
    Cleave,
    #[serde(other)]
    Unknown,
}

/// Action economy cost of a heal.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HealAction {
    #[default]
    Action,
    Bonus,
    #[serde(other)]
    Unknown,
}

/// When a limited-use resource resets.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResetType {
    Short,
    #[default]
    Long,
    #[serde(other)]
    Unknown,
}

/// A combatant as written in `data/party` or `data/monsters`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatantSpec {
    pub name: String,
    pub side: Side,
    pub kind: Kind,
    pub hp: i32,
    pub max_hp: i32,
    #[serde(default)]
    pub temp_hp: i32,
    pub ac: i32,
    #[serde(default)]
    pub init_bonus: i32,
    #[serde(default = "default_speed")]
    pub speed: i32,
    #[serde(default = "default_abilities")]
    pub abilities: BTreeMap<String, i32>,
    #[serde(default)]
    pub saves: BTreeMap<String, i32>,
    #[serde(default)]
    pub skills: BTreeMap<String, i32>,
    #[serde(default)]
    pub creature_type: String,
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub attacks: Vec<Attack>,
    #[serde(default)]
    pub multiattack: Vec<String>,
    #[serde(default)]
    pub heals: Vec<Heal>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub resources: BTreeMap<String, Resource>,
    #[serde(default)]
    pub features: BTreeMap<String, String>,
    #[serde(default)]
    pub resistances: Vec<String>,
    #[serde(default)]
    pub immunities: Vec<String>,
    #[serde(default)]
    pub vulnerabilities: Vec<String>,
    #[serde(default)]
    pub condition_immunities: Vec<String>,
    #[serde(default)]
    pub conditions: Vec<ConditionApplication>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// One attack option on a combatant.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Attack {
    pub name: String,
    pub kind: AttackKind,
    #[serde(default)]
    pub to_hit: Option<i32>,
    #[serde(default)]
    pub damage: Option<String>,
    #[serde(default)]
    pub damage_type: Option<String>,
    #[serde(default)]
    pub extra_damage: Vec<ExtraDamage>,
    #[serde(default)]
    pub reach: Option<i32>,
    /// Normal and long range in feet, for ranged/thrown attacks.
    #[serde(default)]
    pub range: Option<[i32; 2]>,
    #[serde(default)]
    pub mastery: Option<Mastery>,
    #[serde(default)]
    pub sneak_eligible: bool,
    #[serde(default = "default_count")]
    pub count: u32,
    #[serde(default)]
    pub cost: Option<Cost>,
    #[serde(default)]
    pub save: Option<SaveSpec>,
    #[serde(default)]
    pub on_hit_conditions: Vec<ConditionApplication>,
    #[serde(default)]
    pub on_fail_conditions: Vec<ConditionApplication>,
    #[serde(default)]
    pub vs_condition: Option<VsCondition>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Additional damage rider on an attack.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ExtraDamage {
    pub dice: String,
    #[serde(rename = "type", default)]
    pub damage_type: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A limited-use cost, e.g. one spell slot.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Cost {
    pub resource: String,
    #[serde(default = "default_count")]
    pub amount: u32,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Saving throw required by an attack.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SaveSpec {
    pub ability: String,
    pub dc: i32,
    #[serde(default)]
    pub half_on_success: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Conditional advantage/damage against a target suffering from a condition.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VsCondition {
    pub condition: String,
    #[serde(default)]
    pub advantage: bool,
    #[serde(default)]
    pub damage: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A healing ability.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Heal {
    pub name: String,
    pub amount: String,
    #[serde(default)]
    pub range: i32,
    #[serde(default)]
    pub action: HealAction,
    #[serde(default)]
    pub cost: Option<Cost>,
    #[serde(default)]
    pub self_only: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A reaction such as Shield.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub name: String,
    #[serde(default)]
    pub ac_bonus: i32,
    #[serde(default)]
    pub cost: Option<Cost>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A limited-use resource pool.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    #[serde(default)]
    pub current: i32,
    #[serde(default)]
    pub max: i32,
    #[serde(default)]
    pub reset: ResetType,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A condition applied by an attack, feature or initial creature state.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ConditionApplication {
    pub condition: String,
    #[serde(default)]
    pub escape_dc: Option<i32>,
    #[serde(default)]
    pub duration: Option<DurationSpec>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// How long an applied condition lasts.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DurationSpec {
    /// Symbolic expiry, e.g. `end_of_source_next_turn`.
    #[serde(default)]
    pub until: Option<String>,
    /// Expiry at the end of the affected creature's Nth turn.
    #[serde(default)]
    pub rounds: Option<u32>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// An encounter definition: zones plus member files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Encounter {
    pub name: String,
    #[serde(default)]
    pub zones: BTreeMap<String, i32>,
    #[serde(default)]
    pub members: Vec<EncounterMember>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// One member entry in an encounter file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EncounterMember {
    pub file: String,
    #[serde(default = "default_count")]
    pub count: u32,
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

fn default_speed() -> i32 {
    30
}

fn default_count() -> u32 {
    1
}

fn default_abilities() -> BTreeMap<String, i32> {
    ["str", "dex", "con", "int", "wis", "cha"]
        .into_iter()
        .map(|ability| (ability.to_string(), 10))
        .collect()
}

/// Parses one combatant from JSON text.
pub fn parse_combatant_json(input: &str) -> Result<CombatantSpec> {
    serde_json::from_str(input).context("failed to parse combatant JSON")
}

/// Parses one combatant from TOML text.
pub fn parse_combatant_toml(input: &str) -> Result<CombatantSpec> {
    toml::from_str(input).context("failed to parse combatant TOML")
}

/// Loads one combatant, detecting JSON or TOML from the file extension.
pub fn load_combatant(path: impl AsRef<Path>) -> Result<CombatantSpec> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read combatant file {}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());

    match extension.as_deref() {
        Some("json") => parse_combatant_json(&text)
            .with_context(|| format!("failed to load combatant {}", path.display())),
        Some("toml") => parse_combatant_toml(&text)
            .with_context(|| format!("failed to load combatant {}", path.display())),
        Some(other) => bail!(
            "unsupported combatant file extension `.{other}` for {} (expected .json or .toml)",
            path.display()
        ),
        None => bail!(
            "combatant file {} has no extension (expected .json or .toml)",
            path.display()
        ),
    }
}

/// Parses an encounter from JSON text.
pub fn parse_encounter_json(input: &str) -> Result<Encounter> {
    serde_json::from_str(input).context("failed to parse encounter JSON")
}

/// Loads an encounter from JSON.
pub fn load_encounter(path: impl AsRef<Path>) -> Result<Encounter> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read encounter file {}", path.display()))?;
    parse_encounter_json(&text)
        .with_context(|| format!("failed to load encounter {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    const COMBATANT_FILES: [&str; 10] = [
        "data/party/milo.json",
        "data/party/rina.json",
        "data/party/sera.json",
        "data/party/will.json",
        "data/monsters/allosaurus-animatronic.json",
        "data/monsters/allosaurus.json",
        "data/monsters/animated-armor.json",
        "data/monsters/guard.json",
        "data/monsters/mimic.json",
        "data/monsters/scarecrow.json",
    ];

    const ENCOUNTER_FILES: [&str; 5] = [
        "data/encounters/party-vs-2-guards.json",
        "data/encounters/party-vs-animatronic-allosaurus.json",
        "data/encounters/party-vs-mimic.json",
        "data/encounters/party-vs-satyr-statue.json",
        "data/encounters/party-vs-scarecrow.json",
    ];

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
            "combat-sim-model-{label}-{}-{nanos}-{count}",
            std::process::id()
        ))
    }

    #[test]
    fn loads_every_data_file_unchanged() -> Result<()> {
        let mut loaded_combatants = 0;
        for relative in COMBATANT_FILES {
            let spec = load_combatant(repo_path(relative))?;
            assert!(!spec.name.is_empty(), "{relative} has no name");
            assert!(spec.max_hp > 0, "{relative} has no positive max_hp");
            loaded_combatants += 1;
        }
        assert_eq!(loaded_combatants, 10);

        let guard = load_combatant(repo_path("data/monsters/guard.json"))?;
        assert_eq!(guard.ac, 16);
        assert_eq!(guard.hp, 11);
        assert_eq!(guard.init_bonus, 1);

        let mimic = load_combatant(repo_path("data/monsters/mimic.json"))?;
        let bite = mimic
            .attacks
            .iter()
            .find(|attack| attack.name == "bite")
            .expect("mimic has a bite attack");
        let vs_condition = bite
            .vs_condition
            .as_ref()
            .expect("mimic bite has a vs_condition");
        assert_eq!(vs_condition.condition, "grappled");
        assert!(vs_condition.advantage);
        assert_eq!(vs_condition.damage.as_deref(), Some("2d8+3"));

        for relative in ENCOUNTER_FILES {
            let encounter_path = repo_path(relative);
            let encounter = load_encounter(&encounter_path)?;
            assert!(!encounter.name.is_empty());
            assert!(!encounter.zones.is_empty(), "{relative} has no zones");
            assert!(!encounter.members.is_empty(), "{relative} has no members");
            let base = encounter_path
                .parent()
                .expect("encounter path has a parent directory");
            for member in &encounter.members {
                assert!(member.count >= 1, "{relative} has a zero-count member");
                let spec = load_combatant(base.join(&member.file))
                    .with_context(|| format!("could not resolve member {}", member.file))?;
                assert!(!spec.name.is_empty());
            }
        }

        assert_eq!(loaded_combatants + ENCOUNTER_FILES.len(), 15);
        Ok(())
    }

    #[test]
    fn loads_minimal_toml_combatant() -> Result<()> {
        let directory = unique_temp_dir("toml");
        fs::create_dir_all(&directory)?;
        let path = directory.join("test-guard.toml");
        fs::write(
            &path,
            r#"
name = "Test Guard"
side = "enemy"
kind = "monster"
hp = 11
max_hp = 11
ac = 16
notes = "minimal toml"

[[attacks]]
name = "spear"
kind = "melee"
to_hit = 3
damage = "1d6+1"
damage_type = "piercing"
"#,
        )?;

        let spec = load_combatant(&path)?;
        assert_eq!(spec.name, "Test Guard");
        assert_eq!(spec.side, Side::Enemy);
        assert_eq!(spec.kind, Kind::Monster);
        assert_eq!(spec.temp_hp, 0);
        assert_eq!(spec.init_bonus, 0);
        assert_eq!(spec.speed, 30);
        assert_eq!(spec.abilities.len(), 6);
        assert_eq!(spec.abilities.get("str"), Some(&10));
        assert_eq!(spec.attacks.len(), 1);
        assert_eq!(spec.attacks[0].count, 1);
        assert_eq!(spec.attacks[0].kind, AttackKind::Melee);
        assert_eq!(spec.notes.as_deref(), Some("minimal toml"));

        fs::remove_dir_all(&directory)?;
        Ok(())
    }

    #[test]
    fn unknown_enum_values_fall_back_to_unknown() -> Result<()> {
        let spec = parse_combatant_json(
            r#"{
                "name": "Test Creature",
                "side": "neutral",
                "kind": "construct",
                "hp": 1,
                "max_hp": 1,
                "ac": 10
            }"#,
        )?;
        assert_eq!(spec.side, Side::Unknown);
        assert_eq!(spec.kind, Kind::Unknown);
        Ok(())
    }
}
