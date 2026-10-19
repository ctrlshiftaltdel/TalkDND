//! Spatial rules seam.
//!
//! Step 2 implements zone graphs. A grid implementation can be added later
//! without changing the engine: everything spatial goes through [`Space`].

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::content::schema::{Cover, Rules, Scene};

/// Distance from any point in a zone to any other point in the same zone.
pub const SAME_ZONE_DISTANCE: i32 = 0;

/// Everything the engine needs to know about where creatures are.
///
/// Zone ids are used in step 2; a grid implementation can ignore them and work
/// with positions internally.
pub trait Space {
    /// Shortest travel distance in feet, ignoring elevation.
    fn travel_distance(&self, from: &str, to: &str) -> Option<i32>;

    /// Distance used for reach and range: travel distance plus elevation
    /// difference between two different zones.
    fn distance(&self, from: &str, to: &str) -> Option<i32>;

    /// Whether `to` is within `reach` feet for a melee attack.
    fn can_reach(&self, from: &str, to: &str, reach: i32) -> bool;

    /// Cover bonus to AC/Dex saves against attacks crossing zones.
    ///
    /// `None` means total cover: the target cannot be targeted.
    fn cover_bonus(&self, attacker: &str, target: &str, rules: &Rules) -> Option<i32>;

    /// Zones adjacent to `zone` with travel distance.
    fn neighbours(&self, zone: &str) -> Vec<(String, i32)>;

    /// First declared zone, used as a fallback spawn.
    fn first_zone(&self) -> Option<&str>;

    fn has_zone(&self, zone: &str) -> bool;
}

/// Zone-graph implementation of [`Space`].
#[derive(Debug, Clone, Copy)]
pub struct ZoneSpace<'a> {
    scene: &'a Scene,
}

impl<'a> ZoneSpace<'a> {
    pub fn new(scene: &'a Scene) -> Self {
        Self { scene }
    }

    fn zone(&self, id: &str) -> Option<&crate::content::schema::Zone> {
        self.scene.zones.iter().find(|zone| zone.id == id)
    }

    fn elevation(&self, id: &str) -> i32 {
        self.zone(id).and_then(|zone| zone.elevation).unwrap_or(0)
    }
}

impl Space for ZoneSpace<'_> {
    fn travel_distance(&self, from: &str, to: &str) -> Option<i32> {
        if self.zone(from).is_none() || self.zone(to).is_none() {
            return None;
        }
        if from == to {
            return Some(SAME_ZONE_DISTANCE);
        }

        let mut best: HashMap<String, i32> = HashMap::new();
        let mut queue = BinaryHeap::new();
        best.insert(from.to_string(), 0);
        queue.push(Reverse((0i32, from.to_string())));

        while let Some(Reverse((cost, current))) = queue.pop() {
            if current == to {
                return Some(cost);
            }
            if cost > best.get(&current).copied().unwrap_or(i32::MAX) {
                continue;
            }
            for (next, edge_cost) in self.neighbours(&current) {
                let candidate = cost.saturating_add(edge_cost);
                let known = best.get(&next).copied().unwrap_or(i32::MAX);
                if candidate < known {
                    best.insert(next.clone(), candidate);
                    queue.push(Reverse((candidate, next)));
                }
            }
        }
        None
    }

    fn distance(&self, from: &str, to: &str) -> Option<i32> {
        let distance = self.travel_distance(from, to)?;
        if from == to {
            Some(distance)
        } else {
            Some(distance.saturating_add((self.elevation(from) - self.elevation(to)).abs()))
        }
    }

    fn can_reach(&self, from: &str, to: &str, reach: i32) -> bool {
        self.distance(from, to)
            .is_some_and(|distance| distance <= reach.max(5))
    }

    fn cover_bonus(&self, attacker: &str, target: &str, rules: &Rules) -> Option<i32> {
        if attacker == target {
            return Some(0);
        }
        let cover = self.zone(target)?.cover;
        match cover {
            Cover::None => Some(0),
            Cover::Half => Some(
                rules
                    .cover
                    .as_ref()
                    .and_then(|cover| cover.half)
                    .unwrap_or(2),
            ),
            Cover::ThreeQuarters => Some(
                rules
                    .cover
                    .as_ref()
                    .and_then(|cover| cover.three_quarters)
                    .unwrap_or(5),
            ),
            Cover::Total => None,
        }
    }

    fn neighbours(&self, zone: &str) -> Vec<(String, i32)> {
        let mut neighbours = Vec::new();
        for edge in &self.scene.edges {
            let cost = if edge.difficult {
                edge.distance.saturating_mul(2)
            } else {
                edge.distance
            };
            if edge.a == zone {
                neighbours.push((edge.b.clone(), cost));
            } else if edge.b == zone && !edge.one_way {
                neighbours.push((edge.a.clone(), cost));
            }
        }
        neighbours
    }

    fn first_zone(&self) -> Option<&str> {
        self.scene.zones.first().map(|zone| zone.id.as_str())
    }

    fn has_zone(&self, zone: &str) -> bool {
        self.zone(zone).is_some()
    }
}
