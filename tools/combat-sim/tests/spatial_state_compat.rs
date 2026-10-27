//! Spatial state compatibility tests for TASK-0086 (#170).
//!
//! These fixtures are synthetic on purpose: P1 owns types and persistence, not
//! real hover/burrow/AoE content (TASK-0094 owns that). The tests cover the
//! additive contract in `tools/combat-sim/docs/SPATIAL-CONTRACT-v1.md`:
//! legacy `Position`/`Zone` loading, golden state JSON, the schema-version
//! guard, legacy profile semantics and the P4/P5 field inventory.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use combat_sim::content::schema::{
    AreaSpec, Cover, GridCell, Rules, Scene, SceneMode, Size, Speed,
};
use combat_sim::space::{
    AoeShapes, AreaShape, AreaSpecV1, CellPos, Diagonal, Distance3d, Facing, FlightState,
    Footprint, Position, SpatialError, SpatialPose, SpatialProfile, Squeeze, TerrainVolume,
    VerticalState, Volume, MAX_GRID_COORD,
};
use combat_sim::state::{EncounterState, SpatialState, STATE_SCHEMA_VERSION};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Throwaway directory for one test.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "combat-sim-spatial-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Self { dir }
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, text).expect("write fixture");
        path
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Current-format save with an old 2D `Cell`, an old `Zone` and no spatial
/// fields at all. This is the "pre-P1 save" golden fixture.
const LEGACY_STATE_JSON: &str = r#"{
  "encounter_name": "Legacy Cell And Zone",
  "party_zone": "hall",
  "combatants": [
    {
      "creature": {"id": "synthetic-cell", "name": "Synthetic Cell"},
      "name": "Cell One",
      "side": "party",
      "max_hp": 12,
      "hp": 9,
      "zone": "hall",
      "pos": {"kind": "cell", "x": 3, "y": 4}
    },
    {
      "creature": {"id": "synthetic-zone", "name": "Synthetic Zone"},
      "name": "Zone One",
      "side": "enemy",
      "max_hp": 7,
      "hp": 7,
      "zone": "hall"
    }
  ],
  "initiative": [0, 1],
  "round": 2,
  "log": ["legacy save"]
}"#;

/// A mixed encounter: old `pos` plus explicit P1 pose/vertical/footprint,
/// scene-level and state-level spatial settings, and a terrain volume.
const MIXED_STATE_JSON: &str = r#"{
  "schema_version": 1,
  "encounter_name": "Mixed Old And New",
  "combatants": [
    {
      "creature": {"id": "synthetic-mixed", "name": "Synthetic Mixed"},
      "name": "Mixed One",
      "side": "party",
      "max_hp": 10,
      "hp": 10,
      "zone": "hall",
      "pos": {"kind": "cell", "x": 1, "y": 2},
      "pose": {
        "cell": {"x": 1, "y": 2, "z": 10},
        "footprint": {"width": 2, "depth": 2},
        "facing": "north"
      },
      "vertical": {"altitude_ft": 10, "flight": "flying", "fall_distance_ft": 0},
      "footprint": {"width": 2, "depth": 2}
    }
  ],
  "scene": {
    "id": "synthetic-grid",
    "mode": "grid",
    "cell_height_ft": 10,
    "grid": {
      "cell": 5,
      "rows": ["..", ".."],
      "legend": {".": {"height_ft": 4}},
      "zones": {}
    }
  },
  "spatial": {
    "cell_height_ft": 12,
    "terrain_volumes": [
      {
        "id": "ledge",
        "cell": {"x": 5, "y": 5, "z": 0},
        "footprint": {"width": 1, "depth": 2},
        "height_ft": 4
      }
    ],
    "spatial_profile": {
      "diagonal": "five_ten_five",
      "squeeze": "edition_2014",
      "creature_cover": "half",
      "tiny_sharing": false,
      "facing": true,
      "flanking": false,
      "distance_3d": "manhattan_plus_vertical",
      "aoe_shapes": "five"
    }
  }
}"#;

