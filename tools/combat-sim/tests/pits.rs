//! Regression tests for issues found during live play (2026-10-19 session).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

struct Temp(PathBuf);

impl Temp {
    fn new(label: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "combat-sim-pits-{label}-{}-{n}.state.json",
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

fn manifest(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pits")
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

fn setup(label: &str, encounter: &str) -> Temp {
    let t = Temp::new(label);
    let m = manifest("manifest.yaml");
    ok(&[
        "--campaign",
        &m,
        "--state",
        t.p(),
        "new",
        "--encounter",
        encounter,
    ]);
    ok(&["--state", t.p(), "init", "--set", "Hero=30", "--seed", "7"]);
    t
}

fn combatant(t: &Temp, name: &str) -> Value {
    let status: Value = serde_json::from_str(&ok(&["--state", t.p(), "status", "--json"])).unwrap();
    status["combatants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no combatant {name}"))
}

#[test]
fn nonlethal_melee_knocks_out_instead_of_killing() {
    let t = setup("ko", "pit-fight");
    let out = ok(&[
        "--state",
        t.p(),
        "attack",
        "Hero",
        "Mook 1",
        "sure-sword",
        "--nonlethal",
        "--seed",
        "1",
    ]);
    assert!(out.contains("knocked out"), "{out}");
    let mook = combatant(&t, "Mook 1");
    assert_eq!(mook["dead"], false);
    assert_eq!(mook["knocked_out"], true);

    // Without the flag the same blow kills.
    ok(&["--state", t.p(), "next", "--seed", "1"]);
    ok(&["--state", t.p(), "next", "--seed", "1"]);
    ok(&["--state", t.p(), "next", "--seed", "1"]);
    let out = ok(&[
        "--state",
        t.p(),
        "attack",
        "Hero",
        "Mook 2",
        "sure-sword",
        "--seed",
        "1",
    ]);
    assert!(out.contains("is dead") || out.contains("dies"), "{out}");
}

#[test]
fn nonlethal_is_ignored_for_ranged_attacks() {
    let t = setup("ko-ranged", "pit-fight");
    let out = ok(&[
        "--state",
        t.p(),
        "attack",
        "Hero",
        "Mook 1",
        "sure-bow",
        "--nonlethal",
        "--seed",
        "1",
    ]);
    assert!(out.contains("non-lethal ignored"), "{out}");
    assert!(out.contains("is dead") || out.contains("dies"), "{out}");
}

#[test]
fn nonlethal_creature_tag_knocks_pcs_out_stable() {
    let t = Temp::new("captor");
    let m = manifest("manifest.yaml");
    ok(&[
        "--campaign",
        &m,
        "--state",
        t.p(),
        "new",
        "--encounter",
        "captor-fight",
    ]);
    ok(&[
        "--state",
        t.p(),
        "init",
        "--set",
        "Captor=30",
        "--seed",
        "3",
    ]);
    let out = ok(&[
        "--state",
        t.p(),
        "attack",
        "Captor",
        "Hero",
        "cudgel",
        "--seed",
        "3",
    ]);
    assert!(out.contains("knocked out"), "{out}");
    let hero = combatant(&t, "Hero");
    assert_eq!(hero["death_saves"]["stable"], true);
    assert_eq!(hero["dead"], false);
}

#[test]
fn damage_command_accepts_nonlethal() {
    let t = setup("ko-damage", "pit-fight");
    let out = ok(&["--state", t.p(), "damage", "Mook 1", "5", "--nonlethal"]);
    assert!(out.contains("knocked out"), "{out}");
}

#[test]
fn simulate_defaults_to_manifest_party_and_errors_without_one() {
    let m = manifest("manifest.yaml");
    let out = ok(&[
        "--campaign",
        &m,
        "simulate",
        "--encounter",
        "pit-fight",
        "--seed",
        "4",
    ]);
    assert!(out.contains("Hero"), "{out}");

    let m2 = manifest("manifest-noparty.yaml");
    let out = run(&[
        "--campaign",
        &m2,
        "simulate",
        "--encounter",
        "pit-fight",
        "--seed",
        "4",
    ]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("party"), "{err}");
}

#[test]
fn heal_with_slot_and_cast_spend_spell_slots() {
    let t = setup("slots", "pit-fight");
    ok(&["--state", t.p(), "damage", "Hero", "10"]);
    ok(&[
        "--state",
        t.p(),
        "heal",
        "Hero",
        "2d8+3",
        "--from",
        "Hero",
        "--slot",
        "1",
        "--seed",
        "1",
    ]);
    assert_eq!(
        combatant(&t, "Hero")["resources"]["spell-slot-1"]["current"],
        2
    );
    ok(&[
        "--state",
        t.p(),
        "cast",
        "Hero",
        "mend",
        "Hero",
        "--seed",
        "1",
    ]);
    assert_eq!(
        combatant(&t, "Hero")["resources"]["spell-slot-1"]["current"],
        1
    );
}

#[test]
fn surprised_rolls_initiative_with_disadvantage() {
    let t = Temp::new("surprise");
    let m = manifest("manifest.yaml");
    ok(&[
        "--campaign",
        &m,
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
        "--adv",
        "Hero",
        "--seed",
        "9",
    ]);
    assert!(out.contains("Mook 1 is Surprised"), "{out}");
    assert!(
        out.contains("Initiative Mook 1: 1d20(") && out.contains("dis"),
        "{out}"
    );
    assert!(out.contains("adv"), "{out}");
}

#[test]
fn roll_works_without_a_state_file() {
    let out = ok(&["roll", "1d20+5", "--adv", "--dc", "12"]);
    assert!(out.contains("1d20("), "{out}");
    let out = ok(&["roll", "2d6+1"]);
    assert!(out.contains("2d6("), "{out}");
}

#[test]
fn climbing_down_costs_extra_and_carrying_halves_speed() {
    let t = setup("climb", "pit-fight");
    let out = ok(&["--state", t.p(), "move", "Hero", "ledge", "--no-oa"]);
    assert!(out.contains("climbs 10 ft up"), "{out}");
    assert_eq!(combatant(&t, "Hero")["speed_left"], 10);

    ok(&[
        "--state",
        t.p(),
        "condition",
        "add",
        "Hero",
        "carrying-test",
        "--permanent",
    ]);
    for _ in 0..3 {
        ok(&["--state", t.p(), "next", "--seed", "1"]);
    }
    assert_eq!(combatant(&t, "Hero")["current"], true);
    assert_eq!(combatant(&t, "Hero")["speed_left"], 15);
}

#[test]
fn acting_out_of_turn_warns() {
    let t = setup("turn", "pit-fight");
    let out = ok(&[
        "--state",
        t.p(),
        "attack",
        "Mook 1",
        "Hero",
        "poke",
        "--seed",
        "1",
    ]);
    assert!(
        out.contains("WARNING: Mook 1 is acting out of turn"),
        "{out}"
    );
    let out = ok(&[
        "--state",
        t.p(),
        "attack",
        "Hero",
        "Mook 1",
        "sure-sword",
        "--seed",
        "1",
    ]);
    assert!(!out.contains("WARNING"), "{out}");
}

#[test]
fn new_concentration_spell_ends_the_previous_one() {
    let t = setup("conc", "pit-fight");
    ok(&[
        "--state",
        t.p(),
        "cast",
        "Hero",
        "focus-a",
        "Mook 1",
        "--seed",
        "1",
    ]);
    ok(&["--state", t.p(), "next", "--seed", "1"]);
    ok(&["--state", t.p(), "next", "--seed", "1"]);
    ok(&["--state", t.p(), "next", "--seed", "1"]);
    let out = ok(&[
        "--state",
        t.p(),
        "cast",
        "Hero",
        "focus-b",
        "Mook 2",
        "--seed",
        "1",
    ]);
    assert!(out.contains("concentration ends"), "{out}");
    let mook1 = combatant(&t, "Mook 1");
    let conds = mook1["conditions"].to_string();
    assert!(!conds.contains("marked-test"), "{conds}");
}

#[test]
fn roll_grammar_through_the_cli() {
    let a = ok(&["roll", "4d6dl1", "--seed", "3"]);
    let b = ok(&["roll", "4d6dl1", "--seed", "3"]);
    assert_eq!(a, b, "seeded rolls repeat");
    assert!(
        a.starts_with("4d6dl1(") && a.matches('[').count() == 1,
        "{a}"
    );
    let out = ok(&["roll", "2d8+1d6+3", "--crit", "--seed", "1"]);
    assert!(out.starts_with("4d8(") && out.contains("+2d6("), "{out}");
    let json = ok(&["roll", "d%", "--json", "--seed", "2"]);
    let value: Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["terms"][0]["sides"], 100);
    assert!(!run(&["roll", "1d7"]).status.success());
    assert!(!run(&["roll", "2d6", "--adv"]).status.success());
}
