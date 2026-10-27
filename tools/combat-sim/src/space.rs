//! Spatial rules seam: zone graphs and 5-ft grids behind one [`Space`] trait.
//!
//! Every engine rule that depends on where creatures are goes through this
//! module: range/reach, cover, light, movement routes, opportunity attacks and
//! area effects. Zone scenes use [`ZoneGraph`], grid scenes [`GridSpace`].

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::content::schema::{ClimbSpec, Cover, Light, Scene, SceneMode, Size};

/// Distance from any point in a zone to any other point in the same zone.
pub const SAME_ZONE_DISTANCE: i32 = 0;

/// Zone tag shown for a grid cell that has no legend zone.
pub const OPEN_ZONE: &str = "open";

/// A combatant's location.
///
/// Zone scenes use [`Position::Zone`]; grid scenes use [`Position::Cell`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Position {
    Zone { zone: String },
    Cell { x: i32, y: i32 },
}

impl Position {
    pub fn zone(zone: impl Into<String>) -> Self {
        Self::Zone { zone: zone.into() }
    }

    pub fn cell(x: i32, y: i32) -> Self {
        Self::Cell { x, y }
    }

    pub fn cell_coords(&self) -> Option<(i32, i32)> {
        match self {
            Self::Cell { x, y } => Some((*x, *y)),
            Self::Zone { .. } => None,
        }
    }

    pub fn zone_id(&self) -> Option<&str> {
        match self {
            Self::Zone { zone } => Some(zone),
            Self::Cell { .. } => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Additive spatial types (P1)
// ---------------------------------------------------------------------------
//
// These types coexist with `Position` and `SceneMode`; the engine keeps using
// the old representations until P5. Units: x/y are 5-ft grid squares, z and
// every other height/distance is feet.

/// Feet per grid square.
pub const FEET_PER_CELL: i32 = 5;

/// Inclusive bound for a stored grid coordinate, used by validation.
pub const MAX_GRID_COORD: i32 = 10_000;

/// Inclusive bound for a stored altitude or fall distance.
pub const MAX_ALTITUDE_FT: i32 = 100_000;

/// Inclusive bound for one footprint edge, in cells.
pub const MAX_FOOTPRINT_CELLS: u32 = 20;

/// Inclusive bound for a stored volume height.
pub const MAX_VOLUME_HEIGHT_FT: i32 = 10_000;

/// A rejected spatial value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpatialError {
    CoordinateOutOfRange {
        x: i32,
        y: i32,
    },
    AltitudeOutOfRange {
        z: i32,
    },
    FallDistanceOutOfRange {
        distance_ft: i32,
    },
    InvalidFootprint {
        width: u32,
        depth: u32,
    },
    InvalidVolume {
        width: u32,
        depth: u32,
        height_ft: i32,
    },
    InvalidHeight {
        height_ft: i32,
    },
    InvalidArea {
        size_ft: i32,
        width_ft: Option<i32>,
    },
    InvalidFlightState {
        flight: FlightState,
        altitude_ft: i32,
    },
    UnsupportedShape {
        shape: AreaShape,
        shapes: AoeShapes,
    },
    MissingAreaDirection {
        shape: AreaShape,
    },
    UnsupportedInZone {
        shape: AreaShape,
    },
    AreaTooLarge {
        size_ft: i32,
        limit_ft: i32,
    },
    UnknownPosition {
        position: String,
    },
    EmptyTerrainId,
    EmptyBlockerId,
    InvalidMovementMap {
        width: i32,
        height: i32,
        cells: usize,
    },
    InvalidBudget {
        budget_ft: i32,
    },
    InvalidDistance {
        distance_ft: i32,
    },
    InvalidDirection {
        dx: i32,
        dy: i32,
    },
    PoseMismatch {
        expected: Footprint,
        actual: Footprint,
    },
}

impl fmt::Display for SpatialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoordinateOutOfRange { x, y } => write!(
                f,
                "grid coordinate ({x}, {y}) is outside -{MAX_GRID_COORD}..={MAX_GRID_COORD}"
            ),
            Self::AltitudeOutOfRange { z } => write!(
                f,
                "altitude {z} ft is outside 0..={MAX_ALTITUDE_FT} ft"
            ),
            Self::FallDistanceOutOfRange { distance_ft } => write!(
                f,
                "fall distance {distance_ft} ft is outside 0..={MAX_ALTITUDE_FT} ft"
            ),
            Self::InvalidFootprint { width, depth } => write!(
                f,
                "footprint {width}x{depth} cells is outside 1..={MAX_FOOTPRINT_CELLS} cells per edge"
            ),
            Self::InvalidVolume {
                width,
                depth,
                height_ft,
            } => write!(
                f,
                "volume {width}x{depth} cells x {height_ft} ft is outside the supported range"
            ),
            Self::InvalidHeight { height_ft } => write!(
                f,
                "height {height_ft} ft is outside 1..={MAX_VOLUME_HEIGHT_FT} ft"
            ),
            Self::InvalidArea { size_ft, width_ft } => {
                write!(f, "area size {size_ft} ft")?;
                if let Some(width) = width_ft {
                    write!(f, " / width {width} ft")?;
                }
                f.write_str(" must be positive")
            }
            Self::InvalidFlightState {
                flight,
                altitude_ft,
            } => write!(
                f,
                "flight state {flight:?} is not valid at altitude {altitude_ft} ft"
            ),
            Self::UnsupportedShape { shape, shapes } => {
                write!(f, "area shape {shape:?} is not supported by {shapes:?}")
            }
            Self::MissingAreaDirection { shape } => write!(
                f,
                "area shape {shape:?} needs a facing direction but the pose has none"
            ),
            Self::UnsupportedInZone { shape } => write!(
                f,
                "area shape {shape:?} has no exact zone-graph representation"
            ),
            Self::AreaTooLarge { size_ft, limit_ft } => write!(
                f,
                "area size {size_ft} ft is outside the supported 1..={limit_ft} ft range"
            ),
            Self::UnknownPosition { position } => {
                write!(f, "position {position} is not in the zone graph")
            }
            Self::EmptyTerrainId => f.write_str("terrain volume id must not be empty"),
            Self::EmptyBlockerId => f.write_str("spatial blocker id must not be empty"),
            Self::InvalidMovementMap {
                width,
                height,
                cells,
            } => write!(
                f,
                "movement map {width}x{height} needs {} cells but got {cells}",
                i64::from(*width) * i64::from(*height)
            ),
            Self::InvalidBudget { budget_ft } => {
                write!(f, "movement budget {budget_ft} ft must not be negative")
            }
            Self::InvalidDistance { distance_ft } => {
                write!(f, "movement distance {distance_ft} ft must not be negative")
            }
            Self::InvalidDirection { dx, dy } => write!(
                f,
                "movement direction ({dx}, {dy}) must be one grid step"
            ),
            Self::PoseMismatch { expected, actual } => write!(
                f,
                "pose footprint {actual:?} does not match the mover footprint {expected:?}"
            ),
        }
    }
}

impl std::error::Error for SpatialError {}

/// A grid cell. x/y are 5-ft squares; z is altitude in feet above the floor.
#[derive(
    Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(deny_unknown_fields)]
pub struct CellPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Contract alias for a 3D grid position.
pub type GridPosition3 = CellPos;

impl CellPos {
    pub const ORIGIN: Self = Self { x: 0, y: 0, z: 0 };

    pub fn new(x: i32, y: i32, z: i32) -> Result<Self, SpatialError> {
        let cell = Self { x, y, z };
        cell.validate()?;
        Ok(cell)
    }

    pub fn validate(&self) -> Result<(), SpatialError> {
        if !(-MAX_GRID_COORD..=MAX_GRID_COORD).contains(&self.x)
            || !(-MAX_GRID_COORD..=MAX_GRID_COORD).contains(&self.y)
        {
            return Err(SpatialError::CoordinateOutOfRange {
                x: self.x,
                y: self.y,
            });
        }
        if !(0..=MAX_ALTITUDE_FT).contains(&self.z) {
            return Err(SpatialError::AltitudeOutOfRange { z: self.z });
        }
        Ok(())
    }
}

/// Footprint in cells on the ground plane.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    pub width: u32,
    pub depth: u32,
}

impl Footprint {
    pub const ONE: Self = Self { width: 1, depth: 1 };

    pub fn new(width: u32, depth: u32) -> Result<Self, SpatialError> {
        let footprint = Self { width, depth };
        footprint.validate()?;
        Ok(footprint)
    }

    /// Default footprint from a creature size. Unknown sizes stay 1x1.
    pub fn for_size(size: Size) -> Self {
        let edge = match size {
            Size::Large => 2,
            Size::Huge => 3,
            Size::Gargantuan => 4,
            Size::Tiny | Size::Small | Size::Medium | Size::Unknown => 1,
        };
        Self {
            width: edge,
            depth: edge,
        }
    }

    pub fn validate(&self) -> Result<(), SpatialError> {
        if self.width == 0
            || self.depth == 0
            || self.width > MAX_FOOTPRINT_CELLS
            || self.depth > MAX_FOOTPRINT_CELLS
        {
            return Err(SpatialError::InvalidFootprint {
                width: self.width,
                depth: self.depth,
            });
        }
        Ok(())
    }
}

impl Default for Footprint {
    fn default() -> Self {
        Self::ONE
    }
}

/// Vertical extent in feet; width/depth are cells.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct Volume {
    pub width: u32,
    pub depth: u32,
    pub height_ft: i32,
}

impl Volume {
    pub fn new(width: u32, depth: u32, height_ft: i32) -> Result<Self, SpatialError> {
        let volume = Self {
            width,
            depth,
            height_ft,
        };
        volume.validate()?;
        Ok(volume)
    }

    /// Default volume from a creature size. Tiny's 2.5 ft height is rounded up
    /// to 3 ft because [`Volume`] stores whole feet.
    pub fn for_size(size: Size) -> Self {
        let footprint = Footprint::for_size(size);
        let height_ft = match size {
            Size::Tiny => 3,
            Size::Small | Size::Medium | Size::Unknown => 5,
            Size::Large => 10,
            Size::Huge => 15,
            Size::Gargantuan => 20,
        };
        Self {
            width: footprint.width,
            depth: footprint.depth,
            height_ft,
        }
    }

    pub fn validate(&self) -> Result<(), SpatialError> {
        if self.width == 0
            || self.depth == 0
            || self.width > MAX_FOOTPRINT_CELLS
            || self.depth > MAX_FOOTPRINT_CELLS
            || self.height_ft <= 0
            || self.height_ft > MAX_VOLUME_HEIGHT_FT
        {
            return Err(SpatialError::InvalidVolume {
                width: self.width,
                depth: self.depth,
                height_ft: self.height_ft,
            });
        }
        Ok(())
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self::for_size(Size::Medium)
    }
}

/// Facing is opt-in; it stays `None` unless the profile enables it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Facing {
    North,
    East,
    South,
    West,
}

/// A footprint anchored at its top-left cell, plus optional facing.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpatialPose {
    pub cell: CellPos,
    pub footprint: Footprint,
    #[serde(default)]
    pub facing: Option<Facing>,
}

impl SpatialPose {
    pub fn new(
        cell: CellPos,
        footprint: Footprint,
        facing: Option<Facing>,
    ) -> Result<Self, SpatialError> {
        let pose = Self {
            cell,
            footprint,
            facing,
        };
        pose.validate()?;
        Ok(pose)
    }

    pub fn validate(&self) -> Result<(), SpatialError> {
        self.cell.validate()?;
        self.footprint.validate()
    }

    /// Converts an old 2D cell into a ground pose. Old zones stay zones and
    /// return `None`; they are never turned into a metric grid.
    pub fn from_position(position: &Position, size: Size) -> Option<Self> {
        match position {
            Position::Cell { x, y } => Some(Self {
                cell: CellPos { x: *x, y: *y, z: 0 },
                footprint: Footprint::for_size(size),
                facing: None,
            }),
            Position::Zone { .. } => None,
        }
    }

    /// Every cell covered by the footprint at the pose's altitude band.
    pub fn covers(&self) -> Vec<CellPos> {
        let mut cells = Vec::with_capacity((self.footprint.width * self.footprint.depth) as usize);
        for dy in 0..self.footprint.depth {
            for dx in 0..self.footprint.width {
                cells.push(CellPos {
                    x: self.cell.x + dx as i32,
                    y: self.cell.y + dy as i32,
                    z: self.cell.z,
                });
            }
        }
        cells
    }
}

/// Flight mode stored with a combatant.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FlightState {
    #[default]
    Grounded,
    Flying,
    Hovering,
    Falling,
}

/// Vertical runtime state; P4 owns fall resolution, P1 only stores it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct VerticalState {
    #[serde(default)]
    pub altitude_ft: i32,
    #[serde(default)]
    pub flight: FlightState,
    #[serde(default)]
    pub fall_distance_ft: i32,
}

impl VerticalState {
    pub fn new(
        altitude_ft: i32,
        flight: FlightState,
        fall_distance_ft: i32,
    ) -> Result<Self, SpatialError> {
        let vertical = Self {
            altitude_ft,
            flight,
            fall_distance_ft,
        };
        vertical.validate()?;
        Ok(vertical)
    }

    pub fn grounded() -> Self {
        Self::default()
    }

    pub fn validate(&self) -> Result<(), SpatialError> {
        if !(0..=MAX_ALTITUDE_FT).contains(&self.altitude_ft) {
            return Err(SpatialError::AltitudeOutOfRange {
                z: self.altitude_ft,
            });
        }
        if !(0..=MAX_ALTITUDE_FT).contains(&self.fall_distance_ft) {
            return Err(SpatialError::FallDistanceOutOfRange {
                distance_ft: self.fall_distance_ft,
            });
        }
        if self.flight == FlightState::Grounded && self.altitude_ft != 0 {
            return Err(SpatialError::InvalidFlightState {
                flight: self.flight,
                altitude_ft: self.altitude_ft,
            });
        }
        Ok(())
    }
}

/// A named solid terrain block anchored at a grid cell.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TerrainVolume {
    pub id: String,
    pub cell: CellPos,
    pub footprint: Footprint,
    pub height_ft: i32,
}

impl TerrainVolume {
    pub fn new(
        id: impl Into<String>,
        cell: CellPos,
        footprint: Footprint,
        height_ft: i32,
    ) -> Result<Self, SpatialError> {
        let terrain = Self {
            id: id.into(),
            cell,
            footprint,
            height_ft,
        };
        terrain.validate()?;
        Ok(terrain)
    }

    pub fn validate(&self) -> Result<(), SpatialError> {
        if self.id.trim().is_empty() {
            return Err(SpatialError::EmptyTerrainId);
        }
        self.cell.validate()?;
        self.footprint.validate()?;
        if self.height_ft <= 0 || self.height_ft > MAX_VOLUME_HEIGHT_FT {
            return Err(SpatialError::InvalidHeight {
                height_ft: self.height_ft,
            });
        }
        Ok(())
    }
}

/// Diagonal movement policy.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Diagonal {
    #[default]
    Five,
    FiveTenFive,
}

/// Squeezing policy selected by ruleset.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Squeeze {
    #[serde(rename = "edition_2014")]
    Edition2014,
    #[default]
    #[serde(rename = "edition_2024")]
    Edition2024,
    Forbid,
}

/// 3D distance policy.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Distance3d {
    #[default]
    Chebyshev,
    ManhattanPlusVertical,
}

/// Available area shapes: five for 2014, six (adds Emanation) for 2024.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AoeShapes {
    Five,
    #[default]
    Six,
}

impl AoeShapes {
    pub fn supports(self, shape: AreaShape) -> bool {
        match shape {
            AreaShape::Emanation => self == Self::Six,
            AreaShape::Sphere
            | AreaShape::Cone
            | AreaShape::Cube
            | AreaShape::Cylinder
            | AreaShape::Line => true,
        }
    }
}

/// Area shapes. Emanation is available only with [`AoeShapes::Six`].
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AreaShape {
    Sphere,
    Cone,
    Cube,
    Cylinder,
    Line,
    Emanation,
}

/// Engine-facing area specification (frozen contract v1).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct AreaSpecV1 {
    pub shape: AreaShape,
    pub size_ft: i32,
    #[serde(default)]
    pub width_ft: Option<i32>,
}

impl AreaSpecV1 {
    pub fn new(
        shape: AreaShape,
        size_ft: i32,
        width_ft: Option<i32>,
    ) -> Result<Self, SpatialError> {
        let area = Self {
            shape,
            size_ft,
            width_ft,
        };
        area.validate()?;
        Ok(area)
    }

    pub fn validate(&self) -> Result<(), SpatialError> {
        if self.size_ft <= 0 || self.width_ft.is_some_and(|width| width <= 0) {
            return Err(SpatialError::InvalidArea {
                size_ft: self.size_ft,
                width_ft: self.width_ft,
            });
        }
        Ok(())
    }
}

/// Data-selected spatial rules profile. The profile lives on the scene or
/// encounter, never as a global constant.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpatialProfile {
    #[serde(default)]
    pub diagonal: Diagonal,
    #[serde(default)]
    pub squeeze: Squeeze,
    #[serde(default = "default_creature_cover")]
    pub creature_cover: Cover,
    #[serde(default)]
    pub tiny_sharing: bool,
    #[serde(default)]
    pub facing: bool,
    #[serde(default)]
    pub flanking: bool,
    #[serde(default)]
    pub distance_3d: Distance3d,
    #[serde(default)]
    pub aoe_shapes: AoeShapes,
}

/// Name used by the task packet for the same profile.
pub type SpatialRulesProfile = SpatialProfile;

fn default_creature_cover() -> Cover {
    Cover::Half
}

impl Default for SpatialProfile {
    fn default() -> Self {
        Self::for_ruleset(None)
    }
}

impl SpatialProfile {
    /// Ruleset defaults from the frozen matrix. Unknown or missing rulesets use
    /// the project default (2024).
    pub fn for_ruleset(ruleset: Option<&str>) -> Self {
        let is_2014 = ruleset.is_some_and(|value| value.trim() == "2014");
        Self {
            diagonal: Diagonal::Five,
            squeeze: if is_2014 {
                Squeeze::Edition2014
            } else {
                Squeeze::Edition2024
            },
            creature_cover: Cover::Half,
            tiny_sharing: false,
            facing: false,
            flanking: false,
            distance_3d: Distance3d::Chebyshev,
            aoe_shapes: if is_2014 {
                AoeShapes::Five
            } else {
                AoeShapes::Six
            },
        }
    }

    /// Validates an area against this profile's shape set.
    pub fn validate_area(&self, area: &AreaSpecV1) -> Result<(), SpatialError> {
        area.validate()?;
        if self.aoe_shapes.supports(area.shape) {
            Ok(())
        } else {
            Err(SpatialError::UnsupportedShape {
                shape: area.shape,
                shapes: self.aoe_shapes,
            })
        }
    }
}

/// One step of a movement route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveStep {
    /// Where the mover ends after this step.
    pub position: Position,
    /// Movement cost in feet (difficult terrain doubled).
    pub cost: i32,
    /// Edge distance in feet (difficult terrain not applied).
    pub edge_distance: i32,
    /// Climb check required by this edge, if any.
    pub climb: Option<ClimbSpec>,
    /// Whether a locked door blocks this step.
    pub locked: bool,
    /// Destination elevation minus source elevation.
    pub elevation_change: i32,
}

/// Everything the engine needs to know about where creatures are.
pub trait Space {
    fn mode(&self) -> SceneMode;

    /// Distance used for reach and range in feet.
    fn distance(&self, from: &Position, to: &Position) -> Option<i32>;

    /// Shortest movement cost in feet (difficult terrain doubled).
    fn travel_distance(&self, from: &Position, to: &Position) -> Option<i32>;

    /// Whether `to` is within `reach` feet for a melee attack.
    fn can_reach(&self, from: &Position, to: &Position, reach: i32) -> bool;

    /// Cover the target has against attacks from the attacker.
    fn cover(&self, attacker: &Position, target: &Position) -> Cover;

    /// Light level at a position.
    fn light_at(&self, at: &Position) -> Light;

    /// Route from `from` to `to`, step by step. Blocked positions are grid
    /// cells occupied by hostiles; zone graphs ignore them.
    fn move_route(
        &self,
        from: &Position,
        to: &Position,
        blocked: &[Position],
    ) -> Option<Vec<MoveStep>>;

