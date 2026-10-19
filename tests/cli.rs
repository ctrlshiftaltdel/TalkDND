//! End-to-end tests for the v2 command-line interface.

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
            "combat-sim-cli-v2-{label}-{}-{nanos}-{count}.state.json",
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

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/engine")
        .join(relative)
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

fn run_raw(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
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

fn create(state: &TempState, extra: &[&str]) {
    let manifest = fixture("manifest.yaml");
    let mut args = vec![
        "new",
        "--encounter",
        "test-fight",
        "--campaign",
        manifest.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let output = run(state.path(), &args);
    assert_success(&output, "new");
}

#[test]
fn full_cli_flow() {
    let temp = TempState::new("flow");
    create(&temp, &["--party", "striker-a,warden-b,caster-c"]);

    let output = run(temp.path(), &["init", "--seed", "42"]);
    assert_success(&output, "init");
    assert!(
        stdout(&output).contains("Initiative"),
        "{}",
        stdout(&output)
    );

    let status = status_json(temp.path());
    assert_eq!(status["round"], 1);
    assert_eq!(status["turn"], 0);
    let combatants = status["combatants"]
        .as_array()
        .expect("combatants should be an array");
    assert_eq!(combatants.len(), 4);
    assert!(status["active"].as_str().is_some());

    let output = run(temp.path(), &["status"]);
    assert_success(&output, "status");
    assert!(stdout(&output).contains("Round 1 -"));
    assert!(stdout(&output).contains("| conditions:"));
    assert!(stdout(&output).contains("| resources:"));

    let output = run(
        temp.path(),
        &["attack", "Striker", "Brute", "test-sword", "--seed", "1"],
    );
    assert_success(&output, "attack");
    assert!(stdout(&output).contains("attacks Brute A with Test Sword"));
    assert!(stdout(&output).contains("vs AC"));

    let output = run(temp.path(), &["damage", "Brute", "3", "--type", "fire"]);
    assert_success(&output, "damage");
    assert!(stdout(&output).contains("Brute A takes"));

    let output = run(temp.path(), &["heal", "Striker", "4"]);
    assert_success(&output, "heal");
    assert!(stdout(&output).contains("regains"));

    let output = run(
        temp.path(),
        &["condition", "add", "Striker", "hindered", "--rounds", "2"],
    );
    assert_success(&output, "condition add");
    assert!(stdout(&output).contains("Striker A is now hindered"));
    let output = run(temp.path(), &["condition", "remove", "Striker", "hindered"]);
    assert_success(&output, "condition remove");
    assert!(stdout(&output).contains("Striker A is no longer hindered"));

    for _ in 0..4 {
        let output = run(temp.path(), &["next", "--seed", "42"]);
        assert_success(&output, "next");
        assert!(stdout(&output).contains("Round"));
    }

    let output = run(temp.path(), &["log", "--tail", "100"]);
    assert_success(&output, "log --tail");
    let log = stdout(&output);
    assert!(log.contains("Created Test Fight"));
    assert!(log.contains("Initiative"));
    assert!(log.contains("attacks Brute A with Test Sword"));
}

#[test]
fn unknown_combatant_lists_known_names() {
    let temp = TempState::new("unknown");
    create(&temp, &["--party", "striker-a"]);

    let output = run(temp.path(), &["damage", "Nobody", "1"]);
    let message = stderr(&output);
    assert!(!output.status.success(), "unknown name should fail");
    assert!(message.contains("known combatants"), "{message}");
    assert!(message.contains("Striker A"), "{message}");
}

#[test]
fn init_set_uses_a_fixed_total() {
    let temp = TempState::new("init-set");
    create(&temp, &["--party", "striker-a,warden-b"]);

    let output = run(
        temp.path(),
        &["init", "--set", "Striker A=30", "--seed", "42"],
    );
    assert_success(&output, "init --set");
    assert!(stdout(&output).contains("(set)"));

    let status = status_json(temp.path());
    assert_eq!(status["active"], "Striker A");
    let striker = status["combatants"]
        .as_array()
        .expect("combatants")
        .iter()
        .find(|combatant| combatant["name"] == "Striker A")
        .expect("Striker A should be present");
    assert_eq!(striker["initiative"], 30);
}

#[test]
fn grid_scenes_are_rejected_until_step_three() {
    let temp = TempState::new("grid");
    let manifest = fixture("manifest.yaml");
    let output = run_raw(&[
        "--state",
        temp.path().to_str().unwrap(),
        "new",
        "--encounter",
        "grid-fight",
        "--party",
        "striker-a",
        "--campaign",
        manifest.to_str().unwrap(),
    ]);
    assert!(!output.status.success(), "grid scenes must be rejected");
    assert!(
        stderr(&output).contains("grid scenes arrive in v2 step 3"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn simulate_is_a_step_four_stub() {
    let temp = TempState::new("simulate");
    let output = run(temp.path(), &["simulate"]);
    assert_success(&output, "simulate");
    assert_eq!(stdout(&output).trim(), "not yet (v2 step 4)");
}

#[test]
fn content_commands_keep_working() {
    let temp = TempState::new("content");
    let manifest = fixture("manifest.yaml");
    let output = run(
        temp.path(),
        &["content", "list", "--campaign", manifest.to_str().unwrap()],
    );
    assert_success(&output, "content list");
    let text = stdout(&output);
    assert!(text.contains("creature\tstriker-a\t"), "{text}");
    assert!(text.contains("scene\tarena\t"), "{text}");
    assert!(text.contains("core.yaml"), "{text}");

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
    assert!(text.contains("striker-a"));
    assert!(!text.contains("arena"), "kind filter leaked: {text}");

    let output = run(
        temp.path(),
        &[
            "content",
            "show",
            "striker-a",
            "--campaign",
            manifest.to_str().unwrap(),
        ],
    );
    assert_success(&output, "content show striker-a");
    let shown: Value = serde_json::from_slice(&output.stdout).expect("show output is JSON");
    assert_eq!(shown["kind"], "creature");
    assert_eq!(shown["item"]["id"], "striker-a");
    assert!(shown["source"].as_str().unwrap().contains("core.yaml"));

    let output = run(
        temp.path(),
        &["content", "check", "--campaign", manifest.to_str().unwrap()],
    );
    assert_success(&output, "content check");
    assert!(stdout(&output).contains("content check: 0 error(s)"));
}

#[test]
fn bare_state_names_resolve_from_the_manifest_state_dir() {
    let root = unique_temp_dir("bare-state");
    std::fs::create_dir_all(&root).unwrap();
    let manifest = root.join("campaign.yaml");
    let library = fixture("library");
    std::fs::write(
        &manifest,
        format!(
            "name: Bare State Test\nlibrary: [\"{}\"]\nstate_dir: .\n",
            library.display()
        ),
    )
    .unwrap();

    let output = run_raw(&[
        "--campaign",
        manifest.to_str().unwrap(),
        "--state",
        "bare-fight",
        "new",
        "--encounter",
        "test-fight",
        "--party",
        "striker-a",
    ]);
    assert_success(&output, "new with bare state name");
    let state_path = root.join("bare-fight.state.json");
    assert!(state_path.exists(), "expected {}", state_path.display());

    let output = run_raw(&[
        "--campaign",
        manifest.to_str().unwrap(),
        "--state",
        "bare-fight",
        "status",
    ]);
    assert_success(&output, "status with bare state name");
    assert!(stdout(&output).contains("Striker A"));

    let _ = std::fs::remove_dir_all(&root);
}

fn unique_temp_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "combat-sim-cli-dir-{label}-{}-{nanos}-{count}",
        std::process::id()
    ))
}
