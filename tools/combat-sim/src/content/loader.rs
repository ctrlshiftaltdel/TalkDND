//! Content discovery, parsing and registry.
//!
//! Sources are scanned recursively in a deterministic order. Later sources
//! override earlier ones by `id` within each kind; rules merge field-wise.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::pack::{ContentPack, PackMeta};
use super::schema::{
    Ability, AbilityEntry, AiTemplate, CampaignManifest, ConditionDef, ContentFile, Creature,
    DomainEntity, Encounter, ExprValue, MasteryDef, Rules, Scene,
};

/// Top-level keys that make a map a content container.
pub const CONTAINER_KEYS: &[&str] = &[
    "conditions",
    "masteries",
    "abilities",
    "creatures",
    "ai_templates",
    "scenes",
    "encounters",
    "items",
    "classes",
    "subclasses",
    "species",
    "backgrounds",
    "feats",
    "rule_sections",
    "tables",
    "rules",
];

/// The kinds of content item the registry stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKind {
    Condition,
    Mastery,
    Ability,
    Creature,
    AiTemplate,
    Scene,
    Encounter,
    Item,
    Class,
    Subclass,
    Species,
    Background,
    Feat,
    RuleSection,
    Table,
}

impl ItemKind {
    pub const ALL: [ItemKind; 15] = [
        ItemKind::Condition,
        ItemKind::Mastery,
        ItemKind::Ability,
        ItemKind::Creature,
        ItemKind::AiTemplate,
        ItemKind::Scene,
        ItemKind::Encounter,
        ItemKind::Item,
        ItemKind::Class,
        ItemKind::Subclass,
        ItemKind::Species,
        ItemKind::Background,
        ItemKind::Feat,
        ItemKind::RuleSection,
        ItemKind::Table,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Condition => "condition",
            Self::Mastery => "mastery",
            Self::Ability => "ability",
            Self::Creature => "creature",
            Self::AiTemplate => "ai-template",
            Self::Scene => "scene",
            Self::Encounter => "encounter",
            Self::Item => "item",
            Self::Class => "class",
            Self::Subclass => "subclass",
            Self::Species => "species",
            Self::Background => "background",
            Self::Feat => "feat",
            Self::RuleSection => "rule-section",
            Self::Table => "table",
        }
    }

    /// Accepts the singular, plural and underscore spellings used by the CLI.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "condition" | "conditions" => Some(Self::Condition),
            "mastery" | "masteries" => Some(Self::Mastery),
            "ability" | "abilities" => Some(Self::Ability),
            "creature" | "creatures" => Some(Self::Creature),
            "ai-template" | "ai-templates" | "template" | "templates" => Some(Self::AiTemplate),
            "scene" | "scenes" => Some(Self::Scene),
            "encounter" | "encounters" => Some(Self::Encounter),
            "item" | "items" => Some(Self::Item),
            "class" | "classes" => Some(Self::Class),
            "subclass" | "subclasses" => Some(Self::Subclass),
            "species" | "race" | "races" => Some(Self::Species),
            "background" | "backgrounds" => Some(Self::Background),
            "feat" | "feats" => Some(Self::Feat),
            "rule-section" | "rule-sections" | "rule" | "rules-section" => Some(Self::RuleSection),
            "table" | "tables" => Some(Self::Table),
            _ => None,
        }
    }
}

/// Where an item came from. `line` is the opening fence line for `.md` blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentSource {
    pub path: PathBuf,
    pub line: Option<usize>,
}

impl ContentSource {
    pub fn new(path: impl Into<PathBuf>, line: Option<usize>) -> Self {
        Self {
            path: path.into(),
            line,
        }
    }
}

impl fmt::Display for ContentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}", self.path.display()),
            None => write!(f, "{}", self.path.display()),
        }
    }
}

/// An item plus the file it was read from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sourced<T> {
    pub item: T,
    pub source: ContentSource,
}

/// A registry keyed by id for one kind of content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemStore<T> {
    entries: BTreeMap<String, Sourced<T>>,
}

impl<T> Default for ItemStore<T> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

impl<T> ItemStore<T> {
    pub fn get(&self, id: &str) -> Option<&T> {
        self.entries.get(id).map(|sourced| &sourced.item)
    }

    pub fn get_sourced(&self, id: &str) -> Option<&Sourced<T>> {
        self.entries.get(id)
    }

    pub fn source(&self, id: &str) -> Option<&ContentSource> {
        self.entries.get(id).map(|sourced| &sourced.source)
    }

    pub fn insert(&mut self, id: String, item: T, source: ContentSource) {
        self.entries.insert(id, Sourced { item, source });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &T)> {
        self.entries
            .iter()
            .map(|(id, sourced)| (id.as_str(), &sourced.item))
    }

    pub fn iter_sourced(&self) -> impl Iterator<Item = (&str, &Sourced<T>)> {
        self.entries
            .iter()
            .map(|(id, sourced)| (id.as_str(), sourced))
    }
}

/// All content loaded for one campaign (or one set of explicit sources).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContentRegistry {
    #[serde(default)]
    pub conditions: ItemStore<ConditionDef>,
    #[serde(default)]
    pub masteries: ItemStore<MasteryDef>,
    #[serde(default)]
    pub abilities: ItemStore<Ability>,
    #[serde(default)]
    pub creatures: ItemStore<Creature>,
    #[serde(default)]
    pub ai_templates: ItemStore<AiTemplate>,
    #[serde(default)]
    pub scenes: ItemStore<Scene>,
    #[serde(default)]
    pub encounters: ItemStore<Encounter>,
    #[serde(default)]
    pub items: ItemStore<DomainEntity>,
    #[serde(default)]
    pub classes: ItemStore<DomainEntity>,
    #[serde(default)]
    pub subclasses: ItemStore<DomainEntity>,
    #[serde(default)]
    pub species: ItemStore<DomainEntity>,
    #[serde(default)]
    pub backgrounds: ItemStore<DomainEntity>,
    #[serde(default)]
    pub feats: ItemStore<DomainEntity>,
    #[serde(default)]
    pub rule_sections: ItemStore<DomainEntity>,
    #[serde(default)]
    pub tables: ItemStore<DomainEntity>,
    /// Metadata for content packs loaded from this registry. Old state files
    /// without this field load with an empty map.
    #[serde(default)]
    pub packs: BTreeMap<String, PackMeta>,
    #[serde(default)]
    pub rules: Rules,
    /// Active rules edition (`2014` or `2024`).
    pub ruleset: String,
    #[serde(default)]
    pub rules_source: Option<ContentSource>,
    #[serde(default)]
    pub manifest_path: Option<PathBuf>,
    #[serde(default)]
    pub manifest_name: Option<String>,
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
}

