//! TASK-0117 (v1.5c): structured Ready actions, triggers, reactions and expiry.
//!
//! Synthetic content only: no player books, real saves or DM secrets.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use combat_sim::content::schema::DoorSpec;
use combat_sim::content::{self, ContentRegistry, ContentSources};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{
    self, AddConditionOptions, AttackOptions, MoveTarget, ReadyOptions, ReadyOutcome, UseOptions,
};
use combat_sim::state::{
    ConcentrationState, EncounterState, NewEncounter, ReadyResponse, ReadyState, ReadyTrigger,
    STATE_SCHEMA_VERSION,
};
use serde_json::Value;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "combat-sim-ready-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("library")).expect("create scratch content");
        Self { dir }
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture parent");
        }
        fs::write(&path, text).expect("write fixture");
        path
    }

    fn manifest(&self) -> PathBuf {
        self.dir.join("manifest.yaml")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[derive(Debug, Default)]
struct ScriptedRng {
    values: Vec<u32>,
    next: usize,
}

impl ScriptedRng {
    fn new(values: &[u32]) -> Self {
        Self {
            values: values.to_vec(),
            next: 0,
        }
    }
}

impl DiceRng for ScriptedRng {
    fn roll_die(&mut self, sides: u32) -> u32 {
        let value = self
            .values
            .get(self.next)
            .copied()
            .unwrap_or_else(|| panic!("scripted RNG ran out while rolling d{sides}"));
        self.next += 1;
        assert!(
            value <= sides,
            "scripted value {value} does not fit on d{sides}"
        );
        value
    }
}

fn conditions_block(edition: &str) -> &'static str {
    match edition {
        "2014" => {
            r#"  - id: concentrating
  - id: incapacitated
    flags: [incapacitated]
  - id: no-reactions
    flags: [no_reactions]
  - id: prone
    flags: [stand_up_half_speed]
  - id: unconscious
    implies: [incapacitated]
    flags: [speed_zero]
"#
        }
        "2024" => {
            r#"  - id: concentrating
  - id: incapacitated
    flags: [incapacitated]
  - id: no-reactions
    flags: [no_reactions]
  - id: prone
    flags: [stand_up_half_speed]
  - id: unconscious
    implies: [incapacitated]
    flags: [speed_zero]
"#
        }
        other => panic!("unknown edition {other}"),
    }
}

fn content_yaml(edition: &str, max_responses: u32) -> String {
    let text = r##"rules:
  dying_conditions: [unconscious]
  concentration_condition: concentrating
  ready:
    max_responses: __MAX__
    expires: start_of_next_turn
conditions:
__CONDITIONS__

creatures:
  - id: hero
    name: Hero
    kind: pc
    side: party
    ac: 14
    hp: 40
    speed: {walk: 30}
    prof: 2
    abilities: {str: 16, dex: 14, con: 12, int: 10, wis: 10, cha: 10}
    skills: {athletics: 5, perception: 3}
    resources:
      spell-slot-1: {max: 2, recharge: long}
    attacks:
      - {id: sword, name: Sword, kind: melee, ability: str, to_hit: 5, damage: [{dice: "1d8+3", type: slashing}], reach: 5}
    actions:
      - id: damaging-bolt
        name: Damaging Bolt
        activation: action
        target: {type: creature, side: any, count: 1}
        effects:
          - damage: {dice: "1d6", type: fire}
      - id: arcane-bolt
        name: Arcane Bolt
        activation: action
        target: {type: creature, side: any, count: 1}
        range: 60
        cost: {resource: spell-slot-1, amount: 1}
        effects:
          - damage: {dice: "1d8", type: fire}
      - id: blessing-aura
        name: Blessing Aura
        activation: action
        concentration: true
        target: {type: self}
        effects:
          - note: "held blessing"
      - id: hex-bolt
        name: Hex Bolt
        activation: action
        concentration: true
        target: {type: creature, side: any, count: 1}
        range: 60
        cost: {resource: spell-slot-1, amount: 1}
        effects:
          - damage: {dice: "1d6", type: fire}
      - id: self-shield
        name: Self Shield
        activation: action
        target: {type: self}
        effects:
          - apply_condition: {condition: prone, to: self, duration: {permanent: true}}
      - id: quick-step
        name: Quick Step
        activation: bonus
        target: {type: self}
        effects:
          - basic_action: dash
  - id: source
    name: Source
    kind: monster
    side: enemy
    ac: 12
    hp: 30
    speed: {walk: 30}
    prof: 2
    abilities: {str: 12, dex: 12, con: 12, int: 10, wis: 10, cha: 12}
    attacks:
      - {id: claw, name: Claw, kind: melee, ability: str, to_hit: 4, damage: [{dice: "1d6+2", type: slashing}], reach: 5}
    actions:
      - id: damaging-bolt
        name: Damaging Bolt
        activation: action
        target: {type: creature, side: any, count: 1}
        effects:
          - damage: {dice: "1d6", type: fire}
  - id: bystander
    name: Bystander
    kind: monster
    side: enemy
    ac: 12
    hp: 20
    speed: {walk: 30}
    prof: 2
    abilities: {str: 10, dex: 10, con: 10, int: 8, wis: 8, cha: 8}
    attacks:
      - {id: jab, name: Jab, kind: melee, ability: str, to_hit: 2, damage: [{dice: "1d4", type: piercing}], reach: 5}

scenes:
  - id: zones
    name: Zones
    mode: zones
    zones:
      - {id: alpha}
      - {id: beta}
      - {id: gamma}
    edges:
      - {a: alpha, b: beta, distance: 5}
      - {a: beta, b: gamma, distance: 5}

encounters:
  - id: zones-fight
    scene: zones
    party_zone: alpha
    members:
      - {creature: source, zone: beta}
      - {creature: bystander, zone: gamma}
"##;
    text.replace("__MAX__", &max_responses.to_string())
        .replace("__CONDITIONS__", conditions_block(edition))
}

