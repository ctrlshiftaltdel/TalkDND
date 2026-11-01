//! TASK-0127 (v1.6 T5): data-level tests for the handwritten `spells.yaml`
//! files.
//!
//! Covers the 17 (2014) / 18 (2024) `spell` metadata blocks, the upcast and
//! cantrip dice tables, the effect coverage-matrix structured samples and the
//! 2014/2024 edition differences. The engine has no edition branch: every
//! difference asserted here lives in the data files (docs/SPELLCASTING.md
//! 2.6/5.6/5.7). Values are transcribed from the SRD texts, not invented.

use combat_sim::content::schema::{Ability, CastingUnit, Effect, SpellMeta, SpellSchool};
use combat_sim::content::{self, ContentSources};

const RULES_2014: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/rulesets/real-2014.yaml"
);
const RULES_2024: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/rulesets/real-2024.yaml"
);

fn load(manifest: &str) -> content::ContentRegistry {
    let sources = ContentSources {
        campaign: Some(manifest.into()),
        ..ContentSources::default()
    };
    sources.load().expect("ruleset content should load")
}

fn ability<'a>(registry: &'a content::ContentRegistry, id: &str) -> &'a Ability {
    registry
        .abilities
        .get(id)
        .unwrap_or_else(|| panic!("missing handwritten spell `{id}`"))
}

fn spell<'a>(registry: &'a content::ContentRegistry, id: &str) -> &'a SpellMeta {
    ability(registry, id)
        .spell
        .as_ref()
        .unwrap_or_else(|| panic!("spell `{id}` has no `spell` metadata"))
}

/// One material component expectation: (text, cost_gp, consumed).
type Material<'e> = Option<(&'e str, Option<u32>, bool)>;

struct Expectation {
    id: &'static str,
    level: u8,
    school: SpellSchool,
    unit: CastingUnit,
    verbal: bool,
    somatic: bool,
    material: Material<'static>,
    ritual: bool,
    classes: &'static [&'static str],
    range_text: &'static str,
    duration_text: &'static str,
}

fn assert_expectation(registry: &content::ContentRegistry, expected: &Expectation) {
    let meta = spell(registry, expected.id);
    assert_eq!(meta.level, expected.level, "{}: level", expected.id);
    assert_eq!(
        meta.school,
        Some(expected.school),
        "{}: school",
        expected.id
    );
    assert_eq!(
        meta.casting_time.amount,
        Some(1),
        "{}: casting_time amount",
        expected.id
    );
    assert_eq!(
        meta.casting_time.unit, expected.unit,
        "{}: casting_time unit",
        expected.id
    );
    assert_eq!(
        meta.components.verbal, expected.verbal,
        "{}: verbal",
        expected.id
    );
    assert_eq!(
        meta.components.somatic, expected.somatic,
        "{}: somatic",
        expected.id
    );
    match (expected.material, &meta.components.material) {
        (None, None) => {}
        (Some((text, cost, consumed)), Some(material)) => {
            assert_eq!(material.text, text, "{}: material text", expected.id);
            assert_eq!(material.cost_gp, cost, "{}: material cost", expected.id);
            assert_eq!(
                material.consumed, consumed,
                "{}: material consumed",
                expected.id
            );
        }
        (expected_material, actual_material) => panic!(
            "{}: material mismatch, expected {expected_material:?}, got {:?}",
            expected.id,
            actual_material
                .as_ref()
                .map(|m| (&m.text, m.cost_gp, m.consumed))
        ),
    }
    assert_eq!(meta.ritual, expected.ritual, "{}: ritual", expected.id);
    assert_eq!(meta.classes, expected.classes, "{}: classes", expected.id);
    assert_eq!(
        meta.range_text.as_deref(),
        Some(expected.range_text),
        "{}: range_text",
        expected.id
    );
    assert_eq!(
        meta.duration_text.as_deref(),
        Some(expected.duration_text),
        "{}: duration_text",
        expected.id
    );
}

