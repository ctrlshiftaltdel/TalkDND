//! Engine tests: generic fixtures only, no campaign content.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use combat_sim::content::schema::{AbilityName, Duration, DurationUntil};
use combat_sim::content::{ContentRegistry, ContentSources};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{self, AddConditionOptions, AttackOptions, MoveTarget, RestKind, UseOptions};
use combat_sim::space::{Cover, Position};
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

#[test]
fn zone_graph_distance_uses_path_edges_and_elevation() {
    let state = state("zone-empty", &["adept-a"], &[]);
    let space = state.space().expect("zone space");
    let alpha = Position::zone("alpha");
    let beta = Position::zone("beta");
    let gamma = Position::zone("gamma");
    let far = Position::zone("far");
    let high = Position::zone("high-ledge");

    assert_eq!(space.distance(&alpha, &gamma), Some(15));
    assert_eq!(space.distance(&alpha, &alpha), Some(0));
    assert_eq!(space.travel_distance(&beta, &far), Some(20));
    assert_eq!(
        space.distance(&beta, &far),
        Some(10),
        "difficult doubles movement only, not range"
    );
    assert_eq!(space.distance(&alpha, &high), Some(15));
    assert!(space.can_reach(&alpha, &beta, 5));
    assert!(
        !space.can_reach(&alpha, &high, 5),
        "a 10-ft-up ledge blocks 5-ft melee reach"
    );
}

#[test]
fn zone_cover_applies_to_ac_and_dex_saves_and_ignore_cover_skips_it() {
    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "delta")]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Adept A",
        "Sentinel T",
        "test-bow",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 1]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("AC 13+2(half cover)")),
        "{log:?}"
    );

    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "delta")]);
    let options = UseOptions {
        targets: vec!["Sentinel T".to_string()],
        ..UseOptions::default()
    };
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Adept A",
        "cover-save",
        &options,
        &mut ScriptedRng::new(&[10]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("+2[half cover]")),
        "{log:?}"
    );

    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "delta")]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Adept A",
        "cover-save-ignore",
        &options,
        &mut ScriptedRng::new(&[10]),
        &mut log,
    )
    .unwrap();
    assert!(
        !log.iter().any(|line| line.contains("half cover")),
        "ignore_cover must skip the save bonus: {log:?}"
    );
}

#[test]
fn total_cover_rejects_targeting_but_same_zone_attacks_work() {
    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "vault")]);
    let mut log = Vec::new();
    let error = engine::attack(
        &mut state,
        "Adept A",
        "Sentinel T",
        "test-bow",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("total cover"), "{error:#}");

    let sentinel = index(&state, "Sentinel T");
    state.combatants[sentinel].set_zone("alpha");
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Adept A",
        "Sentinel T",
        "test-bow",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 1]),
        &mut log,
    )
    .unwrap();
    assert!(log.iter().any(|line| line.contains("HIT")), "{log:?}");
}

#[test]
fn darkness_imposes_disadvantage_unless_the_attacker_has_darkvision() {
    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "dark-room")]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Adept A",
        "Sentinel T",
        "test-bow",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 1]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("dis: darkness")),
        "{log:?}"
    );

    let mut state = state("zone-empty", &["dusk-scout"], &[("sentinel-t", "dark-room")]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Dusk Scout",
        "Sentinel T",
        "test-bow",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 1]),
        &mut log,
    )
    .unwrap();
    assert!(
        !log.iter().any(|line| line.contains("dis: darkness")),
        "{log:?}"
    );
}

#[test]
fn climbing_up_rolls_and_down_needs_no_check() {
    let mut state = state("zone-empty", &["adept-a"], &[("dormant-d", "high-ledge")]);
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("high-ledge".to_string()),
        false,
        &mut ScriptedRng::new(&[12, 10]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[index(&state, "Adept A")].zone, "high-ledge");
    assert!(
        log.iter().any(|line| line.contains("athletics check") && line.contains("PASS")),
        "{log:?}"
    );
    assert!(!state.combatants[index(&state, "Dormant D")].dormant);
    assert!(log.iter().any(|line| line.contains("activates")), "{log:?}");

    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("alpha".to_string()),
        false,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[index(&state, "Adept A")].zone, "alpha");
    assert!(
        !log.iter().any(|line| line.contains("athletics check")),
        "climbing down must not roll: {log:?}"
    );

    let mut state = state("zone-empty", &["adept-a"], &[("dormant-d", "high-ledge")]);
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("high-ledge".to_string()),
        false,
        &mut ScriptedRng::new(&[5]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[index(&state, "Adept A")].zone, "alpha");
    assert!(
        log.iter().any(|line| line.contains("FAIL") && line.contains("climb")),
        "{log:?}"
    );
}

