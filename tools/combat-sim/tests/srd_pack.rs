//! SRD pack import: fixtures, determinism, layering and committed data.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use combat_sim::content::pack::{ContentPack, Distribution, PackIndex, PackMeta, SCHEMA_VERSION};
use combat_sim::content::{ContentSources, ItemKind};
use combat_sim::srd_pack;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "combat-sim-srd-{label}-{}-{nanos}-{count}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_json(dir: &Path, name: &str, value: Value) {
    let path = dir.join(format!("5e-SRD-{name}.json"));
    fs::write(path, serde_json::to_string_pretty(&value).expect("json")).expect("write fixture");
}

fn monster() -> Value {
    json!({
        "index": "test-monster",
        "name": "Test Monster",
        "size": "Medium",
        "type": "fiend",
        "alignment": "neutral",
        "armor_class": [{"value": 13}],
        "hit_points": 10,
        "hit_points_roll": "3d6",
        "speed": {"walk": "30 ft."},
        "strength": 10,
        "dexterity": 12,
        "constitution": 10,
        "intelligence": 8,
        "wisdom": 10,
        "charisma": 8,
        "actions": [{
            "name": "Claw",
            "desc": "Melee Weapon Attack: +3 to hit, reach 5 ft., one target. Hit: 4 (1d6 + 1) slashing damage.",
            "attack_bonus": 3,
            "damage": [{"damage_dice": "1d6+1", "damage_type": {"index": "slashing"}}]
        }],
        "challenge_rating": 1,
        "xp": 200
    })
}

fn spell() -> Value {
    json!({
        "index": "test-spell",
        "name": "Test Spell",
        "level": 1,
        "desc": ["A test spell."],
        "higher_level": ["The damage increases."],
        "range": "60 feet",
        "components": ["V", "S"],
        "duration": "Instantaneous",
        "casting_time": "1 action",
        "school": {"index": "evocation"},
        "attack_type": "ranged",
        "damage": {
            "damage_type": {"index": "fire"},
            "damage_at_slot_level": {"1": "1d8"}
        }
    })
}

