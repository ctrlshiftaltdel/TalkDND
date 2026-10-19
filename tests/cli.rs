//! End-to-end tests for the SPEC §4 command-line interface.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

struct TempState {
    path: PathBuf,
}

impl TempState {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "combat-sim-cli-{label}-{}-{nanos}-{count}.json",
            std::process::id()
        ));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempState {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn run(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--state")
        .arg(state)
        .args(args)
        .output()
        .expect("failed to run combat-sim")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn status_json(state: &Path) -> Value {
    let output = run(state, &["status", "--json"]);
    assert_success(&output, "status --json");
    serde_json::from_slice(&output.stdout).expect("status JSON should parse")
}

#[test]
fn full_cli_flow() {
    let temp = TempState::new("flow");

    let output = run(
        temp.path(),
        &[
            "new",
            "--encounter",
            "data/encounters/party-vs-2-guards.json",
        ],
    );
    assert_success(&output, "new --encounter");
    assert!(stdout(&output).contains("Created"));
    assert!(temp.path().exists(), "new should save the state file");

    let output = run(temp.path(), &["init", "--seed", "42"]);
    assert_success(&output, "init");
    assert!(stdout(&output).contains("Initiative"));

    let status = status_json(temp.path());
    assert_eq!(status["round"], 1);
    assert_eq!(status["turn"], 0);
    let combatants = status["combatants"]
        .as_array()
        .expect("combatants should be an array");
    assert_eq!(combatants.len(), 6);
    assert!(
        status["active"].as_str().is_some(),
        "active should be a name after init"
    );
    for combatant in combatants {
        assert!(
            combatant["initiative"].as_i64().is_some(),
            "initiative should be numeric: {combatant}"
        );
        assert!(combatant["name"].as_str().is_some());
    }

    let output = run(temp.path(), &["status"]);
    assert_success(&output, "status");
    assert!(stdout(&output).contains("Round 1 —"));
    assert!(stdout(&output).contains("| conditions:"));
    assert!(stdout(&output).contains("| resources:"));

    let output = run(temp.path(), &["move", "Will", "hall"]);
    assert_success(&output, "move Will hall");
    assert!(stdout(&output).contains("Will moves front -> hall"));

    let output = run(
        temp.path(),
        &["attack", "Will", "Guard 1", "longsword", "--seed", "42"],
    );
    assert_success(&output, "attack Will Guard 1 longsword");
    assert!(stdout(&output).contains("attacks Guard 1 with longsword"));
    assert!(stdout(&output).contains("vs AC 16"));

    let output = run(temp.path(), &["damage", "Guard 2", "3"]);
    assert_success(&output, "damage Guard 2 3");
    assert!(stdout(&output).contains("Guard 2 takes 3 damage"));

    let output = run(temp.path(), &["heal", "Will", "4"]);
    assert_success(&output, "heal Will 4");

    let output = run(
        temp.path(),
        &["condition", "add", "Will", "prone", "--rounds", "2"],
    );
    assert_success(&output, "condition add");
    assert!(stdout(&output).contains("Will is now prone"));

    let output = run(temp.path(), &["condition", "remove", "Will", "prone"]);
    assert_success(&output, "condition remove");
    assert!(stdout(&output).contains("Will is no longer prone"));

    for _ in 0..4 {
        let output = run(temp.path(), &["next", "--seed", "42"]);
        assert_success(&output, "next");
        assert!(stdout(&output).contains("Round 1 —"));
        assert!(stdout(&output).contains("HP "));
    }

    let output = run(temp.path(), &["log", "--tail", "100"]);
    assert_success(&output, "log --tail 100");
    let log = stdout(&output);
    assert!(log.contains("Created party vs 2 guards"));
    assert!(log.contains("Initiative Will"));
    assert!(log.contains("attacks Guard 1 with longsword"));
}

#[test]
fn unknown_combatant_lists_known_names() {
    let temp = TempState::new("unknown");
    let output = run(
        temp.path(),
        &[
            "new",
            "--encounter",
            "data/encounters/party-vs-2-guards.json",
        ],
    );
    assert_success(&output, "new");

    let output = run(temp.path(), &["damage", "Nobody", "1"]);
    let message = stderr(&output);
    assert!(!output.status.success(), "unknown name should fail");
    assert!(
        message.contains("known combatants"),
        "stderr should explain the lookup failure: {message}"
    );
    assert!(
        message.contains("Will"),
        "stderr should list known combatants: {message}"
    );
}

#[test]
fn init_set_uses_a_fixed_total() {
    let temp = TempState::new("init-set");
    let output = run(
        temp.path(),
        &[
            "new",
            "--encounter",
            "data/encounters/party-vs-2-guards.json",
        ],
    );
    assert_success(&output, "new");

    let output = run(temp.path(), &["init", "--set", "Will=30", "--seed", "42"]);
    assert_success(&output, "init --set");
    assert!(stdout(&output).contains("(set)"));

    let status = status_json(temp.path());
    assert_eq!(status["active"], "Will");
    let will = status["combatants"]
        .as_array()
        .expect("combatants")
        .iter()
        .find(|combatant| combatant["name"] == "Will")
        .expect("Will should be present");
    assert_eq!(will["initiative"], 30);
}

#[test]
fn roll_is_logged() {
    let temp = TempState::new("roll");
    let output = run(
        temp.path(),
        &[
            "new",
            "--encounter",
            "data/encounters/party-vs-2-guards.json",
        ],
    );
    assert_success(&output, "new");

    let output = run(temp.path(), &["roll", "1d20+2", "--seed", "7"]);
    assert_success(&output, "roll");
    assert!(stdout(&output).contains("1d20"));

    let output = run(temp.path(), &["log", "--tail", "10"]);
    assert_success(&output, "log");
    assert!(
        stdout(&output).contains("1d20"),
        "roll should be appended to the log"
    );
}

#[test]
fn simulate_play_by_play_prints_log_and_summary() {
    let temp = TempState::new("sim-one");
    let output = run(
        temp.path(),
        &[
            "simulate",
            "--encounter",
            "data/encounters/party-vs-2-guards.json",
            "--seed",
            "42",
        ],
    );
    assert_success(&output, "simulate --runs 1");
    let text = stdout(&output);
    assert!(
        text.contains("Round 1 —"),
        "play-by-play log missing: {text}"
    );
    assert!(text.contains("Outcome:"), "summary missing: {text}");
    assert!(text.contains("HP ("), "side summary missing: {text}");
    assert!(
        !temp.path().exists(),
        "simulate --encounter must not touch the state file"
    );
}

#[test]
fn simulate_single_run_json_is_machine_readable() {
    let temp = TempState::new("sim-one-json");
    let output = run(
        temp.path(),
        &[
            "simulate",
            "--encounter",
            "data/encounters/party-vs-2-guards.json",
            "--runs",
            "1",
            "--json",
            "--seed",
            "42",
        ],
    );
    assert_success(&output, "simulate --runs 1 --json");
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("single-run JSON should parse");

    assert_eq!(report["runs"], 1);
    assert!(report["rounds"].as_u64().unwrap() > 0);
    assert!(
        !report["log"].as_array().expect("log array").is_empty(),
        "log should contain the captured engine lines"
    );
    let outcome = report["outcome"].as_str().expect("outcome string");
    assert!(
        matches!(outcome, "party_wins" | "enemy_wins" | "draw" | "timeout"),
        "unexpected outcome {outcome}"
    );
    let combatants = report["combatants"].as_array().expect("combatants");
    assert_eq!(combatants.len(), 6);
    for combatant in combatants {
        assert!(combatant["name"].as_str().is_some());
        assert!(combatant["side"].as_str().is_some());
        assert!(combatant["hp"].as_i64().is_some());
        assert!(combatant["max_hp"].as_i64().is_some());
        assert!(combatant["dead"].as_bool().is_some());
        assert!(combatant["down"].as_bool().is_some());
    }
}

#[test]
fn simulate_monte_carlo_json_is_sane() {
    let temp = TempState::new("sim-many");
    let output = run(
        temp.path(),
        &[
            "simulate",
            "--encounter",
            "data/encounters/party-vs-2-guards.json",
            "--runs",
            "100",
            "--json",
            "--seed",
            "9",
        ],
    );
    assert_success(&output, "simulate --runs 100 --json");
    let report: Value = serde_json::from_slice(&output.stdout).expect("simulation JSON");

    assert_eq!(report["runs"], 100);
    assert_eq!(
        report["combatants"].as_array().expect("combatants").len(),
        6
    );
    assert!(report["avg_rounds"].as_f64().unwrap() > 0.0);
    assert!(report["runs_per_second"].as_f64().unwrap() > 0.0);
    let rates = report["party_win_rate"].as_f64().unwrap()
        + report["enemy_win_rate"].as_f64().unwrap()
        + report["draw_rate"].as_f64().unwrap()
        + report["timeout_rate"].as_f64().unwrap();
    assert!((rates - 1.0).abs() < 1e-9, "rates sum to {rates}");
}

#[test]
fn simulate_from_state_continues_without_rerolling_initiative() {
    let temp = TempState::new("sim-state");
    let output = run(
        temp.path(),
        &[
            "new",
            "--encounter",
            "data/encounters/party-vs-2-guards.json",
        ],
    );
    assert_success(&output, "new");
    let output = run(temp.path(), &["init", "--seed", "42"]);
    assert_success(&output, "init");
    let before = std::fs::read_to_string(temp.path()).unwrap();

    let output = run(temp.path(), &["simulate", "--from-state", "--seed", "42"]);
    assert_success(&output, "simulate --from-state");
    let text = stdout(&output);
    assert!(text.contains("Outcome:"), "summary missing: {text}");
    assert!(
        !text.contains("Initiative"),
        "from-state simulation should continue the existing initiative: {text}"
    );
    let after = std::fs::read_to_string(temp.path()).unwrap();
    assert_eq!(before, after, "simulate must not modify the state file");
}

fn content_fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/content")
        .join(relative)
}

#[test]
fn content_list_and_show_read_the_campaign() {
    let temp = TempState::new("content-list");
    let manifest = content_fixture("manifest.yaml");
    let output = run(
        temp.path(),
        &["content", "list", "--campaign", manifest.to_str().unwrap()],
    );
    assert_success(&output, "content list");
    let text = stdout(&output);
    assert!(
        text.contains("creature\tdummy-a\t"),
        "creature row missing: {text}"
    );
    assert!(
        text.contains("scene\thall-a\t"),
        "scene row missing: {text}"
    );
    assert!(text.contains("core.yaml"), "source path missing: {text}");

    let output = run(
        temp.path(),
        &[
            "content",
            "list",
            "--kind",
            "creatures",
            "--campaign",
            manifest.to_str().unwrap(),
        ],
    );
    assert_success(&output, "content list --kind creatures");
    let text = stdout(&output);
    assert!(text.contains("dummy-a"));
    assert!(!text.contains("hall-a"), "kind filter leaked: {text}");

    let output = run(
        temp.path(),
        &[
            "content",
            "show",
            "dummy-a",
            "--campaign",
            manifest.to_str().unwrap(),
        ],
    );
    assert_success(&output, "content show dummy-a");
    let shown: Value = serde_json::from_slice(&output.stdout).expect("show output is JSON");
    assert_eq!(shown["kind"], "creature");
    assert_eq!(shown["item"]["id"], "dummy-a");
    assert!(
        shown["source"].as_str().unwrap().contains("core.yaml"),
        "source missing: {shown}"
    );
}

#[test]
fn content_check_passes_and_fails_appropriately() {
    let temp = TempState::new("content-check");
    let manifest = content_fixture("manifest.yaml");
    let output = run(
        temp.path(),
        &["content", "check", "--campaign", manifest.to_str().unwrap()],
    );
    assert_success(&output, "content check");
    let text = stdout(&output);
    assert!(text.contains("content check: 0 error(s)"), "{text}");
    assert!(text.contains("TODO: replace with final numbers"), "{text}");

    let bad = content_fixture("bad/refs");
    let output = run(
        temp.path(),
        &["content", "check", "--lib", bad.to_str().unwrap()],
    );
    assert!(
        !output.status.success(),
        "failing content must exit nonzero\nstdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    assert!(text.contains("ERROR"), "error lines missing: {text}");
    assert!(text.contains("unknown mastery"), "{text}");
    assert!(
        stderr(&output).contains("content check failed"),
        "failure summary missing from stderr: {}",
        stderr(&output)
    );
}