    /// Adjacent movement steps.
    fn neighbours(&self, at: &Position) -> Vec<MoveStep>;

    /// Positions inside an area centred on `center`.
    fn area_positions(&self, center: &Position, radius: i32) -> Vec<Position>;

    /// All positions belonging to a zone (a grid zone can cover many cells).
    fn zone_positions(&self, zone: &str) -> Vec<Position>;

    /// Every valid position in the scene (zone nodes or open grid cells).
    fn all_positions(&self) -> Vec<Position>;

    /// Zone tag at a position, when the position belongs to one.
    fn zone_of(&self, at: &Position) -> Option<String>;

    fn first_zone(&self) -> Option<String>;

    fn has_zone(&self, zone: &str) -> bool;

    /// Whether a position exists and is passable.
    fn contains(&self, at: &Position) -> bool;

    fn elevation(&self, at: &Position) -> i32;

    /// Whether a creature at `observer` threatens a creature leaving
    /// `departure` with a melee attack.
    fn opportunity_threat(&self, observer: &Position, departure: &Position) -> bool;

    /// Whether two positions are adjacent (within one grid cell / 5 ft).
    fn adjacent(&self, a: &Position, b: &Position) -> bool;
}

/// Dispatches to the implementation selected by the scene mode.
pub enum SpaceView<'a> {
    Zones(ZoneGraph<'a>),
    Grid(GridSpace<'a>),
}

impl Space for SpaceView<'_> {
    fn mode(&self) -> SceneMode {
        match self {
            Self::Zones(space) => space.mode(),
            Self::Grid(space) => space.mode(),
        }
    }

    fn distance(&self, from: &Position, to: &Position) -> Option<i32> {
        match self {
            Self::Zones(space) => space.distance(from, to),
            Self::Grid(space) => space.distance(from, to),
        }
    }

    fn travel_distance(&self, from: &Position, to: &Position) -> Option<i32> {
        match self {
            Self::Zones(space) => space.travel_distance(from, to),
            Self::Grid(space) => space.travel_distance(from, to),
        }
    }

    fn can_reach(&self, from: &Position, to: &Position, reach: i32) -> bool {
        match self {
            Self::Zones(space) => space.can_reach(from, to, reach),
            Self::Grid(space) => space.can_reach(from, to, reach),
        }
    }

    fn cover(&self, attacker: &Position, target: &Position) -> Cover {
        match self {
            Self::Zones(space) => space.cover(attacker, target),
            Self::Grid(space) => space.cover(attacker, target),
        }
    }

    fn light_at(&self, at: &Position) -> Light {
        match self {
            Self::Zones(space) => space.light_at(at),
            Self::Grid(space) => space.light_at(at),
        }
    }

    fn move_route(
        &self,
        from: &Position,
        to: &Position,
        blocked: &[Position],
    ) -> Option<Vec<MoveStep>> {
        match self {
            Self::Zones(space) => space.move_route(from, to, blocked),
            Self::Grid(space) => space.move_route(from, to, blocked),
        }
    }

    fn neighbours(&self, at: &Position) -> Vec<MoveStep> {
        match self {
            Self::Zones(space) => space.neighbours(at),
            Self::Grid(space) => space.neighbours(at),
        }
    }

    fn area_positions(&self, center: &Position, radius: i32) -> Vec<Position> {
        match self {
            Self::Zones(space) => space.area_positions(center, radius),
            Self::Grid(space) => space.area_positions(center, radius),
        }
    }

    fn zone_positions(&self, zone: &str) -> Vec<Position> {
        match self {
            Self::Zones(space) => space.zone_positions(zone),
            Self::Grid(space) => space.zone_positions(zone),
        }
    }

    fn all_positions(&self) -> Vec<Position> {
        match self {
            Self::Zones(space) => space.all_positions(),
            Self::Grid(space) => space.all_positions(),
        }
    }

    fn zone_of(&self, at: &Position) -> Option<String> {
        match self {
            Self::Zones(space) => space.zone_of(at),
            Self::Grid(space) => space.zone_of(at),
        }
    }

    fn first_zone(&self) -> Option<String> {
        match self {
            Self::Zones(space) => space.first_zone(),
            Self::Grid(space) => space.first_zone(),
        }
    }

    fn has_zone(&self, zone: &str) -> bool {
        match self {
            Self::Zones(space) => space.has_zone(zone),
            Self::Grid(space) => space.has_zone(zone),
        }
    }

    fn contains(&self, at: &Position) -> bool {
        match self {
            Self::Zones(space) => space.contains(at),
            Self::Grid(space) => space.contains(at),
        }
    }

    fn elevation(&self, at: &Position) -> i32 {
        match self {
            Self::Zones(space) => <ZoneGraph as Space>::elevation(space, at),
            Self::Grid(space) => space.elevation(at),
        }
    }

    fn opportunity_threat(&self, observer: &Position, departure: &Position) -> bool {
        match self {
            Self::Zones(space) => space.opportunity_threat(observer, departure),
            Self::Grid(space) => space.opportunity_threat(observer, departure),
        }
    }

    fn adjacent(&self, a: &Position, b: &Position) -> bool {
        match self {
            Self::Zones(space) => space.adjacent(a, b),
            Self::Grid(space) => space.adjacent(a, b),
        }
    }
}

/// Precomputed zone-graph data shared by every clone of an encounter state.
///
/// Building this once per state makes every distance, cover, reach, route and
/// area lookup an array index instead of a fresh Dijkstra pass.
#[derive(Debug, Default)]
pub struct ZoneCache {
    index: HashMap<String, usize>,
    heights: Vec<i32>,
    covers: Vec<Cover>,
    lights: Vec<Option<Light>>,
    scene_light: Option<Light>,
    /// Shortest edge distance, before the elevation difference is added.
    range: Vec<Vec<Option<i32>>>,
    /// Shortest movement cost (difficult terrain doubled).
    travel: Vec<Vec<Option<i32>>>,
    /// Distance of a direct edge between a pair, when one exists.
    direct: Vec<Vec<Option<i32>>>,
    /// Movement steps for every ordered pair (`Some(empty)` for the same zone).
    routes: Vec<Vec<Option<Vec<MoveStep>>>>,
    neighbours: Vec<Vec<MoveStep>>,
    positions: Vec<Position>,
    /// Area positions indexed by zone, then radius `0..=max_area_radius`.
    areas: Vec<Vec<Vec<Position>>>,
    max_area_radius: i32,
}

impl ZoneCache {
    pub fn build(scene: &Scene) -> Self {
        let count = scene.zones.len();
        let mut index = HashMap::with_capacity(count);
        for (zone_index, zone) in scene.zones.iter().enumerate() {
            index.insert(zone.id.clone(), zone_index);
        }
        let heights: Vec<i32> = scene
            .zones
            .iter()
            .map(|zone| zone.elevation.unwrap_or(0))
            .collect();
        let covers: Vec<Cover> = scene.zones.iter().map(|zone| zone.cover).collect();
        let lights: Vec<Option<Light>> = scene.zones.iter().map(|zone| zone.light).collect();
        let positions: Vec<Position> = scene
            .zones
            .iter()
            .map(|zone| Position::Zone {
                zone: zone.id.clone(),
            })
            .collect();

        let mut adjacency: Vec<Vec<(usize, usize)>> = vec![Vec::new(); count];
        let mut direct = vec![vec![None; count]; count];
        let mut max_area_radius = 0;
        for (edge_index, edge) in scene.edges.iter().enumerate() {
            let (Some(&a), Some(&b)) = (index.get(&edge.a), index.get(&edge.b)) else {
                continue;
            };
            adjacency[a].push((b, edge_index));
            if a != b && !edge.one_way {
                adjacency[b].push((a, edge_index));
            }
            if direct[a][b].is_none() {
                direct[a][b] = Some(edge.distance);
            }
            if a != b && !edge.one_way && direct[b][a].is_none() {
                direct[b][a] = Some(edge.distance);
            }
            max_area_radius = max_area_radius.max(edge.distance.max(0));
        }
        let neighbours: Vec<Vec<MoveStep>> = adjacency
            .iter()
            .enumerate()
            .map(|(from, edges)| {
                edges
                    .iter()
                    .map(|&(to, edge_index)| {
                        zone_step(scene, &heights, from, to, &scene.edges[edge_index])
                    })
                    .collect()
            })
            .collect();

        let mut range = vec![vec![None; count]; count];
        let mut travel = vec![vec![None; count]; count];
        let mut routes = vec![vec![None; count]; count];
        let mut route_edges = vec![vec![None; count]; count];
        for source in 0..count {
            let (range_best, _) = zone_dijkstra(source, &adjacency, scene, false);
            let (travel_best, travel_parents) = zone_dijkstra(source, &adjacency, scene, true);
            for target in 0..count {
                if source == target {
                    range[source][target] = Some(SAME_ZONE_DISTANCE);
                    travel[source][target] = Some(SAME_ZONE_DISTANCE);
                    routes[source][target] = Some(Vec::new());
                    continue;
                }
                let distance = range_best[target];
                if distance != i32::MAX {
                    range[source][target] =
                        Some(distance.saturating_add((heights[source] - heights[target]).abs()));
                }
                let cost = travel_best[target];
                if cost != i32::MAX {
                    travel[source][target] = Some(cost);
                    route_edges[source][target] = zone_path(source, target, &travel_parents);
                }
            }
        }
        for source in 0..count {
            for target in 0..count {
                let Some(edges) = route_edges[source][target].take() else {
                    continue;
                };
                routes[source][target] = Some(zone_steps(scene, &heights, &edges));
            }
        }

        let mut areas = vec![Vec::new(); count];
        for (zone_index, zone_areas) in areas.iter_mut().enumerate() {
            zone_areas.reserve(max_area_radius as usize + 1);
            for radius in 0..=max_area_radius {
                let mut area = vec![positions[zone_index].clone()];
                for (neighbour, edge_index) in &adjacency[zone_index] {
                    if scene.edges[*edge_index].distance <= radius
                        && !area.iter().any(|known| {
                            known.zone_id() == Some(scene.zones[*neighbour].id.as_str())
                        })
                    {
                        area.push(positions[*neighbour].clone());
                    }
                }
                zone_areas.push(area);
            }
        }

        Self {
            index,
            heights,
            covers,
            lights,
            scene_light: scene.light,
            range,
            travel,
            direct,
            routes,
            neighbours,
            positions,
            areas,
            max_area_radius,
        }
    }

    fn position_index(&self, position: &Position) -> Option<usize> {
        position
            .zone_id()
            .and_then(|zone| self.index.get(zone))
            .copied()
    }

    fn area(&self, center: &Position, radius: i32) -> Option<&[Position]> {
        let zone = self.position_index(center)?;
        let radius = radius.max(0).min(self.max_area_radius) as usize;
        self.areas.get(zone)?.get(radius).map(Vec::as_slice)
    }

    pub fn positions(&self) -> &[Position] {
        &self.positions
    }
}

fn zone_edge_cost(edge: &crate::content::schema::Edge, movement: bool) -> i32 {
    if movement && edge.difficult {
        edge.distance.saturating_mul(2)
    } else {
        edge.distance
    }
}

fn zone_dijkstra(
    source: usize,
    adjacency: &[Vec<(usize, usize)>],
    scene: &Scene,
    movement: bool,
) -> (Vec<i32>, Vec<Option<(usize, usize)>>) {
    let count = adjacency.len();
    let mut best = vec![i32::MAX; count];
    let mut parents: Vec<Option<(usize, usize)>> = vec![None; count];
    let mut done = vec![false; count];
    let mut queue = BinaryHeap::new();
    best[source] = 0;
    queue.push(Reverse((0i32, source)));
    while let Some(Reverse((cost, current))) = queue.pop() {
        if done[current] || cost > best[current] {
            continue;
        }
        done[current] = true;
        for (next, edge_index) in &adjacency[current] {
            let candidate =
                cost.saturating_add(zone_edge_cost(&scene.edges[*edge_index], movement));
            if candidate < best[*next] {
                best[*next] = candidate;
                parents[*next] = Some((current, *edge_index));
                queue.push(Reverse((candidate, *next)));
            }
        }
    }
    (best, parents)
}

fn zone_path(
    source: usize,
    target: usize,
    parents: &[Option<(usize, usize)>],
) -> Option<Vec<(usize, usize, usize)>> {
    let mut steps = Vec::new();
    let mut node = target;
    while node != source {
        let (previous, edge_index) = parents[node]?;
        steps.push((previous, node, edge_index));
        node = previous;
    }
    steps.reverse();
    Some(steps)
}

fn zone_steps(scene: &Scene, heights: &[i32], path: &[(usize, usize, usize)]) -> Vec<MoveStep> {
    path.iter()
        .map(|&(from, to, edge_index)| {
            zone_step(scene, heights, from, to, &scene.edges[edge_index])
        })
        .collect()
}

fn zone_step(
    scene: &Scene,
    heights: &[i32],
    from: usize,
    to: usize,
    edge: &crate::content::schema::Edge,
) -> MoveStep {
    MoveStep {
        position: Position::Zone {
            zone: scene.zones[to].id.clone(),
        },
        cost: zone_edge_cost(edge, true),
        edge_distance: edge.distance,
        climb: edge.climb.clone(),
        locked: edge.door.as_ref().is_some_and(|door| door.locked),
        elevation_change: heights[to] - heights[from],
    }
}

/// Zone-graph implementation of [`Space`].
pub struct ZoneGraph<'a> {
    scene: &'a Scene,
    cache: ZoneCacheRef<'a>,
}

enum ZoneCacheRef<'a> {
    Borrowed(&'a ZoneCache),
    Owned(Box<ZoneCache>),
}

impl<'a> ZoneGraph<'a> {
    pub fn new(scene: &'a Scene) -> Self {
        Self {
            scene,
            cache: ZoneCacheRef::Owned(Box::new(ZoneCache::build(scene))),
        }
    }

    pub fn cached(scene: &'a Scene, cache: &'a ZoneCache) -> Self {
        Self {
            scene,
            cache: ZoneCacheRef::Borrowed(cache),
        }
    }

    fn cache(&self) -> &ZoneCache {
        match &self.cache {
            ZoneCacheRef::Borrowed(cache) => cache,
            ZoneCacheRef::Owned(cache) => cache,
        }
    }
}

impl Space for ZoneGraph<'_> {
    fn mode(&self) -> SceneMode {
        SceneMode::Zones
    }

    fn distance(&self, from: &Position, to: &Position) -> Option<i32> {
        let cache = self.cache();
        let from = cache.position_index(from)?;
        let to = cache.position_index(to)?;
        cache.range[from][to]
    }

    fn travel_distance(&self, from: &Position, to: &Position) -> Option<i32> {
        let cache = self.cache();
        let from = cache.position_index(from)?;
        let to = cache.position_index(to)?;
        cache.travel[from][to]
    }

    fn can_reach(&self, from: &Position, to: &Position, reach: i32) -> bool {
        let cache = self.cache();
        let (Some(from), Some(to)) = (cache.position_index(from), cache.position_index(to)) else {
            return false;
        };
        let reach = reach.max(5);
        if from == to {
            return cache.heights[from] == cache.heights[to];
        }
        cache.direct[from][to].is_some_and(|distance| {
            distance.saturating_add((cache.heights[from] - cache.heights[to]).abs()) <= reach
        })
    }

    fn cover(&self, attacker: &Position, target: &Position) -> Cover {
        let cache = self.cache();
        let (Some(attacker), Some(target)) =
            (cache.position_index(attacker), cache.position_index(target))
        else {
            return Cover::None;
        };
        if attacker == target {
            return Cover::None;
        }
        cache.covers[target]
    }

    fn light_at(&self, at: &Position) -> Light {
        let cache = self.cache();
        cache
            .position_index(at)
            .and_then(|index| cache.lights[index])
            .or(cache.scene_light)
            .unwrap_or(Light::Bright)
    }

    fn move_route(
        &self,
        from: &Position,
        to: &Position,
        _blocked: &[Position],
    ) -> Option<Vec<MoveStep>> {
        let cache = self.cache();
        let from = cache.position_index(from)?;
        let to = cache.position_index(to)?;
        cache.routes[from][to].clone()
    }

    fn neighbours(&self, at: &Position) -> Vec<MoveStep> {
        let cache = self.cache();
        cache
            .position_index(at)
            .map(|index| cache.neighbours[index].clone())
            .unwrap_or_default()
    }

    fn area_positions(&self, center: &Position, radius: i32) -> Vec<Position> {
        self.cache()
            .area(center, radius)
            .map(<[Position]>::to_vec)
            .unwrap_or_default()
    }

    fn zone_positions(&self, zone: &str) -> Vec<Position> {
        let cache = self.cache();
        cache
            .index
            .get(zone)
            .map(|index| vec![cache.positions[*index].clone()])
            .unwrap_or_default()
    }

    fn all_positions(&self) -> Vec<Position> {
        self.cache().positions.clone()
    }

    fn zone_of(&self, at: &Position) -> Option<String> {
        at.zone_id().map(ToString::to_string)
    }

    fn first_zone(&self) -> Option<String> {
        self.scene.zones.first().map(|zone| zone.id.clone())
    }

    fn has_zone(&self, zone: &str) -> bool {
        self.cache().index.contains_key(zone)
    }

    fn contains(&self, at: &Position) -> bool {
        at.zone_id()
            .is_some_and(|zone| self.cache().index.contains_key(zone))
    }

    fn elevation(&self, at: &Position) -> i32 {
        let cache = self.cache();
        cache
            .position_index(at)
            .map(|index| cache.heights[index])
            .unwrap_or(0)
    }

    fn opportunity_threat(&self, observer: &Position, departure: &Position) -> bool {
        match (observer.zone_id(), departure.zone_id()) {
            (Some(observer), Some(departure)) => observer == departure,
            _ => false,
        }
    }

    fn adjacent(&self, a: &Position, b: &Position) -> bool {
        let cache = self.cache();
        let (Some(a), Some(b)) = (cache.position_index(a), cache.position_index(b)) else {
            return false;
        };
        if a == b {
            return true;
        }
        cache.direct[a][b].is_some_and(|distance| distance <= 5)
    }
}

/// Parsed, immutable view of a scene grid.
#[derive(Debug, Clone, PartialEq)]
pub struct GridMap {
    pub cell: i32,
    pub width: i32,
    pub height: i32,
    cells: Vec<crate::content::schema::GridCell>,
    zone_tags: Vec<Option<String>>,
    zones: BTreeSet<String>,
}

