# Spatial contract v1 (P0)

- Status: frozen by TASK-0085 (#169) on 2026-10-27.
- Companion: `SPATIAL-RULES-MATRIX.md` (rules, sources, gaps).
- Consumers: TASK-0086 (P1 types/state), TASK-0087 (P2 geometry),
  TASK-0088 (P3 movement), TASK-0089 (P4 flight/fall), TASK-0090 (P5 engine).
- This contract is additive. It does not change existing engine behaviour by
  itself; P5 is the only task that wires it into the engine.

## 1. Invariants

1. **Additive only.** `Position` and `SceneMode` keep their current variants and
   signatures. New spatial types live beside them. `engine.rs`, `ai.rs` and
   `cli.rs` must not be edited by P1.
2. **One query kernel.** Distance, travel cost, occupancy, line of sight, cover
   and areas are answered by one module; callers do not re-implement geometry.
3. **Ruleset-selected defaults.** A save has one ruleset; the profile is chosen
   from it and can be overridden explicitly. No mixing.
4. **Explicit approximations.** Zone mode and the one-token-per-cell Tiny rule
   are approximations; every query documents its zone fallback.
5. **Determinism.** No HashMap iteration order, no float tie-breakers in
   ordering. Ties resolve by id.
6. **Old saves load unchanged.** Missing spatial fields take defaults; no
   migration is required to open a pre-P1 save.

## 2. Units and basic types

```rust
/// A grid cell. x/y are 5-ft squares; z is altitude in feet above the floor.
pub struct CellPos { pub x: i32, pub y: i32, pub z: i32 }

/// Footprint in cells on the ground plane.
pub struct Footprint { pub width: u32, pub depth: u32 }

/// Vertical extent in feet; used for line of sight and target volumes.
pub struct Volume { pub width: u32, pub depth: u32, pub height_ft: i32 }

pub struct SpatialPose {
    pub cell: CellPos,
    pub footprint: Footprint,
    /// Facing is None unless the profile enables it.
    pub facing: Option<Facing>,
}

pub enum Facing { North, East, South, West }
```

Rules:

- `z` is feet, not cells. Ground is 0. A creature at altitude 10 occupies the
  volume from 10 to 10 + height.
- Footprint defaults: Tiny/Small/Medium 1×1, Large 2×2, Huge 3×3, Gargantuan
  4×4. Height defaults: Tiny 2.5, Small/Medium 5, Large 10, Huge 15,
  Gargantuan 20 ft. Both are profile data, not hard-coded per monster.
- A pose with `footprint` larger than 1×1 is anchored at its top-left cell.

## 3. Profile

```rust
pub struct SpatialProfile {
    pub diagonal: Diagonal,
    pub squeeze: Squeeze,
    pub creature_cover: CoverDegree,
    pub tiny_sharing: bool,        // v1 default false
    pub facing: bool,              // default false
    pub flanking: bool,            // default false
    pub distance_3d: Distance3d,
    pub aoe_shapes: AoeShapes,
}

pub enum Diagonal { Five, FiveTenFive }
pub enum Squeeze { Edition2014, Edition2024, Forbid }
pub enum Distance3d { Chebyshev, ManhattanPlusVertical }
pub enum AoeShapes { Five, Six }   // Six adds Emanation (2024)
```

Defaults:

| Field | 2014 / 5e | 2024 / 5.5e |
|---|---|---|
| diagonal | Five | Five |
| squeeze | Edition2014 | Edition2024 |
| creature_cover | Half | Half |
| tiny_sharing | false | false |
| facing | false | false |
| flanking | false | false |
| distance_3d | Chebyshev | Chebyshev |
| aoe_shapes | Five | Six |

The profile is data on the scene/encounter, not a global constant.

## 4. Distance

`distance(a, b, profile) -> i32` (feet)

- Ground distance: Chebyshev cells × 5; each diagonal is one square.
- With `FiveTenFive`: every second diagonal costs 10 ft along the chosen path.
  The kernel must use a deterministic path rule (prefer x then y) and document
  it; the result must not depend on direction.
- 3D: Chebyshev ground distance + |z difference| (feet), both editions.
- `distance` ignores walls, cover and occupancy. It is pure geometry.

`travel_distance(from, to, profile, terrain) -> Travel`

- Returns `{ feet, squares, path, blocked_reason }`.
- Applies Difficult Terrain (×2 per foot), squeeze (2014: ×2), crawl/swim/
  climb (×2), and creature-space cost.
- `blocked_reason` is one of `Wall`, `Occupied`, `TooNarrow`, `NoPath`,
  `OutOfBounds`.

## 5. Reach, line of sight and cover

`can_reach(attacker, target, reach_ft, profile) -> bool`

- Geometric only: `distance <= reach_ft` plus 3D reach.
- **Does not** check walls, cover or occupancy. Callers combine it with
  `line_of_sight` and `cover`.
- Documented in the query result as `{ reachable: bool, distance_ft: i32 }`.

`line_of_sight(from, to, blockers, profile) -> LineOfSight`

- `LineOfSight { clear: bool, blocked_by: Vec<BlockerId> }`.
- Blockers: walls, closed doors, total-cover volumes, unpenetrated illusions
  (R-17), and optionally large creatures.
- Half/three-quarters cover does not block sight.

`cover(target, source, blockers, profile) -> Cover`

- `Cover { degree: None|Half|ThreeQuarters|Total, sources: Vec<CoverSource> }`.
- Degrees follow the matrix: half +2, three-quarters +5, total = cannot be
  targeted directly.
- Multiple sources: keep the most protective degree only.
- Creature cover defaults to half; a profile may raise it for size difference.
- 2024 profiles apply the opposite-side rule.

## 6. Occupancy and traverse

`occupies(pose) -> Vec<CellPos>` returns every covered cell at the pose's
altitude band.

`can_occupy(pose, occupants, terrain, profile) -> Occupancy`

- Empty cells are allowed.
- Another creature's cell: allowed while moving through under the matrix rules
  (ally, Incapacitated, Tiny, two size steps), at Difficult Terrain cost.
- Ending a move in an occupied cell is not allowed unless the mover is Tiny or
  larger than the other creature; if forced, apply Prone.
- Tiny sharing is **off** in v1: one token per cell even for Tiny.

`can_traverse(from, to, footprint, occupants, terrain, profile) -> Traverse`

- Checks every cell of the swept footprint, corner cutting, squeeze policy and
  occupancy.
- 2014 squeeze allows a Large footprint through a 1-cell opening at double cost
  (and the engine exposes the disadvantage flags); 2024 treats the narrow
  opening as Difficult Terrain without disadvantage.
- Returns `{ allowed, cost_ft, reason }`.

## 7. Areas of effect

`area_cells(shape, origin, size, profile) -> AreaResult`

```rust
pub enum AreaShape { Sphere, Cone, Cube, Cylinder, Line, Emanation }
pub struct AreaSpecV1 { pub shape: AreaShape, pub size_ft: i32, pub width_ft: Option<i32> }
```

- Sphere: radius, origin included.
- Cone: length = width at the far edge, origin excluded.
- Cube: side length, origin on a face.
- Cylinder: radius + height, origin at base center.
- Line: length + width, origin excluded unless stated.
- Emanation (2024 only): radius around a creature/object, moves with it,
  origin excluded unless stated.
- 2014 profiles reject Emanation with `UnsupportedShape`.
- Area cells use the same footprint grid; no per-cell `contains` over the whole
  map. Complexity must be O(cells in the shape).
- Half/three-quarters cover does not reduce area damage; total cover blocks a
  cell when the line from the origin is blocked.

## 8. Vertical state

```rust
pub enum FlightState { Grounded, Flying, Hovering, Falling }
pub struct VerticalState {
    pub altitude_ft: i32,
    pub flight: FlightState,
    pub fall_distance_ft: i32,
}
```

- `Grounded` at 0 ft. `Flying` keeps altitude until landing/falling.
- `Hovering` prevents the fall from Incapacitated/Prone/0 Fly Speed.
- Falling is resolved at the end of the fall: 1d6 per 10 ft (max 20d6), land
  Prone unless no damage; R-09 adds the DC 10 Acrobatics within 10 ft check.
- P4 owns the resolution; P1 only stores the state.

## 9. Zone mode

Zone mode is a graph; it is not converted into a metric grid.

- `distance`: use the cached all-pairs shortest path over zone edges; each edge
  carries its own feet value. Same zone = **0 ft** for reach/travel.
- `can_reach`: same zone or a path within reach. No wall/cover check; callers
  add `line_of_sight` where the scene defines it.
- `travel_distance`: edge costs; blocked edges return `NoPath`.
- `area_cells`: only zone-wide effects are supported. Cone/line/cube in zone
  mode return `UnsupportedShape`; sphere/emanation affect the origin zone and
  every zone within N hops (documented approximation).
- Display text may say "same zone (≈5 ft)" even though the query returns 0.

## 10. Query surface

| Query | Input | Output | Zone fallback |
|---|---|---|---|
| `distance` | two poses | feet | graph distance |
| `travel_distance` | poses + terrain | cost/path/reason | edge costs |
| `can_reach` | poses + reach | bool/distance | graph distance |
| `can_occupy` | pose + occupants | bool/reason | zone occupancy |
| `can_traverse` | step + footprint | bool/cost/reason | edge traversal |
| `line_of_sight` | poses + blockers | clear + blockers | scene LOS flags |
| `cover` | target/source + blockers | degree + sources | edge cover flag |
| `area_cells` | shape + origin + size | cells/zones | approximate |
| `opportunity_threat` | mover + observers | observers | adjacency + LOS |
| `fall_from` | vertical state | damage/conditions | zone height |

All queries return a struct with an `explain` list of ids so the DM log can
show why a result happened. Queries never mutate state.

## 11. State and compatibility

P1 adds, without changing existing variants:

- `CombatantState`: `pose: Option<SpatialPose>`, `vertical: Option<VerticalState>`,
  `footprint: Option<Footprint>`.
- `Scene`: `cell_height_ft`, `terrain_volumes`, `spatial_profile`.
- `EncounterState`: `spatial: Option<SpatialState>` holding the above.

Defaults and migration:

- Old `Position::Cell { x, y }` → `pose.cell = (x, y, 0)`, default footprint
  from size, `VerticalState::Grounded`.
- Old `Position::Zone` → no pose; zone queries keep using the existing graph.
- Missing fields deserialize to `None` and behave as today.
- Serialization keeps `None` as absent/None; golden fixtures cover old JSON,
  round-trip, and a mixed old/new encounter.
- No schema version bump in P1. A later semantic change (e.g. changing the
  meaning of an existing field) requires a bump and a migration.

## 12. Performance and determinism

- Geometry only: a 50×50×10 scene is 25k cells; a per-turn reachable set must be
  computed once and reused.
- No query may scan the whole map per target. Areas are O(shape cells); LOS is
  O(length of the segment).
- Candidate movement goals are capped by the AI layer (top-K waypoints), not by
  the geometry kernel.
- Ordering is by cell coordinates and entity id. No floating-point comparison
  decides order.

## 13. Task boundaries

| Task | Owns |
|---|---|
| P1 (#170) | types, profile, state fields, serialization, golden fixtures |
| P2 (#171) | distance, LOS, cover, area cells, occupancy queries |
| P3 (#172) | movement edges, Dijkstra/A*, squeeze, creature traversal |
| P4 (#173) | altitude, flight, hover, falling, vertical reach |
| P5 (#174) | engine/CLI wiring, trigger order, legacy behaviour regression |
| TASK-0094 (#178) | real hover/burrow/shape data; synthetic fixtures until then |

## 14. Acceptance anchors for P1–P5

1. Old save JSON loads and re-saves without semantic change.
2. Every query has a test for empty, blocked, out-of-bounds and tie cases.
3. 2014 and 2024 profiles produce the documented squeeze/cover/diagonal results.
4. Hand-written content and old engine behaviour are unchanged until P5.
5. Logs explain blockers and cover sources with ids; no DM-only data leaks into
   player output.

## 15. Non-goals

- Full 3D pathfinding with arbitrary volumes (walls are cell/volume blockers;
  no mesh geometry).
- Facing/flanking automation.
- Vehicle, siege or swimming-current simulation.
- Auto-resolving cover disputes; the DM can always override with a ruling.
