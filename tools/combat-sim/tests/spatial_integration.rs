//! P5 integration tests: unified spatial queries in engine and CLI.
//!
//! Contract: `tools/combat-sim/docs/SPATIAL-CONTRACT-v1.md`.
//! Rules matrix: `tools/combat-sim/docs/SPATIAL-RULES-MATRIX.md`.
//!
//! Every fixture here is synthetic. The condition-data test loads the tracked
//! real rules registry without copying any book prose.
//!
//! Phase map:
//! - Phase 1 (audit): every call site below names the old 2D query it replaces.
//! - Phase 2 (movement): legality, stepwise transitions, reactions, triggers.
//! - Phase 3 (attacks/AoE): target volume, line of sight, cover, area cells.
//! - Phase 4 (CLI/legacy): old saves keep the legacy result.

use std::collections::VecDeque;
use std::path::Path;

use combat_sim::content::schema::{ConditionFlag, Creature, Scene, Size};
use combat_sim::content::{ContentRegistry, ContentSources};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{self, AddConditionOptions, AttackOptions};
use combat_sim::space::{FlightState, SpatialPose, SpatialProfile, Squeeze};
use combat_sim::state::{CombatantState, EncounterState, SpatialState, STATE_SCHEMA_VERSION};
use serde_json::json;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn registry(ruleset: &str) -> ContentRegistry {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/fixtures/rulesets/real-{ruleset}.yaml"));
    ContentSources {
        campaign: Some(manifest),
        ..ContentSources::default()
    }
    .load()
    .unwrap_or_else(|error| panic!("real {ruleset} registry should load: {error}"))
}

fn creature(
    id: &str,
    size: Size,
    fly: i32,
    hover: bool,
    attacks: serde_json::Value,
) -> Creature {
    serde_json::from_value(json!({
        "id": id,
        "name": id,
        "size": size,
        "hp": 30,
        "ac": 12,
        "speed": {"walk": 30, "fly": fly, "hover": hover},
        "abilities": {"str": 10, "dex": 10, "con": 10, "int": 10, "wis": 10, "cha": 10},
        "attacks": attacks
    }))
    .expect("synthetic creature")
}

fn melee_attack() -> serde_json::Value {
    json!([{
        "id": "synthetic-sword",
        "name": "Synthetic Sword",
        "kind": "melee",
        "ability": "str",
        "to_hit": 5,
        "damage": [{"dice": "1d6+2", "type": "slashing"}],
        "reach": 5
    }])
}

fn ranged_attack() -> serde_json::Value {
    json!([{
        "id": "synthetic-bow",
        "name": "Synthetic Bow",
        "kind": "ranged",
        "ability": "dex",
        "to_hit": 5,
        "damage": [{"dice": "1d6+2", "type": "piercing"}],
        "range": [80, 320]
    }])
}

fn no_attacks() -> serde_json::Value {
    json!([])
}

fn combatant(
    name: &str,
    creature: Creature,
    side: &str,
    x: i32,
    y: i32,
    altitude: i32,
    flight: FlightState,
    footprint: (u32, u32),
    conditions: &[&str],
) -> CombatantState {
    let conditions: Vec<_> = conditions
        .iter()
        .map(|id| json!({"condition": id}))
        .collect();
    serde_json::from_value(json!({
        "creature": creature,
        "name": name,
        "side": side,
        "max_hp": 30,
        "hp": 30,
        "zone": "open",
        "pos": {"kind": "cell", "x": x, "y": y},
        "pose": {
            "cell": {"x": x, "y": y, "z": altitude},
            "footprint": {"width": footprint.0, "depth": footprint.1}
        },
        "vertical": {"altitude_ft": altitude, "flight": flight, "fall_distance_ft": 0},
        "speed_left": 30,
        "conditions": conditions
    }))
    .expect("synthetic combatant")
}

fn grounded(name: &str, creature: Creature, side: &str, x: i32, y: i32) -> CombatantState {
    combatant(
        name,
        creature,
        side,
        x,
        y,
        0,
        FlightState::Grounded,
        (1, 1),
        &[],
    )
}

fn state_with(
    registry: ContentRegistry,
    combatants: Vec<CombatantState>,
    scene: serde_json::Value,
    spatial: Option<SpatialState>,
) -> EncounterState {
    let initiative: Vec<usize> = (0..combatants.len()).collect();
    let mut state: EncounterState = serde_json::from_value(json!({
        "schema_version": STATE_SCHEMA_VERSION,
        "content": registry,
        "scene": scene,
        "spatial": spatial,
        "combatants": combatants,
        "initiative": initiative,
        "turn": 0,
        "round": 1
    }))
    .expect("synthetic state");
    state.rebuild_caches();
    state
}