const EVOCATION: SpellSchool = SpellSchool::Evocation;
const ABJURATION: SpellSchool = SpellSchool::Abjuration;
const ENCHANTMENT: SpellSchool = SpellSchool::Enchantment;
const DIVINATION: SpellSchool = SpellSchool::Divination;
const CONJURATION: SpellSchool = SpellSchool::Conjuration;
const ILLUSION: SpellSchool = SpellSchool::Illusion;
const TRANSMUTATION: SpellSchool = SpellSchool::Transmutation;

const ACTION: CastingUnit = CastingUnit::Action;
const BONUS_ACTION: CastingUnit = CastingUnit::BonusAction;
const REACTION: CastingUnit = CastingUnit::Reaction;
const MINUTE: CastingUnit = CastingUnit::Minute;

const CLERIC_2014: &[&str] = &["bard", "cleric", "druid", "paladin", "ranger"];
const CLERIC_2024_DETECT: &[&str] = &[
    "bard", "cleric", "druid", "paladin", "ranger", "sorcerer", "warlock", "wizard",
];
const SORCERER_WIZARD: &[&str] = &["sorcerer", "wizard"];
const CLERIC_PALADIN: &[&str] = &["cleric", "paladin"];
const BARD_SORCERER_WIZARD: &[&str] = &["bard", "sorcerer", "wizard"];
const BARD_SORCERER_WARLOCK_WIZARD: &[&str] = &["bard", "sorcerer", "warlock", "wizard"];
const CLERIC_ONLY: &[&str] = &["cleric"];
const BARD_CLERIC_DRUID: &[&str] = &["bard", "cleric", "druid"];
const CLERIC_DRUID: &[&str] = &["cleric", "druid"];
const BARD_CLERIC_DRUID_SORCERER_WIZARD: &[&str] =
    &["bard", "cleric", "druid", "sorcerer", "wizard"];
const BARD_DRUID_SORCERER_WIZARD: &[&str] = &["bard", "druid", "sorcerer", "wizard"];
const DRUID_ONLY: &[&str] = &["druid"];