/// A resolved reference to a shared ability.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAbility {
    pub ability: Ability,
    pub params: BTreeMap<String, ExprValue>,
}

/// A type-erased item reference, used by `content show`.
#[derive(Debug, Clone, Copy)]
pub enum AnyItem<'a> {
    Condition(&'a ConditionDef),
    Mastery(&'a MasteryDef),
    Ability(&'a Ability),
    Creature(&'a Creature),
    AiTemplate(&'a AiTemplate),
    Scene(&'a Scene),
    Encounter(&'a Encounter),
    Item(&'a DomainEntity),
    Class(&'a DomainEntity),
    Subclass(&'a DomainEntity),
    Species(&'a DomainEntity),
    Background(&'a DomainEntity),
    Feat(&'a DomainEntity),
    RuleSection(&'a DomainEntity),
    Table(&'a DomainEntity),
}

impl AnyItem<'_> {
    pub fn kind(&self) -> ItemKind {
        match self {
            Self::Condition(_) => ItemKind::Condition,
            Self::Mastery(_) => ItemKind::Mastery,
            Self::Ability(_) => ItemKind::Ability,
            Self::Creature(_) => ItemKind::Creature,
            Self::AiTemplate(_) => ItemKind::AiTemplate,
            Self::Scene(_) => ItemKind::Scene,
            Self::Encounter(_) => ItemKind::Encounter,
            Self::Item(_) => ItemKind::Item,
            Self::Class(_) => ItemKind::Class,
            Self::Subclass(_) => ItemKind::Subclass,
            Self::Species(_) => ItemKind::Species,
            Self::Background(_) => ItemKind::Background,
            Self::Feat(_) => ItemKind::Feat,
            Self::RuleSection(_) => ItemKind::RuleSection,
            Self::Table(_) => ItemKind::Table,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Condition(item) => &item.id,
            Self::Mastery(item) => &item.id,
            Self::Ability(item) => &item.id,
            Self::Creature(item) => &item.id,
            Self::AiTemplate(item) => &item.id,
            Self::Scene(item) => &item.id,
            Self::Encounter(item) => &item.id,
            Self::Item(item) => &item.id,
            Self::Class(item) => &item.id,
            Self::Subclass(item) => &item.id,
            Self::Species(item) => &item.id,
            Self::Background(item) => &item.id,
            Self::Feat(item) => &item.id,
            Self::RuleSection(item) => &item.id,
            Self::Table(item) => &item.id,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Condition(item) => serde_json::to_value(item),
            Self::Mastery(item) => serde_json::to_value(item),
            Self::Ability(item) => serde_json::to_value(item),
            Self::Creature(item) => serde_json::to_value(item),
            Self::AiTemplate(item) => serde_json::to_value(item),
            Self::Scene(item) => serde_json::to_value(item),
            Self::Encounter(item) => serde_json::to_value(item),
            Self::Item(item) => serde_json::to_value(item),
            Self::Class(item) => serde_json::to_value(item),
            Self::Subclass(item) => serde_json::to_value(item),
            Self::Species(item) => serde_json::to_value(item),
            Self::Background(item) => serde_json::to_value(item),
            Self::Feat(item) => serde_json::to_value(item),
            Self::RuleSection(item) => serde_json::to_value(item),
            Self::Table(item) => serde_json::to_value(item),
        }
        .expect("content schema must serialize")
    }
}

impl ContentRegistry {
    pub fn find_any(&self, id: &str) -> Option<AnyItem<'_>> {
        if let Some(item) = self.conditions.get(id) {
            return Some(AnyItem::Condition(item));
        }
        if let Some(item) = self.masteries.get(id) {
            return Some(AnyItem::Mastery(item));
        }
        if let Some(item) = self.abilities.get(id) {
            return Some(AnyItem::Ability(item));
        }
        if let Some(item) = self.creatures.get(id) {
            return Some(AnyItem::Creature(item));
        }
        if let Some(item) = self.ai_templates.get(id) {
            return Some(AnyItem::AiTemplate(item));
        }
        if let Some(item) = self.scenes.get(id) {
            return Some(AnyItem::Scene(item));
        }
        if let Some(item) = self.encounters.get(id) {
            return Some(AnyItem::Encounter(item));
        }
        for (kind, store) in [
            (ItemKind::Item, &self.items),
            (ItemKind::Class, &self.classes),
            (ItemKind::Subclass, &self.subclasses),
            (ItemKind::Species, &self.species),
            (ItemKind::Background, &self.backgrounds),
            (ItemKind::Feat, &self.feats),
            (ItemKind::RuleSection, &self.rule_sections),
            (ItemKind::Table, &self.tables),
        ] {
            if let Some(item) = store.get(id) {
                return Some(match kind {
                    ItemKind::Item => AnyItem::Item(item),
                    ItemKind::Class => AnyItem::Class(item),
                    ItemKind::Subclass => AnyItem::Subclass(item),
                    ItemKind::Species => AnyItem::Species(item),
                    ItemKind::Background => AnyItem::Background(item),
                    ItemKind::Feat => AnyItem::Feat(item),
                    ItemKind::RuleSection => AnyItem::RuleSection(item),
                    ItemKind::Table => AnyItem::Table(item),
                    _ => unreachable!(),
                });
            }
        }
        None
    }

    pub fn source_of(&self, kind: ItemKind, id: &str) -> Option<&ContentSource> {
        match kind {
            ItemKind::Condition => self.conditions.source(id),
            ItemKind::Mastery => self.masteries.source(id),
            ItemKind::Ability => self.abilities.source(id),
            ItemKind::Creature => self.creatures.source(id),
            ItemKind::AiTemplate => self.ai_templates.source(id),
            ItemKind::Scene => self.scenes.source(id),
            ItemKind::Encounter => self.encounters.source(id),
            ItemKind::Item => self.items.source(id),
            ItemKind::Class => self.classes.source(id),
            ItemKind::Subclass => self.subclasses.source(id),
            ItemKind::Species => self.species.source(id),
            ItemKind::Background => self.backgrounds.source(id),
            ItemKind::Feat => self.feats.source(id),
            ItemKind::RuleSection => self.rule_sections.source(id),
            ItemKind::Table => self.tables.source(id),
        }
    }

    /// All `(id, source)` pairs for one kind, sorted by id.
    pub fn entries(&self, kind: ItemKind) -> Vec<(&str, &ContentSource)> {
        match kind {
            ItemKind::Condition => self
                .conditions
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Mastery => self
                .masteries
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Ability => self
                .abilities
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Creature => self
                .creatures
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::AiTemplate => self
                .ai_templates
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Scene => self
                .scenes
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Encounter => self
                .encounters
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Item => self
                .items
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Class => self
                .classes
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Subclass => self
                .subclasses
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Species => self
                .species
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Background => self
                .backgrounds
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Feat => self
                .feats
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::RuleSection => self
                .rule_sections
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
            ItemKind::Table => self
                .tables
                .iter_sourced()
                .map(|(id, sourced)| (id, &sourced.source))
                .collect(),
        }
    }

    /// Resolves `base:` chains. Scalars, objects and lists replace when the
    /// child supplies them; `abilities`, `saves`, `skills` and `resources`
    /// merge child-over-base. Missing bases and cycles are errors.
    pub fn resolve_creature(&self, id: &str) -> Result<Creature> {
        let mut path = Vec::new();
        let mut seen = BTreeSet::new();
        let mut current = id.to_string();
        loop {
            if !seen.insert(current.clone()) {
                path.push(current);
                bail!("creature base cycle: {}", path.join(" -> "));
            }
            path.push(current.clone());
            let creature = self.creatures.get(&current).ok_or_else(|| {
                if path.len() == 1 {
                    anyhow!("unknown creature `{id}`")
                } else {
                    anyhow!(
                        "creature `{}` has unknown base `{current}`",
                        path[path.len() - 2]
                    )
                }
            })?;
            match creature.base.as_deref() {
                Some(base) if !base.is_empty() => current = base.to_string(),
                _ => break,
            }
        }

        let chain: Vec<&Creature> = path
            .iter()
            .map(|creature_id| {
                self.creatures
                    .get(creature_id)
                    .expect("base chain was checked above")
            })
            .collect();
        let mut resolved = (*chain.last().expect("base chain is never empty")).clone();
        for child in chain.iter().rev().skip(1) {
            resolved.merge_from((*child).clone());
        }
        resolved.base = None;
        Ok(resolved)
    }

    /// Resolves an `actions`/`features` entry, applying `params`, `cost` and
    /// `name` overrides for references.
    pub fn resolve_ability(&self, entry: &AbilityEntry) -> Result<ResolvedAbility> {
        match entry {
            AbilityEntry::Inline(ability) => Ok(ResolvedAbility {
                ability: (**ability).clone(),
                params: BTreeMap::new(),
            }),
            AbilityEntry::Ref(reference) => {
                let mut ability = self
                    .abilities
                    .get(&reference.reference)
                    .cloned()
                    .ok_or_else(|| anyhow!("unknown ability `{}`", reference.reference))?;
                if let Some(cost) = &reference.cost {
                    ability.cost = Some(cost.clone());
                }
                if let Some(name) = &reference.name {
                    ability.name = Some(name.clone());
                }
                Ok(ResolvedAbility {
                    ability,
                    params: reference.params.clone(),
                })
            }
        }
    }

    fn insert_item(&mut self, item: ContentItem, source: &ContentSource) {
        match item {
            ContentItem::Condition(item) => {
                self.conditions
                    .insert(item.id.clone(), item, source.clone())
            }
            ContentItem::Mastery(item) => {
                self.masteries.insert(item.id.clone(), item, source.clone())
            }
            ContentItem::Ability(item) => {
                self.abilities.insert(item.id.clone(), item, source.clone())
            }
            ContentItem::Creature(item) => {
                self.creatures.insert(item.id.clone(), item, source.clone())
            }
            ContentItem::AiTemplate(item) => {
                self.ai_templates
                    .insert(item.id.clone(), item, source.clone())
            }
            ContentItem::Scene(item) => self.scenes.insert(item.id.clone(), item, source.clone()),
            ContentItem::Encounter(item) => {
                self.encounters
                    .insert(item.id.clone(), item, source.clone())
            }
            ContentItem::Item(item) => self.items.insert(item.id.clone(), item, source.clone()),
            ContentItem::Class(item) => self.classes.insert(item.id.clone(), item, source.clone()),
            ContentItem::Subclass(item) => {
                self.subclasses
                    .insert(item.id.clone(), item, source.clone())
            }
            ContentItem::Species(item) => {
                self.species.insert(item.id.clone(), item, source.clone())
            }
            ContentItem::Background(item) => {
                self.backgrounds
                    .insert(item.id.clone(), item, source.clone())
            }
            ContentItem::Feat(item) => self.feats.insert(item.id.clone(), item, source.clone()),
            ContentItem::RuleSection(item) => {
                self.rule_sections
                    .insert(item.id.clone(), item, source.clone())
            }
            ContentItem::Table(item) => self.tables.insert(item.id.clone(), item, source.clone()),
        }
    }

    fn apply_rules(&mut self, rules: Rules, source: &ContentSource) {
        self.rules.merge_from(rules);
        self.rules_source = Some(source.clone());
    }
}

/// A single parsed item, before it is stored in the registry.
#[allow(clippy::large_enum_variant)]
enum ContentItem {
    Condition(ConditionDef),
    Mastery(MasteryDef),
    Ability(Ability),
    Creature(Creature),
    AiTemplate(AiTemplate),
    Scene(Scene),
    Encounter(Encounter),
    Item(DomainEntity),
    Class(DomainEntity),
    Subclass(DomainEntity),
    Species(DomainEntity),
    Background(DomainEntity),
    Feat(DomainEntity),
    RuleSection(DomainEntity),
    Table(DomainEntity),
}

/// Explicit source flags. A campaign manifest, if supplied (or found in
/// `COMBAT_SIM_CAMPAIGN`), is loaded first and explicit paths are appended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentSources {
    pub libraries: Vec<PathBuf>,
    pub adventures: Vec<PathBuf>,
    pub characters: Vec<PathBuf>,
    pub campaign: Option<PathBuf>,
}

impl ContentSources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn campaign_path(&self) -> Option<PathBuf> {
        self.campaign
            .clone()
            .or_else(|| {
                std::env::var("COMBAT_SIM_CAMPAIGN")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(PathBuf::from)
            })
            .map(|path| resolve_in_play_root(&path, play_root().as_deref()))
    }

    /// Loads the manifest (if any) plus explicit sources and returns a ready
    /// registry. The edition's SRD libraries load first, then the unlocked
    /// full packs (FULL rulesets), then adventures and characters.
    pub fn load(&self) -> Result<ContentRegistry> {
        let mut registry = ContentRegistry::default();
        let mut libraries = Vec::new();
        let mut adventures = Vec::new();
        let mut characters = Vec::new();

        let mut ruleset = Ruleset::FivePointFiveESrd;
        let mut strict = false;
        if let Some(manifest_path) = self.campaign_path() {
            strict = true;
            let manifest = load_manifest(&manifest_path)?;
            if let Some(value) = &manifest.ruleset {
                ruleset = Ruleset::parse(value)
                    .with_context(|| format!("in campaign manifest {}", manifest_path.display()))?;
            }
            let base = manifest_path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            libraries.extend(
                manifest
                    .library
                    .iter()
                    .map(|path| resolve_relative(base, path.as_path())),
            );
            adventures.extend(
                manifest
                    .adventures
                    .iter()
                    .map(|path| resolve_relative(base, path.as_path())),
            );
            characters.extend(
                manifest
                    .characters
                    .iter()
                    .map(|path| resolve_relative(base, path.as_path())),
            );
            registry.manifest_path = Some(manifest_path.clone());
            registry.manifest_name = Some(manifest.name);
            registry.state_dir = manifest
                .state_dir
                .map(|path| resolve_relative(base, path.as_path()));
        }

        libraries.extend(self.libraries.iter().cloned());
        adventures.extend(self.adventures.iter().cloned());
        characters.extend(self.characters.iter().cloned());

        if libraries.is_empty() && adventures.is_empty() && characters.is_empty() {
            bail!(
                "no content sources: pass --campaign, --lib, --adventure or --chars (or set COMBAT_SIM_CAMPAIGN)"
            );
        }

        registry.ruleset = ruleset.edition().to_string();

        // For a FULL save, split explicit pack files and the layered edition
        // into SRD base, full packs, and hand-written overrides. `*-srd` and
        // non-strict callers keep the original library order and behavior.
        if strict && ruleset.is_full() {
            let mut srd_libraries = Vec::new();
            let mut full_libraries = Vec::new();
            let mut handwritten_libraries: Vec<HandwrittenSource> = Vec::new();
            for path in libraries {
                if super::pack::is_pack_path(&path) {
                    let meta = read_pack_header(&path)?;
                    if meta.id.starts_with("full-")
                        && meta.distribution != super::pack::Distribution::Full
                    {
                        bail!(
                            "full pack `{}` ({}) must declare distribution: full, not {}; one save uses one ruleset",
                            meta.id,
                            path.display(),
                            meta.distribution.as_str()
                        );
                    }
                    match meta.distribution {
                        super::pack::Distribution::Full => {
                            if meta.edition != ruleset.edition() {
                                bail!(
                                    "full pack `{}` ({}) is for edition {} but this save uses {} (edition {}); one save uses one ruleset",
                                    meta.id,
                                    path.display(),
                                    meta.edition,
                                    ruleset.display(),
                                    ruleset.edition()
                                );
                            }
                            full_libraries.push(path);
                        }
                        super::pack::Distribution::Srd => srd_libraries.push(path),
                        super::pack::Distribution::Local => {
                            handwritten_libraries.push(HandwrittenSource {
                                path,
                                exclude: None,
                            })
                        }
                    }
                    continue;
                }
                // A layered library (`common/` + edition folders) is the SRD
                // base; every other directory is hand-written or campaign
                // content. Split the edition folder so only `common/` and
                // `<edition>/srd/` stay in the SRD layer; the edition's other
                // files (monsters/, spells.yaml, ...) keep overriding the
                // full pack.
                if path.is_dir() && path.join("common").is_dir() {
                    let (srd, hand) = full_layers(&path, ruleset.edition());
                    srd_libraries.extend(srd);
                    handwritten_libraries.extend(hand);
                } else {
                    handwritten_libraries.push(HandwrittenSource {
                        path,
                        exclude: None,
                    });
                }
            }

            let srd_libraries: Vec<PathBuf> = srd_libraries
                .iter()
                .flat_map(|path| expand_layers(path, ruleset.edition()))
                .collect();
            for source in srd_libraries.iter().chain(full_libraries.iter()) {
                load_source(source, None, ruleset, strict, &mut registry)?;
            }
            for source in &handwritten_libraries {
                load_source(
                    &source.path,
                    source.exclude.as_deref(),
                    ruleset,
                    strict,
                    &mut registry,
                )?;
            }
            for source in adventures.iter().chain(characters.iter()) {
                load_source(source, None, ruleset, strict, &mut registry)?;
            }
        } else {
            let libraries: Vec<PathBuf> = libraries
                .iter()
                .flat_map(|path| expand_layers(path, ruleset.edition()))
                .collect();
            for source in libraries
                .iter()
                .chain(adventures.iter())
                .chain(characters.iter())
            {
                load_source(source, None, ruleset, strict, &mut registry)?;
            }
        }

        // Fail closed: FULL must actually add full-book content. Packs reached
        // through a directory (not listed individually) are validated too.
        if strict && ruleset.is_full() {
            let matching = registry.packs.values().any(|meta| {
                meta.distribution == super::pack::Distribution::Full
                    && meta.edition == ruleset.edition()
                    && meta.id.starts_with(&format!("full-{}-", ruleset.edition()))
            });
            if !matching {
                return Err(full_pack_required_error(ruleset));
            }
        }

        Ok(registry)
    }
}

/// Rules editions the loader understands. These are the on-disk layer names
/// (`Rules/combat/<edition>/`); the save-visible ruleset values are
/// [`Ruleset`], which also selects whether full-book packs may load.
pub const SUPPORTED_RULESETS: [&str; 2] = ["2014", "2024"];

/// The four player-visible rulesets. `srd` loads only the edition's SRD layer;
/// `full` additionally requires at least one unlocked full pack for that
/// edition. The engine itself still receives `2014` / `2024` in
/// [`ContentRegistry::ruleset`], so spatial and rules defaults do not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Ruleset {
    #[serde(rename = "5e-srd")]
    FiveESrd,
    #[serde(rename = "5e-full")]
    FiveEFull,
    #[serde(rename = "5.5e-srd")]
    FivePointFiveESrd,
    #[serde(rename = "5.5e-full")]
    FivePointFiveEFull,
}

impl Ruleset {
    /// Display order used in errors and help text.
    pub const ALL: [Ruleset; 4] = [
        Ruleset::FivePointFiveEFull,
        Ruleset::FiveEFull,
        Ruleset::FivePointFiveESrd,
        Ruleset::FiveESrd,
    ];

    pub const VALUES: [&'static str; 4] = ["5.5e-full", "5e-full", "5.5e-srd", "5e-srd"];

    /// Parses a save manifest value. Case and surrounding whitespace are
    /// ignored. The old edition-only values migrate to the SRD ruleset.
    pub fn parse(text: &str) -> Result<Self> {
        let normalized = text.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "5e-srd" => Ok(Self::FiveESrd),
            "5e-full" => Ok(Self::FiveEFull),
            "5.5e-srd" => Ok(Self::FivePointFiveESrd),
            "5.5e-full" => Ok(Self::FivePointFiveEFull),
            "2014" => Ok(Self::FiveESrd),
            "2024" => Ok(Self::FivePointFiveESrd),
            _ => bail!(
                "unsupported ruleset `{}`; expected one of {} (legacy `2014` -> `5e-srd`, `2024` -> `5.5e-srd`)",
                text.trim(),
                Self::ALL
                    .iter()
                    .map(|ruleset| format!(
                        "{} (`{}`)",
                        ruleset.display(),
                        ruleset.value()
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// Canonical save-manifest value.
    pub fn value(self) -> &'static str {
        match self {
            Self::FiveESrd => "5e-srd",
            Self::FiveEFull => "5e-full",
            Self::FivePointFiveESrd => "5.5e-srd",
            Self::FivePointFiveEFull => "5.5e-full",
        }
    }

    /// Player-facing name used by the digest, prompts and errors.
    pub fn display(self) -> &'static str {
        match self {
            Self::FiveESrd => "5e SRD",
            Self::FiveEFull => "5e FULL",
            Self::FivePointFiveESrd => "5.5e SRD",
            Self::FivePointFiveEFull => "5.5e FULL",
        }
    }

    /// Engine edition / on-disk layer: `2014` or `2024`.
    pub fn edition(self) -> &'static str {
        match self {
            Self::FiveESrd | Self::FiveEFull => "2014",
            Self::FivePointFiveESrd | Self::FivePointFiveEFull => "2024",
        }
    }

    pub fn is_full(self) -> bool {
        matches!(self, Self::FiveEFull | Self::FivePointFiveEFull)
    }

    pub fn is_srd(self) -> bool {
        !self.is_full()
    }
}

impl fmt::Display for Ruleset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display())
    }
}

/// A source that loads after the full packs. `exclude` drops the SRD subfolder
/// from a full-mode layered edition layer.
#[derive(Debug, Clone)]
struct HandwrittenSource {
    path: PathBuf,
    exclude: Option<PathBuf>,
}

/// Split a layered library for a FULL save: `common/` and `<edition>/srd/`
/// stay in the SRD base; the rest of `<edition>/` is hand-written/overlay
/// content and loads after the full packs. `*-srd` keeps using
/// [`expand_layers`] unchanged.
fn full_layers(path: &Path, edition: &str) -> (Vec<PathBuf>, Vec<HandwrittenSource>) {
    let mut srd = Vec::new();
    let common = path.join("common");
    if common.is_dir() {
        srd.push(common);
    }
    let mut hand = Vec::new();
    let edition_dir = path.join(edition);
    if edition_dir.is_dir() {
        let srd_dir = edition_dir.join("srd");
        if srd_dir.is_dir() {
            srd.push(srd_dir.clone());
        }
        hand.push(HandwrittenSource {
            path: edition_dir,
            exclude: Some(srd_dir),
        });
    }
    (srd, hand)
}

fn load_source(
    source: &Path,
    exclude: Option<&Path>,
    ruleset: Ruleset,
    strict: bool,
    registry: &mut ContentRegistry,
) -> Result<()> {
    let mut files = Vec::new();
    collect_files(source, &mut files, ruleset.edition())?;
    if let Some(exclude) = exclude {
        files.retain(|file| !file.starts_with(exclude));
    }
    // Packs are the base layer; ordinary content overrides them by entity id.
    // `sort_by_key` is stable, so files keep their path order inside each group.
    files.sort_by_key(|file| !super::pack::is_pack_path(file));
    for file in files {
        load_file(&file, ruleset, strict, registry)?;
    }
    Ok(())
}

fn full_pack_required_error(ruleset: Ruleset) -> anyhow::Error {
    let edition = ruleset.edition();
    let alternative = if edition == "2014" {
        "5e SRD"
    } else {
        "5.5e SRD"
    };
    anyhow!(
        "{} requires at least one unlocked {edition} full pack (Player's Handbook, Dungeon Master's Guide or Monster Manual); none was loaded. \
         Put the matching PDF in Rules/{edition}/<book>/ and run `talkdnd library scan`, \
         put the built pack at Rules/packs/full-{edition}-<book>.pack.json, then run `talkdnd start`. \
         Or choose {alternative}.",
        ruleset.display()
    )
}

/// A layered library (`<dir>/common/` present) expands to `common/` then
/// `<ruleset>/`; other edition folders are ignored. Plain dirs load as-is.
fn expand_layers(path: &Path, ruleset: &str) -> Vec<PathBuf> {
    let common = path.join("common");
    if path.is_dir() && common.is_dir() {
        let mut out = vec![common];
        let layer = path.join(ruleset);
        if layer.is_dir() {
            out.push(layer);
        }
        out
    } else {
        vec![path.to_path_buf()]
    }
}

fn resolve_relative(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn load_manifest(path: &Path) -> Result<CampaignManifest> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read campaign manifest {}", path.display()))?;
    let format = if extension_of(path).as_deref() == Some("json") {
        Format::Json
    } else {
        Format::Yaml
    };
    let value = parse_value(&text, format)
        .with_context(|| format!("failed to parse campaign manifest {}", path.display()))?;
    serde_json::from_value(value)
        .with_context(|| format!("failed to parse campaign manifest {}", path.display()))
}

/// Reads only the `pack` metadata of an explicitly listed pack file, so the
/// loader can decide whether it is a full pack before loading its content.
fn read_pack_header(path: &Path) -> Result<PackMeta> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read content pack {}", path.display()))?;
    let format = if extension_of(path).as_deref() == Some("json") {
        Format::Json
    } else {
        Format::Yaml
    };
    let value = parse_value(&text, format)
        .with_context(|| format!("failed to parse content pack {}", path.display()))?;
    let pack = value
        .get("pack")
        .ok_or_else(|| anyhow!("{} has no `pack` metadata", path.display()))?;
    serde_json::from_value(pack.clone())
        .with_context(|| format!("failed to read content pack metadata {}", path.display()))
}

fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
}

fn supported_content_file(path: &Path) -> bool {
    matches!(
        extension_of(path).as_deref(),
        Some("yaml" | "yml" | "json" | "md")
    )
}

/// Deterministic recursive scan: entries are sorted by name. Inside a
/// directory, hidden entries (`.md/` conversions, `.git`) are skipped and files
/// that are not content (`.py`, `.jsonl`, images) are ignored with a warning.
/// A file named explicitly must be a content file. Sub-folders named exactly
/// `2014` / `2024` load only when they match the campaign ruleset.
fn collect_files(path: &Path, out: &mut Vec<PathBuf>, ruleset: &str) -> Result<()> {
    collect_inner(path, out, true, ruleset)
}

/// pdf2md output folder (`index/`, or a legacy `.md/<Book>/`).
fn is_index_dir(path: &Path) -> bool {
    path.join("manifest.json").is_file() || path.join("index.json").is_file()
}

/// A one-folder-per-book library entry: it holds the book PDF or its
/// converted `index/`.
fn is_book_folder(path: &Path) -> bool {
    if is_index_dir(&path.join("index")) {
        return true;
    }
    std::fs::read_dir(path).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("pdf"))
        })
    })
}

fn collect_inner(path: &Path, out: &mut Vec<PathBuf>, explicit: bool, ruleset: &str) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read content source {}", path.display()))?;
    if metadata.is_dir() {
        // An adventure folder (`Adventures/<book>`) keeps its combat data in
        // `combat/`; the rest is the book and its conversions.
        let combat = path.join("combat");
        let book = is_book_folder(path);
        if (explicit || book) && combat.is_dir() {
            return collect_inner(&combat, out, false, ruleset);
        }
        // A book folder (`Adventures/<slug>/` with the PDF and its converted
        // `index/`) without combat data has nothing to load.
        if book {
            return Ok(());
        }
        // A nested layered library (`common/` + `2014/` …) is loaded only when
        // listed under `library:`, which picks the save's ruleset layer.
        if !explicit && path.join("common").is_dir() {
            return Ok(());
        }
        let mut entries = std::fs::read_dir(path)
            .with_context(|| format!("failed to read content directory {}", path.display()))?
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("failed to read content directory {}", path.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            // pdf2md output (`<book>/index/`) is book text, never content.
            if name == "index" && is_index_dir(&entry.path()) {
                continue;
            }
            // Edition folders (`heists/2014/`) follow the save's ruleset.
            if SUPPORTED_RULESETS.contains(&name.as_str())
                && name != ruleset
                && entry.path().is_dir()
            {
                continue;
            }
            collect_inner(&entry.path(), out, false, ruleset)?;
        }
    } else if metadata.is_file() {
        if supported_content_file(path) {
            out.push(path.to_path_buf());
        } else if explicit {
            bail!(
                "unsupported content file {} (expected .yaml, .yml, .json or .md)",
                path.display()
            );
        } else {
            eprintln!(
                "warning: skipping non-content file {} (expected .yaml, .yml, .json or .md)",
                path.display()
            );
        }
    } else if explicit {
        bail!(
            "content source {} is not a file or directory",
            path.display()
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Yaml,
    Json,
}

struct FencedBlock {
    format: Format,
    content: String,
    line: usize,
}

fn load_file(
    path: &Path,
    ruleset: Ruleset,
    strict: bool,
    registry: &mut ContentRegistry,
) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read content file {}", path.display()))?;
    match extension_of(path).as_deref() {
        Some("md") => {
            let blocks = extract_combat_blocks(&text)
                .with_context(|| format!("failed to read fenced blocks in {}", path.display()))?;
            for block in blocks {
                let source = ContentSource::new(path, Some(block.line));
                let value = parse_value(&block.content, block.format)
                    .map_err(|error| anyhow!("{source}: {error}"))?;
                apply_document(&value, &source, ruleset, strict, registry)
                    .map_err(|error| anyhow!("{source}: {error}"))?;
            }
        }
        Some("json") => {
            let source = ContentSource::new(path, None);
            let value = serde_json::from_str::<serde_json::Value>(&text)
                .map_err(|error| anyhow!("{}: invalid JSON: {error}", source))?;
            apply_document(&value, &source, ruleset, strict, registry)
                .map_err(|error| anyhow!("{source}: {error}"))?;
        }
        Some("yaml" | "yml") => {
            let source = ContentSource::new(path, None);
            let value =
                parse_value(&text, Format::Yaml).map_err(|error| anyhow!("{source}: {error}"))?;
            apply_document(&value, &source, ruleset, strict, registry)
                .map_err(|error| anyhow!("{source}: {error}"))?;
        }
        _ => bail!("unsupported content file {}", path.display()),
    }
    Ok(())
}

fn parse_value(text: &str, format: Format) -> Result<serde_json::Value> {
    match format {
        Format::Json => serde_json::from_str(text).context("invalid JSON"),
        Format::Yaml => {
            let value: serde_norway::Value =
                serde_norway::from_str(text).context("invalid YAML")?;
            serde_json::to_value(value).context("YAML document cannot be represented as JSON")
        }
    }
}

/// Extract only fenced blocks whose info string starts with `combat yaml` or
/// `combat json`; all other fences and prose are ignored.
fn extract_combat_blocks(text: &str) -> Result<Vec<FencedBlock>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        let Some(info) = trimmed.strip_prefix("```") else {
            index += 1;
            continue;
        };
        let info = info.trim();
        let fence_line = index + 1;
        let mut end = index + 1;
        while end < lines.len() && !lines[end].trim_start().starts_with("```") {
            end += 1;
        }
        if end >= lines.len() {
            bail!("unterminated fenced block starting at line {fence_line}");
        }
        let format = if info.starts_with("combat yaml") {
            Some(Format::Yaml)
        } else if info.starts_with("combat json") {
            Some(Format::Json)
        } else {
            None
        };
        if let Some(format) = format {
            blocks.push(FencedBlock {
                format,
                content: lines[index + 1..end].join("\n"),
                line: fence_line,
            });
        }
        index = end + 1;
    }
    Ok(blocks)
}

