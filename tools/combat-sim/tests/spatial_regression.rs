//! P8 spatial regression matrix (TASK-0093 / Issue #177).
//!
//! The matrix is intentionally synthetic and generic. It is a compact,
//! end-to-end companion to the detailed P2-P5 suites: every row names the
//! contract/matrix rule it protects and exercises the engine or query seam a
//! player actually reaches.
//!
//! Rows:
//! - footprint / size (O-01, O-06)
//! - 3D height and reach (S-06, R-04)
//! - cover (C-01..C-05)
//! - line of sight (C-06, C-07)
//! - areas of effect (A-01..A-08)
//! - corners and narrow passages (S-04, M-06, M-07)
//! - flight and falling (V-02..V-06)
//! - opportunity attacks (R-02..R-04)
//! - legacy saves (contract section 11)
//!
//! Real hover/burrow/shape content is covered by the #178 suites; this file
//! only proves the synthetic matrix stays green.

use std::collections::VecDeque;
use std::path::Path;

use serde_json::{json, Value};

use combat_sim::content::schema::{Creature, Size};
use combat_sim::content::{ContentRegistry, ContentSources};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{self, AttackOptions, FallOptions};
use combat_sim::space::{
    CellPos, Diagonal, FlightState, Footprint, MovementCell, MovementMap, MovementPlanner,
    Position, SpatialBody, SpatialPose, SpatialProfile, Squeeze,
};
use combat_sim::state::{CombatantState, EncounterState, SpatialState, STATE_SCHEMA_VERSION};

// ---------------------------------------------------------------------------
// Synthetic fixtures
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

fn creature(id: &str, size: Size, fly: i32, hover: bool, attacks: Value) -> Creature {
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

fn melee_attack() -> Value {
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

fn ranged_attack() -> Value {
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

fn no_attacks() -> Value {
    json!([])
}

#[allow(clippy::too_many_arguments)]
fn combatant(
    name: &str,
    creature: Creature,
    side: &str,
    x: i32,
    y: i32,
    altitude: i32,
    flight: FlightState,
    footprint: (u32, u32),
) -> CombatantState {
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
        "speed_left": 30
    }))
    .expect("synthetic combatant")
}

fn grounded(name: &str, creature: Creature, side: &str, x: i32, y: i32) -> CombatantState {
    combatant(name, creature, side, x, y, 0, FlightState::Grounded, (1, 1))
}

fn legacy_combatant(name: &str, creature: Creature, side: &str, x: i32, y: i32) -> Value {
    json!({
        "creature": creature,
        "name": name,
        "side": side,
        "max_hp": 30,
        "hp": 30,
        "zone": "open",
        "pos": {"kind": "cell", "x": x, "y": y},
        "speed_left": 30
    })
}

fn state_with(
    registry: ContentRegistry,
    combatants: Vec<CombatantState>,
    scene: Value,
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

fn grid_scene(rows: &[&str], legend: Value) -> Value {
    json!({
        "id": "synthetic-quality-grid",
        "mode": "grid",
        "grid": {"cell": 5, "rows": rows, "legend": legend}
    })
}

fn spatial() -> Option<SpatialState> {
    Some(SpatialState::default())
}

fn index(state: &EncounterState, name: &str) -> usize {
    state.find_index(name).expect("combatant exists")
}

struct ScriptedRng {
    values: VecDeque<u32>,
}

impl ScriptedRng {
    fn new(values: &[u32]) -> Self {
        Self {
            values: values.iter().copied().collect(),
        }
    }
}

impl DiceRng for ScriptedRng {
    fn roll_die(&mut self, _sides: u32) -> u32 {
        self.values
            .pop_front()
            .expect("scripted RNG ran out while rolling")
    }
}

fn pose(x: i32, y: i32, z: i32, edge: u32) -> SpatialPose {
    SpatialPose::new(
        CellPos { x, y, z },
        Footprint::new(edge, edge).expect("valid footprint"),
        None,
    )
    .expect("valid pose")
}

fn body(id: &str, x: i32, y: i32, z: i32, size: Size) -> SpatialBody {
    SpatialBody::new(id, pose(x, y, z, footprint_for(size)), size)
}

fn footprint_for(size: Size) -> u32 {
    match size {
        Size::Large => 2,
        Size::Huge => 3,
        Size::Gargantuan => 4,
        Size::Tiny | Size::Small | Size::Medium | Size::Unknown => 1,
    }
}

fn map_from(width: i32, height: i32, f: impl Fn(i32, i32) -> MovementCell) -> MovementMap {
    let mut cells = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            cells.push(f(x, y));
        }
    }
    MovementMap::new(width, height, cells).expect("valid movement map")
}