fn fixtures(dir: &Path, edition: &str) {
    write_json(dir, "Monsters", json!([monster()]));
    write_json(dir, "Spells", json!([spell()]));
    write_json(
        dir,
        "Classes",
        json!([{
            "index": "test-class",
            "name": "Test Class",
            "hit_die": 8,
            "saving_throws": [{"index": "dex", "name": "DEX"}],
            "proficiencies": [{"index": "light-armor", "name": "Light armor"}]
        }]),
    );
    write_json(
        dir,
        "Subclasses",
        json!([{
            "index": "test-subclass",
            "name": "Test Subclass",
            "class": {"index": "test-class", "name": "Test Class"},
            "desc": ["A test subclass."]
        }]),
    );
    write_json(
        dir,
        "Levels",
        json!([
            {
                "index": "test-class-1",
                "level": 1,
                "prof_bonus": 2,
                "features": [{"index": "test-feature", "name": "Test Feature"}],
                "class": {"index": "test-class", "name": "Test Class"}
            },
            {
                "index": "test-class-3",
                "level": 3,
                "prof_bonus": 2,
                "features": [{"index": "test-subclass-feature", "name": "Subclass Feature"}],
                "class": {"index": "test-class", "name": "Test Class"},
                "subclass": {"index": "test-subclass", "name": "Test Subclass"}
            }
        ]),
    );
    write_json(
        dir,
        "Features",
        json!([
            {
                "index": "test-feature",
                "name": "Test Feature",
                "level": 1,
                "desc": ["A test feature."],
                "class": {"index": "test-class", "name": "Test Class"}
            },
            {
                "index": "test-subclass-feature",
                "name": "Subclass Feature",
                "level": 3,
                "desc": ["A subclass feature."],
                "class": {"index": "test-class", "name": "Test Class"},
                "subclass": {"index": "test-subclass", "name": "Test Subclass"}
            }
        ]),
    );
    write_json(
        dir,
        "Traits",
        json!([{
            "index": "test-trait",
            "name": "Test Trait",
            "desc": ["A test trait."],
            "races": [{"index": "test-species"}],
            "species": [{"index": "test-species"}]
        }]),
    );
    write_json(
        dir,
        "Backgrounds",
        json!([{
            "index": "test-background",
            "name": "Test Background",
            "desc": ["A test background."]
        }]),
    );
    write_json(
        dir,
        "Feats",
        json!([{
            "index": "test-feat",
            "name": "Test Feat",
            "desc": ["A test feat."]
        }]),
    );
    write_json(
        dir,
        "Equipment",
        json!([{
            "index": "test-item",
            "name": "Test Item",
            "desc": ["A test item."],
            "equipment_category": {"index": "gear", "name": "Adventuring Gear"}
        }]),
    );
    write_json(
        dir,
        "Magic-Items",
        json!([{
            "index": "test-magic-item",
            "name": "Test Magic Item",
            "desc": ["A test magic item."],
            "rarity": {"name": "Rare"}
        }]),
    );
    if edition == "2014" {
        write_json(
            dir,
            "Races",
            json!([{
                "index": "test-species",
                "name": "Test Race",
                "speed": 30,
                "size": "Medium",
                "traits": [{"index": "test-trait", "name": "Test Trait"}]
            }]),
        );
        write_json(
            dir,
            "Subraces",
            json!([{
                "index": "test-subrace",
                "name": "Test Subrace",
                "desc": ["A test subrace."],
                "race": {"index": "test-species", "name": "Test Race"}
            }]),
        );
        write_json(
            dir,
            "Rules",
            json!([{"index": "test-rule", "name": "Test Rule", "desc": ["A test rule."]}]),
        );
        write_json(
            dir,
            "Rule-Sections",
            json!([{"index": "test-section", "name": "Test Section", "desc": ["A test section."]}]),
        );
    } else {
        write_json(
            dir,
            "Species",
            json!([{
                "index": "test-species",
                "name": "Test Species",
                "speed": 30,
                "size": "Medium",
                "type": "Humanoid",
                "traits": [{"index": "test-trait", "name": "Test Trait"}]
            }]),
        );
        write_json(
            dir,
            "Subspecies",
            json!([{
                "index": "test-subspecies",
                "name": "Test Subspecies",
                "species": {"index": "test-species", "name": "Test Species"}
            }]),
        );
        write_json(
            dir,
            "Poisons",
            json!([{
                "index": "test-poison",
                "name": "Test Poison",
                "type": "ingested",
                "description": "A test poison."
            }]),
        );
    }
}

fn load(dir: &Path) -> combat_sim::content::ContentRegistry {
    ContentSources {
        libraries: vec![dir.to_path_buf()],
        ..ContentSources::default()
    }
    .load()
    .expect("load library")
}

fn read_pack(path: &Path) -> ContentPack {
    let text = fs::read_to_string(path).expect("read pack");
    let value: Value = serde_json::from_str(&text).expect("pack json");
    ContentPack::from_value(&value).expect("valid pack")
}

#[test]
fn imports_both_editions_from_fixtures() {
    for edition in ["2014", "2024"] {
        let src = TempDir::new("src");
        let out = TempDir::new("out");
        fixtures(src.path(), edition);
        let report = srd_pack::import(edition, src.path(), out.path()).expect("import");
        assert_eq!(report.edition, edition);
        let expected = if edition == "2014" { 8 } else { 7 };
        assert_eq!(report.packs.len(), expected, "{:?}", report.packs);
        assert!(
            report.gaps.iter().any(|gap| gap.contains("class prose")),
            "{:?}",
            report.gaps
        );

        let registry = load(out.path());
        assert!(registry.find_any("test-monster").is_some());
        assert!(registry.find_any("test-spell").is_some());
        assert!(registry.find_any("test-class").is_some());
        assert!(registry.find_any("test-subclass").is_some());
        assert!(registry.find_any("test-feature").is_some());
        assert!(registry.find_any("test-trait").is_some());
        assert!(registry.find_any("test-background").is_some());
        assert!(registry.find_any("test-feat").is_some());
        assert!(registry.find_any("test-item").is_some());
        assert!(registry.find_any("test-magic-item").is_some());
        assert_eq!(
            registry
                .source_of(ItemKind::Class, "test-class")
                .expect("class source")
                .path
                .extension()
                .and_then(|ext| ext.to_str()),
            Some("json")
        );
    }
}