fn apply_document(
    value: &serde_json::Value,
    source: &ContentSource,
    ruleset: Ruleset,
    strict: bool,
    registry: &mut ContentRegistry,
) -> Result<()> {
    if super::pack::looks_like_pack(value) {
        let pack = ContentPack::from_value(value).map_err(|error| anyhow!("{source}: {error}"))?;
        apply_pack(pack, source, ruleset, strict, registry)?;
        return Ok(());
    }
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("top-level content must be a map"))?;
    let has_container_key = CONTAINER_KEYS.iter().any(|key| object.contains_key(*key));
    let looks_like_item = object.contains_key("id");
    if has_container_key && !looks_like_item {
        let file: ContentFile = serde_json::from_value(value.clone())
            .map_err(|error| anyhow!("invalid content container: {error}"))?;
        apply_container(file, source, registry);
        return Ok(());
    }
    let item = parse_single_item(value)?;
    registry.insert_item(item, source);
    Ok(())
}

fn apply_pack(
    pack: ContentPack,
    source: &ContentSource,
    ruleset: Ruleset,
    strict: bool,
    registry: &mut ContentRegistry,
) -> Result<()> {
    let meta = pack.pack.clone();
    if strict && meta.distribution == super::pack::Distribution::Full {
        // SRD rulesets never load full-book content, even if an old manifest
        // still lists a pack path.
        if ruleset.is_srd() {
            return Ok(());
        }
        if meta.edition != ruleset.edition() {
            bail!(
                "{source}: full pack `{}` is for edition {} but this save uses {} (edition {}); one save uses one ruleset",
                meta.id,
                meta.edition,
                ruleset.display(),
                ruleset.edition()
            );
        }
        let expected =
            ["phb", "dmg", "mm"].map(|book| format!("full-{}-{book}", ruleset.edition()));
        if !expected.contains(&meta.id) {
            bail!(
                "{source}: full pack `{}` is not a valid {} book pack (expected one of {})",
                meta.id,
                ruleset.display(),
                expected.join(", ")
            );
        }
    }
    if registry.packs.contains_key(&meta.id) {
        bail!(
            "{source}: duplicate content pack id `{}` (already loaded)",
            meta.id
        );
    }
    registry.packs.insert(meta.id.clone(), meta);
    apply_container(pack.content, source, registry);
    Ok(())
}