#[test]
fn locked_doors_stop_movement() {
    let mut state = state("zone-empty", &["adept-a"], &[]);
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("locked-room".to_string()),
        false,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[index(&state, "Adept A")].zone, "beta");
    assert!(
        log.iter().any(|line| line.contains("locked door")),
        "{log:?}"
    );
}

#[test]
fn leaving_a_zone_provokes_an_opportunity_attack() {
    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "alpha")]);
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("beta".to_string()),
        false,
        &mut ScriptedRng::new(&[15, 3]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter()
            .any(|line| line.contains("makes an opportunity attack")),
        "{log:?}"
    );
    assert_eq!(hp(&state, "Adept A"), 15);
    assert_eq!(
        state.combatants[index(&state, "Sentinel T")].reaction_round,
        Some(state.round)
    );

    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "alpha")]);
    engine::add_condition(
        &mut state,
        "Adept A",
        "disengaged-test",
        &AddConditionOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("beta".to_string()),
        false,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(!log.iter().any(|line| line.contains("opportunity attack")), "{log:?}");
    assert_eq!(hp(&state, "Adept A"), 20);

    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "alpha")]);
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("beta".to_string()),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(!log.iter().any(|line| line.contains("opportunity attack")), "{log:?}");

    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "alpha")]);
    engine::add_condition(
        &mut state,
        "Adept A",
        "no-oa-against-test",
        &AddConditionOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("beta".to_string()),
        false,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(!log.iter().any(|line| line.contains("opportunity attack")), "{log:?}");
}

#[test]
fn enter_triggers_respect_once_and_side_filters() {
    let mut state = state("zone-empty", &["adept-a"], &[]);
    let mut log = Vec::new();
    for zone in ["beta", "alpha", "beta"] {
        engine::move_to(
            &mut state,
            "Adept A",
            &MoveTarget::Zone(zone.to_string()),
            true,
            &mut ScriptedRng::new(&[]),
            &mut log,
        )
        .unwrap();
    }
    assert_eq!(
        log.iter().filter(|line| *line == "beta entered").count(),
        1,
        "once triggers fire only once: {log:?}"
    );
    assert_eq!(
        log.iter().filter(|line| *line == "trap fired").count(),
        2,
        "enabled non-once triggers fire every entry: {log:?}"
    );

    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "beta")]);
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Sentinel T",
        &MoveTarget::Zone("delta".to_string()),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        !log.iter().any(|line| line.contains("party delta")),
        "enemy entry must not fire a party-only trigger: {log:?}"
    );
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("delta".to_string()),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("party delta")),
        "party entry must fire it: {log:?}"
    );
}

#[test]
fn zone_occupant_triggers_affect_everyone_in_the_zone() {
    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "gamma")]);
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("gamma".to_string()),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(state.combatants[index(&state, "Adept A")].has_condition(&state.content, "dazed"));
    assert!(state.combatants[index(&state, "Sentinel T")].has_condition(&state.content, "dazed"));
}

#[test]
fn toggle_trigger_disables_an_enter_trigger() {
    let mut state = state("zone-empty", &["adept-a"], &[]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Adept A",
        "toggle-trap",
        &UseOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("now disabled")),
        "{log:?}"
    );
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("beta".to_string()),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        !log.iter().any(|line| *line == "trap fired"),
        "{log:?}"
    );
}

#[test]
fn forced_movement_fires_enter_triggers_without_opportunity_attacks() {
    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "beta")]);
    let options = UseOptions {
        targets: vec!["Sentinel T".to_string()],
        ..UseOptions::default()
    };
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Adept A",
        "shove-test",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| *line == "forced entered"),
        "{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("opportunity attack")),
        "{log:?}"
    );
    assert_ne!(state.combatants[index(&state, "Sentinel T")].zone, "beta");
}