#[test]
fn imported_spells_still_emit_abilities_and_carry_spell_metadata() {
    let src = TempDir::new("spell-src");
    let out = TempDir::new("spell-out");
    fixtures(src.path(), "2014");
    let report = srd_pack::import("2014", src.path(), out.path()).expect("import");
    // The spells pack is still an abilities pack of the same shape.
    let spells = report
        .packs
        .iter()
        .find(|pack| pack.id.contains("spells"))
        .expect("spells pack");
    assert_eq!(spells.id, "srd-5.1-spells");

    let pack = read_pack(&out.path().join("srd-5.1-spells.pack.json"));
    let abilities = pack.content.abilities.as_ref().expect("abilities");
    assert_eq!(abilities.len(), 1);
    let ability = &abilities[0];
    // Old fields are untouched.
    assert_eq!(ability.id, "test-spell");
    assert_eq!(
        ability.cost.as_ref().expect("cost").resource,
        "spell-slot-1"
    );
    assert!(ability.manual.as_deref().unwrap().contains("A test spell."));
    // The new spell block is filled from the upstream metadata.
    let spell = ability.spell.as_ref().expect("spell metadata");
    assert_eq!(spell.level, 1);
    assert_eq!(
        spell.school.map(|school| school.as_tag()),
        Some("evocation")
    );
    assert!(spell.components.verbal && spell.components.somatic);
    assert!(spell.components.material.is_none());
    assert!(!spell.ritual);
    assert_eq!(spell.range_text.as_deref(), Some("60 feet"));
    assert_eq!(spell.duration_text.as_deref(), Some("Instantaneous"));
    let upcast = spell.upcast.as_ref().expect("upcast");
    assert_eq!(
        upcast.dice_by_level.get(&1).map(String::as_str),
        Some("1d8")
    );

    // The pack loads as a library and the metadata survives the round-trip.
    let registry = load(out.path());
    let loaded = registry.abilities.get("test-spell").expect("ability");
    assert!(loaded.spell.is_some());
}

#[test]
fn generation_is_deterministic() {
    let src = TempDir::new("det-src");
    fixtures(src.path(), "2014");
    let first = TempDir::new("det-a");
    let second = TempDir::new("det-b");
    srd_pack::import("2014", src.path(), first.path()).expect("first import");
    srd_pack::import("2014", src.path(), second.path()).expect("second import");

    let mut names: Vec<String> = fs::read_dir(first.path())
        .expect("read out")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty());
    for name in names {
        let a = fs::read(first.path().join(&name)).expect("first pack");
        let b = fs::read(second.path().join(&name)).expect("second pack");
        assert_eq!(a, b, "pack {name} is not deterministic");
    }
}

#[test]
fn rejects_unknown_edition() {
    let src = TempDir::new("bad-edition");
    let out = TempDir::new("bad-out");
    let error = srd_pack::import("2020", src.path(), out.path()).expect_err("unknown edition");
    assert!(
        format!("{error:#}").contains("unsupported SRD edition"),
        "{error:#}"
    );
}

#[test]
fn packs_load_before_ordinary_content() {
    let dir = TempDir::new("order");
    // The pack sorts after the YAML file by name; the loader must still treat
    // it as the base layer.
    let mut pack = ContentPack::new(
        PackMeta {
            id: "order-pack".to_string(),
            name: None,
            edition: "2014".to_string(),
            source: None,
            license: None,
            distribution: Distribution::Local,
            schema_version: SCHEMA_VERSION,
            content_hash: None,
            created_at: None,
            description: None,
        },
        serde_json::from_value(json!({"conditions": [{"id": "dup", "name": "From Pack"}]}))
            .expect("content"),
    )
    .expect("pack");
    pack.index = PackIndex::default();
    fs::write(
        dir.path().join("z.pack.json"),
        serde_json::to_string_pretty(&pack).expect("json"),
    )
    .expect("write pack");
    fs::write(
        dir.path().join("a.yaml"),
        "conditions:\n  - id: dup\n    name: From YAML\n",
    )
    .expect("write yaml");

    let registry = load(dir.path());
    let condition = registry.conditions.get("dup").expect("condition");
    assert_eq!(condition.name.as_deref(), Some("From YAML"));
    let source = registry
        .source_of(ItemKind::Condition, "dup")
        .expect("source");
    assert!(source.path.ends_with("a.yaml"), "{}", source.path.display());
}

