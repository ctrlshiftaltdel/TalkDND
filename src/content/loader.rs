//! Content discovery, parsing and registry.
//!
//! Sources are scanned recursively in a deterministic order. Later sources
//! override earlier ones by `id` within each kind; rules merge field-wise.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::schema::{
    Ability, AbilityEntry, AiTemplate, CampaignManifest, ConditionDef, ContentFile, Creature,
    Encounter, ExprValue, MasteryDef, Rules, Scene,
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
}

impl ItemKind {
    pub const ALL: [ItemKind; 7] = [
        ItemKind::Condition,
        ItemKind::Mastery,
        ItemKind::Ability,
        ItemKind::Creature,
        ItemKind::AiTemplate,
        ItemKind::Scene,
        ItemKind::Encounter,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    pub rules: Rules,
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
        self.encounters.get(id).map(AnyItem::Encounter)
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
        self.campaign.clone().or_else(|| {
            std::env::var("COMBAT_SIM_CAMPAIGN")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        })
    }

    /// Loads the manifest (if any) plus explicit sources and returns a ready
    /// registry. Libraries are scanned first, then adventures, then characters.
    pub fn load(&self) -> Result<ContentRegistry> {
        let mut registry = ContentRegistry::default();
        let mut libraries = Vec::new();
        let mut adventures = Vec::new();
        let mut characters = Vec::new();

        if let Some(manifest_path) = self.campaign_path() {
            let manifest = load_manifest(&manifest_path)?;
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

        for source in [&libraries, &adventures, &characters] {
            for path in source {
                let mut files = Vec::new();
                collect_files(path, &mut files)?;
                for file in files {
                    load_file(&file, &mut registry)?;
                }
            }
        }

        Ok(registry)
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

/// Deterministic recursive scan: entries are sorted by name.
fn collect_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read content source {}", path.display()))?;
    if metadata.is_dir() {
        let mut entries = std::fs::read_dir(path)
            .with_context(|| format!("failed to read content directory {}", path.display()))?
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("failed to read content directory {}", path.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            collect_files(&entry.path(), out)?;
        }
    } else if metadata.is_file() {
        if supported_content_file(path) {
            out.push(path.to_path_buf());
        } else {
            bail!(
                "unsupported content file {} (expected .yaml, .yml, .json or .md)",
                path.display()
            );
        }
    } else {
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

fn load_file(path: &Path, registry: &mut ContentRegistry) -> Result<()> {
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
                apply_document(&value, &source, registry)
                    .map_err(|error| anyhow!("{source}: {error}"))?;
            }
        }
        Some("json") => {
            let source = ContentSource::new(path, None);
            let value = serde_json::from_str::<serde_json::Value>(&text)
                .map_err(|error| anyhow!("{}: invalid JSON: {error}", source))?;
            apply_document(&value, &source, registry)
                .map_err(|error| anyhow!("{source}: {error}"))?;
        }
        Some("yaml" | "yml") => {
            let source = ContentSource::new(path, None);
            let value =
                parse_value(&text, Format::Yaml).map_err(|error| anyhow!("{source}: {error}"))?;
            apply_document(&value, &source, registry)
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
    registry: &mut ContentRegistry,
) -> Result<()> {
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
