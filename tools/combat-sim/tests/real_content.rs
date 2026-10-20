//! Smoke test against the real TalkDND campaign when it is present.
//!
//! The test is intentionally skipped (not failed) when the save directory is
//! absent, so the repository remains self-contained.

use std::path::Path;
use std::sync::Arc;

use combat_sim::ai::Ai;
use combat_sim::content::{self, ContentSources};
use combat_sim::dice;
use combat_sim::engine::NullSink;
use combat_sim::sim;
use combat_sim::state::{EncounterState, NewEncounter};

const MANIFEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../LetsPlay/Saves/2026-10-19-01/combat/campaign.yaml"
);
const PARTY: &[&str] = &["will", "rina", "milo", "sera"];

#[test]
fn real_campaign_loads_checks_and_simulates_once() {
    if !Path::new(MANIFEST).exists() {
        eprintln!(
            "skipping real_campaign_loads_checks_and_simulates_once: \
             LetsPlay/Saves/2026-10-19-01/combat/campaign.yaml is absent"
        );
        return;
    }
    let sources = ContentSources {
        campaign: Some(MANIFEST.into()),
        ..ContentSources::default()
    };
    let registry = sources.load().expect("real campaign content should load");
    let report = content::check(&registry);
    assert!(
        !report.has_errors(),
        "real content check reported {} errors",
        report.error_count()
    );
    let content = Arc::new(registry);
    let mut encounters: Vec<String> = content.encounters.ids().map(ToString::to_string).collect();
    encounters.sort();
    assert!(!encounters.is_empty(), "real campaign has no encounters");
    for encounter in encounters {
        let options = NewEncounter {
            encounter: encounter.clone(),
            party: PARTY.iter().map(ToString::to_string).collect(),
            party_zone: None,
            adds: Vec::new(),
        };
        let state = EncounterState::from_encounter_shared(content.clone(), &options)
            .unwrap_or_else(|error| panic!("failed to build `{encounter}`: {error:#}"));
        let ai = Ai::prepare(&state);
        let mut rng = dice::seeded_rng(20261019);
        sim::play(state, &ai, &mut rng, 100, &mut NullSink)
            .unwrap_or_else(|error| panic!("simulation of `{encounter}` failed: {error:#}"));
    }
}
