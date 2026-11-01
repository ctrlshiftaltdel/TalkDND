//! v1.6 T4 spell-cast settlement tests (TASK-0126 / Issue #248).
//!
//! Synthetic content only: generic names, no campaign data and no book text.
//! Every scenario drives the one spell executor through `engine::use_ability`
//! (CLI cast/use both route here), covering slot payment, upcast tables,
//! cantrip scaling, pact slots, Mystic Arcanum, material declarations and
//! long casting times. Abilities without spell metadata assert the old
//! behavior stays byte-identical.

use std::collections::VecDeque;

use combat_sim::content::schema::{Ability, Creature, Encounter, Scene};
use combat_sim::content::{ContentRegistry, ContentSource};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{self, ReadyOptions, UseOptions};
use combat_sim::state::{AddSpec, EncounterState, NewEncounter, ReadyResponse, ReadyTrigger};
use serde_json::{json, Value};

fn source() -> ContentSource {
    ContentSource::new("synthetic-t4-spells", None)
}

fn from_value<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("synthetic spell content should deserialize")
}

fn insert_ability(registry: &mut ContentRegistry, value: Value) {
    let ability: Ability = from_value(value);
    let id = ability.id.clone();
    registry.abilities.insert(id, ability, source());
}

fn insert_creature(registry: &mut ContentRegistry, value: Value) {
    let creature: Creature = from_value(value);
    let id = creature.id.clone();
    registry.creatures.insert(id, creature, source());
}

fn registry() -> ContentRegistry {
    let mut registry = ContentRegistry {
        ruleset: "2024".to_string(),
        ..ContentRegistry::default()
    };
    let scene: Scene = from_value(json!({
        "id": "arena",
        "name": "Spell Arena",
        "mode": "zones",
        "zones": [{"id": "alpha"}, {"id": "beta"}],
        "edges": [{"a": "alpha", "b": "beta", "distance": 10}]
    }));
    registry.scenes.insert(scene.id.clone(), scene, source());
    for ability in spell_abilities() {
        insert_ability(&mut registry, ability);
    }
    for creature in spell_creatures() {
        insert_creature(&mut registry, creature);
    }
    let encounter: Encounter = from_value(json!({
        "id": "spell-fight",
        "name": "Spell Fight",
        "scene": "arena",
        "party_zone": "alpha",
        "members": [{"creature": "brute-b", "zone": "beta"}]
    }));
    registry
        .encounters
        .insert(encounter.id.clone(), encounter, source());
    registry
}

fn enemy_target() -> Value {
    json!({"type": "creature", "side": "enemy", "count": 1})
}

fn self_target() -> Value {
    json!({"type": "self"})
}

fn ally_target() -> Value {
    json!({"type": "creature", "side": "ally", "count": 1})
}

fn attack_damage(dice: &str, damage_type: &str) -> Value {
    json!({"attack": {
        "to_hit": 5,
        "kind": "ranged",
        "damage": [{"dice": dice, "type": damage_type}]
    }})
}

