//! Release-only search-performance spike for TASK-0095 / Issue #179.
//!
//! The measurements here answer one question: what search shape fits the
//! existing engine? They are deliberately **not** wall-clock assertions in the
//! ordinary test suite. Run the benchmark explicitly:
//!
//! ```text
//! cargo test --release --manifest-path tools/combat-sim/Cargo.toml \
//!     --test ai_search_perf -- --ignored --nocapture
//! ```
//!
//! Every scenario is synthetic (generic names only). No save file, book or DM
//! secret is read. Each measured action/turn starts from a fresh clone of the
//! same prepared state and writes to [`NullSink`]; the reported action/turn
//! time excludes the clone, and clone cost is measured separately.

use std::hint::black_box;
use std::time::{Duration, Instant};

use combat_sim::ai::Ai;
use combat_sim::content::schema::{Ability, AiTemplate, Creature, Encounter, Scene};
use combat_sim::content::{ContentRegistry, ContentSource};
use combat_sim::dice::{self, DiceRng};
use combat_sim::engine::{self, AttackOptions, NullSink, UseOptions};
use combat_sim::sim;
use combat_sim::state::{EncounterState, NewEncounter};
use serde_json::{json, Value};

/// Base seed. Per-metric seeds are derived from it and printed in the header.
const SEED: u64 = 0x5eed_0179;
const ACTION_SAMPLES: usize = 2000;
const TURN_SAMPLES: usize = 1000;
const CLONE_SAMPLES: usize = 1000;
const ROLLOUT_SAMPLES: usize = 500;
const WARMUP: usize = 50;
/// Log size used for the clone+clear sensitivity measurement. A search node
/// normally discards narration, but a naive implementation may clone a state
/// whose log has already grown; this shows what that costs.
const CLONE_LOG_LINES: usize = 256;

fn source() -> ContentSource {
    ContentSource::new("synthetic-179", None)
}

fn from_value<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("synthetic content should deserialize")
}

fn insert_creature(registry: &mut ContentRegistry, value: Value) {
    let creature: Creature = from_value(value);
    let id = creature.id.clone();
    registry.creatures.insert(id, creature, source());
}

fn insert_ability(registry: &mut ContentRegistry, value: Value) {
    let ability: Ability = from_value(value);
    let id = ability.id.clone();
    registry.abilities.insert(id, ability, source());
}

fn insert_template(registry: &mut ContentRegistry, value: Value) {
    let template: AiTemplate = from_value(value);
    let id = template.id.clone();
    registry.ai_templates.insert(id, template, source());
}

fn insert_scene(registry: &mut ContentRegistry, value: Value) {
    let scene: Scene = from_value(value);
    let id = scene.id.clone();
    registry.scenes.insert(id, scene, source());
}

fn insert_encounter(registry: &mut ContentRegistry, value: Value) {
    let encounter: Encounter = from_value(value);
    let id = encounter.id.clone();
    registry.encounters.insert(id, encounter, source());
}

