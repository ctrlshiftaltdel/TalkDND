//! Content data model for campaign-driven combat data.
//!
//! Everything here is pure data: no campaign, monster, spell, condition or
//! mastery names are baked into code. Fixed structs reject unknown fields so a
//! typo in a content file is reported instead of silently ignored; free-form
//! maps (`abilities`, `saves`, resources, ability params, ...) stay open.
//!
//! [`Effect`] and [`Predicate`] use hand-written deserializers because
//! `serde_norway` (like `serde_yaml`) refuses plain-map externally tagged
//! enums: the content writes `{damage: {...}}`, not `!Damage {...}`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ---------------------------------------------------------------------------
// Shared value types
// ---------------------------------------------------------------------------

/// A side in combat. Open on purpose: campaigns may use `party`, `enemy`,
/// `rampage`, `neutral`, ... and every side is hostile to every other side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Side(pub String);

impl Side {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Side {
    fn default() -> Self {
        Self::new("enemy")
    }
}

/// Player character or monster. Unknown kinds are tolerated for forward
/// compatibility; the engine treats them like monsters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatureKind {
    Pc,
    /// Companion NPC built like a character: makes death saves on the party side.
    Npc,
    #[default]
    Monster,
    #[serde(other)]
    Unknown,
}

/// 5e creature size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    Tiny,
    Small,
    #[default]
    Medium,
    Large,
    Huge,
    Gargantuan,
    #[serde(other)]
    Unknown,
}

/// The six ability scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityName {
    Str,
    Dex,
    Con,
    Int,
    Wis,
    Cha,
}

impl AbilityName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Str => "str",
            Self::Dex => "dex",
            Self::Con => "con",
            Self::Int => "int",
            Self::Wis => "wis",
            Self::Cha => "cha",
        }
    }
}

/// A damage type. Open-ended: `weapon` is a special placeholder replaced by
/// the triggering attack's damage type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DamageType(pub String);

impl DamageType {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How an attack resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackKind {
    #[default]
    Melee,
    Ranged,
    MeleeOrRanged,
}

/// A 2024 weapon property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackProperty {
    Finesse,
    Light,
    Thrown,
    Ranged,
    Versatile,
    Heavy,
    #[serde(rename = "two-handed")]
    TwoHanded,
    Reach,
    Loading,
    Ammunition,
    Special,
    #[serde(other)]
    Unknown,
}

/// When an ability can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    #[default]
    Action,
    Bonus,
    Reaction,
    Passive,
    Free,
}

/// Whether a limited-use resource resets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Recharge {
    Turn,
    Round,
    Short,
    #[default]
    Long,
    Never,
    /// `recharge N`
    Single(u32),
    /// `recharge N-M`
    Range {
        min: u32,
        max: u32,
    },
}

impl fmt::Display for Recharge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Turn => f.write_str("turn"),
            Self::Round => f.write_str("round"),
            Self::Short => f.write_str("short"),
            Self::Long => f.write_str("long"),
            Self::Never => f.write_str("never"),
            Self::Single(value) => write!(f, "recharge {value}"),
            Self::Range { min, max } => write!(f, "recharge {min}-{max}"),
        }
    }
}

impl Serialize for Recharge {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Recharge {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RechargeVisitor;

        impl Visitor<'_> for RechargeVisitor {
            type Value = Recharge;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(
                    "`turn`, `round`, `short`, `long`, `never`, `recharge N` or `recharge N-M`",
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Recharge, E>
            where
                E: de::Error,
            {
                let text = value.trim().to_ascii_lowercase();
                match text.as_str() {
                    "turn" => return Ok(Recharge::Turn),
                    "round" => return Ok(Recharge::Round),
                    "short" => return Ok(Recharge::Short),
                    "long" => return Ok(Recharge::Long),
                    "never" => return Ok(Recharge::Never),
                    _ => {}
                }
                let Some(rest) = text.strip_prefix("recharge") else {
                    return Err(E::custom(format!(
                        "invalid recharge value `{value}`; expected turn, round, short, long, never, `recharge N` or `recharge N-M`"
                    )));
                };
                let rest = rest.trim();
                if rest.is_empty() {
                    return Err(E::custom(
                        "recharge value is missing a number after `recharge`",
                    ));
                }
                if let Some((min, max)) = rest.split_once('-') {
                    let min = parse_recharge_number::<E>(min.trim())?;
                    let max = parse_recharge_number::<E>(max.trim())?;
                    if min > max {
                        return Err(E::custom(format!(
                            "recharge range `{value}` starts above its maximum"
                        )));
                    }
                    return Ok(Recharge::Range { min, max });
                }
                Ok(Recharge::Single(parse_recharge_number::<E>(rest)?))
            }
        }

        deserializer.deserialize_str(RechargeVisitor)
    }
}

fn parse_recharge_number<E: de::Error>(text: &str) -> Result<u32, E> {
    text.parse::<u32>()
        .map_err(|_| E::custom(format!("invalid recharge number `{text}`")))
}

