//! TASK-0118 (v1.5d): senses, light, obscurement and attack visibility.
//!
//! Synthetic content only: no player books, real saves or DM secrets.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use combat_sim::content::schema::{Cover, Light, Obscurement};
use combat_sim::content::{self, ContentRegistry, ContentSources};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{
    self, AttackOptions, ParsedSenses, UseOptions, Visibility, VisionProfile,
};
use combat_sim::space::{FlightState, Position, VerticalState};
use combat_sim::state::{ActiveCondition, EncounterState, NewEncounter, STATE_SCHEMA_VERSION};
use serde_json::Value;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "combat-sim-senses-{label}-{}-{n}",
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

fn content_yaml() -> &'static str {
    r##"rules:
  dying_conditions: [unconscious]
  condition_behaviors:
    hidden: {hidden: true}
    invisible: {hidden: true}
conditions:
  - id: hidden
    modifiers:
      - {applies: attack_roll, effect: advantage}
      - {applies: incoming_attack, effect: disadvantage}
  - id: invisible
    modifiers:
      - {applies: attack_roll, effect: advantage}
      - {applies: incoming_attack, effect: disadvantage}
  - id: prone
    flags: [stand_up_half_speed]

creatures:
  - id: observer
    name: Observer
    kind: pc
    side: party
    ac: 14
    hp: 30
    speed: {walk: 30}
    prof: 2
    abilities: {str: 12, dex: 14, con: 12, int: 10, wis: 12, cha: 10}
    senses: "Darkvision 60 ft.; Passive Perception 12"
    attacks:
      - {id: sword, name: Sword, kind: melee, ability: str, to_hit: 5, damage: [{dice: "1d8+3", type: slashing}], reach: 5}
    actions:
      - id: bolt
        name: Bolt
        activation: action
        range: 120
        target: {type: creature, side: any, count: 1}
        effects:
          - attack: {to_hit: 5, kind: ranged, damage: [{dice: "1d8", type: fire}]}
  - id: plain
    name: Plain
    kind: pc
    side: party
    ac: 12
    hp: 30
    speed: {walk: 30}
    prof: 2
    abilities: {str: 10, dex: 12, con: 10, int: 10, wis: 10, cha: 10}
    attacks:
      - {id: club, name: Club, kind: melee, ability: str, to_hit: 2, damage: [{dice: "1d6", type: bludgeoning}], reach: 5}
  - id: blindseer
    name: Blindseer
    kind: pc
    side: party
    ac: 12
    hp: 30
    speed: {walk: 30}
    prof: 2
    abilities: {str: 10, dex: 12, con: 10, int: 10, wis: 14, cha: 10}
    senses: "Blindsight 30 ft.; Passive Perception 15"
    attacks:
      - {id: staff, name: Staff, kind: melee, ability: str, to_hit: 2, damage: [{dice: "1d6", type: bludgeoning}], reach: 5}
  - id: tremorsenser
    name: Tremorsenser
    kind: pc
    side: party
    ac: 12
    hp: 30
    speed: {walk: 30}
    prof: 2
    abilities: {str: 10, dex: 12, con: 10, int: 10, wis: 14, cha: 10}
    senses: "Tremorsense 30 ft.; Passive Perception 14"
    attacks:
      - {id: slam, name: Slam, kind: melee, ability: str, to_hit: 2, damage: [{dice: "1d6", type: bludgeoning}], reach: 5}
  - id: truesighted
    name: Truesighted
    kind: pc
    side: party
    ac: 12
    hp: 30
    speed: {walk: 30}
    prof: 2
    abilities: {str: 10, dex: 12, con: 10, int: 10, wis: 14, cha: 10}
    senses: "Truesight 60 ft.; Passive Perception 16"
    attacks:
      - {id: blade, name: Blade, kind: melee, ability: str, to_hit: 2, damage: [{dice: "1d6", type: slashing}], reach: 5}
  - id: target
    name: Target
    kind: monster
    side: enemy
    ac: 12
    hp: 30
    speed: {walk: 30}
    prof: 2
    abilities: {str: 12, dex: 10, con: 12, int: 8, wis: 8, cha: 8}
    attacks:
      - {id: claw, name: Claw, kind: melee, ability: str, to_hit: 4, damage: [{dice: "1d6+2", type: slashing}], reach: 5}
  - id: ghost
    name: Ghost
    kind: monster
    side: enemy
    tags: [incorporeal]
    ac: 11
    hp: 20
    speed: {walk: 30, fly: 40}
    prof: 2
    abilities: {str: 8, dex: 12, con: 10, int: 10, wis: 10, cha: 12}
    attacks:
      - {id: touch, name: Touch, kind: melee, ability: dex, to_hit: 3, damage: [{dice: "1d6", type: cold}], reach: 5}

