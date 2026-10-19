//! Encounter simulation and Monte Carlo reporting (SPEC §5).

use std::time::Instant;

use anyhow::{bail, Result};
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;
use serde::Serialize;

use crate::ai::Ai;
use crate::dice::{self, DiceRng};
use crate::engine::{self, CombatOutcome, LogSink, NullSink};
use crate::model::Side;
use crate::state::EncounterState;

/// How a single simulated run finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    PartyWins,
    EnemyWins,
    Draw,
    Timeout,
}

/// Result of one simulated encounter.
#[derive(Debug, Clone, Serialize)]
pub struct SingleRun {
    pub outcome: RunOutcome,
    pub rounds: u32,
    pub hp: Vec<i32>,
    pub dead: Vec<bool>,
}

impl SingleRun {
    /// A creature at 0 HP that is not dead counts as down.
    pub fn down(&self, index: usize) -> bool {
        self.hp[index] <= 0 && !self.dead[index]
    }
}

/// Options for a Monte Carlo batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimOptions {
    pub runs: u64,
    pub max_rounds: u32,
    pub seed: Option<u64>,
    pub threads: Option<usize>,
}

/// Per-combatant aggregate statistics.
#[derive(Debug, Clone, Serialize)]
pub struct CombatantStats {
    pub name: String,
    pub side: Side,
    pub avg_hp: f64,
    pub down_rate: f64,
    pub dead_rate: f64,
}

/// Aggregated Monte Carlo results.
#[derive(Debug, Clone, Serialize)]
pub struct MonteCarloReport {
    pub runs: u64,
    pub threads: usize,
    pub elapsed_seconds: f64,
    pub runs_per_second: f64,
    pub party_wins: u64,
    pub enemy_wins: u64,
    pub draws: u64,
    pub timeouts: u64,
    pub party_win_rate: f64,
    pub enemy_win_rate: f64,
    pub draw_rate: f64,
    pub timeout_rate: f64,
    pub avg_rounds: f64,
    pub median_rounds: f64,
    pub avg_party_hp: f64,
    pub avg_enemy_hp: f64,
    pub combatants: Vec<CombatantStats>,
}

/// Plays one encounter in place and returns its result.
pub fn play_run(
    state: &mut EncounterState,
    ai: &Ai,
    max_rounds: u32,
    rng: &mut dyn DiceRng,
    sink: &mut dyn LogSink,
) -> Result<SingleRun> {
    if state.combatants.is_empty() {
        bail!("there are no combatants to simulate");
    }
    if state.initiative.is_empty() || state.round == 0 {
        engine::init(state, rng, sink)?;
    }

    let outcome = loop {
        if let Some(outcome) = engine::combat_outcome(state) {
            break map_outcome(outcome);
        }
        if state.round > max_rounds {
            break RunOutcome::Timeout;
        }

        ai.take_turn(state, rng, sink)?;
        if engine::next_turn(state, rng, sink).is_err() {
            // All combatants died between turns; treat as a draw unless the
            // outcome already says otherwise.
            break engine::combat_outcome(state)
                .map(map_outcome)
                .unwrap_or(RunOutcome::Draw);
        }
    };

    Ok(SingleRun {
        outcome,
        rounds: state.round,
        hp: state
            .combatants
            .iter()
            .map(|combatant| combatant.hp)
            .collect(),
        dead: state
            .combatants
            .iter()
            .map(|combatant| combatant.death_saves.dead)
            .collect(),
    })
}

/// Clones `template`, plays one run, and returns its result.
pub fn run_once(
    template: &EncounterState,
    max_rounds: u32,
    rng: &mut dyn DiceRng,
    sink: &mut dyn LogSink,
) -> Result<SingleRun> {
    let mut state = template.clone();
    state.log.clear();
    let ai = Ai::new(&state)?;
    play_run(&mut state, &ai, max_rounds, rng, sink)
}

fn map_outcome(outcome: CombatOutcome) -> RunOutcome {
    match outcome {
        CombatOutcome::PartyWins => RunOutcome::PartyWins,
        CombatOutcome::EnemyWins => RunOutcome::EnemyWins,
        CombatOutcome::Draw => RunOutcome::Draw,
    }
}