fn load_state(scratch: &Scratch, text: &str) -> (PathBuf, EncounterState) {
    let path = scratch.write("state.json", text);
    let state = EncounterState::load(&path).expect("fixture loads");
    (path, state)
}

fn round_trip(scratch: &Scratch, state: &EncounterState) -> EncounterState {
    let path = scratch.path("round-trip.state.json");
    state.save(&path).expect("state saves");
    EncounterState::load(&path).expect("saved state loads")
}

#[test]
fn ac1_types_have_units_validation_and_stable_serialization() {
    // Grid coordinates are 5-ft cells; z is altitude in feet above the floor.
    let cell = CellPos::new(2, 3, 10).expect("valid cell");
    assert_eq!((cell.x, cell.y, cell.z), (2, 3, 10));
    assert_eq!(
        CellPos::new(0, 0, -1),
        Err(SpatialError::AltitudeOutOfRange { z: -1 })
    );
    assert_eq!(
        CellPos::new(MAX_GRID_COORD + 1, 0, 0),
        Err(SpatialError::CoordinateOutOfRange {
            x: MAX_GRID_COORD + 1,
            y: 0
        })
    );

    // Footprints are in cells; size defaults follow the frozen contract.
    assert_eq!(
        Footprint::for_size(Size::Tiny),
        Footprint { width: 1, depth: 1 }
    );
    assert_eq!(
        Footprint::for_size(Size::Small),
        Footprint { width: 1, depth: 1 }
    );
    assert_eq!(
        Footprint::for_size(Size::Medium),
        Footprint { width: 1, depth: 1 }
    );
    assert_eq!(
        Footprint::for_size(Size::Large),
        Footprint { width: 2, depth: 2 }
    );
    assert_eq!(
        Footprint::for_size(Size::Huge),
        Footprint { width: 3, depth: 3 }
    );
    assert_eq!(
        Footprint::for_size(Size::Gargantuan),
        Footprint { width: 4, depth: 4 }
    );
    assert_eq!(
        Footprint::new(0, 1),
        Err(SpatialError::InvalidFootprint { width: 0, depth: 1 })
    );

    // Volume widths/depths are cells; height is feet. Tiny's 2.5 ft default is
    // rounded up to whole feet for integer storage.
    assert_eq!(Volume::for_size(Size::Tiny).height_ft, 3);
    assert_eq!(Volume::for_size(Size::Medium).height_ft, 5);
    assert_eq!(Volume::for_size(Size::Large).height_ft, 10);
    assert_eq!(Volume::for_size(Size::Huge).height_ft, 15);
    assert_eq!(Volume::for_size(Size::Gargantuan).height_ft, 20);
    assert_eq!(
        Volume::new(1, 1, 0),
        Err(SpatialError::InvalidVolume {
            width: 1,
            depth: 1,
            height_ft: 0
        })
    );

    // A pose anchors a footprint at its top-left cell.
    let pose = SpatialPose::new(
        cell,
        Footprint::new(2, 2).expect("footprint"),
        Some(Facing::North),
    )
    .expect("pose");
    assert_eq!(pose.covers().len(), 4);
    assert_eq!(pose.covers()[0], cell);
    assert_eq!(pose.covers()[3], CellPos { x: 3, y: 4, z: 10 });

    // Vertical state stores altitude, flight mode and pending fall distance.
    let flying = VerticalState::new(10, FlightState::Flying, 0).expect("flying");
    assert_eq!(flying.altitude_ft, 10);
    assert_eq!(
        VerticalState::new(10, FlightState::Grounded, 0),
        Err(SpatialError::InvalidFlightState {
            flight: FlightState::Grounded,
            altitude_ft: 10
        })
    );
    assert_eq!(
        VerticalState::new(-1, FlightState::Flying, 0),
        Err(SpatialError::AltitudeOutOfRange { z: -1 })
    );
    assert_eq!(
        VerticalState::new(10, FlightState::Flying, -1),
        Err(SpatialError::FallDistanceOutOfRange { distance_ft: -1 })
    );

    // Areas are sized in feet; Emanation is 2024-only.
    let emanation = AreaSpecV1::new(AreaShape::Emanation, 10, None).expect("area");
    assert_eq!(
        SpatialProfile::for_ruleset(Some("2014")).validate_area(&emanation),
        Err(SpatialError::UnsupportedShape {
            shape: AreaShape::Emanation,
            shapes: AoeShapes::Five
        })
    );
    assert!(SpatialProfile::for_ruleset(Some("2024"))
        .validate_area(&emanation)
        .is_ok());
    assert_eq!(
        AreaSpecV1::new(AreaShape::Sphere, 0, None),
        Err(SpatialError::InvalidArea {
            size_ft: 0,
            width_ft: None
        })
    );

    // Stable JSON: field names, enum spellings and round-trip.
    assert_eq!(
        serde_json::to_value(pose).expect("pose json"),
        json!({
            "cell": {"x": 2, "y": 3, "z": 10},
            "footprint": {"width": 2, "depth": 2},
            "facing": "north"
        })
    );
    assert_eq!(
        serde_json::from_value::<SpatialPose>(serde_json::to_value(pose).expect("json"))
            .expect("pose back"),
        pose
    );
    assert_eq!(
        serde_json::to_value(FlightState::Hovering).expect("flight"),
        json!("hovering")
    );
    assert_eq!(
        serde_json::to_value(Squeeze::Edition2014).expect("squeeze"),
        json!("edition_2014")
    );
    assert_eq!(
        serde_json::to_value(Squeeze::Forbid).expect("squeeze"),
        json!("forbid")
    );
    assert_eq!(
        serde_json::to_value(AreaShape::Emanation).expect("shape"),
        json!("emanation")
    );

    // Terrains are named, anchored volumes with positive height.
    let terrain = TerrainVolume::new(
        "ledge",
        CellPos::new(5, 5, 0).expect("cell"),
        Footprint::new(1, 2).expect("footprint"),
        4,
    )
    .expect("terrain");
    assert_eq!(terrain.height_ft, 4);
    assert_eq!(
        TerrainVolume::new("", CellPos::default(), Footprint::ONE, 4),
        Err(SpatialError::EmptyTerrainId)
    );
    assert_eq!(
        TerrainVolume::new("ledge", CellPos::default(), Footprint::ONE, 0),
        Err(SpatialError::InvalidHeight { height_ft: 0 })
    );
}

