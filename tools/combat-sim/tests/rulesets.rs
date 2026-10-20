//! Ruleset selection (2014 vs 2024) via the campaign manifest.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use combat_sim::content::{self, ContentSources};
use serde_json::Value;

struct Temp(PathBuf);
impl Temp {
    fn new(label: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "combat-sim-rulesets-{label}-{}-{n}.state.json",
            std::process::id()
        )))
    }
    fn p(&self) -> &str {
        self.0.to_str().unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rulesets")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(std::env::temp_dir())
        .env_remove("COMBAT_SIM_CAMPAIGN")
        .args(args)
        .output()
        .expect("run combat-sim")
}

fn ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "{args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn conditions(t: &Temp, name: &str) -> String {
    let status: Value = serde_json::from_str(&ok(&["--state", t.p(), "status", "--json"])).unwrap();
    let c = status["combatants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .cloned()
        .unwrap();
    c["conditions"].to_string()
}

fn load(name: &str) -> content::ContentRegistry {
    ContentSources {
        campaign: Some(fixture(name).into()),
        ..ContentSources::default()
    }
    .load()
    .expect("load")
}

#[test]
fn surprise_2014_loses_first_turn_without_initiative_disadvantage() {
    let t = Temp::new("s2014");
    ok(&[
        "--campaign",
        &fixture("manifest-2014.yaml"),
        "--state",
        t.p(),
        "new",
        "--encounter",
        "pit-fight",
    ]);
    let out = ok(&[
        "--state",
        t.p(),
        "init",
        "--surprised",
        "Mook 1",
        "--seed",
        "9",
    ]);
    assert!(out.contains("Surprised (2014)"), "{out}");
    assert!(
        !out.contains("initiative rolled with Disadvantage"),
        "{out}"
    );
    assert!(
        conditions(&t, "Mook 1").contains("surprised"),
        "{}",
        conditions(&t, "Mook 1")
    );
    // Surprise ends at the end of the creature's first turn.
    for _ in 0..4 {
        ok(&["--state", t.p(), "next", "--seed", "1"]);
    }
    assert!(
        !conditions(&t, "Mook 1").contains("surprised"),
        "{}",
        conditions(&t, "Mook 1")
    );
}

#[test]
fn surprise_2024_is_initiative_disadvantage() {
    let t = Temp::new("s2024");
    ok(&[
        "--campaign",
        &fixture("manifest-2024.yaml"),
        "--state",
        t.p(),
        "new",
        "--encounter",
        "pit-fight",
    ]);
    let out = ok(&[
        "--state",
        t.p(),
        "init",
        "--surprised",
        "Mook 1",
        "--seed",
        "9",
    ]);
    assert!(out.contains("initiative rolled with Disadvantage"), "{out}");
    assert!(!conditions(&t, "Mook 1").contains("surprised"));
}

#[test]
fn unknown_ruleset_is_rejected() {
    let state = std::env::temp_dir().join(format!(
        "combat-sim-bad-ruleset-{}.state.json",
        std::process::id()
    ));
    let state = state.to_string_lossy().replace('\\', "/");
    let out = run(&[
        "--campaign",
        &fixture("manifest-bad.yaml"),
        "--state",
        &state,
        "new",
        "--encounter",
        "pit-fight",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unsupported ruleset"));
}

#[test]
fn real_library_layers_per_edition() {
    let r14 = load("real-2014.yaml");
    assert_eq!(r14.ruleset, "2014");
    assert!(
        r14.masteries.get("topple").is_none(),
        "2014 has no weapon mastery"
    );
    assert!(r14.conditions.get("surprised").is_some());
    assert!(r14.abilities.get("inspiration").is_some());
    assert!(r14.abilities.get("heroic-inspiration").is_none());
    assert!(
        !content::check(&r14).has_errors(),
        "2014 library check failed"
    );

    let r24 = load("real-2024.yaml");
    assert_eq!(r24.ruleset, "2024");
    assert!(r24.masteries.get("topple").is_some());
    assert!(r24.conditions.get("surprised").is_none());
    assert!(r24.abilities.get("heroic-inspiration").is_some());
    assert!(
        !content::check(&r24).has_errors(),
        "2024 library check failed"
    );
}
