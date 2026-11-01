//! Smoke test against the real TalkDND campaign when it is present.
//!
//! The test is intentionally skipped (not failed) when the save directory is
//! absent, so the repository remains self-contained.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use combat_sim::ai::Ai;
use combat_sim::content::schema::{Ability, AbilityName, AreaSpec, Creature, Direction, Effect};
use combat_sim::content::{self, ContentSources, ItemKind};
use combat_sim::dice;
use combat_sim::engine::NullSink;
use combat_sim::sim;
use combat_sim::space::AreaShape;
use combat_sim::state::{EncounterState, NewEncounter};

const MANIFEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../LetsPlay/Saves/2026-10-19-01/combat/campaign.yaml"
);
const RULES_2014: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/rulesets/real-2014.yaml"
);
const RULES_2024: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/rulesets/real-2024.yaml"
);
const PACK_2024_MONSTERS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../LetsPlay/Rules/combat/2024/srd/srd-5.2.1-monsters.pack.json"
);
const PACK_2024_SPELLS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../LetsPlay/Rules/combat/2024/srd/srd-5.2.1-spells.pack.json"
);
const PARTY: &[&str] = &["will", "rina", "milo", "sera"];

fn pack_burrow(notes: &str) -> i32 {
    notes
        .split("burrow speed ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|feet| feet.parse().ok())
        .unwrap_or(0)
}

fn load_ruleset(manifest: &str) -> content::ContentRegistry {
    let sources = ContentSources {
        campaign: Some(manifest.into()),
        ..ContentSources::default()
    };
    sources.load().expect("SRD content should load")
}

fn area<'a>(registry: &'a content::ContentRegistry, id: &str) -> &'a content::schema::AreaSpec {
    registry
        .abilities
        .get(id)
        .unwrap_or_else(|| panic!("missing ability `{id}`"))
        .area
        .as_ref()
        .unwrap_or_else(|| panic!("ability `{id}` has no area"))
}

#[test]
fn srd_2014_structures_hover_and_burrow() {
    let registry = load_ruleset(RULES_2014);
    let hover: Vec<&str> = registry
        .creatures
        .iter()
        .filter(|(_, creature)| creature.speed().hover)
        .map(|(id, _)| id)
        .collect();
    let burrow: Vec<&str> = registry
        .creatures
        .iter()
        .filter(|(_, creature)| creature.speed().burrow > 0)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(hover.len(), 7, "hover ids: {hover:?}");
    assert_eq!(burrow.len(), 21, "burrow ids: {burrow:?}");
    assert!(registry.creatures.get("specter").unwrap().speed().hover);
    assert_eq!(registry.creatures.get("xorn").unwrap().speed().burrow, 20);
    assert_eq!(
        registry
            .creatures
            .get("purple-worm")
            .unwrap()
            .speed()
            .burrow,
        30
    );
}

#[test]
fn srd_2024_spatial_overlay_structures_hover_and_burrow() {
    let registry = load_ruleset(RULES_2024);
    let spatial: Vec<&str> = registry
        .creatures
        .iter()
        .filter(|(_, creature)| creature.speed().hover || creature.speed().burrow > 0)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(spatial.len(), 38, "spatial ids: {spatial:?}");
    assert!(registry.creatures.get("specter").unwrap().speed().hover);
    assert!(
        registry
            .creatures
            .get("air-elemental")
            .unwrap()
            .speed()
            .hover
    );
    assert_eq!(
        registry
            .creatures
            .get("purple-worm")
            .unwrap()
            .speed()
            .burrow,
        50
    );
    assert_eq!(registry.creatures.get("xorn").unwrap().speed().burrow, 20);
}

