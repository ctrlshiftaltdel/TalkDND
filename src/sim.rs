//! Single-run play-by-play and Monte Carlo simulation.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{bail, Result};
use rayon::prelude::*;
use serde::Serialize;

use crate::ai::Ai;
use crate::dice::{self, DiceRng};
use crate::engine::{self, LogSink, NullSink};
use crate::state::{CombatOutcome, EncounterState};

/// Result of one simulated run.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub outcome: Option<CombatOutcome>,
    pub timed_out: bool,
    pub rounds: u32,
    /// Current HP per combatant index, with max HP for reporting.
    pub hp: Vec<i32>,
    pub down: Vec<bool>,
    pub dead: Vec<bool>,
}

/// Runs one encounter to completion (or the round limit).
pub fn play(
    mut state: EncounterState,
    ai: &Ai,
    rng: &mut dyn DiceRng,
    max_rounds: u32,
    log: &mut dyn LogSink,
) -> Result<RunOutcome> {
    if state.initiative.is_empty() {
        engine::init(&mut state, &[], rng, log)?;
    }
    let mut timed_out = false;
    loop {
        if let Some(outcome) = engine::combat_outcome(&state) {
            state.outcome = Some(outcome);
            break;
        }
        if state.round > max_rounds {
            timed_out = true;
            break;
        }
        let Some(actor) = state.current_index() else {
            break;
        };
        if state.combatants.get(actor).is_none() {
            break;
        }
        ai.take_turn(&mut state, rng, log)?;
        if engine::combat_outcome(&state).is_some() {
            continue;
        }
        if let Err(error) = engine::next_turn(&mut state, rng, log) {
            // Nothing left to act with and no recorded side: treat as a draw.
            if engine::combat_outcome(&state).is_none() {
                state.outcome = Some(CombatOutcome::Draw);
                let _ = error;
                break;
            }
        }
    }
    let outcome = engine::combat_outcome(&state).or_else(|| state.outcome.clone());
    Ok(RunOutcome {
        outcome,
        timed_out,
        rounds: state.round.max(1),
        hp: state.combatants.iter().map(|combatant| combatant.hp).collect(),
        down: state
            .combatants
            .iter()
            .map(|combatant| combatant.hp <= 0 && !combatant.is_dead())
            .collect(),
        dead: state
            .combatants
            .iter()
            .map(|combatant| combatant.is_dead())
            .collect(),
    })
}

/// One Monte Carlo run with log output discarded.
pub fn play_quiet(
    state: EncounterState,
    ai: &Ai,
    rng: &mut dyn DiceRng,
    max_rounds: u32,
) -> Result<RunOutcome> {
    play(state, ai, rng, max_rounds, &mut NullSink)
}

