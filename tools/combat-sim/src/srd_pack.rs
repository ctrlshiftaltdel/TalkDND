//! Import SRD 5.1 / 5.2.1 from the 5e-database JSON into ContentPack files.
//!
//! The generated packs are the default content base for players who do not have
//! the SRD PDFs. They are CC-BY-4.0 data and are committed to the repository;
//! the upstream JSON is not. Regenerate instead of hand-editing.
//!
//! One pack per content category keeps packs small and makes the index easy to
//! inspect. Monsters and spells reuse the engine converters in [`crate::srd`] so
//! the packs keep the same automation the old YAML import had.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::content::pack::{
    ContentPack, Distribution, PackEntry, PackIndex, PackMeta, SCHEMA_VERSION,
};
use crate::content::schema::ContentFile;
use crate::srd::{self, text_blocks};

const UPSTREAM: &str = "github.com/5e-bits/5e-database";

/// One generated pack, used for the CLI summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackReport {
    pub id: String,
    pub file: String,
    pub entities: usize,
    pub summary: String,
}

/// Aggregate result of one edition import.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub edition: String,
    pub packs: Vec<PackReport>,
    pub gaps: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Edition {
    year: &'static str,
    label: &'static str,
}

fn edition_config(year: &str) -> Result<Edition> {
    match year {
        "2014" => Ok(Edition {
            year: "2014",
            label: "SRD 5.1",
        }),
        "2024" => Ok(Edition {
            year: "2024",
            label: "SRD 5.2.1",
        }),
        other => bail!("unsupported SRD edition `{other}`; expected 2014 or 2024"),
    }
}

/// Imports one edition from `src` (a directory of `5e-SRD-*.json`) into
/// `out`, writing `<pack-id>.pack.json` files.
pub fn import(edition: &str, src: &Path, out: &Path) -> Result<ImportReport> {
    let ed = edition_config(edition)?;
    if !src.is_dir() {
        bail!(
            "SRD source `{}` is not a directory of 5e-database JSON",
            src.display()
        );
    }
    std::fs::create_dir_all(out)
        .with_context(|| format!("create pack output directory {}", out.display()))?;

    let mut report = ImportReport {
        edition: ed.year.to_string(),
        ..ImportReport::default()
    };

    import_monsters(&ed, src, out, &mut report)?;
    import_spells(&ed, src, out, &mut report)?;
    import_classes(&ed, src, out, &mut report)?;
    import_species(&ed, src, out, &mut report)?;
    import_backgrounds(&ed, src, out, &mut report)?;
    import_feats(&ed, src, out, &mut report)?;
    import_items(&ed, src, out, &mut report)?;
    import_rules(&ed, src, out, &mut report)?;

    Ok(report)
}

// ---------------------------------------------------------------------------
// Pack emission
// ---------------------------------------------------------------------------