const EXPECTED_2014: &[Expectation] = &[
    Expectation {
        id: "fire-bolt",
        level: 0,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "120 feet",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "shocking-grasp",
        level: 0,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "Touch",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "sacred-flame",
        level: 0,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: CLERIC_ONLY,
        range_text: "60 feet",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "magic-missile",
        level: 1,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "120 feet",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "shield",
        level: 1,
        school: ABJURATION,
        unit: REACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "Self",
        duration_text: "1 round",
    },
    Expectation {
        id: "cure-wounds",
        level: 1,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: CLERIC_2014,
        range_text: "Touch",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "bless",
        level: 1,
        school: ENCHANTMENT,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: Some(("A sprinkling of holy water.", None, false)),
        ritual: false,
        classes: CLERIC_PALADIN,
        range_text: "30 feet",
        duration_text: "Up to 1 minute",
    },
    Expectation {
        id: "sanctuary",
        level: 1,
        school: ABJURATION,
        unit: BONUS_ACTION,
        verbal: true,
        somatic: true,
        material: Some(("A small silver mirror.", None, false)),
        ritual: false,
        classes: CLERIC_ONLY,
        range_text: "30 feet",
        duration_text: "1 minute",
    },
    Expectation {
        id: "sleep",
        level: 1,
        school: ENCHANTMENT,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: Some((
            "A pinch of fine sand, rose petals, or a cricket.",
            None,
            false,
        )),
        ritual: false,
        classes: BARD_SORCERER_WIZARD,
        range_text: "90 feet",
        duration_text: "1 minute",
    },
    Expectation {
        id: "detect-magic",
        level: 1,
        school: DIVINATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: true,
        classes: &[
            "bard", "cleric", "druid", "paladin", "ranger", "sorcerer", "wizard",
        ],
        range_text: "Self",
        duration_text: "Up to 10 minutes",
    },
    Expectation {
        id: "mage-hand",
        level: 0,
        school: CONJURATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: BARD_SORCERER_WARLOCK_WIZARD,
        range_text: "30 feet",
        duration_text: "1 minute",
    },
    Expectation {
        id: "minor-illusion",
        level: 0,
        school: ILLUSION,
        unit: ACTION,
        verbal: false,
        somatic: true,
        material: Some(("A bit of fleece.", None, false)),
        ritual: false,
        classes: BARD_SORCERER_WARLOCK_WIZARD,
        range_text: "30 feet",
        duration_text: "1 minute",
    },
    Expectation {
        id: "guidance",
        level: 0,
        school: DIVINATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: CLERIC_DRUID,
        range_text: "Touch",
        duration_text: "Up to 1 minute",
    },
    Expectation {
        id: "mending",
        level: 0,
        school: TRANSMUTATION,
        unit: MINUTE,
        verbal: true,
        somatic: true,
        material: Some(("Two lodestones.", None, false)),
        ritual: false,
        classes: BARD_CLERIC_DRUID_SORCERER_WIZARD,
        range_text: "Touch",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "healing-word",
        level: 1,
        school: EVOCATION,
        unit: BONUS_ACTION,
        verbal: true,
        somatic: false,
        material: None,
        ritual: false,
        classes: BARD_CLERIC_DRUID,
        range_text: "60 feet",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "command",
        level: 1,
        school: ENCHANTMENT,
        unit: ACTION,
        verbal: true,
        somatic: false,
        material: None,
        ritual: false,
        classes: CLERIC_PALADIN,
        range_text: "60 feet",
        duration_text: "1 round",
    },
    Expectation {
        id: "mage-armor",
        level: 1,
        school: ABJURATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: Some(("A piece of cured leather.", None, false)),
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "Touch",
        duration_text: "8 hours",
    },
];

const EXPECTED_2024: &[Expectation] = &[
    Expectation {
        id: "fire-bolt",
        level: 0,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "120 feet",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "shocking-grasp",
        level: 0,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "Touch",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "sacred-flame",
        level: 0,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: CLERIC_ONLY,
        range_text: "60 feet",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "magic-missile",
        level: 1,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "120 feet",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "shield",
        level: 1,
        school: ABJURATION,
        unit: REACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "Self",
        duration_text: "1 round",
    },
    Expectation {
        id: "cure-wounds",
        level: 1,
        school: ABJURATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: CLERIC_2014,
        range_text: "Touch",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "bless",
        level: 1,
        school: ENCHANTMENT,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: Some(("a Holy Symbol worth 5+ GP", Some(5), false)),
        ritual: false,
        classes: CLERIC_PALADIN,
        range_text: "30 feet",
        duration_text: "Up to 1 minute",
    },
    Expectation {
        id: "sanctuary",
        level: 1,
        school: ABJURATION,
        unit: BONUS_ACTION,
        verbal: true,
        somatic: true,
        material: Some(("a shard of glass from a mirror", None, false)),
        ritual: false,
        classes: CLERIC_ONLY,
        range_text: "30 feet",
        duration_text: "1 minute",
    },
    Expectation {
        id: "sleep",
        level: 1,
        school: ENCHANTMENT,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: Some(("a pinch of sand or rose petals", None, false)),
        ritual: false,
        classes: BARD_SORCERER_WIZARD,
        range_text: "60 feet",
        duration_text: "Up to 1 minute",
    },
    Expectation {
        id: "detect-magic",
        level: 1,
        school: DIVINATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: true,
        classes: CLERIC_2024_DETECT,
        range_text: "Self",
        duration_text: "Up to 10 minutes",
    },
    Expectation {
        id: "mage-hand",
        level: 0,
        school: CONJURATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: BARD_SORCERER_WARLOCK_WIZARD,
        range_text: "30 feet",
        duration_text: "1 minute",
    },
    Expectation {
        id: "minor-illusion",
        level: 0,
        school: ILLUSION,
        unit: ACTION,
        verbal: false,
        somatic: true,
        material: Some(("a bit of fleece", None, false)),
        ritual: false,
        classes: BARD_SORCERER_WARLOCK_WIZARD,
        range_text: "30 feet",
        duration_text: "1 minute",
    },
    Expectation {
        id: "burning-hands",
        level: 1,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "Self",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "fireball",
        level: 3,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: Some(("a ball of bat guano and sulfur", None, false)),
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "150 feet",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "lightning-bolt",
        level: 3,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: Some(("a bit of fur and a crystal rod", None, false)),
        ritual: false,
        classes: SORCERER_WIZARD,
        range_text: "Self",
        duration_text: "Instantaneous",
    },
    Expectation {
        id: "moonbeam",
        level: 2,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: Some(("a moonseed leaf", None, false)),
        ritual: false,
        classes: DRUID_ONLY,
        range_text: "120 feet",
        duration_text: "Up to 1 minute",
    },
    Expectation {
        id: "spirit-guardians",
        level: 3,
        school: CONJURATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: Some(("a prayer scroll", None, false)),
        ritual: false,
        classes: CLERIC_ONLY,
        range_text: "Self",
        duration_text: "Up to 10 minutes",
    },
    Expectation {
        id: "thunderwave",
        level: 1,
        school: EVOCATION,
        unit: ACTION,
        verbal: true,
        somatic: true,
        material: None,
        ritual: false,
        classes: BARD_DRUID_SORCERER_WIZARD,
        range_text: "Self",
        duration_text: "Instantaneous",
    },
];