fn spell_abilities() -> Vec<Value> {
    vec![
        // 1st-level attack spell with an upcast table (2/3/5; 4 is missing on
        // purpose so the fail-closed path can be tested with --slot 4).
        json!({
            "id": "test-bolt", "name": "Test Bolt", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "range": 60,
            "target": enemy_target(),
            "effects": [attack_damage("1d6", "fire")],
            "spell": {
                "level": 1, "school": "evocation",
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true, "somatic": true},
                "ritual": false,
                "upcast": {
                    "text": "higher",
                    "dice_by_level": {"2": "2d6", "3": "3d6", "5": "5d6"}
                }
            }
        }),
        // 3rd-level save spell; the on-fail damage is the main dice.
        json!({
            "id": "test-blast", "name": "Test Blast", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-3", "amount": 1},
            "slot_level": 3,
            "range": 60,
            "target": enemy_target(),
            "effects": [{"save": {
                "ability": "dex", "dc": 12, "half": true,
                "on_fail": [{"damage": {"dice": "1d6", "type": "fire"}}]
            }}],
            "spell": {
                "level": 3,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true},
                "upcast": {
                    "text": "higher",
                    "dice_by_level": {"4": "2d6", "5": "3d6"}
                }
            }
        }),
        // 1st-level heal spell.
        json!({
            "id": "test-mend", "name": "Test Mend", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "range": 60,
            "target": ally_target(),
            "effects": [{"heal": {"dice": "1d4+1"}}],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true},
                "upcast": {"text": "higher", "dice_by_level": {"2": "3d4+1"}}
            }
        }),
        // 1st-level attack spell without an upcast spec.
        json!({
            "id": "test-mark", "name": "Test Mark", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "range": 60,
            "target": enemy_target(),
            "effects": [attack_damage("1d6", "force")],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true},
            }
        }),
        // Damage cantrip with character-level growth.
        json!({
            "id": "test-spark", "name": "Test Spark", "tags": ["spell", "cantrip"],
            "activation": "action",
            "range": 60,
            "target": enemy_target(),
            "effects": [attack_damage("1d6", "fire")],
            "spell": {
                "level": 0,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true, "somatic": true},
                "cantrip_scaling": {
                    "text": "growth",
                    "dice_by_character_level": {"5": "2d6", "11": "3d6", "17": "4d6"}
                }
            }
        }),
        // Damage cantrip without a scaling table.
        json!({
            "id": "plain-spark", "name": "Plain Spark", "tags": ["spell", "cantrip"],
            "activation": "action",
            "range": 60,
            "target": enemy_target(),
            "effects": [attack_damage("1d6", "fire")],
            "spell": {
                "level": 0,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true},
            }
        }),
        // Long casting times (D-11: fail closed inside combat).
        json!({
            "id": "slow-ritual", "name": "Slow Ritual", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "target": self_target(),
            "effects": [{"note": "long ritual"}],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "minute"},
                "components": {"verbal": true},
            }
        }),
        json!({
            "id": "hour-ward", "name": "Hour Ward", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "target": self_target(),
            "effects": [{"note": "hour cast"}],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "hour"},
                "components": {"verbal": true},
            }
        }),
        // Bonus-action casting time stays a normal cast.
        json!({
            "id": "quick-word", "name": "Quick Word", "tags": ["spell"],
            "activation": "bonus",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "target": self_target(),
            "effects": [{"note": "quick"}],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "bonus_action"},
                "components": {"verbal": true},
            }
        }),
        // Material components: costly, consumed, cheap.
        json!({
            "id": "gem-cast", "name": "Gem Cast", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "target": self_target(),
            "effects": [{"note": "gem"}],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {
                    "verbal": true,
                    "material": {"text": "a diamond worth 50 gp", "cost_gp": 50},
                },
            }
        }),
        json!({
            "id": "dust-cast", "name": "Dust Cast", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "target": self_target(),
            "effects": [{"note": "dust"}],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {
                    "verbal": true,
                    "material": {"text": "powdered iron", "consumed": true},
                },
            }
        }),
        json!({
            "id": "free-cast", "name": "Free Cast", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "target": self_target(),
            "effects": [{"note": "fleece"}],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {
                    "verbal": true,
                    "material": {"text": "a bit of fleece"},
                },
            }
        }),
        // Concentration spell for the upcast concentration check.
        json!({
            "id": "test-hold", "name": "Test Hold", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "concentration": true,
            "target": self_target(),
            "effects": [{"note": "hold"}],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true},
            }
        }),
        // Old-style ability without spell metadata: legacy behavior.
        json!({
            "id": "no-meta-bolt", "name": "No Meta Bolt",
            "activation": "action",
            "cost": {"resource": "spell-slot-2", "amount": 1},
            "slot_level": 2,
            "range": 60,
            "target": enemy_target(),
            "effects": [attack_damage("1d8", "force")],
        }),
        // Pact spell: the base ability charges spell-slot-1; the warlock
        // sheet ref overrides it to spell-slot-5 (the T3 spells sync shape).
        json!({
            "id": "pact-bolt", "name": "Pact Bolt", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "range": 60,
            "target": enemy_target(),
            "effects": [attack_damage("1d6", "force")],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true},
                "upcast": {"text": "higher", "dice_by_level": {"2": "2d6", "5": "4d6"}}
            }
        }),
        // Mystic Arcanum spell: base ability charges spell-slot-6; the ref
        // override charges arcanum-6. The upcast table must stay unused.
        json!({
            "id": "arcanum-word", "name": "Arcanum Word", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-6", "amount": 1},
            "slot_level": 6,
            "range": 60,
            "target": enemy_target(),
            "effects": [attack_damage("1d6", "necrotic")],
            "spell": {
                "level": 6,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true},
                "upcast": {"text": "arcanum never adds dice", "dice_by_level": {"6": "5d6"}}
            }
        }),
        // Upcast table but note-only effects: the dice have no main
        // damage/heal to land on, so the cast must fail closed before the
        // action, slot or concentration are touched.
        json!({
            "id": "note-veil", "name": "Note Veil", "tags": ["spell"],
            "activation": "action",
            "cost": {"resource": "spell-slot-1", "amount": 1},
            "slot_level": 1,
            "concentration": true,
            "target": self_target(),
            "effects": [{"note": "a veil of pure text"}],
            "spell": {
                "level": 1,
                "casting_time": {"amount": 1, "unit": "action"},
                "components": {"verbal": true},
                "upcast": {"text": "higher", "dice_by_level": {"2": "2d6"}}
            }
        }),
    ]
}