fn campaign(edition: &str) -> Scratch {
    let scratch = Scratch::new(edition);
    scratch.write(
        "manifest.yaml",
        &format!("name: Combat Ready {edition}\nruleset: \"{edition}\"\nlibrary: [library]\n"),
    );
    scratch.write("library/all.yaml", &content_yaml(edition, 1));
    scratch
}

fn registry(edition: &str) -> (Scratch, ContentRegistry) {
    let scratch = campaign(edition);
    let registry = ContentSources {
        campaign: Some(scratch.manifest()),
        ..ContentSources::default()
    }
    .load()
    .expect("synthetic ready content should load");
    (scratch, registry)
}

fn make_state(edition: &str) -> EncounterState {
    let (_scratch, registry) = registry(edition);
    EncounterState::from_encounter(
        registry,
        &NewEncounter {
            encounter: "zones-fight".to_string(),
            party: vec!["hero".to_string()],
            party_zone: None,
            adds: Vec::new(),
        },
    )
    .expect("synthetic encounter should build")
}

fn index(state: &EncounterState, name: &str) -> usize {
    state.find_index(name).expect("combatant exists")
}

fn slot(state: &EncounterState, name: &str) -> i32 {
    state.combatants[index(state, name)]
        .resources
        .get("spell-slot-1")
        .map(|resource| resource.current)
        .unwrap_or(-1)
}

fn has_condition(state: &EncounterState, name: &str, condition: &str) -> bool {
    state.combatants[index(state, name)]
        .conditions
        .iter()
        .any(|active| active.condition.eq_ignore_ascii_case(condition))
}

fn attack_response(attack: &str, target: Option<&str>) -> ReadyResponse {
    ReadyResponse::Attack {
        attack: attack.to_string(),
        target: target.map(str::to_string),
    }
}

fn options(trigger: ReadyTrigger, description: &str, response: ReadyResponse) -> ReadyOptions {
    ReadyOptions {
        trigger,
        description: description.to_string(),
        response,
    }
}

fn set_ready(
    state: &mut EncounterState,
    who: &str,
    trigger: ReadyTrigger,
    response: ReadyResponse,
) -> anyhow::Result<Vec<String>> {
    let mut log = Vec::new();
    engine::ready(
        state,
        who,
        &options(trigger, "synthetic trigger", response),
        &mut log,
    )?;
    Ok(log)
}

fn add_condition(state: &mut EncounterState, who: &str, condition: &str) -> Vec<String> {
    let mut log = Vec::new();
    engine::add_condition(
        state,
        who,
        condition,
        &AddConditionOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect("condition should apply");
    log
}

fn run_cli(state_path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--state")
        .arg(state_path)
        .args(args)
        .output()
        .expect("failed to run combat-sim")
}

fn run_help(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("failed to run combat-sim --help")
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

fn status_json(path: &Path) -> Value {
    let output = run_cli(path, &["status", "--json"]);
    assert_success(&output, "status --json");
    serde_json::from_slice(&output.stdout).expect("status JSON")
}

fn combatant<'a>(status: &'a Value, name: &str) -> &'a Value {
    status["combatants"]
        .as_array()
        .expect("combatants")
        .iter()
        .find(|combatant| combatant["name"] == name)
        .expect("combatant present")
}

fn state_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "combat-sim-ready-{label}-{}-{}.state.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

// ---------------------------------------------------------------------------
// AC-1: ready set, repeat, no action, incapacitated, no reactions
// ---------------------------------------------------------------------------

#[test]
fn ready_spends_the_action_and_stores_one_response_for_both_editions() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition);
        let hero = index(&state, "Hero");
        let mut log = Vec::new();
        engine::ready(
            &mut state,
            "Hero",
            &options(
                ReadyTrigger::Manual,
                "when the door opens",
                attack_response("sword", Some("Source")),
            ),
            &mut log,
        )
        .unwrap();

        assert!(state.combatants[hero].action_used, "{edition}: action");
        assert!(
            !state.combatants[hero].bonus_action_used,
            "{edition}: bonus"
        );
        let ready = state.combatants[hero]
            .ready
            .as_ref()
            .expect("ready state stored");
        assert_eq!(ready.trigger, ReadyTrigger::Manual);
        assert_eq!(ready.description, "when the door opens");
        assert_eq!(
            ready.response,
            attack_response("sword", Some("Source")),
            "{edition}: response"
        );
        assert!(
            log.iter().any(|line| line.contains("readies")),
            "{edition}: set log: {log:?}"
        );

        // A second ready fails closed and never overwrites the first one.
        let error = engine::ready(
            &mut state,
            "Hero",
            &options(
                ReadyTrigger::Attacked,
                "second",
                attack_response("sword", None),
            ),
            &mut Vec::new(),
        )
        .expect_err("repeat ready");
        assert!(
            error.to_string().contains("readied action"),
            "{edition}: {error}"
        );
        assert_eq!(
            state.combatants[hero].ready.as_ref().unwrap().trigger,
            ReadyTrigger::Manual,
            "{edition}: first ready survives"
        );
    }
}

#[test]
fn ready_refuses_without_an_action_or_while_unable_and_spends_nothing() {
    for edition in ["2014", "2024"] {
        // No action left this turn.
        let mut state = make_state(edition);
        let hero = index(&state, "Hero");
        state.combatants[hero].action_used = true;
        let error = set_ready(
            &mut state,
            "Hero",
            ReadyTrigger::Manual,
            attack_response("sword", None),
        )
        .expect_err("no action");
        assert!(error.to_string().contains("action"), "{edition}: {error}");
        assert!(
            state.combatants[hero].ready.is_none(),
            "{edition}: no state"
        );
        assert!(state.combatants[hero].action_used, "{edition}: unchanged");

        // Incapacitated.
        let mut state = make_state(edition);
        let hero = index(&state, "Hero");
        add_condition(&mut state, "Hero", "incapacitated");
        let error = set_ready(
            &mut state,
            "Hero",
            ReadyTrigger::Manual,
            attack_response("sword", None),
        )
        .expect_err("incapacitated");
        assert!(
            error.to_string().contains("incapacitated"),
            "{edition}: {error}"
        );
        assert!(
            state.combatants[hero].ready.is_none(),
            "{edition}: no state"
        );
        assert!(
            !state.combatants[hero].action_used,
            "{edition}: action unspent"
        );

        // No reaction ability.
        let mut state = make_state(edition);
        let hero = index(&state, "Hero");
        add_condition(&mut state, "Hero", "no-reactions");
        let error = set_ready(
            &mut state,
            "Hero",
            ReadyTrigger::Manual,
            attack_response("sword", None),
        )
        .expect_err("no reactions");
        assert!(error.to_string().contains("reaction"), "{edition}: {error}");
        assert!(
            state.combatants[hero].ready.is_none(),
            "{edition}: no state"
        );
        assert!(
            !state.combatants[hero].action_used,
            "{edition}: action unspent"
        );
    }
}