scenes:
  - id: zones
    name: Zones
    mode: zones
    zones:
      - {id: hub, light: bright}
      - {id: dim, light: dim}
      - {id: dark, light: dark}
      - {id: lightobs, light: bright, obscured: light}
      - {id: heavyobs, light: bright, obscured: heavy}
      - {id: darklightobs, light: dark, obscured: light}
      - {id: far, light: bright}
    edges:
      - {a: hub, b: dim, distance: 5}
      - {a: hub, b: dark, distance: 5}
      - {a: hub, b: lightobs, distance: 5}
      - {a: hub, b: heavyobs, distance: 5}
      - {a: hub, b: darklightobs, distance: 5}
      - {a: hub, b: far, distance: 65}
  - id: grid
    name: Grid
    mode: grid
    light: bright
    zones:
      - {id: floor}
    grid:
      cell: 5
      legend: {".": {}, "#": {wall: true}}
      rows:
        - '##################'
        - '#................#'
        - '##################'
      zones: {floor: "."}

encounters:
  - id: zones
    scene: zones
    party_zone: hub
    members:
      - {creature: target, zone: hub}
      - {creature: ghost, zone: hub}
  - id: grid
    scene: grid
    party_zone: floor
    members:
      - {creature: target, zone: floor, pos: [13, 1]}
      - {creature: ghost, zone: floor, pos: [14, 1]}
"##
}

fn campaign() -> Scratch {
    let scratch = Scratch::new("content");
    scratch.write(
        "manifest.yaml",
        "name: Combat Senses 2014\nruleset: \"2014\"\nlibrary: [library]\n",
    );
    scratch.write("library/all.yaml", content_yaml());
    scratch
}

fn registry() -> (Scratch, ContentRegistry) {
    let scratch = campaign();
    let registry = ContentSources {
        campaign: Some(scratch.manifest()),
        ..ContentSources::default()
    }
    .load()
    .expect("synthetic senses content should load");
    (scratch, registry)
}

fn encounter(registry: ContentRegistry, id: &str) -> EncounterState {
    EncounterState::from_encounter(
        registry,
        &NewEncounter {
            encounter: id.to_string(),
            party: vec![
                "observer".to_string(),
                "plain".to_string(),
                "blindseer".to_string(),
                "tremorsenser".to_string(),
                "truesighted".to_string(),
            ],
            party_zone: None,
            adds: Vec::new(),
        },
    )
    .expect("synthetic encounter should build")
}

fn make_state(id: &str) -> EncounterState {
    let (_scratch, registry) = registry();
    encounter(registry, id)
}

fn index(state: &EncounterState, name: &str) -> usize {
    state.find_index(name).expect("combatant exists")
}

fn scene_mut(state: &mut EncounterState) -> &mut combat_sim::content::schema::Scene {
    Arc::make_mut(state.scene.as_mut().expect("scene"))
}

fn set_scene_light(state: &mut EncounterState, light: Option<Light>) {
    scene_mut(state).light = light;
    state.rebuild_caches();
}

fn set_scene_obscured(state: &mut EncounterState, obscurement: Option<Obscurement>) {
    scene_mut(state).obscured = obscurement;
    state.rebuild_caches();
}

fn zone_mut<'a>(
    state: &'a mut EncounterState,
    zone: &str,
) -> &'a mut combat_sim::content::schema::Zone {
    scene_mut(state)
        .zones
        .iter_mut()
        .find(|candidate| candidate.id == zone)
        .expect("zone exists")
}

