//! Where the machines stand on the desk, and per-machine options. Stored as `layout.json`:
//!
//! ```json
//! {
//!   "tiles": [ { "machine": "MacBook", "x": 0, "y": 0 }, { "machine": "Gaming-PC", "x": 1, "y": 0.5 } ],
//!   "swap_cmd_ctrl": ["Gaming-PC"],
//!   "corner": 16
//! }
//! ```
//! Every machine is a tile of 1 × 1. Tiles that touch are neighbors where their edges overlap, so a
//! tile may sit half beside another and one side may have several neighbors. Machines used in turn
//! at the same place share a position. Machines are named by their name or the start of their
//! fingerprint.
//!
//! Older versions stored `neighbors` ("Gaming-PC is on the right of MacBook", over the whole edge);
//! a file or a peer with only those is read as tiles on a grid.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::geometry::Side;

/// One entry of the old arrangement; the same type travels between computers.
pub use tc_protocol::Placement as Neighbor;
/// One machine on the desk; the same type travels between computers.
pub use tc_protocol::Tile;

/// Tile positions closer than this count as the same.
const EPS: f64 = 1e-3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutFile {
    /// Where the machines stand. Empty in files of older versions: then `neighbors` counts.
    #[serde(default)]
    pub tiles: Vec<Tile>,
    /// The arrangement for older versions, derived from `tiles`.
    #[serde(default)]
    pub neighbors: Vec<Neighbor>,
    /// Machines that get ⌘ and Ctrl swapped when controlled from here.
    #[serde(default)]
    pub swap_cmd_ctrl: Vec<String>,
    /// No switching within this many logical points of a screen corner.
    #[serde(default = "default_corner")]
    pub corner: f64,
    /// Version of the arrangement, shared between computers: the highest `(revision, editor)` wins.
    #[serde(default)]
    pub revision: u64,
    /// Machine id of the computer that made this version.
    #[serde(default)]
    pub editor: String,
    /// Display names of the machines in the arrangement, by machine id.
    #[serde(default)]
    pub names: BTreeMap<String, String>,
}

fn default_corner() -> f64 {
    16.0
}

impl Default for LayoutFile {
    fn default() -> Self {
        Self {
            tiles: Vec::new(),
            neighbors: Vec::new(),
            swap_cmd_ctrl: Vec::new(),
            corner: default_corner(),
            revision: 0,
            editor: String::new(),
            names: BTreeMap::new(),
        }
    }
}

impl LayoutFile {
    /// The tiles, or for a file of an older version the tiles of its neighbors.
    pub fn effective_tiles(&self) -> Vec<Tile> {
        if self.tiles.is_empty() {
            tiles_from_neighbors(&self.neighbors)
        } else {
            self.tiles.clone()
        }
    }