/// A boolean predicate leaf. YAML writers often leave `{flag:}` empty as a
/// shorthand for true, so `true`, `false` and null (=> true) are all accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Flag(pub bool);

impl Default for Flag {
    fn default() -> Self {
        Self(true)
    }
}

impl Flag {
    pub fn get(self) -> bool {
        self.0
    }
}

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FlagVisitor;

        impl<'de> Visitor<'de> for FlagVisitor {
            type Value = Flag;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a boolean, or null for true")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Flag, E> {
                Ok(Flag(value))
            }

            fn visit_unit<E>(self) -> Result<Flag, E> {
                Ok(Flag(true))
            }

            fn visit_none<E>(self) -> Result<Flag, E> {
                Ok(Flag(true))
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Flag, D::Error>
            where
                D: Deserializer<'de>,
            {
                bool::deserialize(deserializer).map(Flag)
            }
        }

        deserializer.deserialize_option(FlagVisitor)
    }
}

/// A number or a dice/expression string. Expressions are resolved lazily with
/// [`crate::content::expr`] once a context is known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExprValue {
    Number(i64),
    Text(String),
}

impl ExprValue {
    pub fn as_text(&self) -> String {
        match self {
            Self::Number(value) => value.to_string(),
            Self::Text(value) => value.clone(),
        }
    }
}

impl From<i64> for ExprValue {
    fn from(value: i64) -> Self {
        Self::Number(value)
    }
}

impl From<&str> for ExprValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<String> for ExprValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

// ---------------------------------------------------------------------------
// Content file / manifest
// ---------------------------------------------------------------------------

/// A content document: a container map or (handled by the loader) a single
/// item. Every list is optional so a file may contain only one kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentFile {
    #[serde(default)]
    pub conditions: Option<Vec<ConditionDef>>,
    #[serde(default)]
    pub masteries: Option<Vec<MasteryDef>>,
    #[serde(default)]
    pub abilities: Option<Vec<Ability>>,
    #[serde(default)]
    pub creatures: Option<Vec<Creature>>,
    #[serde(default)]
    pub ai_templates: Option<Vec<AiTemplate>>,
    #[serde(default)]
    pub scenes: Option<Vec<Scene>>,
    #[serde(default)]
    pub encounters: Option<Vec<Encounter>>,
    #[serde(default)]
    pub rules: Option<Rules>,
}

/// Campaign manifest (`--campaign`). Paths are resolved relative to the
/// manifest file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignManifest {
    pub name: String,
    #[serde(default)]
    pub library: Vec<PathBuf>,
    #[serde(default)]
    pub adventures: Vec<PathBuf>,
    #[serde(default)]
    pub characters: Vec<PathBuf>,
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
    /// Default party creature ids for `new` / `simulate` when `--party` is omitted.
    #[serde(default)]
    pub party: Vec<String>,
    /// Rules edition: `2014` (5e) or `2024` (5.5e). Default `2024`. Selects the
    /// `<lib>/<ruleset>/` layer next to `<lib>/common/` in layered libraries.
    #[serde(default)]
    pub ruleset: Option<String>,
}

/// Core-rule wiring supplied by data. No condition ids are hard-coded in code.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    #[serde(default)]
    pub dying_conditions: Option<Vec<String>>,
    #[serde(default)]
    pub dead_monster_condition: Option<String>,
    #[serde(default)]
    pub basic_actions: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub cover: Option<CoverRules>,
    #[serde(default)]
    pub concentration_condition: Option<String>,
    /// How `init --surprised` works (edition-dependent).
    #[serde(default)]
    pub surprise: Option<SurpriseRule>,
}

/// Surprise handling. 2024: initiative with disadvantage. 2014: normal
/// initiative, plus a condition that lasts until the end of the creature's first turn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurpriseRule {
    #[serde(default)]
    pub initiative_disadvantage: Option<bool>,
    #[serde(default)]
    pub condition: Option<String>,
}

impl Rules {
    /// Merges `other` into `self`, field-wise. Maps merge entry by entry;
    /// later values win.
    pub fn merge_from(&mut self, other: Rules) {
        if other.dying_conditions.is_some() {
            self.dying_conditions = other.dying_conditions;
        }
        if other.dead_monster_condition.is_some() {
            self.dead_monster_condition = other.dead_monster_condition;
        }
        if let Some(actions) = other.basic_actions {
            self.basic_actions
                .get_or_insert_with(BTreeMap::new)
                .extend(actions);
        }
        if let Some(cover) = other.cover {
            match &mut self.cover {
                Some(existing) => existing.merge_from(cover),
                slot @ None => *slot = Some(cover),
            }
        }
        if other.concentration_condition.is_some() {
            self.concentration_condition = other.concentration_condition;
        }
        if other.surprise.is_some() {
            self.surprise = other.surprise;
        }
    }
}

/// Cover bonuses for the two cover levels that modify AC/Dex saves.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverRules {
    #[serde(default)]
    pub half: Option<i32>,
    #[serde(default)]
    pub three_quarters: Option<i32>,
}