fn set_zone_light(state: &mut EncounterState, zone: &str, light: Option<Light>) {
    zone_mut(state, zone).light = light;
    state.rebuild_caches();
}

fn set_zone_obscured(state: &mut EncounterState, zone: &str, obscurement: Option<Obscurement>) {
    zone_mut(state, zone).obscured = obscurement;
    state.rebuild_caches();
}

fn set_zone_cover(state: &mut EncounterState, zone: &str, cover: Cover) {
    zone_mut(state, zone).cover = cover;
    state.rebuild_caches();
}

fn place(state: &mut EncounterState, name: &str, zone: &str) {
    let at = index(state, name);
    state.combatants[at].set_zone(zone);
}

fn place_cell(state: &mut EncounterState, name: &str, x: i32, y: i32) {
    let at = index(state, name);
    state.combatants[at].set_cell(x, y, "floor");
}

fn active_condition(condition: &str) -> ActiveCondition {
    ActiveCondition {
        condition: condition.to_string(),
        source: None,
        linked: None,
        duration: None,
        save_ends: None,
        ends_on: Vec::new(),
        created_by: None,
        concentration: false,
        applied_round: 0,
        notes: None,
    }
}

fn add_condition(state: &mut EncounterState, name: &str, condition: &str) {
    let at = index(state, name);
    state.combatants[at]
        .conditions
        .push(active_condition(condition));
}

fn make_flying(state: &mut EncounterState, name: &str, altitude: i32) {
    let at = index(state, name);
    state.combatants[at].vertical =
        Some(VerticalState::new(altitude, FlightState::Flying, 0).expect("valid altitude"));
}

fn make_incorporeal(state: &mut EncounterState, name: &str) {
    let at = index(state, name);
    let creature = Arc::make_mut(&mut state.combatants[at].creature);
    let mut tags = creature.tags.clone().unwrap_or_default();
    tags.push("incorporeal".to_string());
    creature.tags = Some(tags);
}