impl GridMap {
    pub fn parse(scene: &Scene) -> Option<Self> {
        let grid = scene.grid.as_ref()?;
        let cell = grid.cell.unwrap_or(5).max(1);
        let height = grid.rows.len() as i32;
        let width = grid
            .rows
            .iter()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(0) as i32;
        if width == 0 || height == 0 {
            return None;
        }
        let zone_chars: BTreeMap<char, String> = grid
            .zones
            .iter()
            .filter_map(|(zone, character)| {
                character
                    .chars()
                    .next()
                    .map(|character| (character, zone.clone()))
            })
            .collect();
        let mut cells = Vec::with_capacity((width * height) as usize);
        let mut zone_tags = Vec::with_capacity((width * height) as usize);
        let mut zones = BTreeSet::new();
        for row in &grid.rows {
            let characters: Vec<char> = row.chars().collect();
            for x in 0..width as usize {
                let character = characters.get(x).copied().unwrap_or(' ');
                let key = character.to_string();
                let definition = grid.legend.get(&key).cloned().unwrap_or_default();
                let zone = definition
                    .zone
                    .clone()
                    .or_else(|| zone_chars.get(&character).cloned());
                if let Some(zone) = &zone {
                    zones.insert(zone.clone());
                }
                cells.push(definition);
                zone_tags.push(zone);
            }
        }
        zones.extend(grid.zones.keys().cloned());
        Some(Self {
            cell,
            width,
            height,
            cells,
            zone_tags,
            zones,
        })
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            None
        } else {
            Some((y * self.width + x) as usize)
        }
    }

    pub fn cell_at(&self, x: i32, y: i32) -> Option<&crate::content::schema::GridCell> {
        self.index(x, y).map(|index| &self.cells[index])
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some()
    }

    pub fn is_wall(&self, x: i32, y: i32) -> bool {
        self.cell_at(x, y).is_none_or(|cell| cell.wall)
    }

    pub fn zone_at(&self, x: i32, y: i32) -> Option<String> {
        self.zone_at_ref(x, y).map(ToString::to_string)
    }

    pub fn zone_at_ref(&self, x: i32, y: i32) -> Option<&str> {
        self.index(x, y)
            .and_then(|index| self.zone_tags[index].as_deref())
    }

    pub fn display_zone(&self, x: i32, y: i32) -> String {
        self.zone_at(x, y).unwrap_or_else(|| OPEN_ZONE.to_string())
    }

    pub fn cells_in_zone(&self, zone: &str) -> Vec<(i32, i32)> {
        let mut cells = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                if self.zone_at(x, y).as_deref() == Some(zone) {
                    cells.push((x, y));
                }
            }
        }
        cells
    }

    pub fn zone_ids(&self) -> Vec<String> {
        self.zones.iter().cloned().collect()
    }

    pub fn open_cells(&self) -> Vec<(i32, i32)> {
        let mut cells = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                if !self.is_wall(x, y) {
                    cells.push((x, y));
                }
            }
        }
        cells
    }

    fn point(&self, position: &Position) -> Option<(i32, i32)> {
        match position {
            Position::Cell { x, y } => Some((*x, *y)),
            Position::Zone { zone } => self.cells_in_zone(zone).first().copied(),
        }
    }

    fn neighbour_cells(&self, x: i32, y: i32) -> Vec<(i32, i32)> {
        let mut cells = Vec::with_capacity(8);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (nx, ny) = (x + dx, y + dy);
                if self.in_bounds(nx, ny) && !self.is_wall(nx, ny) {
                    cells.push((nx, ny));
                }
            }
        }
        cells
    }

    fn movement_cost(&self, x: i32, y: i32) -> i32 {
        let difficult = self.cell_at(x, y).is_some_and(|cell| cell.difficult);
        if difficult {
            self.cell.saturating_mul(2)
        } else {
            self.cell
        }
    }

    fn bresenham(&self, from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
        let (x0, y0) = from;
        let (x1, y1) = to;
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut error = dx + dy;
        let (mut x, mut y) = (x0, y0);
        let mut cells = vec![(x, y)];
        while x != x1 || y != y1 {
            let doubled = 2 * error;
            if doubled >= dy {
                error += dy;
                x += sx;
            }
            if doubled <= dx {
                error += dx;
                y += sy;
            }
            cells.push((x, y));
        }
        cells
    }

    fn line_blocked(&self, from: (i32, i32), to: (i32, i32)) -> bool {
        let line = self.bresenham(from, to);
        for window in line.windows(2) {
            let (x, y) = window[0];
            let (nx, ny) = window[1];
            if (nx, ny) == to {
                break;
            }
            if self.is_wall(nx, ny) {
                return true;
            }
            if nx != x && ny != y && self.is_wall(nx, y) && self.is_wall(x, ny) {
                return true;
            }
        }
        false
    }

    fn cover_between(&self, from: (i32, i32), to: (i32, i32)) -> Cover {
        if from == to {
            return Cover::None;
        }
        if self.line_blocked(from, to) {
            return Cover::Total;
        }
        let mut cover = Cover::None;
        let adjacent = (from.0 - to.0).abs() <= 1 && (from.1 - to.1).abs() <= 1;
        for &(x, y) in self
            .bresenham(from, to)
            .iter()
            .skip(1)
            .take_while(|cell| **cell != to)
        {
            if let Some(cell) = self.cell_at(x, y) {
                cover = max_cover(cover, cell.cover);
            }
        }
        if !adjacent {
            if let Some(cell) = self.cell_at(to.0, to.1) {
                cover = max_cover(cover, cell.cover);
            }
        }
        cover
    }
}

fn max_cover(a: Cover, b: Cover) -> Cover {
    fn rank(cover: Cover) -> u8 {
        match cover {
            Cover::None => 0,
            Cover::Half => 1,
            Cover::ThreeQuarters => 2,
            Cover::Total => 3,
        }
    }
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

/// Grid implementation of [`Space`].
pub struct GridSpace<'a> {
    scene: &'a Scene,
    map: GridMapRef<'a>,
}

enum GridMapRef<'a> {
    Borrowed(&'a GridMap),
    Owned(Box<GridMap>),
}

impl std::ops::Deref for GridMapRef<'_> {
    type Target = GridMap;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(map) => map,
            Self::Owned(map) => map,
        }
    }
}

impl<'a> GridSpace<'a> {
    pub fn new(scene: &'a Scene) -> Option<Self> {
        GridMap::parse(scene).map(|map| Self {
            scene,
            map: GridMapRef::Owned(Box::new(map)),
        })
    }

    pub fn cached(scene: &'a Scene, map: &'a GridMap) -> Self {
        Self {
            scene,
            map: GridMapRef::Borrowed(map),
        }
    }

    pub fn map(&self) -> &GridMap {
        &self.map
    }

    fn dijkstra(
        &self,
        from: (i32, i32),
        to: (i32, i32),
        blocked: &[Position],
    ) -> Option<(i32, Vec<(i32, i32)>)> {
        if self.map.is_wall(to.0, to.1) {
            return None;
        }
        let blocked: BTreeSet<(i32, i32)> =
            blocked.iter().filter_map(Position::cell_coords).collect();
        if blocked.contains(&to) {
            return None;
        }
        if from == to {
            return Some((0, Vec::new()));
        }
        let mut best: HashMap<(i32, i32), i32> = HashMap::new();
        let mut parents: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
        let mut queue = BinaryHeap::new();
        best.insert(from, 0);
        queue.push(Reverse((0i32, from)));
        while let Some(Reverse((cost, current))) = queue.pop() {
            if current == to {
                let mut route = Vec::new();
                let mut node = to;
                while node != from {
                    route.push(node);
                    node = *parents.get(&node)?;
                }
                route.reverse();
                return Some((cost, route));
            }
            if cost > best.get(&current).copied().unwrap_or(i32::MAX) {
                continue;
            }
            for (x, y) in self.map.neighbour_cells(current.0, current.1) {
                if blocked.contains(&(x, y)) {
                    continue;
                }
                let candidate = cost.saturating_add(self.map.movement_cost(x, y));
                let known = best.get(&(x, y)).copied().unwrap_or(i32::MAX);
                if candidate < known {
                    best.insert((x, y), candidate);
                    parents.insert((x, y), current);
                    queue.push(Reverse((candidate, (x, y))));
                }
            }
        }
        None
    }
}

impl Space for GridSpace<'_> {
    fn mode(&self) -> SceneMode {
        SceneMode::Grid
    }

    fn distance(&self, from: &Position, to: &Position) -> Option<i32> {
        let (from, to) = (self.map.point(from)?, self.map.point(to)?);
        let steps = (from.0 - to.0).abs().max((from.1 - to.1).abs());
        let elevation = |x, y| {
            self.map
                .cell_at(x, y)
                .and_then(|cell| cell.elevation)
                .unwrap_or(0)
        };
        Some(
            steps
                .saturating_mul(self.map.cell)
                .saturating_add((elevation(from.0, from.1) - elevation(to.0, to.1)).abs()),
        )
    }

    fn travel_distance(&self, from: &Position, to: &Position) -> Option<i32> {
        let (from, to) = (self.map.point(from)?, self.map.point(to)?);
        self.dijkstra(from, to, &[]).map(|(cost, _)| cost)
    }

    fn can_reach(&self, from: &Position, to: &Position, reach: i32) -> bool {
        self.distance(from, to)
            .is_some_and(|distance| distance <= reach.max(5))
    }

    fn cover(&self, attacker: &Position, target: &Position) -> Cover {
        let (Some(attacker), Some(target)) = (self.map.point(attacker), self.map.point(target))
        else {
            return Cover::None;
        };
        self.map.cover_between(attacker, target)
    }

    fn light_at(&self, _at: &Position) -> Light {
        self.scene.light.unwrap_or(Light::Bright)
    }

    fn move_route(
        &self,
        from: &Position,
        to: &Position,
        blocked: &[Position],
    ) -> Option<Vec<MoveStep>> {
        let (from, to) = (self.map.point(from)?, self.map.point(to)?);
        if self.map.is_wall(to.0, to.1) {
            return None;
        }
        let (_, route) = self.dijkstra(from, to, blocked)?;
        let mut previous = from;
        Some(
            route
                .into_iter()
                .map(|(x, y)| {
                    let step = MoveStep {
                        position: Position::Cell { x, y },
                        cost: self.map.movement_cost(x, y),
                        edge_distance: self.map.cell,
                        climb: None,
                        locked: false,
                        elevation_change: self
                            .map
                            .cell_at(x, y)
                            .and_then(|cell| cell.elevation)
                            .unwrap_or(0)
                            - self
                                .map
                                .cell_at(previous.0, previous.1)
                                .and_then(|cell| cell.elevation)
                                .unwrap_or(0),
                    };
                    previous = (x, y);
                    step
                })
                .collect(),
        )
    }

    fn neighbours(&self, at: &Position) -> Vec<MoveStep> {
        let Some((x, y)) = self.map.point(at) else {
            return Vec::new();
        };
        let elevation = |x, y| {
            self.map
                .cell_at(x, y)
                .and_then(|cell| cell.elevation)
                .unwrap_or(0)
        };
        self.map
            .neighbour_cells(x, y)
            .into_iter()
            .map(|(nx, ny)| MoveStep {
                position: Position::Cell { x: nx, y: ny },
                cost: self.map.movement_cost(nx, ny),
                edge_distance: self.map.cell,
                climb: None,
                locked: false,
                elevation_change: elevation(nx, ny) - elevation(x, y),
            })
            .collect()
    }

    fn area_positions(&self, center: &Position, radius: i32) -> Vec<Position> {
        let mut centers = match center {
            Position::Cell { x, y } => vec![(*x, *y)],
            Position::Zone { zone } => self.map.cells_in_zone(zone),
        };
        centers.sort_unstable();
        centers.dedup();
        let max_steps = radius.max(0) / self.map.cell;
        let mut positions = Vec::new();
        for (cx, cy) in centers {
            for y in 0..self.map.height {
                for x in 0..self.map.width {
                    let steps = (x - cx).abs().max((y - cy).abs());
                    if steps <= max_steps && !positions.contains(&Position::Cell { x, y }) {
                        positions.push(Position::Cell { x, y });
                    }
                }
            }
        }
        positions
    }

    fn zone_positions(&self, zone: &str) -> Vec<Position> {
        self.map
            .cells_in_zone(zone)
            .into_iter()
            .map(|(x, y)| Position::Cell { x, y })
            .collect()
    }

    fn all_positions(&self) -> Vec<Position> {
        self.map
            .open_cells()
            .into_iter()
            .map(|(x, y)| Position::Cell { x, y })
            .collect()
    }

    fn zone_of(&self, at: &Position) -> Option<String> {
        match at {
            Position::Cell { x, y } => self.map.zone_at(*x, *y),
            Position::Zone { zone } => Some(zone.clone()),
        }
    }

    fn first_zone(&self) -> Option<String> {
        self.scene
            .zones
            .first()
            .map(|zone| zone.id.clone())
            .or_else(|| self.map.zone_ids().into_iter().next())
    }

    fn has_zone(&self, zone: &str) -> bool {
        self.scene.zones.iter().any(|known| known.id == zone)
            || self.map.zone_ids().iter().any(|known| known == zone)
    }

    fn contains(&self, at: &Position) -> bool {
        match at {
            Position::Cell { x, y } => !self.map.is_wall(*x, *y),
            Position::Zone { zone } => self.has_zone(zone),
        }
    }

    fn elevation(&self, at: &Position) -> i32 {
        match at {
            Position::Cell { x, y } => self
                .map
                .cell_at(*x, *y)
                .and_then(|cell| cell.elevation)
                .unwrap_or(0),
            Position::Zone { zone } => self
                .map
                .cells_in_zone(zone)
                .first()
                .and_then(|(x, y)| self.map.cell_at(*x, *y))
                .and_then(|cell| cell.elevation)
                .unwrap_or(0),
        }
    }

    fn opportunity_threat(&self, observer: &Position, departure: &Position) -> bool {
        let (Some(observer), Some(departure)) =
            (self.map.point(observer), self.map.point(departure))
        else {
            return false;
        };
        (observer.0 - departure.0).abs() <= 1 && (observer.1 - departure.1).abs() <= 1
    }

    fn adjacent(&self, a: &Position, b: &Position) -> bool {
        let (Some(a), Some(b)) = (self.map.point(a), self.map.point(b)) else {
            return false;
        };
        (a.0 - b.0).abs() <= 1 && (a.1 - b.1).abs() <= 1
    }
}

impl SpaceView<'_> {
    /// Zone tag at a position without allocating. Zone scenes return the
    /// position's own zone id; grid scenes look up the cell's legend zone.
    pub fn zone_tag<'a>(&'a self, at: &'a Position) -> Option<&'a str> {
        match self {
            Self::Zones(_) => at.zone_id(),
            Self::Grid(space) => match at {
                Position::Cell { x, y } => space.map().zone_at_ref(*x, *y),
                Position::Zone { zone } => Some(zone.as_str()),
            },
        }
    }

    /// Cached position list for zone scenes. Grid callers fall back to
    /// [`Space::all_positions`].
    pub fn positions_slice(&self) -> Option<&[Position]> {
        match self {
            Self::Zones(space) => Some(space.cache().positions()),
            Self::Grid(_) => None,
        }
    }

    /// Cached area for zone scenes. Grid callers fall back to
    /// [`Space::area_positions`].
    pub fn area_slice(&self, center: &Position, radius: i32) -> Option<&[Position]> {
        match self {
            Self::Zones(space) => space.cache().area(center, radius),
            Self::Grid(_) => None,
        }
    }

    /// Movement cost from `from` to `to` without materialising a route.
    ///
    /// Zone graphs ignore `blocked` (as their routes always have); grids
    /// honour it, matching [`Space::move_route`] exactly.
    pub fn route_cost(&self, from: &Position, to: &Position, blocked: &[Position]) -> Option<i32> {
        match self {
            Self::Zones(space) => space.travel_distance(from, to),
            Self::Grid(space) => {
                let (from, to) = (space.map.point(from)?, space.map.point(to)?);
                space.dijkstra(from, to, blocked).map(|(cost, _)| cost)
            }
        }
    }
}

/// Precomputed spatial data derived from a scene.
///
/// The state owns one cache behind an `Arc`, so every Monte Carlo clone shares
/// the same matrices instead of rebuilding them per run.
#[derive(Debug, Default)]
pub struct SpaceCache {
    zones: Option<ZoneCache>,
    grid: Option<GridMap>,
}

impl SpaceCache {
    pub fn build(scene: Option<&Scene>) -> Self {
        let Some(scene) = scene else {
            return Self::default();
        };
        match scene.mode {
            SceneMode::Zones => Self {
                zones: Some(ZoneCache::build(scene)),
                grid: None,
            },
            SceneMode::Grid => Self {
                zones: None,
                grid: GridMap::parse(scene),
            },
        }
    }

    /// Cheap, borrow-only view over the cached scene data.
    pub fn view<'a>(&'a self, scene: &'a Scene) -> Option<SpaceView<'a>> {
        match scene.mode {
            SceneMode::Zones => self
                .zones
                .as_ref()
                .map(|cache| SpaceView::Zones(ZoneGraph::cached(scene, cache))),
            SceneMode::Grid => self
                .grid
                .as_ref()
                .map(|map| SpaceView::Grid(GridSpace::cached(scene, map))),
        }
    }
}

// ---------------------------------------------------------------------------
// P2 query kernel (TASK-0087)
// ---------------------------------------------------------------------------
//
// These queries are pure: they read a profile and explicit geometry inputs and
// never mutate an encounter. They coexist with the legacy `Space` trait above;
// P5 is the only task that wires them into engine/AI/CLI. Units follow the
// frozen contract: x/y are 5-ft cells, z/height/reach are feet.

/// Inclusive bound for one area dimension in feet. It keeps shape iteration
/// from allocating without bound and matches the validation style in P1.
pub const MAX_AREA_FEET: i32 = 1_000;

/// The metric a [`Distance`] answer used. Callers must choose one; the kernel
/// never silently substitutes range distance for travel or zone distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistancePolicy {
    Range,
    TravelStep,
    Vertical,
    ZoneGraph,
}

/// A structured explanation entry for a query result. Codes are stable and
/// `detail` is for DM logs / diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explain {
    pub code: &'static str,
    pub detail: String,
}

impl Explain {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// A distance in feet, tagged with the policy that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distance {
    pub policy: DistancePolicy,
    pub feet: i32,
    pub squares: i32,
    pub explain: Vec<Explain>,
}

/// A geometric reach answer. It never checks walls, cover or occupancy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    pub reachable: bool,
    pub distance: Distance,
}

/// Reach combined with line of sight. Use this when a caller wants the
/// contract's "reach plus sight" behaviour in one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackReach {
    pub reachable: bool,
    pub distance_ft: i32,
    pub sight: LineOfSight,
}

/// Why an occupancy query was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupancyReason {
    OutOfBounds,
    Terrain,
    Occupied,
    TooNarrow,
}

/// Result of `can_occupy`. `blockers` holds stable ids (bounds/terrain/creature)
/// so a DM log can explain the refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupancy {
    pub allowed: bool,
    pub reason: Option<OccupancyReason>,
    pub blockers: Vec<String>,
    pub explain: Vec<Explain>,
}

/// Why a travel step is unavailable. P3 (#172) owns multi-step pathfinding;
/// P2 only classifies a single step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TravelBlock {
    Wall,
    Occupied,
    TooNarrow,
    NoPath,
    OutOfBounds,
}

/// Terrain modifiers for one travel step. `difficult` is the caller's
/// Difficult Terrain flag; 2024 narrow openings use it instead of a separate
/// squeeze multiplier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TravelTerrain {
    pub difficult: bool,
    pub narrow_opening: bool,
    pub climb_swim_crawl: bool,
}

/// One travel step: a tagged distance plus an optional block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TravelStep {
    pub distance: Distance,
    pub blocked: Option<TravelBlock>,
}

/// Line-of-sight result with stable blocker ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineOfSight {
    pub clear: bool,
    pub blocked_by: Vec<String>,
    pub explain: Vec<Explain>,
}

/// Cover result: only the most protective degree, with every source at that
/// degree. Multiple sources never add degrees together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverResult {
    pub degree: Cover,
    pub sources: Vec<String>,
    pub explain: Vec<Explain>,
}

/// A body used by footprint/volume queries. `height_ft` is the vertical extent
/// above `pose.cell.z`; `size` drives sharing rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialBody {
    pub id: String,
    pub pose: SpatialPose,
    pub height_ft: i32,
    pub size: Size,
}

impl SpatialBody {
    pub fn new(id: impl Into<String>, pose: SpatialPose, size: Size) -> Self {
        Self {
            id: id.into(),
            pose,
            height_ft: Volume::for_size(size).height_ft,
            size,
        }
    }

    pub fn with_height(mut self, height_ft: i32) -> Self {
        self.height_ft = height_ft;
        self
    }

    /// Every footprint cell at the pose's altitude.
    pub fn cells(&self) -> Vec<CellPos> {
        footprint_cells(self.pose.cell, self.pose.footprint)
    }

    /// Altitude of the top of the volume in feet.
    pub fn top_ft(&self) -> i32 {
        self.pose.cell.z.saturating_add(self.height_ft)
    }
}

/// Optional grid extent for occupancy queries. `None` means an unbounded
/// synthetic field (useful for pure geometry tests and zone fallbacks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridBounds {
    pub width: i32,
    pub height: i32,
}

impl GridBounds {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }
}

/// A solid obstacle used by LOS and cover. `height_ft` extends upward from
/// `cell.z`; a low wall (3 ft) gives cover without blocking a Medium eye line
/// (5 ft). `blocks_sight` is the explicit "opaque" flag; height decides whether
/// a particular line passes over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialBlocker {
    pub id: String,
    pub cell: CellPos,
    pub footprint: Footprint,
    pub height_ft: i32,
    pub kind: BlockerKind,
    pub blocks_sight: bool,
    pub cover: Cover,
}

/// Why a blocker exists. Creature blockers use the profile's
/// `creature_cover` degree rather than their stored `cover` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockerKind {
    Wall,
    LowObstacle,
    Creature,
    Terrain,
    Illusion,
    ClosedDoor,
}