fn spell_creatures() -> Vec<Value> {
    vec![
        json!({
            "id": "caster-x", "name": "Caster X", "kind": "pc", "side": "party",
            "level": 5,
            "ac": 12, "hp": 30, "speed": {"walk": 30}, "init_bonus": 5, "prof": 2,
            "abilities": {"str": 8, "dex": 12, "con": 14, "int": 16, "wis": 10, "cha": 10},
            "resources": {
                "spell-slot-1": {"max": 4, "recharge": "long"},
                "spell-slot-2": {"max": 3, "recharge": "long"},
                "spell-slot-3": {"max": 2, "recharge": "long"},
                "spell-slot-5": {"max": 1, "recharge": "long"},
            },
            "actions": [
                {"ref": "test-bolt"}, {"ref": "test-blast"}, {"ref": "test-mend"},
                {"ref": "test-mark"}, {"ref": "test-spark"}, {"ref": "plain-spark"},
                {"ref": "slow-ritual"}, {"ref": "hour-ward"}, {"ref": "quick-word"},
                {"ref": "gem-cast"}, {"ref": "dust-cast"}, {"ref": "free-cast"},
                {"ref": "test-hold"}, {"ref": "no-meta-bolt"},
                {"ref": "note-veil"},
            ],
        }),
        json!({
            "id": "low-caster", "name": "Low Caster", "kind": "pc", "side": "party",
            "level": 3,
            "ac": 12, "hp": 30, "speed": {"walk": 30}, "init_bonus": 5, "prof": 2,
            "abilities": {"str": 8, "dex": 12, "con": 14, "int": 16, "wis": 10, "cha": 10},
            "actions": [{"ref": "test-spark"}, {"ref": "plain-spark"}],
        }),
        json!({
            "id": "warlock-x", "name": "Pact Caster", "kind": "pc", "side": "party",
            "level": 5,
            "ac": 12, "hp": 30, "speed": {"walk": 30}, "init_bonus": 5, "prof": 2,
            "abilities": {"str": 8, "dex": 12, "con": 14, "int": 10, "wis": 10, "cha": 16},
            "resources": {
                "spell-slot-5": {"max": 2, "recharge": "short"},
                "arcanum-6": {"max": 1, "recharge": "long"},
            },
            "actions": [
                {"ref": "pact-bolt", "cost": {"resource": "spell-slot-5", "amount": 1}},
                {"ref": "arcanum-word", "cost": {"resource": "arcanum-6", "amount": 1}},
            ],
        }),
        json!({
            "id": "ally-a", "name": "Ally A", "kind": "pc", "side": "party",
            "ac": 14, "hp": 40, "speed": {"walk": 30}, "init_bonus": 0, "prof": 2,
            "abilities": {"str": 10, "dex": 10, "con": 10, "int": 10, "wis": 10, "cha": 10},
        }),
        json!({
            "id": "brute-b", "name": "Brute B", "kind": "monster", "side": "enemy",
            "ac": 10, "hp": 100, "speed": {"walk": 30}, "init_bonus": 0, "prof": 2,
            "abilities": {"str": 16, "dex": 10, "con": 14, "int": 8, "wis": 10, "cha": 8},
        }),
    ]
}

