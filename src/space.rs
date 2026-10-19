//! Spatial rules seam: zone graphs and 5-ft grids behind one [`Space`] trait.
//!
//! Every engine rule that depends on where creatures are goes through this
//! module: range/reach, cover, light, movement routes, opportunity attacks and
//! area effects. Zone scenes use [`ZoneGraph`], grid scenes [`GridSpace`].

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};

use serde::{Deserialize, Serialize};

use crate::content::schema::{ClimbSpec, Cover, Light, Scene, SceneMode};

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

/// Zone-graph implementation of [`Space`].
#[derive(Debug, Clone, Copy)]
pub struct ZoneGraph<'a> {
    scene: &'a Scene,
}

impl<'a> ZoneGraph<'a> {
    pub fn new(scene: &'a Scene) -> Self {
        Self { scene }
    }

    fn zone(&self, id: &str) -> Option<&crate::content::schema::Zone> {
        self.scene.zones.iter().find(|zone| zone.id == id)
    }

    fn height(&self, id: &str) -> i32 {
        self.zone(id).and_then(|zone| zone.elevation).unwrap_or(0)
    }

    fn edge_between(&self, a: &str, b: &str) -> Option<&crate::content::schema::Edge> {
        self.scene.edges.iter().find(|edge| {
            (edge.a == a && edge.b == b) || (!edge.one_way && edge.a == b && edge.b == a)
        })
    }

    fn movement_cost(edge: &crate::content::schema::Edge) -> i32 {
        if edge.difficult {
            edge.distance.saturating_mul(2)
        } else {
            edge.distance
        }
    }

    fn directed_neighbours(&self, zone: &str) -> Vec<(String, &crate::content::schema::Edge)> {
        let mut neighbours = Vec::new();
        for edge in &self.scene.edges {
            if edge.a == zone {
                neighbours.push((edge.b.clone(), edge));
            } else if edge.b == zone && !edge.one_way {
                neighbours.push((edge.a.clone(), edge));
            }
        }
        neighbours
    }

    fn shortest_path(
        &self,
        from: &str,
        to: &str,
        movement: bool,
    ) -> Option<(i32, Vec<&crate::content::schema::Edge>)> {
        if self.zone(from).is_none() || self.zone(to).is_none() {
            return None;
        }
        if from == to {
            return Some((SAME_ZONE_DISTANCE, Vec::new()));
        }
        let mut best: HashMap<String, i32> = HashMap::new();
        let mut parents: HashMap<String, (String, &crate::content::schema::Edge)> = HashMap::new();
        let mut queue = BinaryHeap::new();
        best.insert(from.to_string(), 0);
        queue.push(Reverse((0i32, from.to_string())));

        while let Some(Reverse((cost, current))) = queue.pop() {
            if current == to {
                let mut edges = Vec::new();
                let mut node = to.to_string();
                while let Some((previous, edge)) = parents.get(&node) {
                    edges.push(*edge);
                    node = previous.clone();
                }
                edges.reverse();
                return Some((cost, edges));
            }
            if cost > best.get(&current).copied().unwrap_or(i32::MAX) {
                continue;
            }
            for (next, edge) in self.directed_neighbours(&current) {
                let weight = if movement {
                    Self::movement_cost(edge)
                } else {
                    edge.distance
                };
                let candidate = cost.saturating_add(weight);
                let known = best.get(&next).copied().unwrap_or(i32::MAX);
                if candidate < known {
                    best.insert(next.clone(), candidate);
                    parents.insert(next.clone(), (current.clone(), edge));
                    queue.push(Reverse((candidate, next)));
                }
            }
        }
        None
    }

    fn step(&self, from: &str, edge: &crate::content::schema::Edge) -> MoveStep {
        let to = if edge.a == from { &edge.b } else { &edge.a };
        MoveStep {
            position: Position::Zone { zone: to.clone() },
            cost: Self::movement_cost(edge),
            edge_distance: edge.distance,
            climb: edge.climb.clone(),
            locked: edge.door.as_ref().is_some_and(|door| door.locked),
            elevation_change: self.height(to) - self.height(from),
        }
    }
}