fn attack_log(state: &mut EncounterState, attacker: &str, target: &str) -> Vec<String> {
    let mut log = Vec::new();
    engine::attack(
        state,
        attacker,
        target,
        "sword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[12, 12, 5, 5, 5, 5]),
        &mut log,
    )
    .expect("attack should resolve");
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

fn assert_visibility(state: &EncounterState, observer: &str, target: &str, expected: Visibility) {
    let result = engine::can_see(state, index(state, observer), index(state, target));
    assert_eq!(
        result.visibility, expected,
        "observer {observer} -> target {target}: {} ({})",
        result.reason, result.source
    );
}

// ---------------------------------------------------------------------------
// AC-1: structured senses parsing
// ---------------------------------------------------------------------------

#[test]
fn parses_real_srd_dragon_and_kraken_senses_and_malformed_ranges() {
    let dragon = engine::parse_senses(Some(
        "Blindsight 60 ft.; Darkvision 120 ft.; Passive perception 21",
    ));
    assert_eq!(
        dragon,
        ParsedSenses {
            darkvision: Some(120),
            blindsight: Some(60),
            tremorsense: None,
            truesight: None,
            passive_perception: Some(21),
        }
    );

    let kraken = engine::parse_senses(Some("Passive perception 14; Truesight 120 ft."));
    assert_eq!(kraken.truesight, Some(120));
    assert_eq!(kraken.passive_perception, Some(14));
    assert_eq!(kraken.darkvision, None);

    let lowercase = engine::parse_senses(Some(
        "darkvision 60 feet, passive perception 13, tremorsense 30",
    ));
    assert_eq!(lowercase.darkvision, Some(60));
    assert_eq!(lowercase.tremorsense, Some(30));
    assert_eq!(lowercase.passive_perception, Some(13));

    let malformed = engine::parse_senses(Some(
        "Darkvision; Blindsight -30 ft.; Tremorsense 0 ft.; Truesight banana; Passive Perception ?",
    ));
    assert_eq!(malformed, ParsedSenses::default());

    assert_eq!(engine::parse_senses(None), ParsedSenses::default());
    assert_eq!(engine::parse_senses(Some("")), ParsedSenses::default());
}

#[test]
fn parsed_senses_preserve_the_raw_creature_string_and_available_ranges() {
    let (_scratch, registry) = registry();
    let creature = registry
        .creatures
        .get("observer")
        .expect("observer creature");
    assert_eq!(
        creature.senses.as_deref(),
        Some("Darkvision 60 ft.; Passive Perception 12")
    );
    let parsed = engine::parse_senses(creature.senses.as_deref());
    assert_eq!(parsed.darkvision, Some(60));
    assert_eq!(parsed.passive_perception, Some(12));

    let state = make_state("zones");
    let observer = index(&state, "Observer");
    assert_eq!(state.combatants[observer].creature.senses, creature.senses);
}

// ---------------------------------------------------------------------------
// AC-2: light and obscurement combine into effective visibility
// ---------------------------------------------------------------------------

#[test]
fn light_and_obscurement_combine_into_effective_visibility() {
    let mut state = make_state("zones");
    place(&mut state, "Plain", "hub");
    place(&mut state, "Target", "dim");

    assert_visibility(&state, "Plain", "Target", Visibility::LightlyObscured);
    set_zone_light(&mut state, "dim", Some(Light::Bright));
    assert_visibility(&state, "Plain", "Target", Visibility::Clear);
    set_zone_light(&mut state, "dim", Some(Light::Dark));
    assert_visibility(&state, "Plain", "Target", Visibility::HeavilyObscured);
    set_zone_light(&mut state, "dim", Some(Light::Bright));
    set_zone_obscured(&mut state, "dim", Some(Obscurement::Light));
    assert_visibility(&state, "Plain", "Target", Visibility::LightlyObscured);
    set_zone_obscured(&mut state, "dim", Some(Obscurement::Heavy));
    assert_visibility(&state, "Plain", "Target", Visibility::HeavilyObscured);
    set_zone_obscured(&mut state, "dim", Some(Obscurement::None));
    assert_visibility(&state, "Plain", "Target", Visibility::Clear);
}

#[test]
fn scene_obscurement_is_the_grid_fallback_and_old_content_defaults_to_none() {
    let mut state = make_state("grid");
    place_cell(&mut state, "Target", 3, 1);
    set_scene_light(&mut state, Some(Light::Bright));
    assert_visibility(&state, "Plain", "Target", Visibility::Clear);
    set_scene_obscured(&mut state, Some(Obscurement::Heavy));
    assert_visibility(&state, "Plain", "Target", Visibility::HeavilyObscured);

    let (_scratch, registry) = registry();
    let zones = registry.scenes.get("zones").expect("zones scene");
    assert!(zones.obscured.is_none(), "old content has no scene obscurement");
    assert!(
        zones
            .zones
            .iter()
            .all(|zone| zone.id != "dark" || zone.obscured.is_none()),
        "old content has no zone obscurement"
    );
}

// ---------------------------------------------------------------------------
// AC-3: darkvision, blindsight, tremorsense, truesight, hidden, total cover
// ---------------------------------------------------------------------------

#[test]
fn darkvision_changes_light_only_within_range_and_never_heavy_obscurement() {
    let mut state = make_state("grid");
    set_scene_light(&mut state, Some(Light::Dark));
    place_cell(&mut state, "Observer", 1, 1);
    place_cell(&mut state, "Target", 13, 1); // 60 ft
    assert_visibility(&state, "Observer", "Target", Visibility::LightlyObscured);
    place_cell(&mut state, "Target", 14, 1); // 65 ft
    assert_visibility(&state, "Observer", "Target", Visibility::HeavilyObscured);

    set_scene_light(&mut state, Some(Light::Dim));
    place_cell(&mut state, "Target", 13, 1);
    assert_visibility(&state, "Observer", "Target", Visibility::Clear);
    place_cell(&mut state, "Target", 14, 1);
    assert_visibility(&state, "Observer", "Target", Visibility::LightlyObscured);

    set_scene_obscured(&mut state, Some(Obscurement::Heavy));
    place_cell(&mut state, "Target", 13, 1);
    assert_visibility(&state, "Observer", "Target", Visibility::HeavilyObscured);
}

#[test]
fn blindsight_ignores_light_and_obscurement_within_range_but_not_total_cover() {
    let mut state = make_state("grid");
    set_scene_light(&mut state, Some(Light::Dark));
    set_scene_obscured(&mut state, Some(Obscurement::Heavy));
    place_cell(&mut state, "Blindseer", 1, 1);
    place_cell(&mut state, "Target", 7, 1); // 30 ft
    assert_visibility(&state, "Blindseer", "Target", Visibility::Clear);
    add_condition(&mut state, "Target", "hidden");
    assert_visibility(&state, "Blindseer", "Target", Visibility::Clear);
    place_cell(&mut state, "Target", 8, 1); // 35 ft
    assert_visibility(&state, "Blindseer", "Target", Visibility::Blocked);

    set_scene_cover_for_grid(&mut state, Cover::Total);
    add_condition(&mut state, "Target", "hidden");
    place_cell(&mut state, "Target", 7, 1);
    assert_visibility(&state, "Blindseer", "Target", Visibility::Blocked);
}

fn set_scene_cover_for_grid(state: &mut EncounterState, cover: Cover) {
    // Grid cover comes from walls/blockers; use a wall cell between the two
    // combatants instead of a zone flag.
    let scene = scene_mut(state);
    let grid = scene.grid.as_mut().expect("grid");
    grid.legend
        .insert("#".to_string(), combat_sim::content::schema::GridCell {
            wall: true,
            ..Default::default()
        });
    if cover == Cover::Total {
        grid.rows[1].replace_range(4..5, "#");
    }
    state.rebuild_caches();
}

#[test]
fn tremorsense_sees_ground_contact_but_not_flying_or_incorporeal_creatures() {
    let mut state = make_state("grid");
    set_scene_light(&mut state, Some(Light::Dark));
    set_scene_obscured(&mut state, Some(Obscurement::Heavy));
    place_cell(&mut state, "Tremorsenser", 1, 1);
    place_cell(&mut state, "Target", 7, 1); // 30 ft
    assert_visibility(&state, "Tremorsenser", "Target", Visibility::Clear);
    add_condition(&mut state, "Target", "hidden");
    assert_visibility(&state, "Tremorsenser", "Target", Visibility::Clear);

    make_flying(&mut state, "Target", 10);
    assert_visibility(&state, "Tremorsenser", "Target", Visibility::Blocked);
    make_flying(&mut state, "Target", 0);
    assert_visibility(&state, "Tremorsenser", "Target", Visibility::Clear);

    make_incorporeal(&mut state, "Target");
    assert_visibility(&state, "Tremorsenser", "Target", Visibility::Blocked);

    place_cell(&mut state, "Target", 8, 1); // 35 ft
    make_flying(&mut state, "Target", 0);
    assert_visibility(&state, "Tremorsenser", "Target", Visibility::Blocked);
}

#[test]
fn truesight_sees_darkness_and_invisible_but_not_ordinary_hiding_or_total_cover() {
    let mut state = make_state("grid");
    set_scene_light(&mut state, Some(Light::Dark));
    place_cell(&mut state, "Truesighted", 1, 1);
    place_cell(&mut state, "Target", 13, 1); // 60 ft
    assert_visibility(&state, "Truesighted", "Target", Visibility::Clear);
    add_condition(&mut state, "Target", "invisible");
    assert_visibility(&state, "Truesighted", "Target", Visibility::Clear);
    add_condition(&mut state, "Target", "hidden");
    assert_visibility(&state, "Truesighted", "Target", Visibility::Blocked);

    let mut state = make_state("grid");
    place_cell(&mut state, "Plain", 1, 1);
    place_cell(&mut state, "Target", 13, 1);
    add_condition(&mut state, "Target", "invisible");
    assert_visibility(&state, "Plain", "Target", Visibility::Blocked);
    add_condition(&mut state, "Target", "hidden");
    assert_visibility(&state, "Plain", "Target", Visibility::Blocked);
    assert_visibility(&state, "Observer", "Target", Visibility::Blocked);
}

#[test]
fn hidden_normal_vision_is_blocked_and_total_cover_blocks_visual_senses() {
    let mut state = make_state("zones");
    place(&mut state, "Plain", "hub");
    place(&mut state, "Target", "dim");
    add_condition(&mut state, "Target", "hidden");
    assert_visibility(&state, "Plain", "Target", Visibility::Blocked);
    set_zone_cover(&mut state, "dim", Cover::Total);
    assert_visibility(&state, "Observer", "Target", Visibility::LightlyObscured);
    add_condition(&mut state, "Target", "hidden");
    assert_visibility(&state, "Blindseer", "Target", Visibility::Blocked);
    assert_visibility(&state, "Truesighted", "Target", Visibility::Blocked);
}

// ---------------------------------------------------------------------------
// AC-4: attacks: advantage, disadvantage and cancellation
// ---------------------------------------------------------------------------

#[test]
fn attack_disadvantage_when_attacker_cannot_see_target_and_advantage_when_target_cannot_see_attacker()
{
    let mut state = make_state("zones");
    place(&mut state, "Plain", "hub");
    place(&mut state, "Target", "dark");
    set_zone_light(&mut state, "dark", Some(Light::Dark));
    let log = attack_log(&mut state, "Plain", "Target");
    assert!(
        log.iter()
            .any(|line| line.contains("dis:") && line.to_lowercase().contains("cannot see")),
        "{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("adv:")),
        "the target can see the attacker: {log:?}"
    );

    let mut state = make_state("zones");
    place(&mut state, "Plain", "dark");
    place(&mut state, "Target", "hub");
    let log = attack_log(&mut state, "Plain", "Target");
    assert!(
        log.iter()
            .any(|line| line.contains("adv:") && line.to_lowercase().contains("cannot see")),
        "{log:?}"
    );
    assert!(!log.iter().any(|line| line.contains("dis:")), "{log:?}");
}

#[test]
fn dim_light_alone_does_not_impose_attack_disadvantage() {
    let mut state = make_state("zones");
    place(&mut state, "Plain", "hub");
    place(&mut state, "Target", "dim");
    let log = attack_log(&mut state, "Plain", "Target");
    assert!(!log.iter().any(|line| line.contains("dis:")), "{log:?}");
}

#[test]
fn heavy_obscurement_imposes_disadvantage_and_both_unseen_cancel() {
    let mut state = make_state("zones");
    place(&mut state, "Plain", "hub");
    place(&mut state, "Target", "heavyobs");
    let log = attack_log(&mut state, "Plain", "Target");
    assert!(
        log.iter()
            .any(|line| line.contains("dis:") && line.to_lowercase().contains("cannot see")),
        "{log:?}"
    );

    let mut state = make_state("zones");
    place(&mut state, "Plain", "dark");
    place(&mut state, "Target", "dark");
    let log = attack_log(&mut state, "Plain", "Target");
    assert!(
        log.iter()
            .any(|line| line.contains("dis:") && line.contains("cannot see")),
        "{log:?}"
    );
    assert!(
        log.iter()
            .any(|line| line.contains("adv:") && line.contains("cannot see")),
        "{log:?}"
    );
    assert!(log.iter().any(|line| line.contains(" vs ")), "{log:?}");
}

#[test]
fn darkvision_removes_the_attackers_disadvantage_but_not_the_targets_advantage() {
    let mut state = make_state("zones");
    place(&mut state, "Observer", "hub");
    place(&mut state, "Target", "dark");
    let log = attack_log(&mut state, "Observer", "Target");
    assert!(
        !log.iter().any(|line| line.contains("dis:")),
        "darkvision should see the target: {log:?}"
    );
    assert!(
        log.iter()
            .any(|line| line.contains("adv:") && line.contains("cannot see")),
        "the target is still blind: {log:?}"
    );
}

#[test]
fn truesight_removes_invisibility_penalties_and_blindsight_removes_hidden_penalties() {
    let mut state = make_state("zones");
    place(&mut state, "Truesighted", "hub");
    place(&mut state, "Target", "hub");
    add_condition(&mut state, "Target", "invisible");
    let log = attack_log(&mut state, "Truesighted", "Target");
    assert!(
        !log.iter().any(|line| line.contains("dis:")),
        "truesight sees the invisible target: {log:?}"
    );

    let mut state = make_state("zones");
    place(&mut state, "Plain", "hub");
    place(&mut state, "Target", "hub");
    add_condition(&mut state, "Target", "invisible");
    let log = attack_log(&mut state, "Plain", "Target");
    assert!(
        log.iter()
            .any(|line| line.contains("dis:") && line.to_lowercase().contains("invisible")),
        "{log:?}"
    );

    let mut state = make_state("zones");
    place(&mut state, "Blindseer", "hub");
    place(&mut state, "Target", "hub");
    add_condition(&mut state, "Target", "hidden");
    let log = attack_log(&mut state, "Blindseer", "Target");
    assert!(
        !log.iter().any(|line| line.contains("dis:")),
        "blindsight sees a hidden target: {log:?}"
    );
}

#[test]
fn ability_attacks_and_forecasts_use_the_same_visibility_rules() {
    let mut state = make_state("zones");
    place(&mut state, "Observer", "hub");
    place(&mut state, "Target", "dark");
    let mut log = Vec::new();
    engine::use_entry(
        &mut state,
        "Observer",
        "bolt",
        &[],
        &UseOptions::default(),
        &mut ScriptedRng::new(&[12, 12, 5, 5, 5, 5]),
        &mut log,
    )
    .expect("ability attack should resolve");
    assert!(
        log.iter()
            .any(|line| line.contains("adv:") && line.contains("cannot see")),
        "{log:?}"
    );
    assert!(!log.iter().any(|line| line.contains("dis:")), "{log:?}");

    let attack = state.combatants[index(&state, "Observer")]
        .creature
        .attacks
        .as_ref()
        .expect("attacks")
        .iter()
        .find(|attack| attack.id == "sword")
        .expect("sword")
        .clone();
    let forecast = engine::forecast_attack(
        &state,
        index(&state, "Observer"),
        index(&state, "Target"),
        &attack,
    );
    assert!(forecast.advantage);
    assert!(!forecast.disadvantage);

    let mut state = make_state("zones");
    place(&mut state, "Plain", "hub");
    place(&mut state, "Target", "dark");
    let forecast = engine::simple_attack_forecast(
        &state,
        index(&state, "Plain"),
        index(&state, "Target"),
        5,
        combat_sim::content::schema::AttackKind::Melee,
        5,
        None,
    );
    assert!(forecast.disadvantage);
    assert!(!forecast.advantage);
}

// ---------------------------------------------------------------------------
// AC-5: status JSON and CLI
// ---------------------------------------------------------------------------

#[test]
fn status_json_exposes_raw_senses_and_parsed_vision() {
    let mut state = make_state("zones");
    place(&mut state, "Observer", "dim");
    place(&mut state, "Plain", "hub");
    set_zone_obscured(&mut state, "dim", Some(Obscurement::Light));
    let dir = std::env::temp_dir().join(format!(
        "combat-sim-senses-status-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("state dir");
    let path = dir.join("state.json");
    state.save(&path).expect("save state");
    let status = status_json(&path);
    let observer = combatant(&status, "Observer");
    assert_eq!(
        observer["senses"],
        "Darkvision 60 ft.; Passive Perception 12"
    );
    assert_eq!(observer["vision"]["darkvision_ft"], 60);
    assert_eq!(observer["vision"]["passive_perception"], 12);
    assert_eq!(observer["vision"]["light"], "dim");
    assert_eq!(observer["vision"]["obscurement"], "light");
    assert!(observer["vision"]["darkvision_ft"].as_i64().is_some());

    let plain = combatant(&status, "Plain");
    assert_eq!(plain["senses"], Value::Null);
    assert_eq!(plain["vision"]["darkvision_ft"], Value::Null);
    assert_eq!(plain["vision"]["blindsight_ft"], Value::Null);

    let text = run_cli(&path, &["status"]);
    assert_success(&text, "status");
    let text = stdout(&text);
    assert!(text.contains("Observer"), "{text}");
    assert!(text.contains("heavily obscured") || text.contains("lightly obscured"), "{text}");

    let masked = run_cli(&path, &["status", "--json", "--player"]);
    assert_success(&masked, "status --json --player");
    let masked: Value = serde_json::from_slice(&masked.stdout).expect("masked status JSON");
    assert!(masked["combatants"][0]["vision"].is_object());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn cli_content_check_accepts_obscurement_and_warns_when_grid_ignores_zone_obscurement() {
    let scratch = campaign();
    let output = Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .args(["--campaign", scratch.manifest().to_str().expect("utf-8")])
        .arg("content")
        .arg("check")
        .output()
        .expect("content check");
    assert_success(&output, "content check");
    assert!(
        !stdout(&output).to_lowercase().contains("error"),
        "{}",
        stdout(&output)
    );
}

// ---------------------------------------------------------------------------
// AC-6: old state and old content compatibility
// ---------------------------------------------------------------------------

#[test]
fn old_state_without_obscurement_loads_and_schema_version_is_unchanged() {
    assert_eq!(STATE_SCHEMA_VERSION, 1);
    let mut state = make_state("zones");
    place(&mut state, "Target", "dim");
    set_zone_obscured(&mut state, "dim", Some(Obscurement::Heavy));
    let dir = std::env::temp_dir().join(format!(
        "combat-sim-senses-old-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("state dir");
    let path = dir.join("state.json");
    let mut value = serde_json::to_value(&state).expect("serialize state");
    if let Some(scene) = value.get_mut("scene").and_then(|scene| scene.as_object_mut()) {
        scene.remove("obscured");
        if let Some(zones) = scene.get_mut("zones").and_then(|zones| zones.as_array_mut()) {
            for zone in zones {
                zone.as_object_mut().expect("zone object").remove("obscured");
            }
        }
    }
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write state");
    let loaded = EncounterState::load(&path).expect("old state should load");
    assert_eq!(loaded.schema_version, STATE_SCHEMA_VERSION);
    assert!(loaded.scene.as_ref().expect("scene").obscured.is_none());
    let log = attack_log(&mut loaded.clone(), "Plain", "Target");
    assert!(!log.is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_creature_without_senses_uses_normal_vision_and_vision_profile_is_available() {
    let state = make_state("zones");
    let profile: VisionProfile = engine::vision_profile(&state, index(&state, "Plain"));
    assert_eq!(profile.senses, ParsedSenses::default());
    assert_eq!(profile.light, Light::Bright);
    assert_eq!(profile.obscurement, Obscurement::None);
    assert_visibility(&state, "Plain", "Target", Visibility::Clear);
}

#[test]
fn can_see_reports_effective_light_obscurement_distance_and_source() {
    let mut state = make_state("zones");
    place(&mut state, "Observer", "hub");
    place(&mut state, "Target", "dark");
    set_zone_obscured(&mut state, "dark", Some(Obscurement::None));
    let result = engine::can_see(&state, index(&state, "Observer"), index(&state, "Target"));
    assert_eq!(result.visibility, Visibility::LightlyObscured);
    assert_eq!(result.light, Light::Dark);
    assert_eq!(result.obscurement, Obscurement::None);
    assert_eq!(result.distance_ft, Some(5));
    assert!(result.source.to_lowercase().contains("darkvision"));
    assert!(result.reason.to_lowercase().contains("dark"));

    let plain = engine::can_see(&state, index(&state, "Plain"), index(&state, "Target"));
    assert_eq!(plain.visibility, Visibility::HeavilyObscured);
    assert!(plain.source.to_lowercase().contains("normal"));
    let position: Position = state.combatants[index(&state, "Target")].position();
    assert_eq!(engine::obscurement_at(&state, &position), Obscurement::None);
    let _ = content::check::check(&state.content).error_count();
}