    /// Sets the arrangement; `neighbors` follow for older versions.
    pub fn set_tiles(&mut self, tiles: Vec<Tile>) {
        self.neighbors = neighbors_from_tiles(&tiles);
        self.tiles = tiles;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    SelfNeighbor(String),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::SelfNeighbor(m) => write!(f, "{m} cannot be its own neighbor"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Grid cells of an old arrangement, found by following the neighbors outward from the first
/// machine of every group; later groups go to the right of the earlier ones.
pub fn tiles_from_neighbors(neighbors: &[Neighbor]) -> Vec<Tile> {
    let step = |side: Side| match side {
        Side::Left => (-1, 0),
        Side::Right => (1, 0),
        Side::Top => (0, -1),
        Side::Bottom => (0, 1),
    };
    let mut placed: Vec<(String, i32, i32)> = Vec::new();
    let mut group_x = 0;
    for start in neighbors.iter().flat_map(|n| [&n.of, &n.machine]) {
        if placed.iter().any(|(m, ..)| m == start) {
            continue;
        }
        let first = placed.len();
        placed.push((start.clone(), group_x, 0));
        let mut next = first;
        while next < placed.len() {
            let (current, x, y) = placed[next].clone();
            next += 1;
            for n in neighbors {
                let (other, (dx, dy)) = if n.of == current {
                    (&n.machine, step(n.side))
                } else if n.machine == current {
                    (&n.of, step(n.side.opposite()))
                } else {
                    continue;
                };
                if !placed.iter().any(|(m, ..)| m == other) {
                    placed.push((other.clone(), x + dx, y + dy));
                }
            }
        }
        let group = &placed[first..];
        let min_x = group.iter().map(|(_, x, _)| *x).min().unwrap_or(0);
        let max_x = group.iter().map(|(_, x, _)| *x).max().unwrap_or(0);
        group_x += max_x - min_x + 2;
    }
    placed
        .into_iter()
        .map(|(machine, x, y)| Tile {
            machine,
            x: x as f32,
            y: y as f32,
        })
        .collect()
}

/// The neighbors of tiles for older versions: tiles that touch with some overlap, over the whole
/// edge (older versions know nothing finer).
pub fn neighbors_from_tiles(tiles: &[Tile]) -> Vec<Neighbor> {
    let mut neighbors = Vec::new();
    for (i, a) in tiles.iter().enumerate() {
        for b in &tiles[i + 1..] {
            if a.machine == b.machine {
                continue;
            }
            let (a_pos, b_pos) = (pos(a), pos(b));
            if let Some(side) = touching_side(a_pos, b_pos) {
                neighbors.push(Neighbor {
                    machine: b.machine.clone(),
                    side,
                    of: a.machine.clone(),
                });
            }
        }
    }
    neighbors
}

fn pos(t: &Tile) -> (f64, f64) {
    (f64::from(t.x), f64::from(t.y))
}

/// On which side of tile `a` tile `b` touches it with some overlap.
pub fn tile_side(a: &Tile, b: &Tile) -> Option<Side> {
    touching_side(pos(a), pos(b))
}

fn touching_side(a: (f64, f64), b: (f64, f64)) -> Option<Side> {
    let overlap = |p: f64, q: f64| (p + 1.0).min(q + 1.0) - p.max(q) > EPS;
    if (b.0 - (a.0 + 1.0)).abs() < EPS && overlap(a.1, b.1) {
        Some(Side::Right)
    } else if (a.0 - (b.0 + 1.0)).abs() < EPS && overlap(a.1, b.1) {
        Some(Side::Left)
    } else if (b.1 - (a.1 + 1.0)).abs() < EPS && overlap(a.0, b.0) {
        Some(Side::Bottom)
    } else if (a.1 - (b.1 + 1.0)).abs() < EPS && overlap(a.0, b.0) {
        Some(Side::Top)
    } else {
        None
    }
}

/// The resolved arrangement, by machine id.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// In the order they were placed: of machines sharing a place, the first one online counts.
    tiles: Vec<(String, f64, f64)>,
    pub swap_cmd_ctrl: HashSet<String>,
    pub corner: f64,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            tiles: Vec::new(),
            swap_cmd_ctrl: HashSet::new(),
            corner: default_corner(),
        }
    }
}

impl Layout {
    /// `resolve` turns a name from the file into a machine id (`None`: unknown, entry ignored).
    pub fn from_file(file: &LayoutFile, resolve: impl Fn(&str) -> Option<String>) -> Result<Self, LayoutError> {
        if let Some(n) = file
            .neighbors
            .iter()
            .find(|n| resolve(&n.machine).is_some() && resolve(&n.machine) == resolve(&n.of))
        {
            return Err(LayoutError::SelfNeighbor(n.machine.clone()));
        }
        let mut layout = Layout {
            corner: if file.corner.is_finite() && file.corner >= 0.0 {
                file.corner
            } else {
                default_corner()
            },
            ..Layout::default()
        };
        for tile in file.effective_tiles() {
            let (x, y) = pos(&tile);
            if let Some(machine) = resolve(&tile.machine)
                && x.is_finite()
                && y.is_finite()
                && !layout.tiles.iter().any(|(m, ..)| *m == machine)
            {
                layout.tiles.push((machine, x, y));
            }
        }
        layout.swap_cmd_ctrl = file.swap_cmd_ctrl.iter().filter_map(|n| resolve(n)).collect();
        Ok(layout)
    }