struct PackSpec {
    id: String,
    name: String,
    description: String,
    fields: Vec<(&'static str, Vec<Value>)>,
    entries: Vec<PackEntry>,
    summary: String,
}

fn emit(out: &Path, ed: &Edition, mut spec: PackSpec, report: &mut ImportReport) -> Result<()> {
    let mut map = Map::new();
    for (field, values) in spec.fields {
        if !values.is_empty() {
            map.insert(field.to_string(), Value::Array(values));
        }
    }
    let content: ContentFile = serde_json::from_value(Value::Object(map))
        .context("generated SRD content does not match the content schema")?;
    spec.entries
        .sort_by(|a, b| (a.kind.as_str(), a.id.as_str()).cmp(&(b.kind.as_str(), b.id.as_str())));
    let meta = PackMeta {
        id: spec.id.clone(),
        name: Some(spec.name),
        edition: ed.year.to_string(),
        source: Some(format!("{} ({UPSTREAM})", ed.label)),
        license: Some("CC-BY-4.0".to_string()),
        distribution: Distribution::Srd,
        schema_version: SCHEMA_VERSION,
        content_hash: None,
        created_at: None,
        description: Some(spec.description),
    };
    let mut pack = ContentPack::new(meta, content)?;
    if pack.entity_count() == 0 {
        report.gaps.push(format!(
            "{}: no entities imported (source missing or empty)",
            spec.id
        ));
        return Ok(());
    }
    pack.index = PackIndex {
        entries: spec.entries,
        images: Vec::new(),
    };
    pack.validate()?;

    let file = format!("{}.pack.json", spec.id);
    let text = serde_json::to_string_pretty(&pack)
        .with_context(|| format!("serialize content pack {}", spec.id))?;
    std::fs::write(out.join(&file), format!("{text}\n"))
        .with_context(|| format!("write content pack {}", out.join(&file).display()))?;
    report.packs.push(PackReport {
        id: spec.id,
        file,
        entities: pack.entity_count(),
        summary: spec.summary,
    });
    Ok(())
}

fn entry(id: &str, kind: &str, name: Option<&str>, section: &str) -> PackEntry {
    PackEntry {
        id: id.to_string(),
        kind: kind.to_string(),
        name: name.map(str::to_string),
        aliases: Vec::new(),
        page: None,
        section: Some(section.to_string()),
        refs: Vec::new(),
    }
}

fn domain(
    ed: &Edition,
    id: &str,
    name: Option<&str>,
    section: &str,
    tags: &[&str],
    text: String,
    data: Value,
) -> Value {
    json!({
        "id": id,
        "name": name,
        "source": format!("{} ({UPSTREAM})", ed.label),
        "section": section,
        "tags": tags,
        "text": text,
        "data": data,
    })
}

fn with_refs(mut value: Value, refs: &[&str]) -> Value {
    if !refs.is_empty() {
        value["refs"] = json!(refs);
    }
    value
}

// ---------------------------------------------------------------------------
// Source reading
// ---------------------------------------------------------------------------

fn read_array(dir: &Path, name: &str) -> Result<Option<Vec<Value>>> {
    let path = dir.join(format!("5e-SRD-{name}.json"));
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read SRD source {}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("parse SRD source {}", path.display()))?;
    let Value::Array(items) = value else {
        bail!("SRD source {} is not a JSON array", path.display());
    };
    Ok(Some(items))
}

fn require_array(dir: &Path, name: &str, report: &mut ImportReport) -> Result<Vec<Value>> {
    match read_array(dir, name)? {
        Some(items) => Ok(items),
        None => {
            report.gaps.push(format!("{name}.json is missing"));
            Ok(Vec::new())
        }
    }
}

fn id_of(value: &Value) -> Option<&str> {
    value.get("index").and_then(Value::as_str)
}

fn name_of(value: &Value) -> Option<&str> {
    value.get("name").and_then(Value::as_str)
}

fn sort_values(values: &mut [Value]) {
    values.sort_by(|a, b| {
        a.get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("id").and_then(Value::as_str).unwrap_or(""))
    });
}

/// Removes the narrative fields that were already rendered into `text`.
fn without(value: &Value, keys: &[&str]) -> Value {
    let mut map = value.as_object().cloned().unwrap_or_default();
    for key in keys {
        map.remove(*key);
    }
    Value::Object(map)
}

fn narrative(value: &Value, keys: &[&str]) -> String {
    let mut blocks = Vec::new();
    for key in keys {
        for block in text_blocks(value.get(*key)) {
            if !block.trim().is_empty() {
                blocks.push(block);
            }
        }
    }
    blocks.join("\n\n")
}