/// One synthetic content set shared by every scenario.
///
/// The creatures cover medium/large/small sizes, melee, ranged, a single-target
/// save spell and a zone burst. Names are deliberately generic.
fn registry() -> ContentRegistry {
    let mut registry = ContentRegistry {
        ruleset: "2024".to_string(),
        ..ContentRegistry::default()
    };

    insert_template(
        &mut registry,
        json!({
            "id": "synthetic-melee",
            "params": {"targeting": "nearest", "range_pref": "melee"}
        }),
    );
    insert_template(
        &mut registry,
        json!({
            "id": "synthetic-ranged",
            "params": {"targeting": "nearest", "range_pref": "ranged", "keep_distance": 20}
        }),
    );

    insert_ability(
        &mut registry,
        json!({
            "id": "synthetic-bolt",
            "name": "Synthetic Bolt",
            "activation": "action",
            "range": 60,
            "target": {"type": "creature", "side": "enemy", "count": 1},
            "effects": [{
                "save": {
                    "ability": "dex",
                    "dc": 14,
                    "half": true,
                    "on_fail": [{"damage": {"dice": "2d8", "type": "fire"}}]
                }
            }],
            "ai": {"role": "damage"}
        }),
    );
    insert_ability(
        &mut registry,
        json!({
            "id": "synthetic-burst",
            "name": "Synthetic Burst",
            "activation": "action",
            "range": 60,
            "target": {"type": "zone"},
            "area": {"radius": 10},
            "effects": [{
                "save": {
                    "ability": "dex",
                    "dc": 14,
                    "half": true,
                    "on_fail": [{"damage": {"dice": "3d6", "type": "fire"}}]
                }
            }],
            "ai": {"role": "damage"}
        }),
    );

    insert_creature(
        &mut registry,
        json!({
            "id": "synthetic-duelist",
            "name": "Synthetic Duelist",
            "side": "party",
            "size": "medium",
            "ac": 15,
            "hp": 500,
            "speed": {"walk": 30},
            "init_bonus": 2,
            "abilities": {"str": 16, "dex": 12, "con": 14, "int": 10, "wis": 10, "cha": 10},
            "attacks": [{
                "id": "synthetic-sword",
                "name": "Synthetic Sword",
                "kind": "melee",
                "ability": "str",
                "to_hit": 5,
                "damage": [{"dice": "1d8+3", "type": "slashing"}],
                "reach": 5
            }],
            "ai": {"template": "synthetic-melee"}
        }),
    );
    insert_creature(
        &mut registry,
        json!({
            "id": "synthetic-dummy",
            "name": "Synthetic Dummy",
            "side": "enemy",
            "size": "medium",
            "ac": 13,
            "hp": 100000,
            "speed": {"walk": 30},
            "init_bonus": 1,
            "abilities": {"str": 10, "dex": 10, "con": 10, "int": 10, "wis": 10, "cha": 10},
            "attacks": []
        }),
    );
    insert_creature(
        &mut registry,
        json!({
            "id": "synthetic-archer",
            "name": "Synthetic Archer",
            "side": "party",
            "size": "medium",
            "ac": 14,
            "hp": 400,
            "speed": {"walk": 30},
            "init_bonus": 3,
            "abilities": {"str": 10, "dex": 16, "con": 12, "int": 10, "wis": 12, "cha": 10},
            "attacks": [{
                "id": "synthetic-bow",
                "name": "Synthetic Bow",
                "kind": "ranged",
                "ability": "dex",
                "to_hit": 6,
                "damage": [{"dice": "1d8+3", "type": "piercing"}],
                "range": [30, 120]
            }],
            "ai": {"template": "synthetic-ranged"}
        }),
    );
    insert_creature(
        &mut registry,
        json!({
            "id": "synthetic-caster",
            "name": "Synthetic Caster",
            "side": "party",
            "size": "medium",
            "ac": 13,
            "hp": 300,
            "speed": {"walk": 30},
            "init_bonus": 2,
            "abilities": {"str": 8, "dex": 12, "con": 12, "int": 16, "wis": 12, "cha": 10},
            "spellcasting": {"ability": "int", "attack": 6, "dc": 14},
            "actions": [{"ref": "synthetic-bolt"}, {"ref": "synthetic-burst"}],
            "ai": {"template": "synthetic-ranged"}
        }),
    );
    insert_creature(
        &mut registry,
        json!({
            "id": "synthetic-brute",
            "name": "Synthetic Brute",
            "side": "party",
            "size": "large",
            "ac": 14,
            "hp": 800,
            "speed": {"walk": 30},
            "init_bonus": 1,
            "abilities": {"str": 18, "dex": 10, "con": 16, "int": 8, "wis": 10, "cha": 8},
            "attacks": [{
                "id": "synthetic-claw",
                "name": "Synthetic Claw",
                "kind": "melee",
                "ability": "str",
                "to_hit": 7,
                "damage": [{"dice": "1d10+4", "type": "slashing"}],
                "reach": 10
            }],
            "multiattack": ["synthetic-claw", "synthetic-claw"],
            "ai": {"template": "synthetic-melee"}
        }),
    );
    insert_creature(
        &mut registry,
        json!({
            "id": "synthetic-runner",
            "name": "Synthetic Runner",
            "side": "enemy",
            "size": "small",
            "ac": 13,
            "hp": 100000,
            "speed": {"walk": 30},
            "init_bonus": 4,
            "abilities": {"str": 10, "dex": 14, "con": 10, "int": 10, "wis": 10, "cha": 10},
            "attacks": []
        }),
    );

    insert_creature(
        &mut registry,
        json!({
            "id": "synthetic-trooper",
            "name": "Synthetic Trooper",
            "side": "party",
            "size": "medium",
            "ac": 14,
            "hp": 30,
            "speed": {"walk": 30},
            "init_bonus": 2,
            "abilities": {"str": 14, "dex": 12, "con": 12, "int": 10, "wis": 10, "cha": 10},
            "attacks": [{
                "id": "synthetic-spear",
                "name": "Synthetic Spear",
                "kind": "melee",
                "ability": "str",
                "to_hit": 4,
                "damage": [{"dice": "1d8+2", "type": "piercing"}],
                "reach": 5
            }],
            "ai": {"template": "synthetic-melee"}
        }),
    );
    insert_creature(
        &mut registry,
        json!({
            "id": "synthetic-thug",
            "name": "Synthetic Thug",
            "side": "enemy",
            "size": "medium",
            "ac": 12,
            "hp": 24,
            "speed": {"walk": 30},
            "init_bonus": 1,
            "abilities": {"str": 14, "dex": 10, "con": 12, "int": 8, "wis": 8, "cha": 8},
            "attacks": [{
                "id": "synthetic-club",
                "name": "Synthetic Club",
                "kind": "melee",
                "ability": "str",
                "to_hit": 4,
                "damage": [{"dice": "1d6+2", "type": "bludgeoning"}],
                "reach": 5
            }],
            "ai": {"template": "synthetic-melee"}
        }),
    );

    insert_scene(
        &mut registry,
        json!({
            "id": "synthetic-arena",
            "name": "Synthetic Arena",
            "mode": "zones",
            "light": "bright",
            "zones": [
                {"id": "alpha"},
                {"id": "beta"},
                {"id": "gamma"}
            ],
            "edges": [
                {"a": "alpha", "b": "beta", "distance": 5},
                {"a": "beta", "b": "gamma", "distance": 25},
                {"a": "alpha", "b": "gamma", "distance": 30}
            ]
        }),
    );
    insert_scene(
        &mut registry,
        json!({
            "id": "synthetic-grid",
            "name": "Synthetic Grid",
            "mode": "grid",
            "light": "bright",
            "grid": {
                "cell": 5,
                "legend": {"#": {"wall": true}, ".": {"zone": "open"}},
                "rows": [
                    "##########",
                    "#........#",
                    "#........#",
                    "#........#",
                    "#........#",
                    "#........#",
                    "#........#",
                    "##########"
                ]
            }
        }),
    );

    insert_encounter(
        &mut registry,
        json!({
            "id": "synthetic-melee-duel",
            "name": "Synthetic Melee Duel",
            "scene": "synthetic-arena",
            "party_zone": "beta",
            "members": [
                {"creature": "synthetic-duelist", "side": "party", "zone": "beta"},
                {"creature": "synthetic-dummy", "side": "enemy", "zone": "beta"}
            ]
        }),
    );
    insert_encounter(
        &mut registry,
        json!({
            "id": "synthetic-ranged-skirmish",
            "name": "Synthetic Ranged Skirmish",
            "scene": "synthetic-arena",
            "party_zone": "gamma",
            "members": [
                {"creature": "synthetic-archer", "side": "party", "zone": "gamma"},
                {"creature": "synthetic-dummy", "side": "enemy", "zone": "alpha"}
            ]
        }),
    );
    insert_encounter(
        &mut registry,
        json!({
            "id": "synthetic-spell-single",
            "name": "Synthetic Single-target Spell",
            "scene": "synthetic-arena",
            "party_zone": "beta",
            "members": [
                {"creature": "synthetic-caster", "side": "party", "zone": "beta"},
                {"creature": "synthetic-dummy", "side": "enemy", "zone": "beta"}
            ]
        }),
    );
    insert_encounter(
        &mut registry,
        json!({
            "id": "synthetic-spell-multi",
            "name": "Synthetic Multi-target Spell",
            "scene": "synthetic-arena",
            "party_zone": "beta",
            "members": [
                {"creature": "synthetic-caster", "side": "party", "zone": "beta"},
                {"creature": "synthetic-dummy", "side": "enemy", "count": 3, "zone": "beta"}
            ]
        }),
    );
    insert_encounter(
        &mut registry,
        json!({
            "id": "synthetic-size-grid",
            "name": "Synthetic Size Grid",
            "scene": "synthetic-grid",
            "party_zone": "open",
            "members": [
                {"creature": "synthetic-brute", "side": "party", "pos": [1, 1]},
                {"creature": "synthetic-runner", "side": "enemy", "pos": [3, 1]}
            ]
        }),
    );
    insert_encounter(
        &mut registry,
        json!({
            "id": "synthetic-rollout",
            "name": "Synthetic 4v4 Rollout",
            "scene": "synthetic-arena",
            "party_zone": "beta",
            "members": [
                {"creature": "synthetic-trooper", "side": "party", "count": 4, "zone": "beta"},
                {"creature": "synthetic-thug", "side": "enemy", "count": 4, "zone": "alpha"}
            ]
        }),
    );

    registry
}

