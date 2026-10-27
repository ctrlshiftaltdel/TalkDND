//! P7 bounded-search planner tests (TASK-0092 / Issue #176).
//!
//! Every fixture here is synthetic and generic. The tests cover the search
//! seam itself, not a claim that search wins any encounter: the comparison test
//! reports parity or a difference and never turns a lucky sample into a
//! guaranteed win rate.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use combat_sim::ai::{
    self, Ai, AiStrategy, CandidateKind, SearchConfig, SearchFallbackReason, SearchPlanner,
};
use combat_sim::content::schema::{AbilityAi, AbilityEntry, AbilityRef, AiRole, ConditionDef};
use combat_sim::content::{ContentRegistry, ContentSource, ContentSources};
use combat_sim::dice::{self, DiceRng};
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

fn begin_duel(state: &mut EncounterState, actor: &str, enemy: &str) {
    let actor = index(state, actor);
    let enemy = index(state, enemy);
    state.initiative = vec![actor, enemy];
    state.turn = 0;
    state.round = 1;
    state.outcome = None;
    for position in [actor, enemy] {
        let speed = state.combatants[position].base_speed();
        let combatant = &mut state.combatants[position];
        combatant.action_used = false;
        combatant.bonus_action_used = false;
        combatant.speed_left = speed;
        combatant.reaction_round = None;
        combatant.opening_used = false;
    }
}

fn add_unaware_condition(state: &mut EncounterState, target: &str) {
    let definition: ConditionDef = serde_json::from_value(json!({
        "id": "unaware-search-test",
        "flags": ["unaware"]
    }))
    .expect("condition should deserialize");
    let mut content = (*state.content).clone();
    content.conditions.insert(
        "unaware-search-test".to_string(),
        definition,
        ContentSource::new("synthetic-search-test", None),
    );
    state.content = Arc::new(content);
    let index = index(state, target);
    let condition: ActiveCondition = serde_json::from_value(json!({
        "condition": "unaware-search-test"
    }))
    .expect("active condition should deserialize");
    state.combatants[index].conditions.push(condition);
}

fn objective_scene() -> serde_json::Value {
    json!({
        "id": "search-objective",
        "name": "Search Objective",
        "mode": "zones",
        "light": "bright",
        "zones": [
            {"id": "alpha"},
            {"id": "beta"},
            {"id": "gamma"},
            {"id": "delta"}
        ],
        "edges": [
            {"a": "alpha", "b": "beta", "distance": 5},
            {"a": "alpha", "b": "gamma", "distance": 30},
            {"a": "beta", "b": "gamma", "distance": 30}
        ],
        "objects": [
            {
                "id": "search-lever",
                "zone": "alpha",
                "name": "Search Lever",
                "interact": {
                    "effects": [
                        {"toggle_trigger": {"trigger": "search-gate", "enabled": false}}
                    ]
                }
            }
        ],
        "triggers": [
            {
                "id": "search-gate",
                "on": "on_enter_area",
                "zone": "beta",
                "effects": [{"note": "gate opened"}]
            }
        ]
    })
}

fn install_scene(state: &mut EncounterState, scene: serde_json::Value) {
    let scene = serde_json::from_value(scene).expect("synthetic scene should deserialize");
    state.scene = Some(Arc::new(scene));
    state.rebuild_caches();
}

fn trigger_enabled(state: &EncounterState, id: &str) -> bool {
    let scene = state.scene.as_ref().expect("scene");
    let position = scene
        .triggers
        .iter()
        .position(|trigger| trigger.id.as_deref() == Some(id))
        .expect("trigger exists");
    state.trigger_enabled(&EncounterState::trigger_key(
        position,
        &scene.triggers[position],
    ))
}

fn candidate_target(candidate: &ai::CandidateEvaluation) -> Option<&str> {
    match &candidate.kind {
        CandidateKind::Action { target, .. } => target.as_deref(),
        CandidateKind::Move { .. } | CandidateKind::Interact { .. } | CandidateKind::Wait => None,
    }
}

fn candidate_label(candidate: &ai::CandidateEvaluation) -> String {
    match &candidate.kind {
        CandidateKind::Action { action_id, .. } => action_id.clone(),
        CandidateKind::Move { purpose, .. } => format!("move:{purpose:?}"),
        CandidateKind::Interact { object_id, .. } => object_id.clone(),
        CandidateKind::Wait => "wait".to_string(),
    }
}