#[test]
fn ac2_legacy_cell_and_zone_positions_load_without_reinterpretation() {
    let scratch = Scratch::new("legacy-positions");
    let (_, state) = load_state(&scratch, LEGACY_STATE_JSON);

    let cell = &state.combatants[0];
    assert_eq!(cell.position(), Position::cell(3, 4));
    assert_eq!(cell.pos, Some(Position::cell(3, 4)));
    let pose = cell.spatial_pose().expect("old cell gets a ground pose");
    assert_eq!(pose.cell, CellPos { x: 3, y: 4, z: 0 });
    assert_eq!(pose.footprint, Footprint::ONE);
    assert_eq!(pose.facing, None);
    assert_eq!(cell.vertical_state(), VerticalState::default());
    assert_eq!(cell.vertical_state().altitude_ft, 0);
    assert_eq!(cell.vertical_state().flight, FlightState::Grounded);

    let zone = &state.combatants[1];
    assert_eq!(zone.position(), Position::zone("hall"));
    assert_eq!(zone.pos, None);
    assert!(
        zone.spatial_pose().is_none(),
        "legacy Zone is not a 3D pose"
    );
    assert_eq!(zone.vertical_state(), VerticalState::default());

    // Re-saving and re-loading keeps the legacy representation as-is.
    let loaded = round_trip(&scratch, &state);
    assert_eq!(loaded.combatants[0].pos, Some(Position::cell(3, 4)));
    assert_eq!(loaded.combatants[1].pos, None);
    assert!(loaded.combatants[1].spatial_pose().is_none());
}