#[test]
fn ready_rejects_bonus_action_abilities_and_unknown_responses() {
    let mut state = make_state("2024");
    let hero = index(&state, "Hero");
    let error = set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "quick-step".to_string(),
            target: None,
        },
    )
    .expect_err("bonus action");
    assert!(error.to_string().contains("bonus"), "{error}");
    assert!(state.combatants[hero].ready.is_none());
    assert!(!state.combatants[hero].action_used);

    let error = set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("no-such-weapon", None),
    )
    .expect_err("unknown weapon");
    assert!(error.to_string().contains("no-such-weapon"), "{error}");
    assert!(state.combatants[hero].ready.is_none());
    assert!(!state.combatants[hero].action_used);
}

// ---------------------------------------------------------------------------
// AC-2 / AC-3: explicit trigger, reaction, expiry
// ---------------------------------------------------------------------------

#[test]
fn explicit_trigger_executes_the_attack_and_spends_one_reaction() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition);
        state.round = 1;
        state.initiative = vec![index(&state, "Hero"), index(&state, "Source")];
        state.turn = 1;
        let hero = index(&state, "Hero");
        let source = index(&state, "Source");
        set_ready(
            &mut state,
            "Hero",
            ReadyTrigger::Manual,
            attack_response("sword", None),
        )
        .unwrap();

        let mut log = Vec::new();
        let outcome = engine::trigger_ready(
            &mut state,
            "Hero",
            Some("Source"),
            &mut ScriptedRng::new(&[15, 4]),
            &mut log,
        )
        .unwrap();
        assert_eq!(outcome, ReadyOutcome::Triggered, "{edition}");
        assert!(
            state.combatants[source].hp < 30,
            "{edition}: response damage"
        );
        assert_eq!(
            state.combatants[hero].reaction_round,
            Some(1),
            "{edition}: reaction spend"
        );
        assert!(
            state.combatants[hero].ready.is_none(),
            "{edition}: consumed"
        );
        assert!(
            log.iter().any(|line| line.contains("triggers")),
            "{edition}: trigger log: {log:?}"
        );
        assert!(
            log.iter().any(|line| line.contains("reaction")),
            "{edition}: reaction log: {log:?}"
        );

        // Exactly once: a second explicit trigger has nothing left.
        let error = engine::trigger_ready(
            &mut state,
            "Hero",
            Some("Source"),
            &mut ScriptedRng::new(&[]),
            &mut Vec::new(),
        )
        .expect_err("nothing left");
        assert!(
            error.to_string().contains("readied action"),
            "{edition}: {error}"
        );
    }
}

#[test]
fn ready_ability_pays_the_slot_at_setup_and_not_again_on_trigger() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition);
        state.round = 1;
        let hero = index(&state, "Hero");
        let source = index(&state, "Source");
        assert_eq!(slot(&state, "Hero"), 2, "{edition}: fixture slot");
        set_ready(
            &mut state,
            "Hero",
            ReadyTrigger::Manual,
            ReadyResponse::Ability {
                ability: "arcane-bolt".to_string(),
                target: Some("Source".to_string()),
            },
        )
        .unwrap();
        assert_eq!(slot(&state, "Hero"), 1, "{edition}: paid at setup");
        assert!(state.combatants[hero].action_used, "{edition}: action");
        assert!(
            !state.concentration.contains_key("Hero"),
            "{edition}: non-concentration ability does not concentrate"
        );

        let mut log = Vec::new();
        let outcome = engine::trigger_ready(
            &mut state,
            "Hero",
            Some("Source"),
            &mut ScriptedRng::new(&[5]),
            &mut log,
        )
        .unwrap();
        assert_eq!(outcome, ReadyOutcome::Triggered, "{edition}");
        assert_eq!(
            slot(&state, "Hero"),
            1,
            "{edition}: trigger must not pay again"
        );
        assert_eq!(state.combatants[source].hp, 25, "{edition}: 1d8=5");
        assert!(state.combatants[hero].ready.is_none());
    }
}

#[test]
fn ready_concentration_begins_at_setup_and_does_not_rebegin_on_trigger() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition);
        state.round = 1;
        state.initiative = vec![index(&state, "Hero"), index(&state, "Source")];
        state.turn = 0;
        let hero = index(&state, "Hero");
        set_ready(
            &mut state,
            "Hero",
            ReadyTrigger::Manual,
            ReadyResponse::Ability {
                ability: "Blessing Aura".to_string(),
                target: None,
            },
        )
        .unwrap();
        let concentration = state
            .concentration
            .get("Hero")
            .expect("concentration begins at setup");
        assert_eq!(concentration.ability.as_deref(), Some("blessing-aura"));
        assert_eq!(concentration.round, 1, "{edition}: setup round");
        assert!(has_condition(&state, "Hero", "concentrating"));

        state.round = 2;
        state.turn = 1;
        let mut log = Vec::new();
        let outcome = engine::trigger_ready(
            &mut state,
            "Hero",
            None,
            &mut ScriptedRng::new(&[]),
            &mut log,
        )
        .unwrap();
        assert_eq!(outcome, ReadyOutcome::Triggered, "{edition}");
        let concentration = state
            .concentration
            .get("Hero")
            .expect("concentration survives release");
        assert_eq!(
            concentration.ability.as_deref(),
            Some("blessing-aura"),
            "{edition}"
        );
        assert_eq!(
            concentration.round, 1,
            "{edition}: trigger must not re-begin concentration"
        );
        assert_eq!(
            state.combatants[hero]
                .conditions
                .iter()
                .filter(|condition| condition.condition == "concentrating")
                .count(),
            1,
            "{edition}: one concentration marker"
        );
        assert!(state.combatants[hero].ready.is_none());
    }
}