#[derive(Debug, Clone)]
pub struct SimulationConfig {
    pub runs: u32,
    pub max_rounds: u32,
    pub threads: usize,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreatureReport {
    pub name: String,
    pub side: String,
    pub max_hp: i32,
    pub avg_hp: f64,
    pub down_pct: f64,
    pub dead_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimulationReport {
    pub runs: u32,
    pub threads: usize,
    pub elapsed_secs: f64,
    pub runs_per_sec: f64,
    pub wins: BTreeMap<String, u64>,
    pub draws: u64,
    pub timeouts: u64,
    pub errors: u64,
    pub avg_rounds: f64,
    pub median_rounds: f64,
    pub avg_hp_per_side: BTreeMap<String, f64>,
    pub creatures: Vec<CreatureReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub error_examples: Vec<String>,
}

#[derive(Clone)]
struct Stats {
    runs: u64,
    wins: Vec<u64>,
    draws: u64,
    timeouts: u64,
    errors: u64,
    rounds: Vec<u32>,
    side_hp: Vec<i64>,
    creature_hp: Vec<i64>,
    down: Vec<u64>,
    dead: Vec<u64>,
    error_examples: Vec<String>,
}

impl Stats {
    fn new(sides: usize, creatures: usize) -> Self {
        Self {
            runs: 0,
            wins: vec![0; sides],
            draws: 0,
            timeouts: 0,
            errors: 0,
            rounds: Vec::new(),
            side_hp: vec![0; sides],
            creature_hp: vec![0; creatures],
            down: vec![0; creatures],
            dead: vec![0; creatures],
            error_examples: Vec::new(),
        }
    }

    fn record(
        &mut self,
        outcome: RunOutcome,
        side_index: &BTreeMap<String, usize>,
        state: &EncounterState,
    ) {
        self.runs += 1;
        self.rounds.push(outcome.rounds);
        match &outcome.outcome {
            Some(CombatOutcome::Winner(side)) => {
                if let Some(index) = side_index.get(side) {
                    self.wins[*index] += 1;
                }
            }
            Some(CombatOutcome::Draw) => self.draws += 1,
            None => {
                if outcome.timed_out {
                    self.timeouts += 1;
                } else {
                    self.draws += 1;
                }
            }
        }
        let mut side_hp: Vec<i32> = vec![0; self.side_hp.len()];
        for (index, combatant) in state.combatants.iter().enumerate() {
            let hp = outcome.hp.get(index).copied().unwrap_or(0).max(0);
            self.creature_hp[index] += i64::from(hp);
            if outcome.down.get(index).copied().unwrap_or(false) {
                self.down[index] += 1;
            }
            if outcome.dead.get(index).copied().unwrap_or(false) {
                self.dead[index] += 1;
            }
            if let Some(side) = side_index.get(combatant.side.as_str()) {
                side_hp[*side] += hp;
            }
        }
        for (index, hp) in side_hp.into_iter().enumerate() {
            self.side_hp[index] += i64::from(hp);
        }
    }

    fn merge(&mut self, other: Stats) {
        self.runs += other.runs;
        for (a, b) in self.wins.iter_mut().zip(other.wins) {
            *a += b;
        }
        self.draws += other.draws;
        self.timeouts += other.timeouts;
        self.errors += other.errors;
        self.rounds.extend(other.rounds);
        for (a, b) in self.side_hp.iter_mut().zip(other.side_hp) {
            *a += b;
        }
        for (a, b) in self.creature_hp.iter_mut().zip(other.creature_hp) {
            *a += b;
        }
        for (a, b) in self.down.iter_mut().zip(other.down) {
            *a += b;
        }
        for (a, b) in self.dead.iter_mut().zip(other.dead) {
            *a += b;
        }
        for example in other.error_examples {
            if self.error_examples.len() < 3 && !self.error_examples.contains(&example) {
                self.error_examples.push(example);
            }
        }
    }
}

/// Runs many independent encounters on a rayon pool.
pub fn monte_carlo(
    template: &EncounterState,
    ai: &Ai,
    config: &SimulationConfig,
) -> Result<SimulationReport> {
    if config.runs == 0 {
        bail!("--runs must be at least 1");
    }
    let mut sides: Vec<String> = template
        .combatants
        .iter()
        .map(|combatant| combatant.side.as_str().to_string())
        .collect();
    sides.sort();
    sides.dedup();
    let side_index: BTreeMap<String, usize> = sides
        .iter()
        .enumerate()
        .map(|(index, side)| (side.clone(), index))
        .collect();
    let threads = config.threads.max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()?;
    let worker = AtomicUsize::new(0);
    let start = Instant::now();
    let stats = pool.install(|| {
        (0..config.runs)
            .into_par_iter()
            .map_init(
                || {
                    let index = worker.fetch_add(1, Ordering::Relaxed);
                    dice::thread_rng(config.seed, index)
                },
                |rng, _| {
                    let mut stats = Stats::new(sides.len(), template.combatants.len());
                    let mut run_state = template.clone();
                    run_state.log.clear();
                    match play_quiet(run_state, ai, rng, config.max_rounds) {
                        Ok(outcome) => stats.record(outcome, &side_index, template),
                        Err(error) => {
                            stats.runs += 1;
                            stats.errors += 1;
                            if stats.error_examples.len() < 3 {
                                stats.error_examples.push(format!("{error:#}"));
                            }
                        }
                    }
                    stats
                },
            )
            .reduce(|| Stats::new(sides.len(), template.combatants.len()), |mut a, b| {
                a.merge(b);
                a
            })
    });
    let elapsed = start.elapsed().as_secs_f64();
    let wins: BTreeMap<String, u64> = sides
        .iter()
        .enumerate()
        .map(|(index, side)| (side.clone(), stats.wins[index]))
        .collect();
    let mut rounds = stats.rounds.clone();
    rounds.sort_unstable();
    let median_rounds = if rounds.is_empty() {
        0.0
    } else {
        f64::from(rounds[rounds.len() / 2])
    };
    let avg_rounds = if rounds.is_empty() {
        0.0
    } else {
        rounds.iter().map(|&round| f64::from(round)).sum::<f64>() / rounds.len() as f64
    };
    let completed = stats.runs.max(1) as f64;
    let avg_hp_per_side = sides
        .iter()
        .enumerate()
        .map(|(index, side)| {
            (
                side.clone(),
                stats.side_hp[index] as f64 / completed,
            )
        })
        .collect();
    let creatures = template
        .combatants
        .iter()
        .enumerate()
        .map(|(index, combatant)| CreatureReport {
            name: combatant.name.clone(),
            side: combatant.side.as_str().to_string(),
            max_hp: combatant.max_hp,
            avg_hp: stats.creature_hp[index] as f64 / completed,
            down_pct: stats.down[index] as f64 * 100.0 / completed,
            dead_pct: stats.dead[index] as f64 * 100.0 / completed,
        })
        .collect();
    Ok(SimulationReport {
        runs: config.runs,
        threads,
        elapsed_secs: elapsed,
        runs_per_sec: f64::from(config.runs) / elapsed.max(f64::EPSILON),
        wins,
        draws: stats.draws,
        timeouts: stats.timeouts,
        errors: stats.errors,
        avg_rounds,
        median_rounds,
        avg_hp_per_side,
        creatures,
        error_examples: stats.error_examples,
    })
}
