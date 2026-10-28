//! TASK-0116 (v1.5b): Bloodied, Exhaustion, Charmed and Frightened.
//!
//! Synthetic content only: no player books, real saves or DM secrets.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use combat_sim::content::{self, ContentRegistry, ContentSources};
use combat_sim::dice::{DiceRng, RollMode};
use combat_sim::engine::{
    self, AddConditionOptions, AttackOptions, GrappleOptions, MoveTarget, RestKind, ShoveOptions,
    UseOptions,
};
use combat_sim::space::SpatialPose;
use combat_sim::state::{
    ActiveCondition, ConcentrationState, EncounterState, NewEncounter, STATE_SCHEMA_VERSION,
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
            "combat-sim-conditions-{label}-{}-{n}",
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

fn rules_block(edition: &str) -> &'static str {
    match edition {
        "2014" => {
            r#"  exhaustion:
    condition: exhaustion
    max_level: 6
    death_level: 6
    long_rest_reduction: 1
    check_disadvantage_from: 1
    speed_halved_from: 2
    attack_save_disadvantage_from: 3
    hp_max_halved_from: 4
    speed_zero_from: 5
  combat_actions:
    unarmed_strike: {ability: str, proficient: true, damage: "1+{str}", type: bludgeoning, reach: 5}
    grapple:
      resolution: contest
      attacker_skill: athletics
      defender_skills: [athletics, acrobatics]
      max_size_difference: 1
      condition: grappled
    shove:
      resolution: contest
      attacker_skill: athletics
      defender_skills: [athletics, acrobatics]
      max_size_difference: 1
      distance: 5
      prone_condition: prone
"#
        }
        "2024" => {
            r#"  exhaustion:
    condition: exhaustion
    max_level: 6
    death_level: 6
    long_rest_reduction: 1
    d20_penalty_per_level: 2
    speed_penalty_per_level: 5
  combat_actions:
    unarmed_strike: {ability: str, proficient: true, damage: "1d3+{str}", type: bludgeoning, reach: 5}
    grapple:
      resolution: save
      save_abilities: [str, dex]
      dc: "8+{str}+{prof}"
      max_size_difference: 1
      condition: grappled
    shove:
      resolution: save
      save_abilities: [str, dex]
      dc: "8+{str}+{prof}"
      max_size_difference: 1
      distance: 5
      prone_condition: prone
"#
        }
        other => panic!("unknown edition {other}"),
    }
}

fn conditions_block(edition: &str) -> &'static str {
    match edition {
        "2014" => {
            r#"  - id: charmed
  - id: frightened
    modifiers:
      - {applies: attack_roll, effect: disadvantage, when: {source_in_line_of_sight: true}}
      - {applies: check, effect: disadvantage, when: {source_in_line_of_sight: true}}
  - id: exhaustion
    modifiers:
      - {applies: check, effect: disadvantage, when: {exhaustion_level_min: 1}}
      - {applies: attack_roll, effect: disadvantage, when: {exhaustion_level_min: 3}}
      - {applies: save, effect: disadvantage, when: {exhaustion_level_min: 3}}
  - id: hidden
  - id: invisible
  - id: grappled
    flags: [speed_zero]
  - id: prone
    flags: [stand_up_half_speed]
  - id: incapacitated
    flags: [incapacitated]
  - id: unconscious
    implies: [incapacitated]
    flags: [speed_zero]
"#
        }
        "2024" => {
            r#"  - id: charmed
  - id: frightened
    modifiers:
      - {applies: attack_roll, effect: disadvantage, when: {source_in_line_of_sight: true}}
      - {applies: check, effect: disadvantage, when: {source_in_line_of_sight: true}}
  - id: exhaustion
    manual: "data-driven d20 and Speed penalty"
  - id: hidden
  - id: invisible
  - id: grappled
    flags: [speed_zero]
  - id: prone
    flags: [stand_up_half_speed]
  - id: incapacitated
    flags: [incapacitated]
  - id: unconscious
    implies: [incapacitated]
    flags: [speed_zero]
"#
        }
        other => panic!("unknown edition {other}"),
    }
}

