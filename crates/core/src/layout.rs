//! Where the machines stand on the desk, and per-machine options. Stored as `layout.json`:
//!
//! ```json
//! {
//!   "tiles": [ { "machine": "MacBook", "x": 0, "y": 0 }, { "machine": "Gaming-PC", "x": 1, "y": 0.5 } ],
//!   "swap_cmd_ctrl": ["Gaming-PC"],
//!   "corner": 16
//! }
//! ```
//! Since 0.4.2 every machine is an area as large as its screens (`areas`, in thousands of logical
//! points). Areas that touch are neighbors where their edges overlap, so an area may sit half beside
//! another and one side may have several neighbors. Machines used in turn at the same place share a
//! position. Machines are named by their name or the start of their fingerprint.
//!
//! Older versions stored tiles of 1 × 1 (0.4.0) or `neighbors` ("Gaming-PC is on the right of
//! MacBook", over the whole edge); those are read as areas of a stand-in size on a grid.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::geometry::Side;

/// One machine on the desk with its size; the same type travels between computers.
pub use tc_protocol::Area;
/// One entry of the old arrangement; the same type travels between computers.
pub use tc_protocol::Placement as Neighbor;
/// One machine on the desk (0.4.0, size 1 × 1); the same type travels between computers.
pub use tc_protocol::Tile;

/// Size of a machine whose screens are not known yet: one full HD screen, in thousands of points.
pub const STAND_IN: (f32, f32) = (1.92, 1.08);