    /// Builds a layout directly (tests): `machine` on `side` of `of` over the whole edge; `of` is
    /// put at `(0, 0)` if it has no place yet.
    pub fn with(mut self, machine: &str, side: Side, of: &str) -> Self {
        let (x, y) = match self.place_of(of) {
            Some(p) => p,
            None => {
                self.tiles.push((of.to_string(), 0.0, 0.0));
                (0.0, 0.0)
            }
        };
        let (dx, dy) = match side {
            Side::Left => (-1.0, 0.0),
            Side::Right => (1.0, 0.0),
            Side::Top => (0.0, -1.0),
            Side::Bottom => (0.0, 1.0),
        };
        self.tiles.retain(|(m, ..)| m != machine);
        self.tiles.push((machine.to_string(), x + dx, y + dy));
        self
    }

    /// Builds a layout directly (tests): `machine` at `(x, y)`.
    pub fn at(mut self, machine: &str, x: f64, y: f64) -> Self {
        self.tiles.retain(|(m, ..)| m != machine);
        self.tiles.push((machine.to_string(), x, y));
        self
    }

    fn place_of(&self, machine: &str) -> Option<(f64, f64)> {
        self.tiles.iter().find(|(m, ..)| m == machine).map(|(_, x, y)| (*x, *y))
    }

    /// The machines touching `side` of `machine` anywhere along it, in the order they were placed.
    pub fn neighbors(&self, machine: &str, side: Side) -> Vec<String> {
        let Some(me) = self.place_of(machine) else {
            return Vec::new();
        };
        self.tiles
            .iter()
            .filter(|(m, x, y)| m != machine && touching_side(me, (*x, *y)) == Some(side))
            .map(|(m, ..)| m.clone())
            .collect()
    }

    /// The first machine beyond `side` of `machine` (whether online or not).
    pub fn neighbor(&self, machine: &str, side: Side) -> Option<String> {
        self.neighbors(machine, side).into_iter().next()
    }

