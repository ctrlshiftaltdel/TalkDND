//! Synthetic AI behaviour tests. No campaign content names.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use combat_sim::ai::Ai;
use combat_sim::content::schema::AiAssignment;
use combat_sim::content::{ContentRegistry, ContentSources};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{self, AddConditionOptions};
use combat_sim::state::{AddSpec, EncounterState, NewEncounter};

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

fn encounter_state(encounter: &str, party: &[&str], adds: &[(&str, &str)]) -> EncounterState {
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

fn begin(state: &mut EncounterState, name: &str) {
    let actor = index(state, name);
    state.initiative = vec![actor];
    state.turn = 0;
    state.round = 1;
    state.outcome = None;
    let speed = state.combatants[actor].base_speed();
    let combatant = &mut state.combatants[actor];
    combatant.action_used = false;
    combatant.bonus_action_used = false;
    combatant.speed_left = speed;
    combatant.reaction_round = None;
    combatant.opening_used = false;
}

fn take_turn(state: &mut EncounterState, values: &[u32]) -> Vec<String> {
    let ai = Ai::prepare(state);
    let mut rng = ScriptedRng::new(values);
    let mut log = Vec::new();
    ai.take_turn(state, &mut rng, &mut log)
        .expect("AI turn should resolve");
    log
}

#[test]
fn ai_damage_role_attacks() {
    let mut state = encounter_state("ai-fight", &["ai-striker"], &[]);
    begin(&mut state, "Striker X");
    let log = take_turn(&mut state, &[15, 5]);
    assert!(log.iter().any(|line| line.contains("Blade Test")), "{log:?}");
    assert!(
        log.iter().any(|line| line.contains("Blade Test") && line.contains("HIT")),
        "{log:?}"
    );
    assert_eq!(state.combatants[index(&state, "Dummy X")].hp, 40 - 7);
}

#[test]
fn ai_heals_a_wounded_ally() {
    let mut state = encounter_state("ai-fight", &["ai-healer", "ai-striker"], &[]);
    let ally = index(&state, "Striker X");
    state.combatants[ally].hp = 5;
    begin(&mut state, "Healer X");
    let log = take_turn(&mut state, &[4]);
    assert!(log.iter().any(|line| line.contains("Heal Test")), "{log:?}");
    assert_eq!(state.combatants[ally].hp, 11);
}

#[test]
fn ai_uses_bonus_self_heal_below_threshold() {
    let mut state = encounter_state("ai-fight", &["ai-regen"], &[]);
    let actor = index(&state, "Regen X");
    state.combatants[actor].hp = 5;
    begin(&mut state, "Regen X");
    let log = take_turn(&mut state, &[4]);
    assert!(
        log.iter().any(|line| line.contains("Self Heal Test")),
        "{log:?}"
    );
    assert_eq!(state.combatants[actor].hp, 11);
}

#[test]
fn ai_uses_buff_and_control_roles() {
    let mut state = encounter_state("ai-fight", &["ai-buffer", "ai-striker"], &[]);
    begin(&mut state, "Buffer X");
    let log = take_turn(&mut state, &[]);
    assert!(log.iter().any(|line| line.contains("Buff Test")), "{log:?}");
    assert!(state.combatants[index(&state, "Buffer X")]
        .has_condition(&state.content, "ai-buffed"));

    let mut state = encounter_state("ai-fight", &["ai-controller"], &[]);
    begin(&mut state, "Controller X");
    let log = take_turn(&mut state, &[1]);
    assert!(log.iter().any(|line| line.contains("Control Test")), "{log:?}");
    assert!(state.combatants[index(&state, "Dummy X")]
        .has_condition(&state.content, "ai-dazed"));
}

#[test]
fn ai_avoids_warded_targets_and_attacks_downed_when_allowed() {
    let mut state = encounter_state(
        "ai-fight",
        &["ai-striker"],
        &[("ai-warded-dummy", "alpha")],
    );
    let actor = index(&state, "Striker X");
    state.combatants[actor].ai = Some(AiAssignment {
        template: "ai-avoid".to_string(),
        params: None,
    });
    begin(&mut state, "Striker X");
    let log = take_turn(&mut state, &[10, 3]);
    assert!(
        log.iter()
            .any(|line| line.contains("attacks Dummy X")),
        "{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("attacks Warded X")),
        "{log:?}"
    );

    let mut state = encounter_state("ai-fight", &["ai-striker", "ai-healer"], &[("ai-brute", "alpha")]);
    let brute = index(&state, "Brute X");
    state.combatants[brute].ai = Some(AiAssignment {
        template: "ai-downed".to_string(),
        params: None,
    });
    let downed = index(&state, "Striker X");
    state.combatants[downed].hp = 0;
    state.combatants[downed].death_saves.stable = true;
    let healer = index(&state, "Healer X");
    state.combatants[healer].set_zone("gamma");
    begin(&mut state, "Brute X");
    let log = take_turn(&mut state, &[10, 3, 10, 3]);
    assert!(
        log.iter().any(|line| line.contains("attacks Striker X")),
        "{log:?}"
    );
}

#[test]
fn ai_resource_threshold_gates_costed_damage() {
    let mut state = encounter_state("ai-fight", &["ai-costed"], &[]);
    begin(&mut state, "Costed X");
    let log = take_turn(&mut state, &[10, 3]);
    assert!(log.iter().any(|line| line.contains("Costly Test")), "{log:?}");

    let mut state = encounter_state("ai-fight", &["ai-avaricious"], &[]);
    begin(&mut state, "Avaricious X");
    let log = take_turn(&mut state, &[10, 3]);
    assert!(log.iter().any(|line| line.contains("Cantrip Test")), "{log:?}");
    assert!(
        !log.iter().any(|line| line.contains("Costly Test")),
        "{log:?}"
    );
}

#[test]
fn ai_flees_toward_flee_zone() {
    let mut state = encounter_state("ai-fight", &["ai-striker"], &[]);
    let actor = index(&state, "Striker X");
    state.combatants[actor].hp = 4;
    state.combatants[actor].ai = Some(AiAssignment {
        template: "ai-coward".to_string(),
        params: None,
    });
    begin(&mut state, "Striker X");
    let log = take_turn(&mut state, &[]);
    assert_eq!(state.combatants[actor].zone, "gamma", "{log:?}");
    assert!(
        !log.iter().any(|line| line.contains("attacks")),
        "{log:?}"
    );
}

#[test]
fn ai_keeps_distance_with_ranged_attack() {
    let mut state = encounter_state("ai-fight", &["ai-ranger"], &[]);
    begin(&mut state, "Ranger X");
    let log = take_turn(&mut state, &[10, 3]);
    assert!(
        log.iter().any(|line| line.contains("moves alpha -> gamma")),
        "{log:?}"
    );
    assert!(log.iter().any(|line| line.contains("Bow Test")), "{log:?}");
}

#[test]
fn ai_guard_and_leash_holds_position() {
    let mut state = encounter_state("ai-fight", &["ai-striker"], &[("ai-sentry", "beta")]);
    let striker = index(&state, "Striker X");
    state.combatants[striker].set_zone("gamma");
    begin(&mut state, "Sentry X");
    let log = take_turn(&mut state, &[10, 3]);
    assert_eq!(state.combatants[index(&state, "Sentry X")].zone, "beta");
    assert!(
        !log.iter().any(|line| line.contains("Sword Test")),
        "{log:?}"
    );
}

#[test]
fn ai_disengages_before_retreat_to_avoid_opportunity_attack() {
    let mut state = encounter_state("ai-fight", &["ai-ranger"], &[("ai-brute", "alpha")]);
    begin(&mut state, "Ranger X");
    let log = take_turn(&mut state, &[10, 3, 10, 3]);
    assert_eq!(state.combatants[index(&state, "Ranger X")].zone, "gamma");
    assert!(
        log.iter().any(|line| line.contains("Disengage Test")),
        "{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("opportunity attack")),
        "{log:?}"
    );
}

#[test]
fn ai_multiattack_uses_all_entries() {
    let mut state = encounter_state("ai-fight", &["ai-striker"], &[("ai-brute", "alpha")]);
    begin(&mut state, "Brute X");
    let log = take_turn(&mut state, &[10, 3, 10, 3]);
    assert!(
        log.iter().any(|line| line.contains("uses multiattack")),
        "{log:?}"
    );
    assert_eq!(
        log.iter()
            .filter(|line| line.contains("attacks Striker X") && line.contains("Claw Test"))
            .count(),
        2,
        "{log:?}"
    );
}

#[test]
fn ai_opening_ability_is_used_once() {
    let mut state = encounter_state("ai-fight", &["ai-opener", "ai-striker"], &[]);
    let ai = Ai::prepare(&state);
    let actor = index(&state, "Opener X");
    begin(&mut state, "Opener X");
    let mut log = Vec::new();
    let mut rng = ScriptedRng::new(&[]);
    ai.take_turn(&mut state, &mut rng, &mut log).unwrap();
    assert!(log.iter().any(|line| line.contains("Buff Test")), "{log:?}");
    assert!(state.combatants[actor].opening_used);

    let combatant = &mut state.combatants[actor];
    combatant.action_used = false;
    combatant.bonus_action_used = false;
    let speed = combatant.base_speed();
    combatant.speed_left = speed;
    let mut log = Vec::new();
    ai.take_turn(&mut state, &mut rng, &mut log).unwrap();
    assert!(
        !log.iter().any(|line| line.contains("Buff Test")),
        "{log:?}"
    );
}

#[test]
fn ai_targeting_lowest_hp_and_dormant_skip() {
    let mut state = encounter_state(
        "ai-fight",
        &["ai-ranger"],
        &[("ai-sentry", "alpha")],
    );
    let sentry = index(&state, "Sentry X");
    state.combatants[sentry].hp = 5;
    begin(&mut state, "Ranger X");
    let log = take_turn(&mut state, &[10, 3]);
    assert!(
        log.iter().any(|line| line.contains("attacks Sentry X")),
        "{log:?}"
    );

    let mut state = encounter_state("ai-fight", &["ai-striker"], &[("ai-dormant", "beta")]);
    begin(&mut state, "Dormant X");
    let log = take_turn(&mut state, &[]);
    assert!(log.is_empty(), "{log:?}");
}

#[test]
fn ai_multiple_sides_are_all_hostile() {
    let mut state = encounter_state("ai-fight", &["ai-striker"], &[]);
    let dummy = index(&state, "Dummy X");
    state.combatants[dummy].side = combat_sim::content::schema::Side::new("third");
    begin(&mut state, "Striker X");
    let log = take_turn(&mut state, &[10, 3]);
    assert!(log.iter().any(|line| line.contains("attacks Dummy X")), "{log:?}");
}

#[test]
fn ai_condition_helpers_use_generic_flags() {
    let mut state = encounter_state("ai-fight", &["ai-controller"], &[]);
    let dummy = index(&state, "Dummy X");
    engine::add_condition(
        &mut state,
        "Dummy X",
        "ai-dazed",
        &AddConditionOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(engine::has_condition_flag(
        &state,
        dummy,
        combat_sim::content::schema::ConditionFlag::SpeedZero
    ));
}