#[test]
fn every_handwritten_spell_carries_the_srd_metadata() {
    let y2014 = load(RULES_2014);
    for expected in EXPECTED_2014 {
        assert_expectation(&y2014, expected);
    }
    let y2024 = load(RULES_2024);
    for expected in EXPECTED_2024 {
        assert_expectation(&y2024, expected);
    }
}

#[test]
fn metadata_set_is_exactly_the_handwritten_spells() {
    // The tracked SRD packs carry no `spell` blocks (never regenerated);
    // the handwritten files are the only source of metadata.
    let y2014 = load(RULES_2014);
    let mut with_meta: Vec<&str> = y2014
        .abilities
        .ids()
        .filter(|id| {
            y2014
                .abilities
                .get(id)
                .and_then(|a| a.spell.as_ref())
                .is_some()
        })
        .collect();
    with_meta.sort();
    let mut expected: Vec<&str> = EXPECTED_2014.iter().map(|e| e.id).collect();
    expected.sort();
    assert_eq!(with_meta, expected, "2014 metadata set");

    let y2024 = load(RULES_2024);
    let mut with_meta: Vec<&str> = y2024
        .abilities
        .ids()
        .filter(|id| {
            y2024
                .abilities
                .get(id)
                .and_then(|a| a.spell.as_ref())
                .is_some()
        })
        .collect();
    with_meta.sort();
    let mut expected: Vec<&str> = EXPECTED_2024.iter().map(|e| e.id).collect();
    expected.sort();
    assert_eq!(with_meta, expected, "2024 metadata set");
}

fn assert_table(
    actual: &std::collections::BTreeMap<u8, String>,
    expected: &[(u8, &str)],
    what: &str,
) {
    let expected: std::collections::BTreeMap<u8, String> = expected
        .iter()
        .map(|(level, dice)| (*level, dice.to_string()))
        .collect();
    assert_eq!(actual, &expected, "{what}");
}