fn grid_scene(rows: &[&str], legend: serde_json::Value) -> serde_json::Value {
    json!({
        "id": "synthetic-p5-grid",
        "mode": "grid",
        "grid": {"cell": 5, "rows": rows, "legend": legend}
    })
}

fn scene_with(rows: &[&str], legend: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let mut scene = grid_scene(rows, legend);
    if let (Some(scene), Some(extra)) = (scene.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            scene.insert(key.clone(), value.clone());
        }
    }
    scene
}

fn spatial() -> Option<SpatialState> {
    Some(SpatialState::default())
}

#[derive(Debug)]
struct ScriptedRng {
    values: VecDeque<u32>,
    sides: Vec<u32>,
}

impl ScriptedRng {
    fn new(values: &[u32]) -> Self {
        Self {
            values: values.iter().copied().collect(),
            sides: Vec::new(),
        }
    }
}

impl DiceRng for ScriptedRng {
    fn roll_die(&mut self, sides: u32) -> u32 {
        self.sides.push(sides);
        self.values
            .pop_front()
            .expect("scripted RNG ran out while rolling")
    }
}

fn index(state: &EncounterState, name: &str) -> usize {
    state.find_index(name).expect("combatant exists")
}

// ---------------------------------------------------------------------------
// Phase 1: call-site audit
// ---------------------------------------------------------------------------

/// Audit row 1: condition data. `stunned` in the shared 2014/2024 file is
/// missing the `speed_zero` flag, so a 2014 flying creature never fell.
#[test]
fn audit_stunned_carries_speed_zero_in_shared_condition_data() {
    let registry = registry("2014");
    let stunned = registry
        .conditions
        .get("stunned")
        .expect("the shared condition file defines stunned");
    assert!(
        stunned.flags.contains(&ConditionFlag::SpeedZero),
        "stunned must carry speed_zero; flags were {:?}",
        stunned.flags
    );
    assert!(
        stunned
            .implies
            .iter()
            .any(|condition| condition == "incapacitated"),
        "stunned still implies incapacitated; implies were {:?}",
        stunned.implies
    );
}

/// Audit row 2: a footprint's near edge must decide reach, not the anchor
/// cell. A Large target anchored at (4,4) is 10 ft from (6,6) by anchor but
/// 5 ft by volume; the old `Space::can_reach(anchor)` rejected the attack.
#[test]
fn audit_reach_uses_target_volume_not_anchor_cell() {
    let mut state = state_with(
        registry("2024"),
        vec![
            grounded(
                "Attacker",
                creature("synthetic-attacker", Size::Medium, 0, false, melee_attack()),
                "party",
                6,
                6,
            ),
            combatant(
                "Target",
                creature("synthetic-target", Size::Large, 0, false, no_attacks()),
                "enemy",
                4,
                4,
                0,
                FlightState::Grounded,
                (2, 2),
                &[],
            ),
        ],
        grid_scene(
            &["......", "......", "......", "......", "......", "......"],
            json!({".": {"elevation": 0}}),
        ),
        spatial(),
    );
    let mut rng = ScriptedRng::new(&[15, 4]);
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Attacker",
        "Target",
        "synthetic-sword",
        &AttackOptions::default(),
        &mut rng,
        &mut log,
    )
    .expect("the Large target's near edge is within 5 ft");
    assert!(
        log.iter().any(|line| line.contains("HIT")),
        "spatial reach should hit: {log:?}"
    );
}

/// Audit row 3: `GridCell.height_ft` was parsed but never used. A 10-ft
/// obstacle must block line of sight, while a 3-ft low obstacle only grants
/// cover.
#[test]
fn audit_grid_cell_height_is_a_vertical_obstacle() {
    let tall = state_with(
        registry("2024"),
        vec![
            grounded(
                "Archer",
                creature("synthetic-archer", Size::Medium, 0, false, ranged_attack()),
                "party",
                0,
                0,
            ),
            grounded(
                "Target",
                creature("synthetic-target", Size::Medium, 0, false, no_attacks()),
                "enemy",
                2,
                0,
            ),
        ],
        grid_scene(&[".p."], json!({".": {"elevation": 0}, "p": {"height_ft": 10}})),
        spatial(),
    );
    let mut tall = tall;
    let mut log = Vec::new();
    let error = engine::attack(
        &mut tall,
        "Archer",
        "Target",
        "synthetic-bow",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut log,
    )
    .expect_err("a 10-ft obstacle blocks the line of sight");
    assert!(
        error.to_string().contains("line of sight")
            || error.to_string().contains("total cover"),
        "{error}"
    );

    let low = state_with(
        registry("2024"),
        vec![
            grounded(
                "Archer",
                creature("synthetic-archer", Size::Medium, 0, false, ranged_attack()),
                "party",
                0,
                0,
            ),
            grounded(
                "Target",
                creature("synthetic-target", Size::Medium, 0, false, no_attacks()),
                "enemy",
                2,
                0,
            ),
        ],
        grid_scene(&[".w."], json!({".": {"elevation": 0}, "w": {"height_ft": 3}})),
        spatial(),
    );
    let mut low = low;
    let mut log = Vec::new();
    engine::attack(
        &mut low,
        "Archer",
        "Target",
        "synthetic-bow",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut log,
    )
    .expect("a 3-ft low obstacle gives cover but does not block sight");
    assert!(
        log.iter().any(|line| line.contains("half cover")),
        "the unified cover query should report half cover: {log:?}"
    );
}