impl SpatialBlocker {
    pub fn new(
        id: impl Into<String>,
        cell: CellPos,
        footprint: Footprint,
        height_ft: i32,
        kind: BlockerKind,
    ) -> Result<Self, SpatialError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(SpatialError::EmptyBlockerId);
        }
        cell.validate()?;
        footprint.validate()?;
        if height_ft <= 0 || height_ft > MAX_VOLUME_HEIGHT_FT {
            return Err(SpatialError::InvalidHeight { height_ft });
        }
        let (blocks_sight, cover) = match kind {
            BlockerKind::Wall | BlockerKind::ClosedDoor => (true, Cover::Total),
            BlockerKind::LowObstacle | BlockerKind::Creature | BlockerKind::Terrain => {
                (true, Cover::Half)
            }
            BlockerKind::Illusion => (true, Cover::None),
        };
        Ok(Self {
            id,
            cell,
            footprint,
            height_ft,
            kind,
            blocks_sight,
            cover,
        })
    }

    pub fn wall(
        id: impl Into<String>,
        cell: CellPos,
        footprint: Footprint,
        height_ft: i32,
    ) -> Result<Self, SpatialError> {
        Self::new(id, cell, footprint, height_ft, BlockerKind::Wall)
    }

    pub fn low_wall(
        id: impl Into<String>,
        cell: CellPos,
        footprint: Footprint,
        height_ft: i32,
    ) -> Result<Self, SpatialError> {
        Self::new(id, cell, footprint, height_ft, BlockerKind::LowObstacle)
    }

    pub fn creature(body: &SpatialBody, blocks_sight: bool) -> Result<Self, SpatialError> {
        let mut blocker = Self::new(
            &body.id,
            body.pose.cell,
            body.pose.footprint,
            body.height_ft,
            BlockerKind::Creature,
        )?;
        blocker.blocks_sight = blocks_sight;
        blocker.cover = Cover::Half;
        Ok(blocker)
    }

    pub fn terrain(volume: &TerrainVolume, cover: Cover) -> Result<Self, SpatialError> {
        Self::new(
            &volume.id,
            volume.cell,
            volume.footprint,
            volume.height_ft,
            BlockerKind::Terrain,
        )
        .map(|blocker| blocker.with_cover(cover))
    }

    pub fn with_cover(mut self, cover: Cover) -> Self {
        self.cover = cover;
        self
    }

    pub fn with_sight(mut self, blocks_sight: bool) -> Self {
        self.blocks_sight = blocks_sight;
        self
    }

    pub fn cells(&self) -> Vec<CellPos> {
        footprint_cells(self.cell, self.footprint)
    }

    pub fn top_ft(&self) -> i32 {
        self.cell.z.saturating_add(self.height_ft)
    }
}

/// Result of a shape query; cells are sorted by (x, y, z) and deduplicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaResult {
    pub shape: AreaShape,
    pub origin: CellPos,
    pub cells: Vec<CellPos>,
    pub explain: Vec<Explain>,
}

/// How a target volume intersects an area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaIntersection {
    pub intersects: bool,
    /// Target footprint cells covered by at least one area cell that is not
    /// behind total cover.
    pub covered_cells: Vec<CellPos>,
    /// Number of footprint cells in the target body.
    pub target_cells: usize,
    pub fully_covered: bool,
    /// Some overlap existed, but every overlapping cell had total cover.
    pub total_cover_blocked: bool,
    pub explain: Vec<Explain>,
}

/// Zone-mode area answer. It deliberately carries zones, not grid cells, so a
/// caller cannot mistake a graph approximation for a metric shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneArea {
    pub zones: Vec<String>,
    pub approximation: String,
    pub explain: Vec<Explain>,
}

// -- geometry helpers -------------------------------------------------------

fn size_rank(size: Size) -> u8 {
    match size {
        Size::Tiny => 0,
        Size::Small => 1,
        Size::Medium | Size::Unknown => 2,
        Size::Large => 3,
        Size::Huge => 4,
        Size::Gargantuan => 5,
    }
}

fn footprint_cells(cell: CellPos, footprint: Footprint) -> Vec<CellPos> {
    let mut cells = Vec::with_capacity((footprint.width * footprint.depth) as usize);
    for dy in 0..footprint.depth {
        for dx in 0..footprint.width {
            cells.push(CellPos {
                x: cell.x.saturating_add(dx as i32),
                y: cell.y.saturating_add(dy as i32),
                z: cell.z,
            });
        }
    }
    cells
}

fn footprints_overlap(
    a: CellPos,
    a_footprint: Footprint,
    b: CellPos,
    b_footprint: Footprint,
) -> bool {
    let ax0 = i64::from(a.x);
    let ay0 = i64::from(a.y);
    let bx0 = i64::from(b.x);
    let by0 = i64::from(b.y);
    ax0 < bx0 + i64::from(b_footprint.width)
        && bx0 < ax0 + i64::from(a_footprint.width)
        && ay0 < by0 + i64::from(b_footprint.depth)
        && by0 < ay0 + i64::from(a_footprint.depth)
}

fn vertical_overlap(a_base: i32, a_height: i32, b_base: i32, b_height: i32) -> bool {
    i64::from(a_base) < i64::from(b_base) + i64::from(b_height)
        && i64::from(b_base) < i64::from(a_base) + i64::from(a_height)
}

fn cell_label(cell: CellPos) -> String {
    format!("({},{},{})", cell.x, cell.y, cell.z)
}

fn position_label(position: &Position) -> String {
    match position {
        Position::Zone { zone } => format!("zone `{zone}`"),
        Position::Cell { x, y } => format!("cell ({x},{y})"),
    }
}

fn line_cells(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let (x0, y0) = from;
    let (x1, y1) = to;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    let (mut x, mut y) = (x0, y0);
    let mut cells = vec![(x, y)];
    while x != x1 || y != y1 {
        let doubled = 2 * error;
        if doubled >= dy {
            error += dy;
            x += sx;
        }
        if doubled <= dx {
            error += dx;
            y += sy;
        }
        cells.push((x, y));
    }
    cells
}

fn line_z_at(from_z: i32, to_z: i32, index: usize, len: usize) -> i32 {
    if len <= 1 {
        return from_z;
    }
    let steps = (len - 1) as i64;
    let offset = (i64::from(to_z) - i64::from(from_z)) * index as i64 / steps;
    (i64::from(from_z) + offset) as i32
}

fn interior_line_cells(from: CellPos, to: CellPos) -> Vec<(i32, i32)> {
    let line = line_cells((from.x, from.y), (to.x, to.y));
    let len = line.len();
    if len <= 2 {
        Vec::new()
    } else {
        line[1..len - 1].to_vec()
    }
}

fn blocker_covers(blocker: &SpatialBlocker, x: i32, y: i32) -> bool {
    i64::from(x) >= i64::from(blocker.cell.x)
        && i64::from(y) >= i64::from(blocker.cell.y)
        && i64::from(x) < i64::from(blocker.cell.x) + i64::from(blocker.footprint.width)
        && i64::from(y) < i64::from(blocker.cell.y) + i64::from(blocker.footprint.depth)
}

fn blocker_degree(blocker: &SpatialBlocker, profile: &SpatialProfile) -> Cover {
    match blocker.kind {
        BlockerKind::Creature => profile.creature_cover,
        _ => blocker.cover,
    }
}

fn cover_rank(cover: Cover) -> u8 {
    match cover {
        Cover::None => 0,
        Cover::Half => 1,
        Cover::ThreeQuarters => 2,
        Cover::Total => 3,
    }
}

fn most_protective(pairs: Vec<(String, Cover)>) -> (Cover, Vec<String>) {
    let mut best = Cover::None;
    for (_, degree) in &pairs {
        best = max_cover(best, *degree);
    }
    let mut sources: Vec<String> = pairs
        .into_iter()
        .filter(|(_, degree)| *degree == best)
        .map(|(id, _)| id)
        .collect();
    sources.sort();
    sources.dedup();
    (best, sources)
}

fn sight_candidates(from: &SpatialBody, to: &SpatialBody) -> Vec<(CellPos, CellPos)> {
    let mut from_cells = from.cells();
    let mut to_cells = to.cells();
    from_cells.sort();
    from_cells.dedup();
    to_cells.sort();
    to_cells.dedup();
    let mut candidates = Vec::with_capacity(from_cells.len() * to_cells.len());
    for from_cell in from_cells {
        for &to_cell in &to_cells {
            candidates.push((from_cell, to_cell));
        }
    }
    candidates
}

struct SightSegment<'a> {
    from: CellPos,
    from_eye_ft: i32,
    to: CellPos,
    to_eye_ft: i32,
    blockers: &'a [SpatialBlocker],
    from_id: &'a str,
    to_id: &'a str,
}

fn segment_blockers_filtered(
    segment: &SightSegment<'_>,
    predicate: impl Fn(&SpatialBlocker) -> bool,
) -> BTreeSet<String> {
    let line = line_cells(
        (segment.from.x, segment.from.y),
        (segment.to.x, segment.to.y),
    );
    let len = line.len();
    let mut blocked = BTreeSet::new();
    if len <= 2 {
        return blocked;
    }
    for (index, (x, y)) in line.iter().enumerate().skip(1).take(len - 2) {
        let z = line_z_at(segment.from_eye_ft, segment.to_eye_ft, index, len);
        for blocker in segment.blockers {
            if !predicate(blocker)
                || !blocker.blocks_sight
                || blocker.id == segment.from_id
                || blocker.id == segment.to_id
                || !blocker_covers(blocker, *x, *y)
            {
                continue;
            }
            if z >= blocker.cell.z && z <= blocker.top_ft() {
                blocked.insert(blocker.id.clone());
            }
        }
    }
    blocked
}

fn bodies_adjacent(a: &SpatialBody, b: &SpatialBody) -> bool {
    a.cells().iter().any(|a_cell| {
        b.cells()
            .iter()
            .any(|b_cell| (a_cell.x - b_cell.x).abs() <= 1 && (a_cell.y - b_cell.y).abs() <= 1)
    })
}

fn line_cover_pairs(
    from: CellPos,
    to: CellPos,
    source: &SpatialBody,
    target: &SpatialBody,
    blockers: &[SpatialBlocker],
    profile: &SpatialProfile,
) -> Vec<(String, Cover)> {
    let interior = interior_line_cells(from, to);
    // Cover applies when the blocker's vertical span reaches the lower body
    // and starts at or below the higher body's top.
    let lower = source.pose.cell.z.max(target.pose.cell.z);
    let upper = source.top_ft().min(target.top_ft());
    let mut pairs = Vec::new();
    for blocker in blockers {
        if blocker.id == source.id || blocker.id == target.id {
            continue;
        }
        if blocker.top_ft() < lower || blocker.cell.z > upper {
            continue;
        }
        if !interior
            .iter()
            .any(|(x, y)| blocker_covers(blocker, *x, *y))
        {
            continue;
        }
        let degree = blocker_degree(blocker, profile);
        if degree != Cover::None {
            pairs.push((blocker.id.clone(), degree));
        }
    }
    pairs
}

fn target_cell_cover_pairs(
    source: &SpatialBody,
    target: &SpatialBody,
    blockers: &[SpatialBlocker],
    profile: &SpatialProfile,
) -> Vec<(String, Cover)> {
    let target_cells = target.cells();
    let mut pairs = Vec::new();
    for blocker in blockers {
        if blocker.id == source.id || blocker.id == target.id {
            continue;
        }
        if blocker.top_ft() < target.pose.cell.z {
            continue;
        }
        if !target_cells
            .iter()
            .any(|cell| blocker_covers(blocker, cell.x, cell.y))
        {
            continue;
        }
        let degree = blocker_degree(blocker, profile);
        if degree != Cover::None {
            pairs.push((blocker.id.clone(), degree));
        }
    }
    pairs
}

// -- distance and reach -----------------------------------------------------

fn ground_distance(
    from: &SpatialPose,
    to: &SpatialPose,
    profile: &SpatialProfile,
) -> (i32, i32, &'static str) {
    let dx = (i64::from(to.cell.x) - i64::from(from.cell.x))
        .abs()
        .min(i64::from(i32::MAX)) as i32;
    let dy = (i64::from(to.cell.y) - i64::from(from.cell.y))
        .abs()
        .min(i64::from(i32::MAX)) as i32;
    let squares = dx.max(dy);
    match profile.distance_3d {
        Distance3d::Chebyshev => match profile.diagonal {
            Diagonal::Five => (
                squares.saturating_mul(FEET_PER_CELL),
                squares,
                "chebyshev-five",
            ),
            Diagonal::FiveTenFive => {
                let diagonal = dx.min(dy);
                let pairs = diagonal / 2;
                let extra = diagonal % 2;
                let straight = (dx - dy).abs();
                let feet = pairs
                    .saturating_mul(15)
                    .saturating_add(extra.saturating_mul(FEET_PER_CELL))
                    .saturating_add(straight.saturating_mul(FEET_PER_CELL));
                (feet, squares, "five-ten-five")
            }
        },
        Distance3d::ManhattanPlusVertical => (
            dx.saturating_add(dy).saturating_mul(FEET_PER_CELL),
            squares,
            "manhattan-plus-vertical",
        ),
    }
}

/// Range distance in feet. Pure geometry: it ignores walls, cover and
/// occupancy. Vertical difference is always added; `FiveTenFive` affects the
/// ground component only, with a deterministic x-then-y path rule.
pub fn range_distance(from: &SpatialPose, to: &SpatialPose, profile: &SpatialProfile) -> Distance {
    let (ground, squares, ground_rule) = ground_distance(from, to, profile);
    let vertical = (i64::from(to.cell.z) - i64::from(from.cell.z)).abs() as i32;
    let feet = ground.saturating_add(vertical);
    let mut explain = vec![Explain::new("range-ground", ground_rule)];
    if vertical != 0 {
        explain.push(Explain::new(
            "range-vertical",
            format!("altitude difference {vertical} ft"),
        ));
    }
    Distance {
        policy: DistancePolicy::Range,
        feet,
        squares,
        explain,
    }
}

/// Vertical distance only, in feet.
pub fn vertical_distance(from: &SpatialPose, to: &SpatialPose) -> Distance {
    let feet = (i64::from(to.cell.z) - i64::from(from.cell.z)).abs() as i32;
    Distance {
        policy: DistancePolicy::Vertical,
        feet,
        squares: 0,
        explain: vec![Explain::new(
            "vertical",
            format!("altitude difference {feet} ft"),
        )],
    }
}

/// One travel step's cost. This is not a pathfinder: P3 (#172) owns routes,
/// Dijkstra/A* and creature traversal. The caller supplies the terrain flags.
pub fn travel_step_cost(
    from: &SpatialPose,
    to: &SpatialPose,
    profile: &SpatialProfile,
    terrain: TravelTerrain,
) -> TravelStep {
    let (base, squares, ground_rule) = ground_distance(from, to, profile);
    travel_cost_from_base(base, squares, ground_rule, profile, terrain)
}

/// Shared cost kernel for one travel step. `base_ft` is the edge distance
/// before terrain multipliers. P3 passes a 5-10-5-adjusted base here so the
/// diagonal parity cost goes through the same multiplier stack as the kernel.
fn travel_cost_from_base(
    base_ft: i32,
    squares: i32,
    ground_rule: &'static str,
    profile: &SpatialProfile,
    terrain: TravelTerrain,
) -> TravelStep {
    let mut feet = base_ft;
    let mut explain = vec![
        Explain::new("travel-base", ground_rule),
        Explain::new(
            "travel-step",
            "single step only; multi-step planning uses MovementPlanner",
        ),
    ];
    if terrain.difficult {
        feet = feet.saturating_mul(2);
        explain.push(Explain::new("travel-difficult", "Difficult Terrain x2"));
    }
    if terrain.climb_swim_crawl {
        feet = feet.saturating_mul(2);
        explain.push(Explain::new(
            "travel-climb",
            "climb/swim/crawl costs 1 extra ft per ft",
        ));
    }
    if terrain.narrow_opening {
        match profile.squeeze {
            Squeeze::Edition2014 => {
                feet = feet.saturating_mul(2);
                explain.push(Explain::new("travel-squeeze-2014", "2014 squeeze x2"));
            }
            Squeeze::Edition2024 => {
                explain.push(Explain::new(
                    "travel-squeeze-2024",
                    "2024 narrow opening is Difficult Terrain; caller sets `difficult`",
                ));
            }
            Squeeze::Forbid => {
                explain.push(Explain::new("travel-too-narrow", "squeeze is forbidden"));
                return TravelStep {
                    distance: Distance {
                        policy: DistancePolicy::TravelStep,
                        feet: 0,
                        squares,
                        explain,
                    },
                    blocked: Some(TravelBlock::TooNarrow),
                };
            }
        }
    }
    TravelStep {
        distance: Distance {
            policy: DistancePolicy::TravelStep,
            feet,
            squares,
            explain,
        },
        blocked: None,
    }
}

/// Geometric reach only: `distance <= reach_ft`. It never checks walls, cover
/// or occupancy; combine it with [`line_of_sight`] (or use
/// [`can_reach_with_sight`]) when visibility matters.
///
/// This function measures the contract's pose-to-pose distance. Use
/// [`body_distance`] / [`can_reach_bodies`] when a footprint's nearest cell
/// matters.
pub fn can_reach(
    from: &SpatialPose,
    to: &SpatialPose,
    reach_ft: i32,
    profile: &SpatialProfile,
) -> Reach {
    let mut distance = range_distance(from, to, profile);
    let reachable = distance.feet <= reach_ft.max(0);
    distance.explain.push(Explain::new(
        "reach-geometric",
        "reach ignores walls, cover and occupancy; combine with line_of_sight",
    ));
    Reach {
        reachable,
        distance,
    }
}

/// Minimum cell-to-cell range distance between two bodies. This is the
/// footprint-aware primitive: a Large target's near edge can be in reach even
/// when its anchor cell is not.
pub fn body_distance(from: &SpatialBody, to: &SpatialBody, profile: &SpatialProfile) -> Distance {
    let mut best: Option<(i32, i32, CellPos, CellPos)> = None;
    for from_cell in from.cells() {
        for to_cell in to.cells() {
            let from_pose = SpatialPose {
                cell: from_cell,
                footprint: Footprint::ONE,
                facing: None,
            };
            let to_pose = SpatialPose {
                cell: to_cell,
                footprint: Footprint::ONE,
                facing: None,
            };
            let distance = range_distance(&from_pose, &to_pose, profile);
            if best.is_none_or(|(known, _, _, _)| distance.feet < known) {
                best = Some((distance.feet, distance.squares, from_cell, to_cell));
            }
        }
    }
    let (feet, squares, from_cell, to_cell) = best.unwrap_or((0, 0, from.pose.cell, to.pose.cell));
    Distance {
        policy: DistancePolicy::Range,
        feet,
        squares,
        explain: vec![Explain::new(
            "body-distance",
            format!(
                "nearest footprint cells {} -> {}",
                cell_label(from_cell),
                cell_label(to_cell)
            ),
        )],
    }
}

/// Footprint-aware reach. Same geometric-only contract as [`can_reach`].
pub fn can_reach_bodies(
    from: &SpatialBody,
    to: &SpatialBody,
    reach_ft: i32,
    profile: &SpatialProfile,
) -> Reach {
    let mut distance = body_distance(from, to, profile);
    let reachable = distance.feet <= reach_ft.max(0);
    distance.explain.push(Explain::new(
        "reach-geometric",
        "reach ignores walls, cover and occupancy; combine with line_of_sight",
    ));
    Reach {
        reachable,
        distance,
    }
}

/// Reach plus sight. The pure reach query stays separate so callers can see
/// the two decisions in their logs.
pub fn can_reach_with_sight(
    from: &SpatialBody,
    to: &SpatialBody,
    reach_ft: i32,
    blockers: &[SpatialBlocker],
    profile: &SpatialProfile,
) -> AttackReach {
    let reach = can_reach_bodies(from, to, reach_ft, profile);
    let sight = line_of_sight(from, to, blockers, profile);
    AttackReach {
        reachable: reach.reachable && sight.clear,
        distance_ft: reach.distance.feet,
        sight,
    }
}

// -- occupancy --------------------------------------------------------------

fn can_share_cell(mover: &SpatialBody, other: &SpatialBody, profile: &SpatialProfile) -> bool {
    if profile.tiny_sharing && mover.size == Size::Tiny && other.size == Size::Tiny {
        return true;
    }
    size_rank(mover.size) > size_rank(other.size)
}