fn apply_container(file: ContentFile, source: &ContentSource, registry: &mut ContentRegistry) {
    for item in file.conditions.unwrap_or_default() {
        registry.insert_item(ContentItem::Condition(item), source);
    }
    for item in file.masteries.unwrap_or_default() {
        registry.insert_item(ContentItem::Mastery(item), source);
    }
    for item in file.abilities.unwrap_or_default() {
        registry.insert_item(ContentItem::Ability(item), source);
    }
    for item in file.creatures.unwrap_or_default() {
        registry.insert_item(ContentItem::Creature(item), source);
    }
    for item in file.ai_templates.unwrap_or_default() {
        registry.insert_item(ContentItem::AiTemplate(item), source);
    }
    for item in file.scenes.unwrap_or_default() {
        registry.insert_item(ContentItem::Scene(item), source);
    }
    for item in file.encounters.unwrap_or_default() {
        registry.insert_item(ContentItem::Encounter(item), source);
    }
    for item in file.items.unwrap_or_default() {
        registry.insert_item(ContentItem::Item(item), source);
    }
    for item in file.classes.unwrap_or_default() {
        registry.insert_item(ContentItem::Class(item), source);
    }
    for item in file.subclasses.unwrap_or_default() {
        registry.insert_item(ContentItem::Subclass(item), source);
    }
    for item in file.species.unwrap_or_default() {
        registry.insert_item(ContentItem::Species(item), source);
    }
    for item in file.backgrounds.unwrap_or_default() {
        registry.insert_item(ContentItem::Background(item), source);
    }
    for item in file.feats.unwrap_or_default() {
        registry.insert_item(ContentItem::Feat(item), source);
    }
    for item in file.rule_sections.unwrap_or_default() {
        registry.insert_item(ContentItem::RuleSection(item), source);
    }
    for item in file.tables.unwrap_or_default() {
        registry.insert_item(ContentItem::Table(item), source);
    }
    if let Some(rules) = file.rules {
        registry.apply_rules(rules, source);
    }
}

