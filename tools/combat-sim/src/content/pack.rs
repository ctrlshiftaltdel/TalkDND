//! Versioned content packs: metadata, index, canonical hash and validation.
//!
//! A pack is a self-describing container around the existing ContentFile.
//! The content hash is FNV-1a 64 over the canonical JSON bytes of the content
//! block; it is an integrity check, not a security boundary. The hash is not
//! used to unlock books or to identify PDFs.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::content::schema::ContentFile;

/// The only pack schema this build understands. Unknown versions fail closed.
pub const SCHEMA_VERSION: u32 = 1;

/// How a pack may be distributed. The loader records this; gating is handled
/// by the separate book-presence layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Distribution {
    /// SRD content, shippable under CC-BY-4.0.
    Srd,
    /// Full-book content distributed under the owner's licence.
    Full,
    /// Local or generated content.
    #[default]
    Local,
}

impl Distribution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Srd => "srd",
            Self::Full => "full",
            Self::Local => "local",
        }
    }
}

/// Pack metadata. content_hash is filled by ContentPack::new and checked by
/// ContentPack::from_value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackMeta {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub edition: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub distribution: Distribution,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

/// One searchable/indexed entity in a pack.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackEntry {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub page: Option<i32>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub refs: Vec<String>,
}

/// One image/map reference in a pack. Paths are relative to the pack source or
/// the book index; the loader does not open them yet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackImage {
    pub id: String,
    #[serde(default)]
    pub page: Option<i32>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

/// Optional search/index metadata. An empty index is valid; the Agent layer
/// can build one from the content later.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackIndex {
    #[serde(default)]
    pub entries: Vec<PackEntry>,
    #[serde(default)]
    pub images: Vec<PackImage>,
}

/// A complete pack file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentPack {
    pub pack: PackMeta,
    #[serde(default)]
    pub index: PackIndex,
    pub content: ContentFile,
}

impl ContentPack {
    /// Builds a pack and fills content_hash. The caller supplies the content
    /// and the descriptive metadata; the hash is never left to the caller.
    pub fn new(mut pack: PackMeta, content: ContentFile) -> Result<Self> {
        pack.content_hash = Some(content_hash(&content)?);
        let pack = Self {
            pack,
            index: PackIndex::default(),
            content,
        };
        pack.validate()?;
        Ok(pack)
    }

    /// Parses and validates a pack from JSON. This is the only entry point the
    /// loader should use; it verifies the stored hash against the content.
    pub fn from_value(value: &serde_json::Value) -> Result<Self> {
        let pack: Self = serde_json::from_value(value.clone())
            .context("invalid ContentPack: expected pack, optional index and content")?;
        pack.validate()?;
        Ok(pack)
    }

    /// Recomputes the content hash and checks identity, schema, index and
    /// duplicate entity ids. All failures are fail-closed.
    pub fn validate(&self) -> Result<()> {
        if self.pack.id.trim().is_empty() {
            bail!("content pack id is empty");
        }
        if self.pack.edition.trim().is_empty() {
            bail!("content pack {} has an empty edition", self.pack.id);
        }
        if self.pack.schema_version != SCHEMA_VERSION {
            bail!(
                "content pack {} uses schema_version {}; this build supports {}",
                self.pack.id,
                self.pack.schema_version,
                SCHEMA_VERSION
            );
        }
        let declared =
            self.pack.content_hash.as_deref().ok_or_else(|| {
                anyhow::anyhow!("content pack {} has no content_hash", self.pack.id)
            })?;
        let actual = content_hash(&self.content)?;
        if declared != actual {
            bail!(
                "content pack {} content hash mismatch: declared {}, actual {}",
                self.pack.id,
                declared,
                actual
            );
        }
        let entities = entities(&self.content)?;
        validate_index(&self.pack.id, &self.index, &entities)?;
        Ok(())
    }

    pub fn entity_count(&self) -> usize {
        entity_count(&self.content)
    }

    pub fn image_count(&self) -> usize {
        self.index.images.len()
    }
}

/// Canonical hash of a content block. serde_json serializes struct fields in
/// declaration order and content vectors in input order, so the result is
/// stable for a given pack file.
pub fn content_hash(content: &ContentFile) -> Result<String> {
    let bytes = serde_json::to_vec(content).context("serialize pack content")?;
    Ok(crate::integrity::fingerprint(&bytes))
}

/// Whether a JSON/YAML document is a pack rather than ordinary content.
pub fn looks_like_pack(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.contains_key("pack") && object.contains_key("content"))
}