#[test]
fn expired_or_cleared_ready_does_not_refund_the_slot() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "arcane-bolt".to_string(),
            target: Some("Source".to_string()),
        },
    )
    .unwrap();
    assert_eq!(slot(&state, "Hero"), 1);

    let mut log = Vec::new();
    assert!(engine::clear_ready(&mut state, "Hero", "test clear", &mut log).unwrap());
    assert_eq!(slot(&state, "Hero"), 1, "clear does not refund");
    assert!(state.combatants[hero].ready.is_none());
    assert!(log.iter().any(|line| line.contains("cleared")));

    state.combatants[hero].action_used = false;
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "arcane-bolt".to_string(),
            target: Some("Source".to_string()),
        },
    )
    .unwrap();
    assert_eq!(slot(&state, "Hero"), 0);
    state.initiative = vec![hero, index(&state, "Source")];
    state.turn = 0;
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[]), &mut Vec::new()).unwrap();
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[]), &mut log).unwrap();
    assert!(state.combatants[hero].ready.is_none());
    assert_eq!(slot(&state, "Hero"), 0, "expiry does not refund");
    assert!(log.iter().any(|line| line.contains("expires")));
}

#[test]
fn concentration_ready_ends_only_its_own_concentration_on_clear() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "Blessing Aura".to_string(),
            target: None,
        },
    )
    .unwrap();
    assert!(state.concentration.contains_key("Hero"));
    let mut log = Vec::new();
    assert!(engine::clear_ready(&mut state, "Hero", "test clear", &mut log).unwrap());
    assert!(
        !state.concentration.contains_key("Hero"),
        "the readied concentration ends"
    );
    assert!(!has_condition(&state, "Hero", "concentrating"));
    assert!(
        log.iter().any(|line| line.contains("concentration ends")),
        "{log:?}"
    );

    // A different concentration must survive clearing a stale ready.
    state.combatants[hero].action_used = false;
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "blessing-aura".to_string(),
            target: None,
        },
    )
    .unwrap();
    state.concentration.insert(
        "Hero".to_string(),
        ConcentrationState {
            ability: Some("unrelated-spell".to_string()),
            round: 1,
        },
    );
    assert!(engine::clear_ready(&mut state, "Hero", "stale clear", &mut log).unwrap());
    assert_eq!(
        state
            .concentration
            .get("Hero")
            .and_then(|concentration| concentration.ability.as_deref()),
        Some("unrelated-spell"),
        "an unrelated concentration is not broken"
    );
}

#[test]
fn ready_concentration_replaces_an_existing_concentration_per_raw() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Hero",
        "blessing-aura",
        &UseOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(
        state
            .concentration
            .get("Hero")
            .and_then(|concentration| concentration.ability.as_deref()),
        Some("blessing-aura")
    );
    state.combatants[hero].action_used = false;
    state.round = 2;
    let log = set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "blessing-aura".to_string(),
            target: None,
        },
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("concentration ends")),
        "old concentration is interrupted: {log:?}"
    );
    assert!(
        log.iter().any(|line| line.contains("is concentrating")),
        "new concentration starts: {log:?}"
    );
    let concentration = state.concentration.get("Hero").expect("new concentration");
    assert_eq!(concentration.ability.as_deref(), Some("blessing-aura"));
    assert_eq!(concentration.round, 2, "setup round");
    assert!(state.combatants[hero].action_used);
}

#[test]
fn expiry_ends_the_ready_concentration_without_refund() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    state.initiative = vec![hero, index(&state, "Source")];
    state.turn = 0;
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "blessing-aura".to_string(),
            target: None,
        },
    )
    .unwrap();
    assert!(state.concentration.contains_key("Hero"));

    let mut log = Vec::new();
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[]), &mut log).unwrap();
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[]), &mut log).unwrap();
    assert!(state.combatants[hero].ready.is_none());
    assert!(
        !state.concentration.contains_key("Hero"),
        "expiry ends the ready concentration"
    );
    assert!(!has_condition(&state, "Hero", "concentrating"));
    assert!(log.iter().any(|line| line.contains("expires")), "{log:?}");
    assert!(
        log.iter().any(|line| line.contains("concentration ends")),
        "{log:?}"
    );
}

#[test]
fn zero_hp_clears_a_ready_and_ends_its_concentration_without_refund() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "hex-bolt".to_string(),
            target: Some("Source".to_string()),
        },
    )
    .unwrap();
    assert_eq!(slot(&state, "Hero"), 1);
    assert!(state.concentration.contains_key("Hero"));

    let mut log = Vec::new();
    engine::damage(
        &mut state,
        "Hero",
        "40",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[hero].hp, 0);
    assert!(state.combatants[hero].ready.is_none());
    assert!(
        !state.concentration.contains_key("Hero"),
        "0 HP ends the ready concentration"
    );
    assert_eq!(slot(&state, "Hero"), 1, "0 HP does not refund");
    assert!(log.iter().any(|line| line.contains("0 HP")), "{log:?}");
}