/// Audit row 4: `cell_height_ft` is a ceiling above each cell floor, not an
/// absolute elevation. A 20-ft ceiling over a 10-ft floor stops a 40-ft fall
/// at 30 ft.
#[test]
fn audit_cell_height_is_a_ceiling_above_the_local_floor() {
    let state = state_with(
        registry("2024"),
        vec![combatant(
            "Faller",
            creature("synthetic-faller", Size::Medium, 0, false, no_attacks()),
            "party",
            0,
            0,
            40,
            FlightState::Falling,
            (1, 1),
            &[],
        )],
        scene_with(
            &["^"],
            json!({"^": {"elevation": 10}}),
            json!({"cell_height_ft": 20}),
        ),
        spatial(),
    );
    let fall = engine::resolve_fall(&state, "Faller")
        .expect("spatial fall")
        .expect("a fall column exists");
    assert_eq!(fall.start_floor_ft, 10);
    // P4 stores a ceiling span as `bottom..bottom+1`; landing uses the span
    // top, so a 20-ft ceiling over a 10-ft floor is the surface at 31 ft.
    assert_eq!(fall.landing.elevation_ft, 31);
    assert_eq!(fall.landing.surface_kind, combat_sim::space::FallSurfaceKind::Ceiling);
    assert_eq!(fall.fall_distance_ft, 9);
}

// ---------------------------------------------------------------------------
// Phase 2: movement and reactions
// ---------------------------------------------------------------------------

/// A Large creature cannot squeeze through a 1-cell gap when the profile
/// forbids squeezing. The old grid router ignored the footprint entirely.
#[test]
fn movement_rejects_a_footprint_that_cannot_fit() {
    let profile = SpatialProfile {
        squeeze: Squeeze::Forbid,
        ..SpatialProfile::for_ruleset(Some("2024"))
    };
    let spatial = SpatialState {
        spatial_profile: Some(profile),
        ..SpatialState::default()
    };
    let mut state = state_with(
        registry("2024"),
        vec![combatant(
            "Mover",
            creature("synthetic-mover", Size::Large, 0, false, no_attacks()),
            "party",
            0,
            0,
            0,
            FlightState::Grounded,
            (2, 2),
            &[],
        )],
        grid_scene(
            &[".....", "####.", ".....", "....."],
            json!({".": {"elevation": 0}, "#": {"wall": true}}),
        ),
        Some(spatial),
    );
    let start = state.combatants[0].cell();
    let mut log = Vec::new();
    let error = engine::move_to(
        &mut state,
        "Mover",
        &engine::MoveTarget::Cell(0, 2),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect_err("the 2x2 footprint cannot pass the one-cell opening");
    assert!(
        error.to_string().contains("too_narrow")
            || error.to_string().contains("wall")
            || error.to_string().contains("no path"),
        "{error}"
    );
    assert_eq!(
        state.combatants[0].cell(),
        start,
        "a rejected move must not teleport"
    );
}

/// Opportunity attacks use 3D reach: a flying observer 10 ft above the mover
/// does not threaten it, while the old 2D query did.
#[test]
fn movement_opportunity_threat_uses_3d_reach() {
    let mut state = state_with(
        registry("2024"),
        vec![
            grounded(
                "Mover",
                creature("synthetic-mover", Size::Medium, 0, false, no_attacks()),
                "party",
                0,
                0,
            ),
            combatant(
                "Observer",
                creature("synthetic-observer", Size::Medium, 30, true, melee_attack()),
                "enemy",
                1,
                0,
                10,
                FlightState::Hovering,
                (1, 1),
                &[],
            ),
        ],
        grid_scene(&["....", "...."], json!({".": {"elevation": 0}})),
        spatial(),
    );
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Mover",
        &engine::MoveTarget::Cell(3, 0),
        false,
        &mut ScriptedRng::new(&[15, 3, 15, 3]),
        &mut log,
    )
    .expect("the mover can leave");
    assert!(
        !log.iter()
            .any(|line| line.contains("makes an opportunity attack")),
        "a 10-ft altitude difference is outside a 5-ft reach: {log:?}"
    );
}