fn entity_count(content: &ContentFile) -> usize {
    content.conditions.as_ref().map_or(0, Vec::len)
        + content.masteries.as_ref().map_or(0, Vec::len)
        + content.abilities.as_ref().map_or(0, Vec::len)
        + content.creatures.as_ref().map_or(0, Vec::len)
        + content.ai_templates.as_ref().map_or(0, Vec::len)
        + content.scenes.as_ref().map_or(0, Vec::len)
        + content.encounters.as_ref().map_or(0, Vec::len)
        + content.items.as_ref().map_or(0, Vec::len)
        + content.classes.as_ref().map_or(0, Vec::len)
        + content.subclasses.as_ref().map_or(0, Vec::len)
        + content.species.as_ref().map_or(0, Vec::len)
        + content.backgrounds.as_ref().map_or(0, Vec::len)
        + content.feats.as_ref().map_or(0, Vec::len)
        + content.rule_sections.as_ref().map_or(0, Vec::len)
        + content.tables.as_ref().map_or(0, Vec::len)
}

fn entities(content: &ContentFile) -> Result<BTreeMap<(String, &'static str), String>> {
    let mut out = BTreeMap::new();
    let mut add = |id: &str, kind: &'static str, name: Option<&str>| -> Result<()> {
        let key = (id.to_string(), kind);
        if out.insert(key, name.unwrap_or("").to_string()).is_some() {
            bail!("duplicate entity {id} of kind {kind} in pack content");
        }
        Ok(())
    };
    if let Some(items) = &content.conditions {
        for item in items {
            add(&item.id, "condition", item.name.as_deref())?;
        }
    }
    if let Some(items) = &content.masteries {
        for item in items {
            add(&item.id, "mastery", item.name.as_deref())?;
        }
    }
    if let Some(items) = &content.abilities {
        for item in items {
            add(&item.id, "ability", item.name.as_deref())?;
        }
    }
    if let Some(items) = &content.creatures {
        for item in items {
            add(&item.id, "creature", item.name.as_deref())?;
        }
    }
    if let Some(items) = &content.ai_templates {
        for item in items {
            add(&item.id, "ai-template", item.name.as_deref())?;
        }
    }
    if let Some(items) = &content.scenes {
        for item in items {
            add(&item.id, "scene", item.name.as_deref())?;
        }
    }
    if let Some(items) = &content.encounters {
        for item in items {
            add(&item.id, "encounter", item.name.as_deref())?;
        }
    }
    for (items, kind) in [
        (&content.items, "item"),
        (&content.classes, "class"),
        (&content.subclasses, "subclass"),
        (&content.species, "species"),
        (&content.backgrounds, "background"),
        (&content.feats, "feat"),
        (&content.rule_sections, "rule-section"),
        (&content.tables, "table"),
    ] {
        if let Some(items) = items {
            for item in items {
                add(&item.id, kind, item.name.as_deref())?;
            }
        }
    }
    Ok(out)
}

fn canonical_kind(text: &str) -> Option<&'static str> {
    match text.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "condition" | "conditions" => Some("condition"),
        "mastery" | "masteries" => Some("mastery"),
        "ability" | "abilities" => Some("ability"),
        "creature" | "creatures" => Some("creature"),
        "ai-template" | "ai-templates" | "template" | "templates" => Some("ai-template"),
        "scene" | "scenes" => Some("scene"),
        "encounter" | "encounters" => Some("encounter"),
        "item" | "items" => Some("item"),
        "class" | "classes" => Some("class"),
        "subclass" | "subclasses" => Some("subclass"),
        "species" | "race" | "races" => Some("species"),
        "background" | "backgrounds" => Some("background"),
        "feat" | "feats" => Some("feat"),
        "rule-section" | "rule-sections" | "rule" | "rules-section" => Some("rule-section"),
        "table" | "tables" => Some("table"),
        _ => None,
    }
}

fn validate_index(
    pack_id: &str,
    index: &PackIndex,
    entities: &BTreeMap<(String, &'static str), String>,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for entry in &index.entries {
        let kind = canonical_kind(&entry.kind).ok_or_else(|| {
            anyhow::anyhow!(
                "content pack {pack_id} index entry {} has unknown kind {}",
                entry.id,
                entry.kind
            )
        })?;
        if !entities.contains_key(&(entry.id.clone(), kind)) {
            bail!(
                "content pack {pack_id} index references missing {kind} {}",
                entry.id
            );
        }
        if !seen.insert((entry.id.clone(), kind)) {
            bail!("content pack {pack_id} index repeats {kind} {}", entry.id);
        }
    }
    let mut image_ids = BTreeSet::new();
    for image in &index.images {
        if image.id.trim().is_empty() {
            bail!("content pack {pack_id} has an image with an empty id");
        }
        if !image_ids.insert(image.id.clone()) {
            bail!("content pack {pack_id} repeats image id {}", image.id);
        }
    }
    Ok(())
}
