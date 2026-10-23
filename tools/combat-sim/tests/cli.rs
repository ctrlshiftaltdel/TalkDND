//! End-to-end tests for the v2 command-line interface.

use std::fs;
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
fn grid_scenes_create_move_and_report_positions() {
    let temp = TempState::new("grid");
    let manifest = fixture("manifest.yaml");
    let output = run_raw(&[
        "--state",
        temp.path().to_str().unwrap(),
        "--campaign",
        manifest.to_str().unwrap(),
        "new",
        "--encounter",
        "grid-fight",
        "--party",
        "adept-a",
    ]);
    assert_success(&output, "new grid scene");

    let status = status_json(temp.path());
    let adept = status["combatants"]
        .as_array()
        .expect("combatants")
        .iter()
        .find(|combatant| combatant["name"] == "Adept A")
        .expect("Adept A should be present");
    assert_eq!(adept["pos"], serde_json::json!([6, 3]));
    assert_eq!(adept["zone"], "door");

    let output = run(temp.path(), &["scene"]);
    assert_success(&output, "scene");
    assert!(stdout(&output).contains("grid zone tables: (5,1) (6,1)"));

    let output = run(temp.path(), &["move", "Adept", "--to", "3,2", "--no-oa"]);
    assert_success(&output, "move --to");
    let status = status_json(temp.path());
    let adept = status["combatants"]
        .as_array()
        .expect("combatants")
        .iter()
        .find(|combatant| combatant["name"] == "Adept A")
        .expect("Adept A should be present");
    assert_eq!(adept["pos"], serde_json::json!([3, 2]));

    let output = run(temp.path(), &["move", "Adept", "statue", "--no-oa"]);
    assert_success(&output, "move by grid zone");
    let status = status_json(temp.path());
    let adept = status["combatants"]
        .as_array()
        .expect("combatants")
        .iter()
        .find(|combatant| combatant["name"] == "Adept A")
        .expect("Adept A should be present");
    assert_eq!(adept["zone"], "statue");
    assert_eq!(adept["pos"], serde_json::json!([1, 4]));
    assert!(stdout(&run(temp.path(), &["status"])).contains("(1,4)"));
}

#[test]
fn interact_and_move_flags_work_from_the_cli() {
    let temp = TempState::new("interact");
    let manifest = fixture("manifest.yaml");
    let output = run_raw(&[
        "--state",
        temp.path().to_str().unwrap(),
        "--campaign",
        manifest.to_str().unwrap(),
        "new",
        "--encounter",
        "zone-empty",
        "--party",
        "adept-a",
    ]);
    assert_success(&output, "new zone scene");

    let output = run(
        temp.path(),
        &[
            "interact", "Adept", "console", "--option", "1", "--seed", "42",
        ],
    );
    assert_success(&output, "interact");
    assert!(
        stdout(&output).contains("arcana check"),
        "{}",
        stdout(&output)
    );

    let output = run(temp.path(), &["move", "Adept", "beta", "--no-oa"]);
    assert_success(&output, "move --no-oa");
    assert!(stdout(&output).contains("moves alpha -> beta"));

    let output = run(temp.path(), &["move", "Adept", "--to", "1,2"]);
    assert!(!output.status.success(), "zone scenes must reject --to");
    assert!(
        stderr(&output).contains("needs a grid scene"),
        "{}",
        stderr(&output)
    );

    let output = run(temp.path(), &["scene"]);
    assert_success(&output, "scene");
    assert!(stdout(&output).contains("trigger enter-beta"));
    assert!(stdout(&output).contains("object console"));
}

#[test]
fn simulate_runs_once_with_play_by_play_and_summary() {
    let manifest = fixture("manifest.yaml");
    let output = run_raw(&[
        "simulate",
        "--campaign",
        manifest.to_str().unwrap(),
        "--encounter",
        "test-fight",
        "--party",
        "striker-a",
        "--runs",
        "1",
        "--seed",
        "7",
    ]);
    assert_success(&output, "simulate --runs 1");
    let text = stdout(&output);
    assert!(text.contains("Round 1"), "{text}");
    assert!(text.contains("Rounds:"), "{text}");
    assert!(text.contains("Result:"), "{text}");
    assert!(text.contains("HP left:"), "{text}");
}

#[test]
fn simulate_monte_carlo_json_is_sane() {
    let manifest = fixture("manifest.yaml");
    let output = run_raw(&[
        "simulate",
        "--campaign",
        manifest.to_str().unwrap(),
        "--encounter",
        "test-fight",
        "--party",
        "striker-a",
        "--runs",
        "40",
        "--threads",
        "2",
        "--seed",
        "11",
        "--json",
    ]);
    assert_success(&output, "simulate --json");
    let value: Value = serde_json::from_slice(&output.stdout).expect("simulation JSON");
    assert_eq!(value["runs"], 40);
    assert_eq!(value["threads"], 2);
    assert!(value["runs_per_sec"].as_f64().unwrap() > 0.0);
    assert!(value["wins"].is_object());
    assert!(value["creatures"].is_array());
    assert_eq!(
        value["wins"]["party"].as_u64().unwrap()
            + value["wins"]["enemy"].as_u64().unwrap()
            + value["draws"].as_u64().unwrap()
            + value["timeouts"].as_u64().unwrap()
            + value["errors"].as_u64().unwrap(),
        40
    );
}