#[test]
fn upcast_tables_cover_every_slot_level() {
    let y2014 = load(RULES_2014);
    let y2024 = load(RULES_2024);

    // 2014 Cure Wounds: +1d8 per slot above 1st.
    let upcast = spell(&y2014, "cure-wounds").upcast.as_ref().unwrap();
    assert_table(
        &upcast.dice_by_level,
        &[
            (2, "2d8+{spell_mod}"),
            (3, "3d8+{spell_mod}"),
            (4, "4d8+{spell_mod}"),
            (5, "5d8+{spell_mod}"),
            (6, "6d8+{spell_mod}"),
            (7, "7d8+{spell_mod}"),
            (8, "8d8+{spell_mod}"),
            (9, "9d8+{spell_mod}"),
        ],
        "2014 cure-wounds",
    );
    // 2014 Healing Word: +1d4 per slot above 1st.
    let upcast = spell(&y2014, "healing-word").upcast.as_ref().unwrap();
    assert_table(
        &upcast.dice_by_level,
        &[
            (2, "2d4+{spell_mod}"),
            (3, "3d4+{spell_mod}"),
            (4, "4d4+{spell_mod}"),
            (5, "5d4+{spell_mod}"),
            (6, "6d4+{spell_mod}"),
            (7, "7d4+{spell_mod}"),
            (8, "8d4+{spell_mod}"),
            (9, "9d4+{spell_mod}"),
        ],
        "2014 healing-word",
    );

    // 2024 Cure Wounds: +2d8 per slot above 1 (2d8 at 1st, 4d8 at 2nd, ...).
    let upcast = spell(&y2024, "cure-wounds").upcast.as_ref().unwrap();
    assert_table(
        &upcast.dice_by_level,
        &[
            (2, "4d8+{spell_mod}"),
            (3, "6d8+{spell_mod}"),
            (4, "8d8+{spell_mod}"),
            (5, "10d8+{spell_mod}"),
            (6, "12d8+{spell_mod}"),
            (7, "14d8+{spell_mod}"),
            (8, "16d8+{spell_mod}"),
            (9, "18d8+{spell_mod}"),
        ],
        "2024 cure-wounds",
    );
    // 2024 one-shot saves: +1 die step per slot above base.
    assert_table(
        &spell(&y2024, "burning-hands")
            .upcast
            .as_ref()
            .unwrap()
            .dice_by_level,
        &[
            (2, "4d6"),
            (3, "5d6"),
            (4, "6d6"),
            (5, "7d6"),
            (6, "8d6"),
            (7, "9d6"),
            (8, "10d6"),
            (9, "11d6"),
        ],
        "2024 burning-hands",
    );
    for id in ["fireball", "lightning-bolt"] {
        assert_table(
            &spell(&y2024, id).upcast.as_ref().unwrap().dice_by_level,
            &[
                (4, "9d6"),
                (5, "10d6"),
                (6, "11d6"),
                (7, "12d6"),
                (8, "13d6"),
                (9, "14d6"),
            ],
            id,
        );
    }
    assert_table(
        &spell(&y2024, "thunderwave")
            .upcast
            .as_ref()
            .unwrap()
            .dice_by_level,
        &[
            (2, "3d8"),
            (3, "4d8"),
            (4, "5d8"),
            (5, "6d8"),
            (6, "7d8"),
            (7, "8d8"),
            (8, "9d8"),
            (9, "10d8"),
        ],
        "2024 thunderwave",
    );

    // Upcast that is not a main-damage/heal dice table stays unmodelled: an
    // upcast block without the slot's key fails closed in cast --slot, so
    // extra darts/targets/pools and persistent areas keep manual handling.
    for (registry, ids) in [
        (
            &y2014,
            [
                "magic-missile",
                "sleep",
                "bless",
                "command",
                "mage-armor",
                "shield",
                "sanctuary",
            ],
        ),
        (
            &y2024,
            [
                "magic-missile",
                "sleep",
                "bless",
                "sanctuary",
                "shield",
                "moonbeam",
                "spirit-guardians",
            ],
        ),
    ] {
        for id in ids {
            assert!(
                spell(registry, id).upcast.is_none(),
                "{id} must not carry an upcast block without a dice table"
            );
        }
    }
}

