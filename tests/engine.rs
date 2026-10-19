//! Engine tests: generic fixtures only, no campaign content.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use combat_sim::content::schema::{AbilityName, Duration, DurationUntil};
use combat_sim::content::{ContentRegistry, ContentSources};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{self, AddConditionOptions, AttackOptions, RestKind, UseOptions};
use combat_sim::state::{ActiveDuration, AddSpec, CombatOutcome, EncounterState, NewEncounter};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/engine")
        .join(relative)
}

fn registry() -> ContentRegistry {
    ContentSources {
        campaign: Some(fixture("manifest.yaml")),
        ..ContentSources::default()
    }
    .load()
    .expect("engine fixture campaign should load")
}

fn state(encounter: &str, party: &[&str], adds: &[(&str, &str)]) -> EncounterState {
    let options = NewEncounter {
        encounter: encounter.to_string(),
        party: party.iter().map(|name| name.to_string()).collect(),
        party_zone: None,
        adds: adds
            .iter()
            .map(|(id, zone)| AddSpec {
                creature: id.to_string(),
                count: 1,
                zone: Some(zone.to_string()),
            })
            .collect(),
    };
    EncounterState::from_encounter(registry(), &options).expect("fixture encounter should build")
}

#[derive(Debug, Default)]
struct ScriptedRng {
    values: VecDeque<u32>,
    sides: Vec<u32>,
}

impl ScriptedRng {
    fn new(values: &[u32]) -> Self {
        Self {
            values: values.iter().copied().collect(),
            sides: Vec::new(),
        }
    }
}

impl DiceRng for ScriptedRng {
    fn roll_die(&mut self, sides: u32) -> u32 {
        self.sides.push(sides);
        self.values
            .pop_front()
            .unwrap_or_else(|| panic!("scripted RNG ran out while rolling d{sides}"))
    }
}

fn index(state: &EncounterState, name: &str) -> usize {
    state.find_index(name).expect("combatant exists")
}

fn hp(state: &EncounterState, name: &str) -> i32 {
    state.combatants[index(state, name)].hp
}

#[test]
fn critical_hit_doubles_dice_not_modifier() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let mut rng = ScriptedRng::new(&[20, 3, 4]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    assert_eq!(hp(&state, "Brute A"), 40 - (3 + 4 + 2));
    assert!(
        log.iter().any(|line| line.contains("2d8(3,4)+2=9")),
        "{log:?}"
    );
    assert!(log.iter().any(|line| line.contains("HIT")), "{log:?}");
}

#[test]
fn natural_one_always_misses() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let mut rng = ScriptedRng::new(&[1]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    assert_eq!(hp(&state, "Brute A"), 40);
    assert!(log.iter().any(|line| line.contains("MISS")), "{log:?}");
}