impl CoverRules {
    fn merge_from(&mut self, other: CoverRules) {
        if other.half.is_some() {
            self.half = other.half;
        }
        if other.three_quarters.is_some() {
            self.three_quarters = other.three_quarters;
        }
    }
}

// ---------------------------------------------------------------------------
// Creature
// ---------------------------------------------------------------------------

/// A creature. Fields are optional where `base:` inheritance can fill them in;
/// use the accessor defaults (`ac()`, `max_hp()`, `speed()`, ...) after
/// [`crate::content::ContentRegistry::resolve_creature`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Creature {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub kind: Option<CreatureKind>,
    #[serde(default)]
    pub side: Option<Side>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub size: Option<Size>,
    #[serde(default)]
    pub creature_type: Option<String>,
    #[serde(default)]
    pub level: Option<i32>,
    #[serde(default)]
    pub ac: Option<i32>,
    /// Maximum hit points.
    #[serde(default)]
    pub hp: Option<i32>,
    #[serde(default)]
    pub hp_current: Option<i32>,
    #[serde(default)]
    pub speed: Option<Speed>,
    #[serde(default)]
    pub init_bonus: Option<i32>,
    #[serde(default)]
    pub prof: Option<i32>,
    #[serde(default)]
    pub abilities: Option<BTreeMap<String, i32>>,
    #[serde(default)]
    pub saves: Option<BTreeMap<String, i32>>,
    #[serde(default)]
    pub skills: Option<BTreeMap<String, i32>>,
    #[serde(default)]
    pub spellcasting: Option<Spellcasting>,
    #[serde(default)]
    pub resistances: Option<Vec<String>>,
    #[serde(default)]
    pub immunities: Option<Vec<String>>,
    #[serde(default)]
    pub vulnerabilities: Option<Vec<String>>,
    #[serde(default)]
    pub condition_immunities: Option<Vec<String>>,
    #[serde(default)]
    pub senses: Option<String>,
    #[serde(default)]
    pub resources: Option<BTreeMap<String, ResourceDef>>,
    #[serde(default)]
    pub attacks: Option<Vec<Attack>>,
    #[serde(default)]
    pub actions: Option<Vec<AbilityEntry>>,
    #[serde(default)]
    pub features: Option<Vec<AbilityEntry>>,
    #[serde(default)]
    pub multiattack: Option<Vec<String>>,
    /// Initial condition ids.
    #[serde(default)]
    pub conditions: Option<Vec<String>>,
    #[serde(default)]
    pub dormant: Option<bool>,
    #[serde(default)]
    pub ai: Option<AiAssignment>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Optional so a base child that omits `todo` inherits the parent's list.
    #[serde(default)]
    pub todo: Option<Vec<String>>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl Creature {
    pub(crate) fn merge_from(&mut self, child: Creature) {
        macro_rules! replace {
            ($field:ident) => {
                if child.$field.is_some() {
                    self.$field = child.$field;
                }
            };
        }
        replace!(name);
        replace!(kind);
        replace!(side);
        replace!(tags);
        replace!(size);
        replace!(creature_type);
        replace!(level);
        replace!(ac);
        replace!(hp);
        replace!(hp_current);
        replace!(speed);
        replace!(init_bonus);
        replace!(prof);
        merge_map(&mut self.abilities, child.abilities);
        merge_map(&mut self.saves, child.saves);
        merge_map(&mut self.skills, child.skills);
        replace!(spellcasting);
        replace!(resistances);
        replace!(immunities);
        replace!(vulnerabilities);
        replace!(condition_immunities);
        replace!(senses);
        merge_map(&mut self.resources, child.resources);
        replace!(attacks);
        replace!(actions);
        replace!(features);
        replace!(multiattack);
        replace!(conditions);
        replace!(dormant);
        replace!(ai);
        replace!(manual);
        replace!(notes);
        replace!(todo);
        replace!(source);
        replace!(description);
        self.id = child.id;
        self.base = child.base;
    }

    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    pub fn kind(&self) -> CreatureKind {
        self.kind.unwrap_or_default()
    }

    pub fn side(&self) -> Side {
        self.side.clone().unwrap_or_default()
    }

    pub fn size(&self) -> Size {
        self.size.unwrap_or_default()
    }

    pub fn ac(&self) -> i32 {
        self.ac.unwrap_or(10)
    }

    pub fn max_hp(&self) -> i32 {
        self.hp.unwrap_or(1)
    }

    pub fn current_hp(&self) -> i32 {
        self.hp_current.unwrap_or_else(|| self.max_hp())
    }

    pub fn speed(&self) -> Speed {
        self.speed.clone().unwrap_or_default()
    }

    pub fn init_bonus(&self) -> i32 {
        self.init_bonus.unwrap_or(0)
    }

    pub fn proficiency_bonus(&self) -> i32 {
        self.prof.unwrap_or(2)
    }

    pub fn level(&self) -> i32 {
        self.level.unwrap_or(1)
    }

    /// Ability modifier for a score; defaults to 10 (modifier 0) when absent.
    pub fn ability_modifier(&self, ability: AbilityName) -> i32 {
        let score = self
            .abilities
            .as_ref()
            .and_then(|abilities| abilities.get(ability.as_str()))
            .copied()
            .unwrap_or(10);
        (score - 10).div_euclid(2)
    }
}

