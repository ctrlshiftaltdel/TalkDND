//! TASK-0115 (v1.5a): core combat actions.
//!
//! Synthetic content only: no player books, real saves or DM secrets.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use combat_sim::content::schema::{ActionResolution, Size};
use combat_sim::content::{self, ContentRegistry, ContentSources};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{
    self, AttackOptions, GrappleOptions, ShoveMode, ShoveOptions, UnarmedOptions, UseOptions,
};
use combat_sim::state::{EncounterState, NewEncounter, STATE_SCHEMA_VERSION};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "combat-sim-actions-{label}-{}-{n}",
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
            r#"rules:
  dying_conditions: [unconscious]
  combat_actions:
    unarmed_strike:
      ability: str
      proficient: true
      damage: "1+{str}"
      type: bludgeoning
      reach: 5
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
    two_weapon_fighting:
      require_light: true
      add_positive_ability_modifier: false
"#
        }
        "2024" => {
            r#"rules:
  dying_conditions: [unconscious]
  combat_actions:
    unarmed_strike:
      ability: str
      proficient: true
      damage: "1d3+{str}"
      type: bludgeoning
      reach: 5
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
    two_weapon_fighting:
      require_light: true
      add_positive_ability_modifier: false
"#
        }
        other => panic!("unknown edition {other}"),
    }
}

fn grappled_condition(edition: &str) -> &'static str {
    match edition {
        "2014" => {
            r#"  - id: grappled
    flags: [speed_zero]
"#
        }
        "2024" => {
            r#"  - id: grappled
    flags: [speed_zero]
    modifiers:
      - {applies: attack_roll, effect: disadvantage, when: {not: {linked_is_target: true}}}
"#
        }
        other => panic!("unknown edition {other}"),
    }
}