#[test]
fn insufficient_resource_or_illegal_response_fails_before_action_spend() {
    let mut state = make_state("2024");
    let hero = index(&state, "Hero");
    state.combatants[hero]
        .resources
        .get_mut("spell-slot-1")
        .unwrap()
        .current = 0;
    let error = set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "arcane-bolt".to_string(),
            target: Some("Source".to_string()),
        },
    )
    .expect_err("no slot");
    assert!(error.to_string().contains("afford"), "{error}");
    assert_eq!(slot(&state, "Hero"), 0);
    assert!(!state.combatants[hero].action_used);
    assert!(state.combatants[hero].ready.is_none());
    assert!(!state.concentration.contains_key("Hero"));

    state.combatants[hero]
        .resources
        .get_mut("spell-slot-1")
        .unwrap()
        .current = 2;
    let error = set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "arcane-bolt".to_string(),
            target: Some("Nobody".to_string()),
        },
    )
    .expect_err("unknown target");
    assert!(error.to_string().contains("Nobody"), "{error}");
    assert_eq!(slot(&state, "Hero"), 2, "legality failure does not pay");
    assert!(!state.combatants[hero].action_used);
    assert!(state.combatants[hero].ready.is_none());
}

#[test]
fn a_new_concentration_clears_a_held_ready_spell_without_refund() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "hex-bolt".to_string(),
            target: Some("Source".to_string()),
        },
    )
    .unwrap();
    assert_eq!(slot(&state, "Hero"), 1);
    assert_eq!(
        state
            .concentration
            .get("Hero")
            .and_then(|concentration| concentration.ability.as_deref()),
        Some("hex-bolt")
    );

    state.combatants[hero].action_used = false;
    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Hero",
        "blessing-aura",
        &UseOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter()
            .any(|line| line.contains("replaced the readied spell")),
        "{log:?}"
    );
    assert!(state.combatants[hero].ready.is_none());
    assert_eq!(slot(&state, "Hero"), 1, "the held slot is not refunded");
    assert_eq!(
        state
            .concentration
            .get("Hero")
            .and_then(|concentration| concentration.ability.as_deref()),
        Some("blessing-aura"),
        "the new concentration is active"
    );
}

#[test]
fn refused_trigger_keeps_the_paid_slot_and_concentration() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "hex-bolt".to_string(),
            target: Some("Source".to_string()),
        },
    )
    .unwrap();
    assert_eq!(slot(&state, "Hero"), 1);
    assert!(state.concentration.contains_key("Hero"));
    state.combatants[hero].reaction_round = Some(1);

    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        Some("Source"),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    match outcome {
        ReadyOutcome::Refused(reason) => assert!(reason.contains("reaction"), "{reason}"),
        other => panic!("expected refusal, got {other:?}"),
    }
    assert_eq!(slot(&state, "Hero"), 1, "refusal does not refund");
    assert!(state.combatants[hero].ready.is_some());
    assert!(state.concentration.contains_key("Hero"));

    assert!(engine::clear_ready(&mut state, "Hero", "after refusal", &mut log).unwrap());
    assert_eq!(slot(&state, "Hero"), 1);
    assert!(!state.concentration.contains_key("Hero"));
}

#[test]
fn refused_dead_target_keeps_the_paid_slot_and_concentration() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "hex-bolt".to_string(),
            target: Some("Source".to_string()),
        },
    )
    .unwrap();
    assert_eq!(slot(&state, "Hero"), 1);
    assert!(state.concentration.contains_key("Hero"));
    engine::damage(
        &mut state,
        "Source",
        "999",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        Some("Source"),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    match outcome {
        ReadyOutcome::Refused(reason) => assert!(reason.contains("dead"), "{reason}"),
        other => panic!("expected refusal, got {other:?}"),
    }
    assert_eq!(slot(&state, "Hero"), 1, "refusal does not refund");
    assert!(state.combatants[hero].ready.is_some());
    assert!(state.concentration.contains_key("Hero"));
    assert_eq!(state.combatants[hero].reaction_round, None);
}

#[test]
fn a_broken_ready_concentration_refuses_the_trigger_without_refund() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    let source = index(&state, "Source");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Ability {
            ability: "hex-bolt".to_string(),
            target: Some("Source".to_string()),
        },
    )
    .unwrap();
    state.concentration.remove("Hero");

    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        Some("Source"),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    match outcome {
        ReadyOutcome::Refused(reason) => assert!(reason.contains("concentration"), "{reason}"),
        other => panic!("expected refusal, got {other:?}"),
    }
    assert_eq!(slot(&state, "Hero"), 1, "no refund");
    assert_eq!(state.combatants[source].hp, 30, "no free release");
    assert!(state.combatants[hero].ready.is_some());
    assert_eq!(state.combatants[hero].reaction_round, None);
}

#[test]
fn explicit_trigger_needs_a_source_for_a_targeted_response() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("sword", None),
    )
    .unwrap();
    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        None,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    match outcome {
        ReadyOutcome::Refused(reason) => assert!(reason.contains("source"), "{reason}"),
        other => panic!("expected refusal, got {other:?}"),
    }
    assert!(state.combatants[hero].ready.is_some());
    assert_eq!(state.combatants[hero].reaction_round, None);
    assert!(log.iter().any(|line| line.contains("refused")), "{log:?}");
}

#[test]
fn invalid_target_refuses_the_trigger_without_spending_the_reaction() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    let bystander = index(&state, "Bystander");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("sword", Some("Bystander")),
    )
    .unwrap();
    engine::damage(
        &mut state,
        "Bystander",
        "999",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(state.combatants[bystander].hp, 0);

    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        Some("Source"),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    match outcome {
        ReadyOutcome::Refused(reason) => assert!(reason.contains("dead"), "{reason}"),
        other => panic!("expected refusal, got {other:?}"),
    }
    assert!(state.combatants[hero].ready.is_some(), "ready is retained");
    assert_eq!(
        state.combatants[hero].reaction_round, None,
        "reaction is not spent on a refused trigger"
    );
    assert!(log.iter().any(|line| line.contains("refused")), "{log:?}");
}

#[test]
fn an_already_spent_reaction_blocks_the_trigger_and_keeps_the_ready() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    state.combatants[hero].reaction_round = Some(1);
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("sword", None),
    )
    .unwrap();
    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        Some("Source"),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    match outcome {
        ReadyOutcome::Refused(reason) => assert!(reason.contains("reaction"), "{reason}"),
        other => panic!("expected refusal, got {other:?}"),
    }
    assert!(state.combatants[hero].ready.is_some());
    assert_eq!(state.combatants[hero].reaction_round, Some(1));
}

