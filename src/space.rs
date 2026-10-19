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
        let mut neighbours = vec![Vec::new(); count];
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
            let to = if edge.a == scene.zones[a].id { b } else { a };
            neighbours[a].push(zone_step(scene, &heights, a, to, edge));
        }

        let mut range = vec![vec![None; count]; count];
        let mut travel = vec![vec![None; count]; count];
        let mut routes = vec![vec![None; count]; count];
        let mut route_edges = vec![vec![None; count]; count];
        for source in 0..count {
            let (range_best, _) = zone_dijkstra(source, &adjacency, scene, false);
            let (travel_best, travel_parents) =
                zone_dijkstra(source, &adjacency, scene, true);
            for target in 0..count {
                if source == target {
                    range[source][target] = Some(SAME_ZONE_DISTANCE);
                    travel[source][target] = Some(SAME_ZONE_DISTANCE);
                    routes[source][target] = Some(Vec::new());
                    continue;
                }
                let distance = range_best[target];
                if distance != i32::MAX {
                    range[source][target] = Some(distance.saturating_add(
                        (heights[source] - heights[target]).abs(),
                    ));
                }
                let cost = travel_best[target];
                if cost != i32::MAX {
                    travel[source][target] = Some(cost);
                    route_edges[source][target] =
                        zone_path(source, target, &travel_parents);
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
                        && !area
                            .iter()
                            .any(|known| known.zone_id() == Some(scene.zones[*neighbour].id.as_str()))
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
        position.zone_id().and_then(|zone| self.index.get(zone)).copied()
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
            let candidate = cost.saturating_add(zone_edge_cost(&scene.edges[*edge_index], movement));
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
        let (Some(attacker), Some(target)) = (cache.position_index(attacker), cache.position_index(target)) else {
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
    pub fn route_cost(
        &self,
        from: &Position,
        to: &Position,
        blocked: &[Position],
    ) -> Option<i32> {
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