fn content_yaml(edition: &str) -> String {
    let text = r##"rules:
  dying_conditions: [unconscious]
__RULES__
  condition_behaviors:
    charmed: {cannot_target_source: true, ends_when_source_dies: true}
    frightened: {cannot_move_closer_to_source: true, ends_when_source_dies: true}
    hidden: {hidden: true}
    invisible: {hidden: true}
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
    attacks:
      - {id: sword, name: Sword, kind: melee, ability: str, to_hit: 5, damage: [{dice: "1d8+3", type: slashing}], reach: 5}
    features:
      - id: bloodied-fury
        name: Bloodied Fury
        activation: passive
        modifiers:
          - {applies: attack_roll, effect: advantage, when: {bloodied: true}}
    actions:
      - id: dash
        name: Dash
        activation: action
        effects:
          - basic_action: dash
      - id: damaging-bolt
        name: Damaging Bolt
        activation: action
        target: {type: creature, side: any, count: 1}
        effects:
          - damage: {dice: "1d6", type: fire}
      - id: healing-word
        name: Healing Word
        activation: action
        target: {type: creature, side: any, count: 1}
        effects:
          - heal: {dice: "1d4"}
  - id: ally
    name: Ally
    kind: pc
    side: party
    ac: 13
    hp: 20
    speed: {walk: 30}
    prof: 2
    abilities: {str: 10, dex: 12, con: 12, int: 10, wis: 10, cha: 10}
    attacks:
      - {id: spear, name: Spear, kind: melee, ability: str, to_hit: 2, damage: [{dice: "1d6", type: piercing}], reach: 5}
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
  - id: grid-open
    name: Grid Open
    mode: grid
    zones:
      - {id: floor}
    grid:
      cell: 5
      legend: {".": {}, "#": {wall: true}}
      rows: ['########', '#......#', '#......#', '########']
      zones: {floor: "."}
  - id: grid-wall
    name: Grid Wall
    mode: grid
    zones:
      - {id: floor}
    grid:
      cell: 5
      legend: {".": {}, "#": {wall: true}}
      rows: ['########', '#..#...#', '#......#', '########']
      zones: {floor: "."}

encounters:
  - id: zones-fight
    scene: zones
    party_zone: alpha
    members:
      - {creature: source, zone: beta}
      - {creature: bystander, zone: alpha}
  - id: grid-open-fight
    scene: grid-open
    party_zone: floor
    members:
      - {creature: source, zone: floor, pos: [5, 1]}
      - {creature: bystander, zone: floor, pos: [6, 2]}
  - id: grid-wall-fight
    scene: grid-wall
    party_zone: floor
    members:
      - {creature: source, zone: floor, pos: [6, 1]}
      - {creature: bystander, zone: floor, pos: [2, 2]}
"##;
    // The creature id `hero` is built from the party list; the encounter rows
    // name it explicitly so the same fixture can build either a zones or grid
    // fight without a second hero definition.
    text.replace("__RULES__", rules_block(edition))
        .replace("__CONDITIONS__", conditions_block(edition))
}

fn campaign(edition: &str) -> Scratch {
    let scratch = Scratch::new(edition);
    scratch.write(
        "manifest.yaml",
        &format!("name: Combat Conditions {edition}\nruleset: \"{edition}\"\nlibrary: [library]\n"),
    );
    scratch.write("library/all.yaml", &content_yaml(edition));
    scratch
}

fn registry(edition: &str) -> (Scratch, ContentRegistry) {
    let scratch = campaign(edition);
    let registry = ContentSources {
        campaign: Some(scratch.manifest()),
        ..ContentSources::default()
    }
    .load()
    .expect("synthetic combat condition content should load");
    (scratch, registry)
}

fn encounter(registry: ContentRegistry, id: &str, party: &[&str]) -> EncounterState {
    EncounterState::from_encounter(
        registry,
        &NewEncounter {
            encounter: id.to_string(),
            party: party.iter().map(|name| name.to_string()).collect(),
            party_zone: None,
            adds: Vec::new(),
        },
    )
    .expect("synthetic encounter should build")
}

fn make_state(edition: &str, id: &str, party: &[&str]) -> EncounterState {
    let (_scratch, registry) = registry(edition);
    let mut state = encounter(registry, id, party);
    if id.starts_with("grid") {
        // Keep the protagonist at a known open cell regardless of how the
        // encounter builder places party members.
        if let Ok(hero) = state.find_index("Hero") {
            state.combatants[hero].set_cell(1, 1, "floor");
        }
    }
    state
}

fn index(state: &EncounterState, name: &str) -> usize {
    state.find_index(name).expect("combatant exists")
}

fn has_condition(state: &EncounterState, name: &str, condition: &str) -> bool {
    state.combatants[index(state, name)]
        .conditions
        .iter()
        .any(|active| active.condition.eq_ignore_ascii_case(condition))
}

fn active_condition(
    condition: &str,
    source: Option<&str>,
    linked: Option<&str>,
    concentration: bool,
) -> ActiveCondition {
    ActiveCondition {
        condition: condition.to_string(),
        source: source.map(str::to_string),
        linked: linked.map(str::to_string),
        duration: None,
        save_ends: None,
        ends_on: Vec::new(),
        created_by: None,
        concentration,
        applied_round: 0,
        notes: None,
    }
}

fn attack_action_started(state: &EncounterState, name: &str) -> bool {
    let combatant = &state.combatants[index(state, name)];
    combatant.action_used
        || combatant
            .once_used
            .keys()
            .any(|key| key.starts_with("turn:attack-action:"))
}