#[test]
fn an_out_of_range_target_refuses_the_trigger_without_spending_the_reaction() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    let source = index(&state, "Source");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("sword", Some("Source")),
    )
    .unwrap();
    state.combatants[source].set_zone("gamma");
    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        Some("Source"),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    match outcome {
        ReadyOutcome::Refused(reason) => {
            assert!(
                reason.contains("reach") || reason.contains("range"),
                "{reason}"
            )
        }
        other => panic!("expected refusal, got {other:?}"),
    }
    assert!(state.combatants[hero].ready.is_some());
    assert_eq!(state.combatants[hero].reaction_round, None);
}

#[test]
fn a_blocked_readied_move_stops_with_a_structured_log() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Move {
            zone: Some("beta".to_string()),
            cell: None,
        },
    )
    .unwrap();
    // The route was open when Ready was set; a door locks before the trigger.
    let scene = Arc::make_mut(state.scene.as_mut().expect("scene"));
    for edge in &mut scene.edges {
        if (edge.a == "alpha" && edge.b == "beta") || (edge.a == "beta" && edge.b == "alpha") {
            edge.door = Some(DoorSpec { locked: true });
        }
    }
    state.rebuild_caches();

    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        None,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(outcome, ReadyOutcome::Triggered);
    assert_eq!(state.combatants[hero].zone, "alpha");
    assert_eq!(state.combatants[hero].reaction_round, Some(1));
    assert!(
        log.iter()
            .any(|line| line.contains("locked door") || line.contains("stops")),
        "{log:?}"
    );
}

#[test]
fn ready_expires_at_the_holders_next_turn_start() {
    let mut state = make_state("2024");
    state.round = 1;
    state.initiative = vec![index(&state, "Hero"), index(&state, "Source")];
    state.turn = 0;
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("sword", None),
    )
    .unwrap();

    let mut log = Vec::new();
    // Source's turn starts: the manual ready does not trigger and survives.
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[10]), &mut log).unwrap();
    assert!(
        state.combatants[index(&state, "Hero")].ready.is_some(),
        "manual ready survives another creature's turn"
    );

    // Hero's own turn starts: the ready expires.
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[10]), &mut log).unwrap();
    assert!(
        state.combatants[index(&state, "Hero")].ready.is_none(),
        "expired at the holder's turn start"
    );
    assert_eq!(
        state.combatants[index(&state, "Hero")].reaction_round,
        None,
        "expiry spends no reaction"
    );
    assert!(
        log.iter().any(|line| line.contains("expires")),
        "expiry log: {log:?}"
    );
}

#[test]
fn a_ready_past_its_expiry_round_is_cleared_on_trigger() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("sword", None),
    )
    .unwrap();
    let expires = state.combatants[hero].ready.as_ref().unwrap().expires_round;
    state.round = expires + 1;
    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        Some("Source"),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(outcome, ReadyOutcome::Expired);
    assert!(state.combatants[hero].ready.is_none());
    assert_eq!(state.combatants[hero].reaction_round, None);
    assert_eq!(state.combatants[hero].hp, 40);
    assert!(log.iter().any(|line| line.contains("expired")), "{log:?}");
}

// ---------------------------------------------------------------------------
// AC-2: automatic triggers
// ---------------------------------------------------------------------------

#[test]
fn an_enemy_attack_triggers_a_readied_response() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition);
        state.round = 1;
        state.initiative = vec![index(&state, "Hero"), index(&state, "Source")];
        state.turn = 1;
        let hero = index(&state, "Hero");
        let source = index(&state, "Source");
        set_ready(
            &mut state,
            "Hero",
            ReadyTrigger::Attacked,
            attack_response("sword", None),
        )
        .unwrap();

        let before = state.combatants[source].hp;
        let mut log = Vec::new();
        // Source claw: d20 15 (+4) hits AC 14, damage 3. Hero response:
        // d20 12 (+5) hits AC 12, damage 4.
        engine::attack(
            &mut state,
            "Source",
            "Hero",
            "claw",
            &AttackOptions::default(),
            &mut ScriptedRng::new(&[15, 3, 12, 4]),
            &mut log,
        )
        .unwrap();
        assert!(
            state.combatants[source].hp < before,
            "{edition}: readied response landed"
        );
        assert_eq!(
            state.combatants[hero].reaction_round,
            Some(1),
            "{edition}: reaction"
        );
        assert!(state.combatants[hero].ready.is_none(), "{edition}");
        assert!(
            log.iter()
                .any(|line| line.contains("readied action triggers")),
            "{edition}: {log:?}"
        );
    }
}

#[test]
fn an_enemy_entering_reach_triggers_a_readied_response() {
    let mut state = make_state("2024");
    state.round = 1;
    state.initiative = vec![index(&state, "Hero"), index(&state, "Source")];
    state.turn = 0;
    let source = index(&state, "Source");
    state.combatants[source].set_zone("gamma");
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::EnterReach,
        attack_response("sword", None),
    )
    .unwrap();

    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Source",
        &MoveTarget::Zone("beta".to_string()),
        false,
        &mut ScriptedRng::new(&[14, 5]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[source].zone, "beta");
    assert!(
        state.combatants[source].hp < 30,
        "readied response: {log:?}"
    );
    assert_eq!(state.combatants[hero].reaction_round, Some(1));
    assert!(state.combatants[hero].ready.is_none());
}

#[test]
fn an_enemy_ability_triggers_a_readied_response() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    let source = index(&state, "Source");
    let hero_before = state.combatants[hero].hp;
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Ability,
        ReadyResponse::Ability {
            ability: "damaging-bolt".to_string(),
            target: None,
        },
    )
    .unwrap();

    let mut log = Vec::new();
    engine::use_ability(
        &mut state,
        "Source",
        "damaging-bolt",
        &UseOptions {
            targets: vec!["Hero".to_string()],
            ..UseOptions::default()
        },
        &mut ScriptedRng::new(&[4, 5]),
        &mut log,
    )
    .unwrap();
    assert!(
        state.combatants[hero].hp < hero_before,
        "enemy ability landed"
    );
    assert!(
        state.combatants[source].hp < 30,
        "readied ability landed: {log:?}"
    );
    assert_eq!(state.combatants[hero].reaction_round, Some(1));
    assert!(state.combatants[hero].ready.is_none());
}

