//! Temporary local profiling probe (ignored by default).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use combat_sim::ai::Ai;
use combat_sim::content::ContentSources;
use combat_sim::dice;
use combat_sim::engine::{self, NullSink};
use combat_sim::state::{EncounterState, NewEncounter};

#[test]
#[ignore]
fn probe() {
    let manifest = "/workspace/projects/talkdnd/Saves/2026-10-19-01/combat/campaign.yaml";
    if !Path::new(manifest).exists() {
        return;
    }
    let registry = ContentSources {
        campaign: Some(manifest.into()),
        ..ContentSources::default()
    }
    .load()
    .unwrap();
    let content = Arc::new(registry);
    let state = EncounterState::from_encounter_shared(
        content,
        &NewEncounter {
            encounter: "v13-gemstone-guards".to_string(),
            party: ["will", "rina", "milo", "sera"]
                .iter()
                .map(ToString::to_string)
                .collect(),
            party_zone: None,
            adds: Vec::new(),
        },
    )
    .unwrap();
    let ai = Ai::prepare(&state);
    let runs = 2000;
    let (mut t_clone, mut t_init, mut t_ai, mut t_next) = (0u128, 0u128, 0u128, 0u128);
    let (mut n_ai, mut n_next) = (0u64, 0u64);
    let mut sink = NullSink;
    let mut rng = dice::seeded_rng(1);
    for _ in 0..runs {
        let now = Instant::now();
        let mut run = state.clone();
        run.log.clear();
        t_clone += now.elapsed().as_nanos();
        let now = Instant::now();
        engine::init(&mut run, &[], &mut rng, &mut sink).unwrap();
        t_init += now.elapsed().as_nanos();
        let now = Instant::now();
        loop {
            if engine::combat_outcome(&run).is_some() {
                break;
            }
            let now = Instant::now();
            ai.take_turn(&mut run, &mut rng, &mut sink).unwrap();
            t_ai += now.elapsed().as_nanos();
            n_ai += 1;
            if engine::combat_outcome(&run).is_some() {
                break;
            }
            let now = Instant::now();
            let next_ok = engine::next_turn(&mut run, &mut rng, &mut sink).is_ok();
            t_next += now.elapsed().as_nanos();
            n_next += 1;
            if !next_ok {
                break;
            }
        }
        std::hint::black_box(&run);
    }
    println!(
        "clone {:.1}us init {:.1}us ai {:.1}us next {:.1}us turns {}/{}",
        t_clone as f64 / runs as f64 / 1000.0,
        t_init as f64 / runs as f64 / 1000.0,
        t_ai as f64 / runs as f64 / 1000.0,
        t_next as f64 / runs as f64 / 1000.0,
        n_ai as f64 / runs as f64,
        n_next as f64 / runs as f64
    );
}