fn add(
    state: &mut EncounterState,
    who: &str,
    condition: &str,
    level: Option<u32>,
    source: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let mut log = Vec::new();
    engine::add_condition(
        state,
        who,
        condition,
        &AddConditionOptions {
            level,
            source: source.map(str::to_string),
            ..AddConditionOptions::default()
        },
        &mut ScriptedRng::new(&[]),
        &mut log,
    )?;
    Ok(log)
}

fn exhaustion(state: &mut EncounterState, who: &str, level: u32) -> Vec<String> {
    add(state, who, "exhaustion", Some(level), None).expect("exhaustion")
}

fn activate(state: &mut EncounterState, who: &str, ability: &str) -> Vec<String> {
    let mut log = Vec::new();
    engine::use_entry(
        state,
        who,
        ability,
        &[],
        &UseOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect("ability should resolve");
    log
}

fn dash_speed(state: &mut EncounterState, who: &str) -> i32 {
    let index = index(state, who);
    state.combatants[index].speed_left = 0;
    state.combatants[index].action_used = false;
    state.combatants[index].bonus_action_used = false;
    state.combatants[index].once_used.clear();
    activate(state, who, "dash");
    state.combatants[index].speed_left
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

// ---------------------------------------------------------------------------
// AC-1: Bloodied
// ---------------------------------------------------------------------------

#[test]
fn bloodied_is_half_or_less_and_tracks_damage_heal_and_temporary_hp() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        let hero = index(&state, "Hero");
        state.combatants[hero].hp = 21;
        assert!(!engine::is_bloodied(&state, hero), "{edition}: 21/40");
        state.combatants[hero].hp = 20;
        assert!(engine::is_bloodied(&state, hero), "{edition}: exactly half");

        let mut rng = ScriptedRng::new(&[]);
        let mut log = Vec::new();
        engine::damage(
            &mut state,
            "Hero",
            "1",
            Some("slashing"),
            false,
            &mut rng,
            &mut log,
        )
        .unwrap();
        assert_eq!(state.combatants[hero].hp, 19);
        assert!(engine::is_bloodied(&state, hero), "{edition}: after damage");

        engine::heal(
            &mut state,
            "Hero",
            "2",
            None,
            &mut ScriptedRng::new(&[]),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.combatants[hero].hp, 21);
        assert!(
            !engine::is_bloodied(&state, hero),
            "{edition}: healed above half"
        );

        // Temporary HP absorbs damage first, so crossing the threshold only
        // happens after the temporary pool is gone.
        state.combatants[hero].hp = 20;
        state.combatants[hero].temp_hp = 2;
        engine::damage(
            &mut state,
            "Hero",
            "1",
            Some("slashing"),
            false,
            &mut ScriptedRng::new(&[]),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.combatants[hero].hp, 20);
        assert!(engine::is_bloodied(&state, hero));
        engine::damage(
            &mut state,
            "Hero",
            "3",
            Some("slashing"),
            false,
            &mut ScriptedRng::new(&[]),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.combatants[hero].hp, 18);
        assert!(engine::is_bloodied(&state, hero));

        // 0 HP is still at or below half.
        state.combatants[hero].hp = 0;
        assert!(engine::is_bloodied(&state, hero), "{edition}: 0 HP");
    }
}