fn encounter_state(party: &[&str]) -> EncounterState {
    let options = NewEncounter {
        encounter: "spell-fight".to_string(),
        party: party.iter().map(|name| name.to_string()).collect(),
        party_zone: None,
        adds: Vec::<AddSpec>::new(),
    };
    EncounterState::from_encounter(registry(), &options)
        .expect("spell fixture encounter should build")
}

#[derive(Debug, Default)]
struct ScriptedRng {
    values: VecDeque<u32>,
}

impl ScriptedRng {
    fn new(values: &[u32]) -> Self {
        Self {
            values: values.iter().copied().collect(),
        }
    }
}

impl DiceRng for ScriptedRng {
    fn roll_die(&mut self, sides: u32) -> u32 {
        self.values
            .pop_front()
            .unwrap_or_else(|| panic!("scripted RNG ran out while rolling d{sides}"))
    }
}

fn index(state: &EncounterState, name: &str) -> usize {
    state.find_index(name).expect("combatant exists")
}

fn resource(state: &EncounterState, name: &str, id: &str) -> i32 {
    state.combatants[index(state, name)]
        .resource(id)
        .unwrap_or_else(|| panic!("missing resource {id}"))
        .current
}

fn cast_options(
    target: &str,
    slot: Option<i32>,
    material: Option<&str>,
    ignore_components: bool,
) -> UseOptions {
    UseOptions {
        targets: vec![target.to_string()],
        slot_level: slot,
        material: material.map(|text| text.to_string()),
        ignore_components,
        ..UseOptions::default()
    }
}

#[test]
fn upcast_spends_the_requested_slot_and_replaces_attack_dice() {
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "test-bolt",
        &cast_options("Brute B", Some(2), None, false),
        &mut ScriptedRng::new(&[15, 4, 5]),
        &mut log,
    )
    .unwrap();
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-1"),
        4,
        "base slot untouched"
    );
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-2"),
        2,
        "slot 2 paid"
    );
    assert!(
        log.iter()
            .any(|line| line.contains("spends 1 spell-slot-2")),
        "{log:?}"
    );
    assert!(log.iter().any(|line| line.contains("2d6(4,5)")), "{log:?}");
    assert!(!log.iter().any(|line| line.contains("1d6(")), "{log:?}");
    assert_eq!(
        state.combatants[index(&state, "Brute B")].hp,
        100 - 9,
        "upcast dice deal the damage"
    );
    // The slot context follows the cast: the effect source carries level 2.
    assert!(
        !log.iter().any(|line| line.contains("UPCAST NOT MODELED")),
        "{log:?}"
    );
}

#[test]
fn upcast_replaces_save_damage_dice() {
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "test-blast",
        &cast_options("Brute B", Some(5), None, false),
        &mut ScriptedRng::new(&[2, 3, 4, 5]),
        &mut log,
    )
    .unwrap();
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-5"),
        0,
        "slot 5 paid"
    );
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-3"),
        2,
        "base slot untouched"
    );
    assert!(
        log.iter().any(|line| line.contains("3d6(3,4,5)")),
        "{log:?}"
    );
    assert_eq!(state.combatants[index(&state, "Brute B")].hp, 100 - 12);
}

#[test]
fn upcast_replaces_heal_dice() {
    let mut state = encounter_state(&["caster-x", "ally-a"]);
    let mut log = Vec::new();
    engine::damage(
        &mut state,
        "Ally A",
        "20",
        None,
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    engine::use_ability(
        &mut state,
        "Caster X",
        "test-mend",
        &cast_options("Ally A", Some(2), None, false),
        &mut ScriptedRng::new(&[3, 4, 1]),
        &mut log,
    )
    .unwrap();
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-2"),
        2,
        "slot 2 paid"
    );
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-1"),
        4,
        "base slot untouched"
    );
    assert!(log.iter().any(|line| line.contains("3d4")), "{log:?}");
    assert_eq!(
        state.combatants[index(&state, "Ally A")].hp,
        20 + (3 + 4 + 1 + 1)
    );
}

#[test]
fn upcast_without_table_entry_fails_closed() {
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    let error = engine::use_ability(
        &mut state,
        "Caster X",
        "test-blast",
        &cast_options("Brute B", Some(6), None, false),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no upcast entry for slot level 6"),
        "{error}"
    );
    // Fail closed: nothing was spent and the target is unharmed.
    assert_eq!(resource(&state, "Caster X", "spell-slot-3"), 2);
    assert_eq!(state.combatants[index(&state, "Brute B")].hp, 100);
}