#[derive(Clone, Copy)]
enum ActionSpec {
    Attack {
        target: &'static str,
        entry: &'static str,
    },
    Ability {
        target: Option<&'static str>,
        zone: Option<&'static str>,
        entry: &'static str,
    },
}

struct Scenario {
    key: &'static str,
    description: &'static str,
    actor: &'static str,
    action: ActionSpec,
    state: EncounterState,
    ai: Ai,
}

fn build(
    key: &'static str,
    description: &'static str,
    encounter: &str,
    actor: &'static str,
    action: ActionSpec,
) -> Scenario {
    let mut state = EncounterState::from_encounter(
        registry(),
        &NewEncounter {
            encounter: encounter.to_string(),
            ..NewEncounter::default()
        },
    )
    .expect("synthetic encounter should build");
    let actor_index = state.find_index(actor).expect("actor exists");
    // Force the measured actor to the front of initiative and run start-of-turn
    // setup once. Cloning this baseline keeps every sample identical apart from
    // the dice stream.
    let mut rng = dice::seeded_rng(SEED ^ 0x1000);
    engine::init(&mut state, &[(actor_index, 100)], &mut rng, &mut NullSink)
        .expect("synthetic initiative should initialize");
    let ai = Ai::prepare(&state);
    Scenario {
        key,
        description,
        actor,
        action,
        state,
        ai,
    }
}