#[test]
fn ac3_golden_fixtures_cover_deserialize_serialize_and_round_trip() {
    let scratch = Scratch::new("golden");
    let (_, legacy) = load_state(&scratch, LEGACY_STATE_JSON);

    // Missing old fields keep their current defaults.
    assert_eq!(legacy.combatants[0].temp_hp, 0);
    assert!(legacy.combatants[0].conditions.is_empty());
    assert!(legacy.combatants[0].resources.is_empty());
    assert!(!legacy.combatants[0].action_used);
    assert!(legacy.scene.is_none());
    assert!(legacy.spatial.is_none());

    // Serialization of an old save adds only the schema marker; new optional
    // spatial fields stay absent instead of being invented.
    let value = serde_json::to_value(&legacy).expect("legacy json");
    assert_eq!(value["schema_version"], json!(STATE_SCHEMA_VERSION));
    assert!(value.get("spatial").is_none());
    assert!(value["combatants"][0].get("pose").is_none());
    assert!(value["combatants"][0].get("vertical").is_none());
    assert!(value["combatants"][0].get("footprint").is_none());

    let loaded = round_trip(&scratch, &legacy);
    assert_eq!(loaded, legacy);
    assert_eq!(
        loaded.encounter_name.as_deref(),
        Some("Legacy Cell And Zone")
    );
    assert_eq!(loaded.round, 2);
    assert_eq!(loaded.combatants[0].hp, 9);

    // A mixed old/new encounter keeps every explicit spatial field.
    let mixed_scratch = Scratch::new("mixed");
    let (_, mixed) = load_state(&mixed_scratch, MIXED_STATE_JSON);
    let pose = mixed.combatants[0].spatial_pose().expect("explicit pose");
    assert_eq!(pose.cell, CellPos { x: 1, y: 2, z: 10 });
    assert_eq!(pose.footprint, Footprint { width: 2, depth: 2 });
    assert_eq!(pose.facing, Some(Facing::North));
    assert_eq!(
        mixed.combatants[0].vertical_state(),
        VerticalState::new(10, FlightState::Flying, 0).expect("vertical")
    );
    let scene = mixed.scene.as_deref().expect("scene");
    assert_eq!(scene.cell_height_ft, Some(10));
    assert_eq!(
        scene.grid.as_ref().expect("grid").legend["."].height_ft,
        Some(4)
    );
    let spatial = mixed.spatial.as_ref().expect("state spatial");
    assert_eq!(spatial.cell_height_ft, Some(12));
    assert_eq!(
        spatial.terrain_volumes,
        vec![TerrainVolume::new(
            "ledge",
            CellPos::new(5, 5, 0).expect("cell"),
            Footprint::new(1, 2).expect("footprint"),
            4,
        )
        .expect("terrain")]
    );
    let profile = spatial.spatial_profile.expect("explicit profile");
    assert_eq!(profile.diagonal, Diagonal::FiveTenFive);
    assert_eq!(profile.squeeze, Squeeze::Edition2014);
    assert_eq!(profile.creature_cover, Cover::Half);
    assert!(!profile.tiny_sharing);
    assert!(profile.facing);
    assert!(!profile.flanking);
    assert_eq!(profile.distance_3d, Distance3d::ManhattanPlusVertical);
    assert_eq!(profile.aoe_shapes, AoeShapes::Five);

    let mixed_round_trip = round_trip(&mixed_scratch, &mixed);
    assert_eq!(mixed_round_trip, mixed);

    // Unknown fields are preserved, so a round-trip never silently drops
    // data written by a compatible extension.
    let unknown_scratch = Scratch::new("unknown-field");
    let unknown_json = r#"{
      "encounter_name": "Future Extension",
      "future_extension": {"value": 7}
    }"#;
    let (_, unknown) = load_state(&unknown_scratch, unknown_json);
    assert_eq!(
        unknown.extra.get("future_extension"),
        Some(&json!({"value": 7}))
    );
    let unknown_loaded = round_trip(&unknown_scratch, &unknown);
    assert_eq!(
        unknown_loaded.extra.get("future_extension"),
        Some(&json!({"value": 7}))
    );
}