#[test]
fn upcast_without_main_damage_or_heal_fails_before_spending() {
    // The spell has an upcast table but note-only effects, so the dice have
    // no main damage/heal to land on. The failure must happen in the plan
    // phase: action, slot and concentration all stay untouched (round-1
    // review fix; note-veil is concentration so the old order would have
    // started concentration before bailing).
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    let error = engine::use_ability(
        &mut state,
        "Caster X",
        "note-veil",
        &cast_options("Caster X", Some(2), None, false),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no main damage or heal effect to scale"),
        "{error}"
    );
    assert!(
        !state.combatants[index(&state, "Caster X")].action_used,
        "the action was not spent"
    );
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-1"),
        4,
        "base slot untouched"
    );
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-2"),
        3,
        "no slot spent"
    );
    assert!(
        !state.concentration.contains_key("Caster X"),
        "concentration untouched"
    );
    assert!(
        !log.iter().any(|line| line.contains("MANUAL")),
        "no manual note: {log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("uses Note Veil")),
        "no effect settlement: {log:?}"
    );
}

#[test]
fn slot_below_base_level_is_rejected() {
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    let error = engine::use_ability(
        &mut state,
        "Caster X",
        "test-blast",
        &cast_options("Brute B", Some(2), None, false),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("below the base level 3"),
        "{error}"
    );
    assert_eq!(resource(&state, "Caster X", "spell-slot-2"), 3);
}

#[test]
fn slot_above_nine_is_rejected() {
    let mut state = encounter_state(&["caster-x"]);
    let error = engine::use_ability(
        &mut state,
        "Caster X",
        "test-bolt",
        &cast_options("Brute B", Some(10), None, false),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("exceeds the highest spell slot level"),
        "{error}"
    );
}

#[test]
fn cast_without_upcast_spec_allows_base_effects_and_logs() {
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "test-mark",
        &cast_options("Brute B", Some(2), None, false),
        &mut ScriptedRng::new(&[15, 3]),
        &mut log,
    )
    .unwrap();
    assert_eq!(resource(&state, "Caster X", "spell-slot-2"), 2);
    assert!(
        log.iter().any(|line| line.contains("UPCAST NOT MODELED")),
        "{log:?}"
    );
    assert!(log.iter().any(|line| line.contains("1d6(3)")), "{log:?}");
}

#[test]
fn cantrips_reject_spell_slots() {
    let mut state = encounter_state(&["caster-x"]);
    let error = engine::use_ability(
        &mut state,
        "Caster X",
        "test-spark",
        &cast_options("Brute B", Some(2), None, false),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cantrip and cannot be cast with a spell slot"),
        "{error}"
    );
}

#[test]
fn cantrip_scaling_picks_the_highest_character_level_key() {
    // Level 5 caster: table key 5 applies.
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "test-spark",
        &cast_options("Brute B", None, None, false),
        &mut ScriptedRng::new(&[15, 4, 5]),
        &mut log,
    )
    .unwrap();
    assert!(log.iter().any(|line| line.contains("2d6(4,5)")), "{log:?}");
    assert_eq!(state.combatants[index(&state, "Brute B")].hp, 100 - 9);

    // Level 3 caster: below the first key, so the base dice stay.
    let mut state = encounter_state(&["low-caster"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Low Caster",
        "test-spark",
        &cast_options("Brute B", None, None, false),
        &mut ScriptedRng::new(&[15, 3]),
        &mut log,
    )
    .unwrap();
    assert!(log.iter().any(|line| line.contains("1d6(3)")), "{log:?}");
}

#[test]
fn cantrip_without_scaling_table_uses_base_dice() {
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "plain-spark",
        &cast_options("Brute B", None, None, false),
        &mut ScriptedRng::new(&[15, 3]),
        &mut log,
    )
    .unwrap();
    assert!(log.iter().any(|line| line.contains("1d6(3)")), "{log:?}");
}

