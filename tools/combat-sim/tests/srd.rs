//! SRD JSON -> library YAML conversion on synthetic data.

use combat_sim::srd::*;
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn converts_a_monster_and_spells() {
    let m = json!([{
        "index": "cog-wolf", "name": "Cog Wolf", "size": "Medium", "type": "construct",
        "armor_class": [{"type": "natural", "value": 13}], "hit_points": 11, "hit_points_roll": "2d8+2",
        "speed": {"walk": "40 ft.", "burrow": "10 ft."}, "strength": 12, "dexterity": 15, "constitution": 12,
        "intelligence": 3, "wisdom": 12, "charisma": 6, "proficiency_bonus": 2, "challenge_rating": 0.25, "xp": 50,
        "proficiencies": [{"value": 3, "proficiency": {"index": "skill-perception"}}, {"value": 4, "proficiency": {"index": "saving-throw-dex"}}],
        "damage_resistances": ["bludgeoning, piercing, and slashing from nonmagical attacks"], "damage_immunities": ["poison"],
        "condition_immunities": [{"index": "poisoned"}], "senses": {"darkvision": "60 ft.", "passive_perception": 13},
        "actions": [
            {"name": "Multiattack", "desc": "Two bites.", "actions": [{"action_name": "Bite", "count": "2", "type": "melee"}]},
            {"name": "Bite", "desc": "Melee Weapon Attack: +4 to hit, reach 5 ft., one target. Hit: 7 (2d4 + 2) piercing damage. The target is knocked prone.", "attack_bonus": 4, "damage": [{"damage_type": {"index": "piercing"}, "damage_dice": "2d4+2"}]},
            {"name": "Gear Spray", "desc": "DC 11 Dexterity save.", "dc": {"dc_type": {"index": "dex"}, "dc_value": 11}, "damage": [{"damage_type": {"index": "slashing"}, "damage_dice": "2d6"}]}
        ],
        "special_abilities": [{"name": "Keen Hearing", "desc": "Advantage on hearing."}]
    }]);
    let (v, n) = convert_monsters(&m, &BTreeSet::new());
    assert_eq!(n, 1);
    let c = &v["creatures"][0];
    assert_eq!(c["speed"], json!({"walk": 40, "burrow": 10}));
    assert_eq!(c["init_bonus"], 2);
    assert_eq!(c["skills"]["perception"], 3);
    assert_eq!(c["saves"]["dex"], 4);
    assert_eq!(c["immunities"], json!(["poison"]));
    assert!(c.get("resistances").is_none());
    assert_eq!(c["multiattack"], json!(["bite", "bite"]));
    assert_eq!(c["attacks"][0]["manual"], "The target is knocked prone.");
    assert_eq!(c["attacks"].as_array().unwrap().len(), 1);
    assert!(c["notes"].as_str().unwrap().contains("CR 1/4 (XP 50)"));
    assert!(!c["notes"].as_str().unwrap().contains("burrow speed 10"));
    assert_eq!(c["ai"]["template"], "mindless");
    let skip: BTreeSet<String> = ["cog-wolf".to_string()].into();
    assert_eq!(convert_monsters(&m, &skip).1, 0);

    let s = json!([
        {"index": "cog-bolt", "name": "Cog Bolt", "level": 0, "desc": ["Zap."], "range": "60 feet", "casting_time": "1 action",
         "attack_type": "ranged", "damage": {"damage_type": {"index": "lightning"}, "damage_at_character_level": {"1": "1d10", "5": "2d10"}}},
        {"index": "gear-storm", "name": "Gear Storm", "level": 3, "desc": ["Boom."], "range": "150 feet", "casting_time": "1 action",
         "dc": {"dc_type": {"index": "dex"}, "dc_success": "half"}, "area_of_effect": {"type": "sphere", "size": 20},
         "damage": {"damage_type": {"index": "fire"}, "damage_at_slot_level": {"4": "9d6", "3": "8d6"}}},
        {"index": "oil-up", "name": "Oil Up", "level": 1, "desc": ["Heal."], "range": "Touch", "casting_time": "1 bonus action",
         "heal_at_slot_level": {"1": "1d4 + MOD"}},
        {"index": "cog-sense", "name": "Cog Sense", "level": 1, "desc": ["Sense."], "range": "Self", "ritual": true, "concentration": true}
    ]);
    let (v, n) = convert_spells(&s, &BTreeSet::new());
    assert_eq!(n, 4);
    let a = &v["abilities"];
    assert_eq!(a[0]["effects"][0]["attack"]["damage"][0]["dice"], "1d10");
    assert_eq!(a[1]["effects"][0]["save"]["half"], true);
    assert_eq!(
        a[1]["effects"][0]["save"]["on_fail"][0]["damage"]["dice"],
        "8d6"
    );
    assert_eq!(a[1]["cost"]["resource"], "spell-slot-3");
    assert_eq!(a[1]["area"]["radius"], 20);
    assert_eq!(a[2]["effects"][0]["heal"]["dice"], "1d4+{spell_mod}");
    assert_eq!(a[2]["activation"], "bonus");
    assert_eq!(a[2]["range"], 5);
    assert_eq!(a[3]["target"]["type"], "self");
    assert!(a[3]["tags"].as_array().unwrap().contains(&json!("ritual")));
}

#[test]
fn quick_yaml_builds_a_grid_encounter() {
    let y = combat_sim::cli::quick_yaml(&["cog:2".into(), "gear@near".into()], 30, "dim").unwrap();
    assert!(y.contains("- \"n......f\""));
    assert!(y.contains("{creature: cog, count: 2, zone: far}"));
    assert!(y.contains("{creature: gear, count: 1, zone: near}"));
    assert!(combat_sim::cli::quick_yaml(&["cog@mid".into()], 30, "dim").is_err());
    assert!(combat_sim::cli::quick_yaml(&["cog:0".into()], 30, "dim").is_err());
    assert!(combat_sim::cli::quick_yaml(&["a b".into()], 30, "dim").is_err());
}

#[test]
fn shipped_2014_library_with_srd_import_checks_clean() {
    let lib = concat!(env!("CARGO_MANIFEST_DIR"), "/../../LetsPlay/Rules/combat");
    if !std::path::Path::new(lib).join("2014/srd").exists() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("cs-srd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let man = dir.join("campaign.yaml");
    std::fs::write(
        &man,
        format!("name: t\nruleset: \"2014\"\nlibrary: ['{lib}']\n"),
    )
    .unwrap();
    let sources = combat_sim::content::ContentSources {
        campaign: Some(man),
        ..Default::default()
    };
    let reg = sources.load().unwrap();
    assert!(reg.creatures.ids().count() > 300);
    let report = combat_sim::content::check(&reg);
    assert!(!report.has_errors(), "{} errors", report.error_count());
    let _ = std::fs::remove_dir_all(&dir);
}