/// May a body end its move here? Checks every footprint cell, the vertical
/// volume, terrain volumes and other bodies. Moving *through* a hostile's
/// space is P3's traversal rule; this query answers the end-of-move question
/// from the contract (O-05).
pub fn can_occupy(
    body: &SpatialBody,
    bounds: Option<GridBounds>,
    terrain: &[TerrainVolume],
    occupants: &[SpatialBody],
    profile: &SpatialProfile,
) -> Result<Occupancy, SpatialError> {
    body.pose.validate()?;
    if body.height_ft <= 0 || body.height_ft > MAX_VOLUME_HEIGHT_FT {
        return Err(SpatialError::InvalidHeight {
            height_ft: body.height_ft,
        });
    }
    let cells = body.cells();
    let mut explain = vec![Explain::new(
        "occupancy-footprint",
        format!("{} footprint cells at {} ft", cells.len(), body.pose.cell.z),
    )];

    if let Some(bounds) = bounds {
        if cells.iter().any(|cell| !bounds.contains(cell.x, cell.y)) {
            explain.push(Explain::new(
                "occupancy-out-of-bounds",
                "footprint leaves the grid",
            ));
            return Ok(Occupancy {
                allowed: false,
                reason: Some(OccupancyReason::OutOfBounds),
                blockers: vec!["bounds".to_string()],
                explain,
            });
        }
    }

    let mut terrain_blockers = BTreeSet::new();
    for volume in terrain {
        volume.validate()?;
        if footprints_overlap(
            body.pose.cell,
            body.pose.footprint,
            volume.cell,
            volume.footprint,
        ) && vertical_overlap(
            body.pose.cell.z,
            body.height_ft,
            volume.cell.z,
            volume.height_ft,
        ) {
            terrain_blockers.insert(volume.id.clone());
        }
    }
    if !terrain_blockers.is_empty() {
        explain.push(Explain::new(
            "occupancy-terrain",
            "solid terrain overlaps the footprint volume",
        ));
        return Ok(Occupancy {
            allowed: false,
            reason: Some(OccupancyReason::Terrain),
            blockers: terrain_blockers.into_iter().collect(),
            explain,
        });
    }

    let mut occupant_blockers = BTreeSet::new();
    for occupant in occupants {
        if occupant.id == body.id {
            continue;
        }
        occupant.pose.validate()?;
        if occupant.height_ft <= 0 || occupant.height_ft > MAX_VOLUME_HEIGHT_FT {
            return Err(SpatialError::InvalidHeight {
                height_ft: occupant.height_ft,
            });
        }
        if !footprints_overlap(
            body.pose.cell,
            body.pose.footprint,
            occupant.pose.cell,
            occupant.pose.footprint,
        ) || !vertical_overlap(
            body.pose.cell.z,
            body.height_ft,
            occupant.pose.cell.z,
            occupant.height_ft,
        ) {
            continue;
        }
        if !can_share_cell(body, occupant, profile) {
            occupant_blockers.insert(occupant.id.clone());
        }
    }
    if !occupant_blockers.is_empty() {
        explain.push(Explain::new(
            "occupancy-occupied",
            "the body may not end in another body's space",
        ));
        return Ok(Occupancy {
            allowed: false,
            reason: Some(OccupancyReason::Occupied),
            blockers: occupant_blockers.into_iter().collect(),
            explain,
        });
    }

    explain.push(Explain::new("occupancy-ok", "footprint volume is free"));
    Ok(Occupancy {
        allowed: true,
        reason: None,
        blockers: Vec::new(),
        explain,
    })
}

// -- line of sight and cover ------------------------------------------------

/// Line of sight from one volume to another.
///
/// Large footprints produce several candidate paths (one per source cell x
/// target cell). Sight is clear when at least one path is clear; the first
/// clear path in (source cell, target cell) order is reported. When every path
/// is blocked, `blocked_by` is the sorted union of blockers so a DM log can
/// explain the result. LOS is edition-neutral; `_profile` is reserved for
/// future size-based blockers.
pub fn line_of_sight(
    from: &SpatialBody,
    to: &SpatialBody,
    blockers: &[SpatialBlocker],
    _profile: &SpatialProfile,
) -> LineOfSight {
    let candidates = sight_candidates(from, to);
    let mut blocked_union = BTreeSet::new();
    let mut first_clear = None;
    for (from_cell, to_cell) in &candidates {
        let segment = SightSegment {
            from: *from_cell,
            from_eye_ft: from.top_ft(),
            to: *to_cell,
            to_eye_ft: to.top_ft(),
            blockers,
            from_id: &from.id,
            to_id: &to.id,
        };
        let blocked = segment_blockers_filtered(&segment, |_| true);
        if blocked.is_empty() {
            first_clear = Some((*from_cell, *to_cell));
            break;
        }
        blocked_union.extend(blocked);
    }

    let mut explain = vec![Explain::new(
        "los-candidates",
        format!("{} candidate paths", candidates.len()),
    )];
    if let Some((from_cell, to_cell)) = first_clear {
        explain.push(Explain::new(
            "los-candidate",
            format!(
                "first clear path {} -> {}",
                cell_label(from_cell),
                cell_label(to_cell)
            ),
        ));
        LineOfSight {
            clear: true,
            blocked_by: Vec::new(),
            explain,
        }
    } else {
        explain.push(Explain::new(
            "los-blocked",
            format!("all {} candidate paths are blocked", candidates.len()),
        ));
        LineOfSight {
            clear: false,
            blocked_by: blocked_union.into_iter().collect(),
            explain,
        }
    }
}

/// Cover against attacks from `source` to `target`.
///
/// Multiple sources keep only the most protective degree. A blocker grants
/// cover when it is between the bodies and its vertical span overlaps the
/// combatants' shared band; a low wall can therefore give cover without
/// blocking sight. Creature blockers use `profile.creature_cover`.
///
/// The 2014 profile keeps the legacy "target cell cover when not adjacent"
/// rule; the 2024 profile applies the SRD opposite-side rule and only counts
/// blockers on the line between the combatants.
pub fn cover_between(
    source: &SpatialBody,
    to: &SpatialBody,
    blockers: &[SpatialBlocker],
    profile: &SpatialProfile,
) -> CoverResult {
    let candidates = sight_candidates(source, to);
    let mut explain = vec![Explain::new(
        "cover-candidates",
        format!("{} candidate paths", candidates.len()),
    )];
    if candidates.is_empty() {
        explain.push(Explain::new("cover-none", "no valid footprint cells"));
        return CoverResult {
            degree: Cover::None,
            sources: Vec::new(),
            explain,
        };
    }

    let mut clear = Vec::new();
    let mut blocked_union = BTreeSet::new();
    for (from_cell, to_cell) in &candidates {
        let segment = SightSegment {
            from: *from_cell,
            from_eye_ft: source.top_ft(),
            to: *to_cell,
            to_eye_ft: to.top_ft(),
            blockers,
            from_id: &source.id,
            to_id: &to.id,
        };
        let blocked = segment_blockers_filtered(&segment, |_| true);
        if blocked.is_empty() {
            clear.push((*from_cell, *to_cell));
        } else {
            blocked_union.extend(blocked);
        }
    }

    let mut best_degree = Cover::None;
    let mut best_sources: BTreeSet<String> = BTreeSet::new();
    let mut chosen = None;
    for (from_cell, to_cell) in &clear {
        let pairs = line_cover_pairs(*from_cell, *to_cell, source, to, blockers, profile);
        let (degree, sources) = most_protective(pairs);
        if chosen.is_none() || cover_rank(degree) < cover_rank(best_degree) {
            best_degree = degree;
            best_sources = sources.into_iter().collect();
            chosen = Some((*from_cell, *to_cell));
        } else if cover_rank(degree) == cover_rank(best_degree) {
            best_sources.extend(sources);
        }
    }

    if let Some((from_cell, to_cell)) = chosen {
        explain.push(Explain::new(
            "cover-path",
            format!(
                "least-cover clear path {} -> {}",
                cell_label(from_cell),
                cell_label(to_cell)
            ),
        ));
    }

    // 2014 legacy: a target-cell obstacle gives cover when the bodies are not
    // adjacent. 2024 uses the opposite-side rule only. P1 has no separate
    // edition tag on `SpatialProfile`, so the squeeze policy (whose defaults
    // are edition-specific) is the edition marker.
    if profile.squeeze == Squeeze::Edition2014 && !bodies_adjacent(source, to) {
        for (id, degree) in target_cell_cover_pairs(source, to, blockers, profile) {
            if cover_rank(degree) > cover_rank(best_degree) {
                best_degree = degree;
                best_sources = BTreeSet::from([id]);
            } else if degree == best_degree && degree != Cover::None {
                best_sources.insert(id);
            }
        }
        explain.push(Explain::new(
            "cover-2014-target-cell",
            "2014 target-cell cover applies (non-adjacent)",
        ));
    }

    if chosen.is_none() {
        let total_sources: BTreeSet<String> = blockers
            .iter()
            .filter(|blocker| blocked_union.contains(&blocker.id) && blocker.cover == Cover::Total)
            .map(|blocker| blocker.id.clone())
            .collect();
        if !total_sources.is_empty() {
            best_degree = Cover::Total;
            best_sources = total_sources;
            explain.push(Explain::new(
                "cover-los-total",
                "sight is fully blocked by total cover",
            ));
        } else {
            // Sight is blocked by something that is not total cover (for
            // example a sight-blocking creature). It can still provide its
            // cover degree; use the first deterministic candidate unless the
            // 2014 target-cell rule already found a source.
            if best_sources.is_empty() {
                let (from_cell, to_cell) = candidates[0];
                let pairs = line_cover_pairs(from_cell, to_cell, source, to, blockers, profile);
                let (degree, sources) = most_protective(pairs);
                if degree != Cover::None {
                    best_degree = degree;
                    best_sources = sources.into_iter().collect();
                }
            }
            explain.push(Explain::new(
                "cover-los-blocked",
                "sight is blocked, but no blocking source grants total cover",
            ));
        }
    }

    if best_sources.is_empty() {
        explain.push(Explain::new("cover-none", "no source grants cover"));
    } else {
        for source_id in &best_sources {
            explain.push(Explain::new("cover-source", source_id.clone()));
        }
    }
    CoverResult {
        degree: best_degree,
        sources: best_sources.into_iter().collect(),
        explain,
    }
}

// -- areas ------------------------------------------------------------------

fn forward_cell(origin: CellPos, facing: Facing, forward: i32, side: i32) -> CellPos {
    match facing {
        Facing::North => CellPos {
            x: origin.x.saturating_add(side),
            y: origin.y.saturating_sub(forward),
            z: origin.z,
        },
        Facing::East => CellPos {
            x: origin.x.saturating_add(forward),
            y: origin.y.saturating_add(side),
            z: origin.z,
        },
        Facing::South => CellPos {
            x: origin.x.saturating_add(side),
            y: origin.y.saturating_add(forward),
            z: origin.z,
        },
        Facing::West => CellPos {
            x: origin.x.saturating_sub(forward),
            y: origin.y.saturating_add(side),
            z: origin.z,
        },
    }
}

fn ceil_cells(feet: i32) -> i32 {
    (feet + FEET_PER_CELL - 1) / FEET_PER_CELL
}

fn insert_area_cell(cells: &mut BTreeSet<CellPos>, cell: CellPos) -> Result<(), SpatialError> {
    cell.validate()?;
    cells.insert(cell);
    Ok(())
}

/// Cells covered by an area, in deterministic (x, y, z) order.
///
/// Shapes are grid approximations: a 10-ft sphere is a 3D block of 5-ft cells,
/// a cone's far edge is as wide as its length, and a cube's origin sits on a
/// face. Cone/line/cube use `pose.facing` as the template direction; this is
/// not the optional combat-facing rule from the profile. The six P1 shapes are
/// supported; zone mode is handled by [`ZoneQuery::area`], which refuses shapes
/// a graph cannot represent.
pub fn area_cells(
    area: &AreaSpecV1,
    origin: &SpatialPose,
    profile: &SpatialProfile,
) -> Result<AreaResult, SpatialError> {
    profile.validate_area(area)?;
    if area.size_ft > MAX_AREA_FEET || area.width_ft.is_some_and(|width| width > MAX_AREA_FEET) {
        return Err(SpatialError::AreaTooLarge {
            size_ft: area.size_ft,
            limit_ft: MAX_AREA_FEET,
        });
    }
    origin.validate()?;

    let mut cells = BTreeSet::new();
    let mut explain = vec![
        Explain::new("area-shape", format!("{:?}", area.shape)),
        Explain::new("area-grid", "5-ft cells; not a continuous volume"),
    ];

    match area.shape {
        AreaShape::Sphere | AreaShape::Emanation => {
            let radius = i64::from(area.size_ft);
            let radius_cells = ceil_cells(area.size_ft);
            for dz_cells in -radius_cells..=radius_cells {
                let dz = i64::from(dz_cells) * i64::from(FEET_PER_CELL);
                let z = i64::from(origin.cell.z) + dz;
                if z < 0 {
                    continue;
                }
                for dx_cells in -radius_cells..=radius_cells {
                    let dx = i64::from(dx_cells) * i64::from(FEET_PER_CELL);
                    for dy_cells in -radius_cells..=radius_cells {
                        let dy = i64::from(dy_cells) * i64::from(FEET_PER_CELL);
                        if dx * dx + dy * dy + dz * dz > radius * radius {
                            continue;
                        }
                        insert_area_cell(
                            &mut cells,
                            CellPos {
                                x: origin.cell.x.saturating_add(dx_cells),
                                y: origin.cell.y.saturating_add(dy_cells),
                                z: z as i32,
                            },
                        )?;
                    }
                }
            }
            if area.shape == AreaShape::Emanation {
                cells.remove(&origin.cell);
                explain.push(Explain::new("area-origin", "emanation excludes its origin"));
            } else {
                explain.push(Explain::new("area-origin", "sphere includes its origin"));
            }
        }
        AreaShape::Cylinder => {
            let radius = i64::from(area.size_ft);
            let radius_cells = ceil_cells(area.size_ft);
            let height = area.width_ft.unwrap_or(area.size_ft);
            let height_cells = ceil_cells(height.max(FEET_PER_CELL));
            for dz_cells in 0..height_cells {
                let z = origin
                    .cell
                    .z
                    .saturating_add(dz_cells.saturating_mul(FEET_PER_CELL));
                for dx_cells in -radius_cells..=radius_cells {
                    let dx = i64::from(dx_cells) * i64::from(FEET_PER_CELL);
                    for dy_cells in -radius_cells..=radius_cells {
                        let dy = i64::from(dy_cells) * i64::from(FEET_PER_CELL);
                        if dx * dx + dy * dy > radius * radius {
                            continue;
                        }
                        insert_area_cell(
                            &mut cells,
                            CellPos {
                                x: origin.cell.x.saturating_add(dx_cells),
                                y: origin.cell.y.saturating_add(dy_cells),
                                z,
                            },
                        )?;
                    }
                }
            }
            explain.push(Explain::new(
                "area-cylinder",
                format!("radius {} ft, height {} ft", area.size_ft, height),
            ));
        }
        AreaShape::Cone => {
            let facing = origin.facing.ok_or(SpatialError::MissingAreaDirection {
                shape: AreaShape::Cone,
            })?;
            let length_cells = ceil_cells(area.size_ft);
            for forward in 1..=length_cells {
                let half_width = forward / 2;
                for side in -half_width..=half_width {
                    let cell = forward_cell(origin.cell, facing, forward, side);
                    insert_area_cell(&mut cells, cell)?;
                }
            }
            explain.push(Explain::new(
                "area-cone",
                "origin excluded; far edge width equals length",
            ));
        }
        AreaShape::Line => {
            let facing = origin.facing.ok_or(SpatialError::MissingAreaDirection {
                shape: AreaShape::Line,
            })?;
            let length_cells = ceil_cells(area.size_ft);
            let width = area.width_ft.unwrap_or(FEET_PER_CELL);
            let width_cells = ceil_cells(width.max(FEET_PER_CELL));
            let left = (width_cells - 1) / 2;
            let right = width_cells / 2;
            for forward in 1..=length_cells {
                for side in -left..=right {
                    let cell = forward_cell(origin.cell, facing, forward, side);
                    insert_area_cell(&mut cells, cell)?;
                }
            }
            explain.push(Explain::new(
                "area-line",
                format!("origin excluded; width {width} ft"),
            ));
        }
        AreaShape::Cube => {
            let facing = origin.facing.ok_or(SpatialError::MissingAreaDirection {
                shape: AreaShape::Cube,
            })?;
            let side_cells = ceil_cells(area.size_ft);
            let left = (side_cells - 1) / 2;
            let right = side_cells / 2;
            for forward in 0..side_cells {
                for side in -left..=right {
                    for dz_cells in 0..side_cells {
                        let mut cell = forward_cell(origin.cell, facing, forward, side);
                        cell.z = origin
                            .cell
                            .z
                            .saturating_add(dz_cells.saturating_mul(FEET_PER_CELL));
                        insert_area_cell(&mut cells, cell)?;
                    }
                }
            }
            explain.push(Explain::new(
                "area-cube",
                "origin on a face; side length applies on all axes",
            ));
        }
    }

    Ok(AreaResult {
        shape: area.shape,
        origin: origin.cell,
        cells: cells.into_iter().collect(),
        explain,
    })
}

/// Intersect an area result with a target volume.
///
/// A cell counts when its x/y matches a target footprint cell and its z lies
/// inside the target's vertical band. Only total cover removes a cell from the
/// area; half and three-quarters cover do not reduce area damage (contract
/// A-08). The ray starts at the area origin and uses the origin body's eye
/// height.
pub fn area_intersection(
    area: &AreaResult,
    origin: &SpatialBody,
    target: &SpatialBody,
    blockers: &[SpatialBlocker],
) -> AreaIntersection {
    let target_cells = target.cells();
    let origin_cell = CellPos {
        x: area.origin.x,
        y: area.origin.y,
        z: origin.pose.cell.z,
    };
    let origin_eye = origin.top_ft();

    let mut overlapping = BTreeSet::new();
    let mut covered = BTreeSet::new();
    let mut total_blockers = BTreeSet::new();
    for target_cell in &target_cells {
        let target_band_top = target_cell.z.saturating_add(target.height_ft);
        let mut saw_overlap = false;
        let mut saw_clear = false;
        for area_cell in &area.cells {
            if area_cell.x != target_cell.x
                || area_cell.y != target_cell.y
                || area_cell.z < target_cell.z
                || area_cell.z >= target_band_top
            {
                continue;
            }
            saw_overlap = true;
            let segment = SightSegment {
                from: origin_cell,
                from_eye_ft: origin_eye,
                to: *area_cell,
                to_eye_ft: area_cell.z,
                blockers,
                from_id: &origin.id,
                to_id: &target.id,
            };
            let blocked = segment_blockers_filtered(&segment, |blocker| {
                blocker.cover == Cover::Total && blocker.blocks_sight
            });
            if blocked.is_empty() {
                saw_clear = true;
            } else {
                total_blockers.extend(blocked);
            }
        }
        if saw_overlap {
            overlapping.insert(*target_cell);
        }
        if saw_clear {
            covered.insert(*target_cell);
        }
    }

    let fully_covered = !target_cells.is_empty()
        && target_cells
            .iter()
            .all(|target_cell| covered.contains(target_cell));
    let total_cover_blocked = !overlapping.is_empty() && covered.is_empty();

    let mut explain = vec![
        Explain::new(
            "area-target-cells",
            format!("{} target footprint cells", target_cells.len()),
        ),
        Explain::new(
            "area-covered-cells",
            format!("{} target cells overlap", overlapping.len()),
        ),
    ];
    if total_cover_blocked {
        explain.push(Explain::new(
            "area-total-cover",
            format!("total cover blocks every cell: {:?}", total_blockers),
        ));
    } else if fully_covered {
        explain.push(Explain::new("area-full", "target volume is fully covered"));
    } else if !covered.is_empty() {
        explain.push(Explain::new(
            "area-partial",
            "target volume is partly covered",
        ));
    }

    AreaIntersection {
        intersects: !covered.is_empty(),
        covered_cells: covered.into_iter().collect(),
        target_cells: target_cells.len(),
        fully_covered,
        total_cover_blocked,
        explain,
    }
}