#[test]
fn advantage_and_disadvantage_cancel() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let options = AddConditionOptions::default();
    engine::add_condition(
        &mut state,
        "Striker A",
        "hindered",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    engine::add_condition(
        &mut state,
        "Striker A",
        "empowered",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    let mut rng = ScriptedRng::new(&[15, 4]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    assert_eq!(
        rng.sides,
        vec![20, 8],
        "a cancelled roll must use one d20 and one damage die"
    );
    assert!(!log.iter().any(|line| line.contains("adv->")), "{log:?}");
    assert!(state.combatants[index(&state, "Striker A")]
        .conditions
        .iter()
        .all(|condition| condition.condition != "empowered"));
}

#[test]
fn data_defined_on_hit_trigger_runs_once_per_turn() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let options = AddConditionOptions::default();
    engine::add_condition(
        &mut state,
        "Striker A",
        "empowered",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    let mut rng = ScriptedRng::new(&[10, 15, 4, 5]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();
    assert_eq!(hp(&state, "Brute A"), 40 - 6 - 5);
    assert!(log.iter().any(|line| line.contains("1d6(5)")), "{log:?}");
    assert!(
        log.iter().any(|line| line.contains("5 slashing")),
        "{log:?}"
    );

    // Same turn: the once-per-turn trigger is spent, even if the action is reset.
    let striker = index(&state, "Striker A");
    state.combatants[striker].action_used = false;
    engine::add_condition(
        &mut state,
        "Striker A",
        "empowered",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let mut rng = ScriptedRng::new(&[10, 15, 4]);
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();
    assert_eq!(hp(&state, "Brute A"), 40 - 6 - 5 - 6);
}

#[test]
fn reaction_condition_turns_hit_into_miss_and_spends_resource() {
    let mut state = state("test-fight", &["warden-b"], &[]);
    let mut rng = ScriptedRng::new(&[13]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Brute A",
        "Warden B",
        "club",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    let warden = &state.combatants[index(&state, "Warden B")];
    assert_eq!(warden.hp, 20, "the hit must be converted to a miss");
    assert_eq!(warden.resources["ward"].current, 0);
    assert!(warden.has_condition(&state.content, "guarded"));
    assert!(
        log.iter().any(|line| line.contains("+5[guarded]")),
        "{log:?}"
    );
    assert!(log.iter().any(|line| line.contains("MISS")), "{log:?}");
}

#[test]
fn incoming_attack_condition_grants_advantage_and_auto_crit() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let options = AddConditionOptions::default();
    engine::add_condition(
        &mut state,
        "Brute A",
        "exposed",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    let mut rng = ScriptedRng::new(&[3, 17, 1, 2, 3, 4]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    assert!(
        log.iter().any(|line| line.contains("adv: exposed")),
        "{log:?}"
    );
    assert!(log.iter().any(|line| line.contains("2d8")), "{log:?}");
    assert!(log.iter().any(|line| line.contains("2d6")), "{log:?}");
    assert_eq!(hp(&state, "Brute A"), 40 - (1 + 2 + 2 + 3 + 4));
}

#[test]
fn consuming_modifier_removes_its_condition() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let options = AddConditionOptions::default();
    engine::add_condition(
        &mut state,
        "Striker A",
        "empowered",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    let mut rng = ScriptedRng::new(&[10, 15, 4, 5]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    assert!(
        !state.combatants[index(&state, "Striker A")].has_condition(&state.content, "empowered")
    );
}

#[test]
fn mastery_trigger_can_force_a_save_and_condition() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let mut rng = ScriptedRng::new(&[15, 2, 1]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Brute A",
        "Striker A",
        "club",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    assert!(state.combatants[index(&state, "Striker A")].has_condition(&state.content, "dazed"));
    assert!(log.iter().any(|line| line.contains("CON save")), "{log:?}");
}

#[test]
fn linked_condition_modifier_uses_the_linked_creature() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let options = AddConditionOptions {
        linked: Some("Striker A".to_string()),
        ..AddConditionOptions::default()
    };
    engine::add_condition(
        &mut state,
        "Brute A",
        "vexed-test",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    let mut rng = ScriptedRng::new(&[3, 17, 4, 5]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("adv: vexed-test")),
        "{log:?}"
    );
    assert!(!state.combatants[index(&state, "Brute A")].has_condition(&state.content, "vexed-test"));
}

#[test]
fn mastery_predicate_damage_dealt_gates_its_condition() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let mut rng = ScriptedRng::new(&[15, 3]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "slow-blade",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();
    assert!(state.combatants[index(&state, "Brute A")].has_condition(&state.content, "slowed-test"));
}

#[test]
fn mastery_save_dc_can_use_attack_modifier_and_proficiency() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let mut rng = ScriptedRng::new(&[15, 3, 1]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Striker A",
        "Brute A",
        "topple-blade",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();
    assert!(log.iter().any(|line| line.contains("vs DC 13")), "{log:?}");
    assert!(state.combatants[index(&state, "Brute A")].has_condition(&state.content, "dazed"));
}

#[test]
fn duplicate_party_names_get_numbered() {
    let state = state("test-fight", &["striker-a", "striker-a"], &[]);
    let names: Vec<&str> = state
        .combatants
        .iter()
        .map(|combatant| combatant.name.as_str())
        .collect();
    assert!(names.contains(&"Striker A 1"));
    assert!(names.contains(&"Striker A 2"));
}

#[test]
fn ends_on_and_save_ends_expire_conditions() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let options = AddConditionOptions::default();
    engine::add_condition(
        &mut state,
        "Brute A",
        "brittle-test",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    engine::damage(
        &mut state,
        "Brute A",
        "1",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(
        !state.combatants[index(&state, "Brute A")].has_condition(&state.content, "brittle-test")
    );

    let striker = index(&state, "Striker A");
    let brute = index(&state, "Brute A");
    engine::init(
        &mut state,
        &[(striker, 30), (brute, 1)],
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let options = AddConditionOptions {
        rounds: Some(3),
        save_ends: Some((AbilityName::Con, 10)),
        ..AddConditionOptions::default()
    };
    engine::add_condition(
        &mut state,
        "Brute A",
        "lingering-test",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let mut log = Vec::new();
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[]), &mut log).unwrap();
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[15]), &mut log).unwrap();
    assert!(!state.combatants[brute].has_condition(&state.content, "lingering-test"));
}

#[test]
fn static_and_condition_resistance_apply() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    engine::damage(
        &mut state,
        "Brute A",
        "10",
        Some("fire"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(hp(&state, "Brute A"), 35, "static resistance halves damage");

    engine::damage(
        &mut state,
        "Brute A",
        "10",
        Some("cold"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(hp(&state, "Brute A"), 15, "vulnerability doubles damage");

    let options = AddConditionOptions::default();
    engine::add_condition(
        &mut state,
        "Striker A",
        "insulated",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    engine::damage(
        &mut state,
        "Striker A",
        "10",
        Some("fire"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        hp(&state, "Striker A"),
        15,
        "condition resistance halves damage"
    );
}

#[test]
fn death_saves_use_rules_dying_conditions() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    engine::damage(
        &mut state,
        "Striker A",
        "20",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(hp(&state, "Striker A"), 0);
    assert!(
        state.combatants[index(&state, "Striker A")].has_condition(&state.content, "downed-test")
    );

    engine::death_save(
        &mut state,
        "Striker A",
        &mut ScriptedRng::new(&[20]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(hp(&state, "Striker A"), 1);
    assert!(
        !state.combatants[index(&state, "Striker A")].has_condition(&state.content, "downed-test")
    );

    engine::damage(
        &mut state,
        "Striker A",
        "20",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    engine::death_save(
        &mut state,
        "Striker A",
        &mut ScriptedRng::new(&[1]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        state.combatants[index(&state, "Striker A")]
            .death_saves
            .failures,
        2
    );
    engine::death_save(
        &mut state,
        "Striker A",
        &mut ScriptedRng::new(&[2]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(state.combatants[index(&state, "Striker A")].is_dead());
}

#[test]
fn damage_at_zero_hp_adds_one_death_save_failure() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    engine::damage(
        &mut state,
        "Striker A",
        "20",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(hp(&state, "Striker A"), 0);
    assert_eq!(
        state.combatants[index(&state, "Striker A")]
            .death_saves
            .failures,
        0
    );

    engine::damage(
        &mut state,
        "Striker A",
        "5",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let striker = &state.combatants[index(&state, "Striker A")];
    assert_eq!(striker.death_saves.failures, 1);
    assert!(!striker.is_dead());
}

#[test]
fn critical_damage_at_zero_hp_adds_two_death_save_failures() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    engine::damage(
        &mut state,
        "Striker A",
        "20",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    engine::damage(
        &mut state,
        "Striker A",
        "5",
        Some("slashing"),
        true,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        state.combatants[index(&state, "Striker A")]
            .death_saves
            .failures,
        2
    );
}

#[test]
fn temporary_hp_absorbing_damage_prevents_death_save_failure() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    engine::damage(
        &mut state,
        "Striker A",
        "20",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let striker = index(&state, "Striker A");
    state.combatants[striker].temp_hp = 10;

    engine::damage(
        &mut state,
        "Striker A",
        "5",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let striker = &state.combatants[index(&state, "Striker A")];
    assert_eq!(striker.temp_hp, 5);
    assert_eq!(striker.death_saves.failures, 0);
}

#[test]
fn dropping_to_zero_hp_starts_dying_without_a_failure() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    engine::damage(
        &mut state,
        "Striker A",
        "20",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let striker = &state.combatants[index(&state, "Striker A")];
    assert_eq!(striker.hp, 0);
    assert_eq!(striker.death_saves.failures, 0);
    assert!(striker.is_dying());
    assert!(
        striker.has_condition(&state.content, "downed-test"),
        "dying conditions still apply on the drop to 0"
    );
}

#[test]
fn massive_damage_at_zero_hp_kills_without_failures() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    engine::damage(
        &mut state,
        "Striker A",
        "20",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    engine::damage(
        &mut state,
        "Striker A",
        "20",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let striker = &state.combatants[index(&state, "Striker A")];
    assert!(striker.is_dead());
    assert!(striker.death_saves.dead);
    assert_eq!(striker.death_saves.failures, 0);
}

#[test]
fn next_turn_skips_pc_killed_by_start_of_turn_death_save() {
    let mut state = state("test-fight", &["striker-a", "caster-c"], &[]);
    let striker = index(&state, "Striker A");
    let caster = index(&state, "Caster C");
    let brute = index(&state, "Brute A");
    engine::init(
        &mut state,
        &[(brute, 30), (striker, 20), (caster, 10)],
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    engine::damage(
        &mut state,
        "Striker A",
        "20",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    state.combatants[striker].death_saves.failures = 2;

    let mut log = Vec::new();
    // The d20 shows 2: a failed death save, which is the third failure.
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[2]), &mut log).unwrap();

    assert!(state.combatants[striker].is_dead());
    assert_eq!(
        state.combatants[state.initiative[state.turn]].name,
        "Caster C"
    );
    assert_eq!(state.round, 1);
    assert!(
        log.iter().any(|line| line.contains("Striker A dies")),
        "{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("Striker A's turn")),
        "{log:?}"
    );
    assert!(
        log.iter().any(|line| line.contains("Caster C's turn")),
        "{log:?}"
    );
}

#[test]
fn concentration_breaks_on_damage_and_removes_created_conditions() {
    let mut state = state("test-fight", &["caster-c"], &[]);
    let options = UseOptions {
        targets: vec!["Brute A".to_string()],
        ..UseOptions::default()
    };
    engine::use_ability(
        &mut state,
        "Caster C",
        "focus-spell",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(state.concentration.contains_key("Caster C"));
    assert!(state.combatants[index(&state, "Caster C")].has_condition(&state.content, "focus-test"));
    assert!(state.combatants[index(&state, "Brute A")].has_condition(&state.content, "marked-test"));

    engine::damage(
        &mut state,
        "Caster C",
        "6",
        Some("fire"),
        false,
        &mut ScriptedRng::new(&[1]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(!state.concentration.contains_key("Caster C"));
    assert!(
        !state.combatants[index(&state, "Caster C")].has_condition(&state.content, "focus-test")
    );
    assert!(
        !state.combatants[index(&state, "Brute A")].has_condition(&state.content, "marked-test")
    );
}

#[test]
fn incapacitating_a_concentrator_ends_concentration() {
    let mut state = state("test-fight", &["caster-c"], &[]);
    let options = UseOptions {
        targets: vec!["Brute A".to_string()],
        ..UseOptions::default()
    };
    engine::use_ability(
        &mut state,
        "Caster C",
        "focus-spell",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(state.concentration.contains_key("Caster C"));

    engine::add_condition(
        &mut state,
        "Caster C",
        "disabled-test",
        &AddConditionOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(!state.concentration.contains_key("Caster C"));
    assert!(
        !state.combatants[index(&state, "Caster C")].has_condition(&state.content, "focus-test")
    );
}

#[test]
fn dormant_creatures_are_excluded_until_activated() {
    let mut state = state("dormant-fight", &["striker-a"], &[]);
    engine::init(
        &mut state,
        &[],
        &mut ScriptedRng::new(&[10]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(state.initiative.len(), 1);
    assert_eq!(
        combat_outcome(&state),
        Some(CombatOutcome::Winner("party".to_string()))
    );
    assert!(engine::attack(
        &mut state,
        "Striker A",
        "Dormant D",
        "test-sword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .is_err());

    engine::activate(
        &mut state,
        "dormant-d",
        &mut ScriptedRng::new(&[11]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(state.initiative.len(), 2);
    assert_eq!(combat_outcome(&state), None);

    let mut rng = ScriptedRng::new(&[15, 2]);
    engine::attack(
        &mut state,
        "Striker A",
        "Dormant D",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(hp(&state, "Dormant D"), 10 - (2 + 2));
}

fn combat_outcome(state: &EncounterState) -> Option<CombatOutcome> {
    engine::combat_outcome(state)
}

#[test]
fn resources_recharge_per_turn_round_short_long_and_rolled() {
    let mut state = state("test-fight", &["striker-a"], &[("recharger-r", "gamma")]);
    let recharger = index(&state, "Recharger R");
    let striker = index(&state, "Striker A");
    let brute = index(&state, "Brute A");
    engine::init(
        &mut state,
        &[(recharger, 30), (striker, 1), (brute, 2)],
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    for resource in state.combatants[recharger].resources.values_mut() {
        resource.current = 0;
    }

    let mut rng = ScriptedRng::new(&[6]);
    let mut log = Vec::new();
    engine::next_turn(&mut state, &mut rng, &mut log).unwrap();
    engine::next_turn(&mut state, &mut rng, &mut log).unwrap();
    engine::next_turn(&mut state, &mut rng, &mut log).unwrap();

    let resources = &state.combatants[recharger].resources;
    assert_eq!(resources["turn-res"].current, 1);
    assert_eq!(resources["round-res"].current, 1);
    assert_eq!(resources["rolled-res"].current, 1, "rolled 5-6 restores");
    assert_eq!(resources["short-res"].current, 0);
    assert_eq!(resources["long-res"].current, 0);

    engine::rest(
        &mut state,
        RestKind::Short,
        Some("Recharger R"),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        state.combatants[recharger].resources["short-res"].current,
        1
    );
    engine::rest(
        &mut state,
        RestKind::Long,
        Some("Recharger R"),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(state.combatants[recharger].resources["long-res"].current, 1);
}

#[test]
fn state_json_round_trip_keeps_the_content_snapshot() {
    let mut state = state(
        "multi-side",
        &["striker-a", "caster-c"],
        &[("recharger-r", "gamma")],
    );
    engine::init(
        &mut state,
        &[],
        &mut ScriptedRng::new(&[10, 11, 12, 13, 14]),
        &mut Vec::new(),
    )
    .unwrap();
    state.log.push("round trip marker".to_string());

    let root = unique_temp_dir("roundtrip");
    let path = root.join("nested").join("state.state.json");
    state.save(&path).unwrap();
    let loaded = EncounterState::load(&path).unwrap();
    assert_eq!(loaded, state);
    assert!(!loaded.content.creatures.is_empty());
    assert!(loaded.content.creatures.get("striker-a").is_some());

    // The snapshot is enough to keep playing without a campaign manifest.
    let mut loaded = loaded;
    let mut rng = ScriptedRng::new(&[20, 1, 2, 3, 4]);
    let mut log = Vec::new();
    engine::attack(
        &mut loaded,
        "Striker A",
        "Brute A",
        "test-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();
    assert!(!log.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn custom_sides_are_all_hostile_and_exclude_dormant_members() {
    let mut state = state("multi-side", &["striker-a"], &[]);
    assert_eq!(combat_outcome(&state), None, "three active sides");

    engine::damage(
        &mut state,
        "Brute A",
        "100",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(combat_outcome(&state), None, "party and rampage remain");

    engine::damage(
        &mut state,
        "Rampage E",
        "100",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        combat_outcome(&state),
        Some(CombatOutcome::Winner("party".to_string()))
    );

    engine::damage(
        &mut state,
        "Striker A",
        "100",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(combat_outcome(&state), Some(CombatOutcome::Draw));
}

#[test]
fn duration_and_state_helpers_are_data_driven() {
    let mut state = state("test-fight", &["striker-a"], &[]);
    let options = AddConditionOptions {
        rounds: Some(2),
        ..AddConditionOptions::default()
    };
    engine::add_condition(
        &mut state,
        "Brute A",
        "hindered",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let condition = &state.combatants[index(&state, "Brute A")].conditions[0];
    assert!(matches!(
        condition.duration,
        Some(combat_sim::state::ActiveDuration::Rounds { remaining: 2 })
    ));
    let _ = Duration {
        rounds: Some(1),
        until: None,
        minutes: None,
        permanent: None,
        ends_on: Vec::new(),
    };
}

#[test]
fn until_subject_ends_condition_at_the_subjects_next_turn() {
    let mut state = state("test-fight", &["striker-a", "caster-c"], &[]);
    let striker = index(&state, "Striker A");
    let caster = index(&state, "Caster C");
    let brute = index(&state, "Brute A");
    engine::init(
        &mut state,
        &[(striker, 30), (caster, 20), (brute, 10)],
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let options = AddConditionOptions {
        until: Some(DurationUntil::EndOfTargetNextTurn),
        until_subject: Some("Caster C".to_string()),
        ..AddConditionOptions::default()
    };
    engine::add_condition(
        &mut state,
        "Striker A",
        "hindered",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    match &state.combatants[striker].conditions[0].duration {
        Some(ActiveDuration::Until {
            subject, end_round, ..
        }) => {
            assert_eq!(subject, "Caster C");
            assert_eq!(*end_round, 1);
        }
        other => panic!("expected an until duration, got {other:?}"),
    }

    let mut log = Vec::new();
    // Striker A ends its own turn first: the condition survives because the
    // subject is Caster C.
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[]), &mut log).unwrap();
    assert_eq!(
        state.combatants[state.initiative[state.turn]].name,
        "Caster C"
    );
    assert!(state.combatants[striker].has_condition(&state.content, "hindered"));

    // Caster C ends its next turn: the condition expires now, before Striker A
    // gets another turn in this round.
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[]), &mut log).unwrap();
    assert_eq!(
        state.combatants[state.initiative[state.turn]].name,
        "Brute A"
    );
    assert!(!state.combatants[striker].has_condition(&state.content, "hindered"));
    assert!(
        log.iter()
            .any(|line| line.contains("Striker A is no longer hindered")),
        "{log:?}"
    );
}

fn unique_temp_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "combat-sim-engine-{label}-{}-{nanos}-{count}",
        std::process::id()
    ))
}