#[test]
fn ac4_schema_version_is_guarded_without_migration() {
    let scratch = Scratch::new("schema-version");

    // Old files have no version field; they load as the current version.
    let (_, legacy) = load_state(&scratch, LEGACY_STATE_JSON);
    assert_eq!(legacy.schema_version, STATE_SCHEMA_VERSION);

    // The current version is written by save and loads again.
    let path = scratch.path("saved.state.json");
    legacy.save(&path).expect("save");
    let text = fs::read_to_string(&path).expect("read saved");
    let value: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(value["schema_version"], json!(STATE_SCHEMA_VERSION));
    assert_eq!(
        EncounterState::load(&path).expect("reload").schema_version,
        STATE_SCHEMA_VERSION
    );

    // A newer schema is rejected with a user-readable error instead of being
    // silently reinterpreted. P1 is additive, so no migration exists yet.
    let future_path = scratch.write("future.state.json", r#"{"schema_version": 2}"#);
    let error = EncounterState::load(&future_path).expect_err("future schema rejected");
    let message = format!("{error:#}");
    assert!(message.contains("schema version"), "{message}");
    assert!(message.contains('2'), "{message}");
    assert!(message.contains("supports"), "{message}");
}

#[test]
fn ac5_legacy_profile_is_preserved_until_explicitly_enabled() {
    let scratch = Scratch::new("profiles");
    let (_, mut state) = load_state(&scratch, LEGACY_STATE_JSON);

    // No explicit profile anywhere: the ruleset default applies, and nothing
    // changes for old saves.
    assert!(state.content.rules.spatial_profile.is_none());
    assert!(state.scene.is_none());
    assert!(state.spatial.is_none());
    assert_eq!(state.effective_spatial_profile(), SpatialProfile::default());
    assert_eq!(
        state.effective_spatial_profile(),
        SpatialProfile::for_ruleset(Some("2024"))
    );

    // A 2014 registry selects the 2014 defaults.
    Arc::make_mut(&mut state.content).ruleset = "2014".to_string();
    assert_eq!(
        state.effective_spatial_profile(),
        SpatialProfile::for_ruleset(Some("2014"))
    );
    assert_eq!(
        state.effective_spatial_profile().squeeze,
        Squeeze::Edition2014
    );
    assert_eq!(
        state.effective_spatial_profile().aoe_shapes,
        AoeShapes::Five
    );

    // Rules profile < scene profile < state profile.
    let rules_profile = SpatialProfile::for_ruleset(Some("2024"));
    Arc::make_mut(&mut state.content).rules.spatial_profile = Some(rules_profile);
    assert_eq!(state.effective_spatial_profile(), rules_profile);

    let scene_profile = SpatialProfile::for_ruleset(Some("2014"));
    let mut scene: Scene = serde_json::from_value(json!({"id": "synthetic-scene"})).expect("scene");
    scene.spatial_profile = Some(scene_profile);
    state.scene = Some(Arc::new(scene));
    assert_eq!(state.effective_spatial_profile(), scene_profile);

    let state_profile = SpatialProfile::for_ruleset(Some("2024"));
    state.spatial = Some(SpatialState {
        cell_height_ft: Some(15),
        terrain_volumes: Vec::new(),
        spatial_profile: Some(state_profile),
    });
    assert_eq!(state.effective_spatial_profile(), state_profile);

    // Profile defaults match the frozen matrix.
    let p2014 = SpatialProfile::for_ruleset(Some("2014"));
    assert_eq!(p2014.diagonal, Diagonal::Five);
    assert_eq!(p2014.squeeze, Squeeze::Edition2014);
    assert_eq!(p2014.creature_cover, Cover::Half);
    assert!(!p2014.tiny_sharing);
    assert!(!p2014.facing);
    assert!(!p2014.flanking);
    assert_eq!(p2014.distance_3d, Distance3d::Chebyshev);
    assert_eq!(p2014.aoe_shapes, AoeShapes::Five);

    let p2024 = SpatialProfile::for_ruleset(Some("2024"));
    assert_eq!(p2024.squeeze, Squeeze::Edition2024);
    assert_eq!(p2024.aoe_shapes, AoeShapes::Six);
    assert_eq!(SpatialProfile::default(), p2024);
    assert_eq!(SpatialProfile::for_ruleset(None), p2024);
}

#[test]
fn ac6_validation_rejects_invalid_spatial_data_on_load() {
    let scratch = Scratch::new("invalid");

    let bad_footprint = r#"{
      "combatants": [{
        "creature": {"id": "synthetic-bad"},
        "name": "Bad Footprint",
        "side": "party",
        "max_hp": 1,
        "hp": 1,
        "zone": "hall",
        "pose": {
          "cell": {"x": 0, "y": 0, "z": 0},
          "footprint": {"width": 0, "depth": 1}
        }
      }]
    }"#;
    let path = scratch.write("bad-footprint.state.json", bad_footprint);
    let error = EncounterState::load(&path).expect_err("bad footprint rejected");
    let message = format!("{error:#}");
    assert!(message.contains("footprint"), "{message}");
    assert!(message.contains("Bad Footprint"), "{message}");

    let out_of_bounds = bad_footprint.replace(
        r#""footprint": {"width": 0, "depth": 1}"#,
        r#""footprint": {"width": 1, "depth": 1}"#,
    );
    let out_of_bounds = out_of_bounds.replace(
        r#""cell": {"x": 0, "y": 0, "z": 0}"#,
        &format!(r#""cell": {{"x": {}, "y": 0, "z": 0}}"#, MAX_GRID_COORD + 1),
    );
    let path = scratch.write("out-of-bounds.state.json", &out_of_bounds);
    let error = EncounterState::load(&path).expect_err("out-of-bounds cell rejected");
    let message = format!("{error:#}");
    assert!(message.contains("coordinate"), "{message}");

    let bad_altitude = r#"{
      "combatants": [{
        "creature": {"id": "synthetic-bad"},
        "name": "Bad Altitude",
        "side": "party",
        "max_hp": 1,
        "hp": 1,
        "zone": "hall",
        "vertical": {"altitude_ft": -5, "flight": "flying", "fall_distance_ft": 0}
      }]
    }"#;
    let path = scratch.write("bad-altitude.state.json", bad_altitude);
    let error = EncounterState::load(&path).expect_err("bad altitude rejected");
    let message = format!("{error:#}");
    assert!(message.contains("altitude"), "{message}");
    assert!(message.contains("Bad Altitude"), "{message}");

    let bad_terrain = r#"{
      "spatial": {
        "terrain_volumes": [{
          "id": "bad",
          "cell": {"x": 0, "y": 0, "z": 0},
          "footprint": {"width": 1, "depth": 1},
          "height_ft": 0
        }]
      }
    }"#;
    let path = scratch.write("bad-terrain.state.json", bad_terrain);
    let error = EncounterState::load(&path).expect_err("bad terrain rejected");
    let message = format!("{error:#}");
    assert!(message.contains("height"), "{message}");
    assert!(message.contains("bad"), "{message}");

    // Unknown enum spellings fail closed instead of becoming a wrong shape.
    assert!(serde_json::from_value::<AreaSpec>(json!({
        "radius": 5,
        "shape": "hexagon"
    }))
    .is_err());
}