#[test]
fn srd_area_shapes_cover_the_frozen_shape_set() {
    let y2014 = load_ruleset(RULES_2014);
    let shapes: HashSet<AreaShape> = y2014
        .abilities
        .iter()
        .filter_map(|(_, ability)| ability.area.as_ref().and_then(|area| area.shape))
        .collect();
    for shape in [
        AreaShape::Sphere,
        AreaShape::Cone,
        AreaShape::Cube,
        AreaShape::Cylinder,
        AreaShape::Line,
    ] {
        assert!(shapes.contains(&shape), "2014 lacks {shape:?}");
    }
    let fireball = area(&y2014, "fireball");
    assert_eq!(fireball.shape, Some(AreaShape::Sphere));
    assert_eq!(fireball.radius, 20);
    assert_eq!(fireball.width_ft, None);
    let burning_hands = area(&y2014, "burning-hands");
    assert_eq!(burning_hands.shape, Some(AreaShape::Cone));
    assert_eq!(burning_hands.radius, 15);
    let thunderwave = area(&y2014, "thunderwave");
    assert_eq!(thunderwave.shape, Some(AreaShape::Cube));
    assert_eq!(thunderwave.radius, 15);
    let moonbeam = area(&y2014, "moonbeam");
    assert_eq!(moonbeam.shape, Some(AreaShape::Cylinder));
    assert_eq!(moonbeam.radius, 5);
    let lightning = area(&y2014, "lightning-bolt");
    assert_eq!(lightning.shape, Some(AreaShape::Line));
    assert_eq!(lightning.radius, 100);
    assert_eq!(lightning.width_ft, Some(5));

    let y2024 = load_ruleset(RULES_2024);
    let shapes: HashSet<AreaShape> = y2024
        .abilities
        .iter()
        .filter_map(|(_, ability)| ability.area.as_ref().and_then(|area| area.shape))
        .collect();
    for shape in [
        AreaShape::Sphere,
        AreaShape::Cone,
        AreaShape::Cube,
        AreaShape::Cylinder,
        AreaShape::Line,
        AreaShape::Emanation,
    ] {
        assert!(shapes.contains(&shape), "2024 lacks {shape:?}");
    }
    let emanation = area(&y2024, "spirit-guardians");
    assert_eq!(emanation.shape, Some(AreaShape::Emanation));
    assert_eq!(emanation.radius, 15);
}

#[test]
fn srd_2024_spatial_monster_overlay_matches_the_pack() {
    let registry = load_ruleset(RULES_2024);
    let pack: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(PACK_2024_MONSTERS).expect("pack readable"))
            .expect("pack json");
    let mut checked = 0;
    for entry in pack["content"]["creatures"]
        .as_array()
        .expect("creatures array")
    {
        let notes = entry["notes"].as_str().unwrap_or("");
        let hover = notes.contains("hover");
        let burrow = pack_burrow(notes);
        if !hover && burrow == 0 {
            continue;
        }
        let id = entry["id"].as_str().expect("creature id");
        let actual = registry
            .creatures
            .get(id)
            .unwrap_or_else(|| panic!("missing overlay creature `{id}`"));
        let mut expected: Creature =
            serde_json::from_value(entry.clone()).expect("pack creature parses");
        let mut speed = expected.speed.clone().unwrap_or_default();
        speed.hover = hover;
        speed.burrow = burrow;
        expected.speed = Some(speed);
        assert_eq!(actual, &expected, "overlay mismatch for `{id}`");
        checked += 1;
    }
    assert_eq!(checked, 38);
}