/// Zone-graph queries. A zone graph is not a metric grid: same zone is 0 ft
/// for reach/travel, LOS is a scene-level approximation, and only sphere and
/// emanation have zone-wide area answers.
pub struct ZoneQuery<'a> {
    space: &'a dyn Space,
}

impl<'a> ZoneQuery<'a> {
    pub fn new(space: &'a dyn Space) -> Self {
        Self { space }
    }

    pub fn distance(&self, from: &Position, to: &Position) -> Option<Distance> {
        let feet = self.space.distance(from, to)?;
        let mut explain = vec![Explain::new(
            "zone-distance",
            "zone-graph distance; same zone is 0 ft (display may say ~5 ft)",
        )];
        if feet == SAME_ZONE_DISTANCE {
            explain.push(Explain::new(
                "zone-same-zone",
                "both positions are in one zone",
            ));
        }
        Some(Distance {
            policy: DistancePolicy::ZoneGraph,
            feet,
            squares: 0,
            explain,
        })
    }

    pub fn can_reach(&self, from: &Position, to: &Position, reach_ft: i32) -> Reach {
        // Zone contract: same zone or any graph path within reach. The legacy
        // `Space::can_reach` only checks a direct edge, so the query kernel
        // uses the cached all-pairs distance instead.
        let distance = self.distance(from, to);
        let reachable = distance
            .as_ref()
            .is_some_and(|distance| distance.feet <= reach_ft.max(0));
        Reach {
            reachable,
            distance: distance.unwrap_or_else(|| Distance {
                policy: DistancePolicy::ZoneGraph,
                feet: 0,
                squares: 0,
                explain: vec![Explain::new(
                    "zone-missing",
                    "one or both positions are not in the zone graph",
                )],
            }),
        }
    }

    pub fn line_of_sight(&self, from: &Position, to: &Position) -> LineOfSight {
        if !self.space.contains(from) || !self.space.contains(to) {
            return LineOfSight {
                clear: false,
                blocked_by: Vec::new(),
                explain: vec![Explain::new(
                    "zone-los-missing",
                    "one or both positions are not in the zone graph",
                )],
            };
        }
        LineOfSight {
            clear: true,
            blocked_by: Vec::new(),
            explain: vec![Explain::new(
                "zone-los",
                "zone mode has no geometric sight blockers; scene LOS flags are the fallback",
            )],
        }
    }

    pub fn cover_between(&self, from: &Position, to: &Position) -> CoverResult {
        let degree = self.space.cover(from, to);
        let sources = if degree == Cover::None {
            Vec::new()
        } else {
            to.zone_id()
                .map(|zone| vec![zone.to_string()])
                .unwrap_or_default()
        };
        CoverResult {
            degree,
            sources,
            explain: vec![Explain::new(
                "zone-cover",
                "zone/edge cover flag; no geometric cover sources",
            )],
        }
    }

    pub fn can_occupy(&self, at: &Position, occupants: &[Position]) -> Occupancy {
        let mut explain = vec![Explain::new("zone-occupancy", "zone-level occupancy")];
        if !self.space.contains(at) {
            explain.push(Explain::new("zone-unknown", "position is not in the graph"));
            return Occupancy {
                allowed: false,
                reason: Some(OccupancyReason::OutOfBounds),
                blockers: Vec::new(),
                explain,
            };
        }
        if occupants.iter().any(|occupant| occupant == at) {
            explain.push(Explain::new(
                "zone-occupied",
                "another body is in this zone",
            ));
            return Occupancy {
                allowed: false,
                reason: Some(OccupancyReason::Occupied),
                blockers: Vec::new(),
                explain,
            };
        }
        explain.push(Explain::new("zone-ok", "zone is free"));
        Occupancy {
            allowed: true,
            reason: None,
            blockers: Vec::new(),
            explain,
        }
    }

    pub fn area(
        &self,
        origin: &Position,
        area: &AreaSpecV1,
        profile: &SpatialProfile,
    ) -> Result<ZoneArea, SpatialError> {
        profile.validate_area(area)?;
        if self.space.mode() != SceneMode::Zones
            || !matches!(area.shape, AreaShape::Sphere | AreaShape::Emanation)
        {
            return Err(SpatialError::UnsupportedInZone { shape: area.shape });
        }
        if !self.space.contains(origin) {
            return Err(SpatialError::UnknownPosition {
                position: position_label(origin),
            });
        }
        let mut zones = BTreeSet::new();
        for position in self.space.all_positions() {
            let Position::Zone { zone } = position else {
                return Err(SpatialError::UnsupportedInZone { shape: area.shape });
            };
            if area.shape == AreaShape::Emanation && origin.zone_id() == Some(zone.as_str()) {
                continue;
            }
            let Some(distance) = self
                .space
                .distance(origin, &Position::Zone { zone: zone.clone() })
            else {
                continue;
            };
            if distance <= area.size_ft {
                zones.insert(zone);
            }
        }
        Ok(ZoneArea {
            zones: zones.into_iter().collect(),
            approximation:
                "zone graph: same zone = 0 ft; radius uses graph edge distance; not a metric 3D area"
                    .to_string(),
            explain: vec![Explain::new(
                "zone-area",
                "zones whose graph distance from the origin is within the radius",
            )],
        })
    }
}

// ---------------------------------------------------------------------------
// P3 movement planner (TASK-0088)
// ---------------------------------------------------------------------------
//
// This is a planning seam, not an executor. It reads an explicit movement map,
// profile and bodies and returns plans; it never changes encounter state. P5
// (#174) is the only task that wires these results into engine/AI/CLI.
//
// Legacy `Space`/`GridSpace`/`ZoneGraph` behaviour is untouched: callers get
// the new semantics only by constructing a `MovementPlanner` explicitly, which
// is the "explicitly enabled profile" boundary from the frozen contract.
//
// The map is a 2D floor plan with a per-cell floor elevation. A grid step may
// change the floor by at most `MAX_ELEVATION_STEP_FT` (a climb); larger rises
// need a route and larger drops belong to P4 falling. Flight/altitude is P4.

/// One grid step may change the floor by at most this many feet.
pub const MAX_ELEVATION_STEP_FT: i32 = 5;

/// Where a movement cell's floor is: normal ground or water.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MovementMedium {
    #[default]
    Ground,
    Water,
}

/// State of a door in a movement cell. Closed and locked doors block entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DoorState {
    #[default]
    None,
    Open,
    Closed,
    Locked,
}

/// One cell of a synthetic movement map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MovementCell {
    /// A wall or an impassable volume.
    pub blocked: bool,
    /// Difficult Terrain: each foot costs one extra foot.
    pub difficult: bool,
    /// Floor height in feet above the map datum.
    pub elevation_ft: i32,
    pub medium: MovementMedium,
    pub door: DoorState,
}

impl MovementCell {
    pub fn open() -> Self {
        Self::default()
    }

    pub fn wall() -> Self {
        Self {
            blocked: true,
            ..Self::default()
        }
    }

    pub fn difficult() -> Self {
        Self {
            difficult: true,
            ..Self::default()
        }
    }

    pub fn water() -> Self {
        Self {
            medium: MovementMedium::Water,
            ..Self::default()
        }
    }

    pub fn elevated(height_ft: i32) -> Self {
        Self {
            elevation_ft: height_ft,
            ..Self::default()
        }
    }

    pub fn door(state: DoorState) -> Self {
        Self {
            door: state,
            ..Self::default()
        }
    }

    pub fn with_difficult(mut self) -> Self {
        self.difficult = true;
        self
    }

    pub fn with_elevation(mut self, height_ft: i32) -> Self {
        self.elevation_ft = height_ft;
        self
    }

    pub fn with_door(mut self, state: DoorState) -> Self {
        self.door = state;
        self
    }
}

/// A synthetic movement floor. Cells are stored row-major (`y * width + x`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementMap {
    width: i32,
    height: i32,
    cells: Vec<MovementCell>,
}

impl MovementMap {
    pub fn new(width: i32, height: i32, cells: Vec<MovementCell>) -> Result<Self, SpatialError> {
        let expected = i64::from(width) * i64::from(height);
        if width <= 0
            || height <= 0
            || width > MAX_GRID_COORD
            || height > MAX_GRID_COORD
            || expected != cells.len() as i64
        {
            return Err(SpatialError::InvalidMovementMap {
                width,
                height,
                cells: cells.len(),
            });
        }
        for cell in &cells {
            if !(0..=MAX_ALTITUDE_FT).contains(&cell.elevation_ft) {
                return Err(SpatialError::AltitudeOutOfRange {
                    z: cell.elevation_ft,
                });
            }
        }
        Ok(Self {
            width,
            height,
            cells,
        })
    }

    /// An all-open ground map. Dimensions must be positive and bounded.
    pub fn open(width: i32, height: i32) -> Self {
        assert!(
            width > 0 && height > 0 && width <= MAX_GRID_COORD && height <= MAX_GRID_COORD,
            "movement map dimensions must be within 1..=MAX_GRID_COORD"
        );
        let count = (width as usize).saturating_mul(height as usize);
        Self {
            width,
            height,
            cells: vec![MovementCell::open(); count],
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            None
        } else {
            Some((y * self.width + x) as usize)
        }
    }

    pub fn cell(&self, x: i32, y: i32) -> Option<MovementCell> {
        self.index(x, y).map(|index| self.cells[index])
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some()
    }

    pub fn bounds(&self) -> GridBounds {
        GridBounds {
            width: self.width,
            height: self.height,
        }
    }
}

/// Movement modes the planner can select for one edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveMode {
    Walk,
    Climb,
    Swim,
}

/// Speeds that remove the climb/swim movement penalty. Zero means "no matching
/// speed". Walk speed is the caller-provided budget, not a capability here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MoveCapabilities {
    pub climb_ft: i32,
    pub swim_ft: i32,
}

impl MoveCapabilities {
    pub fn with_climb(mut self, feet: i32) -> Self {
        self.climb_ft = feet;
        self
    }

    pub fn with_swim(mut self, feet: i32) -> Self {
        self.swim_ft = feet;
        self
    }

    pub fn has_climb(&self) -> bool {
        self.climb_ft > 0
    }

    pub fn has_swim(&self) -> bool {
        self.swim_ft > 0
    }
}

/// Another creature in the scene plus the two facts that change traversal:
/// whether it is an ally and whether it is Incapacitated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraversalCreature {
    pub body: SpatialBody,
    pub ally: bool,
    pub incapacitated: bool,
}

impl TraversalCreature {
    pub fn new(body: SpatialBody, ally: bool, incapacitated: bool) -> Self {
        Self {
            body,
            ally,
            incapacitated,
        }
    }

    pub fn ally(body: SpatialBody) -> Self {
        Self::new(body, true, false)
    }

    pub fn hostile(body: SpatialBody) -> Self {
        Self::new(body, false, false)
    }
}

/// An observer whose reach may be left by a voluntary move. The planner only
/// marks the potential trigger; visibility, reactions and order stay in P5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpportunityObserver {
    pub body: SpatialBody,
    pub reach_ft: i32,
}

impl OpportunityObserver {
    pub fn new(body: SpatialBody, reach_ft: i32) -> Self {
        Self { body, reach_ft }
    }
}

/// Why a move is unavailable. Variant order is the reporting priority used
/// when several frontier blocks compete for one explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MoveBlock {
    Door,
    LockedDoor,
    Wall,
    Occupied,
    EndOccupied,
    TooNarrow,
    OutOfBounds,
    Height,
    TooSteep,
    Drop,
    NoPath,
    NotAdjacent,
    InvalidPose,
}

impl MoveBlock {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Door => "door",
            Self::LockedDoor => "locked_door",
            Self::Wall => "wall",
            Self::Occupied => "occupied",
            Self::EndOccupied => "end_occupied",
            Self::TooNarrow => "too_narrow",
            Self::OutOfBounds => "out_of_bounds",
            Self::Height => "height",
            Self::TooSteep => "too_steep",
            Self::Drop => "drop",
            Self::NoPath => "no_path",
            Self::NotAdjacent => "not_adjacent",
            Self::InvalidPose => "invalid_pose",
        }
    }
}

/// A point the engine must process; the planner never resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveTrigger {
    /// The mover enters a cell; the engine checks area effects there.
    AreaEntry { at: CellPos },
    /// The edge squeezes through a narrow opening.
    Squeeze { at: CellPos, disadvantage: bool },
    /// The edge changes the floor height.
    Climb {
        from: CellPos,
        to: CellPos,
        elevation_change_ft: i32,
    },
    /// A voluntary move leaves an observer's reach (potential opportunity attack).
    LeavesReach {
        observer: String,
        from: CellPos,
        to: CellPos,
    },
}

/// One planned movement edge. Cost includes terrain multipliers; `distance_ft`
/// is the base edge distance (5-10-5 and Manhattan included) before them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementEdge {
    pub from: SpatialPose,
    pub to: SpatialPose,
    pub cost_ft: i32,
    pub distance_ft: i32,
    pub mode: MoveMode,
    pub diagonal: bool,
    pub elevation_change_ft: i32,
    pub difficult: bool,
    pub narrow_opening: bool,
    pub creature_space: bool,
    pub squeezed: bool,
    pub triggers: Vec<MoveTrigger>,
}

/// Result of evaluating one adjacent step. Blocked steps never carry an edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    pub from: SpatialPose,
    pub to: SpatialPose,
    pub edge: Option<MovementEdge>,
    pub blocked: Option<MoveBlock>,
    pub blockers: Vec<String>,
    pub explain: Vec<Explain>,
}

impl StepResult {
    pub fn allowed(&self) -> bool {
        self.blocked.is_none() && self.edge.is_some()
    }
}

/// Voluntary or forced movement. Forced movement neither spends the mover's
/// Speed nor provokes opportunity attacks, but still triggers area effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    Voluntary,
    Forced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveRules {
    pub consumes_speed: bool,
    pub provokes_opportunity_attacks: bool,
    pub triggers_area_effects: bool,
}

impl MoveKind {
    pub fn rules(self) -> MoveRules {
        match self {
            Self::Voluntary => MoveRules {
                consumes_speed: true,
                provokes_opportunity_attacks: true,
                triggers_area_effects: true,
            },
            Self::Forced => MoveRules {
                consumes_speed: false,
                provokes_opportunity_attacks: false,
                triggers_area_effects: true,
            },
        }
    }
}

/// A planned voluntary route. `total_cost_ft` is the sum of step costs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementPath {
    pub start: SpatialPose,
    pub goal: SpatialPose,
    pub steps: Vec<MovementEdge>,
    pub total_cost_ft: i32,
    pub kind: MoveKind,
    pub explain: Vec<Explain>,
}

/// One pose in a Dijkstra reachable set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachEntry {
    pub pose: SpatialPose,
    pub cost_ft: i32,
    pub diagonals_parity: u8,
    /// Whether the mover may stop here. Transit-only poses (for example an
    /// ally's space) are reachable but not legal end positions.
    pub end_allowed: bool,
    /// The pose itself needs a squeeze strip (the full footprint does not fit).
    pub squeezed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReachNode {
    cost_ft: i32,
    parity: u8,
    parent: Option<(CellPos, u8)>,
    edge: Option<MovementEdge>,
}

/// Every pose reachable within a speed budget, plus the parent data needed to
/// reconstruct a path to any entry. Entries are sorted by cost and coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachableSet {
    pub start: SpatialPose,
    pub budget_ft: i32,
    pub entries: Vec<ReachEntry>,
    pub start_blocked: Option<MoveBlock>,
    pub blockers: Vec<String>,
    pub explain: Vec<Explain>,
    nodes: BTreeMap<(CellPos, u8), ReachNode>,
}

