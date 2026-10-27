//! P8 AI quality gate (TASK-0093 / Issue #177).
//!
//! The same synthetic scenes and seed set are run through three baselines:
//! `LegacyHeuristic`, the single-turn `Utility` planner and the bounded
//! `Search` planner. This file records, rather than hides, parity and
//! regressions: the ordinary tests use small deterministic samples and never
//! assert wall-clock values; the large benchmark is `#[ignore]` and is run
//! explicitly in release mode.
//!
//! All fixtures are synthetic and generic. No campaign, monster, spell or save
//! content is read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;

use combat_sim::ai::{Ai, AiStrategy, SearchConfig};
use combat_sim::content::{ContentRegistry, ContentSources};
use combat_sim::dice;
use combat_sim::engine;
use combat_sim::state::{AddSpec, CombatOutcome, EncounterState, NewEncounter};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

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

fn install_scene(state: &mut EncounterState, scene: serde_json::Value) {
    let scene = serde_json::from_value(scene).expect("synthetic scene should deserialize");
    state.scene = Some(Arc::new(scene));
    state.rebuild_caches();
}

fn objective_scene() -> serde_json::Value {
    json!({
        "id": "quality-objective",
        "name": "Quality Objective",
        "mode": "zones",
        "light": "bright",
        "zones": [
            {"id": "alpha"},
            {"id": "beta"},
            {"id": "isolated"}
        ],
        "edges": [
            {"a": "alpha", "b": "beta", "distance": 5}
        ],
        "objects": [
            {
                "id": "quality-lever",
                "zone": "alpha",
                "name": "Quality Lever",
                "interact": {
                    "effects": [
                        {"toggle_trigger": {"trigger": "quality-gate", "enabled": false}}
                    ]
                }
            }
        ],
        "triggers": [
            {
                "id": "quality-gate",
                "on": "on_enter_area",
                "zone": "beta",
                "effects": [{"note": "gate opened"}]
            }
        ]
    })
}

fn arena_scene() -> serde_json::Value {
    json!({
        "id": "quality-arena",
        "name": "Quality Arena",
        "mode": "zones",
        "light": "bright",
        "zones": [
            {"id": "alpha"},
            {"id": "beta"},
            {"id": "gamma"}
        ],
        "edges": [
            {"a": "alpha", "b": "beta", "distance": 5},
            {"a": "beta", "b": "gamma", "distance": 5}
        ]
    })
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

// ---------------------------------------------------------------------------
// Benchmark model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Strategy {
    Legacy,
    Utility,
    Search,
}

impl Strategy {
    const ALL: [Self; 3] = [Self::Legacy, Self::Utility, Self::Search];