// ---------------------------------------------------------------------------
// Matrix rows
// ---------------------------------------------------------------------------

/// O-01/O-06: a footprint is a volume, not an anchor cell. The Large target's
/// near edge is within 5 ft even when its anchor is 10 ft away.
#[test]
fn matrix_size_footprint_uses_the_target_volume() {
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
            ),
        ],
        grid_scene(
            &["......", "......", "......", "......", "......", "......"],
            json!({".": {"elevation": 0}}),
        ),
        spatial(),
    );
    let mut log = Vec::new();
    engine::attack(
        &mut state,
        "Attacker",
        "Target",
        "synthetic-sword",
        &AttackOptions::default(),
        &mut ScriptedRng::new(&[15, 4]),
        &mut log,
    )
    .expect("the Large target's near edge is within 5 ft");
    assert!(
        log.iter().any(|line| line.contains("HIT")),
        "footprint reach should hit: {log:?}"
    );
}

/// S-06/R-04: the third dimension is part of reach. A melee attacker 10 ft
/// above a ground target is out of reach, while 5 ft is within a 5-ft reach.
#[test]
fn matrix_height_changes_melee_reach() {
    let scene = grid_scene(&["....", "...."], json!({".": {"elevation": 0}}));
    let target = grounded(
        "Target",
        creature("synthetic-target", Size::Medium, 0, false, no_attacks()),
        "enemy",
        0,
        0,
    );

    for (altitude, should_hit) in [(10, false), (5, true)] {
        let mut state = state_with(
            registry("2024"),
            vec![
                combatant(
                    "Flyer",
                    creature("synthetic-flyer", Size::Medium, 30, true, melee_attack()),
                    "party",
                    0,
                    0,
                    altitude,
                    FlightState::Hovering,
                    (1, 1),
                ),
                target.clone(),
            ],
            scene.clone(),
            spatial(),
        );
        let mut log = Vec::new();
        let result = engine::attack(
            &mut state,
            "Flyer",
            "Target",
            "synthetic-sword",
            &AttackOptions::default(),
            &mut ScriptedRng::new(&[15, 4]),
            &mut log,
        );
        if should_hit {
            result.expect("5 ft of altitude is within a 5-ft reach");
            assert!(log.iter().any(|line| line.contains("HIT")), "{log:?}");
        } else {
            let error = result.expect_err("10 ft of altitude is outside a 5-ft reach");
            assert!(error.to_string().contains("out of reach"), "{error}");
        }
    }
}