#[test]
fn srd_2024_area_overlays_match_the_pack() {
    let registry = load_ruleset(RULES_2024);
    let pack: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(PACK_2024_SPELLS).expect("pack readable"))
            .expect("pack json");
    let expected_areas = [
        (
            "burning-hands",
            AreaSpec {
                radius: 15,
                shape: Some(AreaShape::Cone),
                width_ft: None,
            },
        ),
        (
            "fireball",
            AreaSpec {
                radius: 20,
                shape: Some(AreaShape::Sphere),
                width_ft: None,
            },
        ),
        (
            "lightning-bolt",
            AreaSpec {
                radius: 100,
                shape: Some(AreaShape::Line),
                width_ft: Some(5),
            },
        ),
        (
            "moonbeam",
            AreaSpec {
                radius: 5,
                shape: Some(AreaShape::Cylinder),
                width_ft: None,
            },
        ),
        (
            "spirit-guardians",
            AreaSpec {
                radius: 15,
                shape: Some(AreaShape::Emanation),
                width_ft: None,
            },
        ),
        (
            "thunderwave",
            AreaSpec {
                radius: 15,
                shape: Some(AreaShape::Cube),
                width_ft: None,
            },
        ),
    ];
    for (id, area) in expected_areas {
        let entry = pack["content"]["abilities"]
            .as_array()
            .expect("abilities array")
            .iter()
            .find(|entry| entry["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("missing pack spell `{id}`"));
        // The overlay contract: the pack entry plus the frozen area. The
        // registry view is compared field by field against that baseline so
        // no other field (name, tags, activation, cost, slot_level, range,
        // target, concentration, duration, triggers, modifiers, ai, manual,
        // source, notes, description) can drift from the pack unnoticed.
        let mut expected: Ability =
            serde_json::from_value(entry.clone()).expect("pack spell parses");
        assert_eq!(expected.id, id, "pack spell id");
        expected.area = Some(area);
        let mut actual: Ability = registry
            .abilities
            .get(id)
            .unwrap_or_else(|| panic!("missing overlay spell `{id}`"))
            .clone();

        // TASK-0127 added `spell` metadata to every handwritten spell entry
        // (asserted in detail by spell_data.rs). Its presence is asserted
        // separately and it is normalized on both sides so the comparison
        // below keeps guarding every other field.
        assert!(
            actual.spell.is_some(),
            "`{id}` must keep its spell metadata"
        );
        actual.spell = None;
        expected.spell = None;

        // TASK-0127 also re-expressed the four one-shot saves as one
        // structured save instead of the pack's manual note: a Dexterity or
        // Constitution save with `half` whose failed save deals the SRD
        // damage (thunderwave also pushes). Assert that exact shape, then
        // normalize both sides. The two concentration spells keep the pack's
        // manual note; their effects stay compared verbatim below.
        match id {
            "burning-hands" => assert_save_half(&actual, AbilityName::Dex, "3d6", "fire", None),
            "fireball" => assert_save_half(&actual, AbilityName::Dex, "8d6", "fire", None),
            "lightning-bolt" => {
                assert_save_half(&actual, AbilityName::Dex, "8d6", "lightning", None)
            }
            "thunderwave" => {
                assert_save_half(&actual, AbilityName::Con, "2d8", "thunder", Some(10))
            }
            // The concentration spells additionally recorded their
            // persistent-area upcast limitation in `todo`; assert it, then
            // normalize both sides.
            "moonbeam" => {
                assert_eq!(
                    actual.todo,
                    ["Persistent area: initial and recurring CON saves are manual; upcast (+1d10 per slot above 2) has no structured main effect, so no dice table (higher slots keep base effects)"],
                    "moonbeam todo"
                );
                actual.todo.clear();
                expected.todo.clear();
            }
            "spirit-guardians" => {
                assert_eq!(
                    actual.todo,
                    ["Persistent area: recurring WIS saves are manual; upcast (+1d8 per slot above 3) has no structured main effect, so no dice table (higher slots keep base effects)"],
                    "spirit-guardians todo"
                );
                actual.todo.clear();
                expected.todo.clear();
            }
            _ => unreachable!("unexpected overlay spell `{id}`"),
        }
        if matches!(
            id,
            "burning-hands" | "fireball" | "lightning-bolt" | "thunderwave"
        ) {
            actual.effects.clear();
            expected.effects.clear();
        }

        assert_eq!(actual, expected, "overlay mismatch for `{id}`");
    }
}

/// Assert the structured one-shot save shape of a TASK-0127 AoE spell: a
/// single save effect with the given ability and the shared `{spell_dc}`
/// placeholder, half damage on a success, and an `on_fail` that opens with
/// the given damage (plus one optional push away).
fn assert_save_half(
    ability: &Ability,
    save_ability: AbilityName,
    dice: &str,
    damage_type: &str,
    push_ft: Option<i32>,
) {
    let [Effect::Save(save)] = ability.effects.as_slice() else {
        panic!("`{}` must be one structured save", ability.id);
    };
    assert_eq!(save.ability, save_ability, "`{}` save ability", ability.id);
    assert_eq!(save.dc.as_text(), "{spell_dc}", "`{}` save dc", ability.id);
    assert!(
        save.half,
        "`{}` must take half damage on a success",
        ability.id
    );
    assert!(!save.ignore_cover, "`{}` ignore_cover", ability.id);
    let mut on_fail = save.on_fail.iter();
    let Effect::Damage(damage) = on_fail
        .next()
        .unwrap_or_else(|| panic!("`{}` on_fail must open with damage", ability.id))
    else {
        panic!("`{}` on_fail must open with damage", ability.id);
    };
    assert_eq!(damage.dice.as_text(), dice, "`{}` damage", ability.id);
    assert_eq!(
        damage.damage_type.as_str(),
        damage_type,
        "`{}` damage type",
        ability.id
    );
    if let Some(distance) = push_ft {
        assert!(
            matches!(
                on_fail.next(),
                Some(Effect::ForcedMove(push))
                    if push.distance == distance && push.direction == Direction::Away
            ),
            "`{}` must push {distance} ft away",
            ability.id
        );
    }
    assert_eq!(
        on_fail.next(),
        None,
        "`{}` on_fail has unexpected extras",
        ability.id
    );
    assert!(
        save.on_success.is_empty(),
        "`{}` half damage is modelled by `half`, not on_success",
        ability.id
    );
}

#[test]
fn real_content_checks_clean_for_both_rulesets() {
    for manifest in [RULES_2014, RULES_2024] {
        let registry = load_ruleset(manifest);
        let report = content::check(&registry);
        assert!(
            !report.has_errors(),
            "{manifest}: {} errors",
            report.error_count()
        );
    }
}

#[test]
fn handwritten_creature_ids_keep_their_semantics() {
    let y2014 = load_ruleset(RULES_2014);
    let guard = y2014.creatures.get("guard").expect("2014 guard");
    assert_eq!(guard.ac(), 16);
    assert_eq!(guard.speed().walk, 30);
    let source = y2014
        .source_of(ItemKind::Creature, "guard")
        .expect("2014 guard source")
        .to_string()
        .replace('\\', "/");
    assert!(
        source.ends_with("LetsPlay/Rules/combat/2014/monsters/guard.yaml"),
        "unexpected source: {source}"
    );

    let y2024 = load_ruleset(RULES_2024);
    let guard = y2024.creatures.get("guard").expect("2024 guard");
    assert_eq!(guard.ac(), 16);
    assert_eq!(guard.speed().walk, 30);
    let source = y2024
        .source_of(ItemKind::Creature, "guard")
        .expect("2024 guard source")
        .to_string()
        .replace('\\', "/");
    assert!(
        source.ends_with("LetsPlay/Rules/combat/2024/monsters/guard.yaml"),
        "unexpected source: {source}"
    );
}

#[test]
fn real_campaign_loads_checks_and_simulates_once() {
    if !Path::new(MANIFEST).exists() {
        eprintln!(
            "skipping real_campaign_loads_checks_and_simulates_once: \
             LetsPlay/Saves/2026-10-19-01/combat/campaign.yaml is absent"
        );
        return;
    }
    let sources = ContentSources {
        campaign: Some(MANIFEST.into()),
        ..ContentSources::default()
    };
    let registry = sources.load().expect("real campaign content should load");
    let report = content::check(&registry);
    assert!(
        !report.has_errors(),
        "real content check reported {} errors",
        report.error_count()
    );
    let content = Arc::new(registry);
    let mut encounters: Vec<String> = content.encounters.ids().map(ToString::to_string).collect();
    encounters.sort();
    assert!(!encounters.is_empty(), "real campaign has no encounters");
    for encounter in encounters {
        let options = NewEncounter {
            encounter: encounter.clone(),
            party: PARTY.iter().map(ToString::to_string).collect(),
            party_zone: None,
            adds: Vec::new(),
        };
        let state = EncounterState::from_encounter_shared(content.clone(), &options)
            .unwrap_or_else(|error| panic!("failed to build `{encounter}`: {error:#}"));
        let ai = Ai::prepare(&state);
        let mut rng = dice::seeded_rng(20261019);
        sim::play(state, &ai, &mut rng, 100, &mut NullSink)
            .unwrap_or_else(|error| panic!("simulation of `{encounter}` failed: {error:#}"));
    }
}
