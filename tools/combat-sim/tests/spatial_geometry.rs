//! Pure spatial-query kernel tests for TASK-0087 (#171).
//!
//! These fixtures are synthetic on purpose: the real shapes and creature data
//! belong to TASK-0094 (#178). Every test builds its own grid/zone scene so no
//! play content, save file or rulebook data is involved.
//!
//! Contract: `tools/combat-sim/docs/SPATIAL-CONTRACT-v1.md`.
//! Rules matrix: `tools/combat-sim/docs/SPATIAL-RULES-MATRIX.md`.

use serde_json::json;

use combat_sim::content::schema::{Cover, Scene, Size};
use combat_sim::space::{
    area_cells, area_intersection, body_distance, can_occupy, can_reach, can_reach_bodies,
    can_reach_with_sight, cover_between, line_of_sight, range_distance, travel_step_cost,
    vertical_distance, AoeShapes, AreaShape, AreaSpecV1, BlockerKind, CellPos, Diagonal,
    Distance3d, DistancePolicy, Facing, Footprint, GridBounds, GridSpace, LineOfSight,
    OccupancyReason, Position, Space, SpatialBlocker, SpatialBody, SpatialError, SpatialPose,
    SpatialProfile, Squeeze, TerrainVolume, TravelBlock, TravelTerrain, ZoneGraph, ZoneQuery,
    MAX_AREA_FEET,
};

fn cell(x: i32, y: i32, z: i32) -> CellPos {
    CellPos::new(x, y, z).expect("valid cell")
}

fn footprint(width: u32, depth: u32) -> Footprint {
    Footprint::new(width, depth).expect("valid footprint")
}

fn pose(x: i32, y: i32, z: i32, edge: u32, facing: Option<Facing>) -> SpatialPose {
    SpatialPose::new(cell(x, y, z), footprint(edge, edge), facing).expect("valid pose")
}

fn body(id: &str, x: i32, y: i32, z: i32, size: Size) -> SpatialBody {
    let edge = Footprint::for_size(size).width;
    SpatialBody::new(id, pose(x, y, z, edge, None), size)
}

fn body_facing(id: &str, x: i32, y: i32, z: i32, size: Size, facing: Facing) -> SpatialBody {
    let edge = Footprint::for_size(size).width;
    SpatialBody::new(id, pose(x, y, z, edge, Some(facing)), size)
}

fn p2014() -> SpatialProfile {
    SpatialProfile::for_ruleset(Some("2014"))
}

fn p2024() -> SpatialProfile {
    SpatialProfile::for_ruleset(Some("2024"))
}

fn wall(id: &str, x: i32, y: i32, width: u32, depth: u32, height_ft: i32) -> SpatialBlocker {
    SpatialBlocker::wall(id, cell(x, y, 0), footprint(width, depth), height_ft).expect("valid wall")
}

fn low_wall(
    id: &str,
    x: i32,
    y: i32,
    width: u32,
    depth: u32,
    height_ft: i32,
    cover: Cover,
) -> SpatialBlocker {
    SpatialBlocker::low_wall(id, cell(x, y, 0), footprint(width, depth), height_ft)
        .expect("valid low wall")
        .with_cover(cover)
}