fn content_yaml(edition: &str) -> String {
    format!(
        r##"{rules}
conditions:
{grappled}
  - id: prone
    flags: [stand_up_half_speed]
  - id: incapacitated
    flags: [incapacitated]
  - id: unconscious
    implies: [incapacitated]
    flags: [speed_zero]

creatures:
  - id: grappler
    name: Grappler
    kind: pc
    side: party
    ac: 14
    hp: 30
    speed: {{walk: 30}}
    prof: 2
    abilities: {{str: 16, dex: 14, con: 12, int: 10, wis: 10, cha: 10}}
    skills: {{athletics: 5, acrobatics: 4}}
    attacks:
      - {{id: shortsword, name: Short Sword, kind: melee, ability: str, to_hit: 5, damage: [{{dice: "1d6+3", type: slashing}}], reach: 5, properties: [light, finesse]}}
      - {{id: dagger, name: Dagger, kind: melee, ability: str, to_hit: 5, damage: [{{dice: "1d4+3", type: piercing}}], reach: 5, properties: [light, finesse]}}
      - {{id: club, name: Club, kind: melee, ability: str, to_hit: 5, damage: [{{dice: "1d8+3", type: bludgeoning}}], reach: 5}}
  - id: target
    name: Target
    kind: monster
    side: enemy
    ac: 12
    hp: 100
    speed: {{walk: 30}}
    prof: 2
    abilities: {{str: 14, dex: 12, con: 12, int: 8, wis: 10, cha: 8}}
    skills: {{athletics: 4, acrobatics: 3}}
    saves: {{str: 4, dex: 3}}
    attacks:
      - {{id: claw, name: Claw, kind: melee, ability: str, to_hit: 4, damage: [{{dice: "1d6+2", type: slashing}}], reach: 5}}
  - id: ally
    name: Ally
    kind: pc
    side: party
    ac: 13
    hp: 20
    speed: {{walk: 30}}
    prof: 2
    abilities: {{str: 10, dex: 12, con: 12, int: 10, wis: 10, cha: 10}}
    attacks:
      - {{id: spear, name: Spear, kind: melee, ability: str, to_hit: 2, damage: [{{dice: "1d6", type: piercing}}], reach: 5}}
  - id: extra-fighter
    name: Extra Fighter
    kind: pc
    side: party
    ac: 14
    hp: 30
    speed: {{walk: 30}}
    prof: 2
    abilities: {{str: 16, dex: 14, con: 12, int: 10, wis: 10, cha: 10}}
    extra_attacks: 2
    attacks:
      - {{id: longsword, name: Long Sword, kind: melee, ability: str, to_hit: 5, damage: [{{dice: "1d8+3", type: slashing}}], reach: 5}}
  - id: feature-fighter
    name: Feature Fighter
    kind: pc
    side: party
    ac: 14
    hp: 30
    speed: {{walk: 30}}
    prof: 2
    abilities: {{str: 16, dex: 14, con: 12, int: 10, wis: 10, cha: 10}}
    attacks:
      - {{id: mace, name: Mace, kind: melee, ability: str, to_hit: 5, damage: [{{dice: "1d8+3", type: bludgeoning}}], reach: 5}}
    features:
      - id: extra-attack-feature
        name: Extra Attack Feature
        activation: passive
        extra_attacks: 1
    actions:
      - id: test-strike
        name: Test Strike
        activation: action
        target: {{type: creature, side: enemy, count: 1}}
        effects:
          - attack: {{to_hit: 5, kind: melee, damage: [{{dice: "1d8+3", type: slashing}}]}}

scenes:
  - id: zones
    name: Zones
    mode: zones
    zones:
      - {{id: alpha}}
      - {{id: beta}}
      - {{id: gamma}}
    edges:
      - {{a: alpha, b: beta, distance: 5}}
      - {{a: beta, b: gamma, distance: 5}}
  - id: grid
    name: Grid
    mode: grid
    zones:
      - {{id: floor}}
    grid:
      cell: 5
      legend: __LEGEND__
      rows: __ROWS__
      zones: {{floor: "."}}

encounters:
  - id: zones-fight
    scene: zones
    party_zone: alpha
    members:
      - {{creature: target, zone: beta}}
      - {{creature: ally, zone: alpha}}
  - id: extra-fight
    scene: zones
    party_zone: alpha
    members:
      - {{creature: extra-fighter, zone: alpha}}
      - {{creature: target, zone: beta}}
  - id: feature-fight
    scene: zones
    party_zone: alpha
    members:
      - {{creature: feature-fighter, zone: alpha}}
      - {{creature: target, zone: beta}}
  - id: grid-fight
    scene: grid
    party_zone: floor
    members:
      - {{creature: target, zone: floor, pos: [6, 1]}}
"##,
        rules = rules_block(edition),
        grappled = grappled_condition(edition),
    )
    .replace("__LEGEND__", r##"{".": {}, "#": {wall: true}}"##)
    .replace(
        "__ROWS__",
        "[\"########\", \"#......#\", \"########\"]",
    )
}

fn campaign(edition: &str) -> Scratch {
    let scratch = Scratch::new(edition);
    scratch.write(
        "manifest.yaml",
        &format!("name: Combat Actions {edition}\nruleset: \"{edition}\"\nlibrary: [library]\n"),
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
    .expect("synthetic combat action content should load");
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

fn state(edition: &str, id: &str, party: &[&str]) -> EncounterState {
    let (_scratch, registry) = registry(edition);
    encounter(registry, id, party)
}

fn index(state: &EncounterState, name: &str) -> usize {
    state.find_index(name).expect("combatant exists")
}

fn hp(state: &EncounterState, name: &str) -> i32 {
    state.combatants[index(state, name)].hp
}

fn has_condition(state: &EncounterState, name: &str, condition: &str) -> bool {
    state.combatants[index(state, name)]
        .conditions
        .iter()
        .any(|active| active.condition.eq_ignore_ascii_case(condition))
}

fn attack_slot_count(state: &EncounterState, name: &str) -> usize {
    state.combatants[index(state, name)]
        .once_used
        .keys()
        .filter(|key| key.starts_with("turn:attack-action:"))
        .count()
}

fn make_large(state: &mut EncounterState, name: &str) {
    let target = index(state, name);
    Arc::make_mut(&mut state.combatants[target].creature).size = Some(Size::Huge);
}

fn immune_to(state: &mut EncounterState, name: &str, condition: &str) {
    let target = index(state, name);
    Arc::make_mut(&mut state.combatants[target].creature).condition_immunities =
        Some(vec![condition.to_string()]);
}

// ---------------------------------------------------------------------------
// AC-1: grapple
// ---------------------------------------------------------------------------

#[test]
fn grapple_2014_contested_check_applies_condition_and_spends_the_action() {
    let mut state = state("2014", "zones-fight", &["grappler"]);
    let mut rng = ScriptedRng::new(&[15, 5]);
    let mut log = Vec::new();
    engine::grapple(
        &mut state,
        "Grappler",
        "Target",
        &GrappleOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    assert!(state.combatants[index(&state, "Grappler")].action_used);
    assert!(has_condition(&state, "Target", "grappled"));
    let active = state.combatants[index(&state, "Target")]
        .conditions
        .iter()
        .find(|active| active.condition == "grappled")
        .expect("grappled condition");
    assert_eq!(active.source.as_deref(), Some("Grappler"));
    assert_eq!(active.linked.as_deref(), Some("Grappler"));
    assert!(active.notes.as_deref().unwrap_or("").contains("escape"));
    assert!(
        log.iter()
            .any(|line| line.contains("Athletics") && line.contains("PASS")),
        "{log:?}"
    );
}

#[test]
fn grapple_2014_failure_does_not_apply_condition_but_spends_the_action() {
    let mut state = state("2014", "zones-fight", &["grappler"]);
    let mut rng = ScriptedRng::new(&[3, 19]);
    let mut log = Vec::new();
    engine::grapple(
        &mut state,
        "Grappler",
        "Target",
        &GrappleOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    assert!(state.combatants[index(&state, "Grappler")].action_used);
    assert!(!has_condition(&state, "Target", "grappled"));
    assert!(log.iter().any(|line| line.contains("FAIL")), "{log:?}");
}

#[test]
fn grapple_rejects_immune_large_and_out_of_reach_before_spending_the_action() {
    let mut immune = state("2014", "zones-fight", &["grappler"]);
    immune_to(&mut immune, "Target", "grappled");
    let error = engine::grapple(
        &mut immune,
        "Grappler",
        "Target",
        &GrappleOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("immune target");
    assert!(error.to_string().contains("immune"), "{error}");
    assert!(!immune.combatants[index(&immune, "Grappler")].action_used);

    let mut large = state("2014", "zones-fight", &["grappler"]);
    make_large(&mut large, "Target");
    let error = engine::grapple(
        &mut large,
        "Grappler",
        "Target",
        &GrappleOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("size limit");
    assert!(error.to_string().contains("size"), "{error}");
    assert!(!large.combatants[index(&large, "Grappler")].action_used);

    let mut far = state("2014", "zones-fight", &["grappler"]);
    let far_target = index(&far, "Target");
    far.combatants[far_target].set_zone("gamma");
    let error = engine::grapple(
        &mut far,
        "Grappler",
        "Target",
        &GrappleOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("out of reach");
    assert!(error.to_string().contains("reach"), "{error}");
    assert!(!far.combatants[index(&far, "Grappler")].action_used);
}

#[test]
fn grapple_2024_uses_a_strength_or_dexterity_save_with_the_unarmed_dc() {
    let mut state = state("2024", "zones-fight", &["grappler"]);
    let mut rng = ScriptedRng::new(&[12]);
    let mut log = Vec::new();
    engine::grapple(
        &mut state,
        "Grappler",
        "Target",
        &GrappleOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();

    assert!(has_condition(&state, "Target", "grappled"));
    assert!(
        log.iter()
            .any(|line| line.contains("DC 13") && line.contains("save")),
        "{log:?}"
    );

    // 2024 grappled creatures have disadvantage when attacking anyone except
    // the grappler, and the condition records who that grappler is.
    let target = index(&state, "Target");
    state.combatants[target].action_used = false;
    let mut rng = ScriptedRng::new(&[15, 10, 4]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Target",
        "Ally",
        "claw",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();
    assert!(
        log.iter()
            .any(|line| line.contains("dis:") && line.contains("grappled")),
        "{log:?}"
    );
}

#[test]
fn grapple_2024_failed_save_leaves_the_target_free() {
    let mut state = state("2024", "zones-fight", &["grappler"]);
    let mut log = Vec::new();
    engine::grapple(
        &mut state,
        "Grappler",
        "Target",
        &GrappleOptions::default(),
        &mut ScriptedRng::new(&[2]),
        &mut log,
    )
    .unwrap();
    assert!(!has_condition(&state, "Target", "grappled"));
    assert!(state.combatants[index(&state, "Grappler")].action_used);
    assert!(log.iter().any(|line| line.contains("FAIL")), "{log:?}");
}

// ---------------------------------------------------------------------------
// AC-2: shove
// ---------------------------------------------------------------------------

#[test]
fn shove_2014_can_knock_prone_or_push_five_feet() {
    let mut prone = state("2014", "zones-fight", &["grappler"]);
    let mut log = Vec::new();
    engine::shove(
        &mut prone,
        "Grappler",
        "Target",
        &ShoveOptions {
            mode: ShoveMode::KnockDown,
            ..ShoveOptions::default()
        },
        &mut ScriptedRng::new(&[15, 5]),
        &mut log,
    )
    .unwrap();
    assert!(has_condition(&prone, "Target", "prone"));
    assert!(log.iter().any(|line| line.contains("prone")), "{log:?}");

    let mut pushed = state("2014", "zones-fight", &["grappler"]);
    let mut log = Vec::new();
    engine::shove(
        &mut pushed,
        "Grappler",
        "Target",
        &ShoveOptions {
            mode: ShoveMode::Away,
            ..ShoveOptions::default()
        },
        &mut ScriptedRng::new(&[15, 5]),
        &mut log,
    )
    .unwrap();
    assert_eq!(pushed.combatants[index(&pushed, "Target")].zone, "gamma");
    assert!(log.iter().any(|line| line.contains("gamma")), "{log:?}");
}

#[test]
fn shove_push_uses_forced_movement_and_stops_at_a_wall() {
    let mut state = state("2014", "grid-fight", &["grappler"]);
    state.spatial = Some(combat_sim::state::SpatialState::default());
    let grappler = index(&state, "Grappler");
    let target = index(&state, "Target");
    state.combatants[grappler].set_cell(5, 1, "floor");
    state.combatants[target].set_cell(6, 1, "floor");
    let before = state.combatants[target].cell();
    let mut log = Vec::new();
    engine::shove(
        &mut state,
        "Grappler",
        "Target",
        &ShoveOptions {
            mode: ShoveMode::Away,
            ..ShoveOptions::default()
        },
        &mut ScriptedRng::new(&[15, 5]),
        &mut log,
    )
    .unwrap();
    assert_eq!(before, state.combatants[index(&state, "Target")].cell());
    assert!(
        log.iter()
            .any(|line| line.contains("stops") && line.contains("wall")),
        "{log:?}"
    );
}

#[test]
fn invalid_defense_does_not_spend_the_action_or_bonus_action() {
    for (edition, defense) in [
        ("2014", "foo"),
        ("2014", "str"),
        ("2024", "foo"),
        ("2024", "athletics"),
    ] {
        let mut grapple = state(edition, "zones-fight", &["grappler"]);
        let error = engine::grapple(
            &mut grapple,
            "Grappler",
            "Target",
            &GrappleOptions {
                defense: Some(defense.to_string()),
            },
            &mut ScriptedRng::new(&[]),
            &mut Vec::new(),
        )
        .expect_err("invalid grapple defense");
        assert!(
            error.to_string().contains("not a legal"),
            "{edition} {defense}: {error}"
        );
        let grappler = index(&grapple, "Grappler");
        assert!(!grapple.combatants[grappler].action_used);
        assert!(!grapple.combatants[grappler].bonus_action_used);
        assert!(!has_condition(&grapple, "Target", "grappled"));

        let mut shove = state(edition, "zones-fight", &["grappler"]);
        let error = engine::shove(
            &mut shove,
            "Grappler",
            "Target",
            &ShoveOptions {
                defense: Some(defense.to_string()),
                mode: ShoveMode::KnockDown,
            },
            &mut ScriptedRng::new(&[]),
            &mut Vec::new(),
        )
        .expect_err("invalid shove defense");
        assert!(
            error.to_string().contains("not a legal"),
            "{edition} {defense}: {error}"
        );
        let grappler = index(&shove, "Grappler");
        assert!(!shove.combatants[grappler].action_used);
        assert!(!shove.combatants[grappler].bonus_action_used);
        assert!(!has_condition(&shove, "Target", "prone"));
    }
}

// ---------------------------------------------------------------------------
// AC-3: unarmed strike
// ---------------------------------------------------------------------------

#[test]
fn unarmed_strike_uses_edition_damage_and_can_knock_out() {
    let mut state_2014 = state("2014", "zones-fight", &["grappler"]);
    let before = hp(&state_2014, "Target");
    let mut log = Vec::new();
    engine::attack_unarmed(
        &mut state_2014,
        "Grappler",
        "Target",
        &UnarmedOptions::default(),
        &mut ScriptedRng::new(&[15]),
        &mut log,
    )
    .unwrap();
    assert_eq!(hp(&state_2014, "Target"), before - 4);
    assert!(
        log.iter().any(|line| line.contains("Unarmed Strike")),
        "{log:?}"
    );

    let mut state_2024 = state("2024", "zones-fight", &["grappler"]);
    let before = hp(&state_2024, "Target");
    let mut log = Vec::new();
    engine::attack_unarmed(
        &mut state_2024,
        "Grappler",
        "Target",
        &UnarmedOptions::default(),
        &mut ScriptedRng::new(&[15, 2]),
        &mut log,
    )
    .unwrap();
    assert_eq!(hp(&state_2024, "Target"), before - 5);
    assert!(
        log.iter().any(|line| line.contains("1d3(2)+3=5")),
        "{log:?}"
    );

    let mut knockout = state("2014", "zones-fight", &["grappler"]);
    let knockout_target = index(&knockout, "Target");
    knockout.combatants[knockout_target].hp = 2;
    engine::attack_unarmed(
        &mut knockout,
        "Grappler",
        "Target",
        &UnarmedOptions {
            nonlethal: true,
            ..UnarmedOptions::default()
        },
        &mut ScriptedRng::new(&[15]),
        &mut Vec::new(),
    )
    .unwrap();
    let target = index(&knockout, "Target");
    assert!(knockout.combatants[target].knocked_out);
    assert!(has_condition(&knockout, "Target", "unconscious"));
}

// ---------------------------------------------------------------------------
// AC-4: two-weapon fighting
// ---------------------------------------------------------------------------

#[test]
fn two_weapon_fighting_off_hand_spends_bonus_action_and_loses_positive_modifier() {
    let mut state = state("2014", "zones-fight", &["grappler"]);
    let before = hp(&state, "Target");
    engine::attack(
        &mut state,
        "Grappler",
        "Target",
        "shortsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    let after_main = hp(&state, "Target");
    assert_eq!(after_main, before - 7);

    let mut log = Vec::new();
    engine::attack_offhand(
        &mut state,
        "Grappler",
        "Target",
        "dagger",
        &mut ScriptedRng::new(&[15, 4]),
        &mut log,
    )
    .unwrap();
    assert!(state.combatants[index(&state, "Grappler")].bonus_action_used);
    assert_eq!(hp(&state, "Target"), after_main - 4);
    assert!(log.iter().any(|line| line.contains("off-hand")), "{log:?}");
}

#[test]
fn two_weapon_fighting_fails_closed_without_the_required_state() {
    let mut before_main = state("2014", "zones-fight", &["grappler"]);
    let error = engine::attack_offhand(
        &mut before_main,
        "Grappler",
        "Target",
        "dagger",
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("off-hand before main hand");
    assert!(
        error.to_string().to_lowercase().contains("light") || error.to_string().contains("first"),
        "{error}"
    );
    assert!(!before_main.combatants[index(&before_main, "Grappler")].bonus_action_used);

    let mut light_only = state("2014", "zones-fight", &["grappler"]);
    engine::attack(
        &mut light_only,
        "Grappler",
        "Target",
        "shortsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    let error = engine::attack_offhand(
        &mut light_only,
        "Grappler",
        "Target",
        "club",
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("non-light off-hand");
    assert!(
        error.to_string().to_lowercase().contains("light"),
        "{error}"
    );
    assert!(!light_only.combatants[index(&light_only, "Grappler")].bonus_action_used);

    let error = engine::attack_offhand(
        &mut light_only,
        "Grappler",
        "Target",
        "shortsword",
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("same weapon");
    assert!(
        error.to_string().contains("different") || error.to_string().contains("second"),
        "{error}"
    );

    let mut spent = state("2014", "zones-fight", &["grappler"]);
    let spent_grappler = index(&spent, "Grappler");
    engine::attack(
        &mut spent,
        "Grappler",
        "Target",
        "shortsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    spent.combatants[spent_grappler].bonus_action_used = true;
    let error = engine::attack_offhand(
        &mut spent,
        "Grappler",
        "Target",
        "dagger",
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("bonus action already used");
    assert!(error.to_string().contains("bonus action"), "{error}");
}

// ---------------------------------------------------------------------------
// AC-5: extra attack
// ---------------------------------------------------------------------------

#[test]
fn extra_attack_count_comes_from_creature_data_and_skips_a_dead_target() {
    let mut state = state("2014", "extra-fight", &[]);
    let target = index(&state, "Target");
    state.combatants[target].hp = 10;
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4, 15, 5]),
        &mut log,
    )
    .unwrap();
    assert!(state.combatants[index(&state, "Extra Fighter")].action_used);
    assert_eq!(hp(&state, "Target"), 0);
    assert!(log.iter().any(|line| line.contains("skipped")), "{log:?}");
    let attacks = log
        .iter()
        .filter(|line| line.contains("attacks Target with Long Sword"))
        .count();
    assert_eq!(attacks, 2, "{log:?}");
}

#[test]
fn extra_attack_can_come_from_a_passive_feature_and_not_from_use_or_off_hand() {
    let mut feature = state("2014", "feature-fight", &[]);
    let mut log = Vec::new();
    engine::attack(
        &mut feature,
        "Feature Fighter",
        "Target",
        "mace",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4, 15, 5]),
        &mut log,
    )
    .unwrap();
    assert_eq!(
        log.iter()
            .filter(|line| line.contains("attacks Target with Mace"))
            .count(),
        2,
        "{log:?}"
    );

    let (_scratch, registry) = registry("2014");
    let mut use_state = encounter(registry, "feature-fight", &[]);
    let mut rng = ScriptedRng::new(&[15, 4]);
    let mut log = Vec::new();
    engine::use_ability(
        &mut use_state,
        "Feature Fighter",
        "test-strike",
        &UseOptions {
            targets: vec!["Target".to_string()],
            ..UseOptions::default()
        },
        &mut rng,
        &mut log,
    )
    .unwrap();
    assert_eq!(
        log.iter()
            .filter(|line| line.contains("attacks Target"))
            .count(),
        1,
        "{log:?}"
    );
}

#[test]
fn extra_attack_budget_allows_grapple_then_attack() {
    let mut state = state("2014", "extra-fight", &[]);
    engine::grapple(
        &mut state,
        "Extra Fighter",
        "Target",
        &GrappleOptions::default(),
        &mut ScriptedRng::new(&[15, 5]),
        &mut Vec::new(),
    )
    .unwrap();
    let fighter = index(&state, "Extra Fighter");
    assert!(state.combatants[fighter].action_used);
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 1);
    assert!(has_condition(&state, "Target", "grappled"));

    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4, 15, 5]),
        &mut log,
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 3);
    assert_eq!(
        log.iter()
            .filter(|line| line.contains("attacks Target with Long Sword"))
            .count(),
        2,
        "{log:?}"
    );
    assert!(
        log.iter().any(|line| line.contains("attack 1/2")),
        "{log:?}"
    );
    assert_eq!(hp(&state, "Target"), 85);
}

#[test]
fn extra_attack_budget_allows_shove_then_attack() {
    let mut state = state("2014", "extra-fight", &[]);
    engine::shove(
        &mut state,
        "Extra Fighter",
        "Target",
        &ShoveOptions {
            mode: ShoveMode::KnockDown,
            ..ShoveOptions::default()
        },
        &mut ScriptedRng::new(&[15, 5]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 1);
    assert!(has_condition(&state, "Target", "prone"));

    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4, 15, 5]),
        &mut log,
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 3);
    assert_eq!(
        log.iter()
            .filter(|line| line.contains("attacks Target with Long Sword"))
            .count(),
        2,
        "{log:?}"
    );
}

#[test]
fn single_attack_creature_cannot_act_twice_after_grapple() {
    let mut state = state("2014", "zones-fight", &["grappler"]);
    engine::grapple(
        &mut state,
        "Grappler",
        "Target",
        &GrappleOptions::default(),
        &mut ScriptedRng::new(&[15, 5]),
        &mut Vec::new(),
    )
    .unwrap();
    let grappler = index(&state, "Grappler");
    assert!(state.combatants[grappler].action_used);
    assert_eq!(attack_slot_count(&state, "Grappler"), 1);

    let error = engine::attack(
        &mut state,
        "Grappler",
        "Target",
        "shortsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("no attack slot after a single-attack grapple");
    assert!(
        error.to_string().contains("no Attack action attacks left"),
        "{error}"
    );
}

#[test]
fn creature_with_no_attack_slots_left_cannot_start_another_action() {
    let mut state = state("2014", "extra-fight", &[]);
    engine::attack(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4, 15, 5, 15, 6]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 3);

    let error = engine::attack(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("no attack slot");
    assert!(
        error.to_string().contains("no Attack action attacks left"),
        "{error}"
    );

    let error = engine::grapple(
        &mut state,
        "Extra Fighter",
        "Target",
        &GrappleOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("no attack slot for grapple");
    assert!(
        error.to_string().contains("no Attack action attacks left"),
        "{error}"
    );
}

#[test]
fn open_attack_action_blocks_use() {
    let mut state = state("2014", "feature-fight", &[]);
    engine::grapple(
        &mut state,
        "Feature Fighter",
        "Target",
        &GrappleOptions::default(),
        &mut ScriptedRng::new(&[15, 5]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Feature Fighter"), 1);

    let error = engine::use_ability(
        &mut state,
        "Feature Fighter",
        "test-strike",
        &UseOptions {
            targets: vec!["Target".to_string()],
            ..UseOptions::default()
        },
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("use after an open Attack action");
    assert!(
        error.to_string().contains("already used an action"),
        "{error}"
    );
}

#[test]
fn off_hand_after_attack_uses_bonus_action_without_an_attack_slot() {
    let mut state = state("2014", "zones-fight", &["grappler"]);
    let grappler = index(&state, "Grappler");
    Arc::make_mut(&mut state.combatants[grappler].creature).extra_attacks = Some(1);
    engine::attack(
        &mut state,
        "Grappler",
        "Target",
        "shortsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4, 15, 5]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Grappler"), 2);
    assert!(state.combatants[grappler].action_used);

    engine::attack_offhand(
        &mut state,
        "Grappler",
        "Target",
        "dagger",
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(state.combatants[grappler].bonus_action_used);
    assert!(state.combatants[grappler].action_used);
    assert_eq!(attack_slot_count(&state, "Grappler"), 2);

    let error = engine::attack(
        &mut state,
        "Grappler",
        "Target",
        "shortsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("off-hand did not free an attack slot");
    assert!(
        error.to_string().contains("no Attack action attacks left"),
        "{error}"
    );
}

#[test]
fn attack_budget_resets_at_turn_start() {
    let mut state = state("2014", "extra-fight", &[]);
    let fighter = index(&state, "Extra Fighter");
    let target = index(&state, "Target");
    state.initiative = vec![fighter, target];
    state.turn = 0;
    state.round = 1;

    engine::attack(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4, 15, 5, 15, 6]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(state.combatants[fighter].action_used);
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 3);

    let mut rng = ScriptedRng::new(&[]);
    engine::next_turn(&mut state, &mut rng, &mut Vec::new()).unwrap();
    engine::next_turn(&mut state, &mut rng, &mut Vec::new()).unwrap();

    assert_eq!(state.round, 2);
    assert!(!state.combatants[fighter].action_used);
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 0);

    engine::attack(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4, 15, 5, 15, 6]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(state.combatants[fighter].action_used);
}

#[test]
fn attack_one_then_grapple_keeps_remaining_slots() {
    let mut state = state("2014", "extra-fight", &[]);
    engine::attack_one(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 1);
    assert_eq!(hp(&state, "Target"), 93);

    engine::grapple(
        &mut state,
        "Extra Fighter",
        "Target",
        &GrappleOptions::default(),
        &mut ScriptedRng::new(&[15, 5]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 2);
    assert!(has_condition(&state, "Target", "grappled"));

    // One slot remains and can still be used for a single attack.
    engine::attack_one(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 5]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 3);
    assert_eq!(hp(&state, "Target"), 85);
}

#[test]
fn attack_one_then_shove_keeps_remaining_slots() {
    let mut state = state("2014", "extra-fight", &[]);
    engine::attack_one(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    engine::shove(
        &mut state,
        "Extra Fighter",
        "Target",
        &ShoveOptions {
            mode: ShoveMode::KnockDown,
            ..ShoveOptions::default()
        },
        &mut ScriptedRng::new(&[15, 5]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 2);
    assert!(has_condition(&state, "Target", "prone"));
}

#[test]
fn attack_one_then_remaining_attack_resolves_the_rest() {
    let mut state = state("2014", "extra-fight", &[]);
    engine::attack_one(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 1);

    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 5, 15, 6]),
        &mut log,
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 3);
    assert_eq!(
        log.iter()
            .filter(|line| line.contains("attacks Target with Long Sword"))
            .count(),
        2,
        "{log:?}"
    );
    assert!(
        log.iter().any(|line| line.contains("attack 1/2")),
        "{log:?}"
    );
}

#[test]
fn unarmed_attack_one_consumes_one_slot() {
    let mut state = state("2014", "extra-fight", &[]);
    engine::attack_unarmed(
        &mut state,
        "Extra Fighter",
        "Target",
        &UnarmedOptions {
            one: true,
            ..UnarmedOptions::default()
        },
        &mut ScriptedRng::new(&[15]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 1);
    assert_eq!(hp(&state, "Target"), 96);

    engine::attack(
        &mut state,
        "Extra Fighter",
        "Target",
        "longsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 5, 15, 6]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(attack_slot_count(&state, "Extra Fighter"), 3);
}

#[test]
fn attack_one_without_extra_attacks_matches_a_normal_attack() {
    let mut one = state("2014", "zones-fight", &["grappler"]);
    engine::attack_one(
        &mut one,
        "Grappler",
        "Target",
        "shortsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(hp(&one, "Target"), 93);
    assert_eq!(attack_slot_count(&one, "Grappler"), 1);
    assert!(one.combatants[index(&one, "Grappler")].action_used);

    let mut normal = state("2014", "zones-fight", &["grappler"]);
    engine::attack(
        &mut normal,
        "Grappler",
        "Target",
        "shortsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(hp(&normal, "Target"), 93);
    assert_eq!(attack_slot_count(&normal, "Grappler"), 1);
    assert!(normal.combatants[index(&normal, "Grappler")].action_used);
}

// ---------------------------------------------------------------------------
// AC-6: compatibility
// ---------------------------------------------------------------------------

#[test]
fn old_state_without_new_fields_loads_and_keeps_the_old_attack_path() {
    let scratch = Scratch::new("compat");
    let state = state("2014", "zones-fight", &["grappler"]);
    let mut value = serde_json::to_value(&state).expect("serialize state");
    if let Some(combatants) = value.get_mut("combatants").and_then(|v| v.as_array_mut()) {
        for combatant in combatants {
            if let Some(creature) = combatant.get_mut("creature") {
                if let Some(object) = creature.as_object_mut() {
                    object.remove("extra_attacks");
                }
            }
        }
    }
    if let Some(rules) = value
        .get_mut("content")
        .and_then(|v| v.get_mut("rules"))
        .and_then(|v| v.as_object_mut())
    {
        rules.remove("combat_actions");
    }
    let path = scratch.write("old.state.json", &value.to_string());
    let mut loaded = EncounterState::load(&path).expect("old state loads");
    assert_eq!(loaded.schema_version, STATE_SCHEMA_VERSION);
    assert_eq!(loaded.schema_version, 1);

    engine::attack(
        &mut loaded,
        "Grappler",
        "Target",
        "shortsword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(hp(&loaded, "Target"), 93);

    let error = engine::attack_unarmed(
        &mut loaded,
        "Grappler",
        "Target",
        &UnarmedOptions::default(),
        &mut ScriptedRng::new(&[]),
        &mut Vec::new(),
    )
    .expect_err("old rules have no unarmed data");
    assert!(error.to_string().contains("unarmed"), "{error}");
}

// ---------------------------------------------------------------------------
// AC-8: content validation
// ---------------------------------------------------------------------------

#[test]
fn combat_action_rules_pass_content_check_for_both_editions() {
    for edition in ["2014", "2024"] {
        let (_scratch, registry) = registry(edition);
        let report = content::check(&registry);
        assert_eq!(report.error_count(), 0, "{edition}: {:#?}", report.issues);
    }
}

#[test]
fn real_edition_files_merge_the_expected_combat_action_data() {
    let cases = [
        (
            "2014",
            "real-2014.yaml",
            ActionResolution::Contest,
            "1+{str}",
        ),
        (
            "2024",
            "real-2024.yaml",
            ActionResolution::Save,
            "1d3+{str}",
        ),
    ];
    for (edition, manifest, resolution, damage) in cases {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/rulesets")
            .join(manifest);
        let registry = ContentSources {
            campaign: Some(path),
            ..ContentSources::default()
        }
        .load()
        .expect("real edition content should load");
        let actions = registry
            .rules
            .combat_actions
            .as_ref()
            .expect("combat action data");
        assert_eq!(
            actions.grapple.as_ref().unwrap().resolution.unwrap(),
            resolution,
            "{edition}"
        );
        assert_eq!(
            actions
                .unarmed_strike
                .as_ref()
                .unwrap()
                .damage
                .as_ref()
                .unwrap()
                .as_text(),
            damage,
            "{edition}"
        );
        assert_eq!(
            actions.two_weapon_fighting.as_ref().unwrap().require_light,
            Some(true),
            "{edition}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC-7: CLI surface and smoke
// ---------------------------------------------------------------------------

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("run combat-sim")
}

fn cli_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn cli_lists_and_runs_the_new_combat_actions() {
    let help = run(&["--help"]);
    assert!(help.status.success(), "{}", cli_text(&help));
    let text = cli_text(&help);
    assert!(text.contains("grapple"), "{text}");
    assert!(text.contains("shove"), "{text}");

    let attack_help = run(&["attack", "--help"]);
    assert!(attack_help.status.success(), "{}", cli_text(&attack_help));
    let text = cli_text(&attack_help);
    assert!(text.contains("--unarmed"), "{text}");
    assert!(text.contains("--offhand"), "{text}");
    assert!(text.contains("--one"), "{text}");

    for args in [
        ["attack", "A", "B", "x", "--one", "--offhand"],
        ["attack", "A", "B", "x", "--one", "--multi"],
    ] {
        let output = run(&args);
        assert!(!output.status.success(), "{}", cli_text(&output));
        let text = cli_text(&output);
        assert!(text.contains("--one"), "{text}");
    }

    let scratch = campaign("2014");
    let state_path = scratch.dir.join("cli.state.json");
    let manifest = scratch.manifest();
    let new = run(&[
        "--state",
        state_path.to_str().unwrap(),
        "--campaign",
        manifest.to_str().unwrap(),
        "new",
        "--encounter",
        "zones-fight",
        "--party",
        "grappler",
    ]);
    assert!(new.status.success(), "{}", cli_text(&new));

    let init = run(&[
        "--state",
        state_path.to_str().unwrap(),
        "--seed",
        "7",
        "init",
        "--set",
        "Grappler=30",
    ]);
    assert!(init.status.success(), "{}", cli_text(&init));

    let grapple = run(&[
        "--state",
        state_path.to_str().unwrap(),
        "--seed",
        "7",
        "grapple",
        "Grappler",
        "Target",
    ]);
    assert!(grapple.status.success(), "{}", cli_text(&grapple));
    let text = cli_text(&grapple);
    assert!(
        text.contains("Grapple") || text.contains("grapple"),
        "{text}"
    );
    assert!(text.contains("PASS") || text.contains("FAIL"), "{text}");

    let status = run(&["--state", state_path.to_str().unwrap(), "status", "--json"]);
    assert!(status.status.success(), "{}", cli_text(&status));
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(value["combatants"].is_array());
}