    fn label(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Utility => "utility",
            Self::Search => "search",
        }
    }

    fn prepare(self, state: &EncounterState, config: &SearchConfig) -> Ai {
        match self {
            Self::Legacy => Ai::prepare(state),
            Self::Utility => Ai::prepare_with(state, AiStrategy::Utility),
            Self::Search => Ai::prepare_with_search(state, config.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Objective {
    /// The lever interaction must stop the configured trigger.
    Lever,
    /// The party must win before the round limit.
    PartyWin,
}

#[derive(Debug, Clone)]
struct Scenario {
    key: &'static str,
    state: EncounterState,
    objective: Objective,
    max_rounds: u32,
}

fn scenarios() -> Vec<Scenario> {
    let mut objective = encounter_state("zone-empty", &["ai-striker"], &[("ai-dummy", "delta", 1)]);
    install_scene(&mut objective, objective_scene());
    let dummy = index(&objective, "Dummy X");
    objective.combatants[dummy].set_zone("isolated");

    let mut melee = encounter_state("zone-empty", &["ai-striker"], &[("ai-brute", "beta", 1)]);
    install_scene(&mut melee, arena_scene());
    let mut team = encounter_state(
        "zone-empty",
        &["ai-striker", "ai-ranger"],
        &[("ai-brute", "beta", 1), ("ai-dummy", "beta", 1)],
    );
    install_scene(&mut team, arena_scene());

    vec![
        Scenario {
            key: "objective",
            state: objective,
            objective: Objective::Lever,
            max_rounds: 6,
        },
        Scenario {
            key: "melee",
            state: melee,
            objective: Objective::PartyWin,
            max_rounds: 20,
        },
        Scenario {
            key: "team",
            state: team,
            objective: Objective::PartyWin,
            max_rounds: 20,
        },
    ]
}

#[derive(Debug, Clone, Default)]
struct RunMetrics {
    turns: u32,
    action_turns: u32,
    errors: u32,
    error_text: Vec<String>,
    rounds: u32,
    timed_out: bool,
    party_win: bool,
    draw: bool,
    objective_complete: bool,
    party_hp: i32,
    resource_current: i32,
    resource_max: i32,
    search_fallbacks: u32,
    utility_fallbacks: u32,
    nodes: u64,
    latencies_us: Vec<u64>,
}

impl RunMetrics {
    fn fallback_rate(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            f64::from(self.search_fallbacks + self.utility_fallbacks) / f64::from(self.turns)
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Aggregate {
    runs: u32,
    turns: u32,
    action_turns: u32,
    errors: u32,
    wins: u32,
    draws: u32,
    timeouts: u32,
    objectives: u32,
    total_rounds: u32,
    total_party_hp: i64,
    resource_current: i64,
    resource_max: i64,
    search_fallbacks: u32,
    utility_fallbacks: u32,
    nodes: u64,
    latencies_us: Vec<u64>,
}

impl Aggregate {
    fn record(&mut self, metrics: &RunMetrics) {
        self.runs += 1;
        self.turns += metrics.turns;
        self.action_turns += metrics.action_turns;
        self.errors += metrics.errors;
        self.wins += u32::from(metrics.party_win);
        self.draws += u32::from(metrics.draw);
        self.timeouts += u32::from(metrics.timed_out);
        self.objectives += u32::from(metrics.objective_complete);
        self.total_rounds += metrics.rounds;
        self.total_party_hp += i64::from(metrics.party_hp);
        self.resource_current += i64::from(metrics.resource_current);
        self.resource_max += i64::from(metrics.resource_max);
        self.search_fallbacks += metrics.search_fallbacks;
        self.utility_fallbacks += metrics.utility_fallbacks;
        self.nodes += metrics.nodes;
        self.latencies_us.extend_from_slice(&metrics.latencies_us);
    }

    fn legal_action_rate(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            f64::from(self.action_turns) / f64::from(self.turns)
        }
    }

    fn fallback_rate(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            f64::from(self.search_fallbacks + self.utility_fallbacks) / f64::from(self.turns)
        }
    }

    fn avg_rounds(&self) -> f64 {
        if self.runs == 0 {
            0.0
        } else {
            f64::from(self.total_rounds) / f64::from(self.runs)
        }
    }

    fn avg_party_hp(&self) -> f64 {
        if self.runs == 0 {
            0.0
        } else {
            self.total_party_hp as f64 / f64::from(self.runs)
        }
    }

    fn resource_ratio(&self) -> f64 {
        if self.resource_max == 0 {
            1.0
        } else {
            self.resource_current as f64 / self.resource_max as f64
        }
    }

    fn p50_us(&self) -> u64 {
        percentile(&self.latencies_us, 50.0)
    }

    fn p95_us(&self) -> u64 {
        percentile(&self.latencies_us, 95.0)
    }
}

fn percentile(values: &[u64], percentile: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let last = sorted.len() - 1;
    let rank = ((percentile / 100.0) * last as f64).round() as usize;
    sorted[rank.min(last)]
}

/// A compact pre-turn snapshot used to decide whether the turn produced an
/// engine-accepted action. A wait-only turn is legal but is not counted as an
/// action; the report exposes both turns and action turns.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    hp: i32,
    zone: String,
    pos: Option<combat_sim::space::Position>,
    speed_left: i32,
    action_used: bool,
    bonus_action_used: bool,
}

fn snapshot(state: &EncounterState, actor: usize) -> Snapshot {
    let combatant = &state.combatants[actor];
    Snapshot {
        hp: combatant.hp,
        zone: combatant.zone.clone(),
        pos: combatant.pos.clone(),
        speed_left: combatant.speed_left,
        action_used: combatant.action_used,
        bonus_action_used: combatant.bonus_action_used,
    }
}

fn action_happened(before: &Snapshot, after: &Snapshot) -> bool {
    before != after
}

fn party_hp(state: &EncounterState) -> i32 {
    state
        .combatants
        .iter()
        .filter(|combatant| combatant.side.as_str() == "party")
        .map(|combatant| combatant.hp.max(0))
        .sum()
}

fn resource_totals(state: &EncounterState) -> (i32, i32) {
    let mut current = 0;
    let mut max = 0;
    for combatant in &state.combatants {
        if combatant.side.as_str() != "party" {
            continue;
        }
        for resource in combatant.resources.values() {
            current += resource.current.max(0);
            max += resource.max.max(0);
        }
    }
    (current, max)
}

fn parse_search_nodes(line: &str) -> Option<u64> {
    let rest = line.split("nodes=").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

fn fallback_lines(lines: &[String]) -> (u32, u32) {
    let mut search = 0;
    let mut utility = 0;
    for line in lines {
        if line.contains("search planner fallback")
            || line.contains("search planner failed")
            || line.contains("search planner found no executable candidate")
        {
            search += 1;
        }
        if line.contains("using heuristic fallback") || line.contains("tactical plan failed") {
            utility += 1;
        }
    }
    (search, utility)
}

/// Runs one encounter with the same loop shape as `sim::play`, but keeps the
/// log, per-turn timing and planner telemetry. `sim::play` intentionally drops
/// those, and this benchmark must not change the production simulation path.
fn run_one(
    mut state: EncounterState,
    ai: &Ai,
    seed: u64,
    max_rounds: u32,
) -> (RunMetrics, EncounterState) {
    let mut rng = dice::seeded_rng(seed);
    let mut log: Vec<String> = Vec::new();
    let mut metrics = RunMetrics::default();
    if state.initiative.is_empty() {
        if let Err(error) = engine::init(&mut state, &[], &mut rng, &mut log) {
            metrics.errors += 1;
            let _ = error;
            return (metrics, state);
        }
    }
    loop {
        if let Some(outcome) = engine::combat_outcome(&state) {
            state.outcome = Some(outcome);
            break;
        }
        if state.round > max_rounds {
            metrics.timed_out = true;
            break;
        }
        let Some(actor) = state.current_index() else {
            break;
        };
        if state.combatants.get(actor).is_none() {
            break;
        }
        let before = snapshot(&state, actor);
        let before_log = log.len();
        let started = Instant::now();
        let result = ai.take_turn(&mut state, &mut rng, &mut log);
        let elapsed = started.elapsed();
        metrics.latencies_us.push(elapsed.as_micros() as u64);
        metrics.turns += 1;
        let after = snapshot(&state, actor);
        if action_happened(&before, &after) {
            metrics.action_turns += 1;
        }
        let turn_lines = &log[before_log..];
        let (search, utility) = fallback_lines(turn_lines);
        metrics.search_fallbacks += search;
        metrics.utility_fallbacks += utility;
        for line in turn_lines {
            if let Some(nodes) = parse_search_nodes(line) {
                metrics.nodes += nodes;
            }
        }
        if let Err(error) = result {
            metrics.errors += 1;
            metrics.error_text.push(format!("{error:#}"));
            break;
        }
        if engine::combat_outcome(&state).is_some() {
            continue;
        }
        if let Err(error) = engine::next_turn(&mut state, &mut rng, &mut log) {
            if engine::combat_outcome(&state).is_none() {
                state.outcome = Some(CombatOutcome::Draw);
                let _ = error;
                break;
            }
        }
    }
    let outcome = engine::combat_outcome(&state).or_else(|| state.outcome.clone());
    metrics.rounds = if metrics.timed_out {
        max_rounds
    } else {
        state.round.max(1)
    };
    metrics.party_win =
        matches!(&outcome, Some(CombatOutcome::Winner(side)) if side.as_str() == "party");
    metrics.draw = matches!(&outcome, Some(CombatOutcome::Draw));
    metrics.party_hp = party_hp(&state);
    let (current, max) = resource_totals(&state);
    metrics.resource_current = current;
    metrics.resource_max = max;
    (metrics, state)
}

fn run_scenario(scenario: &Scenario, strategy: Strategy, seed: u64) -> RunMetrics {
    let mut state = scenario.state.clone();
    state.log.clear();
    let config = SearchConfig::default();
    let ai = strategy.prepare(&state, &config);
    let (mut metrics, final_state) = run_one(state, &ai, seed, scenario.max_rounds);
    metrics.objective_complete = match scenario.objective {
        Objective::Lever => !trigger_enabled(&final_state, "quality-gate"),
        Objective::PartyWin => metrics.party_win,
    };
    metrics
}

fn aggregate(
    scenarios: &[Scenario],
    strategy: Strategy,
    seeds: &[u64],
) -> BTreeMap<&'static str, Aggregate> {
    let mut result: BTreeMap<&'static str, Aggregate> = BTreeMap::new();
    for scenario in scenarios {
        let entry = result.entry(scenario.key).or_default();
        for &seed in seeds {
            entry.record(&run_scenario(scenario, strategy, seed));
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Ordinary deterministic quality gate
// ---------------------------------------------------------------------------

#[test]
fn objective_scene_utility_and_search_beat_the_legacy_baseline() {
    let mut state = encounter_state("zone-empty", &["ai-striker"], &[("ai-dummy", "delta", 1)]);
    install_scene(&mut state, objective_scene());
    let dummy = index(&state, "Dummy X");
    state.combatants[dummy].set_zone("isolated");
    let mut seed: u64 = 0x5eed_0093;
    let mut results = BTreeMap::new();
    for strategy in Strategy::ALL {
        let config = SearchConfig::default();
        let ai = strategy.prepare(&state, &config);
        let mut completed = 0;
        let mut errors = 0;
        for _ in 0..4 {
            seed = seed.wrapping_add(0x9e37_79b9);
            let (run, final_state) = run_one(state.clone(), &ai, seed, 6);
            errors += run.errors;
            if !trigger_enabled(&final_state, "quality-gate") {
                completed += 1;
            }
        }
        results.insert(strategy, (completed, errors));
    }
    println!("P8 objective completion: {results:?}");
    assert_eq!(
        results.get(&Strategy::Utility).map(|value| value.1),
        Some(0)
    );
    assert_eq!(results.get(&Strategy::Search).map(|value| value.1), Some(0));
    assert!(
        results[&Strategy::Utility].0 > results[&Strategy::Legacy].0,
        "the objective is the P6 improvement that P8 must not lose: {results:?}"
    );
    assert!(
        results[&Strategy::Search].0 >= results[&Strategy::Utility].0,
        "search must not regress the utility objective baseline: {results:?}"
    );
}

#[test]
fn quality_metrics_are_complete_and_search_stays_bounded() {
    let scenarios = scenarios();
    let config = SearchConfig::default();
    for scenario in &scenarios {
        for strategy in Strategy::ALL {
            for seed in [11, 23] {
                let metrics = run_scenario(scenario, strategy, seed);
                assert_eq!(
                    metrics.errors,
                    0,
                    "{} {}: {:?}",
                    scenario.key,
                    strategy.label(),
                    metrics.error_text
                );
                assert_eq!(
                    metrics.latencies_us.len(),
                    metrics.turns as usize,
                    "every turn is timed"
                );
                assert!(metrics.turns > 0, "{} {}", scenario.key, strategy.label());
            }
        }
    }
    // Search telemetry is parsed from the same log a real run produces.
    for scenario in &scenarios {
        let metrics = run_scenario(scenario, Strategy::Search, 11);
        assert!(metrics.nodes <= u64::from(config.node_budget) * u64::from(metrics.turns));
        assert!(metrics.fallback_rate() <= 1.0);
    }
}

#[test]
fn search_planner_default_budget_matches_the_p8_gate() {
    let defaults = SearchConfig::default();
    assert!((256..=512).contains(&defaults.node_budget));
    assert_eq!(defaults.soft_time_limit_ms, 100);
    assert_eq!(defaults.hard_time_limit_ms, 300);
    assert_eq!(defaults.max_depth, 2);
    assert!(defaults.top_k <= 8);
}

// ---------------------------------------------------------------------------
// Explicit large benchmark (ignored)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "explicit release benchmark; prints hardware, OS, commit and quality metrics"]
fn quality_benchmark_prints_environment_and_metrics() {
    let seeds: Vec<u64> = (0..8)
        .map(|index| 0x5eed_0093_u64.wrapping_add(index * 0x9e37_79b9))
        .collect();
    let scenarios = scenarios();
    println!("{}", environment());
    println!(
        "seed_base=0x{:x} seeds={:?} scenarios={:?}",
        seeds[0],
        seeds,
        scenarios.iter().map(|scene| scene.key).collect::<Vec<_>>()
    );
    for strategy in Strategy::ALL {
        let results = aggregate(&scenarios, strategy, &seeds);
        for scenario in &scenarios {
            let result = &results[scenario.key];
            println!(
                "strategy={} scenario={} runs={} legal_action_rate={:.3} fallback_rate={:.3} \
                 wins={} draws={} timeouts={} objectives={} avg_rounds={:.2} avg_party_hp={:.2} \
                 resource_ratio={:.3} p50_us={} p95_us={} search_fallbacks={} utility_fallbacks={} \
                 nodes={}",
                strategy.label(),
                scenario.key,
                result.runs,
                result.legal_action_rate(),
                result.fallback_rate(),
                result.wins,
                result.draws,
                result.timeouts,
                result.objectives,
                result.avg_rounds(),
                result.avg_party_hp(),
                result.resource_ratio(),
                result.p50_us(),
                result.p95_us(),
                result.search_fallbacks,
                result.utility_fallbacks,
                result.nodes
            );
            assert_eq!(result.errors, 0, "{} {}", scenario.key, strategy.label());
        }
    }
    // Keep the ignored benchmark deterministic: it must not depend on a
    // wall-clock threshold. The printed latency is evidence, not a gate.
}

fn environment() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let kernel = Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split(':').nth(1))
                .map(|part| part.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let rustc = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "os={os} arch={arch} kernel={kernel} cpu={cpu} rustc={rustc} commit={commit} \
         profile={} threads={}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1)
    )
}