#[test]
fn an_enemy_turn_start_triggers_a_readied_response() {
    let mut state = make_state("2014");
    state.round = 1;
    let hero = index(&state, "Hero");
    let source = index(&state, "Source");
    state.initiative = vec![hero, source];
    state.turn = 0;
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::TurnStart,
        attack_response("sword", None),
    )
    .unwrap();

    let mut log = Vec::new();
    engine::next_turn(&mut state, &mut ScriptedRng::new(&[13, 4]), &mut log).unwrap();
    assert!(
        state.combatants[source].hp < 30,
        "turn-start response: {log:?}"
    );
    assert_eq!(state.combatants[hero].reaction_round, Some(1));
    assert!(state.combatants[hero].ready.is_none());
    assert_eq!(
        state.current().map(|combatant| combatant.name.as_str()),
        Some("Source")
    );
}

// ---------------------------------------------------------------------------
// AC-2 / AC-3: movement response and cleanup
// ---------------------------------------------------------------------------

#[test]
fn a_readied_move_uses_the_existing_move_path() {
    let mut state = make_state("2024");
    state.round = 1;
    let source = index(&state, "Source");
    state.combatants[source].set_zone("gamma");
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Move {
            zone: Some("beta".to_string()),
            cell: None,
        },
    )
    .unwrap();

    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        None,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(outcome, ReadyOutcome::Triggered);
    assert_eq!(state.combatants[hero].zone, "beta");
    assert_eq!(state.combatants[hero].reaction_round, Some(1));
    assert!(state.combatants[hero].ready.is_none());
}

#[test]
fn a_readied_move_to_an_unknown_destination_fails_without_spending_the_action() {
    let mut state = make_state("2024");
    let hero = index(&state, "Hero");
    let error = set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        ReadyResponse::Move {
            zone: Some("nowhere".to_string()),
            cell: None,
        },
    )
    .expect_err("unknown zone");
    assert!(error.to_string().contains("unknown zone"), "{error}");
    assert!(state.combatants[hero].ready.is_none());
    assert!(!state.combatants[hero].action_used);
}

#[test]
fn incapacitation_and_death_clean_up_a_ready_action() {
    let mut state = make_state("2024");
    let hero = index(&state, "Hero");
    let source = index(&state, "Source");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("sword", None),
    )
    .unwrap();
    let mut log = Vec::new();
    engine::add_condition(
        &mut state,
        "Hero",
        "incapacitated",
        &AddConditionOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        state.combatants[hero].ready.is_none(),
        "incapacitation clears the ready"
    );
    assert!(
        log.iter().any(|line| line.contains("incapacitated")),
        "{log:?}"
    );

    // The enemy side can be ready too, and death clears it.
    set_ready(
        &mut state,
        "Source",
        ReadyTrigger::Manual,
        attack_response("claw", Some("Hero")),
    )
    .unwrap();
    let mut log = Vec::new();
    engine::damage(
        &mut state,
        "Source",
        "999",
        Some("slashing"),
        false,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert!(
        state.combatants[source].ready.is_none(),
        "death clears ready"
    );
    assert!(
        log.iter().any(|line| line.contains("readied action")),
        "{log:?}"
    );
}

#[test]
fn a_dormant_combatant_loses_its_ready() {
    let mut state = make_state("2024");
    state.round = 1;
    let hero = index(&state, "Hero");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("sword", None),
    )
    .unwrap();
    state.combatants[hero].dormant = true;
    let mut log = Vec::new();
    let outcome = engine::trigger_ready(
        &mut state,
        "Hero",
        Some("Source"),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(outcome, ReadyOutcome::Expired);
    assert!(state.combatants[hero].ready.is_none());
    assert!(
        log.iter().any(|line| line.contains("no longer active")),
        "{log:?}"
    );
}

// ---------------------------------------------------------------------------
// AC-4 / AC-5: old state, old commands, content and CLI
// ---------------------------------------------------------------------------