const COMMITTED_2014: &str = "../../LetsPlay/Rules/combat/2014/srd";
const COMMITTED_2024: &str = "../../LetsPlay/Rules/combat/2024/srd";

fn committed_dir(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn committed_packs(relative: &str) -> Vec<ContentPack> {
    let dir = committed_dir(relative);
    let mut packs = Vec::new();
    for entry in fs::read_dir(&dir).expect("committed pack dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            packs.push(read_pack(&path));
        }
    }
    packs.sort_by(|a, b| a.pack.id.cmp(&b.pack.id));
    assert!(!packs.is_empty(), "no packs in {}", dir.display());
    packs
}

fn pack_by_id<'a>(packs: &'a [ContentPack], id: &str) -> &'a ContentPack {
    packs
        .iter()
        .find(|pack| pack.pack.id == id)
        .unwrap_or_else(|| panic!("pack {id} missing"))
}

#[test]
fn committed_2014_packs_are_complete() {
    let packs = committed_packs(COMMITTED_2014);
    assert_eq!(
        packs.len(),
        8,
        "{:?}",
        packs.iter().map(|p| &p.pack.id).collect::<Vec<_>>()
    );
    let expected = [
        ("srd-5.1-monsters", 334),
        ("srd-5.1-spells", 319),
        ("srd-5.1-classes", 443),
        ("srd-5.1-species", 51),
        ("srd-5.1-backgrounds", 1),
        ("srd-5.1-feats", 1),
        ("srd-5.1-items", 599),
        ("srd-5.1-rules", 39),
    ];
    for (id, count) in expected {
        let pack = pack_by_id(&packs, id);
        assert_eq!(pack.entity_count(), count, "{id}");
        assert_eq!(pack.pack.edition, "2014", "{id}");
        assert_eq!(pack.pack.distribution, Distribution::Srd, "{id}");
        assert_eq!(pack.pack.license.as_deref(), Some("CC-BY-4.0"), "{id}");
    }
    let monsters = pack_by_id(&packs, "srd-5.1-monsters");
    for creature in monsters.content.creatures.as_ref().expect("creatures") {
        assert!(
            creature
                .manual
                .as_deref()
                .is_some_and(|text| !text.is_empty()),
            "{} has no statblock text",
            creature.id
        );
    }
    let spells = pack_by_id(&packs, "srd-5.1-spells");
    for spell in spells.content.abilities.as_ref().expect("spells") {
        assert!(
            spell.manual.as_deref().is_some_and(|text| !text.is_empty()),
            "{} has no spell text",
            spell.id
        );
    }
}

#[test]
fn committed_2024_packs_are_complete() {
    let packs = committed_packs(COMMITTED_2024);
    // Seven 5e-database packs plus the pdf2md rules-prose pack.
    assert_eq!(packs.len(), 8);
    let expected = [
        ("srd-5.2.1-monsters", 341),
        ("srd-5.2.1-spells", 339),
        ("srd-5.2.1-classes", 268),
        ("srd-5.2.1-species", 100),
        ("srd-5.2.1-backgrounds", 4),
        ("srd-5.2.1-feats", 17),
        ("srd-5.2.1-items", 457),
    ];
    for (id, count) in expected {
        let pack = pack_by_id(&packs, id);
        assert_eq!(pack.entity_count(), count, "{id}");
        assert_eq!(pack.pack.edition, "2024", "{id}");
    }
    assert!(packs.iter().all(|pack| pack.pack.id != "srd-5.2.1-rules"));
    assert!(pack_by_id(&packs, "srd-5.2.1-rules-prose").entity_count() > 0);
}

#[test]
fn committed_packs_load_as_a_library() {
    for relative in [COMMITTED_2014, COMMITTED_2024] {
        let dir = committed_dir(relative);
        let registry = load(&dir);
        assert!(registry.packs.contains_key(if relative.contains("2014") {
            "srd-5.1-monsters"
        } else {
            "srd-5.2.1-monsters"
        }));
        assert!(registry.find_any("fireball").is_some());
        assert!(registry.find_any("fighter").is_some());
        assert!(registry.find_any("longsword").is_some());
    }
}