fn scenarios() -> Vec<Scenario> {
    vec![
        build(
            "melee-duel",
            "medium melee attacker vs medium dummy, same zone",
            "synthetic-melee-duel",
            "Synthetic Duelist",
            ActionSpec::Attack {
                target: "Synthetic Dummy",
                entry: "Synthetic Sword",
            },
        ),
        build(
            "ranged-skirmish",
            "medium ranged attacker at 30 ft vs medium dummy",
            "synthetic-ranged-skirmish",
            "Synthetic Archer",
            ActionSpec::Attack {
                target: "Synthetic Dummy",
                entry: "Synthetic Bow",
            },
        ),
        build(
            "spell-single",
            "single-target save spell (2d8 fire) vs medium dummy",
            "synthetic-spell-single",
            "Synthetic Caster",
            ActionSpec::Ability {
                target: Some("Synthetic Dummy"),
                zone: None,
                entry: "Synthetic Bolt",
            },
        ),
        build(
            "spell-multi",
            "10-ft zone burst (3d6 fire) vs three medium dummies",
            "synthetic-spell-multi",
            "Synthetic Caster",
            ActionSpec::Ability {
                target: None,
                zone: Some("beta"),
                entry: "Synthetic Burst",
            },
        ),
        build(
            "size-grid",
            "large brute single claw vs small dummy on a grid",
            "synthetic-size-grid",
            "Synthetic Brute",
            ActionSpec::Attack {
                target: "Synthetic Runner",
                entry: "Synthetic Claw",
            },
        ),
    ]
}

