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

    // Attacks are turn-gated: advance to Striker's turn before attacking.
    while !status_json(temp.path())["active"]
        .as_str()
        .unwrap_or("")
        .starts_with("Striker")
    {
        assert_success(&run(temp.path(), &["next", "--seed", "1"]), "next");
    }

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
fn player_view_keeps_duplicate_monsters_apart() {
    let state = TempState::new("player-mask-dup");
    let save = std::env::temp_dir().join(format!(
        "combat-sim-player-dup-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&save).unwrap();
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/content/manifest.yaml");
    let base = [
        "--campaign",
        manifest.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
        "--state",
        state.path().to_str().unwrap(),
    ];
    let mut q = base.to_vec();
    q.extend_from_slice(&["quick", "--foe", "dummy-a:2", "--party", "hero-a"]);
    assert_success(&run_raw(&q), "quick encounter");
    let mut p = base.to_vec();
    p.extend_from_slice(&["--player", "status"]);
    let player = run_raw(&p);
    assert_success(&player, "player status");
    let out = stdout(&player);
    assert!(out.contains(" #1") && out.contains(" #2"), "{out}");
    assert!(!out.contains("Dummy A"), "{out}");
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

#[test]
fn attack_out_of_turn_fails_without_changing_any_state() {
    let temp = TempState::new("attack-outofturn");
    create(&temp, &["--party", "striker-a,warden-b,caster-c"]);
    assert_success(&run(temp.path(), &["init", "--seed", "42"]), "init");
    while status_json(temp.path())["active"]
        .as_str()
        .unwrap_or("")
        .starts_with("Striker")
    {
        assert_success(&run(temp.path(), &["next", "--seed", "1"]), "next");
    }
    let before = fs::read(temp.path()).unwrap();

    let output = run(
        temp.path(),
        &["attack", "Striker", "Brute", "test-sword", "--seed", "1"],
    );
    assert!(!output.status.success(), "out-of-turn attack must fail");
    let message = stderr(&output);
    assert!(message.contains("cannot attack now"), "{message}");
    assert!(
        message.contains("Run `next` first, or add --force"),
        "{message}"
    );
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));

    // The refusal must precede any roll, log line, action spend, or HP or
    // resource change: the self-contained state file stays byte-identical
    // (rolls.log is only appended on success).
    let after = fs::read(temp.path()).unwrap();
    assert_eq!(
        before, after,
        "state file must be byte-identical after a failed out-of-turn attack"
    );
}

#[test]
fn attack_on_the_attacker_s_turn_still_succeeds() {
    let temp = TempState::new("attack-onturn");
    create(&temp, &["--party", "striker-a,warden-b,caster-c"]);
    assert_success(&run(temp.path(), &["init", "--seed", "42"]), "init");
    while !status_json(temp.path())["active"]
        .as_str()
        .unwrap_or("")
        .starts_with("Striker")
    {
        assert_success(&run(temp.path(), &["next", "--seed", "1"]), "next");
    }
    let output = run(
        temp.path(),
        &["attack", "Striker", "Brute", "test-sword", "--seed", "1"],
    );
    assert_success(&output, "on-turn attack");
    assert!(stdout(&output).contains("attacks Brute A with Test Sword"));
    assert!(!stdout(&output).contains("WARNING"), "{}", stdout(&output));
}

#[test]
fn attack_force_overrides_the_turn_rule_with_an_audit_line() {
    let temp = TempState::new("attack-force");
    create(&temp, &["--party", "striker-a,warden-b,caster-c"]);
    assert_success(&run(temp.path(), &["init", "--seed", "42"]), "init");
    while status_json(temp.path())["active"]
        .as_str()
        .unwrap_or("")
        .starts_with("Striker")
    {
        assert_success(&run(temp.path(), &["next", "--seed", "1"]), "next");
    }
    let output = run(
        temp.path(),
        &[
            "attack",
            "Striker",
            "Brute",
            "test-sword",
            "--force",
            "--seed",
            "1",
        ],
    );
    assert_success(&output, "forced out-of-turn attack");
    let out = stdout(&output);
    assert!(out.contains("WARNING"), "{out}");
    assert!(out.contains("with --force"), "{out}");
    assert!(out.contains("attacks Brute A with Test Sword"), "{out}");
    // The audit line is part of the persistent encounter log.
    let output = run(temp.path(), &["log", "--tail", "100"]);
    assert_success(&output, "log");
    assert!(
        stdout(&output).contains("WARNING: Striker A acts out of turn to attack"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn cast_and_use_reactions_still_work_out_of_turn() {
    let temp = TempState::new("reaction-outofturn");
    create(&temp, &["--party", "striker-a,warden-b,caster-c"]);
    assert_success(&run(temp.path(), &["init", "--seed", "42"]), "init");
    while status_json(temp.path())["active"]
        .as_str()
        .unwrap_or("")
        .starts_with("Warden")
    {
        assert_success(&run(temp.path(), &["next", "--seed", "1"]), "next");
    }
    // `warding` is a reaction: allowed out of turn without --force and
    // without a warning.
    let output = run(
        temp.path(),
        &["use", "Warden", "warding", "Warden", "--seed", "1"],
    );
    assert_success(&output, "reaction out of turn");
    assert!(!stdout(&output).contains("WARNING"), "{}", stdout(&output));
}

#[test]
fn move_and_interact_out_of_turn_warn_but_proceed() {
    let temp = TempState::new("move-interact-outofturn");
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
        "--add",
        "sentinel-t",
    ]);
    assert_success(&output, "new zone scene with a foe");
    assert_success(
        &run(temp.path(), &["init", "--set", "Sentinel T=30"]),
        "init",
    );
    assert_eq!(status_json(temp.path())["active"], "Sentinel T");

    // Movement and object interaction do not consume an action, so they stay
    // warn-only out of turn (see LetsPlay/docs/combat.md).
    let output = run(temp.path(), &["move", "Adept", "beta", "--no-oa"]);
    assert_success(&output, "out-of-turn move");
    assert!(
        stdout(&output).contains("WARNING: Adept A is acting out of turn"),
        "{}",
        stdout(&output)
    );
    assert!(stdout(&output).contains("moves alpha -> beta"));

    let output = run(
        temp.path(),
        &[
            "interact", "Adept", "console", "--option", "1", "--seed", "42",
        ],
    );
    assert_success(&output, "out-of-turn interact");
    assert!(
        stdout(&output).contains("WARNING: Adept A is acting out of turn"),
        "{}",
        stdout(&output)
    );

    // On the mover's own turn no warning appears.
    while !status_json(temp.path())["active"]
        .as_str()
        .unwrap_or("")
        .starts_with("Adept")
    {
        assert_success(&run(temp.path(), &["next", "--seed", "1"]), "next");
    }
    let output = run(temp.path(), &["move", "Adept", "alpha", "--no-oa"]);
    assert_success(&output, "on-turn move");
    assert!(!stdout(&output).contains("WARNING"), "{}", stdout(&output));
}

#[test]
fn damage_works_on_any_turn_and_spends_no_action() {
    let temp = TempState::new("damage-dm");
    create(&temp, &["--party", "striker-a,warden-b,caster-c"]);
    assert_success(&run(temp.path(), &["init", "--seed", "42"]), "init");
    // `damage` has no actor: it must work regardless of whose turn it is.
    let output = run(temp.path(), &["damage", "Brute", "5"]);
    assert_success(&output, "damage as a DM command");
    assert!(stdout(&output).contains("takes"), "{}", stdout(&output));
    let brute = status_json(temp.path())["combatants"]
        .as_array()
        .expect("combatants")
        .iter()
        .find(|combatant| combatant["name"] == "Brute A")
        .expect("Brute A should be present")
        .clone();
    assert_eq!(brute["hp"], brute["max_hp"].as_i64().unwrap() - 5);

    // It must not consume an action either: the attacker can still act on
    // its own turn afterwards.
    while !status_json(temp.path())["active"]
        .as_str()
        .unwrap_or("")
        .starts_with("Striker")
    {
        assert_success(&run(temp.path(), &["next", "--seed", "1"]), "next");
    }
    let output = run(
        temp.path(),
        &["attack", "Striker", "Brute", "test-sword", "--seed", "1"],
    );
    assert_success(&output, "attack after a DM damage");
}

#[test]
fn add_combatants_to_a_running_encounter_preserves_everything() {
    let temp = TempState::new("add-running");
    create(&temp, &["--party", "striker-a,warden-b"]);
    assert_success(
        &run(
            temp.path(),
            &[
                "init",
                "--set",
                "Striker A=20",
                "--set",
                "Warden B=15",
                "--set",
                "Brute A=10",
            ],
        ),
        "init",
    );
    assert_success(&run(temp.path(), &["damage", "Brute", "5"]), "damage");

    let before: Value = serde_json::from_slice(&fs::read(temp.path()).unwrap()).unwrap();
    let before_combatants = before["combatants"].as_array().unwrap().clone();
    let before_log = before["log"].as_array().unwrap().clone();

    let output = run(
        temp.path(),
        &["add", "--creature", "brute-a:2@beta", "--seed", "42"],
    );
    assert_success(&output, "add two foes");
    let out = stdout(&output);
    assert!(
        out.contains("Added Brute A 1 (enemy brute-a) at beta"),
        "{out}"
    );
    assert!(
        out.contains("Added Brute A 2 (enemy brute-a) at beta"),
        "{out}"
    );
    assert!(out.contains("(rolled)"), "{out}");
    assert!(!out.contains("(set)"), "{out}");

    let after: Value = serde_json::from_slice(&fs::read(temp.path()).unwrap()).unwrap();
    let combatants = after["combatants"].as_array().unwrap().clone();
    assert_eq!(combatants.len(), before_combatants.len() + 2);

    // Existing combatants keep every field, including the damage taken.
    for (old, new) in before_combatants.iter().zip(combatants.iter()) {
        assert_eq!(old, new, "existing combatant changed: {old}");
    }
    // The log only grows: every pre-existing line is byte-identical.
    let log = after["log"].as_array().unwrap().clone();
    assert!(log.len() > before_log.len());
    for (index, line) in before_log.iter().enumerate() {
        assert_eq!(line, &log[index], "log line {index} changed");
    }

    // The new combatants have rolled initiative and enter the order without
    // moving the current actor or the round.
    assert_eq!(after["round"], 1);
    assert_eq!(after["turn"], 0);
    assert_eq!(status_json(temp.path())["active"], "Striker A");
    assert_eq!(
        after["initiative"].as_array().unwrap()[0],
        before["initiative"].as_array().unwrap()[0]
    );
    for name in ["Brute A 1", "Brute A 2"] {
        let combatant = combatants
            .iter()
            .find(|combatant| combatant["name"] == name)
            .unwrap_or_else(|| panic!("{name} should be present"));
        assert_eq!(combatant["zone"], "beta");
        assert_eq!(combatant["hp"], combatant["max_hp"]);
        assert!(combatant["initiative_total"].is_i64(), "{combatant}");
    }

    // The audit lines are part of the persistent log.
    let output = run(temp.path(), &["log", "--tail", "10"]);
    assert_success(&output, "log");
    let log = stdout(&output);
    assert!(
        log.contains("Added Brute A 1 (enemy brute-a) at beta"),
        "{log}"
    );
    assert!(log.contains("Initiative Brute A 1: 1d20("), "{log}");
    assert!(log.contains("(rolled)"), "{log}");
}

#[test]
fn added_combatant_before_the_current_turn_waits_until_next_round() {
    let temp = TempState::new("add-retroactive");
    create(&temp, &["--party", "striker-a,warden-b"]);
    assert_success(
        &run(
            temp.path(),
            &[
                "init",
                "--set",
                "Striker A=20",
                "--set",
                "Warden B=15",
                "--set",
                "Brute A=10",
            ],
        ),
        "init",
    );
    assert_eq!(status_json(temp.path())["active"], "Striker A");

    // Higher than everyone: inserted at the top, before the current turn
    // position, so it must not act in round 1.
    let output = run(
        temp.path(),
        &[
            "add",
            "--creature",
            "rampage-e",
            "--side",
            "enemy",
            "--initiative",
            "25",
        ],
    );
    assert_success(&output, "add with explicit initiative");
    let out = stdout(&output);
    assert!(out.contains("Initiative Rampage E: 25 (set)"), "{out}");
    assert!(
        out.contains("Rampage E joins before the current turn: no action in round 1"),
        "{out}"
    );

    let status = status_json(temp.path());
    assert_eq!(
        status["active"], "Striker A",
        "current actor must be preserved"
    );
    assert_eq!(status["round"], 1);
    let rampage = status["combatants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|combatant| combatant["name"] == "Rampage E")
        .expect("Rampage E should be present")
        .clone();
    assert_eq!(rampage["initiative"], 25);
    assert_eq!(rampage["side"], "enemy");

    // The remaining round-1 turns are Warden B and Brute A, then the order
    // wraps around to Rampage E in round 2 - it never acted in round 1.
    let mut turns = Vec::new();
    for _ in 0..3 {
        let output = run(temp.path(), &["next"]);
        assert_success(&output, "next");
        let line = stdout(&output)
            .lines()
            .find(|line| line.contains("'s turn"))
            .expect("next prints a turn line")
            .to_string();
        turns.push(line);
    }
    assert_eq!(turns[0], "Round 1 - Warden B's turn", "{turns:?}");
    assert_eq!(turns[1], "Round 1 - Brute A's turn", "{turns:?}");
    assert_eq!(turns[2], "Round 2 - Rampage E's turn", "{turns:?}");

    // And on that legitimate turn it can act normally.
    let output = run(
        temp.path(),
        &["attack", "Rampage E", "Warden", "bite", "--seed", "1"],
    );
    assert_success(&output, "attack on the added combatant's turn");
    assert!(
        stdout(&output).contains("Rampage E attacks Warden B with Bite"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn added_combatant_after_the_current_turn_acts_this_round() {
    let temp = TempState::new("add-lower");
    create(&temp, &["--party", "striker-a,warden-b"]);
    assert_success(
        &run(
            temp.path(),
            &[
                "init",
                "--set",
                "Striker A=20",
                "--set",
                "Warden B=15",
                "--set",
                "Brute A=10",
            ],
        ),
        "init",
    );
    // Lower than the current actor: joins behind it and acts later in the
    // same round.
    let output = run(
        temp.path(),
        &["add", "--creature", "recharger-r", "--initiative", "5"],
    );
    assert_success(&output, "add with a low explicit initiative");
    assert!(
        !stdout(&output).contains("no action in round"),
        "{}",
        stdout(&output)
    );
    let mut turns = Vec::new();
    for _ in 0..3 {
        let output = run(temp.path(), &["next"]);
        assert_success(&output, "next");
        let line = stdout(&output)
            .lines()
            .find(|line| line.contains("'s turn"))
            .expect("next prints a turn line")
            .to_string();
        turns.push(line);
    }
    assert_eq!(turns[0], "Round 1 - Warden B's turn", "{turns:?}");
    assert_eq!(turns[1], "Round 1 - Brute A's turn", "{turns:?}");
    assert_eq!(turns[2], "Round 1 - Recharger R's turn", "{turns:?}");
}

#[test]
fn add_before_initiative_adds_without_rolling() {
    let temp = TempState::new("add-before-init");
    create(&temp, &["--party", "striker-a"]);

    // Before init runs there is no order yet: --initiative is refused.
    let output = run(
        temp.path(),
        &["add", "--creature", "brute-a", "--initiative", "15"],
    );
    assert!(
        !output.status.success(),
        "--initiative before init must fail"
    );
    assert!(
        stderr(&output).contains("--initiative needs an initiative order"),
        "{}",
        stderr(&output)
    );

    let output = run(temp.path(), &["add", "--creature", "brute-a@beta"]);
    assert_success(&output, "add before init");
    assert!(
        stdout(&output).contains("Added Brute A 2"),
        "{}",
        stdout(&output)
    );
    let status = status_json(temp.path());
    assert_eq!(status["round"], 0);
    assert_eq!(status["combatants"].as_array().unwrap().len(), 3);
    let raw: Value = serde_json::from_slice(&fs::read(temp.path()).unwrap()).unwrap();
    assert_eq!(raw["initiative"].as_array().unwrap().len(), 0);

    // init rolls for everyone, the added combatant included.
    assert_success(&run(temp.path(), &["init", "--seed", "7"]), "init");
    let status = status_json(temp.path());
    let combatants = status["combatants"].as_array().unwrap();
    assert_eq!(combatants.len(), 3);
    assert!(combatants
        .iter()
        .all(|combatant| combatant["initiative"].is_i64()));
    let raw: Value = serde_json::from_slice(&fs::read(temp.path()).unwrap()).unwrap();
    assert_eq!(raw["initiative"].as_array().unwrap().len(), 3);
}

#[test]
fn add_rejects_invalid_input_without_touching_the_state() {
    let temp = TempState::new("add-invalid");
    create(&temp, &["--party", "striker-a"]);
    assert_success(&run(temp.path(), &["init", "--seed", "42"]), "init");
    let before = fs::read(temp.path()).unwrap();

    let failures: &[(&[&str], &str)] = &[
        (&["add", "--creature", "no-such-foe"], "no-such-foe"),
        (&["add", "--creature", "brute-a@nowhere"], "unknown zone"),
        (
            &["add", "--creature", "brute-a:0"],
            "count must be at least 1",
        ),
        (
            &["add", "--creature", "brute-a", "--side", "Bad Side!"],
            "invalid side",
        ),
        (
            &["add", "--creature", "brute-a", "--initiative", "abc"],
            "invalid value",
        ),
    ];
    for (args, expected) in failures {
        let output = run(temp.path(), args);
        assert!(!output.status.success(), "{args:?} should fail");
        if !expected.is_empty() {
            let message = format!("{}{}", stderr(&output), stdout(&output));
            assert!(message.contains(expected), "{args:?}: {message}");
        }
        assert_eq!(
            before,
            fs::read(temp.path()).unwrap(),
            "state file must be byte-identical after a failed {args:?}"
        );
    }
}

#[test]
fn add_to_the_party_side_syncs_party_state_and_the_integrity_baseline() {
    let root = unique_temp_dir("add-party-integrity");
    fs::create_dir_all(root.join("combat")).unwrap();
    let manifest = root.join("combat/campaign.yaml");
    let library = yaml_path(&fixture("library"));
    fs::write(
        &manifest,
        format!(
            "name: Add Party Test\nlibrary: [\"{library}\"]\nstate_dir: .\nparty: [striker-a]\n"
        ),
    )
    .unwrap();
    // The integrity manifest exists before the first tool write (talkdnd
    // creates it on first use); a combat/ state file is part of it.
    combat_sim::integrity::ensure(&root).unwrap();
    let cs = |args: &[&str]| {
        let mut all = vec!["--campaign", manifest.to_str().unwrap()];
        all.extend_from_slice(args);
        run_raw(&all)
    };
    assert_success(
        &cs(&["--state", "v13", "new", "--encounter", "test-fight"]),
        "new",
    );
    assert!(
        combat_sim::integrity::changes(&root).unwrap().is_empty(),
        "new must adopt the state file into the baseline"
    );
    assert_success(
        &cs(&["--state", "v13", "init", "--set", "Striker A=20"]),
        "init",
    );

    // Add a party-side member mid-fight: the party-state file gains the
    // member and the integrity baseline moves with the tool's own write.
    let output = cs(&[
        "--state",
        "v13",
        "add",
        "--creature",
        "warden-b",
        "--side",
        "party",
        "--initiative",
        "8",
    ]);
    assert_success(&output, "add party-side member");
    let out = stdout(&output);
    assert!(out.contains("Added Warden B (party warden-b)"), "{out}");
    assert!(out.contains("Initiative Warden B: 8 (set)"), "{out}");

    let party = fs::read_to_string(root.join("combat/party-state.yaml")).unwrap();
    assert!(party.contains("warden-b"), "{party}");
    assert!(
        combat_sim::integrity::changes(&root).unwrap().is_empty(),
        "add must refresh the integrity baseline"
    );
    let show = cs(&["--state", "v13", "party", "show"]);
    assert_success(&show, "party show");
    assert!(stdout(&show).contains("Warden B"), "{}", stdout(&show));

    // A later hand edit is still detected on top of the refreshed baseline.
    let state_file = root.join("combat/v13.state.json");
    let text = fs::read_to_string(&state_file).unwrap();
    fs::write(&state_file, text.replace("Striker A", "Striker X")).unwrap();
    assert_eq!(
        combat_sim::integrity::changes(&root).unwrap(),
        vec![combat_sim::integrity::Change::Modified(
            "combat/v13.state.json".into()
        )]
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn add_to_a_grid_scene_places_on_a_free_cell() {
    let temp = TempState::new("add-grid");
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
    assert_success(&output, "new grid fight");
    assert_success(&run(temp.path(), &["init", "--set", "Adept A=20"]), "init");

    let output = run(
        temp.path(),
        &[
            "add",
            "--creature",
            "resistant-r@statue",
            "--initiative",
            "12",
        ],
    );
    assert_success(&output, "add to a grid scene");
    let status = status_json(temp.path());
    let cells: Vec<[i64; 2]> = status["combatants"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|combatant| {
            combatant["pos"]
                .as_array()
                .map(|pos| [pos[0].as_i64().unwrap(), pos[1].as_i64().unwrap()])
        })
        .collect();
    assert_eq!(cells.len(), 4, "{cells:?}");
    let mut sorted = cells.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        cells.len(),
        sorted.len(),
        "positions must not overlap: {cells:?}"
    );
    let added = status["combatants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|combatant| combatant["name"] == "Resistant R")
        .expect("Resistant R should be present")
        .clone();
    assert_eq!(added["zone"], "statue");
    assert_eq!(added["pos"], serde_json::json!([1, 4]));
}

// ---------------------------------------------------------------------------
// v1.6 T4: spell-cast settlement through the real binary (TASK-0126).
// ---------------------------------------------------------------------------

const SPELL_LIBRARY: &str = r#"---
abilities:
  - id: test-bolt
    name: Test Bolt
    tags: [spell]
    activation: action
    cost: {resource: spell-slot-1, amount: 1}
    slot_level: 1
    range: 60
    target: {type: creature, side: enemy, count: 1}
    effects:
      - attack:
          to_hit: 5
          kind: ranged
          damage: [{dice: 1d6, type: fire}]
    spell:
      level: 1
      school: evocation
      casting_time: {amount: 1, unit: action}
      components: {verbal: true, somatic: true}
      ritual: false
      upcast:
        text: higher
        dice_by_level: {2: 2d6, 3: 3d6, 5: 5d6}
  - id: gem-cast
    name: Gem Cast
    tags: [spell]
    activation: action
    cost: {resource: spell-slot-1, amount: 1}
    slot_level: 1
    target: {type: self}
    effects:
      - note: gem
    spell:
      level: 1
      casting_time: {amount: 1, unit: action}
      components:
        verbal: true
        material: {text: a diamond worth 50 gp, cost_gp: 50}
  - id: slow-ritual
    name: Slow Ritual
    tags: [spell]
    activation: action
    cost: {resource: spell-slot-1, amount: 1}
    slot_level: 1
    target: {type: self}
    effects:
      - note: long
    spell:
      level: 1
      casting_time: {amount: 1, unit: minute}
      components: {verbal: true}
  - id: no-meta-bolt
    name: No Meta Bolt
    activation: action
    cost: {resource: spell-slot-2, amount: 1}
    slot_level: 2
    range: 60
    target: {type: creature, side: enemy, count: 1}
    effects:
      - attack:
          to_hit: 5
          kind: ranged
          damage: [{dice: 1d8, type: force}]

creatures:
  - id: caster-x
    name: Caster X
    kind: pc
    side: party
    ac: 12
    hp: 30
    speed: {walk: 30}
    init_bonus: 5
    prof: 2
    abilities: {str: 8, dex: 12, con: 14, int: 16, wis: 10, cha: 10}
    resources:
      spell-slot-1: {max: 4, recharge: long}
      spell-slot-2: {max: 3, recharge: long}
    actions:
      - {ref: test-bolt}
      - {ref: gem-cast}
      - {ref: slow-ritual}
      - {ref: no-meta-bolt}
  - id: brute-b
    name: Brute B
    kind: monster
    side: enemy
    ac: 10
    hp: 100
    speed: {walk: 30}
    init_bonus: 0
    prof: 2
    abilities: {str: 16, dex: 10, con: 14, int: 8, wis: 10, cha: 8}

scenes:
  - id: arena
    name: Spell Arena
    mode: zones
    zones:
      - {id: alpha}
      - {id: beta}
    edges:
      - {a: alpha, b: beta, distance: 10}

encounters:
  - id: spell-fight
    name: Spell Fight
    scene: arena
    party_zone: alpha
    members:
      - {creature: brute-b, zone: beta}
"#;

struct TempCampaign {
    root: PathBuf,
}

impl TempCampaign {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "combat-sim-cli-t4-{label}-{}-{nanos}-{count}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("library")).expect("campaign dir should create");
        fs::write(
            root.join("manifest.yaml"),
            "name: Spell CLI Fixtures\nlibrary: [library]\nstate_dir: state\n",
        )
        .expect("manifest should write");
        fs::write(root.join("library/spells.yaml"), SPELL_LIBRARY).expect("library should write");
        Self { root }
    }

    fn manifest(&self) -> PathBuf {
        self.root.join("manifest.yaml")
    }
}

impl Drop for TempCampaign {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn spell_status(state: &Path, manifest: &Path) -> Value {
    let output = run_with_campaign(state, manifest, &["status", "--json"]);
    assert_success(&output, "status --json");
    serde_json::from_slice(&output.stdout).expect("status JSON should parse")
}

fn run_with_campaign(state: &Path, manifest: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--state")
        .arg(state)
        .arg("--campaign")
        .arg(manifest)
        .args(args)
        .output()
        .expect("failed to run combat-sim")
}

fn wait_for_turn(state: &Path, manifest: &Path, name: &str) {
    for _ in 0..10 {
        let active = spell_status(state, manifest)["active"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if active.starts_with(name) {
            return;
        }
        assert_success(
            &run_with_campaign(state, manifest, &["next", "--seed", "1"]),
            "next",
        );
    }
    panic!("{name} never became active");
}

fn resource_of(status: &Value, name: &str, id: &str) -> i64 {
    let combatants = status["combatants"].as_array().expect("combatants array");
    let combatant = combatants
        .iter()
        .find(|c| c["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("missing combatant {name}"));
    combatant["resources"][id]["current"]
        .as_i64()
        .unwrap_or_else(|| panic!("missing resource {id} for {name}"))
}
#[test]
fn cast_slot_pays_and_upcasts_through_the_cli() {
    let temp = TempState::new("t4-slot");
    let campaign = TempCampaign::new("slot");
    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &["new", "--encounter", "spell-fight", "--party", "caster-x"],
    );
    assert_success(&output, "new");
    assert_success(
        &run_with_campaign(temp.path(), &campaign.manifest(), &["init", "--seed", "42"]),
        "init",
    );
    wait_for_turn(temp.path(), &campaign.manifest(), "Caster");

    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &[
            "cast",
            "Caster",
            "test-bolt",
            "Brute",
            "--slot",
            "2",
            "--seed",
            "1",
        ],
    );
    assert_success(&output, "cast --slot 2");
    let text = stdout(&output);
    assert!(text.contains("spends 1 spell-slot-2"), "{text}");
    assert!(text.contains("2d6("), "{text}");

    let status = spell_status(temp.path(), &campaign.manifest());
    assert_eq!(
        resource_of(&status, "Caster X", "spell-slot-2"),
        2,
        "slot 2 charged"
    );
    assert_eq!(
        resource_of(&status, "Caster X", "spell-slot-1"),
        4,
        "base slot untouched"
    );
}

#[test]
fn cast_material_rejection_leaves_the_state_unsaved() {
    let temp = TempState::new("t4-material");
    let campaign = TempCampaign::new("material");
    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &["new", "--encounter", "spell-fight", "--party", "caster-x"],
    );
    assert_success(&output, "new");
    assert_success(
        &run_with_campaign(temp.path(), &campaign.manifest(), &["init", "--seed", "42"]),
        "init",
    );
    wait_for_turn(temp.path(), &campaign.manifest(), "Caster");

    // Missing declaration: the cast fails and nothing is persisted.
    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &["cast", "Caster", "gem-cast", "--seed", "1"],
    );
    assert!(
        !output.status.success(),
        "costly material must fail without a declaration"
    );
    assert!(
        stderr(&output).contains("needs a material component that costs 50 gp"),
        "{}",
        stderr(&output)
    );
    let status = spell_status(temp.path(), &campaign.manifest());
    assert_eq!(
        resource_of(&status, "Caster X", "spell-slot-1"),
        4,
        "failed casts are not saved"
    );

    // With the declaration the cast succeeds and the log records it.
    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &[
            "cast",
            "Caster",
            "gem-cast",
            "--material",
            "a diamond worth 50 gp",
            "--seed",
            "1",
        ],
    );
    assert_success(&output, "cast with material");
    assert!(
        stdout(&output).contains("MATERIAL DECLARED: a diamond worth 50 gp"),
        "{}",
        stdout(&output)
    );

    // The action was spent: move past this turn, then wait for the next one.
    assert_success(
        &run_with_campaign(temp.path(), &campaign.manifest(), &["next", "--seed", "1"]),
        "next",
    );
    wait_for_turn(temp.path(), &campaign.manifest(), "Caster");

    // The DM override succeeds but logs COMPONENTS IGNORED.
    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &[
            "cast",
            "Caster",
            "gem-cast",
            "--ignore-components",
            "--seed",
            "1",
        ],
    );
    assert_success(&output, "cast with ignore-components");
    assert!(
        stdout(&output).contains("COMPONENTS IGNORED"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn cast_rejects_minute_casting_time() {
    let temp = TempState::new("t4-minute");
    let campaign = TempCampaign::new("minute");
    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &["new", "--encounter", "spell-fight", "--party", "caster-x"],
    );
    assert_success(&output, "new");
    assert_success(
        &run_with_campaign(temp.path(), &campaign.manifest(), &["init", "--seed", "42"]),
        "init",
    );
    wait_for_turn(temp.path(), &campaign.manifest(), "Caster");

    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &["cast", "Caster", "slow-ritual", "--seed", "1"],
    );
    assert!(
        !output.status.success(),
        "minute casts fail closed in combat"
    );
    assert!(
        stderr(&output).contains("cannot be completed within a combat round"),
        "{}",
        stderr(&output)
    );
    let status = spell_status(temp.path(), &campaign.manifest());
    assert_eq!(
        resource_of(&status, "Caster X", "spell-slot-1"),
        4,
        "nothing spent"
    );
}

#[test]
fn cast_legacy_ability_keeps_the_old_slot_meaning() {
    let temp = TempState::new("t4-legacy");
    let campaign = TempCampaign::new("legacy");
    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &["new", "--encounter", "spell-fight", "--party", "caster-x"],
    );
    assert_success(&output, "new");
    assert_success(
        &run_with_campaign(temp.path(), &campaign.manifest(), &["init", "--seed", "42"]),
        "init",
    );
    wait_for_turn(temp.path(), &campaign.manifest(), "Caster");

    let output = run_with_campaign(
        temp.path(),
        &campaign.manifest(),
        &[
            "cast",
            "Caster",
            "no-meta-bolt",
            "Brute",
            "--slot",
            "5",
            "--seed",
            "1",
        ],
    );
    assert_success(&output, "legacy cast with --slot");
    let text = stdout(&output);
    assert!(text.contains("spends 1 spell-slot-2"), "{text}");
    assert!(!text.contains("spell-slot-5"), "{text}");

    let status = spell_status(temp.path(), &campaign.manifest());
    assert_eq!(
        resource_of(&status, "Caster X", "spell-slot-2"),
        2,
        "base cost charged"
    );
}