impl ReachableSet {
    /// Cheapest reconstructed path to a pose in the set.
    pub fn path_to(&self, goal: &SpatialPose) -> Option<MovementPath> {
        if goal.footprint != self.start.footprint || goal.facing != self.start.facing {
            return None;
        }
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.pose.cell == goal.cell)?;
        let key = (entry.pose.cell, entry.diagonals_parity);
        Some(reconstruct_path(
            &self.nodes,
            key,
            self.start,
            *goal,
            MoveKind::Voluntary,
        ))
    }

    pub fn cost_to(&self, goal: &SpatialPose) -> Option<i32> {
        self.entries
            .iter()
            .find(|entry| entry.pose.cell == goal.cell)
            .map(|entry| entry.cost_ft)
    }

    pub fn contains(&self, goal: &SpatialPose) -> bool {
        self.cost_to(goal).is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathAlgorithm {
    Auto,
    Dijkstra,
    AStar,
}

/// The three structured terminal outcomes of a path query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanOutcome {
    /// Cheapest path fits the budget.
    Reached(MovementPath),
    /// Cheapest path exists but costs more than the budget.
    OverBudget(MovementPath),
    /// No route exists; `reason` and `blockers` explain the nearest frontier.
    Unreachable {
        reason: MoveBlock,
        blockers: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPlan {
    pub start: SpatialPose,
    pub goal: SpatialPose,
    pub budget_ft: i32,
    pub kind: MoveKind,
    /// The algorithm actually used. `Auto` reports A* or the Dijkstra fallback.
    pub algorithm: PathAlgorithm,
    pub outcome: PlanOutcome,
    pub nodes_expanded: usize,
    pub explain: Vec<Explain>,
}

impl PathPlan {
    pub fn is_reached(&self) -> bool {
        matches!(self.outcome, PlanOutcome::Reached(_))
    }

    pub fn path(&self) -> Option<&MovementPath> {
        match &self.outcome {
            PlanOutcome::Reached(path) | PlanOutcome::OverBudget(path) => Some(path),
            PlanOutcome::Unreachable { .. } => None,
        }
    }
}

/// Re-verification verdict for a previously planned path. Execution must run
/// this against current state before each step or at the start of a move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathVerdict {
    Valid,
    Blocked {
        step_index: usize,
        reason: MoveBlock,
        blockers: Vec<String>,
    },
    CostChanged {
        step_index: usize,
        planned_cost_ft: i32,
        actual_cost_ft: i32,
    },
    OverBudget {
        total_cost_ft: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathCheck {
    pub verdict: PathVerdict,
    pub checked_steps: usize,
    pub explain: Vec<Explain>,
}

/// End-position query result (contract O-05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndCheck {
    pub allowed: bool,
    pub reason: Option<MoveBlock>,
    pub blockers: Vec<String>,
    pub squeezed: bool,
    pub explain: Vec<Explain>,
}

/// A straight forced push/pull. `moved_ft` uses geometric distance, not the
/// mover's Speed; difficult terrain does not shorten a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForcedMovePlan {
    pub start: SpatialPose,
    pub direction: (i32, i32),
    pub distance_ft: i32,
    pub steps: Vec<MovementEdge>,
    pub moved_ft: i32,
    pub stopped_by: Option<MoveBlock>,
    pub rules: MoveRules,
    pub explain: Vec<Explain>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

#[derive(Debug, Clone)]
struct Placement {
    cells: Vec<(i32, i32)>,
    squeezed: bool,
    block: Option<(MoveBlock, Vec<String>)>,
}

impl Placement {
    fn fits(&self) -> bool {
        self.block.is_none()
    }
}

struct SearchResult {
    nodes: BTreeMap<(CellPos, u8), ReachNode>,
    goal_key: Option<(CellPos, u8)>,
    expanded: usize,
    goal_blocks: BTreeMap<MoveBlock, BTreeSet<String>>,
    frontier_blocks: BTreeMap<(u32, MoveBlock), BTreeSet<String>>,
}

const MOVE_DIRECTIONS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Ground-plane lower bound used by A*. Vertical is ignored because no edge is
/// cheaper than its ground component. `FiveTenFive` uses the cheapest
/// alternating diagonal pattern, which is a lower bound for any path.
pub fn movement_heuristic(from: &SpatialPose, to: &SpatialPose, profile: &SpatialProfile) -> i32 {
    let (ground, _, _) = ground_distance(from, to, profile);
    ground
}

/// A* is only used when the heuristic above is provably admissible. Every
/// profile combination the frozen contract can express is supported; a future
/// distance policy that cannot be proved must return `false` here and fall back
/// to Dijkstra.
pub fn astar_heuristic_is_admissible(profile: &SpatialProfile) -> bool {
    matches!(
        profile.distance_3d,
        Distance3d::Chebyshev | Distance3d::ManhattanPlusVertical
    )
}

/// Occupancy-, terrain- and footprint-aware movement planning over a
/// [`MovementMap`]. Constructing a planner is the explicit opt-in for the new
/// path semantics; legacy `Space` callers never reach this code until P5.
pub struct MovementPlanner<'a> {
    map: &'a MovementMap,
    profile: &'a SpatialProfile,
    mover: &'a SpatialBody,
    occupants: Vec<&'a TraversalCreature>,
    observers: Vec<&'a OpportunityObserver>,
    capabilities: MoveCapabilities,
}

impl<'a> MovementPlanner<'a> {
    pub fn new(map: &'a MovementMap, profile: &'a SpatialProfile, mover: &'a SpatialBody) -> Self {
        Self {
            map,
            profile,
            mover,
            occupants: Vec::new(),
            observers: Vec::new(),
            capabilities: MoveCapabilities::default(),
        }
    }

    pub fn with_capabilities(mut self, capabilities: MoveCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_occupants(mut self, occupants: &'a [TraversalCreature]) -> Self {
        self.occupants = occupants.iter().collect();
        self.occupants
            .sort_by(|left, right| left.body.id.cmp(&right.body.id));
        self
    }

    pub fn with_observers(mut self, observers: &'a [OpportunityObserver]) -> Self {
        self.observers = observers.iter().collect();
        self.observers
            .sort_by(|left, right| left.body.id.cmp(&right.body.id));
        self
    }

    fn pose_at(&self, cell: CellPos) -> SpatialPose {
        SpatialPose {
            cell,
            footprint: self.mover.pose.footprint,
            facing: self.mover.pose.facing,
        }
    }

    fn body_at(&self, pose: &SpatialPose) -> SpatialBody {
        SpatialBody {
            id: self.mover.id.clone(),
            pose: *pose,
            height_ft: self.mover.height_ft,
            size: self.mover.size,
        }
    }

    fn full_cells(&self, pose: &SpatialPose) -> Vec<(i32, i32)> {
        let mut cells = Vec::with_capacity((pose.footprint.width * pose.footprint.depth) as usize);
        for dy in 0..pose.footprint.depth {
            for dx in 0..pose.footprint.width {
                cells.push((
                    pose.cell.x.saturating_add(dx as i32),
                    pose.cell.y.saturating_add(dy as i32),
                ));
            }
        }
        cells
    }

    /// The one-size-smaller strip used by squeezing. Along X the strip keeps
    /// `depth - 1` rows anchored at the top; along Y it keeps `width - 1`
    /// columns anchored at the left. That is exactly one size smaller.
    fn strip_cells(&self, pose: &SpatialPose, axis: Axis) -> Vec<(i32, i32)> {
        let mut cells = Vec::new();
        match axis {
            Axis::X => {
                let rows = pose.footprint.depth.saturating_sub(1).max(1);
                for dy in 0..rows {
                    for dx in 0..pose.footprint.width {
                        cells.push((
                            pose.cell.x.saturating_add(dx as i32),
                            pose.cell.y.saturating_add(dy as i32),
                        ));
                    }
                }
            }
            Axis::Y => {
                let cols = pose.footprint.width.saturating_sub(1).max(1);
                for dy in 0..pose.footprint.depth {
                    for dx in 0..cols {
                        cells.push((
                            pose.cell.x.saturating_add(dx as i32),
                            pose.cell.y.saturating_add(dy as i32),
                        ));
                    }
                }
            }
        }
        cells
    }

    fn perpendicular_gt_one(&self, pose: &SpatialPose, axis: Axis) -> bool {
        match axis {
            Axis::X => pose.footprint.depth > 1,
            Axis::Y => pose.footprint.width > 1,
        }
    }

    fn cells_block(&self, cells: &[(i32, i32)], z: i32) -> Option<(MoveBlock, Vec<String>)> {
        let mut out_of_bounds = BTreeSet::new();
        let mut walls = BTreeSet::new();
        let mut doors = BTreeSet::new();
        let mut locked = BTreeSet::new();
        let mut heights = BTreeSet::new();
        for &(x, y) in cells {
            let Some(cell) = self.map.cell(x, y) else {
                out_of_bounds.insert(format!("cell:{x},{y}"));
                continue;
            };
            if cell.blocked {
                walls.insert(format!("cell:{x},{y}"));
                continue;
            }
            match cell.door {
                DoorState::Closed => {
                    doors.insert(format!("cell:{x},{y}"));
                    continue;
                }
                DoorState::Locked => {
                    locked.insert(format!("cell:{x},{y}"));
                    continue;
                }
                DoorState::None | DoorState::Open => {}
            }
            if cell.elevation_ft != z {
                heights.insert(format!("cell:{x},{y}"));
            }
        }
        if !out_of_bounds.is_empty() {
            return Some((MoveBlock::OutOfBounds, out_of_bounds.into_iter().collect()));
        }
        if !locked.is_empty() {
            return Some((MoveBlock::LockedDoor, locked.into_iter().collect()));
        }
        if !doors.is_empty() {
            return Some((MoveBlock::Door, doors.into_iter().collect()));
        }
        if !walls.is_empty() {
            return Some((MoveBlock::Wall, walls.into_iter().collect()));
        }
        if !heights.is_empty() {
            return Some((MoveBlock::Height, heights.into_iter().collect()));
        }
        None
    }

    /// Effective placement of a pose. The full footprint is preferred. If it
    /// is blocked and the movement axis has a perpendicular edge greater than
    /// one, the one-size-smaller strip is tried:
    /// - 2014/2024 squeeze: a fitting strip is a legal squeezed placement.
    /// - `Forbid`: a fitting strip is reported as `TooNarrow`.
    ///
    /// Out-of-bounds is never fixed by squeezing. A strip that avoids a wall,
    /// door or height seam in the rest of the footprint is legal; if the strip
    /// runs into the blocker too, that blocker is reported.
    fn placement(&self, pose: &SpatialPose, axis: Option<Axis>) -> Placement {
        let full = self.full_cells(pose);
        let Some(block) = self.cells_block(&full, pose.cell.z) else {
            return Placement {
                cells: full,
                squeezed: false,
                block: None,
            };
        };
        let Some(axis) = axis else {
            return Placement {
                cells: full,
                squeezed: false,
                block: Some(block),
            };
        };
        if !self.perpendicular_gt_one(pose, axis) || block.0 == MoveBlock::OutOfBounds {
            return Placement {
                cells: full,
                squeezed: false,
                block: Some(block),
            };
        }
        let strip = self.strip_cells(pose, axis);
        if let Some(strip_block) = self.cells_block(&strip, pose.cell.z) {
            return Placement {
                cells: strip,
                squeezed: false,
                block: Some(strip_block),
            };
        }
        if self.profile.squeeze == Squeeze::Forbid {
            return Placement {
                cells: strip,
                squeezed: false,
                block: Some((MoveBlock::TooNarrow, block.1)),
            };
        }
        Placement {
            cells: strip,
            squeezed: true,
            block: None,
        }
    }

    fn full_fits(&self, pose: &SpatialPose) -> bool {
        self.placement(pose, None).fits()
    }

    fn may_pass_through(&self, other: &TraversalCreature) -> bool {
        if other.incapacitated || other.ally || other.body.size == Size::Tiny {
            return true;
        }
        let mover = i32::from(size_rank(self.mover.size));
        let other_rank = i32::from(size_rank(other.body.size));
        (mover - other_rank).abs() >= 2
    }

    fn end_occupancy(&self, pose: &SpatialPose, cells: &[(i32, i32)]) -> (bool, Vec<String>) {
        let mut blockers = BTreeSet::new();
        for occupant in &self.occupants {
            if occupant.body.id == self.mover.id {
                continue;
            }
            let occupant_cells = occupant.body.cells();
            let overlaps = occupant_cells.iter().any(|occupant_cell| {
                cells.iter().any(|&(x, y)| {
                    occupant_cell.x == x
                        && occupant_cell.y == y
                        && vertical_overlap(
                            pose.cell.z,
                            self.mover.height_ft,
                            occupant_cell.z,
                            occupant.body.height_ft,
                        )
                })
            });
            if overlaps && !can_share_cell(&self.body_at(pose), &occupant.body, self.profile) {
                blockers.insert(occupant.body.id.clone());
            }
        }
        (blockers.is_empty(), blockers.into_iter().collect())
    }

    fn end_check(
        &self,
        pose: &SpatialPose,
        placement: &Placement,
        squeezed: bool,
        mut explain: Vec<Explain>,
    ) -> EndCheck {
        let (allowed, blockers) = self.end_occupancy(pose, &placement.cells);
        if allowed {
            explain.push(Explain::new(
                "end-ok",
                if squeezed {
                    "squeezed pose is a legal end position"
                } else {
                    "footprint volume is free"
                },
            ));
            EndCheck {
                allowed: true,
                reason: None,
                blockers: Vec::new(),
                squeezed,
                explain,
            }
        } else {
            explain.push(Explain::new(
                "end-occupied",
                "the body may not willingly end in another body's space",
            ));
            EndCheck {
                allowed: false,
                reason: Some(MoveBlock::EndOccupied),
                blockers,
                squeezed,
                explain,
            }
        }
    }

    /// May the mover end its move at `pose`? Checks bounds, every effective
    /// footprint cell, the floor level, doors/walls and O-05 occupancy.
    pub fn can_end_at(&self, pose: &SpatialPose) -> EndCheck {
        let explain = vec![Explain::new(
            "end-check",
            format!(
                "{}x{} footprint at ({},{},{})",
                pose.footprint.width, pose.footprint.depth, pose.cell.x, pose.cell.y, pose.cell.z
            ),
        )];
        if pose.validate().is_err() {
            return EndCheck {
                allowed: false,
                reason: Some(MoveBlock::InvalidPose),
                blockers: Vec::new(),
                squeezed: false,
                explain,
            };
        }
        let full = self.placement(pose, None);
        if full.fits() {
            return self.end_check(pose, &full, false, explain);
        }
        let (reason, blockers) = full
            .block
            .clone()
            .expect("a non-fitting placement carries a block");
        if reason != MoveBlock::OutOfBounds
            && (pose.footprint.width > 1 || pose.footprint.depth > 1)
        {
            let mut first_block = None;
            for axis in [Axis::X, Axis::Y] {
                if !self.perpendicular_gt_one(pose, axis) {
                    continue;
                }
                let candidate = self.placement(pose, Some(axis));
                if candidate.fits() {
                    return self.end_check(pose, &candidate, true, explain);
                }
                if first_block.is_none() {
                    first_block = candidate.block;
                }
            }
            if let Some((reason, blockers)) = first_block {
                return EndCheck {
                    allowed: false,
                    reason: Some(reason),
                    blockers,
                    squeezed: false,
                    explain,
                };
            }
        }
        EndCheck {
            allowed: false,
            reason: Some(reason),
            blockers,
            squeezed: false,
            explain,
        }
    }

    /// Evaluate one adjacent step. Parity 0 is the default (first diagonal);
    /// multi-step callers use `verify_path` or the planner search.
    pub fn step(&self, from: &SpatialPose, to: &SpatialPose) -> StepResult {
        self.edge_at(from, to, 0, MoveKind::Voluntary)
    }

    fn edge_at(
        &self,
        from: &SpatialPose,
        to: &SpatialPose,
        parity: u8,
        kind: MoveKind,
    ) -> StepResult {
        if from.validate().is_err() || to.validate().is_err() {
            return blocked_step(
                from,
                to,
                MoveBlock::InvalidPose,
                Vec::new(),
                "step-invalid-pose",
                "pose coordinates or footprint are invalid",
            );
        }
        if from.footprint != to.footprint || from.facing != to.facing {
            return blocked_step(
                from,
                to,
                MoveBlock::InvalidPose,
                Vec::new(),
                "step-pose-mismatch",
                "both poses must describe the same footprint and facing",
            );
        }
        let dx = to.cell.x.saturating_sub(from.cell.x);
        let dy = to.cell.y.saturating_sub(from.cell.y);
        if dx.abs() > 1 || dy.abs() > 1 || (dx == 0 && dy == 0) {
            return blocked_step(
                from,
                to,
                MoveBlock::NotAdjacent,
                Vec::new(),
                "step-not-adjacent",
                "movement edges connect adjacent cells only",
            );
        }
        let diagonal = dx != 0 && dy != 0;
        if diagonal && from.cell.z != to.cell.z {
            return blocked_step(
                from,
                to,
                MoveBlock::Height,
                Vec::new(),
                "step-diagonal-height",
                "diagonal edges cannot change the floor level; climb orthogonally",
            );
        }
        let axis = if diagonal {
            None
        } else if dx != 0 {
            Some(Axis::X)
        } else {
            Some(Axis::Y)
        };
        let refine = |placement: Placement| -> Placement {
            if diagonal
                && placement
                    .block
                    .as_ref()
                    .is_some_and(|(reason, _)| *reason == MoveBlock::Wall)
                && (from.footprint.width > 1 || from.footprint.depth > 1)
            {
                let (_, blockers) = placement.block.expect("wall block");
                return Placement {
                    cells: placement.cells,
                    squeezed: placement.squeezed,
                    block: Some((MoveBlock::TooNarrow, blockers)),
                };
            }
            placement
        };
        let source = refine(self.placement(from, axis));
        if let Some((reason, blockers)) = source.block {
            return blocked_step(
                from,
                to,
                reason,
                blockers,
                "step-source-footprint",
                "the mover's source footprint does not fit",
            );
        }
        let dest = refine(self.placement(to, axis));
        if let Some((reason, blockers)) = dest.block {
            return blocked_step(
                from,
                to,
                reason,
                blockers,
                "step-dest-footprint",
                "the destination footprint does not fit",
            );
        }
        let elevation_change = to.cell.z.saturating_sub(from.cell.z);
        if elevation_change > MAX_ELEVATION_STEP_FT {
            return blocked_step(
                from,
                to,
                MoveBlock::TooSteep,
                Vec::new(),
                "step-too-steep",
                format!("a {elevation_change} ft rise exceeds one climb step"),
            );
        }
        if elevation_change < -MAX_ELEVATION_STEP_FT {
            return blocked_step(
                from,
                to,
                MoveBlock::Drop,
                Vec::new(),
                "step-drop",
                format!("a {elevation_change} ft drop belongs to P4 falling"),
            );
        }

        let mut swept: BTreeMap<(i32, i32), BTreeSet<i32>> = BTreeMap::new();
        for &(x, y) in &source.cells {
            swept.entry((x, y)).or_default().insert(from.cell.z);
        }
        for &(x, y) in &dest.cells {
            swept.entry((x, y)).or_default().insert(to.cell.z);
        }
        if diagonal {
            let mids = [
                SpatialPose {
                    cell: CellPos {
                        x: from.cell.x.saturating_add(dx),
                        y: from.cell.y,
                        z: from.cell.z,
                    },
                    ..*from
                },
                SpatialPose {
                    cell: CellPos {
                        x: from.cell.x,
                        y: from.cell.y.saturating_add(dy),
                        z: from.cell.z,
                    },
                    ..*from
                },
            ];
            for mid in &mids {
                let placement = self.placement(mid, None);
                if let Some((reason, blockers)) = placement.block {
                    return blocked_step(
                        from,
                        to,
                        reason,
                        blockers,
                        "step-corner",
                        "a diagonal may not cross a blocked corner",
                    );
                }
                for (x, y) in self.full_cells(mid) {
                    swept.entry((x, y)).or_default().insert(from.cell.z);
                }
            }
        }

        let mut creature_space = false;
        let mut occupant_blockers = BTreeSet::new();
        for occupant in &self.occupants {
            if occupant.body.id == self.mover.id {
                continue;
            }
            let occupant_cells = occupant.body.cells();
            let swept_overlap = occupant_cells.iter().any(|occupant_cell| {
                swept
                    .get(&(occupant_cell.x, occupant_cell.y))
                    .is_some_and(|bands| {
                        bands.iter().any(|&base_z| {
                            vertical_overlap(
                                base_z,
                                self.mover.height_ft,
                                occupant_cell.z,
                                occupant.body.height_ft,
                            )
                        })
                    })
            });
            if !swept_overlap {
                continue;
            }
            if !self.may_pass_through(occupant) {
                occupant_blockers.insert(occupant.body.id.clone());
            }
            // Difficult Terrain applies when entering a creature's space, not
            // when leaving one. The swept volume above is the passage check.
            let enters = occupant_cells.iter().any(|occupant_cell| {
                dest.cells.iter().any(|&(x, y)| {
                    occupant_cell.x == x
                        && occupant_cell.y == y
                        && vertical_overlap(
                            to.cell.z,
                            self.mover.height_ft,
                            occupant_cell.z,
                            occupant.body.height_ft,
                        )
                })
            });
            if enters {
                creature_space = true;
            }
        }
        if !occupant_blockers.is_empty() {
            return blocked_step(
                from,
                to,
                MoveBlock::Occupied,
                occupant_blockers.into_iter().collect(),
                "step-occupied",
                "another creature's space is not passable for this mover",
            );
        }

        let mode = if elevation_change != 0 {
            MoveMode::Climb
        } else if self
            .map
            .cell(to.cell.x, to.cell.y)
            .is_some_and(|cell| cell.medium == MovementMedium::Water)
        {
            MoveMode::Swim
        } else {
            MoveMode::Walk
        };
        let dest_difficult = dest
            .cells
            .iter()
            .any(|&(x, y)| self.map.cell(x, y).is_some_and(|cell| cell.difficult));
        let squeezed = source.squeezed || dest.squeezed;
        let difficult = dest_difficult
            || creature_space
            || (squeezed && self.profile.squeeze == Squeeze::Edition2024);
        let climb_without_speed = mode == MoveMode::Climb && !self.capabilities.has_climb();
        let swim_without_speed = mode == MoveMode::Swim && !self.capabilities.has_swim();
        let terrain = TravelTerrain {
            difficult,
            narrow_opening: squeezed,
            climb_swim_crawl: climb_without_speed || swim_without_speed,
        };
        let (mut base, squares, ground_rule) = ground_distance(from, to, self.profile);
        if diagonal
            && parity == 1
            && self.profile.diagonal == Diagonal::FiveTenFive
            && self.profile.distance_3d == Distance3d::Chebyshev
        {
            base = base.saturating_add(FEET_PER_CELL);
        }
        let travel = travel_cost_from_base(base, squares, ground_rule, self.profile, terrain);
        if travel.blocked.is_some() {
            return blocked_step(
                from,
                to,
                MoveBlock::TooNarrow,
                Vec::new(),
                "step-too-narrow",
                "the profile forbids squeezing",
            );
        }
        let mut explain = travel.distance.explain.clone();
        if parity == 1
            && diagonal
            && self.profile.diagonal == Diagonal::FiveTenFive
            && self.profile.distance_3d == Distance3d::Chebyshev
        {
            explain.push(Explain::new(
                "travel-five-ten-five",
                "second consecutive diagonal costs 10 ft",
            ));
        }
        let edge = MovementEdge {
            from: *from,
            to: *to,
            cost_ft: travel.distance.feet,
            distance_ft: base,
            mode,
            diagonal,
            elevation_change_ft: elevation_change,
            difficult,
            narrow_opening: squeezed,
            creature_space,
            squeezed,
            triggers: self.triggers(from, to, kind, mode, elevation_change, squeezed),
        };
        StepResult {
            from: *from,
            to: *to,
            edge: Some(edge),
            blocked: None,
            blockers: Vec::new(),
            explain,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn triggers(
        &self,
        from: &SpatialPose,
        to: &SpatialPose,
        kind: MoveKind,
        mode: MoveMode,
        elevation_change: i32,
        squeezed: bool,
    ) -> Vec<MoveTrigger> {
        let mut triggers = vec![MoveTrigger::AreaEntry { at: to.cell }];
        if squeezed {
            triggers.push(MoveTrigger::Squeeze {
                at: to.cell,
                disadvantage: self.profile.squeeze == Squeeze::Edition2014,
            });
        }
        if mode == MoveMode::Climb {
            triggers.push(MoveTrigger::Climb {
                from: from.cell,
                to: to.cell,
                elevation_change_ft: elevation_change,
            });
        }
        if kind == MoveKind::Voluntary && !self.observers.is_empty() {
            let mover_from = self.body_at(from);
            let mover_to = self.body_at(to);
            for observer in &self.observers {
                let was =
                    can_reach_bodies(&observer.body, &mover_from, observer.reach_ft, self.profile)
                        .reachable;
                let now =
                    can_reach_bodies(&observer.body, &mover_to, observer.reach_ft, self.profile)
                        .reachable;
                if was && !now {
                    triggers.push(MoveTrigger::LeavesReach {
                        observer: observer.body.id.clone(),
                        from: from.cell,
                        to: to.cell,
                    });
                }
            }
        }
        triggers
    }

    fn search(
        &self,
        goal: Option<&SpatialPose>,
        budget_ft: Option<i32>,
        use_heuristic: bool,
    ) -> SearchResult {
        let start = self.mover.pose;
        let start_key = (start.cell, 0u8);
        let mut nodes = BTreeMap::new();
        nodes.insert(
            start_key,
            ReachNode {
                cost_ft: 0,
                parity: 0,
                parent: None,
                edge: None,
            },
        );
        let mut best: BTreeMap<(CellPos, u8), i32> = BTreeMap::new();
        best.insert(start_key, 0);
        let mut open = BinaryHeap::new();
        let start_h = if use_heuristic {
            goal.map_or(0, |goal| movement_heuristic(&start, goal, self.profile))
        } else {
            0
        };
        open.push(Reverse((
            start_h,
            0i32,
            start.cell.x,
            start.cell.y,
            start.cell.z,
            0u8,
        )));
        let mut expanded = 0usize;
        let mut goal_key = None;
        let mut goal_blocks: BTreeMap<MoveBlock, BTreeSet<String>> = BTreeMap::new();
        let mut frontier_blocks: BTreeMap<(u32, MoveBlock), BTreeSet<String>> = BTreeMap::new();

        while let Some(Reverse((_f, cost, x, y, z, parity))) = open.pop() {
            let key = (CellPos { x, y, z }, parity);
            if best.get(&key).copied() != Some(cost) {
                continue;
            }
            if goal.is_some_and(|goal| goal.cell == key.0) {
                goal_key = Some(key);
                break;
            }
            expanded = expanded.saturating_add(1);
            let from = self.pose_at(key.0);
            for (dx, dy) in MOVE_DIRECTIONS {
                let nx = x.saturating_add(dx);
                let ny = y.saturating_add(dy);
                let nz = self.map.cell(nx, ny).map_or(z, |cell| cell.elevation_ft);
                let to = self.pose_at(CellPos {
                    x: nx,
                    y: ny,
                    z: nz,
                });
                let result = self.edge_at(&from, &to, parity, MoveKind::Voluntary);
                if let Some(reason) = result.blocked {
                    if let Some(goal) = goal {
                        let distance = chebyshev(CellPos { x: nx, y: ny, z: 0 }, goal.cell);
                        frontier_blocks
                            .entry((distance, reason))
                            .or_default()
                            .extend(result.blockers.iter().cloned());
                        if nx == goal.cell.x && ny == goal.cell.y && nz == goal.cell.z {
                            goal_blocks
                                .entry(reason)
                                .or_default()
                                .extend(result.blockers);
                        }
                    }
                    continue;
                }
                let edge = result.edge.expect("an allowed step carries an edge");
                let new_parity = if edge.diagonal { 1 - parity } else { parity };
                let new_cost = cost.saturating_add(edge.cost_ft);
                if let Some(limit) = budget_ft {
                    if new_cost > limit {
                        continue;
                    }
                }
                let new_key = (to.cell, new_parity);
                if new_cost < best.get(&new_key).copied().unwrap_or(i32::MAX) {
                    best.insert(new_key, new_cost);
                    nodes.insert(
                        new_key,
                        ReachNode {
                            cost_ft: new_cost,
                            parity: new_parity,
                            parent: Some(key),
                            edge: Some(edge),
                        },
                    );
                    let heuristic = if use_heuristic {
                        goal.map_or(0, |goal| movement_heuristic(&to, goal, self.profile))
                    } else {
                        0
                    };
                    open.push(Reverse((
                        new_cost.saturating_add(heuristic),
                        new_cost,
                        nx,
                        ny,
                        nz,
                        new_parity,
                    )));
                }
            }
        }

        SearchResult {
            nodes,
            goal_key,
            expanded,
            goal_blocks,
            frontier_blocks,
        }
    }

    fn unreachable_reason(&self, search: &SearchResult) -> (MoveBlock, Vec<String>) {
        if let Some((reason, blockers)) = search.goal_blocks.iter().next() {
            return (*reason, blockers.iter().cloned().collect());
        }
        if let Some(((_, reason), blockers)) = search.frontier_blocks.iter().next() {
            return (*reason, blockers.iter().cloned().collect());
        }
        (MoveBlock::NoPath, Vec::new())
    }

    /// Shortest movement cost to every pose within `budget_ft`, with parent
    /// data for path reconstruction. This is the Dijkstra half of the seam.
    pub fn reachable(&self, budget_ft: i32) -> Result<ReachableSet, SpatialError> {
        if budget_ft < 0 {
            return Err(SpatialError::InvalidBudget { budget_ft });
        }
        let start = self.can_end_at(&self.mover.pose);
        if !start.allowed {
            return Ok(ReachableSet {
                start: self.mover.pose,
                budget_ft,
                entries: Vec::new(),
                start_blocked: Some(start.reason.unwrap_or(MoveBlock::NoPath)),
                blockers: start.blockers,
                explain: vec![
                    Explain::new("reachable-dijkstra", "start pose is blocked"),
                    Explain::new(
                        "reachable-start",
                        format!("start reason {:?}", start.reason),
                    ),
                ],
                nodes: BTreeMap::new(),
            });
        }
        let search = self.search(None, Some(budget_ft), false);
        let mut best_by_cell: BTreeMap<CellPos, (u8, i32)> = BTreeMap::new();
        for (&(cell, parity), node) in &search.nodes {
            let entry = best_by_cell.entry(cell).or_insert((parity, node.cost_ft));
            if node.cost_ft < entry.1 || (node.cost_ft == entry.1 && parity < entry.0) {
                *entry = (parity, node.cost_ft);
            }
        }
        let mut entries = Vec::with_capacity(best_by_cell.len());
        for (cell, (parity, cost_ft)) in best_by_cell {
            let pose = self.pose_at(cell);
            let end = self.can_end_at(&pose);
            entries.push(ReachEntry {
                pose,
                cost_ft,
                diagonals_parity: parity,
                end_allowed: end.allowed,
                squeezed: !self.full_fits(&pose),
            });
        }
        entries.sort_by_key(|entry| {
            (
                entry.cost_ft,
                entry.pose.cell.x,
                entry.pose.cell.y,
                entry.pose.cell.z,
                entry.diagonals_parity,
            )
        });
        let explain = vec![
            Explain::new("reachable-dijkstra", "budget-limited reachable set"),
            Explain::new("reachable-budget", format!("{budget_ft} ft speed budget")),
            Explain::new("reachable-count", format!("{} poses", entries.len())),
        ];
        Ok(ReachableSet {
            start: self.mover.pose,
            budget_ft,
            entries,
            start_blocked: None,
            blockers: Vec::new(),
            explain,
            nodes: search.nodes,
        })
    }

    /// Targeted path query. `Auto` uses A* when `astar_heuristic_is_admissible`
    /// proves the heuristic, otherwise it falls back to Dijkstra.
    pub fn path(&self, goal: &SpatialPose, budget_ft: i32) -> Result<PathPlan, SpatialError> {
        self.path_with_algorithm(goal, budget_ft, PathAlgorithm::Auto)
    }

    pub fn path_with_algorithm(
        &self,
        goal: &SpatialPose,
        budget_ft: i32,
        requested: PathAlgorithm,
    ) -> Result<PathPlan, SpatialError> {
        if budget_ft < 0 {
            return Err(SpatialError::InvalidBudget { budget_ft });
        }
        goal.validate()?;
        if goal.footprint != self.mover.pose.footprint || goal.facing != self.mover.pose.facing {
            return Err(SpatialError::PoseMismatch {
                expected: self.mover.pose.footprint,
                actual: goal.footprint,
            });
        }
        let (algorithm, use_heuristic) = match requested {
            PathAlgorithm::Dijkstra => (PathAlgorithm::Dijkstra, false),
            PathAlgorithm::Auto | PathAlgorithm::AStar => {
                if astar_heuristic_is_admissible(self.profile) {
                    (PathAlgorithm::AStar, true)
                } else {
                    (PathAlgorithm::Dijkstra, false)
                }
            }
        };
        let base_explain = vec![
            Explain::new("path-request", format!("budget {budget_ft} ft")),
            Explain::new("path-algorithm", format!("{algorithm:?}")),
        ];
        let start_check = self.can_end_at(&self.mover.pose);
        if !start_check.allowed {
            return Ok(PathPlan {
                start: self.mover.pose,
                goal: *goal,
                budget_ft,
                kind: MoveKind::Voluntary,
                algorithm,
                outcome: PlanOutcome::Unreachable {
                    reason: start_check.reason.unwrap_or(MoveBlock::NoPath),
                    blockers: start_check.blockers,
                },
                nodes_expanded: 0,
                explain: base_explain,
            });
        }
        if goal.cell == self.mover.pose.cell {
            return Ok(PathPlan {
                start: self.mover.pose,
                goal: *goal,
                budget_ft,
                kind: MoveKind::Voluntary,
                algorithm,
                outcome: PlanOutcome::Reached(MovementPath {
                    start: self.mover.pose,
                    goal: *goal,
                    steps: Vec::new(),
                    total_cost_ft: 0,
                    kind: MoveKind::Voluntary,
                    explain: vec![Explain::new("path-empty", "already at the goal")],
                }),
                nodes_expanded: 0,
                explain: base_explain,
            });
        }
        let goal_check = self.can_end_at(goal);
        if !goal_check.allowed {
            return Ok(PathPlan {
                start: self.mover.pose,
                goal: *goal,
                budget_ft,
                kind: MoveKind::Voluntary,
                algorithm,
                outcome: PlanOutcome::Unreachable {
                    reason: goal_check.reason.unwrap_or(MoveBlock::NoPath),
                    blockers: goal_check.blockers,
                },
                nodes_expanded: 0,
                explain: base_explain,
            });
        }
        let search = self.search(Some(goal), None, use_heuristic);
        let Some(goal_key) = search.goal_key else {
            let (reason, blockers) = self.unreachable_reason(&search);
            let mut explain = base_explain;
            explain.push(Explain::new("path-unreachable", reason.as_str()));
            return Ok(PathPlan {
                start: self.mover.pose,
                goal: *goal,
                budget_ft,
                kind: MoveKind::Voluntary,
                algorithm,
                outcome: PlanOutcome::Unreachable { reason, blockers },
                nodes_expanded: search.expanded,
                explain,
            });
        };
        let path = reconstruct_path(
            &search.nodes,
            goal_key,
            self.mover.pose,
            *goal,
            MoveKind::Voluntary,
        );
        let outcome = if path.total_cost_ft <= budget_ft {
            PlanOutcome::Reached(path)
        } else {
            PlanOutcome::OverBudget(path)
        };
        let mut explain = base_explain;
        explain.push(Explain::new(
            "path-expanded",
            format!("{} nodes", search.expanded),
        ));
        Ok(PathPlan {
            start: self.mover.pose,
            goal: *goal,
            budget_ft,
            kind: MoveKind::Voluntary,
            algorithm,
            outcome,
            nodes_expanded: search.expanded,
            explain,
        })
    }

    /// Re-check a planned path against the planner's current map, occupants and
    /// capabilities. This is the per-step gate P5 must run before execution.
    pub fn verify_path(&self, path: &MovementPath, budget_ft: i32) -> PathCheck {
        let mut explain = vec![Explain::new(
            "verify-path",
            format!("{} planned steps", path.steps.len()),
        )];
        if path.start != self.mover.pose {
            explain.push(Explain::new(
                "verify-start",
                "the path does not start at the mover's current pose",
            ));
            return PathCheck {
                verdict: PathVerdict::Blocked {
                    step_index: 0,
                    reason: MoveBlock::NotAdjacent,
                    blockers: Vec::new(),
                },
                checked_steps: 0,
                explain,
            };
        }
        let mut previous = path.start;
        let mut parity = 0u8;
        let mut total = 0i32;
        for (index, step) in path.steps.iter().enumerate() {
            if step.from != previous {
                explain.push(Explain::new(
                    "verify-chain",
                    "the path steps do not chain from the start pose",
                ));
                return PathCheck {
                    verdict: PathVerdict::Blocked {
                        step_index: index,
                        reason: MoveBlock::NotAdjacent,
                        blockers: Vec::new(),
                    },
                    checked_steps: index + 1,
                    explain,
                };
            }
            let result = self.edge_at(&step.from, &step.to, parity, MoveKind::Voluntary);
            if let Some(reason) = result.blocked {
                explain.push(Explain::new("verify-blocked", reason.as_str()));
                return PathCheck {
                    verdict: PathVerdict::Blocked {
                        step_index: index,
                        reason,
                        blockers: result.blockers,
                    },
                    checked_steps: index + 1,
                    explain,
                };
            }
            let edge = result.edge.expect("an allowed verification step");
            if edge.cost_ft != step.cost_ft {
                explain.push(Explain::new(
                    "verify-cost",
                    format!(
                        "step {index} cost changed from {} to {}",
                        step.cost_ft, edge.cost_ft
                    ),
                ));
                return PathCheck {
                    verdict: PathVerdict::CostChanged {
                        step_index: index,
                        planned_cost_ft: step.cost_ft,
                        actual_cost_ft: edge.cost_ft,
                    },
                    checked_steps: index + 1,
                    explain,
                };
            }
            parity = if edge.diagonal { 1 - parity } else { parity };
            total = total.saturating_add(edge.cost_ft);
            previous = step.to;
        }
        if previous != path.goal {
            explain.push(Explain::new(
                "verify-goal-chain",
                "the last step does not end at the path goal",
            ));
            return PathCheck {
                verdict: PathVerdict::Blocked {
                    step_index: path.steps.len().saturating_sub(1),
                    reason: MoveBlock::NotAdjacent,
                    blockers: Vec::new(),
                },
                checked_steps: path.steps.len(),
                explain,
            };
        }
        let end = self.can_end_at(&path.goal);
        if !end.allowed {
            explain.push(Explain::new(
                "verify-end",
                "the goal is no longer a legal end position",
            ));
            return PathCheck {
                verdict: PathVerdict::Blocked {
                    step_index: path.steps.len().saturating_sub(1),
                    reason: end.reason.unwrap_or(MoveBlock::EndOccupied),
                    blockers: end.blockers,
                },
                checked_steps: path.steps.len(),
                explain,
            };
        }
        if total > budget_ft {
            explain.push(Explain::new(
                "verify-budget",
                format!("{total} ft exceeds the {budget_ft} ft budget"),
            ));
            return PathCheck {
                verdict: PathVerdict::OverBudget {
                    total_cost_ft: total,
                },
                checked_steps: path.steps.len(),
                explain,
            };
        }
        explain.push(Explain::new("verify-ok", "every step still matches"));
        PathCheck {
            verdict: PathVerdict::Valid,
            checked_steps: path.steps.len(),
            explain,
        }
    }

    /// Straight-line forced movement. It never spends Speed and never emits a
    /// `LeavesReach` trigger; area-entry triggers stay for the engine.
    pub fn forced_move(
        &self,
        direction: (i32, i32),
        distance_ft: i32,
    ) -> Result<ForcedMovePlan, SpatialError> {
        let (dx, dy) = direction;
        if dx.unsigned_abs() > 1 || dy.unsigned_abs() > 1 || (dx == 0 && dy == 0) {
            return Err(SpatialError::InvalidDirection { dx, dy });
        }
        if distance_ft < 0 {
            return Err(SpatialError::InvalidDistance { distance_ft });
        }
        let start = self.mover.pose;
        let mut explain = vec![
            Explain::new(
                "forced-movement",
                "forced movement: no Speed, no opportunity attack",
            ),
            Explain::new("forced-distance", format!("{distance_ft} ft")),
        ];
        let start_check = self.can_end_at(&start);
        if !start_check.allowed {
            return Ok(ForcedMovePlan {
                start,
                direction,
                distance_ft,
                steps: Vec::new(),
                moved_ft: 0,
                stopped_by: start_check.reason,
                rules: MoveKind::Forced.rules(),
                explain,
            });
        }
        let mut steps = Vec::new();
        let mut moved = 0;
        let mut current = start;
        let mut stopped_by = None;
        let mut legal_steps = 0usize;
        let mut legal_moved = 0i32;
        while moved < distance_ft {
            let nx = current.cell.x.saturating_add(dx);
            let ny = current.cell.y.saturating_add(dy);
            let nz = self
                .map
                .cell(nx, ny)
                .map_or(current.cell.z, |cell| cell.elevation_ft);
            let next = self.pose_at(CellPos {
                x: nx,
                y: ny,
                z: nz,
            });
            let result = self.edge_at(&current, &next, 0, MoveKind::Forced);
            let Some(edge) = result.edge else {
                stopped_by = result.blocked;
                explain.push(Explain::new(
                    "forced-stop",
                    format!(
                        "blocked: {}",
                        result.blocked.map_or("unknown", MoveBlock::as_str)
                    ),
                ));
                break;
            };
            let remaining = distance_ft.saturating_sub(moved);
            if edge.distance_ft > remaining {
                explain.push(Explain::new(
                    "forced-partial",
                    "remaining distance is shorter than one grid step",
                ));
                break;
            }
            let end = self.can_end_at(&next);
            if !end.allowed {
                let after_cell = CellPos {
                    x: nx.saturating_add(dx),
                    y: ny.saturating_add(dy),
                    z: self
                        .map
                        .cell(nx.saturating_add(dx), ny.saturating_add(dy))
                        .map_or(nz, |cell| cell.elevation_ft),
                };
                let after = self.pose_at(after_cell);
                let transit_possible = remaining > edge.distance_ft
                    && self
                        .edge_at(&next, &after, 0, MoveKind::Forced)
                        .edge
                        .is_some();
                if !transit_possible {
                    stopped_by = end.reason.or(Some(MoveBlock::EndOccupied));
                    explain.push(Explain::new(
                        "forced-end",
                        "the pushed body cannot end in the next space",
                    ));
                    break;
                }
            } else {
                legal_steps = steps.len() + 1;
                legal_moved = moved.saturating_add(edge.distance_ft);
            }
            moved = moved.saturating_add(edge.distance_ft);
            steps.push(edge);
            current = next;
        }
        if !self.can_end_at(&current).allowed {
            steps.truncate(legal_steps);
            moved = legal_moved;
            stopped_by = stopped_by.or(Some(MoveBlock::EndOccupied));
            explain.push(Explain::new(
                "forced-truncate",
                "forced movement stops at the last legal end before an occupied space",
            ));
        }
        Ok(ForcedMovePlan {
            start,
            direction,
            distance_ft,
            steps,
            moved_ft: moved,
            stopped_by,
            rules: MoveKind::Forced.rules(),
            explain,
        })
    }
}

fn blocked_step(
    from: &SpatialPose,
    to: &SpatialPose,
    reason: MoveBlock,
    blockers: Vec<String>,
    code: &'static str,
    detail: impl Into<String>,
) -> StepResult {
    StepResult {
        from: *from,
        to: *to,
        edge: None,
        blocked: Some(reason),
        blockers,
        explain: vec![Explain::new(code, detail)],
    }
}

fn reconstruct_path(
    nodes: &BTreeMap<(CellPos, u8), ReachNode>,
    mut key: (CellPos, u8),
    start: SpatialPose,
    goal: SpatialPose,
    kind: MoveKind,
) -> MovementPath {
    let mut steps = Vec::new();
    while let Some(node) = nodes.get(&key) {
        let Some(edge) = node.edge.clone() else {
            break;
        };
        steps.push(edge);
        let Some(parent) = node.parent else {
            break;
        };
        key = parent;
    }
    steps.reverse();
    let total_cost_ft = steps.iter().map(|step| step.cost_ft).sum();
    MovementPath {
        start,
        goal,
        explain: vec![Explain::new(
            "path-reconstruct",
            format!("{} steps from the parent map", steps.len()),
        )],
        steps,
        total_cost_ft,
        kind,
    }
}

fn chebyshev(a: CellPos, b: CellPos) -> u32 {
    a.x.saturating_sub(b.x)
        .unsigned_abs()
        .max(a.y.saturating_sub(b.y).unsigned_abs())
}