#[test]
fn pact_cast_pays_the_override_slot_and_casts_at_the_pact_ring() {
    let mut state = encounter_state(&["warlock-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Pact Caster",
        "pact-bolt",
        &cast_options("Brute B", None, None, false),
        &mut ScriptedRng::new(&[15, 1, 2, 3, 4]),
        &mut log,
    )
    .unwrap();
    assert_eq!(
        resource(&state, "Pact Caster", "spell-slot-5"),
        1,
        "the pact slot is charged"
    );
    assert!(
        log.iter()
            .any(|line| line.contains("spends 1 spell-slot-5")),
        "{log:?}"
    );
    // The dice table is picked at the pact ring (5), not the base ring (1).
    assert!(
        log.iter().any(|line| line.contains("4d6(1,2,3,4)")),
        "{log:?}"
    );
    assert!(!log.iter().any(|line| line.contains("1d6(")), "{log:?}");
}

#[test]
fn pact_cast_rejects_slot_override() {
    let mut state = encounter_state(&["warlock-x"]);
    let error = engine::use_ability(
        &mut state,
        "Pact Caster",
        "pact-bolt",
        &cast_options("Brute B", Some(2), None, false),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("pact magic slots and can only be cast at slot level 5"),
        "{error}"
    );
    assert_eq!(
        resource(&state, "Pact Caster", "spell-slot-5"),
        2,
        "nothing spent"
    );
}

#[test]
fn arcanum_consumes_the_arcanum_resource_without_slots_or_dice() {
    let mut state = encounter_state(&["warlock-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Pact Caster",
        "arcanum-word",
        &cast_options("Brute B", None, None, false),
        &mut ScriptedRng::new(&[15, 3]),
        &mut log,
    )
    .unwrap();
    assert_eq!(
        resource(&state, "Pact Caster", "arcanum-6"),
        0,
        "arcanum spent"
    );
    assert!(
        log.iter().any(|line| line.contains("spends 1 arcanum-6")),
        "{log:?}"
    );
    // Mystic Arcanum never adds dice, even though the table has a ring-6 entry.
    assert!(log.iter().any(|line| line.contains("1d6(3)")), "{log:?}");
    assert!(!log.iter().any(|line| line.contains("5d6(")), "{log:?}");
}

#[test]
fn arcanum_rejects_slot_override() {
    let mut state = encounter_state(&["warlock-x"]);
    let error = engine::use_ability(
        &mut state,
        "Pact Caster",
        "arcanum-word",
        &cast_options("Brute B", Some(6), None, false),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Mystic Arcanum spell and consumes arcanum-6"),
        "{error}"
    );
    assert_eq!(
        resource(&state, "Pact Caster", "arcanum-6"),
        1,
        "nothing spent"
    );
}

#[test]
fn costly_material_requires_a_declaration() {
    let mut state = encounter_state(&["caster-x"]);
    let error = engine::use_ability(
        &mut state,
        "Caster X",
        "gem-cast",
        &cast_options("Caster X", None, None, false),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("needs a material component that costs 50 gp"),
        "{error}"
    );
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-1"),
        4,
        "nothing spent"
    );

    // A non-empty declaration lets the cast through and is logged verbatim.
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "gem-cast",
        &cast_options("Caster X", None, Some("a diamond worth 50 gp"), false),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(resource(&state, "Caster X", "spell-slot-1"), 3);
    assert!(
        log.iter()
            .any(|line| line.contains("MATERIAL DECLARED: a diamond worth 50 gp")),
        "{log:?}"
    );

    // An empty declaration is not a declaration.
    let error = engine::use_ability(
        &mut state,
        "Caster X",
        "gem-cast",
        &cast_options("Caster X", None, Some("   "), false),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("needs a material component"),
        "{error}"
    );
}

#[test]
fn consumed_material_requires_a_declaration_and_logs_consumption() {
    let mut state = encounter_state(&["caster-x"]);
    let error = engine::use_ability(
        &mut state,
        "Caster X",
        "dust-cast",
        &cast_options("Caster X", None, None, false),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("needs a material component that is consumed"),
        "{error}"
    );

    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "dust-cast",
        &cast_options("Caster X", None, Some("powdered iron"), false),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter()
            .any(|line| line.contains("MATERIAL CONSUMED: powdered iron")),
        "{log:?}"
    );
    assert!(
        log.iter()
            .any(|line| line.contains("MATERIAL DECLARED: powdered iron")),
        "{log:?}"
    );
}

#[test]
fn ignore_components_overrides_the_declaration_and_logs() {
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "gem-cast",
        &cast_options("Caster X", None, None, true),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-1"),
        3,
        "the cast succeeds"
    );
    assert!(
        log.iter().any(|line| line.contains("COMPONENTS IGNORED")),
        "{log:?}"
    );
}