/// C-01..C-05: a low obstacle gives half cover without blocking sight, while a
/// 10-ft obstacle blocks line of sight and cannot be shot through.
#[test]
fn matrix_cover_and_line_of_sight() {
    {
        let mut state = state_with(
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
            grid_scene(
                &[".w."],
                json!({".": {"elevation": 0}, "w": {"height_ft": 3}}),
            ),
            spatial(),
        );
        let mut log = Vec::new();
        engine::attack(
            &mut state,
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

    let mut tall = state_with(
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
        grid_scene(
            &[".W."],
            json!({".": {"elevation": 0}, "W": {"wall": true, "height_ft": 10}}),
        ),
        spatial(),
    );
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
    .expect_err("a 10-ft wall blocks line of sight and gives total cover");
    assert!(
        error.to_string().contains("line of sight") || error.to_string().contains("cover"),
        "{error}"
    );
    assert_eq!(tall.combatants[index(&tall, "Target")].hp, 30);
}

/// A-01/A-08: a sphere uses the target volume in 3D. A ground target is hit;
/// a flyer 10 ft up is outside a 5-ft sphere.
#[test]
fn matrix_area_sphere_uses_target_volume() {
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
    let mut state = state_with(
        registry("2024"),
        vec![
            grounded("Caster", caster_creature, "party", 0, 0),
            grounded(
                "Ground",
                creature("synthetic-ground", Size::Medium, 0, false, no_attacks()),
                "enemy",
                0,
                0,
            ),
            combatant(
                "Flyer",
                creature("synthetic-flyer", Size::Medium, 30, false, no_attacks()),
                "enemy",
                0,
                0,
                10,
                FlightState::Flying,
                (1, 1),
            ),
        ],
        grid_scene(&["....", "...."], json!({".": {"elevation": 0}})),
        spatial(),
    );
    engine::use_ability(
        &mut state,
        "Caster",
        "synthetic-burst",
        &engine::UseOptions {
            targets: vec!["0,0".to_string()],
            ..Default::default()
        },
        &mut ScriptedRng::new(&[3, 3]),
        &mut Vec::new(),
    )
    .expect("area spell resolves");
    assert_eq!(state.combatants[index(&state, "Ground")].hp, 27);
    assert_eq!(
        state.combatants[index(&state, "Flyer")].hp,
        30,
        "the 10-ft-high target is outside the 5-ft sphere"
    );
}

/// A-02/A-07: a cone uses its actual shape, not a sphere. A target in front is
/// hit; a target behind the caster is not.
#[test]
fn matrix_area_cone_excludes_the_space_behind() {
    let caster_creature: Creature = serde_json::from_value(json!({
        "id": "synthetic-cone-caster",
        "name": "synthetic-cone-caster",
        "size": "medium",
        "hp": 30,
        "ac": 12,
        "speed": {"walk": 30},
        "abilities": {"str": 10, "dex": 10, "con": 10, "int": 10, "wis": 10, "cha": 10},
        "actions": [{
            "id": "synthetic-cone",
            "name": "Synthetic Cone",
            "activation": "action",
            "target": {"type": "zone"},
            "area": {"radius": 10, "shape": "cone"},
            "effects": [{"damage": {"dice": "1d6", "type": "fire"}}]
        }]
    }))
    .expect("caster with a cone");
    let mut caster = grounded("Caster", caster_creature, "party", 0, 1);
    caster.pose.as_mut().expect("pose").facing = Some(combat_sim::space::Facing::East);
    let mut state = state_with(
        registry("2024"),
        vec![
            caster,
            grounded(
                "Front",
                creature("synthetic-front", Size::Medium, 0, false, no_attacks()),
                "enemy",
                1,
                1,
            ),
            grounded(
                "Behind",
                creature("synthetic-behind", Size::Medium, 0, false, no_attacks()),
                "enemy",
                0,
                0,
            ),
        ],
        grid_scene(&["...", "...", "..."], json!({".": {"elevation": 0}})),
        spatial(),
    );
    engine::use_ability(
        &mut state,
        "Caster",
        "synthetic-cone",
        &engine::UseOptions {
            targets: vec!["Caster".to_string()],
            ..Default::default()
        },
        &mut ScriptedRng::new(&[3]),
        &mut Vec::new(),
    )
    .expect("cone resolves");
    assert_eq!(state.combatants[index(&state, "Front")].hp, 27);
    assert_eq!(
        state.combatants[index(&state, "Behind")].hp,
        30,
        "a target behind the caster is inside a sphere but outside the cone"
    );
}

/// S-04/M-06/M-07/O-06: diagonals cannot cut a blocked corner, and a Large
/// footprint is handled by the edition squeeze policy rather than teleporting
/// through a one-cell opening.
#[test]
fn matrix_corners_and_narrow_passages() {
    let profile = SpatialProfile::for_ruleset(Some("2024"));
    let mover = body("mover", 0, 0, 0, Size::Medium);
    for wall_cell in [(1, 0), (0, 1)] {
        let map = map_from(3, 3, |x, y| {
            if (x, y) == wall_cell {
                MovementCell::wall()
            } else {
                MovementCell::open()
            }
        });
        let planner = MovementPlanner::new(&map, &profile, &mover);
        let step = planner.step(&pose(0, 0, 0, 1), &pose(1, 1, 0, 1));
        assert!(
            step.blocked.is_some(),
            "a diagonal through a blocked corner must be rejected: {wall_cell:?}"
        );
    }

    let squeeze_map = map_from(6, 3, |_, y| {
        if y == 2 {
            MovementCell::wall()
        } else {
            MovementCell::open()
        }
    });
    let large = body("large", 0, 1, 0, Size::Large);
    let goal = pose(4, 1, 0, 2);

    let p2014 = SpatialProfile::for_ruleset(Some("2014"));
    let planned = MovementPlanner::new(&squeeze_map, &p2014, &large);
    let path = planned.path(&goal, 100).expect("2014 squeeze path plan");
    assert!(
        path.is_reached(),
        "2014 squeezing allows a Large creature through the narrow gap"
    );

    let p2024 = SpatialProfile::for_ruleset(Some("2024"));
    let planned = MovementPlanner::new(&squeeze_map, &p2024, &large);
    let path = planned.path(&goal, 100).expect("2024 narrow-gap path plan");
    assert!(
        path.is_reached(),
        "2024 treats the narrow gap as Difficult Terrain, not an impossible wall"
    );

    let forbid = SpatialProfile {
        squeeze: Squeeze::Forbid,
        ..p2024
    };
    let planned = MovementPlanner::new(&squeeze_map, &forbid, &large);
    let path = planned.path(&goal, 100).expect("forbid path plan");
    assert!(
        !path.is_reached(),
        "Squeeze::Forbid must reject the one-cell opening"
    );
    assert_eq!(path.path().map_or(0, |path| path.steps.len()), 0);
}

/// V-02..V-06: flight respects a ceiling, and a fall resolves distance,
/// damage, grounding and the R-09 landing condition.
#[test]
fn matrix_flight_and_fall() {
    let mut state = state_with(
        registry("2024"),
        vec![combatant(
            "Flyer",
            creature("synthetic-flyer", Size::Medium, 30, false, no_attacks()),
            "party",
            0,
            0,
            0,
            FlightState::Grounded,
            (1, 1),
        )],
        json!({
            "id": "quality-ceiling",
            "mode": "grid",
            "cell_height_ft": 15,
            "grid": {"cell": 5, "rows": ["...."], "legend": {".": {"elevation": 0}}}
        }),
        spatial(),
    );
    engine::fly(&mut state, "Flyer", 30, &mut Vec::new()).expect("take off");
    let vertical = state.combatants[0].vertical.expect("vertical");
    assert_eq!(vertical.flight, FlightState::Flying);
    assert_eq!(
        vertical.altitude_ft, 10,
        "the 15-ft ceiling stops a 5-ft-tall flyer at 10 ft"
    );

    let mut log = Vec::new();
    let report = engine::fall_with_checks(
        &mut state,
        "Flyer",
        FallOptions::default(),
        combat_sim::space::FallAbility::Dexterity,
        &mut ScriptedRng::new(&[9, 4]),
        &mut log,
    )
    .expect("fall resolves")
    .expect("a spatial fall column exists");
    assert_eq!(report.resolution.fall_distance_ft, 10);
    assert!(
        log.iter().any(|line| line.contains("falls 10 ft")),
        "the fall log reports the distance: {log:?}"
    );
    assert_eq!(
        state.combatants[0].vertical.expect("vertical").flight,
        FlightState::Grounded
    );
    assert!(state.combatants[0].hp < 30, "fall damage is applied");
    assert!(
        state.combatants[0]
            .conditions
            .iter()
            .any(|active| active.condition == "prone"),
        "a failed R-09 check lands Prone: {:?}",
        state.combatants[0].conditions
    );
}

/// R-02..R-04: leaving a threatened square provokes, but the same observer
/// 10 ft above does not threaten the ground mover.
#[test]
fn matrix_opportunity_attacks_use_3d_reach() {
    let scene = grid_scene(&["....", "...."], json!({".": {"elevation": 0}}));
    let mover = grounded(
        "Mover",
        creature("synthetic-mover", Size::Medium, 0, false, no_attacks()),
        "party",
        0,
        0,
    );
    let mut adjacent = state_with(
        registry("2024"),
        vec![
            mover.clone(),
            grounded(
                "Observer",
                creature("synthetic-observer", Size::Medium, 0, false, melee_attack()),
                "enemy",
                1,
                0,
            ),
        ],
        scene.clone(),
        spatial(),
    );
    let mut log = Vec::new();
    engine::move_to(
        &mut adjacent,
        "Mover",
        &engine::MoveTarget::Cell(3, 0),
        false,
        &mut ScriptedRng::new(&[15, 3]),
        &mut log,
    )
    .expect("the mover can leave");
    assert!(
        log.iter()
            .any(|line| line.contains("makes an opportunity attack")),
        "a ground observer adjacent at departure threatens the mover: {log:?}"
    );

    let mut elevated = state_with(
        registry("2024"),
        vec![
            mover,
            combatant(
                "Observer",
                creature("synthetic-observer", Size::Medium, 30, true, melee_attack()),
                "enemy",
                1,
                0,
                10,
                FlightState::Hovering,
                (1, 1),
            ),
        ],
        scene,
        spatial(),
    );
    let mut log = Vec::new();
    engine::move_to(
        &mut elevated,
        "Mover",
        &engine::MoveTarget::Cell(3, 0),
        false,
        &mut ScriptedRng::new(&[]),
        &mut log,
    )
    .expect("the mover can leave");
    assert!(
        !log.iter()
            .any(|line| line.contains("makes an opportunity attack")),
        "a 10-ft altitude difference is outside a 5-ft reach: {log:?}"
    );
}

/// Contract section 11: a pre-P1 save has no pose/vertical/spatial fields. It
/// must round-trip without reinterpretation and keep the legacy anchor-cell
/// reach path.
#[test]
fn matrix_legacy_save_round_trip_and_anchor_reach() {
    let attacker = legacy_combatant(
        "Attacker",
        creature("synthetic-attacker", Size::Medium, 0, false, melee_attack()),
        "party",
        6,
        6,
    );
    let target = legacy_combatant(
        "Target",
        creature("synthetic-target", Size::Large, 0, false, no_attacks()),
        "enemy",
        4,
        4,
    );
    let scene = grid_scene(
        &["......", "......", "......", "......", "......", "......"],
        json!({".": {"elevation": 0}}),
    );
    let text = json!({
        "encounter_name": "Legacy Quality Save",
        "content": registry("2024"),
        "scene": scene,
        "combatants": [attacker, target],
        "initiative": [0, 1],
        "round": 2
    })
    .to_string();
    let mut state: EncounterState =
        serde_json::from_str(&text).expect("legacy save should load without migration");
    state.rebuild_caches();
    assert_eq!(state.schema_version, STATE_SCHEMA_VERSION);
    assert!(state.spatial.is_none());
    assert!(state
        .combatants
        .iter()
        .all(|combatant| combatant.pose.is_none()));
    assert!(state
        .combatants
        .iter()
        .all(|combatant| combatant.vertical.is_none()));
    assert_eq!(
        state.combatants[1].position(),
        Position::Cell { x: 4, y: 4 }
    );

    let round_trip = serde_json::to_string(&state).expect("serialize legacy save");
    let reloaded: EncounterState = serde_json::from_str(&round_trip).expect("reload legacy save");
    assert_eq!(
        reloaded.combatants[1].position(),
        Position::Cell { x: 4, y: 4 }
    );
    assert!(reloaded.spatial.is_none());

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
    .expect_err("legacy anchor-cell reach still rejects a 10-ft anchor distance");
    assert!(error.to_string().contains("out of reach"), "{error}");
    assert_eq!(state.combatants[1].hp, 30);
}

/// The profile switches are explicit: the default 2024 environment is
/// 5-ft diagonals, only a profile with 5-10-5 changes that.
#[test]
fn matrix_profile_switch_keeps_diagonal_defaults() {
    let default = SpatialProfile::for_ruleset(Some("2024"));
    assert_eq!(default.diagonal, Diagonal::Five);
    let optional = SpatialProfile {
        diagonal: Diagonal::FiveTenFive,
        ..default
    };
    let mover = body("mover", 0, 0, 0, Size::Medium);
    let map = MovementMap::open(5, 5);
    let planner = MovementPlanner::new(&map, &optional, &mover);
    let path = planner.path(&pose(2, 2, 0, 1), 100).expect("optional path");
    assert_eq!(path.path().expect("reached").total_cost_ft, 15);
}