/// Merges a free-form child map over a base map (child entries win).
fn merge_map<V>(base: &mut Option<BTreeMap<String, V>>, child: Option<BTreeMap<String, V>>) {
    if let Some(child) = child {
        base.get_or_insert_with(BTreeMap::new).extend(child);
    }
}

/// Walking, climbing, flying and swimming speeds in feet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Speed {
    #[serde(default = "default_walk_speed")]
    pub walk: i32,
    #[serde(default)]
    pub climb: i32,
    #[serde(default)]
    pub fly: i32,
    #[serde(default)]
    pub swim: i32,
}

fn default_walk_speed() -> i32 {
    30
}

impl Default for Speed {
    fn default() -> Self {
        Self {
            walk: default_walk_speed(),
            climb: 0,
            fly: 0,
            swim: 0,
        }
    }
}

/// Spellcasting numbers for `{spell_*}` expressions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spellcasting {
    pub ability: AbilityName,
    #[serde(default)]
    pub attack: Option<i32>,
    #[serde(default)]
    pub dc: Option<i32>,
}

/// A named resource pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceDef {
    pub max: i32,
    pub recharge: Recharge,
    #[serde(default)]
    pub short_rest_regain: Option<i32>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Die size for a `hit-dice` pool (d6..d12); spent on short rests.
    #[serde(default)]
    pub die: Option<u32>,
}

/// A weapon or natural attack on a creature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attack {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub kind: AttackKind,
    #[serde(default)]
    pub ability: Option<AbilityName>,
    #[serde(default)]
    pub to_hit: Option<ExprValue>,
    #[serde(default)]
    pub damage: Vec<DamagePart>,
    #[serde(default)]
    pub reach: Option<i32>,
    /// Normal and long range in feet.
    #[serde(default)]
    pub range: Option<[i32; 2]>,
    #[serde(default)]
    pub properties: Vec<AttackProperty>,
    #[serde(default)]
    pub mastery: Option<String>,
    /// Reroll damage dice showing this value or lower once, keeping the new
    /// roll (Great Weapon Fighting: 2).
    #[serde(default)]
    pub reroll_damage_below: Option<u32>,
    /// Treat damage dice below this value as this value (2024 Great Weapon
    /// Fighting: 3).
    #[serde(default)]
    pub damage_die_min: Option<u32>,
    #[serde(default)]
    pub on_hit: Vec<Effect>,
    #[serde(default)]
    pub cost: Option<AbilityCost>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl Attack {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// One damage component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamagePart {
    pub dice: ExprValue,
    #[serde(rename = "type")]
    pub damage_type: DamageType,
}