#[test]
fn cheap_material_needs_no_declaration() {
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "free-cast",
        &cast_options("Caster X", None, None, false),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(resource(&state, "Caster X", "spell-slot-1"), 3);
    assert!(
        !log.iter().any(|line| line.contains("COMPONENTS IGNORED")),
        "{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("MATERIAL CONSUMED")),
        "{log:?}"
    );
}

#[test]
fn minute_and_hour_casting_times_fail_closed_in_combat() {
    for ability in ["slow-ritual", "hour-ward"] {
        let mut state = encounter_state(&["caster-x"]);
        let error = engine::use_ability(
            &mut state,
            "Caster X",
            ability,
            &cast_options("Caster X", None, None, false),
            &mut ScriptedRng::new(&[]),
            &mut Vec::new(),
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("cannot be completed within a combat round"),
            "{message}"
        );
        assert!(
            message.contains("outside combat"),
            "the error points at the manual or ritual path: {message}"
        );
        assert_eq!(
            resource(&state, "Caster X", "spell-slot-1"),
            4,
            "nothing spent for {ability}"
        );
    }
}

#[test]
fn bonus_action_casting_time_is_unchanged() {
    let mut state = encounter_state(&["caster-x"]);
    engine::use_ability(
        &mut state,
        "Caster X",
        "quick-word",
        &cast_options("Caster X", None, None, false),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(resource(&state, "Caster X", "spell-slot-1"), 3);
}

#[test]
fn abilities_without_spell_metadata_are_unchanged() {
    // --slot keeps the legacy meaning: context only, base cost charged.
    let mut state = encounter_state(&["caster-x"]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Caster X",
        "no-meta-bolt",
        &cast_options("Brute B", Some(5), None, false),
        &mut ScriptedRng::new(&[15, 6]),
        &mut log,
    )
    .unwrap();
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-2"),
        2,
        "the base cost resource is charged, not slot 5"
    );
    assert!(log.iter().any(|line| line.contains("1d8(6)")), "{log:?}");
    assert!(
        !log.iter().any(|line| line.contains("UPCAST NOT MODELED")),
        "legacy abilities never hit the spell checks: {log:?}"
    );

    // A slot below the stored ring is also accepted (no spell validation).
    let mut state = encounter_state(&["caster-x"]);
    engine::use_ability(
        &mut state,
        "Caster X",
        "no-meta-bolt",
        &cast_options("Brute B", Some(1), None, false),
        &mut ScriptedRng::new(&[15, 6]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(resource(&state, "Caster X", "spell-slot-2"), 2);

    // The new flags are inert for legacy abilities.
    let mut state = encounter_state(&["caster-x"]);
    engine::use_ability(
        &mut state,
        "Caster X",
        "no-meta-bolt",
        &cast_options("Brute B", None, Some("anything"), true),
        &mut ScriptedRng::new(&[15, 6]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(resource(&state, "Caster X", "spell-slot-2"), 2);
}

#[test]
fn upcast_concentration_spell_still_starts_concentration() {
    let mut state = encounter_state(&["caster-x"]);
    engine::use_ability(
        &mut state,
        "Caster X",
        "test-hold",
        &cast_options("Caster X", Some(3), None, false),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(
        state.concentration.contains_key("Caster X"),
        "concentration starts on an upcast cast"
    );
    assert_eq!(
        resource(&state, "Caster X", "spell-slot-3"),
        1,
        "slot 3 paid"
    );
}

#[test]
fn ready_setup_pays_the_ref_override_for_spell_abilities() {
    // Ready is prepaid and must keep working for spell abilities whose sheet
    // ref overrides the cost (the pact-caster shape).
    let mut state = encounter_state(&["warlock-x"]);
    let options = ReadyOptions {
        trigger: ReadyTrigger::Manual,
        description: "hold the bolt".to_string(),
        response: ReadyResponse::Ability {
            ability: "pact-bolt".to_string(),
            target: Some("Brute B".to_string()),
        },
    };
    engine::ready(&mut state, "Pact Caster", &options, &mut Vec::new()).unwrap();
    assert_eq!(
        resource(&state, "Pact Caster", "spell-slot-5"),
        1,
        "ready setup pays the overridden pact cost"
    );
}