fn plan_keys(plan: &ai::TacticalPlan) -> Vec<String> {
    plan.candidates
        .iter()
        .map(|candidate| format!("{:?}", candidate.kind))
        .collect()
}

/// Deterministic `DiceRng` that records how many real dice it rolled.
struct CountingRng {
    inner: rand::rngs::StdRng,
    rolls: u32,
}

impl CountingRng {
    fn new(seed: u64) -> Self {
        Self {
            inner: dice::seeded_rng(seed),
            rolls: 0,
        }
    }
}

impl DiceRng for CountingRng {
    fn roll_die(&mut self, sides: u32) -> u32 {
        self.rolls += 1;
        self.inner.roll_die(sides)
    }
}

#[test]
fn search_is_opt_in_and_defaults_are_shallow() {
    let state = encounter_state("zone-empty", &["ai-striker"], &[("ai-brute", "beta", 1)]);

    // Default entry points must not silently enable search.
    assert_eq!(Ai::prepare(&state).strategy(), AiStrategy::LegacyHeuristic);
    assert_eq!(
        Ai::prepare_with(&state, AiStrategy::Utility).strategy(),
        AiStrategy::Utility
    );

    let search = Ai::prepare_with_search(&state, SearchConfig::default());
    assert_eq!(search.strategy(), AiStrategy::Search);
    assert_eq!(search.search_config(), Some(&SearchConfig::default()));
    let explicit = Ai::prepare_with(&state, AiStrategy::Search);
    assert_eq!(explicit.strategy(), AiStrategy::Search);
    assert_eq!(explicit.search_config(), Some(&SearchConfig::default()));

    let defaults = SearchConfig::default();
    assert_eq!(defaults.max_depth, 2, "default is shallow: two plies");
    assert_eq!(defaults.top_k, 8);
    assert_eq!(defaults.opponent_top_k, 4);
    assert_eq!(defaults.samples_per_candidate, 2);
    assert_eq!(defaults.node_budget, 384);
    assert_eq!(defaults.soft_time_limit_ms, 100);
    assert_eq!(defaults.hard_time_limit_ms, 300);
}