fn names_of(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.get("name")
                .and_then(Value::as_str)
                .or_else(|| item.as_str())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn scalar(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

fn level_label(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Object(map)) => map
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

fn damage_note(value: Option<&Value>) -> String {
    let Some(damage) = value else {
        return String::new();
    };
    let dice = damage
        .get("damage_dice")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = damage
        .pointer("/damage_type/index")
        .and_then(Value::as_str)
        .unwrap_or("");
    if dice.is_empty() && kind.is_empty() {
        String::new()
    } else if kind.is_empty() {
        dice.to_string()
    } else if dice.is_empty() {
        kind.to_string()
    } else {
        format!("{dice} {kind}")
    }
}

// ---------------------------------------------------------------------------
// Monsters
// ---------------------------------------------------------------------------

fn import_monsters(ed: &Edition, src: &Path, out: &Path, report: &mut ImportReport) -> Result<()> {
    let items = require_array(src, "Monsters", report)?;
    let mut values = Vec::new();
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for item in &items {
        match srd::monster(item, ed.label) {
            Some(value) => {
                let id = value["id"].as_str().unwrap_or_default().to_string();
                let name = value["name"].as_str().unwrap_or(&id).to_string();
                entries.push(entry(&id, "creature", Some(&name), "Monsters"));
                values.push(value);
            }
            None => skipped.push(id_of(item).unwrap_or("<no-index>").to_string()),
        }
    }
    sort_values(&mut values);
    if !skipped.is_empty() {
        report.gaps.push(format!(
            "monsters without a statblock: {}",
            skipped.join(", ")
        ));
    }
    let summary = format!("{} creatures", values.len());
    emit(
        out,
        ed,
        PackSpec {
            id: format!("srd-{}-monsters", pack_version(ed)),
            name: format!("{} Monsters", ed.label),
            description: format!("{} monsters from {}.", ed.label, UPSTREAM),
            fields: vec![("creatures", values)],
            entries,
            summary,
        },
        report,
    )
}

// ---------------------------------------------------------------------------
// Spells
// ---------------------------------------------------------------------------

fn import_spells(ed: &Edition, src: &Path, out: &Path, report: &mut ImportReport) -> Result<()> {
    let items = require_array(src, "Spells", report)?;
    let mut values = Vec::new();
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for item in &items {
        match srd::spell(item, ed.label) {
            Some(value) => {
                let id = value["id"].as_str().unwrap_or_default().to_string();
                let name = value["name"].as_str().unwrap_or(&id).to_string();
                entries.push(entry(&id, "ability", Some(&name), "Spells"));
                values.push(value);
            }
            None => skipped.push(id_of(item).unwrap_or("<no-index>").to_string()),
        }
    }
    sort_values(&mut values);
    if !skipped.is_empty() {
        report
            .gaps
            .push(format!("spells without text: {}", skipped.join(", ")));
    }
    let summary = format!("{} spells", values.len());
    emit(
        out,
        ed,
        PackSpec {
            id: format!("srd-{}-spells", pack_version(ed)),
            name: format!("{} Spells", ed.label),
            description: format!("{} spells from {}.", ed.label, UPSTREAM),
            fields: vec![("abilities", values)],
            entries,
            summary,
        },
        report,
    )
}

// ---------------------------------------------------------------------------
// Classes, subclasses, features and level tables
// ---------------------------------------------------------------------------

fn import_classes(ed: &Edition, src: &Path, out: &Path, report: &mut ImportReport) -> Result<()> {
    let classes = require_array(src, "Classes", report)?;
    let subclasses = require_array(src, "Subclasses", report)?;
    let levels = require_array(src, "Levels", report)?;
    let features = require_array(src, "Features", report)?;
    if classes
        .iter()
        .all(|class| narrative(class, &["desc", "description"]).is_empty())
    {
        report.gaps.push(
            "class prose: upstream 5e-database has no class descriptions; mechanics, level tables and feature text are imported"
                .to_string(),
        );
    }

    let mut rows_by_class: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut rows_by_subclass: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in &levels {
        let subclass = row.pointer("/subclass/index").and_then(Value::as_str);
        if let Some(subclass) = subclass {
            rows_by_subclass
                .entry(subclass.to_string())
                .or_default()
                .push(row.clone());
            continue;
        }
        if let Some(class) = row.pointer("/class/index").and_then(Value::as_str) {
            rows_by_class
                .entry(class.to_string())
                .or_default()
                .push(row.clone());
        }
    }
    for rows in rows_by_class
        .values_mut()
        .chain(rows_by_subclass.values_mut())
    {
        rows.sort_by_key(|row| row.get("level").and_then(Value::as_i64).unwrap_or(0));
    }

    let mut class_values = Vec::new();
    let mut class_entries = Vec::new();
    let mut table_values = Vec::new();
    let mut table_entries = Vec::new();
    for class in &classes {
        let Some(id) = id_of(class) else { continue };
        let name = name_of(class).unwrap_or(id);
        let rows = rows_by_class.get(id).cloned().unwrap_or_default();
        let mut text = narrative(class, &["desc"]);
        if let Some(hit_die) = class.get("hit_die").and_then(Value::as_i64) {
            text.push_str(&format!("\n\nHit Die: d{hit_die}"));
        }
        let saves = names_of(class.get("saving_throws"));
        if !saves.is_empty() {
            text.push_str(&format!("\nSaving Throws: {saves}"));
        }
        let profs = names_of(class.get("proficiencies"));
        if !profs.is_empty() {
            text.push_str(&format!("\nProficiencies: {profs}"));
        }
        let primary = class
            .pointer("/primary_ability/desc")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !primary.is_empty() {
            text.push_str(&format!("\nPrimary Ability: {primary}"));
        }
        for row in &rows {
            let level = level_label(row.get("level"));
            let prof = scalar(row.get("prof_bonus"));
            let feats = names_of(row.get("features"));
            text.push_str(&format!("\nLevel {level} (prof +{prof}): {feats}"));
        }
        class_values.push(domain(
            ed,
            id,
            Some(name),
            "Classes",
            &["class", "srd"],
            text,
            without(class, &["desc"]),
        ));
        class_entries.push(entry(id, "class", Some(name), "Classes"));

        let mut table_text = String::new();
        for row in &rows {
            let level = level_label(row.get("level"));
            let prof = scalar(row.get("prof_bonus"));
            let feats = names_of(row.get("features"));
            table_text.push_str(&format!("Level {level} (prof +{prof}): {feats}\n"));
        }
        let table_id = format!("class-{id}-levels");
        let table_name = format!("{name} Levels");
        table_values.push(domain(
            ed,
            &table_id,
            Some(&table_name),
            "Classes",
            &["table", "class", "srd"],
            table_text,
            json!({"class": id, "rows": rows}),
        ));
        table_entries.push(entry(&table_id, "table", Some(&table_name), "Classes"));
    }

    let mut subclass_values = Vec::new();
    let mut subclass_entries = Vec::new();
    for subclass in &subclasses {
        let Some(id) = id_of(subclass) else {
            continue;
        };
        let name = name_of(subclass).unwrap_or(id);
        let class = subclass
            .pointer("/class/index")
            .and_then(Value::as_str)
            .unwrap_or("");
        let class_name = subclass
            .pointer("/class/name")
            .and_then(Value::as_str)
            .unwrap_or(class);
        let flavor = scalar(subclass.get("subclass_flavor"));
        let summary = scalar(subclass.get("summary"));
        let mut text = narrative(subclass, &["desc", "description"]);
        if !flavor.is_empty() {
            text.push_str(&format!("\n\nSubclass flavor: {flavor}"));
        }
        if !class_name.is_empty() {
            text.push_str(&format!("\nClass: {class_name}"));
        }
        if !summary.is_empty() && summary != "null" {
            text.push_str(&format!("\nSummary: {summary}"));
        }
        let rows = rows_by_subclass.get(id).cloned().unwrap_or_default();
        for row in &rows {
            let level = level_label(row.get("level"));
            let feats = names_of(row.get("features"));
            if !feats.is_empty() {
                text.push_str(&format!("\nLevel {level}: {feats}"));
            }
        }
        let mut data = without(subclass, &["desc", "description"]);
        if !rows.is_empty() {
            data["levels"] = json!(rows);
        }
        let refs: Vec<&str> = if class.is_empty() {
            Vec::new()
        } else {
            vec![class]
        };
        subclass_values.push(with_refs(
            domain(
                ed,
                id,
                Some(name),
                "Classes",
                &["subclass", "srd"],
                text,
                data,
            ),
            &refs,
        ));
        subclass_entries.push(entry(id, "subclass", Some(name), "Classes"));
    }

    let mut feature_values = Vec::new();
    let mut feature_entries = Vec::new();
    for feature in &features {
        let Some(id) = id_of(feature) else { continue };
        let name = name_of(feature).unwrap_or(id);
        let class = feature
            .pointer("/class/index")
            .and_then(Value::as_str)
            .unwrap_or("");
        let subclass = feature
            .pointer("/subclass/index")
            .and_then(Value::as_str)
            .unwrap_or("");
        let class_name = feature
            .pointer("/class/name")
            .and_then(Value::as_str)
            .unwrap_or("Features");
        let mut text = narrative(feature, &["desc", "description"]);
        let level = level_label(feature.get("level"));
        if !level.is_empty() {
            text.push_str(&format!("\n\nLevel: {level}"));
        }
        let prerequisites = names_of(feature.get("prerequisites"));
        if !prerequisites.is_empty() {
            text.push_str(&format!("\nPrerequisites: {prerequisites}"));
        }
        let refs: Vec<&str> = [class, subclass]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect();
        let tags = if subclass.is_empty() {
            vec!["feature", "class", "srd"]
        } else {
            vec!["feature", "subclass", "srd"]
        };
        feature_values.push(with_refs(
            domain(
                ed,
                id,
                Some(name),
                class_name,
                &tags,
                text,
                without(feature, &["desc", "description"]),
            ),
            &refs,
        ));
        feature_entries.push(entry(id, "rule-section", Some(name), class_name));
    }

    let summary = format!(
        "{} classes, {} subclasses, {} features, {} tables",
        class_values.len(),
        subclass_values.len(),
        feature_values.len(),
        table_values.len()
    );
    emit(
        out,
        ed,
        PackSpec {
            id: format!("srd-{}-classes", pack_version(ed)),
            name: format!("{} Classes", ed.label),
            description: format!(
                "{} classes, subclasses, features and level tables from {}.",
                ed.label, UPSTREAM
            ),
            fields: vec![
                ("classes", class_values),
                ("subclasses", subclass_values),
                ("rule_sections", feature_values),
                ("tables", table_values),
            ],
            entries: [
                class_entries,
                subclass_entries,
                feature_entries,
                table_entries,
            ]
            .concat(),
            summary,
        },
        report,
    )
}

// ---------------------------------------------------------------------------
// Species / races, subspecies / subraces, traits
// ---------------------------------------------------------------------------

fn import_species(ed: &Edition, src: &Path, out: &Path, report: &mut ImportReport) -> Result<()> {
    let (species_file, subspecies_file) = if ed.year == "2014" {
        ("Races", "Subraces")
    } else {
        ("Species", "Subspecies")
    };
    let species = require_array(src, species_file, report)?;
    let subspecies = require_array(src, subspecies_file, report)?;
    let traits = require_array(src, "Traits", report)?;

    let mut species_values = Vec::new();
    let mut species_entries = Vec::new();
    for item in &species {
        let Some(id) = id_of(item) else { continue };
        let name = name_of(item).unwrap_or(id);
        let mut text = narrative(
            item,
            &[
                "desc",
                "description",
                "age",
                "alignment",
                "size_description",
                "language_desc",
            ],
        );
        let speed = scalar(item.get("speed"));
        if !speed.is_empty() {
            text.push_str(&format!("\n\nSpeed: {speed}"));
        }
        let size = scalar(item.get("size"));
        if !size.is_empty() {
            text.push_str(&format!("\nSize: {size}"));
        }
        let kind = scalar(item.get("type"));
        if !kind.is_empty() {
            text.push_str(&format!("\nCreature Type: {kind}"));
        }
        let trait_names = names_of(item.get("traits"));
        if !trait_names.is_empty() {
            text.push_str(&format!("\nTraits: {trait_names}"));
        }
        species_values.push(domain(
            ed,
            id,
            Some(name),
            "Species",
            &["species", "srd"],
            text,
            without(
                item,
                &[
                    "desc",
                    "description",
                    "age",
                    "alignment",
                    "size_description",
                    "language_desc",
                ],
            ),
        ));
        species_entries.push(entry(id, "species", Some(name), "Species"));
    }

    let mut subspecies_values = Vec::new();
    let mut subspecies_entries = Vec::new();
    for item in &subspecies {
        let Some(id) = id_of(item) else { continue };
        let name = name_of(item).unwrap_or(id);
        let parent = if ed.year == "2014" {
            item.pointer("/race/index").and_then(Value::as_str)
        } else {
            item.pointer("/species/index").and_then(Value::as_str)
        }
        .unwrap_or("");
        let mut text = narrative(item, &["desc", "description"]);
        let trait_names = names_of(item.get("traits").or_else(|| item.get("racial_traits")));
        if !trait_names.is_empty() {
            text.push_str(&format!("\n\nTraits: {trait_names}"));
        }
        let refs: Vec<&str> = if parent.is_empty() {
            Vec::new()
        } else {
            vec![parent]
        };
        subspecies_values.push(with_refs(
            domain(
                ed,
                id,
                Some(name),
                "Species",
                &["subspecies", "srd"],
                text,
                without(item, &["desc", "description"]),
            ),
            &refs,
        ));
        subspecies_entries.push(entry(id, "species", Some(name), "Species"));
    }

    let mut trait_values = Vec::new();
    let mut trait_entries = Vec::new();
    for item in &traits {
        let Some(id) = id_of(item) else { continue };
        let name = name_of(item).unwrap_or(id);
        let mut text = narrative(item, &["desc", "description"]);
        let profs = names_of(item.get("proficiencies"));
        if !profs.is_empty() {
            text.push_str(&format!("\n\nProficiencies: {profs}"));
        }
        let refs: Vec<&str> = ["races", "subraces", "species", "subspecies"]
            .iter()
            .filter_map(|key| item.get(*key))
            .flat_map(|value| value.as_array().into_iter().flatten())
            .filter_map(|value| value.get("index").and_then(Value::as_str))
            .collect();
        trait_values.push(with_refs(
            domain(
                ed,
                id,
                Some(name),
                "Species",
                &["trait", "species", "srd"],
                text,
                without(item, &["desc", "description"]),
            ),
            &refs,
        ));
        trait_entries.push(entry(id, "rule-section", Some(name), "Species"));
    }

    let summary = format!(
        "{} species, {} subspecies, {} traits",
        species_values.len(),
        subspecies_values.len(),
        trait_values.len()
    );
    emit(
        out,
        ed,
        PackSpec {
            id: format!("srd-{}-species", pack_version(ed)),
            name: format!("{} Species", ed.label),
            description: format!(
                "{} species, subspecies and traits from {}.",
                ed.label, UPSTREAM
            ),
            fields: vec![
                ("species", [species_values, subspecies_values].concat()),
                ("rule_sections", trait_values),
            ],
            entries: [species_entries, subspecies_entries, trait_entries].concat(),
            summary,
        },
        report,
    )
}

// ---------------------------------------------------------------------------
// Backgrounds, feats, items, rules
// ---------------------------------------------------------------------------

fn import_backgrounds(
    ed: &Edition,
    src: &Path,
    out: &Path,
    report: &mut ImportReport,
) -> Result<()> {
    let items = require_array(src, "Backgrounds", report)?;
    let mut values = Vec::new();
    let mut entries = Vec::new();
    for item in &items {
        let Some(id) = id_of(item) else { continue };
        let name = name_of(item).unwrap_or(id);
        let mut text = narrative(item, &["desc", "description"]);
        let feat = scalar(item.pointer("/feat/name"));
        if !feat.is_empty() {
            text.push_str(&format!("\n\nFeat: {feat}"));
        }
        let profs = names_of(item.get("proficiencies"))
            + &names_of(item.get("skill_proficiencies"))
            + &names_of(item.get("tool_proficiencies"));
        if !profs.is_empty() {
            text.push_str(&format!("\nProficiencies: {profs}"));
        }
        let languages = names_of(item.get("languages"));
        if !languages.is_empty() {
            text.push_str(&format!("\nLanguages: {languages}"));
        }
        let feature = scalar(item.pointer("/feature/name"));
        let feature_desc = scalar(item.pointer("/feature/desc"));
        if !feature.is_empty() {
            text.push_str(&format!("\n\n{feature}. {feature_desc}"));
        }
        values.push(domain(
            ed,
            id,
            Some(name),
            "Backgrounds",
            &["background", "srd"],
            text,
            without(item, &["desc", "description"]),
        ));
        entries.push(entry(id, "background", Some(name), "Backgrounds"));
    }
    sort_values(&mut values);
    let summary = format!("{} backgrounds", values.len());
    emit(
        out,
        ed,
        PackSpec {
            id: format!("srd-{}-backgrounds", pack_version(ed)),
            name: format!("{} Backgrounds", ed.label),
            description: format!("{} backgrounds from {}.", ed.label, UPSTREAM),
            fields: vec![("backgrounds", values)],
            entries,
            summary,
        },
        report,
    )
}

fn import_feats(ed: &Edition, src: &Path, out: &Path, report: &mut ImportReport) -> Result<()> {
    let items = require_array(src, "Feats", report)?;
    let mut values = Vec::new();
    let mut entries = Vec::new();
    for item in &items {
        let Some(id) = id_of(item) else { continue };
        let name = name_of(item).unwrap_or(id);
        let mut text = narrative(item, &["desc", "description"]);
        let kind = scalar(item.get("type"));
        if !kind.is_empty() {
            text.push_str(&format!("\n\nType: {kind}"));
        }
        let prerequisites = item
            .get("prerequisites")
            .map(|value| names_of(Some(value)))
            .unwrap_or_default();
        if !prerequisites.is_empty() {
            text.push_str(&format!("\nPrerequisites: {prerequisites}"));
        }
        let repeatable = item
            .get("repeatable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if repeatable {
            text.push_str("\nRepeatable.");
        }
        values.push(domain(
            ed,
            id,
            Some(name),
            "Feats",
            &["feat", "srd"],
            text,
            without(item, &["desc", "description"]),
        ));
        entries.push(entry(id, "feat", Some(name), "Feats"));
    }
    sort_values(&mut values);
    let summary = format!("{} feats", values.len());
    emit(
        out,
        ed,
        PackSpec {
            id: format!("srd-{}-feats", pack_version(ed)),
            name: format!("{} Feats", ed.label),
            description: format!("{} feats from {}.", ed.label, UPSTREAM),
            fields: vec![("feats", values)],
            entries,
            summary,
        },
        report,
    )
}

fn import_items(ed: &Edition, src: &Path, out: &Path, report: &mut ImportReport) -> Result<()> {
    let equipment = require_array(src, "Equipment", report)?;
    let magic = require_array(src, "Magic-Items", report)?;
    let poisons = read_array(src, "Poisons")?.unwrap_or_default();

    let mut values = Vec::new();
    let mut entries = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for item in &equipment {
        let Some(id) = id_of(item) else { continue };
        let name = name_of(item).unwrap_or(id);
        seen.insert(id.to_string());
        let categories = names_of(
            item.get("equipment_categories")
                .or_else(|| item.get("equipment_category")),
        );
        let mut text = narrative(item, &["desc", "description"]);
        if !categories.is_empty() {
            text.push_str(&format!("\n\nCategory: {categories}"));
        }
        let cost = item
            .get("cost")
            .map(|cost| {
                format!(
                    "{} {}",
                    scalar(cost.get("quantity")),
                    scalar(cost.get("unit"))
                )
            })
            .unwrap_or_default();
        if !cost.trim().is_empty() {
            text.push_str(&format!("\nCost: {}", cost.trim()));
        }
        let weight = scalar(item.get("weight"));
        if !weight.is_empty() {
            text.push_str(&format!("\nWeight: {weight}"));
        }
        let damage = damage_note(item.get("damage"));
        if !damage.is_empty() {
            text.push_str(&format!("\nDamage: {damage}"));
        }
        let ac = item
            .pointer("/armor_class/base")
            .and_then(Value::as_i64)
            .map(|base| base.to_string())
            .unwrap_or_default();
        if !ac.is_empty() {
            text.push_str(&format!("\nArmor Class: {ac}"));
        }
        let props = names_of(item.get("properties"));
        if !props.is_empty() {
            text.push_str(&format!("\nProperties: {props}"));
        }
        let contents = names_of(item.get("contents"));
        if !contents.is_empty() {
            text.push_str(&format!("\nContents: {contents}"));
        }
        values.push(domain(
            ed,
            id,
            Some(name),
            "Items",
            &["equipment", "srd"],
            text,
            without(item, &["desc", "description"]),
        ));
        entries.push(entry(id, "item", Some(name), "Items"));
    }
    for item in &magic {
        let Some(id) = id_of(item) else { continue };
        let name = name_of(item).unwrap_or(id);
        if !seen.insert(id.to_string()) {
            report.gaps.push(format!(
                "items: magic item `{id}` duplicates an equipment id; kept the equipment entry"
            ));
            continue;
        }
        let rarity = scalar(item.pointer("/rarity/name"));
        let mut text = narrative(item, &["desc", "description"]);
        if !rarity.is_empty() {
            text.push_str(&format!("\n\nRarity: {rarity}"));
        }
        let attunement = item
            .get("attunement")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || item.get("requires_attunement").is_some();
        if attunement {
            text.push_str("\nRequires attunement.");
        }
        let variants = names_of(item.get("variants"));
        if !variants.is_empty() {
            text.push_str(&format!("\nVariants: {variants}"));
        }
        values.push(domain(
            ed,
            id,
            Some(name),
            "Items",
            &["magic-item", "srd"],
            text,
            without(item, &["desc", "description"]),
        ));
        entries.push(entry(id, "item", Some(name), "Items"));
    }
    for item in &poisons {
        let Some(id) = id_of(item) else { continue };
        let name = name_of(item).unwrap_or(id);
        if !seen.insert(id.to_string()) {
            report.gaps.push(format!(
                "items: poison `{id}` duplicates an item id; skipped"
            ));
            continue;
        }
        let mut text = narrative(item, &["desc", "description"]);
        let kind = scalar(item.get("type"));
        if !kind.is_empty() {
            text.push_str(&format!("\n\nType: {kind}"));
        }
        let cost = scalar(item.get("cost"));
        if !cost.is_empty() {
            text.push_str(&format!("\nCost: {cost}"));
        }
        values.push(domain(
            ed,
            id,
            Some(name),
            "Items",
            &["poison", "srd"],
            text,
            without(item, &["desc", "description"]),
        ));
        entries.push(entry(id, "item", Some(name), "Items"));
    }
    sort_values(&mut values);
    let summary = format!(
        "{} items ({} equipment, {} magic, {} poisons)",
        values.len(),
        equipment.len(),
        magic.len(),
        poisons.len()
    );
    emit(
        out,
        ed,
        PackSpec {
            id: format!("srd-{}-items", pack_version(ed)),
            name: format!("{} Items", ed.label),
            description: format!(
                "{} equipment, magic items and poisons from {}.",
                ed.label, UPSTREAM
            ),
            fields: vec![("items", values)],
            entries,
            summary,
        },
        report,
    )
}

fn import_rules(ed: &Edition, src: &Path, out: &Path, report: &mut ImportReport) -> Result<()> {
    let rules = read_array(src, "Rules")?.unwrap_or_default();
    let sections = read_array(src, "Rule-Sections")?.unwrap_or_default();
    if rules.is_empty() && sections.is_empty() {
        report.gaps.push(
            "rules text: upstream 5e-database has no Rules/Rule-Sections dataset for this edition"
                .to_string(),
        );
        return Ok(());
    }
    let mut values = Vec::new();
    let mut entries = Vec::new();
    for (items, tag) in [(&rules, "rule"), (&sections, "rule-section")] {
        for item in items {
            let Some(id) = id_of(item) else { continue };
            let name = name_of(item).unwrap_or(id);
            let text = narrative(item, &["desc", "description"]);
            let tags: Vec<&str> = vec![tag, "srd"];
            values.push(domain(
                ed,
                id,
                Some(name),
                "Rules",
                &tags,
                text,
                without(item, &["desc", "description"]),
            ));
            entries.push(entry(id, "rule-section", Some(name), "Rules"));
        }
    }
    sort_values(&mut values);
    let summary = format!("{} rule sections", values.len());
    emit(
        out,
        ed,
        PackSpec {
            id: format!("srd-{}-rules", pack_version(ed)),
            name: format!("{} Rules", ed.label),
            description: format!("{} rules text from {}.", ed.label, UPSTREAM),
            fields: vec![("rule_sections", values)],
            entries,
            summary,
        },
        report,
    )
}

fn pack_version(ed: &Edition) -> &'static str {
    match ed.year {
        "2014" => "5.1",
        _ => "5.2.1",
    }
}
