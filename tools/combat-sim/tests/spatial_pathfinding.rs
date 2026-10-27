//! Movement planning tests for TASK-0088 (#172).
//!
//! These fixtures are synthetic on purpose: real hover/burrow/shape data belongs
//! to TASK-0094 (#178). Every test builds its own movement map so no play
//! content, save file or rulebook data is involved.
//!
//! Contract: `tools/combat-sim/docs/SPATIAL-CONTRACT-v1.md`.
//! Rules matrix: `tools/combat-sim/docs/SPATIAL-RULES-MATRIX.md`.
//!
//! The planner is the P3 seam only; P5 (#174) wires it into engine/AI/CLI.

use serde_json::json;

use combat_sim::content::schema::{Scene, Size};
use combat_sim::space::{
    astar_heuristic_is_admissible, movement_heuristic, CellPos, Diagonal, Distance3d, DoorState,
    GridSpace, MoveBlock, MoveCapabilities, MoveKind, MoveMode, MoveTrigger, MovementCell,
    MovementMap, MovementPath, MovementPlanner, OpportunityObserver, PathAlgorithm, PathPlan,
    PathVerdict, PlanOutcome, Position, Space, SpatialBody, SpatialError, SpatialPose,
    SpatialProfile, Squeeze, TraversalCreature, ZoneGraph,
};

fn cell(x: i32, y: i32, z: i32) -> CellPos {
    CellPos::new(x, y, z).expect("valid cell")
}

fn pose(x: i32, y: i32, z: i32, edge: u32) -> SpatialPose {
    SpatialPose::new(cell(x, y, z), footprint(edge), None).expect("valid pose")
}

fn footprint(edge: u32) -> combat_sim::space::Footprint {
    combat_sim::space::Footprint::new(edge, edge).expect("valid footprint")
}

fn body(id: &str, x: i32, y: i32, z: i32, size: Size) -> SpatialBody {
    SpatialBody::new(id, pose(x, y, z, footprint_for(size)), size)
}

/// A body whose footprint stays 1x1 while its `Size` drives traversal rules.
fn sized_body(id: &str, x: i32, y: i32, z: i32, size: Size) -> SpatialBody {
    SpatialBody::new(id, pose(x, y, z, 1), size)
}

fn footprint_for(size: Size) -> u32 {
    match size {
        Size::Large => 2,
        Size::Huge => 3,
        Size::Gargantuan => 4,
        Size::Tiny | Size::Small | Size::Medium | Size::Unknown => 1,
    }
}

fn p2014() -> SpatialProfile {
    SpatialProfile::for_ruleset(Some("2014"))
}