    /// The machines beyond `side` of `machine` at `along` (0.0 to 1.0 of that edge), with where the
    /// pointer comes in along their edge; several when they share a place. Empty where no tile
    /// touches the edge.
    pub fn across(&self, machine: &str, side: Side, along: f64) -> Vec<(String, f64)> {
        let Some((x, y)) = self.place_of(machine) else {
            return Vec::new();
        };
        let along = along.clamp(0.0, 1.0);
        // The point on the edge in desk units, and which coordinate runs along it
        let (edge, at, vertical) = match side {
            Side::Left => (x, y + along, true),
            Side::Right => (x + 1.0, y + along, true),
            Side::Top => (y, x + along, false),
            Side::Bottom => (y + 1.0, x + along, false),
        };
        self.tiles
            .iter()
            .filter(|(m, ..)| m != machine)
            .filter_map(|(m, tx, ty)| {
                let (near, far, start) = if vertical {
                    (*tx, tx + 1.0, *ty)
                } else {
                    (*ty, ty + 1.0, *tx)
                };
                let touches = match side {
                    Side::Left | Side::Top => (far - edge).abs() < EPS,
                    Side::Right | Side::Bottom => (near - edge).abs() < EPS,
                };
                let covers = at >= start - EPS && at < start + 1.0 - EPS
                    || (along >= 1.0 - EPS && (at - (start + 1.0)).abs() < EPS);
                (touches && covers).then(|| (m.clone(), (at - start).clamp(0.0, 1.0)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(name: &str) -> Option<String> {
        (name != "unknown").then(|| name.to_string())
    }

    #[test]
    fn neighbors_work_both_ways() {
        let file: LayoutFile = serde_json::from_str(
            r#"{"neighbors":[{"machine":"pc","side":"right","of":"mac"},
                             {"machine":"laptop","side":"top","of":"mac"},
                             {"machine":"unknown","side":"left","of":"mac"}],
                "swap_cmd_ctrl":["pc"]}"#,
        )
        .unwrap();
        let layout = Layout::from_file(&file, same).unwrap();
        assert_eq!(layout.neighbor("mac", Side::Right).as_deref(), Some("pc"));
        assert_eq!(layout.neighbor("pc", Side::Left).as_deref(), Some("mac"));
        assert_eq!(layout.neighbor("laptop", Side::Bottom).as_deref(), Some("mac"));
        assert_eq!(layout.neighbor("mac", Side::Left).as_deref(), None);
        assert!(layout.swap_cmd_ctrl.contains("pc"));
        assert_eq!(layout.corner, 16.0);
    }

    #[test]
    fn computers_used_in_turn_share_a_place() {
        let file: LayoutFile = serde_json::from_str(
            r#"{"neighbors":[{"machine":"book1","side":"left","of":"imac"},
                             {"machine":"book2","side":"left","of":"imac"}]}"#,
        )
        .unwrap();
        let layout = Layout::from_file(&file, same).unwrap();
        assert_eq!(layout.neighbors("imac", Side::Left), ["book1", "book2"]);
        assert_eq!(layout.neighbor("book2", Side::Right).as_deref(), Some("imac"));
        let file: LayoutFile =
            serde_json::from_str(r#"{"neighbors":[{"machine":"a","side":"left","of":"a"}]}"#).unwrap();
        assert!(Layout::from_file(&file, same).is_err());
    }

    #[test]
    fn half_beside_and_two_below() {
        // The iMac with a laptop half below its left side and another one half below its right side
        let file: LayoutFile = serde_json::from_str(
            r#"{"tiles":[{"machine":"imac","x":0,"y":0},
                         {"machine":"left","x":-0.5,"y":1},
                         {"machine":"right","x":0.5,"y":1},
                         {"machine":"far","x":3,"y":0}]}"#,
        )
        .unwrap();
        let layout = Layout::from_file(&file, same).unwrap();
        assert_eq!(layout.neighbors("imac", Side::Bottom), ["left", "right"]);
        assert_eq!(layout.neighbors("left", Side::Top), ["imac"]);
        // The two laptops stand side by side
        assert_eq!(layout.neighbors("left", Side::Right), ["right"]);
        assert!(layout.neighbors("far", Side::Left).is_empty());
        // The left quarter of the iMac's bottom edge leads to the right half of "left"
        let at = |m: &str, side, along| layout.across(m, side, along);
        assert_eq!(at("imac", Side::Bottom, 0.25), [("left".to_string(), 0.75)]);
        assert_eq!(at("imac", Side::Bottom, 0.75), [("right".to_string(), 0.25)]);
        // Only half of the laptop's top edge touches the iMac
        assert_eq!(at("left", Side::Top, 0.75), [("imac".to_string(), 0.25)]);
        assert!(at("left", Side::Top, 0.25).is_empty());
        assert!(at("imac", Side::Top, 0.5).is_empty());
    }

    #[test]
    fn old_neighbors_become_tiles_and_back() {
        let file: LayoutFile = serde_json::from_str(
            r#"{"neighbors":[{"machine":"laptop","side":"bottom","of":"imac"},
                             {"machine":"book","side":"left","of":"imac"},
                             {"machine":"tv","side":"right","of":"pi"}]}"#,
        )
        .unwrap();
        let tiles = file.effective_tiles();
        let at = |m: &str| tiles.iter().find(|t| t.machine == m).map(|t| (t.x, t.y)).unwrap();
        assert_eq!(at("imac"), (0.0, 0.0));
        assert_eq!(at("laptop"), (0.0, 1.0));
        assert_eq!(at("book"), (-1.0, 0.0));
        // A second group stands apart, to the right of the first one
        assert!(at("pi").0 >= 2.0);
        assert_eq!(at("tv"), (at("pi").0 + 1.0, at("pi").1));
        let mut back = neighbors_from_tiles(&tiles);
        back.sort_by(|a, b| a.machine.cmp(&b.machine));
        assert_eq!(
            back.iter()
                .map(|n| (n.machine.as_str(), n.side, n.of.as_str()))
                .collect::<Vec<_>>(),
            [
                ("book", Side::Left, "imac"),
                ("laptop", Side::Bottom, "imac"),
                ("tv", Side::Right, "pi")
            ]
        );
        // Setting tiles keeps the neighbors for older versions in step
        let mut file = LayoutFile::default();
        file.set_tiles(tiles);
        assert_eq!(file.neighbors.len(), 3);
    }
}
