use std::sync::Arc;
use std::time::Instant;
use combat_sim::ai::Ai;
use combat_sim::content::{self, ContentSources};
use combat_sim::dice;
use combat_sim::engine::{self, NullSink};
use combat_sim::sim;
use combat_sim::state::{EncounterState, NewEncounter};

fn main() {
    let sources = ContentSources { campaign: Some("/workspace/projects/talkdnd/Saves/2026-10-19-01/combat/campaign.yaml".into()), ..Default::default() };
    let _ = content::check(&sources.load().unwrap());
    let content = Arc::new(sources.load().unwrap());
    let options = NewEncounter {
        encounter: "v12-satyr-and-guards".into(),
        party: ["will","rina","milo","sera"].iter().map(|s| s.to_string()).collect(),
        party_zone: None,
        adds: Vec::new(),
    };
    let state = EncounterState::from_encounter_shared(content, &options).unwrap();
    let ai = Ai::prepare(&state);
    let mut base = state.clone();
    base.log.clear();
    let n = 20_000;

    let start = Instant::now();
    for _ in 0..n { let c = base.clone(); std::hint::black_box(&c); }
    println!("clone: {:.2} us", start.elapsed().as_secs_f64() / n as f64 * 1e6);

    let mut rng = dice::seeded_rng(1);
    let start = Instant::now();
    for _ in 0..n {
        let mut c = base.clone();
        c.log.clear();
        engine::init(&mut c, &[], &mut rng, &mut NullSink).unwrap();
        std::hint::black_box(&c);
    }
    println!("clone+init: {:.2} us", start.elapsed().as_secs_f64() / n as f64 * 1e6);

    for rounds in [1u32, 100] {
        combat_sim::prof::reset();
        let m = if rounds == 1 { n } else { 2000 };
        let start = Instant::now();
        for _ in 0..m {
            let c = base.clone();
            std::hint::black_box(sim::play_quiet(c, &ai, &mut rng, rounds).unwrap());
        }
        let total = start.elapsed().as_secs_f64();
        println!("play rounds={rounds}: {:.2} us", total / m as f64 * 1e6);
        combat_sim::prof::dump();
        println!("(sum per run: turn {:.1}us next {:.1}us init {:.1}us)", 0.0, 0.0, 0.0);
    }
}