#[test]
fn simulate_from_state_and_missing_source_errors() {
    let temp = TempState::new("simulate-from");
    create(&temp, &["--party", "striker-a"]);
    let output = run_raw(&[
        "simulate",
        "--from-state",
        temp.path().to_str().unwrap(),
        "--runs",
        "1",
        "--seed",
        "3",
    ]);
    assert_success(&output, "simulate --from-state");
    assert!(stdout(&output).contains("Result:"), "{}", stdout(&output));

    let output = run_raw(&["simulate"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("simulate needs --encounter"),
        "{}",
        stderr(&output)
    );
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
    let library = yaml_path(&fixture("library"));
    std::fs::write(
        &manifest,
        format!("name: Bare State Test\nlibrary: [\"{library}\"]\nstate_dir: .\n"),
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

/// YAML double quotes treat `\` as an escape. Forward slashes work on Windows.
fn yaml_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
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

#[test]
fn casting_out_of_turn_needs_a_reaction_or_force() {
    let temp = TempState::new("outofturn");
    create(&temp, &["--party", "striker-a,warden-b,caster-c"]);
    assert_success(&run(temp.path(), &["init", "--seed", "42"]), "init");
    while status_json(temp.path())["active"]
        .as_str()
        .unwrap_or("")
        .starts_with("Caster")
    {
        assert_success(&run(temp.path(), &["next", "--seed", "1"]), "next");
    }
    let output = run(
        temp.path(),
        &["cast", "Caster", "focus-spell", "Brute", "--seed", "1"],
    );
    assert!(!output.status.success(), "out-of-turn cast must fail");
    assert!(stderr(&output).contains("Only reactions work out of turn"));
    let output = run(
        temp.path(),
        &[
            "cast",
            "Caster",
            "focus-spell",
            "Brute",
            "--force",
            "--seed",
            "1",
        ],
    );
    assert_success(&output, "forced cast");
    assert!(stdout(&output).contains("WARNING"));
}

#[test]
fn party_state_carries_hp_between_encounters_and_rests() {
    let root = unique_temp_dir("party-state");
    std::fs::create_dir_all(&root).unwrap();
    let manifest = root.join("campaign.yaml");
    let library = yaml_path(&fixture("library"));
    std::fs::write(
        &manifest,
        format!("name: Party Test\nlibrary: [\"{library}\"]\nstate_dir: .\nparty: [striker-a]\n"),
    )
    .unwrap();
    let m = manifest.to_str().unwrap();
    let cs = |args: &[&str]| {
        let mut all = vec!["--campaign", m];
        all.extend_from_slice(args);
        run_raw(&all)
    };
    assert_success(
        &cs(&["--state", "f1", "new", "--encounter", "test-fight"]),
        "new f1",
    );
    assert_success(&cs(&["--state", "f1", "damage", "Striker", "5"]), "damage");
    let party_file = root.join("party-state.yaml");
    assert!(party_file.exists());
    let show = cs(&["party", "show"]);
    assert_success(&show, "party show");
    let max: i32 = {
        let text = stdout(&show);
        let hp = text
            .split("HP ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_string();
        let (cur, max) = hp.split_once('/').unwrap();
        let (cur, max): (i32, i32) = (
            cur.parse().unwrap(),
            max.trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .unwrap(),
        );
        assert_eq!(cur, max - 5, "{text}");
        max
    };
    let out = cs(&["--state", "f2", "new", "--encounter", "test-fight"]);
    assert_success(&out, "new f2");
    assert!(
        stdout(&out).contains("Party state loaded"),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains(&format!("HP {}/{max}", max - 5)));
    let out = cs(&["party", "rest", "long", "all"]);
    assert_success(&out, "party rest long");
    assert!(
        stdout(&out).contains(&format!("HP {max}/{max}")),
        "{}",
        stdout(&out)
    );
    // Dropping to 0 HP adds the campaign's dying condition; raising it removes it.
    assert_success(
        &cs(&["party", "set-hp", "striker-a", "0"]),
        "party set-hp 0",
    );
    let show = cs(&["party", "show"]);
    assert_success(&show, "party show after set-hp 0");
    assert!(stdout(&show).contains("downed-test"), "{}", stdout(&show));
    assert_success(
        &cs(&["party", "set-hp", "striker-a", "1"]),
        "party set-hp 1",
    );
    let show = cs(&["party", "show"]);
    assert_success(&show, "party show after set-hp 1");
    assert!(!stdout(&show).contains("downed-test"), "{}", stdout(&show));
    // Scratch states outside state_dir never touch the party file.
    let before = std::fs::read_to_string(&party_file).unwrap();
    let scratch = root.join("elsewhere");
    std::fs::create_dir_all(&scratch).unwrap();
    let sp = scratch.join("x.state.json");
    assert_success(
        &cs(&[
            "--state",
            sp.to_str().unwrap(),
            "new",
            "--encounter",
            "test-fight",
        ]),
        "scratch new",
    );
    assert_success(
        &cs(&["--state", sp.to_str().unwrap(), "damage", "Striker", "3"]),
        "scratch damage",
    );
    assert_eq!(before, std::fs::read_to_string(&party_file).unwrap());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn player_view_masks_unidentified_monsters_and_restores_the_name() {
    let state = TempState::new("player-mask");
    let save = std::env::temp_dir().join(format!(
        "combat-sim-player-save-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&save).unwrap();
    fs::write(
        save.join("identified.yaml"),
        "version: 1\nentries:\n  - {name: Dummy A, identified: false, appearance: 一只木制假人}\n",
    )
    .unwrap();
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/content/manifest.yaml");
    let quick = run_raw(&[
        "--campaign",
        manifest.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
        "--state",
        state.path().to_str().unwrap(),
        "quick",
        "--foe",
        "dummy-a",
        "--party",
        "hero-a",
    ]);
    assert_success(&quick, "quick encounter");
    let dm = run_raw(&[
        "--campaign",
        manifest.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
        "--state",
        state.path().to_str().unwrap(),
        "status",
    ]);
    assert_success(&dm, "dm status");
    assert!(stdout(&dm).contains("Dummy A"), "{}", stdout(&dm));
    let player = run_raw(&[
        "--campaign",
        manifest.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
        "--state",
        state.path().to_str().unwrap(),
        "--player",
        "status",
    ]);
    assert_success(&player, "player status");
    assert!(
        stdout(&player).contains("一只木制假人"),
        "{}",
        stdout(&player)
    );
    assert!(!stdout(&player).contains("Dummy A"), "{}", stdout(&player));
    fs::write(
        save.join("identified.yaml"),
        "version: 1\nentries:\n  - {name: Dummy A, identified: true}\n",
    )
    .unwrap();
    let named = run_raw(&[
        "--campaign",
        manifest.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
        "--state",
        state.path().to_str().unwrap(),
        "--player",
        "status",
    ]);
    assert_success(&named, "identified status");
    assert!(stdout(&named).contains("Dummy A"), "{}", stdout(&named));
    let _ = fs::remove_dir_all(&save);
}

#[test]
fn roll_is_appended_to_the_save_roll_log() {
    let save = std::env::temp_dir().join(format!(
        "combat-sim-roll-log-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&save).unwrap();
    let output = run_raw(&[
        "--save",
        save.to_str().unwrap(),
        "roll",
        "1d20+3",
        "--dc",
        "15",
    ]);
    assert_success(&output, "roll");
    let log = fs::read_to_string(save.join("rolls.log")).unwrap();
    assert!(log.contains("combat-sim 1d20+3"), "{log}");
    assert!(log.contains("1d20("), "{log}");
    let _ = fs::remove_dir_all(&save);
}

#[test]
fn state_command_rolls_are_appended_without_narration() {
    let state = TempState::new("state-roll-log");
    let save = std::env::temp_dir().join(format!(
        "combat-sim-state-roll-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&save).unwrap();
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/content/manifest.yaml");
    let base = vec![
        "--campaign",
        manifest.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
        "--state",
        state.path().to_str().unwrap(),
    ];
    let quick = {
        let mut v = base.clone();
        v.extend_from_slice(&["quick", "--foe", "dummy-a", "--party", "hero-a"]);
        run_raw(&v)
    };
    assert_success(&quick, "quick encounter");
    let damage = {
        let mut v = base.clone();
        v.extend_from_slice(&["damage", "Dummy A", "1d6+1", "--type", "fire"]);
        run_raw(&v)
    };
    assert_success(&damage, "damage command");
    let log = fs::read_to_string(save.join("rolls.log")).unwrap();
    assert!(log.contains("combat-sim combat"), "{log}");
    assert!(log.contains("1d6("), "{log}");
    let before = log.lines().count();
    // A condition change has no dice notation and must not grow the roll log.
    let condition = {
        let mut v = base;
        v.extend_from_slice(&["condition", "add", "Dummy A", "dazed"]);
        run_raw(&v)
    };
    assert_success(&condition, "condition command");
    let log = fs::read_to_string(save.join("rolls.log")).unwrap();
    assert_eq!(log.lines().count(), before, "{log}");
    let _ = fs::remove_dir_all(&save);
}