/// Ranges and targets for an ability.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSpec {
    #[serde(rename = "type")]
    pub target_type: TargetType,
    #[serde(default)]
    pub side: Option<TargetSide>,
    #[serde(default)]
    pub count: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetType {
    Creature,
    #[default]
    #[serde(rename = "self")]
    SelfTarget,
    Zone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetSide {
    Enemy,
    Ally,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaSpec {
    pub radius: i32,
}

/// A limited-use cost such as one spell slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbilityCost {
    pub resource: String,
    #[serde(default = "default_amount")]
    pub amount: u32,
}

fn default_amount() -> u32 {
    1
}

// ---------------------------------------------------------------------------
// Ability
// ---------------------------------------------------------------------------

/// A spell, feature, item action or trap effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ability {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub activation: Activation,
    #[serde(default)]
    pub cost: Option<AbilityCost>,
    #[serde(default)]
    pub slot_level: Option<i32>,
    #[serde(default)]
    pub range: Option<i32>,
    #[serde(default)]
    pub target: Option<TargetSpec>,
    #[serde(default)]
    pub area: Option<AreaSpec>,
    #[serde(default)]
    pub concentration: bool,
    #[serde(default)]
    pub duration: Option<Duration>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    #[serde(default)]
    pub ai: Option<AbilityAi>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl Ability {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbilityAi {
    pub role: AiRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiRole {
    Damage,
    Heal,
    SelfHeal,
    Buff,
    Control,
    Defense,
    #[default]
    None,
}

/// A reference to a shared ability or a complete inline ability.
#[derive(Debug, Clone, PartialEq)]
pub enum AbilityEntry {
    Ref(AbilityRef),
    Inline(Box<Ability>),
}

/// `{ref: id, params, cost, name}` — references are resolved from the registry
/// with optional per-creature overrides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbilityRef {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub params: BTreeMap<String, ExprValue>,
    #[serde(default)]
    pub cost: Option<AbilityCost>,
    #[serde(default)]
    pub name: Option<String>,
}

impl AbilityEntry {
    pub fn id(&self) -> &str {
        match self {
            Self::Ref(reference) => &reference.reference,
            Self::Inline(ability) => &ability.id,
        }
    }

    pub fn cost(&self) -> Option<&AbilityCost> {
        match self {
            Self::Ref(reference) => reference.cost.as_ref(),
            Self::Inline(ability) => ability.cost.as_ref(),
        }
    }
}

impl Serialize for AbilityEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Ref(reference) => reference.serialize(serializer),
            Self::Inline(ability) => ability.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for AbilityEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value.as_object().ok_or_else(|| {
            de::Error::custom("ability entry must be a map with `ref` or an inline ability")
        })?;
        if object.contains_key("ref") {
            serde_json::from_value::<AbilityRef>(value)
                .map(AbilityEntry::Ref)
                .map_err(de::Error::custom)
        } else if object.contains_key("id") {
            serde_json::from_value::<Ability>(value)
                .map(Box::new)
                .map(AbilityEntry::Inline)
                .map_err(de::Error::custom)
        } else {
            Err(de::Error::custom(
                "ability entry must contain `ref` or an inline ability with `id`",
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Duration, effects and triggers
// ---------------------------------------------------------------------------

/// How long an applied effect lasts. Exactly one of `rounds`, `until`,
/// `minutes` or `permanent` must be supplied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Duration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rounds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<DurationUntil>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permanent: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ends_on: Vec<EndsOn>,
}

impl Duration {
    fn kind_count(&self) -> usize {
        usize::from(self.rounds.is_some())
            + usize::from(self.until.is_some())
            + usize::from(self.minutes.is_some())
            + usize::from(self.permanent.is_some())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.kind_count() != 1 {
            return Err(
                "duration must specify exactly one of `rounds`, `until`, `minutes` or `permanent`"
                    .to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurationRaw {
    #[serde(default)]
    rounds: Option<u32>,
    #[serde(default)]
    until: Option<DurationUntil>,
    #[serde(default)]
    minutes: Option<u32>,
    #[serde(default)]
    permanent: Option<bool>,
    #[serde(default)]
    ends_on: Vec<EndsOn>,
}

impl<'de> Deserialize<'de> for Duration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = DurationRaw::deserialize(deserializer)?;
        let duration = Self {
            rounds: raw.rounds,
            until: raw.until,
            minutes: raw.minutes,
            permanent: raw.permanent,
            ends_on: raw.ends_on,
        };
        duration.validate().map_err(de::Error::custom)?;
        Ok(duration)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationUntil {
    EndOfSourceNextTurn,
    StartOfSourceNextTurn,
    EndOfTargetNextTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndsOn {
    DamageTaken,
    OwnAttack,
    OwnSpell,
    DealsDamage,
}

/// One primitive effect. Exactly one map key selects the effect.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Attack(AttackEffect),
    Save(SaveEffect),
    Damage(DamageEffect),
    Heal(HealEffect),
    ApplyCondition(ApplyConditionEffect),
    RemoveCondition(RemoveConditionEffect),
    ForcedMove(ForcedMoveEffect),
    Use(String),
    Activate(ActivateEffect),
    Check(CheckEffect),
    BasicAction(BasicAction),
    Choice(Vec<Vec<Effect>>),
    Note(String),
    ToggleTrigger(ToggleTriggerEffect),
}

const EFFECT_KEYS: &[&str] = &[
    "attack",
    "save",
    "damage",
    "heal",
    "apply_condition",
    "remove_condition",
    "forced_move",
    "use",
    "activate",
    "check",
    "basic_action",
    "choice",
    "note",
    "toggle_trigger",
];

impl<'de> Deserialize<'de> for Effect {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EffectVisitor;

        impl<'de> Visitor<'de> for EffectVisitor {
            type Value = Effect;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map with exactly one effect key")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Effect, A::Error>
            where
                A: MapAccess<'de>,
            {
                let key: String = match map.next_key()? {
                    Some(key) => key,
                    None => {
                        return Err(de::Error::custom(
                            "effect map is empty; expected a single effect key",
                        ))
                    }
                };
                let effect = match key.as_str() {
                    "attack" => Effect::Attack(map.next_value()?),
                    "save" => Effect::Save(map.next_value()?),
                    "damage" => Effect::Damage(map.next_value()?),
                    "heal" => Effect::Heal(map.next_value()?),
                    "apply_condition" => Effect::ApplyCondition(map.next_value()?),
                    "remove_condition" => Effect::RemoveCondition(map.next_value()?),
                    "forced_move" => Effect::ForcedMove(map.next_value()?),
                    "use" => Effect::Use(map.next_value()?),
                    "activate" => Effect::Activate(map.next_value()?),
                    "check" => Effect::Check(map.next_value()?),
                    "basic_action" => Effect::BasicAction(map.next_value()?),
                    "choice" => Effect::Choice(map.next_value()?),
                    "note" => Effect::Note(map.next_value()?),
                    "toggle_trigger" => Effect::ToggleTrigger(map.next_value()?),
                    _ => return Err(de::Error::unknown_variant(&key, EFFECT_KEYS)),
                };
                if let Some(extra) = map.next_key::<String>()? {
                    return Err(de::Error::custom(format!(
                        "effect map must contain exactly one key; found extra key `{extra}`"
                    )));
                }
                Ok(effect)
            }
        }

        deserializer.deserialize_map(EffectVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttackEffect {
    pub to_hit: ExprValue,
    pub kind: AttackKind,
    #[serde(default)]
    pub damage: Vec<DamagePart>,
    #[serde(default)]
    pub on_hit: Vec<Effect>,
    #[serde(default)]
    pub on_miss: Vec<Effect>,
    #[serde(default)]
    pub properties: Vec<AttackProperty>,
    #[serde(default)]
    pub mastery: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveEffect {
    pub ability: AbilityName,
    pub dc: ExprValue,
    #[serde(default)]
    pub half: bool,
    #[serde(default)]
    pub ignore_cover: bool,
    #[serde(default)]
    pub on_fail: Vec<Effect>,
    #[serde(default)]
    pub on_success: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageEffect {
    pub dice: ExprValue,
    #[serde(rename = "type")]
    pub damage_type: DamageType,
    #[serde(default = "default_repeat")]
    pub repeat: u32,
    #[serde(default)]
    pub crit: CritMode,
}

fn default_repeat() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CritMode {
    #[default]
    Inherit,
    Never,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealEffect {
    pub dice: ExprValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyConditionEffect {
    pub condition: String,
    #[serde(default)]
    pub to: TargetRef,
    #[serde(default)]
    pub duration: Option<Duration>,
    #[serde(default)]
    pub link: bool,
    #[serde(default)]
    pub ends_on: Vec<EndsOn>,
    #[serde(default)]
    pub save_ends: Option<SaveEnds>,
}

/// A repeated save that can end an applied condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveEnds {
    pub ability: AbilityName,
    pub dc: ExprValue,
    pub at: SaveEndsAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveEndsAt {
    EndOfTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRef {
    #[default]
    Target,
    #[serde(rename = "self")]
    SelfTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveConditionEffect {
    pub condition: String,
    #[serde(default)]
    pub from: TargetRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForcedMoveEffect {
    pub distance: i32,
    pub direction: Direction,
    #[serde(default)]
    pub max_size: Option<Size>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Away,
    Toward,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivateEffect {
    pub creature: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckEffect {
    #[serde(default)]
    pub skill: Option<String>,
    #[serde(default)]
    pub ability: Option<AbilityName>,
    pub dc: ExprValue,
    #[serde(default)]
    pub who: CheckWho,
    #[serde(default)]
    pub on_success: Vec<Effect>,
    #[serde(default)]
    pub on_fail: Vec<Effect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckWho {
    #[default]
    Actor,
    Target,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BasicAction {
    Dash,
    Disengage,
    Dodge,
    Hide,
    Help,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToggleTriggerEffect {
    pub trigger: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// A condition/feature trigger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    pub on: TriggerWhen,
    #[serde(default)]
    pub when: Option<Predicate>,
    #[serde(default)]
    pub once_per: Option<OncePer>,
    #[serde(default)]
    pub reaction: bool,
    #[serde(default)]
    pub cost: Option<AbilityCost>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerWhen {
    OnHit,
    OnMiss,
    OnHitTaken,
    OnDamageTaken,
    StartOfTurn,
    EndOfTurn,
    OnEnterArea,
    OnInteract,
    OnHealCast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OncePer {
    Turn,
    Round,
}

// ---------------------------------------------------------------------------
// Predicates and modifiers
// ---------------------------------------------------------------------------

/// A condition-style predicate. Exactly one map key selects the leaf or
/// combinator.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Predicate {
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
    Not(Box<Predicate>),
    HasAdvantage(Flag),
    HasDisadvantage(Flag),
    #[serde(rename = "ally_within_5_of_target")]
    AllyWithin5OfTarget(Flag),
    TargetHasCondition(String),
    SelfHasCondition(String),
    AttackerWithin(i32),
    AttackerBeyond(i32),
    LinkedIsAttacker(Flag),
    LinkedIsTarget(Flag),
    AttackHasProperty(Vec<AttackProperty>),
    AttackKind(AttackKind),
    SelfHpBelowPct(i32),
    TargetHpBelowPct(i32),
    IsSpell(Flag),
    SlotLevelMin(i32),
    DamageDealt(Flag),
    WouldMissWithAcBonus(i32),
    TargetSizeMax(Size),
}

const PREDICATE_KEYS: &[&str] = &[
    "all",
    "any",
    "not",
    "has_advantage",
    "has_disadvantage",
    "ally_within_5_of_target",
    "target_has_condition",
    "self_has_condition",
    "attacker_within",
    "attacker_beyond",
    "linked_is_attacker",
    "linked_is_target",
    "attack_has_property",
    "attack_kind",
    "self_hp_below_pct",
    "target_hp_below_pct",
    "is_spell",
    "slot_level_min",
    "damage_dealt",
    "would_miss_with_ac_bonus",
    "target_size_max",
];

impl<'de> Deserialize<'de> for Predicate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PredicateVisitor;

        impl<'de> Visitor<'de> for PredicateVisitor {
            type Value = Predicate;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map with exactly one predicate key")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Predicate, A::Error>
            where
                A: MapAccess<'de>,
            {
                let key: String = match map.next_key()? {
                    Some(key) => key,
                    None => {
                        return Err(de::Error::custom(
                            "predicate map is empty; expected a single predicate key",
                        ))
                    }
                };
                let predicate = match key.as_str() {
                    "all" => Predicate::All(map.next_value()?),
                    "any" => Predicate::Any(map.next_value()?),
                    "not" => Predicate::Not(map.next_value()?),
                    "has_advantage" => Predicate::HasAdvantage(map.next_value()?),
                    "has_disadvantage" => Predicate::HasDisadvantage(map.next_value()?),
                    "ally_within_5_of_target" => Predicate::AllyWithin5OfTarget(map.next_value()?),
                    "target_has_condition" => Predicate::TargetHasCondition(map.next_value()?),
                    "self_has_condition" => Predicate::SelfHasCondition(map.next_value()?),
                    "attacker_within" => Predicate::AttackerWithin(map.next_value()?),
                    "attacker_beyond" => Predicate::AttackerBeyond(map.next_value()?),
                    "linked_is_attacker" => Predicate::LinkedIsAttacker(map.next_value()?),
                    "linked_is_target" => Predicate::LinkedIsTarget(map.next_value()?),
                    "attack_has_property" => Predicate::AttackHasProperty(map.next_value()?),
                    "attack_kind" => Predicate::AttackKind(map.next_value()?),
                    "self_hp_below_pct" => Predicate::SelfHpBelowPct(map.next_value()?),
                    "target_hp_below_pct" => Predicate::TargetHpBelowPct(map.next_value()?),
                    "is_spell" => Predicate::IsSpell(map.next_value()?),
                    "slot_level_min" => Predicate::SlotLevelMin(map.next_value()?),
                    "damage_dealt" => Predicate::DamageDealt(map.next_value()?),
                    "would_miss_with_ac_bonus" => {
                        Predicate::WouldMissWithAcBonus(map.next_value()?)
                    }
                    "target_size_max" => Predicate::TargetSizeMax(map.next_value()?),
                    _ => return Err(de::Error::unknown_variant(&key, PREDICATE_KEYS)),
                };
                if let Some(extra) = map.next_key::<String>()? {
                    return Err(de::Error::custom(format!(
                        "predicate map must contain exactly one key; found extra key `{extra}`"
                    )));
                }
                Ok(predicate)
            }
        }

        deserializer.deserialize_map(PredicateVisitor)
    }
}

/// A conditional modifier attached to a condition or ability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modifier {
    pub applies: ModifierApplies,
    pub effect: ModifierEffect,
    #[serde(default)]
    pub value: Option<ExprValue>,
    #[serde(default)]
    pub filter: Option<ModifierFilter>,
    #[serde(default)]
    pub when: Option<Predicate>,
    #[serde(default)]
    pub once_per: Option<OncePer>,
    #[serde(default)]
    pub consume: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModifierApplies {
    AttackRoll,
    IncomingAttack,
    Save,
    Check,
    Ac,
    DamageRoll,
    DamageTaken,
    Speed,
    HealingDone,
    HealingReceived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModifierEffect {
    Advantage,
    Disadvantage,
    Bonus,
    AutoFail,
    AutoCrit,
    Resistance,
    Immunity,
    Vulnerability,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModifierFilter {
    #[serde(default)]
    pub ability: Option<Vec<AbilityName>>,
    #[serde(default)]
    pub damage_type: Option<Vec<DamageType>>,
    #[serde(default)]
    pub attack_kind: Option<Vec<AttackKind>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Conditions and masteries
// ---------------------------------------------------------------------------

/// Generic flags the engine understands. Everything else is a modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionFlag {
    Incapacitated,
    SpeedZero,
    NoReactions,
    NoOpportunityAttacks,
    NoOpportunityAttacksAgainst,
    StandUpHalfSpeed,
    Unaware,
    /// Speed is halved (for example while carrying or dragging a creature).
    SpeedHalved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionDef {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub flags: Vec<ConditionFlag>,
    #[serde(default)]
    pub implies: Vec<String>,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    #[serde(default)]
    pub duration_default: Option<Duration>,
    #[serde(default)]
    pub ends_on: Vec<EndsOn>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl ConditionDef {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

impl MasteryDef {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MasteryDef {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// AI templates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiTemplate {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub params: AiParams,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Every AI parameter is optional so assignments can override a subset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiParams {
    #[serde(default)]
    pub targeting: Option<AiTargeting>,
    #[serde(default)]
    pub range_pref: Option<RangePreference>,
    #[serde(default)]
    pub keep_distance: Option<i32>,
    #[serde(default)]
    pub self_heal_below_pct: Option<i32>,
    #[serde(default)]
    pub heal_ally_below_pct: Option<i32>,
    #[serde(default)]
    pub heal_downed: Option<bool>,
    #[serde(default)]
    pub resource_threshold: Option<f64>,
    #[serde(default)]
    pub flee_below_pct: Option<i32>,
    /// Zone to retreat to when fleeing (optional).
    #[serde(default)]
    pub flee_zone: Option<String>,
    #[serde(default)]
    pub guard_zone: Option<String>,
    #[serde(default)]
    pub leash: Option<i32>,
    #[serde(default)]
    pub attack_downed: Option<bool>,
    #[serde(default)]
    pub opening: Option<Vec<String>>,
    #[serde(default)]
    pub avoid_targets_with: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiTargeting {
    Nearest,
    LowestHp,
    HighestDamage,
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangePreference {
    Melee,
    Ranged,
    Any,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiAssignment {
    pub template: String,
    #[serde(default)]
    pub params: Option<AiParams>,
}

// ---------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub mode: SceneMode,
    #[serde(default)]
    pub light: Option<Light>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub objects: Vec<SceneObject>,
    #[serde(default)]
    pub triggers: Vec<SceneTrigger>,
    #[serde(default)]
    pub grid: Option<Grid>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

impl Scene {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneMode {
    #[default]
    Zones,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Light {
    Bright,
    Dim,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cover {
    #[default]
    None,
    Half,
    ThreeQuarters,
    Total,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub cover: Cover,
    #[serde(default)]
    pub elevation: Option<i32>,
    #[serde(default)]
    pub light: Option<Light>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub a: String,
    pub b: String,
    pub distance: i32,
    #[serde(default)]
    pub climb: Option<ClimbSpec>,
    #[serde(default)]
    pub door: Option<DoorSpec>,
    #[serde(default)]
    pub difficult: bool,
    #[serde(default)]
    pub one_way: bool,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClimbSpec {
    pub skill: String,
    pub dc: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoorSpec {
    #[serde(default)]
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneObject {
    pub id: String,
    pub zone: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub interact: Option<Interact>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Interaction behaviour for a scene object. Supply either the direct
/// `effects` / `check` form or an `options` list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interact {
    #[serde(default)]
    pub options: Option<Vec<InteractOption>>,
    #[serde(default)]
    pub check: Option<CheckSpec>,
    #[serde(default)]
    pub effects: Option<Vec<Effect>>,
    #[serde(default)]
    pub on_success: Option<Vec<Effect>>,
    #[serde(default)]
    pub on_fail: Option<Vec<Effect>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractOption {
    pub name: String,
    #[serde(default)]
    pub check: Option<CheckSpec>,
    #[serde(default)]
    pub effects: Option<Vec<Effect>>,
    #[serde(default)]
    pub on_success: Option<Vec<Effect>>,
    #[serde(default)]
    pub on_fail: Option<Vec<Effect>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSpec {
    #[serde(default)]
    pub skill: Option<String>,
    #[serde(default)]
    pub ability: Option<AbilityName>,
    pub dc: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneTrigger {
    #[serde(default)]
    pub id: Option<String>,
    pub on: TriggerWhen,
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub zones: Vec<String>,
    #[serde(default)]
    pub side: Option<Side>,
    #[serde(default)]
    pub affects: Option<TriggerAffects>,
    #[serde(default)]
    pub once: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub when: Option<Predicate>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerAffects {
    #[default]
    Entering,
    ZoneOccupants,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    #[serde(default)]
    pub cell: Option<i32>,
    #[serde(default)]
    pub legend: BTreeMap<String, GridCell>,
    #[serde(default)]
    pub rows: Vec<String>,
    /// Zone id -> the legend character representing that zone.
    #[serde(default)]
    pub zones: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridCell {
    #[serde(default)]
    pub wall: bool,
    #[serde(default)]
    pub difficult: bool,
    #[serde(default)]
    pub cover: Cover,
    #[serde(default)]
    pub elevation: Option<i32>,
    #[serde(default)]
    pub zone: Option<String>,
}

// ---------------------------------------------------------------------------
// Encounters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Encounter {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub scene: String,
    #[serde(default)]
    pub party_zone: Option<String>,
    pub members: Vec<EncounterMember>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterMember {
    pub creature: String,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub pos: Option<[i32; 2]>,
    #[serde(default)]
    pub side: Option<Side>,
    #[serde(default)]
    pub ai: Option<AiAssignment>,
    #[serde(default)]
    pub dormant: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub todo: Vec<String>,
}

impl EncounterMember {
    pub fn count(&self) -> u32 {
        self.count.unwrap_or(1)
    }
}