#[test]
fn area_targeting_hits_every_zone_in_radius() {
    let mut state = state("zone-empty", &["adept-a"], &[("sentinel-t", "beta")]);
    let options = UseOptions {
        targets: vec!["beta".to_string()],
        ..UseOptions::default()
    };
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Adept A",
        "zone-burst",
        &options,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(state.combatants[index(&state, "Sentinel T")].has_condition(&state.content, "dazed"));
    assert!(state.combatants[index(&state, "Adept A")].has_condition(&state.content, "dazed"));
}

#[test]
fn interact_runs_direct_effects_and_option_checks() {
    let mut state = state("zone-empty", &["adept-a"], &[]);
    let mut log = Vec::new();
    engine::interact(
        &mut state,
        "Adept A",
        "lever",
        None,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("lever moves")),
        "{log:?}"
    );

    let mut log = Vec::new();
    engine::interact(
        &mut state,
        "Adept A",
        "console",
        Some(1),
        &mut ScriptedRng::new(&[15]),
        &mut log,
    )
    .unwrap();
    assert!(log.iter().any(|line| line.contains("arcana check") && line.contains("PASS")), "{log:?}");
    assert!(log.iter().any(|line| line.contains("glyphs glow")), "{log:?}");

    let mut log = Vec::new();
    engine::interact(
        &mut state,
        "Adept A",
        "console",
        Some(2),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(log.iter().any(|line| line.contains("bell rings")), "{log:?}");

    let error = engine::interact(
        &mut state,
        "Adept A",
        "console",
        None,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("requires --option"), "{error:#}");

    let mut log = Vec::new();
    engine::interact(
        &mut state,
        "Adept A",
        "sigil",
        None,
        &mut ScriptedRng::new(&[2]),
        &mut log,
    )
    .unwrap();
    assert!(log.iter().any(|line| line.contains("rejects")), "{log:?}");
}

#[test]
fn grid_distance_and_line_of_sight_cover() {
    let mut state = state("grid-pos-fight", &["adept-a"], &[]);
    let space = state.space().expect("grid space");
    assert_eq!(
        space.distance(&Position::cell(1, 1), &Position::cell(3, 3)),
        Some(10),
        "diagonals are one 5-ft step"
    );

    let mut state2 = state("grid-fight", &["adept-a"], &[]);
    let adept = index(&state2, "Adept A");
    let sentinel = index(&state2, "Sentinel T 1");
    state2.combatants[adept].set_cell(1, 1, "open");
    state2.combatants[sentinel].set_cell(5, 1, "tables");
    let space = state2.space().unwrap();
    assert_eq!(
        space.cover(&Position::cell(1, 1), &Position::cell(5, 1)),
        Cover::Half
    );
    let mut log = Vec::new();
    engine::attack(
        &mut state2,
        "Adept A",
        "Sentinel T 1",
        "test-bow",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 1]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("AC 13+2(half cover)")),
        "{log:?}"
    );

    let mut state3 = state("grid-fight", &["adept-a"], &[]);
    state3.combatants[index(&state3, "Adept A")].set_cell(1, 2, "open");
    state3.combatants[index(&state3, "Sentinel T 1")].set_cell(5, 2, "open");
    let mut log = Vec::new();
    let error = engine::attack(
        &mut state3,
        "Adept A",
        "Sentinel T 1",
        "test-bow",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("total cover"), "{error:#}");

    let mut state4 = state("grid-fight", &["adept-a"], &[]);
    state4.combatants[index(&state4, "Adept A")].set_cell(1, 4, "statue");
    state4.combatants[index(&state4, "Sentinel T 1")].set_cell(7, 4, "open");
    let space = state4.space().unwrap();
    assert_eq!(
        space.cover(&Position::cell(1, 4), &Position::cell(7, 4)),
        Cover::ThreeQuarters
    );
    assert_eq!(
        space.cover(&Position::cell(2, 1), &Position::cell(3, 1)),
        Cover::None,
        "an adjacent target does not get its own cell's cover"
    );
}