fn p2024() -> SpatialProfile {
    SpatialProfile::for_ruleset(Some("2024"))
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

fn open_map(width: i32, height: i32) -> MovementMap {
    MovementMap::open(width, height)
}

fn grid_scene(rows: &[&str]) -> Scene {
    serde_json::from_value(json!({
        "id": "synthetic-grid",
        "mode": "grid",
        "grid": {
            "cell": 5,
            "rows": rows,
            "legend": {".": {}, "#": {"wall": true}},
            "zones": {}
        }
    }))
    .expect("grid fixture")
}

fn zone_scene() -> Scene {
    serde_json::from_value(json!({
        "id": "synthetic-zones",
        "mode": "zones",
        "zones": [
            {"id": "a"},
            {"id": "b"},
            {"id": "c"},
            {"id": "d"}
        ],
        "edges": [
            {"a": "a", "b": "b", "distance": 10},
            {"a": "b", "b": "c", "distance": 10},
            {"a": "c", "b": "d", "distance": 10}
        ]
    }))
    .expect("zone fixture")
}

fn reached(plan: &PathPlan) -> &MovementPath {
    match &plan.outcome {
        PlanOutcome::Reached(path) => path,
        other => panic!("expected a reached path, got {other:?}"),
    }
}

fn over_budget(plan: &PathPlan) -> &MovementPath {
    match &plan.outcome {
        PlanOutcome::OverBudget(path) => path,
        other => panic!("expected an over-budget path, got {other:?}"),
    }
}

fn unreachable(plan: &PathPlan) -> (MoveBlock, Vec<String>) {
    match &plan.outcome {
        PlanOutcome::Unreachable { reason, blockers } => (*reason, blockers.clone()),
        other => panic!("expected an unreachable plan, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// AC-1: Dijkstra returns every pose within the speed budget, its cost and a
// reconstructible path.
// ---------------------------------------------------------------------------

#[test]
fn ac1_dijkstra_reachable_set_has_costs_and_reconstructible_paths() {
    let profile = p2024();
    let mover = body("mover", 1, 1, 0, Size::Medium);
    let map = open_map(5, 5);
    let planner = MovementPlanner::new(&map, &profile, &mover);

    let reach = planner.reachable(15).expect("budget");
    assert_eq!(reach.start, mover.pose);
    assert_eq!(reach.budget_ft, 15);
    assert_eq!(
        reach.entries.len(),
        25,
        "all 25 cells are within 15 ft (Chebyshev 3) of (1,1)"
    );
    assert_eq!(reach.entries[0].cost_ft, 0);
    assert!(reach.entries[0].end_allowed);
    assert_eq!(reach.cost_to(&pose(4, 4, 0, 1)), Some(15));
    let path = reach.path_to(&pose(4, 4, 0, 1)).expect("path in set");
    assert_eq!(path.total_cost_ft, 15);
    assert_eq!(path.steps.len(), 3);
    assert!(path.steps.iter().all(|step| step.diagonal));
    assert!(path.steps.iter().all(|step| step.cost_ft == 5));
    assert_eq!(path.steps.last().expect("last step").to, pose(4, 4, 0, 1));

    let mut sorted = reach.entries.clone();
    sorted.sort_by_key(|entry| {
        (
            entry.cost_ft,
            entry.pose.cell.x,
            entry.pose.cell.y,
            entry.pose.cell.z,
            entry.diagonals_parity,
        )
    });
    assert_eq!(sorted, reach.entries, "entries use a stable order");

    let small = planner.reachable(5).expect("budget");
    assert_eq!(small.entries.len(), 9, "start plus eight neighbours");
    assert_eq!(small.entries.iter().filter(|entry| entry.cost_ft == 5).count(), 8);

    for entry in &reach.entries {
        let path = reach.path_to(&entry.pose).expect("entry path");
        assert_eq!(path.total_cost_ft, entry.cost_ft);
        assert_eq!(
            path.steps.iter().map(|step| step.cost_ft).sum::<i32>(),
            entry.cost_ft
        );
    }

    let plan = planner.path(&pose(4, 4, 0, 1), 15).expect("plan");
    assert_eq!(reached(&plan).total_cost_ft, 15);
    let plan = planner.path(&pose(4, 4, 0, 1), 10).expect("plan");
    assert_eq!(over_budget(&plan).total_cost_ft, 15);
}

// ---------------------------------------------------------------------------
// AC-2: A* is used for targeted paths only, and its heuristic is admissible.
// ---------------------------------------------------------------------------

#[test]
fn ac2_astar_is_admissible_and_matches_dijkstra() {
    let profile = p2024();
    let mover = body("mover", 0, 0, 0, Size::Medium);
    let map = open_map(9, 9);
    let planner = MovementPlanner::new(&map, &profile, &mover);
    let goal = pose(8, 8, 0, 1);

    let auto = planner.path(&goal, 100).expect("plan");
    let dijkstra = planner
        .path_with_algorithm(&goal, 100, PathAlgorithm::Dijkstra)
        .expect("plan");
    let astar = planner
        .path_with_algorithm(&goal, 100, PathAlgorithm::AStar)
        .expect("plan");
    assert_eq!(auto.algorithm, PathAlgorithm::AStar);
    assert_eq!(dijkstra.algorithm, PathAlgorithm::Dijkstra);
    assert_eq!(astar.algorithm, PathAlgorithm::AStar);
    let auto_cost = reached(&auto).total_cost_ft;
    assert_eq!(auto_cost, 40);
    assert_eq!(reached(&dijkstra).total_cost_ft, auto_cost);
    assert_eq!(reached(&astar).total_cost_ft, auto_cost);
    assert!(auto.nodes_expanded > 0);
    assert!(astar.nodes_expanded <= dijkstra.nodes_expanded);

    let reach = planner.reachable(40).expect("budget");
    for entry in &reach.entries {
        let h = movement_heuristic(&mover.pose, &entry.pose, &profile);
        assert!(
            h <= entry.cost_ft,
            "heuristic {h} overestimates cost {} at {:?}",
            entry.cost_ft,
            entry.pose.cell
        );
    }
    assert_eq!(movement_heuristic(&mover.pose, &goal, &profile), 40);
    assert!(astar_heuristic_is_admissible(&p2014()));
    assert!(astar_heuristic_is_admissible(&p2024()));

    // 5-10-5: eight diagonals cost 5 + 10 + ... = 60 ft.
    let five_ten_five = SpatialProfile {
        diagonal: Diagonal::FiveTenFive,
        ..profile
    };
    assert!(astar_heuristic_is_admissible(&five_ten_five));
    let planner = MovementPlanner::new(&map, &five_ten_five, &mover);
    let path = reached(&planner.path(&goal, 100).expect("plan"));
    assert_eq!(path.total_cost_ft, 60);
    let reach = planner.reachable(60).expect("budget");
    for entry in &reach.entries {
        let h = movement_heuristic(&mover.pose, &entry.pose, &five_ten_five);
        assert!(h <= entry.cost_ft, "5-10-5 heuristic overestimates");
    }

    // Manhattan 3D treats a diagonal as 10 ft; the heuristic follows it.
    let manhattan = SpatialProfile {
        distance_3d: Distance3d::ManhattanPlusVertical,
        ..profile
    };
    assert!(astar_heuristic_is_admissible(&manhattan));
    let planner = MovementPlanner::new(&map, &manhattan, &mover);
    let path = reached(&planner.path(&goal, 100).expect("plan"));
    assert_eq!(path.total_cost_ft, 80);
    assert_eq!(movement_heuristic(&mover.pose, &goal, &manhattan), 80);
}

// ---------------------------------------------------------------------------
// AC-3: diagonals, corners, walls, doors, difficult terrain, climb/descent,
// height, mode switching and bounds.
// ---------------------------------------------------------------------------

#[test]
fn ac3_diagonal_costs_corners_and_five_ten_five_are_defined() {
    let profile = p2024();
    let mover = body("mover", 0, 0, 0, Size::Medium);
    let map = open_map(6, 6);
    let planner = MovementPlanner::new(&map, &profile, &mover);
    let step = planner.step(&pose(0, 0, 0, 1), &pose(1, 1, 0, 1));
    assert!(step.blocked.is_none());
    let edge = step.edge.as_ref().expect("diagonal edge");
    assert_eq!(edge.cost_ft, 5);
    assert!(edge.diagonal);

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
        assert_eq!(step.blocked, Some(MoveBlock::Wall), "wall at {wall_cell:?}");
        assert!(!step.blockers.is_empty());
    }

    let five = SpatialProfile {
        diagonal: Diagonal::FiveTenFive,
        ..profile
    };
    let planner = MovementPlanner::new(&map, &five, &mover);
    assert_eq!(
        reached(&planner.path(&pose(2, 2, 0, 1), 100).expect("plan")).total_cost_ft,
        15
    );
    assert_eq!(
        reached(&planner.path(&pose(3, 3, 0, 1), 100).expect("plan")).total_cost_ft,
        20
    );
    assert_eq!(
        reached(&planner.path(&pose(4, 4, 0, 1), 100).expect("plan")).total_cost_ft,
        30
    );
    assert_eq!(
        reached(&planner.path(&pose(3, 2, 0, 1), 100).expect("plan")).total_cost_ft,
        20,
        "two diagonals + one straight = 15 + 5"
    );
}

#[test]
fn ac3_walls_doors_bounds_height_and_modes_have_structured_edges() {
    let profile = p2024();
    let mover = body("mover", 0, 0, 0, Size::Medium);

    for (state, expected) in [
        (DoorState::Closed, Some(MoveBlock::Door)),
        (DoorState::Locked, Some(MoveBlock::LockedDoor)),
        (DoorState::Open, None),
    ] {
        let map = map_from(3, 1, |x, _| {
            if x == 1 {
                MovementCell::open().with_door(state)
            } else {
                MovementCell::open()
            }
        });
        let planner = MovementPlanner::new(&map, &profile, &mover);
        let step = planner.step(&pose(0, 0, 0, 1), &pose(1, 0, 0, 1));
        assert_eq!(step.blocked, expected, "door state {state:?}");
        assert_eq!(step.edge.is_some(), expected.is_none());
    }

    let map = open_map(3, 1);
    let planner = MovementPlanner::new(&map, &profile, &mover);
    assert_eq!(
        planner
            .step(&pose(2, 0, 0, 1), &pose(3, 0, 0, 1))
            .blocked,
        Some(MoveBlock::OutOfBounds)
    );
    let map = open_map(3, 2);
    let large = body("large", 1, 0, 0, Size::Large);
    let planner = MovementPlanner::new(&map, &profile, &large);
    assert_eq!(
        planner
            .step(&pose(1, 0, 0, 2), &pose(2, 0, 0, 2))
            .blocked,
        Some(MoveBlock::OutOfBounds)
    );

    let climb_map = map_from(2, 1, |x, _| {
        if x == 0 {
            MovementCell::open()
        } else {
            MovementCell::open().with_elevation(5)
        }
    });
    let planner = MovementPlanner::new(&climb_map, &profile, &mover);
    let climb = planner.step(&pose(0, 0, 0, 1), &pose(1, 0, 5, 1));
    let edge = climb.edge.as_ref().expect("climb allowed");
    assert_eq!(edge.mode, MoveMode::Climb);
    assert_eq!(edge.elevation_change_ft, 5);
    assert_eq!(edge.cost_ft, 10);
    assert!(edge.triggers.contains(&MoveTrigger::Climb {
        from: cell(0, 0, 0),
        to: cell(1, 0, 5),
        elevation_change_ft: 5,
    }));

    let steep = map_from(2, 1, |x, _| {
        if x == 0 {
            MovementCell::open()
        } else {
            MovementCell::open().with_elevation(10)
        }
    });
    let planner = MovementPlanner::new(&steep, &profile, &mover);
    assert_eq!(
        planner
            .step(&pose(0, 0, 0, 1), &pose(1, 0, 10, 1))
            .blocked,
        Some(MoveBlock::TooSteep)
    );
    let drop_map = map_from(2, 1, |x, _| {
        if x == 0 {
            MovementCell::open().with_elevation(10)
        } else {
            MovementCell::open()
        }
    });
    let planner = MovementPlanner::new(&drop_map, &profile, &mover);
    assert_eq!(
        planner
            .step(&pose(0, 0, 10, 1), &pose(1, 0, 0, 1))
            .blocked,
        Some(MoveBlock::Drop)
    );

    let water_map = map_from(2, 1, |x, _| {
        if x == 0 {
            MovementCell::open()
        } else {
            MovementCell::water()
        }
    });
    let planner = MovementPlanner::new(&water_map, &profile, &mover);
    let swim = planner.step(&pose(0, 0, 0, 1), &pose(1, 0, 0, 1));
    assert_eq!(swim.edge.as_ref().expect("swim allowed").mode, MoveMode::Swim);
    assert_eq!(swim.edge.as_ref().expect("swim allowed").cost_ft, 10);

    let planner = MovementPlanner::new(&water_map, &profile, &mover)
        .with_capabilities(MoveCapabilities::default().with_swim(30).with_climb(30));
    assert_eq!(
        planner
            .step(&pose(0, 0, 0, 1), &pose(1, 0, 0, 1))
            .edge
            .as_ref()
            .expect("swim allowed")
            .cost_ft,
        5
    );
}

#[test]
fn ac3_mode_switches_are_per_edge_and_terrain_flags_are_reported() {
    let profile = p2024();
    let mover = body("mover", 0, 0, 0, Size::Medium);
    let map = map_from(5, 1, |x, _| match x {
        0 => MovementCell::open(),
        1 => MovementCell::difficult(),
        2 => MovementCell::open(),
        3 => MovementCell::open().with_elevation(5),
        _ => MovementCell::water().with_elevation(5),
    });
    let planner = MovementPlanner::new(&map, &profile, &mover);
    let path = reached(&planner.path(&pose(4, 0, 5, 1), 100).expect("plan"));
    assert_eq!(path.total_cost_ft, 35);
    assert_eq!(path.steps[0].mode, MoveMode::Walk);
    assert!(path.steps[0].difficult);
    assert_eq!(path.steps[1].mode, MoveMode::Walk);
    assert!(!path.steps[1].difficult);
    assert_eq!(path.steps[2].mode, MoveMode::Climb);
    assert_eq!(path.steps[2].elevation_change_ft, 5);
    assert_eq!(path.steps[3].mode, MoveMode::Swim);
    assert_eq!(path.steps[3].elevation_change_ft, 0);
}

// ---------------------------------------------------------------------------
// AC-4: the whole footprint and its swept volume are checked; squeeze depends
// on the edition profile.
// ---------------------------------------------------------------------------

#[test]
fn ac4_full_footprint_and_squeeze_follow_the_edition_profile() {
    let mover = body("large", 0, 1, 0, Size::Large);
    let squeeze_map = map_from(6, 3, |_, y| {
        if y == 2 {
            MovementCell::wall()
        } else {
            MovementCell::open()
        }
    });
    let goal = pose(4, 1, 0, 2);

    let p14 = MovementPlanner::new(&squeeze_map, &p2014(), &mover);
    assert!(
        p14.can_end_at(&mover.pose).allowed,
        "a squeezed start is legal"
    );
    let path = reached(&p14.path(&goal, 100).expect("plan"));
    assert_eq!(path.steps.len(), 4);
    assert!(path.steps.iter().all(|step| step.squeezed));
    assert!(path.steps.iter().all(|step| step.cost_ft == 10));
    assert!(path.steps.iter().all(|step| step
        .triggers
        .contains(&MoveTrigger::Squeeze {
            at: step.to.cell,
            disadvantage: true,
        })));

    let p24 = MovementPlanner::new(&squeeze_map, &p2024(), &mover);
    let path = reached(&p24.path(&goal, 100).expect("plan"));
    assert!(path.steps.iter().all(|step| step.squeezed));
    assert!(path.steps.iter().all(|step| step.cost_ft == 10));
    assert!(path.steps.iter().all(|step| step
        .triggers
        .iter()
        .any(|trigger| matches!(trigger, MoveTrigger::Squeeze { disadvantage: false, .. }))));

    let forbid = MovementPlanner::new(
        &squeeze_map,
        &SpatialProfile {
            squeeze: Squeeze::Forbid,
            ..p2024()
        },
        &mover,
    );
    assert_eq!(
        forbid.can_end_at(&mover.pose).reason,
        Some(MoveBlock::TooNarrow)
    );
    assert_eq!(
        unreachable(&forbid.path(&goal, 100).expect("plan")).0,
        MoveBlock::TooNarrow
    );

    let wide = map_from(6, 3, |_, _| MovementCell::open());
    let planner = MovementPlanner::new(&wide, &p2024(), &mover);
    let path = reached(&planner.path(&goal, 100).expect("plan"));
    assert_eq!(path.total_cost_ft, 20);
    assert!(path.steps.iter().all(|step| !step.squeezed));
}

#[test]
fn ac4_squeeze_cannot_pass_through_impassable_cells_or_diagonally() {
    let mover = body("large", 0, 1, 0, Size::Large);
    let map = map_from(6, 3, |x, y| {
        if y != 1 {
            MovementCell::wall()
        } else if x == 3 {
            MovementCell::wall()
        } else {
            MovementCell::open()
        }
    });
    let planner = MovementPlanner::new(&map, &p2014(), &mover);
    let (reason, blockers) = unreachable(&planner.path(&pose(4, 1, 0, 2), 100).expect("plan"));
    assert_eq!(reason, MoveBlock::Wall);
    assert!(
        blockers.iter().any(|blocker| blocker.contains("3,1")),
        "{blockers:?}"
    );

    let diag = planner.step(&pose(0, 1, 0, 2), &pose(1, 2, 0, 2));
    assert_eq!(diag.blocked, Some(MoveBlock::TooNarrow));

    // A Huge creature may squeeze through a 2-row corridor (one size smaller),
    // but not through a 1-row corridor.
    let huge = body("huge", 0, 1, 0, Size::Huge);
    let two_rows = map_from(6, 4, |_, y| {
        if y == 1 || y == 2 {
            MovementCell::open()
        } else {
            MovementCell::wall()
        }
    });
    let planner = MovementPlanner::new(&two_rows, &p2014(), &huge);
    assert!(planner.can_end_at(&huge.pose).allowed);
    let path = reached(&planner.path(&pose(3, 1, 0, 3), 100).expect("plan"));
    assert!(path.steps.iter().all(|step| step.squeezed && step.cost_ft == 10));

    let one_row = map_from(6, 3, |_, y| {
        if y == 1 {
            MovementCell::open()
        } else {
            MovementCell::wall()
        }
    });
    let planner = MovementPlanner::new(&one_row, &p2014(), &huge);
    assert!(!planner.can_end_at(&huge.pose).allowed);
}

// ---------------------------------------------------------------------------
// AC-5: paths carry cost, mode, terrain/height and trigger points; no path,
// over-budget and mid-block are structured results.
// ---------------------------------------------------------------------------

#[test]
fn ac5_paths_report_cost_mode_height_triggers_and_failures() {
    let profile = p2024();
    let mover = body("mover", 0, 0, 0, Size::Medium);
    let observer = OpportunityObserver::new(body("sentinel", 0, 1, 0, Size::Medium), 5);
    let observers = [observer];
    let map = map_from(5, 2, |x, y| match (x, y) {
        (0, 0) => MovementCell::open(),
        (1, 0) => MovementCell::difficult(),
        (2, 0) => MovementCell::open(),
        (3, 0) => MovementCell::open().with_elevation(5),
        (4, 0) => MovementCell::water().with_elevation(5),
        (0, 1) | (1, 1) | (2, 1) | (3, 1) | (4, 1) => MovementCell::open(),
        _ => MovementCell::wall(),
    });
    let planner = MovementPlanner::new(&map, &profile, &mover).with_observers(&observers);
    let path = reached(&planner.path(&pose(4, 0, 5, 1), 100).expect("plan"));
    assert_eq!(path.total_cost_ft, 35);
    assert_eq!(path.steps.len(), 4);
    assert!(path.steps[0]
        .triggers
        .contains(&MoveTrigger::AreaEntry { at: cell(1, 0, 0) }));
    assert!(path.steps[1].triggers.iter().any(
        |trigger| matches!(trigger, MoveTrigger::LeavesReach { observer, .. } if observer == "sentinel")
    ));
    assert!(path.steps[2].triggers.contains(&MoveTrigger::Climb {
        from: cell(2, 0, 0),
        to: cell(3, 0, 5),
        elevation_change_ft: 5,
    }));
    assert!(path.steps[3]
        .triggers
        .contains(&MoveTrigger::AreaEntry { at: cell(4, 0, 5) }));

    let plan = planner.path(&pose(4, 0, 5, 1), 30).expect("plan");
    assert_eq!(over_budget(&plan).total_cost_ft, 35);

    let walled = map_from(5, 1, |x, _| {
        if x == 2 {
            MovementCell::wall()
        } else {
            MovementCell::open()
        }
    });
    let walled_planner = MovementPlanner::new(&walled, &profile, &mover);
    let plan = walled_planner.path(&pose(4, 0, 0, 1), 100).expect("plan");
    let (reason, blockers) = unreachable(&plan);
    assert_eq!(reason, MoveBlock::Wall);
    assert!(
        blockers.iter().any(|blocker| blocker.contains("2,0")),
        "{blockers:?}"
    );

    let check = walled_planner.verify_path(path, 100);
    match check.verdict {
        PathVerdict::Blocked {
            step_index,
            reason,
            ..
        } => {
            assert_eq!(step_index, 1);
            assert_eq!(reason, MoveBlock::Wall);
        }
        other => panic!("expected a blocked step, got {other:?}"),
    }
    assert_eq!(check.checked_steps, 2);
}

// ---------------------------------------------------------------------------
// AC-6: voluntary and forced movement are separated.
// ---------------------------------------------------------------------------

#[test]
fn ac6_forced_movement_does_not_consume_speed_or_provoke() {
    let voluntary = MoveKind::Voluntary.rules();
    let forced = MoveKind::Forced.rules();
    assert!(voluntary.consumes_speed);
    assert!(voluntary.provokes_opportunity_attacks);
    assert!(voluntary.triggers_area_effects);
    assert!(!forced.consumes_speed);
    assert!(!forced.provokes_opportunity_attacks);
    assert!(forced.triggers_area_effects);

    let profile = p2024();
    let mover = body("mover", 0, 0, 0, Size::Medium);
    let observer = OpportunityObserver::new(body("sentinel", 0, 1, 0, Size::Medium), 5);
    let observers = [observer];
    let map = map_from(5, 2, |_, _| MovementCell::open());
    let planner = MovementPlanner::new(&map, &profile, &mover).with_observers(&observers);

    let voluntary_path = reached(&planner.path(&pose(4, 0, 0, 1), 100).expect("plan"));
    assert!(voluntary_path.steps.iter().any(|step| step
        .triggers
        .iter()
        .any(|trigger| matches!(trigger, MoveTrigger::LeavesReach { .. }))));

    let push = planner.push((1, 0), 20).expect("push");
    assert_eq!(push.steps.len(), 4);
    assert_eq!(push.moved_ft, 20);
    assert_eq!(push.stopped_by, None);
    assert!(!push.rules.consumes_speed);
    assert!(!push.rules.provokes_opportunity_attacks);
    assert!(push.rules.triggers_area_effects);
    assert!(!push.steps.iter().any(|step| step
        .triggers
        .iter()
        .any(|trigger| matches!(trigger, MoveTrigger::LeavesReach { .. }))));
    assert!(push.steps.iter().all(|step| step
        .triggers
        .iter()
        .any(|trigger| matches!(trigger, MoveTrigger::AreaEntry { .. }))));

    let walled = map_from(5, 1, |x, _| {
        if x == 2 {
            MovementCell::wall()
        } else {
            MovementCell::open()
        }
    });
    let planner = MovementPlanner::new(&walled, &profile, &mover);
    let push = planner.push((1, 0), 20).expect("push");
    assert_eq!(push.moved_ft, 5);
    assert_eq!(push.stopped_by, Some(MoveBlock::Wall));

    let hostile = TraversalCreature::hostile(body("orc", 1, 0, 0, Size::Medium));
    let occupants = [hostile];
    let open = open_map(5, 1);
    let planner = MovementPlanner::new(&open, &profile, &mover).with_occupants(&occupants);
    let push = planner.push((1, 0), 20).expect("push");
    assert_eq!(push.moved_ft, 0);
    assert_eq!(push.stopped_by, Some(MoveBlock::Occupied));

    let ally = TraversalCreature::ally(body("ally", 2, 0, 0, Size::Medium));
    let occupants = [ally];
    let planner = MovementPlanner::new(&open, &profile, &mover).with_occupants(&occupants);
    let push = planner.push((1, 0), 20).expect("push");
    assert_eq!(push.moved_ft, 20);
    assert_eq!(push.stopped_by, None);
    let short = planner.push((1, 0), 10).expect("push");
    assert_eq!(short.moved_ft, 5);
    assert_eq!(short.stopped_by, Some(MoveBlock::EndOccupied));
}

// ---------------------------------------------------------------------------
// AC-7: legacy Zone and Grid movement is unchanged; the new planner is opt-in.
// ---------------------------------------------------------------------------

#[test]
fn ac7_legacy_zone_and_grid_movement_is_unchanged_and_the_new_planner_is_opt_in() {
    let scene = grid_scene(&[".....", ".....", "..#..", ".....", "....."]);
    let grid = GridSpace::new(&scene).expect("grid");
    assert_eq!(
        grid.travel_distance(&Position::cell(0, 0), &Position::cell(4, 0)),
        Some(20)
    );
    let route = grid
        .move_route(&Position::cell(0, 0), &Position::cell(0, 2), &[])
        .expect("route");
    assert_eq!(route.len(), 2);
    assert_eq!(route[0].position, Position::cell(0, 1));
    assert_eq!(route[1].position, Position::cell(0, 2));
    assert_eq!(route.iter().map(|step| step.cost).sum::<i32>(), 10);

    let zones = zone_scene();
    let graph = ZoneGraph::new(&zones);
    assert_eq!(
        graph.travel_distance(&Position::zone("a"), &Position::zone("c")),
        Some(20)
    );
    let route = graph
        .move_route(&Position::zone("a"), &Position::zone("c"), &[])
        .expect("route");
    assert_eq!(route.len(), 2);
    assert_eq!(route.iter().map(|step| step.cost).sum::<i32>(), 20);

    // Legacy grid movement treats a token as one cell; the new footprint and
    // squeeze semantics are only reachable through the explicit planner.
    let legacy = GridSpace::new(&grid_scene(&["...."])).expect("grid");
    assert_eq!(
        legacy.travel_distance(&Position::cell(0, 0), &Position::cell(3, 0)),
        Some(15)
    );
    let corridor = map_from(4, 3, |_, y| {
        if y == 1 {
            MovementCell::open()
        } else {
            MovementCell::wall()
        }
    });
    let large = body("large", 0, 1, 0, Size::Large);
    let forbid = SpatialProfile {
        squeeze: Squeeze::Forbid,
        ..p2024()
    };
    let planner = MovementPlanner::new(&corridor, &forbid, &large);
    assert_eq!(
        unreachable(&planner.path(&pose(2, 1, 0, 2), 100).expect("plan")).0,
        MoveBlock::TooNarrow
    );
}

// ---------------------------------------------------------------------------
// AC-9: creature traversal (ally / Incapacitated / Tiny / size difference),
// creature-space cost and legal end positions.
// ---------------------------------------------------------------------------

#[test]
fn ac9_creature_traversal_and_end_positions_follow_the_matrix() {
    let mover = body("mover", 0, 0, 0, Size::Medium);
    let map = open_map(5, 1);
    let goal = pose(4, 0, 0, 1);

    let cases = [
        (
            "ally",
            TraversalCreature::ally(sized_body("other", 2, 0, 0, Size::Medium)),
            true,
        ),
        (
            "incapacitated hostile",
            TraversalCreature::new(sized_body("other", 2, 0, 0, Size::Medium), false, true),
            true,
        ),
        (
            "tiny hostile",
            TraversalCreature::hostile(sized_body("other", 2, 0, 0, Size::Tiny)),
            true,
        ),
        (
            "two sizes larger hostile",
            TraversalCreature::hostile(sized_body("other", 2, 0, 0, Size::Huge)),
            true,
        ),
        (
            "same size hostile",
            TraversalCreature::hostile(sized_body("other", 2, 0, 0, Size::Medium)),
            false,
        ),
        (
            "one size smaller hostile",
            TraversalCreature::hostile(sized_body("other", 2, 0, 0, Size::Small)),
            false,
        ),
    ];
    for (label, creature, allowed) in cases {
        let occupants = [creature];
        for profile in [p2014(), p2024()] {
            let planner =
                MovementPlanner::new(&map, &profile, &mover).with_occupants(&occupants);
            let plan = planner.path(&goal, 100).expect("plan");
            if allowed {
                let path = reached(&plan);
                let through = path
                    .steps
                    .iter()
                    .find(|step| step.creature_space)
                    .expect("a creature-space step");
                assert_eq!(
                    through.cost_ft, 10,
                    "{label}: creature space is Difficult Terrain"
                );
            } else {
                let (reason, blockers) = unreachable(&plan);
                assert_eq!(reason, MoveBlock::Occupied, "{label}");
                assert!(
                    blockers.iter().any(|blocker| blocker == "other"),
                    "{label}: {blockers:?}"
                );
            }
        }
    }

    let ally = TraversalCreature::ally(sized_body("ally", 2, 0, 0, Size::Medium));
    let occupants = [ally];
    let planner = MovementPlanner::new(&map, &p2024(), &mover).with_occupants(&occupants);
    assert_eq!(
        planner.can_end_at(&pose(2, 0, 0, 1)).reason,
        Some(MoveBlock::EndOccupied)
    );
    assert_eq!(
        unreachable(&planner.path(&pose(2, 0, 0, 1), 100).expect("plan")).0,
        MoveBlock::EndOccupied
    );

    let tiny = TraversalCreature::hostile(sized_body("tiny", 2, 0, 0, Size::Tiny));
    let occupants = [tiny];
    let planner = MovementPlanner::new(&map, &p2024(), &mover).with_occupants(&occupants);
    assert!(
        planner.can_end_at(&pose(2, 0, 0, 1)).allowed,
        "Medium may end in a Tiny creature's space"
    );

    let tiny_mover = body("tiny-mover", 0, 0, 0, Size::Tiny);
    let tiny_other = TraversalCreature::hostile(sized_body("tiny-other", 2, 0, 0, Size::Tiny));
    let occupants = [tiny_other];
    let planner =
        MovementPlanner::new(&map, &p2024(), &tiny_mover).with_occupants(&occupants);
    assert_eq!(
        planner.can_end_at(&pose(2, 0, 0, 1)).reason,
        Some(MoveBlock::EndOccupied)
    );
    let sharing = SpatialProfile {
        tiny_sharing: true,
        ..p2024()
    };
    let planner = MovementPlanner::new(&map, &sharing, &tiny_mover).with_occupants(&occupants);
    assert!(planner.can_end_at(&pose(2, 0, 0, 1)).allowed);
}

// ---------------------------------------------------------------------------
// AC-10: deterministic ordering and structured terminal outcomes.
// ---------------------------------------------------------------------------

#[test]
fn ac10_results_are_deterministic_and_structured() {
    let profile = p2014();
    let mover = body("mover", 0, 0, 0, Size::Medium);
    let map = map_from(6, 4, |x, y| {
        if x == 3 && y != 3 {
            MovementCell::wall()
        } else {
            MovementCell::open()
        }
    });
    let a = TraversalCreature::ally(sized_body("ally", 2, 0, 0, Size::Medium));
    let b = TraversalCreature::hostile(sized_body("orc", 2, 1, 0, Size::Medium));
    let first = [a.clone(), b.clone()];
    let second = [b, a];
    let planner1 = MovementPlanner::new(&map, &profile, &mover).with_occupants(&first);
    let planner2 = MovementPlanner::new(&map, &profile, &mover).with_occupants(&second);
    let set1 = planner1.reachable(20).expect("set");
    let set2 = planner2.reachable(20).expect("set");
    assert_eq!(set1.entries, set2.entries);
    let goal = pose(5, 3, 0, 1);
    let plan1 = planner1.path(&goal, 100).expect("plan");
    let plan2 = planner2.path(&goal, 100).expect("plan");
    assert_eq!(plan1.outcome, plan2.outcome);
    assert_eq!(plan1.nodes_expanded, plan2.nodes_expanded);

    let again = planner1.path(&goal, 100).expect("plan");
    assert_eq!(plan1.outcome, again.outcome);
    let set_again = planner1.reachable(20).expect("set");
    assert_eq!(set1.entries, set_again.entries);

    assert!(matches!(plan1.outcome, PlanOutcome::Reached(_)));
    let short = planner1.path(&goal, 5).expect("plan");
    assert!(matches!(short.outcome, PlanOutcome::OverBudget(_)));
    let blocked = planner1.path(&pose(3, 0, 0, 1), 100).expect("plan");
    assert_eq!(unreachable(&blocked).0, MoveBlock::Wall);
}

#[test]
fn ac10_invalid_inputs_return_structured_errors() {
    assert_eq!(
        MovementMap::new(2, 2, vec![MovementCell::open()]).unwrap_err(),
        SpatialError::InvalidMovementMap {
            width: 2,
            height: 2,
            cells: 1
        }
    );
    let map = open_map(2, 2);
    let mover = body("mover", 0, 0, 0, Size::Medium);
    let planner = MovementPlanner::new(&map, &p2024(), &mover);
    assert_eq!(
        planner.reachable(-1).unwrap_err(),
        SpatialError::InvalidBudget { budget_ft: -1 }
    );
    assert_eq!(
        planner.push((0, 0), 5).unwrap_err(),
        SpatialError::InvalidDirection { dx: 0, dy: 0 }
    );
    assert_eq!(
        planner.path(&pose(1, 1, 0, 2), 5).unwrap_err(),
        SpatialError::PoseMismatch {
            expected: footprint(1),
            actual: footprint(2),
        }
    );
}