#[test]
fn cantrip_scaling_covers_levels_5_11_17() {
    let y2014 = load(RULES_2014);
    let y2024 = load(RULES_2024);
    for (registry, prefix) in [(&y2014, "2014"), (&y2024, "2024")] {
        assert_table(
            &spell(registry, "fire-bolt")
                .cantrip_scaling
                .as_ref()
                .unwrap()
                .dice_by_character_level,
            &[(5, "2d10"), (11, "3d10"), (17, "4d10")],
            "{prefix} fire-bolt",
        );
        assert_table(
            &spell(registry, "shocking-grasp")
                .cantrip_scaling
                .as_ref()
                .unwrap()
                .dice_by_character_level,
            &[(5, "2d8"), (11, "3d8"), (17, "4d8")],
            "{prefix} shocking-grasp",
        );
        assert_table(
            &spell(registry, "sacred-flame")
                .cantrip_scaling
                .as_ref()
                .unwrap()
                .dice_by_character_level,
            &[(5, "2d8"), (11, "3d8"), (17, "4d8")],
            "{prefix} sacred-flame",
        );
        for id in ["mage-hand", "minor-illusion", "guidance", "mending"] {
            // The tracked pack also supplies these ids (e.g. 2024 guidance)
            // without metadata; only handwritten entries are asserted here.
            let Some(entry) = registry.abilities.get(id) else {
                continue;
            };
            if entry.spell.is_none() {
                continue;
            }
            assert!(
                spell(registry, id).cantrip_scaling.is_none(),
                "{prefix} {id} has no damage to scale"
            );
        }
    }
}

#[test]
fn coverage_matrix_has_structured_samples() {
    // attack
    let y2014 = load(RULES_2014);
    let y2024 = load(RULES_2024);
    for registry in [&y2014, &y2024] {
        let Effect::Attack(attack) = &ability(registry, "fire-bolt").effects[0] else {
            panic!("fire-bolt must be an attack sample");
        };
        assert_eq!(attack.damage[0].dice.as_text(), "1d10");
        // auto damage
        let Effect::Damage(missile) = &ability(registry, "magic-missile").effects[0] else {
            panic!("magic-missile must be an auto-damage sample");
        };
        assert_eq!(missile.repeat, 3);
        // heal
        let Effect::Heal(heal) = &ability(registry, "cure-wounds").effects[0] else {
            panic!("cure-wounds must be a heal sample");
        };
        assert!(heal.dice.as_text().ends_with("d8+{spell_mod}"));
        // condition
        let Effect::ApplyCondition(blessed) = &ability(registry, "bless").effects[0] else {
            panic!("bless must be a condition sample");
        };
        assert_eq!(blessed.condition, "blessed");
        // note + manual stays manual
        assert!(matches!(
            ability(registry, "detect-magic").effects[0],
            Effect::Note(_)
        ));
    }
    // save + half damage (2024 only: the 2014 set has no save-half spell)
    let Effect::Save(fireball) = &ability(&y2024, "fireball").effects[0] else {
        panic!("2024 fireball must be a save-half sample");
    };
    assert!(fireball.half);
    let Effect::Damage(fireball_damage) = &fireball.on_fail[0] else {
        panic!("fireball on_fail must open with damage");
    };
    assert_eq!(fireball_damage.dice.as_text(), "8d6");
    // forced move rides on the failed save
    let Effect::Save(thunderwave) = &ability(&y2024, "thunderwave").effects[0] else {
        panic!("2024 thunderwave must be a save sample");
    };
    assert!(thunderwave.on_fail.iter().any(|effect| matches!(
        effect,
        Effect::ForcedMove(push) if push.distance == 10 && matches!(push.direction, combat_sim::content::schema::Direction::Away)
    )));
    // 2024 sleep keeps its structured condition-on-failed-save
    let Effect::Save(sleep) = &ability(&y2024, "sleep").effects[0] else {
        panic!("2024 sleep must be a save sample");
    };
    assert!(sleep.on_fail.iter().any(|effect| matches!(
        effect,
        Effect::ApplyCondition(condition) if condition.condition == "incapacitated"
    )));
    // persistent areas and the 2014 HP pool stay note + manual
    for id in ["moonbeam", "spirit-guardians"] {
        let spell = ability(&y2024, id);
        assert!(
            matches!(spell.effects[0], Effect::Note(_)),
            "{id} stays manual"
        );
        assert!(spell
            .manual
            .as_deref()
            .unwrap_or_default()
            .contains("At higher levels"));
    }
    assert!(matches!(
        ability(&y2014, "sleep").effects[0],
        Effect::Note(_)
    ));
}

