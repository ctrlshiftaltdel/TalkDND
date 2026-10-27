//! P6 tactical utility planner tests.
//!
//! These tests are deliberately synthetic and generic: no campaign, spell,
//! monster or class names. They drive `Ai` through the explicit utility
//! strategy and compare it with the legacy heuristic so improvements and
//! non-improvements are both visible.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use combat_sim::ai::{
    self, Ai, AiStrategy, BehaviourRole, CandidateEvaluation, CandidateKind, MovePurpose,
    UtilityTerms, UtilityWeights, MAX_MOVEMENT_CANDIDATES,
};
use combat_sim::content::schema::{AiAssignment, AiParams, AiTargeting, ConditionDef, Cover};
use combat_sim::content::{ContentRegistry, ContentSource, ContentSources};
use combat_sim::dice;
use combat_sim::state::{ActiveCondition, AddSpec, EncounterState, NewEncounter};

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

fn encounter_state(encounter: &str, party: &[&str], adds: &[(&str, &str, u32)]) -> EncounterState {
    let options = NewEncounter {
        encounter: encounter.to_string(),
        party: party.iter().map(|name| name.to_string()).collect(),
        party_zone: None,
        adds: adds
            .iter()
            .map(|(id, zone, count)| AddSpec {
                creature: id.to_string(),
                count: *count,
                zone: Some(zone.to_string()),
            })
            .collect(),
    };
    EncounterState::from_encounter(registry(), &options).expect("fixture encounter should build")
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

fn install_scene(state: &mut EncounterState, scene: serde_json::Value) {
    let scene = serde_json::from_value(scene).expect("synthetic scene should deserialize");
    state.scene = Some(Arc::new(scene));
    state.rebuild_caches();
}

fn set_cell(state: &mut EncounterState, name: &str, x: i32, y: i32) {
    let index = index(state, name);
    state.combatants[index].set_cell(x, y, "open");
}

fn grid_scene(rows: &[&str]) -> serde_json::Value {
    json!({
        "id": "tactics-grid",
        "name": "Tactics Grid",
        "mode": "grid",
        "light": "bright",
        "grid": {
            "cell": 5,
            "legend": {
                ".": {},
                "#": {"wall": true},
                "h": {"cover": "half"},
                "t": {"cover": "three_quarters"}
            },
            "rows": rows,
        }
    })
}

fn objective_scene() -> serde_json::Value {
    json!({
        "id": "tactics-objective",
        "name": "Tactics Objective",
        "mode": "zones",
        "light": "bright",
        "zones": [
            {"id": "alpha"},
            {"id": "beta"},
            {"id": "gamma"}
        ],
        "edges": [
            {"a": "alpha", "b": "beta", "distance": 5},
            {"a": "alpha", "b": "gamma", "distance": 30},
            {"a": "beta", "b": "gamma", "distance": 30}
        ],
        "objects": [
            {
                "id": "gate-lever",
                "zone": "alpha",
                "name": "Gate Lever",
                "interact": {
                    "effects": [
                        {"toggle_trigger": {"trigger": "gate", "enabled": false}}
                    ]
                }
            },
            {
                "id": "manual-notes",
                "zone": "alpha",
                "name": "Notes",
                "manual": "Only a human can adjudicate this."
            }
        ],
        "triggers": [
            {
                "id": "gate",
                "on": "on_enter_area",
                "zone": "beta",
                "effects": [{"note": "gate opened"}]
            }
        ]
    })
}

fn candidate_target(candidate: &CandidateEvaluation) -> Option<&str> {
    match &candidate.kind {
        CandidateKind::Action { target, .. } => target.as_deref(),
        CandidateKind::Move { .. } | CandidateKind::Interact { .. } | CandidateKind::Wait => None,
    }
}

fn candidate_action(candidate: &CandidateEvaluation) -> Option<&str> {
    match &candidate.kind {
        CandidateKind::Action { action_id, .. } => Some(action_id.as_str()),
        _ => None,
    }
}

fn add_unaware_condition(state: &mut EncounterState, target: &str) {
    let definition: ConditionDef = serde_json::from_value(json!({
        "id": "unaware-test",
        "flags": ["unaware"]
    }))
    .expect("condition should deserialize");
    let mut content = (*state.content).clone();
    content.conditions.insert(
        "unaware-test".to_string(),
        definition,
        ContentSource::new("synthetic-test", None),
    );
    state.content = Arc::new(content);
    let index = index(state, target);
    let condition: ActiveCondition = serde_json::from_value(json!({
        "condition": "unaware-test"
    }))
    .expect("active condition should deserialize");
    state.combatants[index].conditions.push(condition);
}

fn trigger_enabled(state: &EncounterState, id: &str) -> bool {
    let scene = state.scene.as_ref().expect("scene");
    let index = scene
        .triggers
        .iter()
        .position(|trigger| trigger.id.as_deref() == Some(id))
        .expect("trigger exists");
    state.trigger_enabled(&EncounterState::trigger_key(index, &scene.triggers[index]))
}

#[test]
fn utility_terms_have_explicit_weights() {
    let terms = UtilityTerms {
        expected_damage: 1.0,
        ally_risk_reduction: 1.0,
        healing: 1.0,
        control: 1.0,
        objective_progress: 1.0,
        resource_opportunity_cost: 1.0,
        cover_positioning: 1.0,
        cooperation: 1.0,
        retreat_alert_cost: 1.0,
        behaviour_consistency: 1.0,
        movement_cost: 1.0,
    };
    let weights = UtilityWeights {
        expected_damage: 1.0,
        ally_risk_reduction: 1.0,
        healing: 1.0,
        control: 1.0,
        objective_progress: 1.0,
        resource_opportunity_cost: 1.0,
        cover_positioning: 1.0,
        cooperation: 1.0,
        retreat_alert_cost: 1.0,
        behaviour_consistency: 1.0,
        movement_cost: 1.0,
    };
    // Eight positive terms minus three costs.
    assert!((terms.score(&weights) - 5.0).abs() < 1.0e-9);
    assert_eq!(UtilityWeights::DEFAULT, UtilityWeights::DEFAULT);
}

#[test]
fn preview_separates_profile_candidates_and_legality() {
    let mut state = encounter_state("ai-fight", &["ai-avaricious"], &[]);
    begin(&mut state, "Avaricious X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("utility plan");

    assert_eq!(plan.actor, index(&state, "Avaricious X"));
    assert_eq!(plan.profile.target.targeting, AiTargeting::Nearest);
    assert_eq!(plan.profile.role, BehaviourRole::Striker);
    assert!(plan
        .rejected
        .iter()
        .any(|rejected| rejected.action_id == "ai-costly" && rejected.reason.contains("resource")));
    let selected = plan.selected().expect("a candidate should be selected");
    assert!(matches!(selected.kind, CandidateKind::Action { .. }));
    assert_eq!(candidate_action(selected), Some("ai-cantrip"));
}

#[test]
fn target_profile_prefers_lowest_hp_when_configured() {
    let mut state = encounter_state("ai-fight", &["ai-ranger"], &[("ai-brute", "beta", 1)]);
    let dummy = index(&state, "Dummy X");
    state.combatants[dummy].hp = 7;
    begin(&mut state, "Ranger X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("utility plan");
    assert_eq!(plan.profile.target.targeting, AiTargeting::LowestHp);
    let selected = plan.selected().expect("candidate");
    assert_eq!(candidate_target(selected), Some("Dummy X"));
}

#[test]
fn candidate_generation_skips_dormant_dead_and_unaware_targets() {
    let mut state = encounter_state("ai-fight", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    begin(&mut state, "Striker X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);

    let dummy = index(&state, "Dummy X");
    state.combatants[dummy].dormant = true;
    let plan = ai.preview_turn(&state).expect("utility plan");
    assert!(!plan
        .candidates
        .iter()
        .any(|candidate| candidate_target(candidate) == Some("Dummy X")));

    state.combatants[dummy].dormant = false;
    state.combatants[dummy].dead = true;
    let plan = ai.preview_turn(&state).expect("utility plan");
    assert!(!plan
        .candidates
        .iter()
        .any(|candidate| candidate_target(candidate) == Some("Dummy X")));

    state.combatants[dummy].dead = false;
    add_unaware_condition(&mut state, "Dummy X");
    let plan = ai.preview_turn(&state).expect("utility plan");
    assert!(!plan
        .candidates
        .iter()
        .any(|candidate| candidate_target(candidate) == Some("Dummy X")));
    assert!(plan
        .candidates
        .iter()
        .any(|candidate| candidate_target(candidate) == Some("Brute X")));
}

#[test]
fn fixed_seed_and_preview_order_are_deterministic() {
    let mut state = encounter_state("ai-fight", &["ai-ranger"], &[("ai-brute", "gamma", 1)]);
    begin(&mut state, "Ranger X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let first = ai.preview_turn(&state).expect("plan");
    let second = ai.preview_turn(&state).expect("plan");
    let describe = |plan: &ai::TacticalPlan| {
        plan.candidates
            .iter()
            .map(|candidate| (format!("{:?}", candidate.kind), candidate.score))
            .collect::<Vec<_>>()
    };
    assert_eq!(describe(&first), describe(&second));

    let mut first_state = state.clone();
    let mut second_state = state.clone();
    let first_ai = Ai::prepare_with(&first_state, AiStrategy::Utility);
    let second_ai = Ai::prepare_with(&second_state, AiStrategy::Utility);
    let mut first_rng = dice::seeded_rng(77);
    let mut second_rng = dice::seeded_rng(77);
    let mut first_log = Vec::new();
    let mut second_log = Vec::new();
    first_ai
        .take_turn(&mut first_state, &mut first_rng, &mut first_log)
        .expect("first turn");
    second_ai
        .take_turn(&mut second_state, &mut second_rng, &mut second_log)
        .expect("second turn");
    assert_eq!(
        first_log, second_log,
        "same seed and same state should produce the same turn"
    );
}

#[test]
fn hold_point_does_not_chase_beyond_leash() {
    let mut state = encounter_state("ai-fight", &["ai-striker"], &[("ai-sentry", "beta", 1)]);
    let striker = index(&state, "Striker X");
    state.combatants[striker].set_zone("gamma");
    let dummy = index(&state, "Dummy X");
    state.combatants[dummy].set_zone("alpha");
    begin(&mut state, "Sentry X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("plan");
    assert_eq!(plan.profile.role, BehaviourRole::Guardian);
    assert!(!plan
        .candidates
        .iter()
        .any(|candidate| candidate_target(candidate) == Some("Striker X")));

    let mut rng = dice::seeded_rng(1);
    let mut log = Vec::new();
    ai.take_turn(&mut state, &mut rng, &mut log).expect("turn");
    assert_eq!(state.combatants[index(&state, "Sentry X")].zone, "beta");
    assert!(!log.iter().any(|line| line.contains("Sword Test")));
}

#[test]
fn retreat_uses_disengage_and_flee_zone() {
    let mut state = encounter_state(
        "ai-fight",
        &["ai-ranger", "ai-striker"],
        &[("ai-brute", "alpha", 1)],
    );
    let ranger = index(&state, "Ranger X");
    state.combatants[ranger].hp = 4;
    state.combatants[ranger].ai = Some(AiAssignment {
        template: "ai-coward".to_string(),
        params: None,
    });
    begin(&mut state, "Ranger X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("plan");
    assert_eq!(plan.profile.role, BehaviourRole::Retreat);
    assert!(plan.candidates.iter().any(|candidate| {
        matches!(
            &candidate.kind,
            CandidateKind::Move {
                purpose: MovePurpose::Retreat,
                ..
            }
        )
    }));
    assert!(plan.candidates.iter().any(|candidate| {
        candidate_action(candidate) == Some("ai-disengage")
            && matches!(
                &candidate.kind,
                CandidateKind::Action {
                    role: combat_sim::content::schema::AiRole::Defense,
                    ..
                }
            )
    }));

    let mut rng = dice::seeded_rng(3);
    let mut log = Vec::new();
    ai.take_turn(&mut state, &mut rng, &mut log).expect("turn");
    assert_eq!(state.combatants[ranger].zone, "gamma", "{log:?}");
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
fn heal_downed_ally_scores_rescue() {
    let mut state = encounter_state("ai-fight", &["ai-healer", "ai-striker"], &[]);
    let striker = index(&state, "Striker X");
    state.combatants[striker].hp = 0;
    state.combatants[striker].death_saves.stable = true;
    begin(&mut state, "Healer X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("plan");
    let heal = plan
        .candidates
        .iter()
        .find(|candidate| candidate_action(candidate) == Some("ai-heal"))
        .expect("heal candidate");
    assert!(heal.terms.healing > 0.0);
    assert_eq!(candidate_target(heal), Some("Striker X"));

    let mut rng = dice::seeded_rng(4);
    let mut log = Vec::new();
    ai.take_turn(&mut state, &mut rng, &mut log).expect("turn");
    assert!(log.iter().any(|line| line.contains("Heal Test")), "{log:?}");
    assert!(state.combatants[striker].hp > 0);
}

#[test]
fn resource_conservation_keeps_the_limited_resource() {
    let mut state = encounter_state("ai-fight", &["ai-avaricious"], &[]);
    begin(&mut state, "Avaricious X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let mut rng = dice::seeded_rng(5);
    let mut log = Vec::new();
    ai.take_turn(&mut state, &mut rng, &mut log).expect("turn");
    assert!(
        log.iter().any(|line| line.contains("Cantrip Test")),
        "{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("Costly Test")),
        "{log:?}"
    );
    let focus = state.combatants[index(&state, "Avaricious X")]
        .resources
        .get("focus")
        .expect("focus resource");
    assert_eq!(focus.current, 1);
}

#[test]
fn escort_profile_holds_with_ally_and_supports() {
    let mut state = encounter_state(
        "ai-fight",
        &["ai-healer", "ai-striker"],
        &[("ai-brute", "gamma", 1)],
    );
    let striker = index(&state, "Striker X");
    state.combatants[striker].hp = 10;
    let healer = index(&state, "Healer X");
    state.combatants[healer].set_zone("beta");
    state.combatants[striker].set_zone("beta");
    state.combatants[healer].ai = Some(AiAssignment {
        template: "ai-healer".to_string(),
        params: Some(AiParams {
            guard_zone: Some("beta".to_string()),
            leash: Some(5),
            ..AiParams::default()
        }),
    });
    begin(&mut state, "Healer X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("plan");
    assert_eq!(plan.profile.role, BehaviourRole::Escort);
    let mut rng = dice::seeded_rng(6);
    let mut log = Vec::new();
    ai.take_turn(&mut state, &mut rng, &mut log).expect("turn");
    assert_eq!(state.combatants[healer].zone, "beta");
    assert!(log.iter().any(|line| line.contains("Heal Test")), "{log:?}");
}

#[test]
fn cover_candidate_moves_into_cover() {
    let mut state = encounter_state("ai-fight", &["ai-ranger"], &[]);
    install_scene(
        &mut state,
        grid_scene(&[
            "#########",
            "#.......#",
            "#.h.....#",
            "#.......#",
            "#########",
        ]),
    );
    set_cell(&mut state, "Ranger X", 1, 2);
    set_cell(&mut state, "Dummy X", 6, 2);
    begin(&mut state, "Ranger X");
    let ranger = index(&state, "Ranger X");
    state.combatants[ranger].action_used = true;
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("plan");
    let selected = plan.selected().expect("candidate");
    assert!(matches!(
        &selected.kind,
        CandidateKind::Move {
            purpose: MovePurpose::Cover,
            ..
        }
    ));
    let mut rng = dice::seeded_rng(7);
    let mut log = Vec::new();
    ai.take_turn(&mut state, &mut rng, &mut log).expect("turn");
    let cell = state.combatants[ranger].cell().expect("grid cell");
    let scene = state.scene.as_ref().expect("scene");
    let grid = combat_sim::space::GridMap::parse(scene).expect("grid");
    let grid_cell = grid.cell_at(cell[0], cell[1]).expect("cell exists");
    assert_ne!(grid_cell.cover, Cover::None, "{log:?}");
}

#[test]
fn stop_objective_interacts_with_configured_object() {
    let mut state = encounter_state("ai-fight", &["ai-striker"], &[]);
    install_scene(&mut state, objective_scene());
    let striker = index(&state, "Striker X");
    state.combatants[striker].set_zone("alpha");
    let dummy = index(&state, "Dummy X");
    state.combatants[dummy].set_zone("gamma");
    begin(&mut state, "Striker X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("plan");
    let selected = plan.selected().expect("candidate");
    assert!(matches!(
        &selected.kind,
        CandidateKind::Interact { object_id, .. } if object_id == "gate-lever"
    ));
    assert!(
        plan.rejected
            .iter()
            .any(|rejected| rejected.action_id == "manual-notes"
                && rejected.reason.contains("manual"))
    );
    assert!(trigger_enabled(&state, "gate"));

    let mut rng = dice::seeded_rng(8);
    let mut log = Vec::new();
    ai.take_turn(&mut state, &mut rng, &mut log).expect("turn");
    assert!(
        log.iter().any(|line| line.contains("Gate Lever")),
        "{log:?}"
    );
    assert!(!trigger_enabled(&state, "gate"), "{log:?}");
}

#[test]
fn movement_candidates_are_capped_on_a_large_grid() {
    let mut state = encounter_state("ai-fight", &["ai-ranger"], &[]);
    let mut rows = Vec::new();
    rows.push("###############".to_string());
    for _ in 0..13 {
        rows.push("#.............#".to_string());
    }
    rows.push("###############".to_string());
    let row_refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    install_scene(&mut state, grid_scene(&row_refs));
    set_cell(&mut state, "Ranger X", 1, 1);
    set_cell(&mut state, "Dummy X", 13, 13);
    begin(&mut state, "Ranger X");
    let ranger = index(&state, "Ranger X");
    state.combatants[ranger].action_used = true;
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("plan");
    assert!(plan.movement_waypoint_count() > 0);
    assert!(plan.movement_waypoint_count() <= MAX_MOVEMENT_CANDIDATES);
}

#[test]
fn execution_revalidates_before_acting() {
    let mut state = encounter_state("ai-fight", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    begin(&mut state, "Striker X");
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("plan");
    let top = plan.candidates.first().expect("candidate");
    let top_target = candidate_target(top).expect("top target").to_string();
    let other = if top_target == "Dummy X" {
        "Brute X"
    } else {
        "Dummy X"
    };
    let top_index = index(&state, &top_target);
    state.combatants[top_index].dormant = true;
    let mut rng = dice::seeded_rng(9);
    let mut log = Vec::new();
    let acted = ai
        .execute_tactical_plan(&mut state, &plan, &mut rng, &mut log)
        .expect("execution should not panic");
    assert!(acted, "{log:?}");
    assert!(
        !log.iter().any(|line| line.contains(&top_target)),
        "the newly dormant target must not be used: {log:?}"
    );
    assert!(
        log.iter().any(|line| line.contains(other)),
        "the next candidate should be used: {log:?}"
    );
}

#[test]
fn utility_strategy_completes_a_synthetic_encounter() {
    let state = encounter_state("ai-fight", &["ai-striker"], &[]);
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let mut rng = dice::seeded_rng(31);
    let mut log = Vec::new();
    let outcome = combat_sim::sim::play(state, &ai, &mut rng, 20, &mut log)
        .expect("utility-driven encounter should not error");
    assert!(outcome.rounds >= 1);
    assert!(outcome.outcome.is_some() || outcome.timed_out, "{log:?}");
}

#[test]
fn quantitative_comparison_reports_improvement_and_parity() {
    // Objective completion: utility should use the configured interaction while
    // the legacy heuristic walks toward the enemy instead.
    let mut objective_state = encounter_state("ai-fight", &["ai-striker"], &[]);
    install_scene(&mut objective_state, objective_scene());
    let striker = index(&objective_state, "Striker X");
    objective_state.combatants[striker].set_zone("alpha");
    let dummy = index(&objective_state, "Dummy X");
    objective_state.combatants[dummy].set_zone("gamma");
    begin(&mut objective_state, "Striker X");

    let mut legacy_objective = objective_state.clone();
    let mut utility_objective = objective_state.clone();
    let legacy_ai = Ai::prepare(&legacy_objective);
    let utility_ai = Ai::prepare_with(&utility_objective, AiStrategy::Utility);
    let mut rng = dice::seeded_rng(21);
    let mut log = Vec::new();
    legacy_ai
        .take_turn(&mut legacy_objective, &mut rng, &mut log)
        .expect("legacy turn");
    let legacy_completed = !trigger_enabled(&legacy_objective, "gate");
    let mut rng = dice::seeded_rng(21);
    utility_ai
        .take_turn(&mut utility_objective, &mut rng, &mut log)
        .expect("utility turn");
    let utility_completed = !trigger_enabled(&utility_objective, "gate");

    // Damage parity: the baseline melee turn should not get worse.
    let mut melee_state = encounter_state("ai-fight", &["ai-striker"], &[]);
    begin(&mut melee_state, "Striker X");
    let mut legacy_melee = melee_state.clone();
    let mut utility_melee = melee_state.clone();
    let legacy_ai = Ai::prepare(&legacy_melee);
    let utility_ai = Ai::prepare_with(&utility_melee, AiStrategy::Utility);
    let mut rng = dice::seeded_rng(22);
    legacy_ai
        .take_turn(&mut legacy_melee, &mut rng, &mut log)
        .expect("legacy turn");
    let legacy_damage = 40 - legacy_melee.combatants[index(&legacy_melee, "Dummy X")].hp;
    let mut rng = dice::seeded_rng(22);
    utility_ai
        .take_turn(&mut utility_melee, &mut rng, &mut log)
        .expect("utility turn");
    let utility_damage = 40 - utility_melee.combatants[index(&utility_melee, "Dummy X")].hp;

    println!(
        "P6 comparison: objective legacy={legacy_completed} utility={utility_completed}; \
         melee damage legacy={legacy_damage} utility={utility_damage}"
    );
    assert!(utility_completed);
    assert!(!legacy_completed);
    assert_eq!(
        utility_damage, legacy_damage,
        "the utility planner must not regress the baseline attack turn"
    );
}