fn run_action(
    state: &mut EncounterState,
    actor: &str,
    spec: ActionSpec,
    rng: &mut dyn DiceRng,
) -> anyhow::Result<()> {
    let mut sink = NullSink;
    match spec {
        ActionSpec::Attack { target, entry } => {
            let target_index = state.find_index(target)?;
            let attacker_index = state.find_index(actor)?;
            engine::attack_at(
                state,
                attacker_index,
                target_index,
                entry,
                &AttackOptions::default(),
                rng,
                &mut sink,
            )
        }
        ActionSpec::Ability {
            target,
            zone,
            entry,
        } => {
            let mut options = UseOptions::default();
            if let Some(target) = target {
                options.targets.push(target.to_string());
            }
            if let Some(zone) = zone {
                options.targets.push(zone.to_string());
            }
            engine::use_ability(state, actor, entry, &options, rng, &mut sink)
        }
    }
}

struct Summary {
    p50: Duration,
    p95: Duration,
    mean: Duration,
}

fn summarize(samples: &mut [Duration]) -> Summary {
    samples.sort_unstable();
    let len = samples.len();
    let p50 = samples[((len as f64 * 0.50).ceil() as usize)
        .saturating_sub(1)
        .min(len - 1)];
    let p95 = samples[((len as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(len - 1)];
    let total: Duration = samples.iter().sum();
    Summary {
        p50,
        p95,
        mean: total / len as u32,
    }
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn percentile_text(summary: &Summary) -> String {
    format!(
        "{:>10.2} {:>10.2} {:>10.2}",
        micros(summary.p50),
        micros(summary.p95),
        micros(summary.mean)
    )
}

fn measure_action(scenario: &Scenario) -> Summary {
    let mut rng = dice::seeded_rng(SEED ^ 0x2000);
    for _ in 0..WARMUP {
        let mut state = scenario.state.clone();
        run_action(&mut state, scenario.actor, scenario.action, &mut rng)
            .expect("warm-up action should resolve");
    }
    {
        // Guard against measuring a rejected/no-op action: the primitive must
        // actually spend the actor's action.
        let mut probe = scenario.state.clone();
        run_action(&mut probe, scenario.actor, scenario.action, &mut rng)
            .expect("probe action should resolve");
        let actor = probe.find_index(scenario.actor).expect("actor exists");
        assert!(
            probe.combatants[actor].action_used,
            "{} probe did not spend an action",
            scenario.key
        );
    }
    let mut samples = Vec::with_capacity(ACTION_SAMPLES);
    for _ in 0..ACTION_SAMPLES {
        let mut state = scenario.state.clone();
        let started = Instant::now();
        run_action(&mut state, scenario.actor, scenario.action, &mut rng)
            .expect("measured action should resolve");
        let elapsed = started.elapsed();
        black_box(&state);
        samples.push(elapsed);
    }
    summarize(&mut samples)
}

fn measure_turn(scenario: &Scenario) -> Summary {
    let mut rng = dice::seeded_rng(SEED ^ 0x3000);
    for _ in 0..WARMUP {
        let mut state = scenario.state.clone();
        scenario
            .ai
            .take_turn(&mut state, &mut rng, &mut NullSink)
            .expect("warm-up AI turn should resolve");
    }
    {
        let mut probe = scenario.state.clone();
        scenario
            .ai
            .take_turn(&mut probe, &mut rng, &mut NullSink)
            .expect("probe AI turn should resolve");
        let actor = probe.find_index(scenario.actor).expect("actor exists");
        assert!(
            probe.combatants[actor].action_used,
            "{} AI turn probe did not spend an action",
            scenario.key
        );
    }
    let mut samples = Vec::with_capacity(TURN_SAMPLES);
    for _ in 0..TURN_SAMPLES {
        let mut state = scenario.state.clone();
        let started = Instant::now();
        scenario
            .ai
            .take_turn(&mut state, &mut rng, &mut NullSink)
            .expect("measured AI turn should resolve");
        let elapsed = started.elapsed();
        black_box(&state);
        samples.push(elapsed);
    }
    summarize(&mut samples)
}

/// One complete encounter played to a result. This is the rollout unit MCTS
/// would need; it is reported separately from the per-action/turn scenarios.
fn measure_rollout(state: &EncounterState, ai: &Ai) -> (Summary, Vec<u32>) {
    let mut rng = dice::seeded_rng(SEED ^ 0x4000);
    for _ in 0..WARMUP {
        let outcome = sim::play_quiet(state.clone(), ai, &mut rng, 50)
            .expect("warm-up rollout should resolve");
        black_box(&outcome);
    }
    let mut samples = Vec::with_capacity(ROLLOUT_SAMPLES);
    let mut rounds = Vec::with_capacity(ROLLOUT_SAMPLES);
    for _ in 0..ROLLOUT_SAMPLES {
        let started = Instant::now();
        let outcome = sim::play_quiet(state.clone(), ai, &mut rng, 50)
            .expect("measured rollout should resolve");
        let elapsed = started.elapsed();
        black_box(&outcome);
        samples.push(elapsed);
        rounds.push(outcome.rounds);
    }
    (summarize(&mut samples), rounds)
}

/// Clones a state, then clears its log. `log_lines` overrides the natural log
/// so the sensitivity to accumulated narration is visible.
fn measure_clone(state: &EncounterState, log_lines: Option<usize>) -> Summary {
    let mut logged = state.clone();
    if let Some(lines) = log_lines {
        logged.log = (0..lines)
            .map(|index| {
                format!("synthetic log line {index}: the engine narrates a hit for 7 damage")
            })
            .collect();
    }
    for _ in 0..WARMUP {
        let mut copy = logged.clone();
        copy.log.clear();
        black_box(&copy);
    }
    let mut samples = Vec::with_capacity(CLONE_SAMPLES);
    for _ in 0..CLONE_SAMPLES {
        let started = Instant::now();
        let mut copy = logged.clone();
        copy.log.clear();
        let elapsed = started.elapsed();
        black_box(&copy);
        samples.push(elapsed);
    }
    summarize(&mut samples)
}

fn cpu_model() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(text) = std::fs::read_to_string("/proc/cpuinfo") {
            if let Some(line) = text.lines().find(|line| line.starts_with("model name")) {
                if let Some((_, value)) = line.split_once(':') {
                    return value.trim().to_string();
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(value) = std::env::var("PROCESSOR_IDENTIFIER") {
            if !value.trim().is_empty() {
                return value.trim().to_string();
            }
        }
    }
    "unknown".to_string()
}

fn rustc_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn nodes_in_budget(budget_ms: f64, cost_us: f64) -> f64 {
    (budget_ms * 1000.0) / cost_us.max(f64::EPSILON)
}

fn magnitude(nodes: f64) -> &'static str {
    if nodes < 10.0 {
        "<10"
    } else if nodes < 100.0 {
        "10^1"
    } else if nodes < 1_000.0 {
        "10^2"
    } else if nodes < 10_000.0 {
        "10^3"
    } else if nodes < 100_000.0 {
        "10^4"
    } else if nodes < 1_000_000.0 {
        "10^5"
    } else {
        ">=10^6"
    }
}

fn format_nodes(value: f64) -> String {
    if value >= 100_000.0 {
        format!("{:.1}e5", value / 100_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

#[test]
#[ignore = "release-only search performance spike; run with --release --ignored --nocapture"]
fn ai_search_perf_spike() {
    if cfg!(debug_assertions) {
        panic!("this benchmark must run with --release; debug numbers are not evidence");
    }

    println!("== ai search performance spike (Issue #179) ==");
    println!(
        "os={} arch={}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("cpu={}", cpu_model());
    println!(
        "logical_cpus={}",
        std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(0)
    );
    println!("compiler={}", rustc_version());
    println!("seed_base=0x{SEED:x}");
    println!(
        "samples: action={ACTION_SAMPLES} turn={TURN_SAMPLES} clone={CLONE_SAMPLES} warmup={WARMUP}"
    );
    println!("clone_log_lines={CLONE_LOG_LINES}");
    println!("sink=NullSink build=release");

    let scenarios = scenarios();
    let mut rows = Vec::new();

    println!();
    println!("== raw measurements (microseconds) ==");
    println!(
        "{:<16} {:<14} {:>5} {:>10} {:>10} {:>10}",
        "scenario", "metric", "n", "p50", "p95", "mean"
    );
    for scenario in &scenarios {
        let action = measure_action(scenario);
        let turn = measure_turn(scenario);
        let clone = measure_clone(&scenario.state, None);
        let clone_logged = measure_clone(&scenario.state, Some(CLONE_LOG_LINES));
        println!(
            "{:<16} {:<14} {:>5} {}",
            scenario.key,
            "action",
            ACTION_SAMPLES,
            percentile_text(&action)
        );
        println!(
            "{:<16} {:<14} {:>5} {}",
            scenario.key,
            "turn",
            TURN_SAMPLES,
            percentile_text(&turn)
        );
        println!(
            "{:<16} {:<14} {:>5} {}",
            scenario.key,
            "clone+clear",
            CLONE_SAMPLES,
            percentile_text(&clone)
        );
        println!(
            "{:<16} {:<14} {:>5} {}",
            scenario.key,
            "clone+256log",
            CLONE_SAMPLES,
            percentile_text(&clone_logged)
        );
        rows.push((scenario, action, turn, clone));
    }

    println!();
    println!("== search-node budgets (p50; one node = clone+clear + one action) ==");
    println!(
        "{:<16} {:>14} {:>14} {:>14}",
        "scenario", "100 ms", "300 ms", "1000 ms"
    );
    for (scenario, action, _turn, clone) in &rows {
        let cost = micros(action.p50) + micros(clone.p50);
        let nodes_100 = nodes_in_budget(100.0, cost);
        let nodes_300 = nodes_in_budget(300.0, cost);
        let nodes_1000 = nodes_in_budget(1000.0, cost);
        println!(
            "{:<16} {:>14} {:>14} {:>14}",
            scenario.key,
            format!("{} ({})", format_nodes(nodes_100), magnitude(nodes_100)),
            format!("{} ({})", format_nodes(nodes_300), magnitude(nodes_300)),
            format!("{} ({})", format_nodes(nodes_1000), magnitude(nodes_1000)),
        );
    }

    println!();
    println!("== turn-level budgets (p50; one node = clone+clear + one AI turn) ==");
    println!(
        "{:<16} {:>14} {:>14} {:>14}",
        "scenario", "100 ms", "300 ms", "1000 ms"
    );
    for (scenario, _action, turn, clone) in &rows {
        let cost = micros(turn.p50) + micros(clone.p50);
        let nodes_100 = nodes_in_budget(100.0, cost);
        let nodes_300 = nodes_in_budget(300.0, cost);
        let nodes_1000 = nodes_in_budget(1000.0, cost);
        println!(
            "{:<16} {:>14} {:>14} {:>14}",
            scenario.key,
            format!("{} ({})", format_nodes(nodes_100), magnitude(nodes_100)),
            format!("{} ({})", format_nodes(nodes_300), magnitude(nodes_300)),
            format!("{} ({})", format_nodes(nodes_1000), magnitude(nodes_1000)),
        );
    }

    println!();
    println!("== complete encounter rollout (synthetic 4v4, max_rounds=50) ==");
    let mut rollout_state = EncounterState::from_encounter(
        registry(),
        &NewEncounter {
            encounter: "synthetic-rollout".to_string(),
            ..NewEncounter::default()
        },
    )
    .expect("synthetic rollout encounter should build");
    rollout_state.log.clear();
    let rollout_ai = Ai::prepare(&rollout_state);
    let (rollout, mut rounds) = measure_rollout(&rollout_state, &rollout_ai);
    rounds.sort_unstable();
    let median_rounds = rounds.get(rounds.len() / 2).copied().unwrap_or(0);
    println!(
        "runs={} p50_us={:.2} p95_us={:.2} mean_us={:.2} runs_per_sec={:.0} median_rounds={}",
        ROLLOUT_SAMPLES,
        micros(rollout.p50),
        micros(rollout.p95),
        micros(rollout.mean),
        ROLLOUT_SAMPLES as f64 / rollout.mean.as_secs_f64().max(f64::EPSILON),
        median_rounds
    );
    for budget in [100.0, 300.0, 1000.0] {
        let count = nodes_in_budget(budget, micros(rollout.p50));
        println!(
            "rollouts_in_{budget:.0}ms={} ({})",
            format_nodes(count),
            magnitude(count)
        );
    }

    println!();
    println!("== scenarios ==");
    for scenario in &scenarios {
        println!("{}: {}", scenario.key, scenario.description);
    }
}