#[test]
fn search_uses_an_isolated_clone_and_never_consumes_real_dice() {
    let mut state = encounter_state("zone-empty", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    let enemy = index(&state, "Brute X");
    begin_duel(&mut state, "Striker X", "Brute X");
    // A zero movement budget leaves only the attack and the zero-value wait, so
    // utility and search must execute the same root action. Any extra real
    // dice would then show up as a different roll count or log.
    let actor = index(&state, "Striker X");
    state.combatants[actor].speed_left = 0;
    let _ = enemy;

    let ai = Ai::prepare_with_search(&state, SearchConfig::default());
    let before = state.clone();
    let planner = SearchPlanner::new(SearchConfig::default());
    let decision = planner.plan(&ai, &state);

    assert!(decision.fallback.is_none(), "{:?}", decision.fallback);
    assert_eq!(
        state, before,
        "planning must not mutate the real encounter or its log"
    );
    assert!(decision.stats.nodes > 0);
    assert!(decision.stats.root_candidates_evaluated > 0);

    let mut utility_state = state.clone();
    let mut search_state = state.clone();
    let utility_ai = Ai::prepare_with(&utility_state, AiStrategy::Utility);
    let search_ai = Ai::prepare_with_search(&search_state, SearchConfig::default());
    let mut utility_rng = CountingRng::new(117);
    let mut search_rng = CountingRng::new(117);
    let mut utility_log = Vec::new();
    let mut search_log = Vec::new();
    utility_ai
        .take_turn(&mut utility_state, &mut utility_rng, &mut utility_log)
        .expect("utility turn");
    search_ai
        .take_turn(&mut search_state, &mut search_rng, &mut search_log)
        .expect("search turn");

    assert_eq!(
        utility_rng.rolls, search_rng.rolls,
        "search must not consume the real action RNG"
    );
    let search_execution: Vec<&String> = search_log
        .iter()
        .filter(|line| !line.starts_with("search planner"))
        .collect();
    let utility_execution: Vec<&String> = utility_log
        .iter()
        .filter(|line| !line.starts_with("search planner"))
        .collect();
    assert_eq!(
        utility_execution, search_execution,
        "the real execution must see the same dice stream"
    );
    assert_eq!(
        utility_state.combatants[index(&utility_state, "Brute X")].hp,
        search_state.combatants[index(&search_state, "Brute X")].hp
    );
}

#[test]
fn search_models_probability_samples_and_opponent_responses() {
    let mut state = encounter_state("zone-empty", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    begin_duel(&mut state, "Striker X", "Brute X");
    let ai = Ai::prepare_with_search(&state, SearchConfig::default());

    let one = SearchPlanner::new(SearchConfig {
        samples_per_candidate: 1,
        ..SearchConfig::default()
    })
    .plan(&ai, &state);
    let two = SearchPlanner::new(SearchConfig::default()).plan(&ai, &state);

    assert!(one.fallback.is_none(), "{:?}", one.fallback);
    assert!(two.fallback.is_none(), "{:?}", two.fallback);
    assert_eq!(two.stats.samples_per_candidate, 2);
    assert!(
        two.stats.opponent_responses_evaluated > 0,
        "the next hostile actor must be allowed to respond"
    );
    assert!(
        two.stats.opponent_candidates_considered > 0,
        "opponent responses need candidates, not a free pass"
    );
    assert!(
        two.stats.nodes > one.stats.nodes,
        "a second probability sample must add simulated branches: one={} two={}",
        one.stats.nodes,
        two.stats.nodes
    );

    let plan = two.plan.as_ref().expect("successful search returns a plan");
    assert_eq!(plan.selected_index, Some(0));
    assert!(plan.candidates.iter().any(|candidate| {
        matches!(
            candidate.kind,
            CandidateKind::Action {
                ref role,
                ..
            } if *role != combat_sim::content::schema::AiRole::None
        )
    }));
}

#[test]
fn node_budget_terminates_with_a_structured_fallback() {
    let mut state = encounter_state("zone-empty", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    begin_duel(&mut state, "Striker X", "Brute X");
    let ai = Ai::prepare_with_search(&state, SearchConfig::default());

    let decision = SearchPlanner::new(SearchConfig {
        node_budget: 1,
        samples_per_candidate: 1,
        ..SearchConfig::default()
    })
    .plan(&ai, &state);

    assert_eq!(
        decision.fallback,
        Some(SearchFallbackReason::BudgetExhausted)
    );
    assert!(decision.plan.is_none());
    assert!(decision.stats.nodes <= 1);
    assert!(decision.stats.reproducible_eq(&decision.stats));

    let no_time = SearchPlanner::new(SearchConfig {
        hard_time_limit_ms: 0,
        ..SearchConfig::default()
    })
    .plan(&ai, &state);
    assert_eq!(no_time.fallback, Some(SearchFallbackReason::TimeLimit));

    // Execution-level fallback: the turn must still happen through the
    // utility planner (and never through a half-complete search ranking).
    let fallback_ai = Ai::prepare_with_search(
        &state,
        SearchConfig {
            node_budget: 1,
            samples_per_candidate: 1,
            ..SearchConfig::default()
        },
    );
    let mut fallback_state = state.clone();
    let mut rng = dice::seeded_rng(44);
    let mut log = Vec::new();
    fallback_ai
        .take_turn(&mut fallback_state, &mut rng, &mut log)
        .expect("budget fallback turn");
    assert!(
        log.iter()
            .any(|line| line.contains("using utility fallback")),
        "{log:?}"
    );
    assert!(fallback_state.combatants[index(&fallback_state, "Striker X")].action_used);
}

#[test]
fn duplicate_states_are_pruned_and_counted() {
    let mut state = encounter_state("zone-empty", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    // Two mechanically identical attacks: after execution the lightweight
    // state key is the same, so the second root must reuse the cached value.
    let actor = index(&state, "Striker X");
    {
        let creature = Arc::make_mut(&mut state.combatants[actor].creature);
        let attacks = creature.attacks.as_mut().expect("striker attacks");
        let mut twin = attacks[0].clone();
        twin.id = "ai-blade-twin".to_string();
        twin.name = Some("Blade Twin".to_string());
        attacks.push(twin);
    }
    begin_duel(&mut state, "Striker X", "Brute X");
    let ai = Ai::prepare_with_search(&state, SearchConfig::default());

    let decision = SearchPlanner::new(SearchConfig {
        samples_per_candidate: 1,
        ..SearchConfig::default()
    })
    .plan(&ai, &state);

    assert!(decision.fallback.is_none(), "{:?}", decision.fallback);
    assert!(
        decision.stats.duplicate_states_pruned > 0,
        "identical action outcomes must be recognised: {:#?}",
        decision.stats
    );
    assert!(
        decision.stats.nodes
            < u32::try_from(decision.stats.root_candidates_available * 10).unwrap_or(u32::MAX),
        "deduplication must keep the tree bounded"
    );
}

#[test]
fn fixed_config_is_reproducible_including_across_threads() {
    let mut state = encounter_state("zone-empty", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    begin_duel(&mut state, "Striker X", "Brute X");
    let config = SearchConfig {
        seed: 0x5eed_0092,
        ..SearchConfig::default()
    };
    let ai = Ai::prepare_with_search(&state, config.clone());

    let first = SearchPlanner::new(config.clone()).plan(&ai, &state);
    let second = SearchPlanner::new(config.clone()).plan(&ai, &state);
    let first_plan = first.plan.as_ref().expect("first plan");
    let second_plan = second.plan.as_ref().expect("second plan");
    assert_eq!(plan_keys(first_plan), plan_keys(second_plan));
    assert!(first.stats.reproducible_eq(&second.stats));

    let repeated = std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            let ai = Ai::prepare_with_search(&state, config.clone());
            SearchPlanner::new(config.clone()).plan(&ai, &state)
        });
        let right = scope.spawn(|| {
            let ai = Ai::prepare_with_search(&state, config.clone());
            SearchPlanner::new(config.clone()).plan(&ai, &state)
        });
        (
            left.join().expect("left search thread"),
            right.join().expect("right search thread"),
        )
    });
    let left_plan = repeated.0.plan.as_ref().expect("left plan");
    let right_plan = repeated.1.plan.as_ref().expect("right plan");
    assert_eq!(plan_keys(left_plan), plan_keys(right_plan));
    assert!(repeated.0.stats.reproducible_eq(&repeated.1.stats));
}

#[test]
fn unsupported_effects_fall_back_with_a_reason() {
    let mut state = encounter_state("zone-empty", &["adept-a"], &[("ai-brute", "beta", 1)]);
    let actor = index(&state, "Adept A");
    {
        // `toggle-trap` has no AI role in the fixture; give it one so the
        // utility planner reaches the unsupported-effect rejection instead of
        // rejecting it for "no valuation".
        let mut content = (*state.content).clone();
        let mut ability = content
            .abilities
            .get("toggle-trap")
            .expect("toggle-trap fixture ability")
            .clone();
        ability.ai = Some(AbilityAi {
            role: AiRole::Damage,
        });
        content.abilities.insert(
            "toggle-trap".to_string(),
            ability,
            ContentSource::new("synthetic-search-test", None),
        );
        state.content = Arc::new(content);
        let creature = Arc::make_mut(&mut state.combatants[actor].creature);
        creature.attacks = Some(Vec::new());
        creature.actions = Some(vec![AbilityEntry::Ref(AbilityRef {
            reference: "toggle-trap".to_string(),
            params: Default::default(),
            cost: None,
            name: None,
        })]);
    }
    begin_duel(&mut state, "Adept A", "Brute X");
    state.combatants[actor].speed_left = 0;
    let ai = Ai::prepare_with_search(&state, SearchConfig::default());

    let decision = SearchPlanner::new(SearchConfig::default()).plan(&ai, &state);
    assert_eq!(
        decision.fallback,
        Some(SearchFallbackReason::UnsupportedEffect)
    );
    assert!(decision.plan.is_none());

    let mut search_state = state.clone();
    let mut rng = dice::seeded_rng(9);
    let mut log = Vec::new();
    ai.take_turn(&mut search_state, &mut rng, &mut log)
        .expect("fallback turn must still complete");
    assert!(
        log.iter()
            .any(|line| line.contains("search planner") && line.contains("fallback")),
        "{log:?}"
    );
}

#[test]
fn hidden_enemy_state_does_not_change_the_search_ranking() {
    let mut state = encounter_state(
        "zone-empty",
        &["ai-striker"],
        &[("ai-brute", "beta", 1), ("ai-dummy", "beta", 1)],
    );
    add_unaware_condition(&mut state, "Dummy X");
    begin_duel(&mut state, "Striker X", "Brute X");
    let config = SearchConfig::default();
    let ai = Ai::prepare_with_search(&state, config.clone());

    let hidden = index(&state, "Dummy X");
    let mut wounded = state.clone();
    wounded.combatants[hidden].hp = 1;

    let first = SearchPlanner::new(config.clone()).plan(&ai, &state);
    let second = SearchPlanner::new(config).plan(&ai, &wounded);
    let first_plan = first.plan.as_ref().expect("first plan");
    let second_plan = second.plan.as_ref().expect("second plan");

    assert_eq!(plan_keys(first_plan), plan_keys(second_plan));
    assert!(first.stats.reproducible_eq(&second.stats));
    assert!(!first_plan
        .candidates
        .iter()
        .any(|candidate| candidate_target(candidate) == Some("Dummy X")));
    assert!(first.stats.nodes > 0);
}

#[test]
fn utility_versus_search_comparison_reports_quality_time_nodes_and_fallbacks() {
    // Objective scene: both planners must find the configured interaction. The
    // comparison prints measured quality, time, nodes and fallback rate; it
    // does not claim a general win rate.
    let mut state = encounter_state("zone-empty", &["ai-striker"], &[("ai-dummy", "beta", 1)]);
    install_scene(&mut state, objective_scene());
    let striker = index(&state, "Striker X");
    state.combatants[striker].set_zone("alpha");
    // Keep the opponent active but unable to reach the lever zone. This keeps
    // the quality comparison about objective completion; the hostile-response
    // path is covered by `search_models_probability_samples_and_opponent_responses`.
    let dummy = index(&state, "Dummy X");
    state.combatants[dummy].set_zone("delta");
    begin_duel(&mut state, "Striker X", "Dummy X");

    let utility_ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let utility_plan = utility_ai.preview_turn(&state).expect("utility plan");
    let utility_selected = utility_plan.selected().expect("utility selection");

    let config = SearchConfig::default();
    let search_ai = Ai::prepare_with_search(&state, config.clone());
    let started = std::time::Instant::now();
    let search = SearchPlanner::new(config.clone()).plan(&search_ai, &state);
    let search_elapsed = started.elapsed();
    let search_plan = search.plan.as_ref().expect("search plan");
    let search_selected = search_plan.selected().expect("search selection");

    let mut utility_state = state.clone();
    let mut search_state = state.clone();
    let mut utility_rng = dice::seeded_rng(321);
    let mut search_rng = dice::seeded_rng(321);
    let mut utility_log = Vec::new();
    let mut search_log = Vec::new();
    utility_ai
        .take_turn(&mut utility_state, &mut utility_rng, &mut utility_log)
        .expect("utility turn");
    search_ai
        .take_turn(&mut search_state, &mut search_rng, &mut search_log)
        .expect("search turn");

    let utility_quality = !trigger_enabled(&utility_state, "search-gate");
    let search_quality = !trigger_enabled(&search_state, "search-gate");
    let mut fallbacks = 0;
    for _ in 0..10 {
        let decision = SearchPlanner::new(config.clone()).plan(&search_ai, &state);
        if decision.fallback.is_some() {
            fallbacks += 1;
        }
    }
    let fallback_rate = f64::from(fallbacks) / 10.0;

    println!(
        "P7 comparison: objective utility={utility_quality} search={search_quality}; \
         utility_selected={:?} search_selected={:?}; search_wall_us={} nodes={} \
         roots={} opponent_responses={} duplicates={} fallback_rate={fallback_rate:.2}",
        candidate_label(utility_selected),
        candidate_label(search_selected),
        search_elapsed.as_micros(),
        search.stats.nodes,
        search.stats.root_candidates_evaluated,
        search.stats.opponent_responses_evaluated,
        search.stats.duplicate_states_pruned,
    );

    assert!(utility_quality, "{utility_log:?}");
    assert!(search_quality, "{search_log:?}");
    assert_eq!(candidate_label(utility_selected), "search-lever");
    assert_eq!(candidate_label(search_selected), "search-lever");
    assert!(search.stats.nodes > 0);
    assert!(search.stats.nodes <= search.stats.node_budget);
    assert!(
        fallback_rate <= 0.10,
        "unexpected fallback rate: {fallback_rate}"
    );
}

#[test]
fn search_never_enables_a_hidden_ai_priority_and_stays_observable() {
    // A hidden (unaware) enemy must not become a search target even when it is
    // nearly dead. This is the P6 observable filter, reused by P7.
    let mut state = encounter_state(
        "zone-empty",
        &["ai-striker"],
        &[("ai-brute", "beta", 1), ("ai-dummy", "beta", 1)],
    );
    add_unaware_condition(&mut state, "Dummy X");
    begin_duel(&mut state, "Striker X", "Brute X");
    let hidden = index(&state, "Dummy X");
    state.combatants[hidden].hp = 1;
    let ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let plan = ai.preview_turn(&state).expect("utility plan");
    assert!(!plan
        .candidates
        .iter()
        .any(|candidate| candidate_target(candidate) == Some("Dummy X")));

    let search_ai = Ai::prepare_with_search(&state, SearchConfig::default());
    let decision = SearchPlanner::new(SearchConfig::default()).plan(&search_ai, &state);
    let plan = decision.plan.as_ref().expect("search plan");
    assert!(!plan
        .candidates
        .iter()
        .any(|candidate| candidate_target(candidate) == Some("Dummy X")));
}

#[test]
fn search_strategy_completes_a_synthetic_encounter() {
    let state = encounter_state("zone-empty", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    let ai = Ai::prepare_with_search(&state, SearchConfig::default());
    let mut rng = dice::seeded_rng(31);
    let mut log = Vec::new();
    let outcome = combat_sim::sim::play(state, &ai, &mut rng, 20, &mut log)
        .expect("search-driven encounter should not error");
    assert!(outcome.rounds >= 1);
    assert!(outcome.outcome.is_some() || outcome.timed_out, "{log:?}");
}

#[test]
fn utility_melee_parity_is_reported_not_claimed_as_a_win_rate() {
    let mut state = encounter_state("zone-empty", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    begin_duel(&mut state, "Striker X", "Brute X");
    let actor = index(&state, "Striker X");
    state.combatants[actor].speed_left = 0;

    let utility_ai = Ai::prepare_with(&state, AiStrategy::Utility);
    let search_ai = Ai::prepare_with_search(&state, SearchConfig::default());
    let started = std::time::Instant::now();
    let preview = SearchPlanner::new(SearchConfig::default()).plan(&search_ai, &state);
    let search_wall = started.elapsed();
    assert!(preview.fallback.is_none(), "{:?}", preview.fallback);
    let mut utility_state = state.clone();
    let mut search_state = state.clone();
    let mut utility_rng = dice::seeded_rng(77);
    let mut search_rng = dice::seeded_rng(77);
    let mut utility_log = Vec::new();
    let mut search_log = Vec::new();
    utility_ai
        .take_turn(&mut utility_state, &mut utility_rng, &mut utility_log)
        .expect("utility turn");
    search_ai
        .take_turn(&mut search_state, &mut search_rng, &mut search_log)
        .expect("search turn");

    let utility_damage = 30 - utility_state.combatants[index(&utility_state, "Brute X")].hp;
    let search_damage = 30 - search_state.combatants[index(&search_state, "Brute X")].hp;
    println!(
        "P7 melee parity: utility_damage={utility_damage} search_damage={search_damage} \
         search_wall_us={} nodes={} roots={} opponent_responses={} fallback={:?}",
        search_wall.as_micros(),
        preview.stats.nodes,
        preview.stats.root_candidates_evaluated,
        preview.stats.opponent_responses_evaluated,
        preview.fallback
    );
    assert_eq!(
        utility_damage, search_damage,
        "this deterministic duel should report parity, not a lucky win"
    );
}