fn parse_single_item(value: &serde_json::Value) -> Result<ContentItem> {
    let mut failures = Vec::new();
    macro_rules! attempt {
        ($kind:literal, $ty:ty, $variant:path) => {
            match serde_json::from_value::<$ty>(value.clone()) {
                Ok(item) => return Ok($variant(item)),
                Err(error) => failures.push(format!("{}: {}", $kind, error)),
            }
        };
    }
    attempt!("creature", Creature, ContentItem::Creature);
    attempt!("ability", Ability, ContentItem::Ability);
    attempt!("condition", ConditionDef, ContentItem::Condition);
    attempt!("mastery", MasteryDef, ContentItem::Mastery);
    attempt!("ai-template", AiTemplate, ContentItem::AiTemplate);
    attempt!("scene", Scene, ContentItem::Scene);
    attempt!("encounter", Encounter, ContentItem::Encounter);

    Err(anyhow!(
        "map is not a content container (expected one of: {}) and does not match any single item kind:\n  {}",
        CONTAINER_KEYS.join(", "),
        failures.join("\n  ")
    ))
}

/// Serializes one item plus its kind and source for `content show`.
#[derive(Serialize)]
pub struct ShownItem<'a> {
    pub kind: &'a str,
    pub source: String,
    pub item: serde_json::Value,
}