impl Space for ZoneGraph<'_> {
    fn mode(&self) -> SceneMode {
        SceneMode::Zones
    }

    fn distance(&self, from: &Position, to: &Position) -> Option<i32> {
        let (Some(from), Some(to)) = (from.zone_id(), to.zone_id()) else {
            return None;
        };
        let (travel, _) = self.shortest_path(from, to, false)?;
        if from == to {
            Some(travel)
        } else {
            Some(travel.saturating_add((self.height(from) - self.height(to)).abs()))
        }
    }

    fn travel_distance(&self, from: &Position, to: &Position) -> Option<i32> {
        let (Some(from), Some(to)) = (from.zone_id(), to.zone_id()) else {
            return None;
        };
        self.shortest_path(from, to, true).map(|(cost, _)| cost)
    }

    fn can_reach(&self, from: &Position, to: &Position, reach: i32) -> bool {
        let (Some(from), Some(to)) = (from.zone_id(), to.zone_id()) else {
            return false;
        };
        let reach = reach.max(5);
        if from == to {
            return self.height(from) == self.height(to);
        }
        let Some(edge) = self.edge_between(from, to) else {
            return false;
        };
        let distance = edge
            .distance
            .saturating_add((self.height(from) - self.height(to)).abs());
        distance <= reach
    }

    fn cover(&self, attacker: &Position, target: &Position) -> Cover {
        let (Some(attacker), Some(target)) = (attacker.zone_id(), target.zone_id()) else {
            return Cover::None;
        };
        if attacker == target {
            return Cover::None;
        }
        self.zone(target)
            .map(|zone| zone.cover)
            .unwrap_or(Cover::None)
    }

    fn light_at(&self, at: &Position) -> Light {
        let zone = at
            .zone_id()
            .and_then(|id| self.zone(id))
            .and_then(|zone| zone.light);
        zone.or(self.scene.light).unwrap_or(Light::Bright)
    }

    fn move_route(
        &self,
        from: &Position,
        to: &Position,
        _blocked: &[Position],
    ) -> Option<Vec<MoveStep>> {
        let (Some(from), Some(to)) = (from.zone_id(), to.zone_id()) else {
            return None;
        };
        let (_, edges) = self.shortest_path(from, to, true)?;
        let mut current = from.to_string();
        let mut route = Vec::new();
        for edge in edges {
            let step = self.step(&current, edge);
            if let Position::Zone { zone } = &step.position {
                current = zone.clone();
            }
            route.push(step);
        }
        Some(route)
    }

    fn neighbours(&self, at: &Position) -> Vec<MoveStep> {
        let Some(zone) = at.zone_id() else {
            return Vec::new();
        };
        self.directed_neighbours(zone)
            .into_iter()
            .map(|(_, edge)| self.step(zone, edge))
            .collect()
    }

    fn area_positions(&self, center: &Position, radius: i32) -> Vec<Position> {
        let Some(zone) = center.zone_id() else {
            return Vec::new();
        };
        let mut zones = vec![zone.to_string()];
        if radius > 0 {
            for (neighbour, edge) in self.directed_neighbours(zone) {
                if edge.distance <= radius && !zones.contains(&neighbour) {
                    zones.push(neighbour);
                }
            }
        }
        zones
            .into_iter()
            .map(|zone| Position::Zone { zone })
            .collect()
    }

    fn zone_positions(&self, zone: &str) -> Vec<Position> {
        if self.has_zone(zone) {
            vec![Position::Zone {
                zone: zone.to_string(),
            }]
        } else {
            Vec::new()
        }
    }

    fn zone_of(&self, at: &Position) -> Option<String> {
        at.zone_id().map(ToString::to_string)
    }

    fn first_zone(&self) -> Option<String> {
        self.scene.zones.first().map(|zone| zone.id.clone())
    }

    fn has_zone(&self, zone: &str) -> bool {
        self.zone(zone).is_some()
    }

    fn contains(&self, at: &Position) -> bool {
        at.zone_id().is_some_and(|zone| self.zone(zone).is_some())
    }

    fn elevation(&self, at: &Position) -> i32 {
        at.zone_id().map(|id| self.height(id)).unwrap_or(0)
    }

    fn opportunity_threat(&self, observer: &Position, departure: &Position) -> bool {
        match (observer.zone_id(), departure.zone_id()) {
            (Some(observer), Some(departure)) => observer == departure,
            _ => false,
        }
    }

    fn adjacent(&self, a: &Position, b: &Position) -> bool {
        let (Some(a), Some(b)) = (a.zone_id(), b.zone_id()) else {
            return false;
        };
        if a == b {
            return true;
        }
        self.edge_between(a, b)
            .is_some_and(|edge| edge.distance <= 5)
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
        self.index(x, y)
            .and_then(|index| self.zone_tags[index].clone())
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
    map: GridMap,
}

impl<'a> GridSpace<'a> {
    pub fn new(scene: &'a Scene) -> Option<Self> {
        GridMap::parse(scene).map(|map| Self { scene, map })
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
        Some(steps.saturating_mul(self.map.cell))
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