#[test]
fn ac8_position_and_scene_mode_stay_additive() {
    // Exhaustive destructuring without `..` proves no fields were added to the
    // existing variants.
    let Position::Cell { x, y } = Position::cell(1, 2) else {
        panic!("cell variant");
    };
    assert_eq!((x, y), (1, 2));
    let Position::Zone { zone } = Position::zone("hall") else {
        panic!("zone variant");
    };
    assert_eq!(zone, "hall");

    assert_eq!(SceneMode::default(), SceneMode::Zones);
    match SceneMode::Grid {
        SceneMode::Zones | SceneMode::Grid => {}
    }
    assert_eq!(
        serde_json::from_value::<Position>(json!({"kind": "cell", "x": 3, "y": 4}))
            .expect("old cell json"),
        Position::cell(3, 4)
    );
    assert_eq!(
        serde_json::from_value::<Position>(json!({"kind": "zone", "zone": "hall"}))
            .expect("old zone json"),
        Position::zone("hall")
    );
}

#[test]
fn ac9_p4_p5_state_and_schema_fields_are_complete_with_defaults() {
    // CombatantState: pose, vertical, footprint (all optional, absent by
    // default).
    let scratch = Scratch::new("field-inventory");
    let (_, legacy) = load_state(&scratch, LEGACY_STATE_JSON);
    assert!(legacy.combatants[0].pose.is_none());
    assert!(legacy.combatants[0].vertical.is_none());
    assert!(legacy.combatants[0].footprint.is_none());

    // Scene: per-cell height, terrain volumes, spatial profile.
    let scene: Scene =
        serde_json::from_value(json!({"id": "synthetic-scene"})).expect("minimal scene");
    assert!(scene.cell_height_ft.is_none());
    assert!(scene.terrain_volumes.is_empty());
    assert!(scene.spatial_profile.is_none());

    // GridCell: obstacle height.
    let cell: GridCell = serde_json::from_value(json!({"wall": false})).expect("minimal cell");
    assert!(cell.height_ft.is_none());

    // Rules: the spatial profile slot.
    let rules: Rules = serde_json::from_value(json!({})).expect("minimal rules");
    assert!(rules.spatial_profile.is_none());

    // AreaSpec: shape and width are optional; radius keeps its old meaning.
    let area: AreaSpec = serde_json::from_value(json!({"radius": 5})).expect("legacy area");
    assert!(area.shape.is_none());
    assert!(area.width_ft.is_none());
    assert_eq!(
        serde_json::to_value(&area).expect("area json"),
        json!({"radius": 5})
    );

    // Speed: hover and burrow are additive with safe defaults.
    let speed = Speed::default();
    assert!(!speed.hover);
    assert_eq!(speed.burrow, 0);
    let speed: Speed = serde_json::from_value(json!({"walk": 30})).expect("legacy speed");
    assert!(!speed.hover);
    assert_eq!(speed.burrow, 0);
    let flying_speed = Speed {
        walk: 30,
        climb: 10,
        fly: 60,
        swim: 0,
        hover: true,
        burrow: 5,
    };
    assert_eq!(
        serde_json::from_value::<Speed>(serde_json::to_value(&flying_speed).expect("speed json"))
            .expect("speed round-trip"),
        flying_speed
    );

    // SpatialState mirrors the scene-level P1 fields.
    let spatial = SpatialState::default();
    assert!(spatial.cell_height_ft.is_none());
    assert!(spatial.terrain_volumes.is_empty());
    assert!(spatial.spatial_profile.is_none());

    // A profile can be written partially; every omitted field takes the 2024
    // default, including half creature cover.
    let profile: SpatialProfile =
        serde_json::from_value(json!({"diagonal": "five_ten_five"})).expect("partial profile");
    assert_eq!(profile.diagonal, Diagonal::FiveTenFive);
    assert_eq!(profile.squeeze, Squeeze::Edition2024);
    assert_eq!(profile.creature_cover, Cover::Half);
    assert_eq!(profile.aoe_shapes, AoeShapes::Six);
}

#[test]
fn ac10_compat_layer_is_public_and_additive() {
    // The compatibility layer is additive: old positions keep working and the
    // new types are public from `combat_sim::space` without touching
    // engine.rs/ai.rs/cli.rs. The PR diff is the evidence that those files are
    // untouched; this test backs the compile-time part.
    let scratch = Scratch::new("additive");
    let (_, state) = load_state(&scratch, LEGACY_STATE_JSON);
    let exported: fn() -> SpatialProfile = SpatialProfile::default;
    assert_eq!(exported(), SpatialProfile::for_ruleset(Some("2024")));
    assert_eq!(state.combatants.len(), 2);
}