#[test]
fn grid_movement_routes_around_walls_difficult_cells_and_hostiles() {
    let mut state = state("grid-fight", &["adept-a"], &[]);
    let adept = index(&state, "Adept A");
    state.combatants[adept].set_cell(1, 2, "open");
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Cell(3, 2),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[adept].cell(), Some([3, 2]));
    assert_eq!(
        state.combatants[adept].speed_left, 10,
        "the wall forces a 20-ft route instead of a 10-ft diagonal"
    );

    let mut state = state("grid-fight", &["adept-a"], &[]);
    let adept = index(&state, "Adept A");
    state.combatants[adept].set_cell(2, 3, "open");
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Cell(3, 3),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[adept].speed_left, 20);

    let mut state = state("grid-fight", &["adept-a"], &[]);
    let adept = index(&state, "Adept A");
    state.combatants[adept].set_cell(1, 3, "open");
    state.combatants[index(&state, "Sentinel T 1")].set_cell(2, 3, "open");
    state.combatants[index(&state, "Sentinel T 2")].set_cell(6, 1, "tables");
    let space = state.space().unwrap();
    let route = space
        .move_route(
            &Position::cell(1, 3),
            &Position::cell(3, 3),
            &[Position::cell(2, 3)],
        )
        .unwrap();
    assert!(route.iter().all(|step| step.position != Position::cell(2, 3)));
    assert!(route.iter().map(|step| step.cost).sum::<i32>() > 10);
}

#[test]
fn grid_encounters_place_by_zone_and_explicit_position() {
    let state = state("grid-fight", &["adept-a"], &[]);
    assert_eq!(state.combatants[index(&state, "Sentinel T 1")].cell(), Some([5, 1]));
    assert_eq!(state.combatants[index(&state, "Sentinel T 2")].cell(), Some([6, 1]));
    assert_eq!(state.combatants[index(&state, "Adept A")].cell(), Some([6, 3]));
    assert_eq!(state.combatants[index(&state, "Adept A")].zone, "door");

    let state = state("grid-pos-fight", &["adept-a"], &[]);
    assert_eq!(state.combatants[index(&state, "Sentinel T")].cell(), Some([2, 1]));
}

#[test]
fn grid_zone_placement_falls_back_to_the_nearest_free_cell() {
    let options = NewEncounter {
        encounter: "grid-fight".to_string(),
        party: vec!["adept-a".to_string()],
        party_zone: None,
        adds: vec![AddSpec {
            creature: "sentinel-t".to_string(),
            count: 1,
            zone: Some("tables".to_string()),
        }],
    };
    let state = EncounterState::from_encounter(registry(), &options).unwrap();
    let cells: Vec<[i32; 2]> = state
        .combatants
        .iter()
        .filter(|combatant| combatant.creature.id == "sentinel-t")
        .filter_map(|combatant| combatant.cell())
        .collect();
    assert_eq!(cells.len(), 3);
    let unique: std::collections::BTreeSet<[i32; 2]> = cells.iter().copied().collect();
    assert_eq!(unique.len(), 3, "third member falls back to a free cell");
    assert!(
        state.log.iter().any(|line| line.contains("has no free cell")),
        "{:?}",
        state.log
    );
}

#[test]
fn grid_zone_and_exact_destinations_work() {
    let mut state = state("grid-fight", &["adept-a"], &[]);
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Zone("statue".to_string()),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    let adept = index(&state, "Adept A");
    assert_eq!(state.combatants[adept].cell(), Some([1, 4]));
    assert_eq!(state.combatants[adept].zone, "statue");

    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Cell(2, 4),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[adept].cell(), Some([2, 4]));
    assert_eq!(state.combatants[adept].zone, "open");
}

#[test]
fn grid_scene_text_reports_coordinates_and_zone_cells() {
    let state = state("grid-fight", &["adept-a"], &[]);
    let text = engine::scene_text(&state).join("\n");
    assert!(text.contains("Grid 5 ft per cell"), "{text}");
    assert!(text.contains("grid zone tables: (5,1) (6,1)"), "{text}");
    assert!(text.contains("Adept A party 20 HP at (6,3) [door]"), "{text}");
}

#[test]
fn grid_state_round_trip_keeps_positions() {
    let mut state = state("grid-fight", &["adept-a"], &[]);
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Adept A",
        &MoveTarget::Cell(4, 2),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    let root = unique_temp_dir("grid-roundtrip");
    let path = root.join("grid.state.json");
    state.save(&path).unwrap();
    let loaded = EncounterState::load(&path).unwrap();
    assert_eq!(loaded, state);
    assert_eq!(
        loaded.combatants[index(&loaded, "Adept A")].cell(),
        Some([4, 2])
    );
    let _ = std::fs::remove_dir_all(&root);
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