#[test]
fn old_combat_state_loads_with_no_ready_and_schema_one() {
    assert_eq!(STATE_SCHEMA_VERSION, 1);
    let mut state = make_state("2024");
    set_ready(
        &mut state,
        "Hero",
        ReadyTrigger::Manual,
        attack_response("sword", None),
    )
    .unwrap();
    let path = state_path("old-state");
    state.save(&path).unwrap();
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let hero = value["combatants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|combatant| combatant["name"] == "Hero")
        .expect("hero");
    assert!(hero.get("ready").is_some(), "{value}");
    for combatant in value["combatants"].as_array_mut().unwrap() {
        combatant.as_object_mut().unwrap().remove("ready");
    }
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let loaded = EncounterState::load(&path).unwrap();
    for combatant in &loaded.combatants {
        assert!(combatant.ready.is_none());
    }
    assert_eq!(loaded.schema_version, STATE_SCHEMA_VERSION);
    let _ = fs::remove_file(path);
}

#[test]
fn old_attack_force_and_use_keep_working_and_create_no_ready() {
    let state = make_state("2014");
    let path = state_path("old-cli");
    state.save(&path).unwrap();

    let output = run_cli(&path, &["attack", "Hero", "Source", "sword", "--force"]);
    assert_success(&output, "legacy attack --force");
    let status = status_json(&path);
    assert!(combatant(&status, "Hero").get("ready").is_none());

    // Reset the action counter for the legacy `use` check.
    let mut state = EncounterState::load(&path).unwrap();
    let hero = index(&state, "Hero");
    state.combatants[hero].action_used = false;
    state.combatants[hero].once_used.clear();
    state.save(&path).unwrap();
    let output = run_cli(
        &path,
        &["use", "Hero", "damaging-bolt", "Source", "--force"],
    );
    assert_success(&output, "legacy use --force");
    let status = status_json(&path);
    assert!(combatant(&status, "Hero").get("ready").is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn cli_help_lists_the_ready_commands_and_their_arguments() {
    let output = run_help(&["ready", "--help"]);
    assert_success(&output, "ready --help");
    let text = stdout(&output);
    for flag in [
        "--trigger",
        "--on",
        "--attack",
        "--unarmed",
        "--ability",
        "--target",
        "--zone",
        "--to",
    ] {
        assert!(text.contains(flag), "ready --help lacks {flag}:\n{text}");
    }

    let output = run_help(&["trigger", "--help"]);
    assert_success(&output, "trigger --help");
    assert!(stdout(&output).contains("--source"));

    let output = run_help(&["ready-clear", "--help"]);
    assert_success(&output, "ready-clear --help");
    assert!(stdout(&output).contains("WHO"));
}

#[test]
fn cli_ready_trigger_and_ready_clear_round_trip_through_status_json() {
    let state = make_state("2024");
    let path = state_path("cli-cycle");
    state.save(&path).unwrap();

    let output = run_cli(
        &path,
        &[
            "ready",
            "Hero",
            "--trigger",
            "an enemy attacks",
            "--on",
            "attacked",
            "--attack",
            "sword",
        ],
    );
    assert_success(&output, "ready");
    let status = status_json(&path);
    let ready = &combatant(&status, "Hero")["ready"];
    assert_eq!(ready["trigger"], "attacked");
    assert_eq!(ready["description"], "an enemy attacks");
    assert_eq!(ready["response"]["kind"], "attack");
    assert_eq!(ready["response"]["attack"], "sword");

    // Clear it explicitly.
    let output = run_cli(&path, &["ready-clear", "Hero"]);
    assert_success(&output, "ready-clear");
    let status = status_json(&path);
    assert!(combatant(&status, "Hero").get("ready").is_none());

    // Set it again (the action still counts as used, so open a new turn).
    let mut state = EncounterState::load(&path).unwrap();
    let hero = index(&state, "Hero");
    state.combatants[hero].action_used = false;
    state.save(&path).unwrap();
    let output = run_cli(
        &path,
        &[
            "ready",
            "Hero",
            "--trigger",
            "an enemy attacks",
            "--on",
            "attacked",
            "--attack",
            "sword",
        ],
    );
    assert_success(&output, "second ready");
    let output = run_cli(&path, &["trigger", "Hero", "--source", "Source"]);
    assert_success(&output, "trigger");
    let status = status_json(&path);
    assert!(combatant(&status, "Hero").get("ready").is_none());
    assert!(stdout(&output).contains("reaction"), "{}", stdout(&output));

    let _ = fs::remove_file(path);
}

#[test]
fn cli_refuses_ready_outside_the_holders_turn() {
    let mut state = make_state("2024");
    state.round = 1;
    state.initiative = vec![index(&state, "Hero"), index(&state, "Source")];
    state.turn = 1;
    let path = state_path("cli-out-of-turn");
    state.save(&path).unwrap();
    let output = run_cli(
        &path,
        &[
            "ready",
            "Hero",
            "--trigger",
            "an enemy attacks",
            "--attack",
            "sword",
        ],
    );
    assert!(!output.status.success(), "out-of-turn ready must fail");
    assert!(stderr(&output).contains("turn"), "{}", stderr(&output));
    let status = status_json(&path);
    assert!(combatant(&status, "Hero").get("ready").is_none());
    let _ = fs::remove_file(path);
}

#[test]
fn content_check_validates_the_ready_rule_and_both_editions_load_it() {
    for edition in ["2014", "2024"] {
        let (_scratch, registry) = registry(edition);
        let report = content::check(&registry);
        assert!(!report.has_errors(), "{edition}: {:#?}", report.issues);
        assert_eq!(
            registry.rules.ready.as_ref().unwrap().max_responses(),
            1,
            "{edition}"
        );
    }

    let scratch = Scratch::new("bad-ready");
    scratch.write(
        "manifest.yaml",
        "name: Bad Ready\nruleset: \"2024\"\nlibrary: [library]\n",
    );
    scratch.write(
        "library/all.yaml",
        &content_yaml("2024", 2).replace(
            "    max_responses: 2",
            "    max_responses: 2\n    unsupported: true",
        ),
    );
    let error = ContentSources {
        campaign: Some(scratch.manifest()),
        ..ContentSources::default()
    }
    .load()
    .expect_err("unknown ready field");
    assert!(
        error.to_string().contains("unsupported"),
        "unknown fields fail closed: {error}"
    );

    let scratch = Scratch::new("bad-ready-count");
    scratch.write(
        "manifest.yaml",
        "name: Bad Ready Count\nruleset: \"2024\"\nlibrary: [library]\n",
    );
    scratch.write("library/all.yaml", &content_yaml("2024", 2));
    let registry = ContentSources {
        campaign: Some(scratch.manifest()),
        ..ContentSources::default()
    }
    .load()
    .expect("load");
    let report = content::check(&registry);
    let messages = report
        .issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(messages.contains("max_responses"), "{messages}");
}

#[test]
fn ready_state_serializes_all_response_kinds() {
    let responses = [
        attack_response("sword", Some("Source")),
        ReadyResponse::Unarmed {
            target: Some("Source".to_string()),
        },
        ReadyResponse::Ability {
            ability: "damaging-bolt".to_string(),
            target: Some("Source".to_string()),
        },
        ReadyResponse::Move {
            zone: Some("beta".to_string()),
            cell: None,
        },
    ];
    for response in responses {
        let ready = ReadyState {
            trigger: ReadyTrigger::Attacked,
            description: "audit trail".to_string(),
            response: response.clone(),
            set_round: 2,
            expires_round: 3,
        };
        let json = serde_json::to_value(&ready).expect("serialize");
        let round: ReadyState = serde_json::from_value(json).expect("deserialize");
        assert_eq!(round, ready);
    }
    assert_eq!(
        serde_json::to_value(ReadyTrigger::EnterReach).unwrap(),
        serde_json::json!("enter_reach")
    );
}