/// Positions closer than this count as the same (thousands of points: one point).
const EPS: f64 = 1e-3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutFile {
    /// Where the machines stand and how large they are. Empty in files of older versions: then
    /// `tiles` or `neighbors` count.
    #[serde(default)]
    pub areas: Vec<Area>,
    /// The arrangement of 0.4.0 (kept empty once there are areas).
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
            areas: Vec::new(),
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
    /// The areas, or for a file of an older version those of its tiles or neighbors.
    pub fn effective_areas(&self) -> Vec<Area> {
        if !self.areas.is_empty() {
            self.areas.clone()
        } else if !self.tiles.is_empty() {
            areas_from_tiles(&self.tiles)
        } else {
            areas_from_tiles(&tiles_from_neighbors(&self.neighbors))
        }
    }

    /// Sets the arrangement; `neighbors` follow for older versions.
    pub fn set_areas(&mut self, areas: Vec<Area>) {
        self.neighbors = neighbors_from_areas(&areas);
        self.tiles.clear();
        self.areas = areas;
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

/// Tiles of 0.4.0 as areas of the stand-in size on the same grid.
pub fn areas_from_tiles(tiles: &[Tile]) -> Vec<Area> {
    let (w, h) = STAND_IN;
    tiles
        .iter()
        .map(|t| Area {
            machine: t.machine.clone(),
            x: t.x * w,
            y: t.y * h,
            w,
            h,
            measured: false,
        })
        .collect()
}

/// The neighbors of areas for older versions: areas that touch with some overlap, over the whole
/// edge (older versions know nothing finer).
pub fn neighbors_from_areas(areas: &[Area]) -> Vec<Neighbor> {
    let mut neighbors = Vec::new();
    for (i, a) in areas.iter().enumerate() {
        for b in &areas[i + 1..] {
            if a.machine == b.machine {
                continue;
            }
            if let Some(side) = touching_side(rect(a), rect(b)) {
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

/// x, y, width, height.
type R = (f64, f64, f64, f64);

fn rect(a: &Area) -> R {
    (f64::from(a.x), f64::from(a.y), f64::from(a.w), f64::from(a.h))
}

/// On which side of area `a` area `b` touches it with some overlap.
pub fn area_side(a: &Area, b: &Area) -> Option<Side> {
    touching_side(rect(a), rect(b))
}

/// On which side of `a` `b` touches it with some overlap.
fn touching_side(a: R, b: R) -> Option<Side> {
    let overlap = |p: f64, pl: f64, q: f64, ql: f64| (p + pl).min(q + ql) - p.max(q) > EPS;
    if (b.0 - (a.0 + a.2)).abs() < EPS && overlap(a.1, a.3, b.1, b.3) {
        Some(Side::Right)
    } else if (a.0 - (b.0 + b.2)).abs() < EPS && overlap(a.1, a.3, b.1, b.3) {
        Some(Side::Left)
    } else if (b.1 - (a.1 + a.3)).abs() < EPS && overlap(a.0, a.2, b.0, b.2) {
        Some(Side::Bottom)
    } else if (a.1 - (b.1 + b.3)).abs() < EPS && overlap(a.0, a.2, b.0, b.2) {
        Some(Side::Top)
    } else {
        None
    }
}

/// The arrangement after some machines changed size (`size` gives the new one, `None` keeps it):
/// starting at `anchor`, every machine keeps the side it touches its neighbor on and the place of
/// its middle along that edge, so the desk keeps its shape. Machines sharing a place stay together;
/// machines not connected to `anchor` keep their corner.
pub fn resized(areas: &[Area], size: impl Fn(&Area) -> Option<(f32, f32)>, anchor: &str) -> Vec<Area> {
    let mut out: Vec<Area> = areas
        .iter()
        .map(|a| {
            let (w, h) = size(a).unwrap_or((a.w, a.h));
            Area { w, h, ..a.clone() }
        })
        .collect();
    let same = |a: &Area, b: &Area| (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3;
    let Some(start) = areas.iter().position(|a| a.machine == anchor) else {
        return out;
    };
    let mut done = vec![false; areas.len()];
    done[start] = true;
    let mut queue = vec![start];
    while let Some(p) = queue.pop() {
        for c in 0..areas.len() {
            if done[c] {
                continue;
            }
            let (old_p, old_c) = (&areas[p], &areas[c]);
            let new_p = out[p].clone();
            let (x, y) = if same(old_p, old_c) {
                (new_p.x, new_p.y)
            } else if let Some(side) = area_side(old_p, old_c) {
                let (w, h) = (out[c].w, out[c].h);
                match side {
                    Side::Left | Side::Right => {
                        let r = (old_c.y + old_c.h / 2.0 - old_p.y) / old_p.h;
                        let y = new_p.y + r * new_p.h - h / 2.0;
                        (
                            if side == Side::Right {
                                new_p.x + new_p.w
                            } else {
                                new_p.x - w
                            },
                            y,
                        )
                    }
                    Side::Top | Side::Bottom => {
                        let r = (old_c.x + old_c.w / 2.0 - old_p.x) / old_p.w;
                        let x = new_p.x + r * new_p.w - w / 2.0;
                        (
                            x,
                            if side == Side::Bottom {
                                new_p.y + new_p.h
                            } else {
                                new_p.y - h
                            },
                        )
                    }
                }
            } else {
                continue;
            };
            out[c].x = x;
            out[c].y = y;
            done[c] = true;
            queue.push(c);
        }
    }
    out
}

/// The resolved arrangement, by machine id.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// In the order they were placed: of machines sharing a place, the first one online counts.
    tiles: Vec<(String, R)>,
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
        for area in file.effective_areas() {
            let r = rect(&area);
            if let Some(machine) = resolve(&area.machine)
                && [r.0, r.1, r.2, r.3].iter().all(|v| v.is_finite())
                && r.2 > 0.0
                && r.3 > 0.0
                && !layout.tiles.iter().any(|(m, _)| *m == machine)
            {
                layout.tiles.push((machine, r));
            }
        }
        layout.swap_cmd_ctrl = file.swap_cmd_ctrl.iter().filter_map(|n| resolve(n)).collect();
        Ok(layout)
    }

    /// Builds a layout directly (tests): `machine` on `side` of `of` over the whole edge, both
    /// 1 × 1; `of` is put at `(0, 0)` if it has no place yet.
    pub fn with(mut self, machine: &str, side: Side, of: &str) -> Self {
        let (x, y, ..) = match self.place_of(of) {
            Some(r) => r,
            None => {
                self.tiles.push((of.to_string(), (0.0, 0.0, 1.0, 1.0)));
                (0.0, 0.0, 1.0, 1.0)
            }
        };
        let (dx, dy) = match side {
            Side::Left => (-1.0, 0.0),
            Side::Right => (1.0, 0.0),
            Side::Top => (0.0, -1.0),
            Side::Bottom => (0.0, 1.0),
        };
        self.at_sized(machine, x + dx, y + dy, 1.0, 1.0)
    }

    /// Builds a layout directly (tests): `machine` at `(x, y)`, 1 × 1.
    pub fn at(self, machine: &str, x: f64, y: f64) -> Self {
        self.at_sized(machine, x, y, 1.0, 1.0)
    }

    /// Builds a layout directly (tests): `machine` at `(x, y)` with its size.
    pub fn at_sized(mut self, machine: &str, x: f64, y: f64, w: f64, h: f64) -> Self {
        self.tiles.retain(|(m, _)| m != machine);
        self.tiles.push((machine.to_string(), (x, y, w, h)));
        self
    }

    fn place_of(&self, machine: &str) -> Option<R> {
        self.tiles.iter().find(|(m, _)| m == machine).map(|(_, r)| *r)
    }

    /// The machines touching `side` of `machine` anywhere along it, in the order they were placed.
    pub fn neighbors(&self, machine: &str, side: Side) -> Vec<String> {
        let Some(me) = self.place_of(machine) else {
            return Vec::new();
        };
        self.tiles
            .iter()
            .filter(|(m, r)| m != machine && touching_side(me, *r) == Some(side))
            .map(|(m, _)| m.clone())
            .collect()
    }

    /// The first machine beyond `side` of `machine` (whether online or not).
    pub fn neighbor(&self, machine: &str, side: Side) -> Option<String> {
        self.neighbors(machine, side).into_iter().next()
    }

    /// The machines beyond `side` of `machine` at `along` (0.0 to 1.0 of that edge), with where the
    /// pointer comes in along their edge; several when they share a place. Empty where no machine
    /// touches the edge.
    pub fn across(&self, machine: &str, side: Side, along: f64) -> Vec<(String, f64)> {
        let Some((x, y, w, h)) = self.place_of(machine) else {
            return Vec::new();
        };
        let along = along.clamp(0.0, 1.0);
        // The point on the edge in desk units, and which coordinate runs along it
        let (edge, at, vertical) = match side {
            Side::Left => (x, y + along * h, true),
            Side::Right => (x + w, y + along * h, true),
            Side::Top => (y, x + along * w, false),
            Side::Bottom => (y + h, x + along * w, false),
        };
        let end = along >= 1.0 - 1e-9;
        self.tiles
            .iter()
            .filter(|(m, _)| m != machine)
            .filter_map(|(m, (tx, ty, tw, th))| {
                let (near, far, start, len) = if vertical {
                    (*tx, tx + tw, *ty, *th)
                } else {
                    (*ty, ty + th, *tx, *tw)
                };
                let touches = match side {
                    Side::Left | Side::Top => (far - edge).abs() < EPS,
                    Side::Right | Side::Bottom => (near - edge).abs() < EPS,
                };
                let covers = at >= start - EPS && at < start + len - EPS || (end && (at - (start + len)).abs() < EPS);
                (touches && covers).then(|| (m.clone(), ((at - start) / len).clamp(0.0, 1.0)))
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
    fn old_neighbors_become_areas_and_back() {
        let file: LayoutFile = serde_json::from_str(
            r#"{"neighbors":[{"machine":"laptop","side":"bottom","of":"imac"},
                             {"machine":"book","side":"left","of":"imac"},
                             {"machine":"tv","side":"right","of":"pi"}]}"#,
        )
        .unwrap();
        let areas = file.effective_areas();
        let (w, h) = STAND_IN;
        let at = |m: &str| areas.iter().find(|a| a.machine == m).map(|a| (a.x, a.y)).unwrap();
        assert_eq!(at("imac"), (0.0, 0.0));
        assert_eq!(at("laptop"), (0.0, h));
        assert_eq!(at("book"), (-w, 0.0));
        // A second group stands apart, to the right of the first one
        assert!(at("pi").0 >= 2.0 * w);
        assert_eq!(at("tv"), (at("pi").0 + w, at("pi").1));
        let mut back = neighbors_from_areas(&areas);
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
        // Setting areas keeps the neighbors for older versions in step
        let mut file = LayoutFile::default();
        file.set_areas(areas);
        assert_eq!(file.neighbors.len(), 3);
    }

    fn area(m: &str, x: f32, y: f32, w: f32, h: f32) -> Area {
        Area {
            machine: m.into(),
            x,
            y,
            w,
            h,
            measured: true,
        }
    }

    #[test]
    fn real_sizes_cross_at_the_matching_place() {
        // A wide iMac, a small laptop below its left part
        let file = LayoutFile {
            areas: vec![area("imac", 0.0, 0.0, 4.0, 2.0), area("book", 0.5, 2.0, 1.0, 0.75)],
            ..LayoutFile::default()
        };
        let layout = Layout::from_file(&file, same).unwrap();
        assert_eq!(layout.across("imac", Side::Bottom, 0.25), [("book".to_string(), 0.5)]);
        assert!(layout.across("imac", Side::Bottom, 0.75).is_empty());
        assert_eq!(layout.across("book", Side::Top, 0.5), [("imac".to_string(), 0.25)]);
    }

    #[test]
    fn a_new_size_keeps_the_desk_together() {
        // The laptop below the middle of the iMac, a PC right of the iMac, a TV sharing the PC's place
        let areas = vec![
            area("imac", 0.0, 0.0, 2.0, 1.0),
            area("book", 0.5, 1.0, 1.0, 1.0),
            area("pc", 2.0, 0.0, 1.0, 1.0),
            area("tv", 2.0, 0.0, 1.0, 1.0),
            area("far", 9.0, 9.0, 1.0, 1.0),
        ];
        let sizes = |a: &Area| match a.machine.as_str() {
            "imac" => Some((5.12, 1.44)),
            "book" => Some((1.728, 1.117)),
            _ => None,
        };
        let out = resized(&areas, sizes, "imac");
        let get = |m: &str| out.iter().find(|a| a.machine == m).unwrap().clone();
        assert_eq!((get("imac").x, get("imac").y, get("imac").w), (0.0, 0.0, 5.12));
        // Still below the middle of the iMac
        let book = get("book");
        assert!((book.y - 1.44).abs() < 1e-4 && (book.x + book.w / 2.0 - 2.56).abs() < 1e-4);
        // Still right of it, in the middle of its height, the TV with it
        let pc = get("pc");
        assert!((pc.x - 5.12).abs() < 1e-4 && (pc.y + 0.5 - 0.72).abs() < 1e-4);
        assert_eq!((get("tv").x, get("tv").y), (pc.x, pc.y));
        assert_eq!((get("far").x, get("far").y), (9.0, 9.0));
        // Everything that touched still touches
        assert_eq!(area_side(&get("imac"), &book), Some(Side::Bottom));
        assert_eq!(area_side(&get("imac"), &pc), Some(Side::Right));
    }
}