#[test]
fn edition_differences_live_in_data_not_the_engine() {
    let y2014 = load(RULES_2014);
    let y2024 = load(RULES_2024);
    assert_eq!(y2014.ruleset, "2014");
    assert_eq!(y2024.ruleset, "2024");

    // Cure Wounds: 1d8+mod (evocation) vs 2d8+mod (abjuration); the 2024
    // upcast doubles the dice per slot instead of adding one.
    let Effect::Heal(heal_2014) = &ability(&y2014, "cure-wounds").effects[0] else {
        panic!("2014 cure-wounds must heal");
    };
    let Effect::Heal(heal_2024) = &ability(&y2024, "cure-wounds").effects[0] else {
        panic!("2024 cure-wounds must heal");
    };
    assert_eq!(heal_2014.dice.as_text(), "1d8+{spell_mod}");
    assert_eq!(heal_2024.dice.as_text(), "2d8+{spell_mod}");
    assert_eq!(
        spell(&y2014, "cure-wounds").school,
        Some(SpellSchool::Evocation)
    );
    assert_eq!(
        spell(&y2024, "cure-wounds").school,
        Some(SpellSchool::Abjuration)
    );
    assert_eq!(
        spell(&y2014, "cure-wounds")
            .upcast
            .as_ref()
            .unwrap()
            .dice_by_level[&2],
        "2d8+{spell_mod}"
    );
    assert_eq!(
        spell(&y2024, "cure-wounds")
            .upcast
            .as_ref()
            .unwrap()
            .dice_by_level[&2],
        "4d8+{spell_mod}"
    );

    // Sleep: 2014 is a manual HP pool (no save), 2024 is a structured WIS
    // save that applies a condition.
    assert!(matches!(
        ability(&y2014, "sleep").effects[0],
        Effect::Note(_)
    ));
    let Effect::Save(sleep_2024) = &ability(&y2024, "sleep").effects[0] else {
        panic!("2024 sleep must save");
    };
    assert!(matches!(
        sleep_2024.ability,
        combat_sim::content::schema::AbilityName::Wis
    ));

    // Detect Magic class list: 2024 adds Warlock.
    assert!(!spell(&y2014, "detect-magic")
        .classes
        .contains(&"warlock".to_string()));
    assert!(spell(&y2024, "detect-magic")
        .classes
        .contains(&"warlock".to_string()));

    // Bless material: holy water (2014, free) vs a priced holy symbol (2024).
    let material_2014 = spell(&y2014, "bless").components.material.as_ref().unwrap();
    let material_2024 = spell(&y2024, "bless").components.material.as_ref().unwrap();
    assert_eq!(material_2014.text, "A sprinkling of holy water.");
    assert_eq!(material_2014.cost_gp, None);
    assert_eq!(material_2024.text, "a Holy Symbol worth 5+ GP");
    assert_eq!(material_2024.cost_gp, Some(5));
}