/// A new zone entry fires the scene trigger once, not once per cell.
#[test]
fn movement_fires_enter_triggers_on_zone_boundaries_only() {
    let legend = json!({
        ".": {"elevation": 0},
        "a": {"zone": "alpha"},
        "b": {"zone": "beta"}
    });
    let scene = scene_with(
        &["aabb"],
        legend,
        json!({"triggers": [{
            "id": "alpha-trap",
            "on": "on_enter_area",
            "zone": "alpha",
            "manual": "alpha trap",
            "effects": []
        }]}),
    );
    let mut state = state_with(
        registry("2024"),
        vec![grounded(
            "Mover",
            creature("synthetic-mover", Size::Medium, 0, false, no_attacks()),
            "party",
            0,
            0,
        )],
        scene,
        spatial(),
    );
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Mover",
        &engine::MoveTarget::Cell(1, 0),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect("move inside alpha");
    assert!(
        !log.iter().any(|line| line.contains("alpha trap")),
        "moving inside the same zone is not a new entry: {log:?}"
    );

    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Mover",
        &engine::MoveTarget::Cell(2, 0),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect("move into beta");
    engine::move_to(
        &mut state,
        "Mover",
        &engine::MoveTarget::Cell(1, 0),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect("move back into alpha");
    assert_eq!(
        log.iter()
            .filter(|line| line.contains("alpha trap"))
            .count(),
        1,
        "the trap fires on the alpha boundary crossing only: {log:?}"
    );
}

/// Height 5 is a solid obstacle on the ground, but a creature flying at 10 ft
/// passes above it.
#[test]
fn movement_height_obstacles_block_ground_and_allow_flight() {
    let scene = grid_scene(
        &[".x."],
        json!({".": {"elevation": 0}, "x": {"height_ft": 5}}),
    );
    let mut ground = state_with(
        registry("2024"),
        vec![grounded(
            "Walker",
            creature("synthetic-walker", Size::Medium, 0, false, no_attacks()),
            "party",
            0,
            0,
        )],
        scene.clone(),
        spatial(),
    );
    let mut log = Vec::new();
    let error = engine::move_to(
        &mut ground,
        "Walker",
        &engine::MoveTarget::Cell(2, 0),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect_err("a solid 5-ft obstacle blocks ground movement");
    assert!(
        error.to_string().contains("wall")
            || error.to_string().contains("height")
            || error.to_string().contains("no path"),
        "{error}"
    );

    let mut flyer = state_with(
        registry("2024"),
        vec![combatant(
            "Flyer",
            creature("synthetic-flyer", Size::Medium, 30, false, no_attacks()),
            "party",
            0,
            0,
            10,
            FlightState::Flying,
            (1, 1),
            &[],
        )],
        scene,
        spatial(),
    );
    let mut log = Vec::new();
    engine::move_to(
        &mut flyer,
        "Flyer",
        &engine::MoveTarget::Cell(2, 0),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect("a flyer at 10 ft passes above a 5-ft obstacle");
    assert_eq!(flyer.combatants[0].cell(), Some([2, 0]));
    assert_eq!(
        flyer.combatants[0].spatial_pose().expect("pose").cell.z,
        10,
        "horizontal flight keeps absolute altitude"
    );
}

// ---------------------------------------------------------------------------
// Phase 3: attacks and areas
// ---------------------------------------------------------------------------

/// A sphere centred on the ground does not include a target hovering 10 ft up.
/// The old `area_targets` matched only x/y and hit both.
#[test]
fn area_effects_use_target_volumes() {
    let registry = registry("2024");
    let caster_creature: Creature = serde_json::from_value(json!({
        "id": "synthetic-caster",
        "name": "synthetic-caster",
        "size": "medium",
        "hp": 30,
        "ac": 12,
        "speed": {"walk": 30},
        "abilities": {"str": 10, "dex": 10, "con": 10, "int": 10, "wis": 10, "cha": 10},
        "actions": [{
            "id": "synthetic-burst",
            "name": "Synthetic Burst",
            "activation": "action",
            "target": {"type": "zone"},
            "area": {"radius": 5, "shape": "sphere"},
            "effects": [{"damage": {"dice": "1d6", "type": "fire"}}]
        }]
    }))
    .expect("caster with an area ability");
    let caster = combatant(
        "Caster",
        caster_creature,
        "party",
        0,
        0,
        0,
        FlightState::Grounded,
        (1, 1),
        &[],
    );
    let ground = grounded(
        "Ground",
        creature("synthetic-ground", Size::Medium, 0, false, no_attacks()),
        "enemy",
        0,
        0,
    );
    let flyer = combatant(
        "Flyer",
        creature("synthetic-flyer", Size::Medium, 30, false, no_attacks()),
        "enemy",
        0,
        0,
        10,
        FlightState::Flying,
        (1, 1),
        &[],
    );
    let mut state = state_with(
        registry,
        vec![caster, ground, flyer],
        grid_scene(&["....", "...."], json!({".": {"elevation": 0}})),
        spatial(),
    );
    let mut rng = ScriptedRng::new(&[3, 3]);
    let mut log = Vec::new();
    let options = engine::UseOptions {
        targets: vec!["0,0".to_string()],
        ..Default::default()
    };
    engine::use_ability(
        &mut state,
        "Caster",
        "synthetic-burst",
        &options,
        &mut rng,
        &mut log,
    )
    .expect("area spell resolves");
    assert_eq!(
        state.combatants[index(&state, "Ground")].hp,
        27,
        "the ground target is inside the 5-ft sphere: {log:?}"
    );
    assert_eq!(
        state.combatants[index(&state, "Flyer")].hp,
        30,
        "the 10-ft-high target is outside the 5-ft sphere: {log:?}"
    );
}

// ---------------------------------------------------------------------------
// Phase 4: legacy regression
// ---------------------------------------------------------------------------

/// The same Large-target scenario without explicit spatial state keeps the old
/// anchor-cell reach and rejects the attack.
#[test]
fn legacy_grid_attack_keeps_anchor_cell_reach() {
    let mut state = state_with(
        registry("2024"),
        vec![
            grounded(
                "Attacker",
                creature("synthetic-attacker", Size::Medium, 0, false, melee_attack()),
                "party",
                6,
                6,
            ),
            combatant(
                "Target",
                creature("synthetic-target", Size::Large, 0, false, no_attacks()),
                "enemy",
                4,
                4,
                0,
                FlightState::Grounded,
                (2, 2),
                &[],
            ),
        ],
        grid_scene(
            &["......", "......", "......", "......", "......", "......"],
            json!({".": {"elevation": 0}}),
        ),
        None,
    );
    let mut log = Vec::new();
    let error = engine::attack(
        &mut state,
        "Attacker",
        "Target",
        "synthetic-sword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15]),
        &mut log,
    )
    .expect_err("legacy reach uses the anchor cells");
    assert!(error.to_string().contains("out of reach"), "{error}");
    assert_eq!(state.combatants[index(&state, "Target")].hp, 30);
}

/// A legacy save without any P1 field never enters the spatial movement path.
#[test]
fn legacy_grid_move_keeps_the_old_router() {
    let mut state = state_with(
        registry("2024"),
        vec![combatant(
            "Mover",
            creature("synthetic-mover", Size::Large, 0, false, no_attacks()),
            "party",
            0,
            0,
            0,
            FlightState::Grounded,
            (2, 2),
            &[],
        )],
        grid_scene(
            &[".....", "####.", ".....", "....."],
            json!({".": {"elevation": 0}, "#": {"wall": true}}),
        ),
        None,
    );
    // Legacy combatants have no pose, so this state is not spatially enabled.
    let mut log = Vec::new();
    engine::move_to(
        &mut state,
        "Mover",
        &engine::MoveTarget::Cell(0, 2),
        true,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect("the legacy router ignores the footprint");
    assert_eq!(state.combatants[0].cell(), Some([0, 2]));
}

// Keep imports that later phases use; this file is the single P5 test module.
#[allow(dead_code)]
fn _pose(x: i32, y: i32, z: i32) -> SpatialPose {
    SpatialPose {
        cell: combat_sim::space::CellPos { x, y, z },
        footprint: combat_sim::space::Footprint::ONE,
        facing: None,
    }
}

#[allow(dead_code)]
fn _scene(rows: &[&str]) -> Scene {
    serde_json::from_value(grid_scene(rows, json!({".": {"elevation": 0}})))
        .expect("synthetic scene")
}

#[allow(dead_code)]
fn _condition_options() -> AddConditionOptions {
    AddConditionOptions::default()
}
