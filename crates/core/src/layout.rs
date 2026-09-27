//! Which machine sits next to which, and per-machine options. Stored as `layout.json`:
//!
//! ```json
//! {
//!   "neighbors": [ { "machine": "Gaming-PC", "side": "right", "of": "MacBook" } ],
//!   "swap_cmd_ctrl": ["Gaming-PC"],
//!   "corner": 16
//! }
//! ```
//! "Gaming-PC is on the right of MacBook" also means "MacBook is on the left of Gaming-PC".
//! Machines are named by their name or the start of their fingerprint.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::geometry::Side;

/// One entry of the arrangement; the same type travels between computers.
pub use tc_protocol::Placement as Neighbor;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutFile {
    #[serde(default)]
    pub neighbors: Vec<Neighbor>,
    /// Machines that get ⌘ and Ctrl swapped when controlled from here.
    #[serde(default)]
    pub swap_cmd_ctrl: Vec<String>,
    /// No switching within this many logical points of a screen corner.
    #[serde(default = "default_corner")]
    pub corner: f64,
    /// Version of `neighbors`, shared between computers: the highest `(revision, editor)` wins.
    #[serde(default)]
    pub revision: u64,
    /// Machine id of the computer that made this version.
    #[serde(default)]
    pub editor: String,
    /// Display names of the machines in `neighbors`, by machine id.
    #[serde(default)]
    pub names: BTreeMap<String, String>,
}

fn default_corner() -> f64 {
    16.0
}

impl Default for LayoutFile {
    fn default() -> Self {
        Self {
            neighbors: Vec::new(),
            swap_cmd_ctrl: Vec::new(),
            corner: default_corner(),
            revision: 0,
            editor: String::new(),
            names: BTreeMap::new(),
        }
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

/// Resolved neighbors, keyed by machine id. Several machines may share a place (computers used in
/// turn at the same spot); the first of them that is online counts.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    neighbors: HashMap<(String, Side), Vec<String>>,
    pub swap_cmd_ctrl: HashSet<String>,
    pub corner: f64,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            neighbors: HashMap::new(),
            swap_cmd_ctrl: HashSet::new(),
            corner: default_corner(),
        }
    }
}

impl Layout {
    /// `resolve` turns a name from the file into a machine id (`None`: unknown, entry ignored).
    pub fn from_file(file: &LayoutFile, resolve: impl Fn(&str) -> Option<String>) -> Result<Self, LayoutError> {
        let mut layout = Layout {
            corner: if file.corner.is_finite() && file.corner >= 0.0 {
                file.corner
            } else {
                default_corner()
            },
            ..Layout::default()
        };
        for n in &file.neighbors {
            let (Some(machine), Some(of)) = (resolve(&n.machine), resolve(&n.of)) else {
                continue;
            };
            if machine == of {
                return Err(LayoutError::SelfNeighbor(n.machine.clone()));
            }
            layout.set(&of, n.side, &machine);
            layout.set(&machine, n.side.opposite(), &of);
        }
        layout.swap_cmd_ctrl = file.swap_cmd_ctrl.iter().filter_map(|n| resolve(n)).collect();
        Ok(layout)
    }

    fn set(&mut self, machine: &str, side: Side, neighbor: &str) {
        let list = self.neighbors.entry((machine.to_string(), side)).or_default();
        if !list.iter().any(|n| n == neighbor) {
            list.push(neighbor.to_string());
        }
    }

    /// Builds a layout directly (tests).
    pub fn with(mut self, machine: &str, side: Side, of: &str) -> Self {
        self.set(of, side, machine);
        self.set(machine, side.opposite(), of);
        self
    }

    /// The machines beyond `side` of `machine`, in the order they were placed.
    pub fn neighbors(&self, machine: &str, side: Side) -> &[String] {
        self.neighbors
            .get(&(machine.to_string(), side))
            .map_or(&[], Vec::as_slice)
    }

    /// The first machine beyond `side` of `machine` (whether online or not).
    pub fn neighbor(&self, machine: &str, side: Side) -> Option<&str> {
        self.neighbors(machine, side).first().map(String::as_str)
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
        assert_eq!(layout.neighbor("mac", Side::Right), Some("pc"));
        assert_eq!(layout.neighbor("pc", Side::Left), Some("mac"));
        assert_eq!(layout.neighbor("laptop", Side::Bottom), Some("mac"));
        assert_eq!(layout.neighbor("mac", Side::Left), None);
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
        assert_eq!(layout.neighbor("book2", Side::Right), Some("imac"));
        let file: LayoutFile =
            serde_json::from_str(r#"{"neighbors":[{"machine":"a","side":"left","of":"a"}]}"#).unwrap();
        assert!(Layout::from_file(&file, same).is_err());
    }
}