fn zone_scene() -> Scene {
    serde_json::from_value(json!({
        "id": "synthetic-zones",
        "mode": "zones",
        "zones": [
            {"id": "a"},
            {"id": "b"},
            {"id": "c", "cover": "half"},
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

fn sphere(size_ft: i32) -> AreaSpecV1 {
    AreaSpecV1::new(AreaShape::Sphere, size_ft, None).expect("valid sphere")
}

// ---------------------------------------------------------------------------
// AC-1: range, reach, travel, vertical and zone distance are separate choices.
// ---------------------------------------------------------------------------

#[test]
fn ac1_distance_policies_are_explicit_and_do_not_mix() {
    let a = body("a", 0, 0, 0, Size::Medium);
    let b = body("b", 3, 4, 0, Size::Medium);
    let profile = p2024();

    // Range distance is Chebyshev cells x 5 ft.
    let range = range_distance(&a.pose, &b.pose, &profile);
    assert_eq!(range.policy, DistancePolicy::Range);
    assert_eq!(range.feet, 20);
    assert_eq!(range.squares, 4);

    // Vertical distance is its own query.
    let flying = body("flying", 0, 0, 15, Size::Medium);
    let vertical = vertical_distance(&a.pose, &flying.pose);
    assert_eq!(vertical.policy, DistancePolicy::Vertical);
    assert_eq!(vertical.feet, 15);
    assert_eq!(vertical.squares, 0);

    // 3D range = ground distance + altitude difference.
    assert_eq!(
        range_distance(&a.pose, &flying.pose, &profile).feet,
        15,
        "0 ground cells + 15 ft altitude"
    );

    // A diagonal is one square by default, or 5-10-5 when the profile asks.
    let diagonal = body("diagonal", 3, 3, 0, Size::Medium);
    assert_eq!(range_distance(&a.pose, &diagonal.pose, &profile).feet, 15);
    let five_ten_five = SpatialProfile {
        diagonal: Diagonal::FiveTenFive,
        ..profile
    };
    assert_eq!(
        range_distance(&a.pose, &diagonal.pose, &five_ten_five).feet,
        20,
        "three diagonals cost 5 + 10 + 5"
    );

    // ManhattanPlusVertical is a distinct 3D policy.
    let manhattan = SpatialProfile {
        distance_3d: Distance3d::ManhattanPlusVertical,
        ..profile
    };
    let m = body("m", 2, 3, 7, Size::Medium);
    assert_eq!(range_distance(&a.pose, &m.pose, &manhattan).feet, 32);

    // Travel is a step-cost query, not the range query.
    let step_target = body("step", 1, 1, 0, Size::Medium);
    let step = travel_step_cost(
        &a.pose,
        &step_target.pose,
        &profile,
        TravelTerrain::default(),
    );
    assert_eq!(step.distance.policy, DistancePolicy::TravelStep);
    assert_eq!(step.distance.feet, 5);
    assert!(step.blocked.is_none());

    let difficult = travel_step_cost(
        &a.pose,
        &step_target.pose,
        &profile,
        TravelTerrain {
            difficult: true,
            ..TravelTerrain::default()
        },
    );
    assert_eq!(difficult.distance.feet, 10);
    assert_ne!(
        difficult.distance.feet,
        range_distance(&a.pose, &step_target.pose, &profile).feet,
        "travel applies terrain; range does not"
    );

    let climbing = travel_step_cost(
        &a.pose,
        &step_target.pose,
        &profile,
        TravelTerrain {
            climb_swim_crawl: true,
            ..TravelTerrain::default()
        },
    );
    assert_eq!(climbing.distance.feet, 10);

    // A 2014-size squeeze doubles the step; 2024 leaves it to Difficult
    // Terrain, so the caller must set `difficult` explicitly.
    let squeeze = TravelTerrain {
        narrow_opening: true,
        ..TravelTerrain::default()
    };
    assert_eq!(
        travel_step_cost(&a.pose, &step_target.pose, &p2014(), squeeze)
            .distance
            .feet,
        10
    );
    assert_eq!(
        travel_step_cost(&a.pose, &step_target.pose, &p2024(), squeeze)
            .distance
            .feet,
        5
    );

    let forbid = SpatialProfile {
        squeeze: Squeeze::Forbid,
        ..profile
    };
    let blocked = travel_step_cost(&a.pose, &step_target.pose, &forbid, squeeze);
    assert_eq!(blocked.blocked, Some(TravelBlock::TooNarrow));
    assert_eq!(blocked.distance.feet, 0);
}

// ---------------------------------------------------------------------------
// AC-2: queries use footprint and volume, not one centre point.
// ---------------------------------------------------------------------------

#[test]
fn ac2_occupancy_uses_the_whole_footprint_and_volume() {
    let bounds = GridBounds {
        width: 10,
        height: 10,
    };
    let large = body("large", 2, 2, 0, Size::Large);
    let allowed = can_occupy(&large, Some(bounds), &[], &[], &p2024()).expect("query");
    assert!(allowed.allowed);
    assert_eq!(allowed.reason, None);

    // A 1x1 post at the footprint's corner blocks the whole 2x2 body. A
    // centre-only check would say the square (2,2) is free.
    let post = TerrainVolume::new("post", cell(3, 3, 0), Footprint::ONE, 10).expect("terrain");
    let blocked = can_occupy(
        &large,
        Some(bounds),
        std::slice::from_ref(&post),
        &[],
        &p2024(),
    )
    .expect("query");
    assert!(!blocked.allowed);
    assert_eq!(blocked.reason, Some(OccupancyReason::Terrain));
    assert_eq!(blocked.blockers, vec!["post".to_string()]);

    // The same post does not block a body whose volume starts above it.
    let flying = body("flying", 2, 2, 10, Size::Large);
    assert!(
        can_occupy(&flying, Some(bounds), &[post], &[], &p2024())
            .expect("query")
            .allowed
    );

    // A body on the map edge fits; one cell past it does not.
    let edge = body("edge", 8, 8, 0, Size::Large);
    assert!(
        can_occupy(&edge, Some(bounds), &[], &[], &p2024())
            .expect("query")
            .allowed
    );
    let corner = body("corner", 9, 9, 0, Size::Large);
    let outside = can_occupy(&corner, Some(bounds), &[], &[], &p2024()).expect("query");
    assert!(!outside.allowed);
    assert_eq!(outside.reason, Some(OccupancyReason::OutOfBounds));
}

#[test]
fn ac2_reach_uses_the_nearest_footprint_cell_for_a_large_body() {
    let source = body("source", 10, 10, 0, Size::Medium);
    let large = body("large", 3, 3, 0, Size::Large);
    // Anchor-to-anchor distance is 7 cells (35 ft); the nearest 2x2 cell is
    // 6 cells away (30 ft).
    assert_eq!(range_distance(&source.pose, &large.pose, &p2024()).feet, 35);
    assert_eq!(body_distance(&source, &large, &p2024()).feet, 30);
    assert!(!can_reach(&source.pose, &large.pose, 30, &p2024()).reachable);
    assert!(can_reach_bodies(&source, &large, 30, &p2024()).reachable);
}

#[test]
fn ac2_cover_uses_the_target_footprint_not_only_its_centre() {
    // A medium target directly behind the low wall has half cover.
    let source = body("source", 0, 4, 0, Size::Medium);
    let medium = body("medium", 4, 4, 0, Size::Medium);
    let low = low_wall("low", 2, 4, 1, 1, 3, Cover::Half);
    assert_eq!(
        cover_between(&source, &medium, std::slice::from_ref(&low), &p2024()).degree,
        Cover::Half
    );

    // A Gargantuan target peeks around the wall with one 4x4 footprint cell
    // that has no cover. The kernel reports the footprint, not the centre.
    let gargantuan = body("gargantuan", 4, 2, 0, Size::Gargantuan);
    let cover = cover_between(&source, &gargantuan, &[low], &p2024());
    assert_eq!(cover.degree, Cover::None, "{cover:?}");
}

// ---------------------------------------------------------------------------
// AC-3: LOS handles height, walls, low obstacles and multiple paths.
// ---------------------------------------------------------------------------

#[test]
fn ac3_line_of_sight_handles_walls_low_obstacles_and_height() {
    let source = body("source", 0, 4, 0, Size::Medium);
    let target = body("target", 4, 4, 0, Size::Medium);
    let high = wall("wall", 2, 4, 1, 1, 10);

    let blocked = line_of_sight(&source, &target, std::slice::from_ref(&high), &p2024());
    assert!(!blocked.clear);
    assert_eq!(blocked.blocked_by, vec!["wall".to_string()]);
    assert!(blocked
        .explain
        .iter()
        .any(|note| note.code == "los-blocked"));

    // A low wall gives cover without blocking sight: the line passes above it.
    let low = low_wall("low", 2, 4, 1, 1, 3, Cover::Half);
    let clear = line_of_sight(&source, &target, std::slice::from_ref(&low), &p2024());
    assert!(clear.clear);
    assert!(clear.blocked_by.is_empty());

    // Altitude changes the answer: the same 10-ft wall does not block a
    // creature flying at 20 ft, but a 20-ft wall does.
    let flying = body("flying", 4, 4, 20, Size::Medium);
    assert!(
        line_of_sight(&source, &flying, std::slice::from_ref(&high), &p2024()).clear,
        "eye line from 5 ft to 25 ft passes over a 10-ft wall"
    );
    let tall = wall("tall", 2, 4, 1, 1, 20);
    assert!(!line_of_sight(&source, &flying, &[tall], &p2024()).clear);
}

#[test]
fn ac3_line_of_sight_evaluates_multiple_paths_deterministically() {
    let source = body("source", 0, 4, 0, Size::Medium);
    // The wall covers y = 0..3 but leaves a gap at y = 4.
    let wall_block = wall("block", 2, 0, 1, 4, 10);
    let large = body("large", 4, 2, 0, Size::Large);
    let clear = line_of_sight(&source, &large, std::slice::from_ref(&wall_block), &p2024());
    assert!(
        clear.clear,
        "one 2x2 footprint cell sees around the wall: {clear:?}"
    );
    assert!(clear
        .explain
        .iter()
        .any(|note| note.code == "los-candidate"));

    // A medium target in the same place is wholly behind the wall.
    let medium = body("medium", 4, 2, 0, Size::Medium);
    assert!(
        !line_of_sight(
            &source,
            &medium,
            std::slice::from_ref(&wall_block),
            &p2024()
        )
        .clear
    );

    // Blocker order never changes the answer or the reported id order.
    let a = wall("zeta", 2, 3, 1, 1, 10);
    let b = wall("alpha", 2, 3, 1, 1, 10);
    let medium_center = body("center", 4, 3, 0, Size::Medium);
    let first = line_of_sight(&source, &medium_center, &[a.clone(), b.clone()], &p2024());
    let second = line_of_sight(&source, &medium_center, &[b, a], &p2024());
    assert_eq!(first, second);
    assert_eq!(
        first.blocked_by,
        vec!["alpha".to_string(), "zeta".to_string()]
    );

    // No blockers is the documented "missing data" default: clear sight.
    assert!(line_of_sight(&source, &medium_center, &[], &p2024()).clear);
}

// ---------------------------------------------------------------------------
// AC-4: area shapes cover the P1 schema, with explicit unsupported errors.
// ---------------------------------------------------------------------------

#[test]
fn ac4_area_shapes_are_cells_in_the_shape_and_are_sorted() {
    let origin = pose(5, 5, 0, 1, None);

    let ball = area_cells(&sphere(10), &origin, &p2024()).expect("sphere");
    assert_eq!(ball.shape, AreaShape::Sphere);
    assert!(ball.cells.contains(&cell(5, 5, 0)), "origin included");
    assert!(ball.cells.contains(&cell(5, 7, 0)), "10 ft edge included");
    assert!(!ball.cells.contains(&cell(5, 8, 0)), "15 ft excluded");
    assert!(ball.cells.windows(2).all(|pair| pair[0] < pair[1]));

    let cone_origin = body_facing("cone", 5, 5, 0, Size::Medium, Facing::North);
    let cone = area_cells(
        &AreaSpecV1::new(AreaShape::Cone, 15, None).expect("cone"),
        &cone_origin.pose,
        &p2024(),
    )
    .expect("cone");
    assert!(!cone.cells.contains(&cell(5, 5, 0)), "origin excluded");
    assert!(cone.cells.contains(&cell(5, 4, 0)));
    assert!(cone.cells.contains(&cell(4, 2, 0)));
    assert!(cone.cells.contains(&cell(6, 2, 0)));
    assert!(
        !cone.cells.contains(&cell(3, 2, 0)),
        "far edge is 3 cells wide, not 5"
    );

    let cube_origin = body_facing("cube", 5, 5, 0, Size::Medium, Facing::East);
    let cube = area_cells(
        &AreaSpecV1::new(AreaShape::Cube, 15, None).expect("cube"),
        &cube_origin.pose,
        &p2024(),
    )
    .expect("cube");
    assert!(cube.cells.contains(&cell(5, 5, 0)), "origin on a face");
    assert!(cube.cells.contains(&cell(7, 6, 10)));
    assert!(!cube.cells.contains(&cell(8, 5, 0)));
    assert!(!cube.cells.contains(&cell(5, 5, 15)));

    let cylinder = area_cells(
        &AreaSpecV1::new(AreaShape::Cylinder, 10, Some(10)).expect("cylinder"),
        &origin,
        &p2024(),
    )
    .expect("cylinder");
    assert!(cylinder.cells.contains(&cell(5, 5, 0)));
    assert!(cylinder.cells.contains(&cell(5, 7, 0)));
    assert!(cylinder.cells.contains(&cell(5, 5, 5)));
    assert!(!cylinder.cells.contains(&cell(5, 8, 0)));
    assert!(!cylinder.cells.contains(&cell(5, 5, 10)));

    // Missing width falls back to documented defaults: height = size for a
    // cylinder, one cell for a line.
    let default_cylinder = area_cells(
        &AreaSpecV1::new(AreaShape::Cylinder, 10, None).expect("cylinder"),
        &origin,
        &p2024(),
    )
    .expect("cylinder");
    assert!(default_cylinder.cells.contains(&cell(5, 5, 5)));
    assert!(!default_cylinder.cells.contains(&cell(5, 5, 10)));

    let line_origin = body_facing("line", 5, 5, 0, Size::Medium, Facing::East);
    let line = area_cells(
        &AreaSpecV1::new(AreaShape::Line, 15, Some(5)).expect("line"),
        &line_origin.pose,
        &p2024(),
    )
    .expect("line");
    assert!(!line.cells.contains(&cell(5, 5, 0)), "origin excluded");
    assert!(line.cells.contains(&cell(6, 5, 0)));
    assert!(line.cells.contains(&cell(8, 5, 0)));
    assert!(!line.cells.contains(&cell(9, 5, 0)));

    let thin_line = area_cells(
        &AreaSpecV1::new(AreaShape::Line, 15, None).expect("line"),
        &line_origin.pose,
        &p2024(),
    )
    .expect("line");
    assert!(thin_line.cells.contains(&cell(6, 5, 0)));
    assert!(
        !thin_line.cells.contains(&cell(6, 6, 0)),
        "missing line width defaults to one cell"
    );

    let emanation = area_cells(
        &AreaSpecV1::new(AreaShape::Emanation, 10, None).expect("emanation"),
        &origin,
        &p2024(),
    )
    .expect("emanation");
    assert!(!emanation.cells.contains(&cell(5, 5, 0)), "origin excluded");
    assert!(emanation.cells.contains(&cell(5, 6, 0)));

    // 2014 has no Emanation; the profile rejects it before any cells exist.
    let error = area_cells(
        &AreaSpecV1::new(AreaShape::Emanation, 10, None).expect("emanation"),
        &origin,
        &p2014(),
    )
    .expect_err("2014 rejects emanation");
    assert_eq!(
        error,
        SpatialError::UnsupportedShape {
            shape: AreaShape::Emanation,
            shapes: AoeShapes::Five
        }
    );
}

#[test]
fn ac4_area_intersection_handles_partial_targets_and_volume_bounds() {
    let ball = area_cells(&sphere(10), &pose(5, 5, 0, 1, None), &p2024()).expect("sphere");
    let caster = body("caster", 5, 5, 0, Size::Medium);

    // A Large 2x2 target has one cell in the 10-ft sphere: partial, not full.
    let large = body("large", 6, 6, 0, Size::Large);
    let partial = area_intersection(&ball, &caster, &large, &[]);
    assert!(partial.intersects);
    assert!(!partial.fully_covered);
    assert_eq!(partial.target_cells, 4);
    assert_eq!(partial.covered_cells.len(), 1);
    assert!(!partial.total_cover_blocked);

    // A Medium target wholly inside the sphere is fully covered.
    let medium = body("medium", 5, 6, 0, Size::Medium);
    let full = area_intersection(&ball, &caster, &medium, &[]);
    assert!(full.intersects);
    assert!(full.fully_covered);
    assert_eq!(full.covered_cells.len(), 1);

    // Volume bounds: a ground sphere does not reach a target 20 ft up.
    let flying = body("flying", 5, 5, 20, Size::Medium);
    assert!(!area_intersection(&ball, &caster, &flying, &[]).intersects);

    // The same target at 5 ft is inside the sphere's 3D volume.
    let hover = body("hover", 5, 5, 5, Size::Medium);
    assert!(area_intersection(&ball, &caster, &hover, &[]).intersects);

    // Total cover blocks the area cell; half/three-quarters cover do not.
    let near = area_cells(&sphere(10), &pose(2, 4, 0, 1, None), &p2024()).expect("sphere");
    let near_caster = body("near-caster", 2, 4, 0, Size::Medium);
    let target = body("target", 4, 4, 0, Size::Medium);
    let total = wall("total", 3, 4, 1, 1, 10);
    let blocked = area_intersection(&near, &near_caster, &target, &[total]);
    assert!(blocked.total_cover_blocked);
    assert!(!blocked.intersects);
    assert!(blocked.covered_cells.is_empty());

    let half = low_wall("half", 3, 4, 1, 1, 3, Cover::Half);
    assert!(area_intersection(&near, &near_caster, &target, &[half]).intersects);
}

#[test]
fn ac4_area_validation_rejects_invalid_and_unsupported_shapes() {
    let origin = pose(0, 0, 0, 1, None);

    let zero = AreaSpecV1 {
        shape: AreaShape::Sphere,
        size_ft: 0,
        width_ft: None,
    };
    assert_eq!(
        area_cells(&zero, &origin, &p2024()).expect_err("zero radius"),
        SpatialError::InvalidArea {
            size_ft: 0,
            width_ft: None
        }
    );

    let huge = AreaSpecV1 {
        shape: AreaShape::Sphere,
        size_ft: MAX_AREA_FEET + 1,
        width_ft: None,
    };
    assert_eq!(
        area_cells(&huge, &origin, &p2024()).expect_err("huge area"),
        SpatialError::AreaTooLarge {
            size_ft: MAX_AREA_FEET + 1,
            limit_ft: MAX_AREA_FEET
        }
    );

    // Cones, lines and cubes need a facing; missing data is an error, not a
    // silent downward default.
    let facingless = pose(0, 0, 0, 1, None);
    let cone = AreaSpecV1::new(AreaShape::Cone, 15, None).expect("cone");
    assert_eq!(
        area_cells(&cone, &facingless, &p2024()).expect_err("missing facing"),
        SpatialError::MissingAreaDirection {
            shape: AreaShape::Cone
        }
    );
}

// ---------------------------------------------------------------------------
// AC-5: large footprints, edges, corners, height and defaults.
// ---------------------------------------------------------------------------

#[test]
fn ac5_occupancy_handles_large_footprints_sharing_and_altitude_layers() {
    let medium = body("medium", 2, 2, 0, Size::Medium);
    let other = body("other", 2, 2, 0, Size::Medium);

    let blocked =
        can_occupy(&medium, None, &[], std::slice::from_ref(&other), &p2024()).expect("query");
    assert!(!blocked.allowed);
    assert_eq!(blocked.reason, Some(OccupancyReason::Occupied));
    assert_eq!(blocked.blockers, vec!["other".to_string()]);

    // A bigger creature may end on a smaller one's cell (contract O-05).
    let huge = body("huge", 2, 2, 0, Size::Huge);
    assert!(
        can_occupy(&huge, None, &[], std::slice::from_ref(&other), &p2024())
            .expect("query")
            .allowed
    );

    // Tiny sharing is off by default; only an explicit profile may turn it on.
    let tiny_a = body("tiny-a", 3, 3, 0, Size::Tiny);
    let tiny_b = body("tiny-b", 3, 3, 0, Size::Tiny);
    assert!(
        !can_occupy(&tiny_a, None, &[], std::slice::from_ref(&tiny_b), &p2024())
            .expect("query")
            .allowed
    );
    let sharing = SpatialProfile {
        tiny_sharing: true,
        ..p2024()
    };
    assert!(
        can_occupy(&tiny_a, None, &[], std::slice::from_ref(&tiny_b), &sharing)
            .expect("query")
            .allowed
    );

    // Ground and flying creatures in the same column do not share a volume.
    let flying = body("flying", 2, 2, 10, Size::Medium);
    assert!(
        can_occupy(&medium, None, &[], std::slice::from_ref(&flying), &p2024())
            .expect("query")
            .allowed
    );

    // Missing bounds/terrain/occupants is a valid empty field.
    assert!(
        can_occupy(&medium, None, &[], &[], &SpatialProfile::default())
            .expect("query")
            .allowed
    );

    // Invalid body data is rejected, not defaulted.
    let mut invalid = body("invalid", 0, 0, 0, Size::Medium);
    invalid.height_ft = 0;
    assert_eq!(
        can_occupy(&invalid, None, &[], &[], &p2024()),
        Err(SpatialError::InvalidHeight { height_ft: 0 })
    );
}

// ---------------------------------------------------------------------------
// AC-6: 2014 and 2024 differences only through the profile.
// ---------------------------------------------------------------------------

#[test]
fn ac6_profiles_switch_only_documented_edition_rules() {
    let p14 = p2014();
    let p24 = p2024();

    assert_eq!(p24.diagonal, Diagonal::Five);
    assert!(!p24.facing, "no house-rule facing in the 2024 profile");
    assert!(!p24.flanking, "no house-rule flanking in the 2024 profile");
    assert!(!p24.tiny_sharing);
    assert_eq!(p24.distance_3d, Distance3d::Chebyshev);
    assert_eq!(p24.squeeze, Squeeze::Edition2024);
    assert_eq!(p24.aoe_shapes, AoeShapes::Six);
    assert_eq!(p14.squeeze, Squeeze::Edition2014);
    assert_eq!(p14.aoe_shapes, AoeShapes::Five);

    // The profile decides whether Emanation exists.
    let emanation = AreaSpecV1::new(AreaShape::Emanation, 10, None).expect("emanation");
    assert_eq!(
        p14.validate_area(&emanation),
        Err(SpatialError::UnsupportedShape {
            shape: AreaShape::Emanation,
            shapes: AoeShapes::Five
        })
    );
    assert!(p24.validate_area(&emanation).is_ok());

    // Squeeze policy: 2014 adds a cost; 2024 treats the opening as Difficult
    // Terrain, which the caller represents with `difficult`.
    let a = body("a", 0, 0, 0, Size::Medium);
    let b = body("b", 1, 1, 0, Size::Medium);
    let narrow = TravelTerrain {
        narrow_opening: true,
        ..TravelTerrain::default()
    };
    assert_eq!(
        travel_step_cost(&a.pose, &b.pose, &p14, narrow)
            .distance
            .feet,
        10
    );
    assert_eq!(
        travel_step_cost(&a.pose, &b.pose, &p24, narrow)
            .distance
            .feet,
        5
    );
    assert_eq!(
        travel_step_cost(
            &a.pose,
            &b.pose,
            &p24,
            TravelTerrain {
                narrow_opening: true,
                difficult: true,
                climb_swim_crawl: false,
            }
        )
        .distance
        .feet,
        10,
        "2024 narrow opening is Difficult Terrain"
    );

    // 2024 applies the opposite-side cover rule to a blocker in the target's
    // own cell; 2014 keeps the legacy target-cell cover.
    let source = body("source", 0, 4, 0, Size::Medium);
    let target = body("target", 4, 4, 0, Size::Medium);
    let crate_blocker = low_wall("crate", 4, 4, 1, 1, 3, Cover::Half);
    assert_eq!(
        cover_between(&source, &target, std::slice::from_ref(&crate_blocker), &p14).degree,
        Cover::Half
    );
    assert_eq!(
        cover_between(&source, &target, std::slice::from_ref(&crate_blocker), &p24).degree,
        Cover::None
    );
}

// ---------------------------------------------------------------------------
// AC-7: legacy zone/grid behaviour is unchanged.
// ---------------------------------------------------------------------------

#[test]
fn ac7_legacy_zone_and_grid_queries_keep_their_contract() {
    let grid = grid_scene(&[".....", "..#..", "....."]);
    let grid_space = GridSpace::new(&grid).expect("grid");
    assert_eq!(
        grid_space.distance(&Position::cell(0, 1), &Position::cell(3, 1)),
        Some(15)
    );
    assert_eq!(
        grid_space.cover(&Position::cell(0, 1), &Position::cell(3, 1)),
        Cover::Total
    );
    assert!(grid_space.can_reach(&Position::cell(0, 1), &Position::cell(3, 1), 15));
    let area = grid_space.area_positions(&Position::cell(1, 1), 5);
    assert!(area.contains(&Position::cell(2, 1)));

    let zones = zone_scene();
    let zone_graph = ZoneGraph::new(&zones);
    assert_eq!(
        zone_graph.distance(&Position::zone("a"), &Position::zone("c")),
        Some(20)
    );
    assert_eq!(
        zone_graph.travel_distance(&Position::zone("a"), &Position::zone("c")),
        Some(20)
    );
    assert_eq!(
        zone_graph.cover(&Position::zone("a"), &Position::zone("c")),
        Cover::Half
    );
    assert!(zone_graph.can_reach(&Position::zone("a"), &Position::zone("b"), 10));
}

// ---------------------------------------------------------------------------
// AC-8: creature cover, reach-vs-sight, and zone approximation boundaries.
// ---------------------------------------------------------------------------

#[test]
fn ac8_creature_cover_and_reach_semantics_are_explicit() {
    let source = body("source", 0, 4, 0, Size::Medium);
    let target = body("target", 4, 4, 0, Size::Medium);
    let ally = body("ally", 2, 4, 0, Size::Medium);
    let creature = SpatialBlocker::creature(&ally, false).expect("creature blocker");

    let cover = cover_between(&source, &target, std::slice::from_ref(&creature), &p2024());
    assert_eq!(cover.degree, Cover::Half);
    assert_eq!(cover.sources, vec!["ally".to_string()]);
    assert_eq!(
        cover
            .explain
            .iter()
            .filter(|n| n.code == "cover-source")
            .count(),
        1
    );

    let raised = SpatialProfile {
        creature_cover: Cover::ThreeQuarters,
        ..p2024()
    };
    assert_eq!(
        cover_between(&source, &target, std::slice::from_ref(&creature), &raised).degree,
        Cover::ThreeQuarters
    );

    // A sight-blocking creature still provides its cover degree when it is
    // the reason LOS fails (no total-cover source exists).
    let blocking_creature = SpatialBlocker::creature(&ally, true).expect("creature blocker");
    assert!(
        !line_of_sight(
            &source,
            &target,
            std::slice::from_ref(&blocking_creature),
            &p2024()
        )
        .clear
    );
    assert_eq!(
        cover_between(
            &source,
            &target,
            std::slice::from_ref(&blocking_creature),
            &p2024()
        )
        .degree,
        Cover::Half
    );

    // A creature behind the target is not between them and grants no cover.
    let behind = body("behind", 6, 4, 0, Size::Medium);
    let behind_blocker = SpatialBlocker::creature(&behind, false).expect("creature blocker");
    assert_eq!(
        cover_between(&source, &target, &[behind_blocker], &p2024()).degree,
        Cover::None
    );

    // Reach is geometric and never checks walls; the combined query does.
    let wall_between = wall("wall", 2, 4, 1, 1, 10);
    let reach = can_reach(&source.pose, &target.pose, 20, &p2024());
    assert!(reach.reachable);
    assert_eq!(reach.distance.policy, DistancePolicy::Range);
    let combined = can_reach_with_sight(
        &source,
        &target,
        20,
        std::slice::from_ref(&wall_between),
        &p2024(),
    );
    assert!(!combined.reachable);
    assert!(!combined.sight.clear);
    assert_eq!(combined.distance_ft, 20);

    let los = line_of_sight(&source, &target, &[wall_between], &p2024());
    assert!(!los.clear);
}

#[test]
fn ac8_zone_graph_queries_are_explicit_and_approximate() {
    let scene = zone_scene();
    let graph = ZoneGraph::new(&scene);
    let query = ZoneQuery::new(&graph);
    let a = Position::zone("a");
    let c = Position::zone("c");

    let distance = query.distance(&a, &c).expect("zone distance");
    assert_eq!(distance.policy, DistancePolicy::ZoneGraph);
    assert_eq!(distance.feet, 20);
    assert_eq!(distance.squares, 0);
    assert_eq!(query.distance(&a, &a).expect("same zone").feet, 0);
    assert!(
        query.distance(&a, &Position::zone("missing")).is_none(),
        "unknown zones have no distance"
    );

    assert!(!query.can_reach(&a, &c, 15).reachable);
    assert!(query.can_reach(&a, &c, 20).reachable);
    assert!(
        query.can_reach(&a, &a, 0).reachable,
        "same zone is 0 ft for reach"
    );

    let los = query.line_of_sight(&a, &c);
    assert!(los.clear);
    assert!(
        los.explain.iter().any(|note| note.code == "zone-los"),
        "zone LOS must say it is a scene-level approximation"
    );

    let cover = query.cover_between(&a, &c);
    assert_eq!(cover.degree, Cover::Half);
    assert_eq!(cover.sources, vec!["c".to_string()]);

    let blocked = query.can_occupy(&a, std::slice::from_ref(&a));
    assert!(!blocked.allowed);
    assert_eq!(blocked.reason, Some(OccupancyReason::Occupied));
    assert!(query.can_occupy(&a, &[]).allowed);
    let unknown = query.can_occupy(&Position::zone("missing"), &[]);
    assert_eq!(unknown.reason, Some(OccupancyReason::OutOfBounds));

    // Sphere/emanation are the only zone-wide approximations.
    let ball = query.area(&a, &sphere(10), &p2024()).expect("zone sphere");
    assert_eq!(ball.zones, vec!["a".to_string(), "b".to_string()]);
    assert!(ball.approximation.contains("zone"));
    assert!(ball.explain.iter().any(|note| note.code == "zone-area"));

    // Graph distance, not just one hop: A -> C is 20 ft through B.
    let wide_ball = query
        .area(&a, &sphere(20), &p2024())
        .expect("wide zone sphere");
    assert_eq!(
        wide_ball.zones,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );

    let emanation = AreaSpecV1::new(AreaShape::Emanation, 10, None).expect("emanation");
    assert_eq!(
        query
            .area(&a, &emanation, &p2014())
            .expect_err("2014 rejects emanation"),
        SpatialError::UnsupportedShape {
            shape: AreaShape::Emanation,
            shapes: AoeShapes::Five
        }
    );
    let aura = query.area(&a, &emanation, &p2024()).expect("2024 aura");
    assert_eq!(aura.zones, vec!["b".to_string()]);

    let cone = AreaSpecV1::new(AreaShape::Cone, 10, None).expect("cone");
    assert_eq!(
        query
            .area(&a, &cone, &p2024())
            .expect_err("zone mode does not fake a cone"),
        SpatialError::UnsupportedInZone {
            shape: AreaShape::Cone
        }
    );

    assert_eq!(
        query
            .area(&Position::zone("missing"), &sphere(10), &p2024())
            .expect_err("missing origin zone"),
        SpatialError::UnknownPosition {
            position: "zone `missing`".to_string()
        }
    );
}

#[test]
fn ac8_blocker_kinds_and_explain_records_are_structured() {
    // Blocker constructors keep the kind and default sight/cover policy.
    let wall_blocker = wall("wall", 1, 1, 1, 1, 10);
    assert_eq!(wall_blocker.kind, BlockerKind::Wall);
    assert_eq!(wall_blocker.cover, Cover::Total);
    assert!(wall_blocker.blocks_sight);

    let low = low_wall("low", 1, 1, 1, 1, 3, Cover::Half);
    assert_eq!(low.kind, BlockerKind::LowObstacle);
    assert_eq!(low.cover, Cover::Half);

    let medium = body("medium", 1, 1, 0, Size::Medium);
    let creature = SpatialBlocker::creature(&medium, true).expect("creature");
    assert_eq!(creature.kind, BlockerKind::Creature);
    assert_eq!(creature.cover, Cover::Half);
    assert!(creature.blocks_sight);

    let terrain = TerrainVolume::new("rock", cell(1, 1, 0), Footprint::ONE, 5).expect("terrain");
    let terrain_blocker = SpatialBlocker::terrain(&terrain, Cover::Half).expect("terrain blocker");
    assert_eq!(terrain_blocker.kind, BlockerKind::Terrain);

    let source = body("source", 0, 1, 0, Size::Medium);
    let target = body("target", 4, 1, 0, Size::Medium);
    let los = line_of_sight(&source, &target, &[wall("wall", 2, 1, 1, 1, 10)], &p2024());
    assert!(los.explain.iter().any(|note| !note.detail.is_empty()));
    assert_eq!(los.blocked_by, vec!["wall".to_string()]);
    let _: LineOfSight = los;
}