/// Runs a Monte Carlo batch in parallel and aggregates the results.
pub fn monte_carlo(template: EncounterState, options: SimOptions) -> Result<MonteCarloReport> {
    if options.runs == 0 {
        bail!("--runs must be at least 1");
    }
    if options.max_rounds == 0 {
        bail!("--max-rounds must be at least 1");
    }
    if options.threads == Some(0) {
        bail!("--threads must be at least 1");
    }

    let mut template = template;
    template.log.clear();
    let ai = Ai::new(&template)?;

    let start = Instant::now();
    let results = match options.threads {
        Some(threads) => {
            let pool = ThreadPoolBuilder::new().num_threads(threads).build()?;
            pool.install(|| collect_runs(&template, &ai, &options))
        }
        None => collect_runs(&template, &ai, &options),
    }?;
    let elapsed = start.elapsed().as_secs_f64();
    let threads = options.threads.unwrap_or_else(rayon::current_num_threads);

    Ok(build_report(
        options.runs,
        threads,
        elapsed,
        &template,
        &results,
    ))
}

fn collect_runs(
    template: &EncounterState,
    ai: &Ai,
    options: &SimOptions,
) -> Result<Vec<SingleRun>> {
    let max_rounds = options.max_rounds;
    let runs = usize::try_from(options.runs).unwrap_or(usize::MAX);
    let results: Vec<Result<SingleRun>> = match options.seed {
        Some(seed) => (0..runs)
            .into_par_iter()
            .map(|index| {
                let mut rng = dice::thread_rng(Some(seed), index);
                run_quiet(template, ai, max_rounds, &mut rng)
            })
            .collect(),
        None => (0..runs)
            .into_par_iter()
            .map_init(dice::sim_rng, |rng, _| {
                run_quiet(template, ai, max_rounds, rng)
            })
            .collect(),
    };
    results.into_iter().collect()
}

fn run_quiet(
    template: &EncounterState,
    ai: &Ai,
    max_rounds: u32,
    rng: &mut dyn DiceRng,
) -> Result<SingleRun> {
    let mut state = template.clone();
    let mut sink = NullSink;
    play_run(&mut state, ai, max_rounds, rng, &mut sink)
}

fn build_report(
    runs: u64,
    threads: usize,
    elapsed: f64,
    template: &EncounterState,
    results: &[SingleRun],
) -> MonteCarloReport {
    let runs_f = runs as f64;
    let mut party_wins = 0_u64;
    let mut enemy_wins = 0_u64;
    let mut draws = 0_u64;
    let mut timeouts = 0_u64;
    let mut round_total = 0_u64;
    let mut rounds: Vec<u32> = Vec::with_capacity(results.len());
    let mut party_hp_total = 0.0;
    let mut enemy_hp_total = 0.0;

    let mut hp_totals = vec![0.0_f64; template.combatants.len()];
    let mut down_counts = vec![0_u64; template.combatants.len()];
    let mut dead_counts = vec![0_u64; template.combatants.len()];

    for result in results {
        match result.outcome {
            RunOutcome::PartyWins => party_wins += 1,
            RunOutcome::EnemyWins => enemy_wins += 1,
            RunOutcome::Draw => draws += 1,
            RunOutcome::Timeout => timeouts += 1,
        }
        round_total += u64::from(result.rounds);
        rounds.push(result.rounds);

        for (index, combatant) in template.combatants.iter().enumerate() {
            let hp = result.hp[index].max(0);
            hp_totals[index] += f64::from(hp);
            if result.down(index) {
                down_counts[index] += 1;
            }
            if result.dead[index] {
                dead_counts[index] += 1;
            }
            if combatant.side() == Side::Party {
                party_hp_total += f64::from(hp);
            } else {
                enemy_hp_total += f64::from(hp);
            }
        }
    }

    let combatants = template
        .combatants
        .iter()
        .enumerate()
        .map(|(index, combatant)| CombatantStats {
            name: combatant.name().to_string(),
            side: combatant.side(),
            avg_hp: hp_totals[index] / runs_f,
            down_rate: down_counts[index] as f64 / runs_f,
            dead_rate: dead_counts[index] as f64 / runs_f,
        })
        .collect();

    let elapsed = elapsed.max(f64::MIN_POSITIVE);
    MonteCarloReport {
        runs,
        threads,
        elapsed_seconds: elapsed,
        runs_per_second: runs_f / elapsed,
        party_wins,
        enemy_wins,
        draws,
        timeouts,
        party_win_rate: party_wins as f64 / runs_f,
        enemy_win_rate: enemy_wins as f64 / runs_f,
        draw_rate: draws as f64 / runs_f,
        timeout_rate: timeouts as f64 / runs_f,
        avg_rounds: round_total as f64 / runs_f,
        median_rounds: median(&mut rounds),
        avg_party_hp: party_hp_total / runs_f,
        avg_enemy_hp: enemy_hp_total / runs_f,
        combatants,
    }
}