impl<'a> ShownItem<'a> {
    pub fn new(registry: &ContentRegistry, item: AnyItem<'a>) -> Self {
        let kind = item.kind();
        let source = registry
            .source_of(kind, item.id())
            .map(ToString::to_string)
            .unwrap_or_else(|| "(unknown)".to_string());
        Self {
            kind: kind.as_str(),
            source,
            item: item.to_json(),
        }
    }
}

/// The play folder this binary belongs to: `TALKDND_ROOT`, else the parent of
/// the executable's `bin/` when it carries the `.talkdnd-root` marker (or the
/// classic `AGENTS.md` + `Saves/`). Folder names and locations do not matter.
pub fn play_root() -> Option<PathBuf> {
    let is_root = |p: &Path| {
        p.join(".talkdnd-root").is_file()
            || (p.join("AGENTS.md").is_file() && p.join("Saves").is_dir())
    };
    if let Some(p) = std::env::var_os("TALKDND_ROOT").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(p);
        if is_root(&p) {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let root = exe.parent()?.parent()?.to_path_buf();
    is_root(&root).then_some(root)
}

/// A relative campaign path that does not exist from the current directory
/// is looked up inside the play folder, so `bin/combat-sim --campaign
/// Saves/<slot>/combat/campaign.yaml` works from any working directory.
pub fn resolve_in_play_root(path: &Path, root: Option<&Path>) -> PathBuf {
    if path.is_absolute() || path.exists() {
        return path.to_path_buf();
    }
    match root {
        Some(root) if root.join(path).exists() => root.join(path),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod play_root_tests {
    use super::*;

    #[test]
    fn relative_campaign_falls_back_to_the_play_root() {
        let root =
            std::env::temp_dir().join(format!("cs-root-{} odd name 冒险", std::process::id()));
        let rel = Path::new("Saves/2030-01-01-01/combat/campaign.yaml");
        std::fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
        std::fs::write(root.join(rel), "name: x\n").unwrap();
        assert_eq!(resolve_in_play_root(rel, Some(&root)), root.join(rel));
        assert_eq!(
            resolve_in_play_root(Path::new("nope.yaml"), Some(&root)),
            PathBuf::from("nope.yaml")
        );
        assert_eq!(resolve_in_play_root(rel, None), rel.to_path_buf());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn book_folders_load_only_their_combat_data() {
        let root = std::env::temp_dir().join(format!("cs-books-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let adv = root.join("Adventures");
        let with = adv.join("with-combat");
        let without = adv.join("no-combat");
        for d in [
            with.join("index"),
            with.join("combat"),
            without.join("index"),
        ] {
            std::fs::create_dir_all(&d).unwrap();
        }
        for b in [&with, &without] {
            std::fs::write(b.join("Book.pdf"), "pdf").unwrap();
            std::fs::write(b.join("index/manifest.json"), "{}").unwrap();
            std::fs::write(b.join("index/index.json"), "{}").unwrap();
            std::fs::write(b.join("index/Book.pages.md"), "# Book\n").unwrap();
        }
        std::fs::write(with.join("combat/goblin.yaml"), "x: 1\n").unwrap();
        let mut out = Vec::new();
        collect_inner(&adv, &mut out, true, "2014").unwrap();
        assert_eq!(out, vec![with.join("combat/goblin.yaml")]);
        let mut out = Vec::new();
        collect_inner(&with, &mut out, true, "2014").unwrap();
        assert_eq!(out, vec![with.join("combat/goblin.yaml")]);
        let mut out = Vec::new();
        collect_inner(&with.join("combat"), &mut out, true, "2014").unwrap();
        assert_eq!(out, vec![with.join("combat/goblin.yaml")]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