#[test]
fn bloodied_predicate_is_data_driven_for_features() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        let hero = index(&state, "Hero");
        state.combatants[hero].hp = 20;
        let mut log = Vec::new();
        engine::attack(
            &mut state,
            "Hero",
            "Bystander",
            "sword",
            &AttackOptions::default(),
            &mut ScriptedRng::new(&[15, 15, 4]),
            &mut log,
        )
        .unwrap();
        assert!(
            log.iter().any(|line| line.contains("Bloodied Fury")),
            "{edition}: {log:?}"
        );
        assert!(
            log.iter().any(|line| line.contains("adv:")),
            "{edition}: {log:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC-2: Exhaustion
// ---------------------------------------------------------------------------

#[test]
fn exhaustion_2014_six_levels_apply_the_edition_effects() {
    // Level 1: ability-check disadvantage.
    let mut state = make_state("2014", "zones-fight", &["hero", "ally"]);
    let log = exhaustion(&mut state, "Hero", 1);
    assert!(
        log.iter()
            .any(|line| line.contains("level 1") && line.contains("ability-check disadvantage")),
        "{log:?}"
    );
    let mut log = Vec::new();
    engine::check(
        &mut state,
        "Hero",
        "athletics",
        10,
        RollMode::Normal,
        0,
        &mut ScriptedRng::new(&[15, 9]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter()
            .any(|line| line.contains("dis:") && line.to_lowercase().contains("exhaustion")),
        "{log:?}"
    );

    // Level 2: Speed halved (30 -> 15).
    let mut state = make_state("2014", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 2);
    assert_eq!(dash_speed(&mut state, "Hero"), 15);

    // Level 3: attack and save disadvantage.
    let mut state = make_state("2014", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 3);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Hero",
        "Bystander",
        "sword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter()
            .any(|line| line.contains("dis:") && line.to_lowercase().contains("exhaustion")),
        "{log:?}"
    );
    let mut log = Vec::new();
    engine::save(
        &mut state,
        "Hero",
        "dex",
        10,
        RollMode::Normal,
        &mut ScriptedRng::new(&[15, 9]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter()
            .any(|line| line.contains("dis:") && line.to_lowercase().contains("exhaustion")),
        "{log:?}"
    );

    // Level 4: HP maximum halved (40 -> 20), including current HP.
    let mut state = make_state("2014", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 4);
    assert_eq!(state.combatants[index(&state, "Hero")].hp, 20);
    engine::heal(
        &mut state,
        "Hero",
        "99",
        None,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(state.combatants[index(&state, "Hero")].hp, 20);

    // Level 5: Speed 0.
    let mut state = make_state("2014", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 5);
    assert_eq!(dash_speed(&mut state, "Hero"), 0);

    // Level 6: death.
    let mut state = make_state("2014", "zones-fight", &["hero", "ally"]);
    let log = exhaustion(&mut state, "Hero", 6);
    assert!(state.combatants[index(&state, "Hero")].dead);
    assert_eq!(state.combatants[index(&state, "Hero")].hp, 0);
    assert!(log.iter().any(|line| line.contains("dies")), "{log:?}");
}

#[test]
fn exhaustion_2024_applies_minus_two_and_speed_penalty_per_level() {
    // Level 1: -2 on d20 tests and -5 ft Speed.
    let mut state = make_state("2024", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 1);
    let mut log = Vec::new();
    engine::check(
        &mut state,
        "Hero",
        "athletics",
        10,
        RollMode::Normal,
        0,
        &mut ScriptedRng::new(&[10]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("-2[exhaustion]")),
        "{log:?}"
    );
    assert_eq!(dash_speed(&mut state, "Hero"), 25);

    // Every level subtracts 5 ft, down to 5 ft at level 5.
    for level in 2..=5 {
        let mut state = make_state("2024", "zones-fight", &["hero", "ally"]);
        exhaustion(&mut state, "Hero", level);
        assert_eq!(
            dash_speed(&mut state, "Hero"),
            30 - 5 * level as i32,
            "level {level}"
        );
    }

    // Level 3: -6 and Speed 30 - 15 = 15.
    let mut state = make_state("2024", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 3);
    let mut log = Vec::new();
    engine::save(
        &mut state,
        "Hero",
        "dex",
        10,
        RollMode::Normal,
        &mut ScriptedRng::new(&[15]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter().any(|line| line.contains("-6[exhaustion]")),
        "{log:?}"
    );
    assert_eq!(dash_speed(&mut state, "Hero"), 15);

    // Death saves are saving throws and take the same penalty.
    let mut state = make_state("2024", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 3);
    let hero = index(&state, "Hero");
    state.combatants[hero].hp = 0;
    let mut log = Vec::new();
    engine::death_save(&mut state, "Hero", &mut ScriptedRng::new(&[15]), &mut log).unwrap();
    assert!(
        log.iter().any(|line| line.contains("-6[exhaustion]")),
        "{log:?}"
    );
    assert_eq!(
        state.combatants[index(&state, "Hero")].death_saves.failures,
        1
    );

    // Level 6: death.
    let mut state = make_state("2024", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 6);
    assert!(state.combatants[index(&state, "Hero")].dead);
    assert_eq!(state.combatants[index(&state, "Hero")].hp, 0);
}

#[test]
fn exhaustion_rejects_zero_and_out_of_range_levels_without_changing_state() {
    let mut state = make_state("2024", "zones-fight", &["hero", "ally"]);
    for level in [0, 7] {
        let error = add(&mut state, "Hero", "exhaustion", Some(level), None)
            .expect_err("invalid exhaustion level");
        assert!(error.to_string().contains("level"), "{error}");
        assert_eq!(state.combatants[index(&state, "Hero")].exhaustion_level, 0);
        assert!(!has_condition(&state, "Hero", "exhaustion"));
    }
}

#[test]
fn long_rest_reduces_exhaustion_and_restores_the_effective_maximum() {
    // 2014: level 4 halves the maximum; a long rest lowers to 3 and restores it.
    let mut state = make_state("2014", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 4);
    assert_eq!(state.combatants[index(&state, "Hero")].hp, 20);
    let mut log = Vec::new();
    engine::rest(
        &mut state,
        RestKind::Long,
        Some("Hero"),
        None,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .unwrap();
    assert_eq!(state.combatants[index(&state, "Hero")].exhaustion_level, 3);
    assert_eq!(state.combatants[index(&state, "Hero")].hp, 40);
    assert!(
        log.iter().any(|line| line.contains("exhaustion")),
        "{log:?}"
    );

    // 2024: level 3 -> 2, and level 1 -> 0 removes the marker.
    let mut state = make_state("2024", "zones-fight", &["hero", "ally"]);
    exhaustion(&mut state, "Hero", 3);
    engine::rest(
        &mut state,
        RestKind::Long,
        Some("Hero"),
        None,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(state.combatants[index(&state, "Hero")].exhaustion_level, 2);
    exhaustion(&mut state, "Hero", 1);
    engine::rest(
        &mut state,
        RestKind::Long,
        Some("Hero"),
        None,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(state.combatants[index(&state, "Hero")].exhaustion_level, 0);
    assert!(!has_condition(&state, "Hero", "exhaustion"));
}

#[test]
fn condition_add_exhaustion_without_level_keeps_the_legacy_manual_marker() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        let log = add(&mut state, "Hero", "exhaustion", None, None).unwrap();
        assert!(has_condition(&state, "Hero", "exhaustion"));
        assert_eq!(state.combatants[index(&state, "Hero")].exhaustion_level, 0);
        assert_eq!(dash_speed(&mut state, "Hero"), 30);
        assert!(!log.iter().any(|line| line.contains("level 1")), "{log:?}");

        let log = exhaustion(&mut state, "Hero", 3);
        assert_eq!(state.combatants[index(&state, "Hero")].exhaustion_level, 3);
        assert!(log.iter().any(|line| line.contains("level 3")), "{log:?}");
        assert_eq!(dash_speed(&mut state, "Hero"), 15);
    }
}

#[test]
fn exhaustion_death_runs_the_normal_death_cleanup_once() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        add(&mut state, "Ally", "charmed", None, Some("Source")).unwrap();
        add(&mut state, "Ally", "frightened", None, Some("Source")).unwrap();
        let hero = index(&state, "Hero");
        state.combatants[hero].conditions.push(active_condition(
            "blessed",
            Some("Source"),
            None,
            true,
        ));
        state.concentration.insert(
            "Source".to_string(),
            ConcentrationState {
                ability: Some("test-spell".to_string()),
                round: 1,
            },
        );

        let mut log = exhaustion(&mut state, "Source", 6);
        assert!(state.combatants[index(&state, "Source")].is_dead());
        assert!(!has_condition(&state, "Ally", "charmed"), "{edition}");
        assert!(!has_condition(&state, "Ally", "frightened"), "{edition}");
        assert!(
            !state.combatants[index(&state, "Hero")]
                .conditions
                .iter()
                .any(|condition| condition.condition == "blessed"),
            "{edition}"
        );
        assert!(!state.concentration.contains_key("Source"), "{edition}");
        assert_eq!(
            log.iter()
                .filter(|line| line.contains("dies from exhaustion"))
                .count(),
            1,
            "{edition}: {log:?}"
        );

        // Repeated level 6 must not run the death cleanup or the death log again.
        let mut second = exhaustion(&mut state, "Source", 6);
        log.append(&mut second);
        assert_eq!(
            log.iter()
                .filter(|line| line.contains("dies from exhaustion"))
                .count(),
            1,
            "{edition}: {log:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC-3: Charmed
// ---------------------------------------------------------------------------

#[test]
fn charmed_records_its_source_and_blocks_attacks_and_damaging_abilities() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        let log = add(&mut state, "Hero", "charmed", None, Some("Source")).unwrap();
        let active = state.combatants[index(&state, "Hero")]
            .conditions
            .iter()
            .find(|active| active.condition == "charmed")
            .expect("charmed");
        assert_eq!(active.source.as_deref(), Some("Source"));
        assert!(
            log.iter().any(|line| line.contains("source: Source")),
            "{edition}: {log:?}"
        );
        assert!(
            log.iter()
                .any(|line| line.contains("cannot attack or damage the source")),
            "{edition}: {log:?}"
        );

        let error = engine::attack(
            &mut state,
            "Hero",
            "Source",
            "sword",
            &AttackOptions::default(),
            &mut ScriptedRng::new(&[]),
            &mut Vec::new(),
        )
        .expect_err("charmed attack");
        assert!(error.to_string().contains("charmed"), "{error}");
        assert!(!state.combatants[index(&state, "Hero")].action_used);

        // A different target is allowed.
        engine::attack(
            &mut state,
            "Hero",
            "Bystander",
            "sword",
            &AttackOptions::default(),
            &mut ScriptedRng::new(&[15, 4]),
            &mut Vec::new(),
        )
        .unwrap();

        // A damaging ability cannot target the charmer, but a healing one can.
        let hero = index(&state, "Hero");
        state.combatants[hero].action_used = false;
        let error = engine::use_entry(
            &mut state,
            "Hero",
            "damaging-bolt",
            &[],
            &UseOptions {
                targets: vec!["Source".to_string()],
                ..UseOptions::default()
            },
            &mut ScriptedRng::new(&[]),
            &mut Vec::new(),
        )
        .expect_err("charmed damaging ability");
        assert!(error.to_string().contains("charmed"), "{error}");
        assert!(!state.combatants[index(&state, "Hero")].action_used);
        engine::use_entry(
            &mut state,
            "Hero",
            "healing-word",
            &[],
            &UseOptions {
                targets: vec!["Source".to_string()],
                ..UseOptions::default()
            },
            &mut ScriptedRng::new(&[2]),
            &mut Vec::new(),
        )
        .unwrap();
    }
}

#[test]
fn charmed_blocks_grapple_and_shove_against_the_source() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        add(&mut state, "Hero", "charmed", None, Some("Source")).unwrap();

        for kind in ["grapple", "shove"] {
            let mut log = Vec::new();
            let error = if kind == "grapple" {
                engine::grapple(
                    &mut state,
                    "Hero",
                    "Source",
                    &GrappleOptions::default(),
                    &mut ScriptedRng::new(&[]),
                    &mut log,
                )
            } else {
                engine::shove(
                    &mut state,
                    "Hero",
                    "Source",
                    &ShoveOptions::default(),
                    &mut ScriptedRng::new(&[]),
                    &mut log,
                )
            }
            .expect_err("charmed grapple/shove");
            assert!(
                error.to_string().contains("charmed"),
                "{edition} {kind}: {error}"
            );
            assert!(
                !attack_action_started(&state, "Hero"),
                "{edition} {kind}: refusal must not spend an attack slot"
            );
        }

        // A different target is still allowed.
        let mut rng = if edition == "2014" {
            ScriptedRng::new(&[15, 5])
        } else {
            ScriptedRng::new(&[2])
        };
        engine::grapple(
            &mut state,
            "Hero",
            "Bystander",
            &GrappleOptions::default(),
            &mut rng,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(attack_action_started(&state, "Hero"));

        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        add(&mut state, "Hero", "charmed", None, Some("Source")).unwrap();
        let mut rng = if edition == "2014" {
            ScriptedRng::new(&[15, 5])
        } else {
            ScriptedRng::new(&[2])
        };
        engine::shove(
            &mut state,
            "Hero",
            "Bystander",
            &ShoveOptions::default(),
            &mut rng,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(attack_action_started(&state, "Hero"));
    }
}

#[test]
fn source_death_cleanup_uses_the_linked_fallback() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        let hero = index(&state, "Hero");
        state.combatants[hero].conditions.push(active_condition(
            "charmed",
            None,
            Some("Source"),
            false,
        ));
        state.combatants[hero].conditions.push(active_condition(
            "frightened",
            None,
            Some("Source"),
            false,
        ));

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
        assert!(!has_condition(&state, "Hero", "charmed"), "{edition}");
        assert!(!has_condition(&state, "Hero", "frightened"), "{edition}");
        assert!(
            log.iter()
                .filter(|line| line.contains("source died"))
                .count()
                >= 2,
            "{edition}: {log:?}"
        );
    }
}

#[test]
fn behavior_conditions_reject_unknown_source_or_linked() {
    for edition in ["2014", "2024"] {
        for condition in ["charmed", "frightened"] {
            let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
            let error = add(&mut state, "Hero", condition, None, Some("Nobody"))
                .expect_err("unknown source");
            assert!(
                error.to_string().contains("unknown"),
                "{edition} {condition}: {error}"
            );
            assert!(!has_condition(&state, "Hero", condition));
            assert!(!attack_action_started(&state, "Hero"));

            let error = engine::add_condition(
                &mut state,
                "Hero",
                condition,
                &AddConditionOptions {
                    linked: Some("Nobody".to_string()),
                    ..AddConditionOptions::default()
                },
                &mut ScriptedRng::new(&[]),
                &mut Vec::new(),
            )
            .expect_err("unknown linked");
            assert!(
                error.to_string().contains("unknown"),
                "{edition} {condition} linked: {error}"
            );
            assert!(!has_condition(&state, "Hero", condition));
        }
    }
}

#[test]
fn charmed_ends_when_removed_or_when_its_source_dies() {
    for edition in ["2014", "2024"] {
        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        add(&mut state, "Hero", "charmed", None, Some("Source")).unwrap();
        engine::remove_condition(&mut state, "Hero", "charmed", &mut Vec::new()).unwrap();
        assert!(!has_condition(&state, "Hero", "charmed"));
        engine::attack(
            &mut state,
            "Hero",
            "Source",
            "sword",
            &AttackOptions::default(),
            &mut ScriptedRng::new(&[15, 4]),
            &mut Vec::new(),
        )
        .unwrap();

        let mut state = make_state(edition, "zones-fight", &["hero", "ally"]);
        add(&mut state, "Hero", "charmed", None, Some("Source")).unwrap();
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
        assert!(state.combatants[index(&state, "Source")].is_dead());
        assert!(!has_condition(&state, "Hero", "charmed"));
        assert!(
            log.iter()
                .any(|line| line.contains("no longer charmed") && line.contains("source")),
            "{edition}: {log:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC-4: Frightened
// ---------------------------------------------------------------------------

#[test]
fn frightened_disadvantage_requires_the_source_to_be_in_line_of_sight() {
    let mut state = make_state("2014", "zones-fight", &["hero", "ally"]);
    add(&mut state, "Hero", "frightened", None, Some("Source")).unwrap();
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Hero",
        "Bystander",
        "sword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter()
            .any(|line| line.contains("dis:") && line.to_lowercase().contains("frightened")),
        "{log:?}"
    );

    // A hidden source lifts the disadvantage.
    let source = index(&state, "Source");
    state.combatants[source]
        .conditions
        .push(combat_sim::state::ActiveCondition {
            condition: "hidden".to_string(),
            source: None,
            linked: None,
            duration: None,
            save_ends: None,
            ends_on: Vec::new(),
            created_by: None,
            concentration: false,
            applied_round: 0,
            notes: None,
        });
    let hero = index(&state, "Hero");
    state.combatants[hero].action_used = false;
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Hero",
        "Bystander",
        "sword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut log,
    )
    .unwrap();
    assert!(!log.iter().any(|line| line.contains("dis:")), "{log:?}");
}

#[test]
fn frightened_source_behind_a_wall_does_not_impose_disadvantage() {
    let mut state = make_state("2014", "grid-wall-fight", &["hero"]);
    add(&mut state, "Hero", "frightened", None, Some("Source")).unwrap();
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Hero",
        "Bystander",
        "sword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut log,
    )
    .unwrap();
    assert!(!log.iter().any(|line| line.contains("dis:")), "{log:?}");
}

#[test]
fn frightened_blocks_willing_movement_closer_to_a_visible_source() {
    let mut state = make_state("2014", "grid-open-fight", &["hero"]);
    add(&mut state, "Hero", "frightened", None, Some("Source")).unwrap();
    let error = engine::move_to(
        &mut state,
        "Hero",
        &MoveTarget::Cell(2, 1),
        true,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("move closer");
    assert!(error.to_string().contains("closer"), "{error}");
    assert!(
        error.to_string().contains("source of fear"),
        "movement refusal should name the condition: {error}"
    );

    // Moving sideways or away is still allowed.
    engine::move_to(
        &mut state,
        "Hero",
        &MoveTarget::Cell(1, 2),
        true,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
}

#[test]
fn frightened_source_hidden_dead_or_out_of_sight_lifts_the_movement_restriction() {
    // Hidden source.
    let mut state = make_state("2014", "grid-open-fight", &["hero"]);
    add(&mut state, "Hero", "frightened", None, Some("Source")).unwrap();
    let source = index(&state, "Source");
    state.combatants[source]
        .conditions
        .push(combat_sim::state::ActiveCondition {
            condition: "hidden".to_string(),
            source: None,
            linked: None,
            duration: None,
            save_ends: None,
            ends_on: Vec::new(),
            created_by: None,
            concentration: false,
            applied_round: 0,
            notes: None,
        });
    engine::move_to(
        &mut state,
        "Hero",
        &MoveTarget::Cell(2, 1),
        true,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    // Dead source.
    let mut state = make_state("2014", "grid-open-fight", &["hero"]);
    add(&mut state, "Hero", "frightened", None, Some("Source")).unwrap();
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
    engine::move_to(
        &mut state,
        "Hero",
        &MoveTarget::Cell(2, 1),
        true,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();

    // Source behind a wall (out of sight).
    let mut state = make_state("2014", "grid-wall-fight", &["hero"]);
    add(&mut state, "Hero", "frightened", None, Some("Source")).unwrap();
    engine::move_to(
        &mut state,
        "Hero",
        &MoveTarget::Cell(2, 1),
        true,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
}

#[test]
fn frightened_uses_the_spatial_movement_path_when_poses_are_explicit() {
    let mut state = make_state("2024", "grid-open-fight", &["hero"]);
    for combatant in &mut state.combatants {
        combatant.pose =
            SpatialPose::from_position(&combatant.position(), combatant.creature.size());
    }
    add(&mut state, "Hero", "frightened", None, Some("Source")).unwrap();
    let error = engine::move_to(
        &mut state,
        "Hero",
        &MoveTarget::Cell(2, 1),
        true,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("spatial move closer");
    assert!(error.to_string().contains("closer"), "{error}");
}

#[test]
fn frightened_zone_movement_closer_is_refused_until_the_source_is_gone() {
    let mut state = make_state("2014", "zones-fight", &["hero", "ally"]);
    add(&mut state, "Hero", "frightened", None, Some("Source")).unwrap();
    let error = engine::move_to(
        &mut state,
        "Hero",
        &MoveTarget::Zone("beta".to_string()),
        true,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("zone move closer");
    assert!(error.to_string().contains("closer"), "{error}");

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
    engine::move_to(
        &mut state,
        "Hero",
        &MoveTarget::Zone("beta".to_string()),
        true,
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// AC-5 and AC-6: compatibility, CLI and logs
// ---------------------------------------------------------------------------

#[test]
fn old_combat_state_loads_with_no_exhaustion_and_schema_one() {
    assert_eq!(STATE_SCHEMA_VERSION, 1);
    let state = make_state("2024", "zones-fight", &["hero", "ally"]);
    let path = std::env::temp_dir().join(format!(
        "combat-sim-conditions-old-{}-{}.state.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    state.save(&path).unwrap();
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for combatant in value["combatants"].as_array_mut().unwrap() {
        combatant
            .as_object_mut()
            .unwrap()
            .remove("exhaustion_level");
    }
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let mut loaded = EncounterState::load(&path).unwrap();
    for combatant in &loaded.combatants {
        assert_eq!(combatant.exhaustion_level, 0);
    }
    assert_eq!(dash_speed(&mut loaded, "Hero"), 30);
    let _ = fs::remove_file(path);
}

#[test]
fn condition_cli_adds_levels_and_sources_and_keeps_the_old_form_working() {
    let state = make_state("2024", "zones-fight", &["hero", "ally"]);
    let path = std::env::temp_dir().join(format!(
        "combat-sim-conditions-cli-{}-{}.state.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    state.save(&path).unwrap();

    // The old form without --level keeps the manual-marker contract.
    let output = run_cli(&path, &["condition", "add", "Hero", "exhaustion"]);
    assert_success(&output, "legacy condition add exhaustion");
    let status = status_json(&path);
    let exhaustion = combatant(&status, "Hero")["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|condition| condition["id"] == "exhaustion")
        .expect("exhaustion marker");
    assert_eq!(exhaustion["level"], 0);

    let output = run_cli(
        &path,
        &["condition", "add", "Hero", "exhaustion", "--level", "3"],
    );
    assert_success(&output, "condition add exhaustion --level");
    assert!(stdout(&output).contains("level 3"), "{}", stdout(&output));
    let status = status_json(&path);
    let hero = combatant(&status, "Hero");
    let exhaustion = hero["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|condition| condition["id"] == "exhaustion")
        .expect("exhaustion condition");
    assert_eq!(exhaustion["level"], 3);

    let output = run_cli(&path, &["condition", "remove", "Hero", "exhaustion"]);
    assert_success(&output, "condition remove exhaustion");
    assert!(
        stdout(&output).contains("no longer exhaustion")
            && stdout(&output).contains("removed by command"),
        "{}",
        stdout(&output)
    );
    let status = status_json(&path);
    assert!(!combatant(&status, "Hero")["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|condition| condition["id"] == "exhaustion"));

    // The old form (no new flags) still works.
    let output = run_cli(&path, &["condition", "add", "Hero", "hidden"]);
    assert_success(&output, "legacy condition add");
    assert!(has_condition(
        &EncounterState::load(&path).unwrap(),
        "Hero",
        "hidden"
    ));

    let output = run_cli(
        &path,
        &["condition", "add", "Hero", "charmed", "--source", "Source"],
    );
    assert_success(&output, "condition add charmed --source");
    let status = status_json(&path);
    let charmed = combatant(&status, "Hero")["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|condition| condition["id"] == "charmed")
        .expect("charmed condition");
    assert_eq!(charmed["source"], "Source");

    let output = run_cli(
        &path,
        &["condition", "add", "Hero", "exhaustion", "--level", "7"],
    );
    assert!(!output.status.success());
    assert!(stderr(&output).contains("level"), "{}", stderr(&output));

    let _ = fs::remove_file(path);
}

#[test]
fn real_content_check_passes_for_both_editions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for manifest in ["real-2014.yaml", "real-2024.yaml"] {
        let output = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
            .current_dir(root)
            .arg("--campaign")
            .arg(root.join("tests/fixtures/rulesets").join(manifest))
            .args(["content", "check"])
            .output()
            .expect("run content check");
        assert_success(&output, manifest);
    }
}

#[test]
fn content_check_reports_bad_exhaustion_data() {
    let scratch = Scratch::new("bad-exhaustion");
    scratch.write(
        "manifest.yaml",
        "name: Bad Exhaustion\nruleset: \"2024\"\nlibrary: [library]\n",
    );
    scratch.write(
        "library/all.yaml",
        r#"rules:
  exhaustion:
    condition: exhaustion
    max_level: 6
    death_level: 9
    speed_zero_from: 7
conditions:
  - id: exhaustion
"#,
    );
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
    assert!(messages.contains("death_level"), "{messages}");
    assert!(messages.contains("speed_zero_from"), "{messages}");
}