fn median(rounds: &mut [u32]) -> f64 {
    if rounds.is_empty() {
        return 0.0;
    }
    rounds.sort_unstable();
    let middle = rounds.len() / 2;
    if rounds.len() & 1 == 0 {
        (f64::from(rounds[middle - 1]) + f64::from(rounds[middle])) / 2.0
    } else {
        f64::from(rounds[middle])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn repo_path(relative: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    fn guard_template() -> EncounterState {
        EncounterState::from_encounter_file(repo_path("data/encounters/party-vs-2-guards.json"))
            .expect("guard encounter should load")
    }

    #[test]
    fn seeded_run_once_reaches_a_winner() {
        let template = guard_template();
        let mut rng = dice::seeded_rng(1);
        let mut sink = NullSink;
        let run = run_once(&template, 100, &mut rng, &mut sink).unwrap();
        assert_ne!(run.outcome, RunOutcome::Timeout);
        assert!(run.rounds >= 1);
        assert_eq!(run.hp.len(), template.combatants.len());
        assert_eq!(run.dead.len(), template.combatants.len());
        assert!(
            run.outcome == RunOutcome::PartyWins
                || run.outcome == RunOutcome::EnemyWins
                || run.outcome == RunOutcome::Draw
        );
    }

    #[test]
    fn logged_and_quiet_runs_agree() {
        let template = guard_template();
        for seed in 1..=5 {
            let loud =
                run_once(&template, 100, &mut dice::seeded_rng(seed), &mut Vec::new()).unwrap();
            let quiet =
                run_once(&template, 100, &mut dice::seeded_rng(seed), &mut NullSink).unwrap();
            assert_eq!(
                loud.outcome, quiet.outcome,
                "outcome differs for seed {seed}"
            );
            assert_eq!(loud.rounds, quiet.rounds, "rounds differ for seed {seed}");
            assert_eq!(loud.hp, quiet.hp, "hp differs for seed {seed}");
            assert_eq!(loud.dead, quiet.dead, "deaths differ for seed {seed}");
        }
    }

    #[test]
    fn logged_and_quiet_runs_agree_on_complex_encounters() {
        for encounter in [
            "data/encounters/party-vs-mimic.json",
            "data/encounters/party-vs-scarecrow.json",
            "data/encounters/party-vs-satyr-statue.json",
            "data/encounters/party-vs-animatronic-allosaurus.json",
        ] {
            let template = EncounterState::from_encounter_file(repo_path(encounter)).unwrap();
            for seed in 1..=3 {
                let loud =
                    run_once(&template, 100, &mut dice::seeded_rng(seed), &mut Vec::new()).unwrap();
                let quiet =
                    run_once(&template, 100, &mut dice::seeded_rng(seed), &mut NullSink).unwrap();
                assert_eq!(
                    loud.outcome, quiet.outcome,
                    "{encounter} outcome differs for seed {seed}"
                );
                assert_eq!(
                    loud.rounds, quiet.rounds,
                    "{encounter} rounds differ for seed {seed}"
                );
                assert_eq!(loud.hp, quiet.hp, "{encounter} hp differs for seed {seed}");
                assert_eq!(
                    loud.dead, quiet.dead,
                    "{encounter} deaths differ for seed {seed}"
                );
            }
        }
    }

    #[test]
    fn monte_carlo_reports_sane_statistics() {
        let template = guard_template();
        let report = monte_carlo(
            template,
            SimOptions {
                runs: 1000,
                max_rounds: 100,
                seed: Some(3),
                threads: None,
            },
        )
        .unwrap();

        assert_eq!(report.runs, 1000);
        assert!(report.avg_rounds > 0.0);
        assert!(report.median_rounds > 0.0);
        assert!(report.runs_per_second > 0.0);
        assert_eq!(report.combatants.len(), 6);
        let rates =
            report.party_win_rate + report.enemy_win_rate + report.draw_rate + report.timeout_rate;
        assert!((rates - 1.0).abs() < 1e-9, "rates sum to {rates}");
        for combatant in &report.combatants {
            assert!(combatant.down_rate >= 0.0 && combatant.down_rate <= 1.0);
            assert!(combatant.dead_rate >= 0.0 && combatant.dead_rate <= 1.0);
        }
    }
}
