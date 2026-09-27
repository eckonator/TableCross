//! `layout.json` in the settings folder: where the paired computers stand.
//!
//! Machines are stored by the first 16 hex digits of their fingerprint, so renaming a computer does
//! not break the layout; names and fingerprint prefixes are accepted when reading.
//!
//! The arrangement is shared: after every change it goes to all connected computers, and each one
//! keeps the version with the highest `(revision, editor)`. Revisions are milliseconds of the wall
//! clock but always above every revision seen, so a computer with a late clock still wins with
//! a newer change.

use std::path::Path;

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tc_core::{LayoutFile, Neighbor, Side};
use tc_net::Fingerprint;
use tc_protocol::{MAX_PLACEMENTS, Message};

pub const LAYOUT_FILE: &str = "layout.json";

pub fn load(dir: &Path) -> Result<LayoutFile> {
    let path = dir.join(LAYOUT_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("{} is not valid", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LayoutFile::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

pub fn save(dir: &Path, file: &LayoutFile) -> Result<()> {
    let path = dir.join(LAYOUT_FILE);
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(file)?)?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))
}

pub fn machine_id(fp: &Fingerprint) -> String {
    fp.to_string()[..16].to_string()
}

/// Whether a name in the file means this machine (`fp`, `name`).
fn names(entry: &str, fp: &Fingerprint, name: &str) -> bool {
    let entry = entry.to_lowercase();
    entry == name.to_lowercase() || (entry.len() >= 8 && fp.to_string().starts_with(&entry))
}

/// Puts `peer` on `side` of `me` (`None` removes it). Whoever stood on that side before moves away.
pub fn place(file: &mut LayoutFile, me: &Fingerprint, peer: &Fingerprint, side: Option<Side>) {
    let both = |n: &Neighbor| {
        (names(&n.machine, peer, "") && names(&n.of, me, "")) || (names(&n.machine, me, "") && names(&n.of, peer, ""))
    };
    file.neighbors.retain(|n| !both(n));
    if let Some(side) = side {
        file.neighbors.retain(|n| {
            !(names(&n.of, me, "") && n.side == side) && !(names(&n.machine, me, "") && n.side.opposite() == side)
        });
        file.neighbors.push(Neighbor {
            machine: machine_id(peer),
            side,
            of: machine_id(me),
        });
    }
}

/// Where `peer` stands next to `me`, if anywhere.
pub fn side_of(
    file: &LayoutFile,
    me: &Fingerprint,
    my_name: &str,
    peer: &Fingerprint,
    peer_name: &str,
) -> Option<Side> {
    file.neighbors.iter().find_map(|n| {
        if names(&n.machine, peer, peer_name) && names(&n.of, me, my_name) {
            Some(n.side)
        } else if names(&n.machine, me, my_name) && names(&n.of, peer, peer_name) {
            Some(n.side.opposite())
        } else {
            None
        }
    })
}

/// Marks the arrangement as changed here, so other computers take it over.
pub fn bump(file: &mut LayoutFile, me: &Fingerprint) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    file.revision = now.max(file.revision + 1);
    file.editor = machine_id(me);
}

/// The arrangement for other computers. `resolve` turns a name from the file into a fingerprint
/// (`None`: unknown, sent as written); `known` gives display names of fingerprints.
pub fn message(
    file: &LayoutFile,
    resolve: impl Fn(&str) -> Option<Fingerprint>,
    known: impl Fn(&Fingerprint) -> Option<String>,
) -> Message {
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    let mut id = |entry: &str| match resolve(entry) {
        Some(fp) => {
            let id = machine_id(&fp);
            if let Some(name) = known(&fp).or_else(|| file.names.get(&id).cloned()) {
                names.insert(id.clone(), name);
            }
            id
        }
        None => {
            if let Some(name) = file.names.get(entry) {
                names.insert(entry.to_string(), name.clone());
            }
            entry.to_string()
        }
    };
    let placements = file
        .neighbors
        .iter()
        .take(MAX_PLACEMENTS)
        .map(|n| Neighbor {
            machine: id(&n.machine),
            side: n.side,
            of: id(&n.of),
        })
        .collect();
    Message::Layout {
        revision: file.revision,
        editor: file.editor.clone(),
        placements,
        names: names.into_iter().take(MAX_PLACEMENTS).collect(),
    }
}

/// Takes over an arrangement from another computer if it is newer than ours.
pub fn adopt(
    file: &mut LayoutFile,
    revision: u64,
    editor: String,
    placements: Vec<Neighbor>,
    names: Vec<(String, String)>,
) -> bool {
    if (revision, editor.as_str()) <= (file.revision, file.editor.as_str()) {
        return false;
    }
    file.revision = revision;
    file.editor = editor;
    file.neighbors = placements;
    file.names = names.into_iter().collect();
    true
}

/// Where each machine stands on the desk: grid cells `(column, row)` around `me` at `(0, 0)`, found by
/// following the placements outward. `canonical` maps an entry of the file to one id per machine.
/// Several machines may share a cell (computers used in turn at the same place).
pub fn desk(file: &LayoutFile, me: &str, canonical: impl Fn(&str) -> String) -> Vec<(String, i32, i32)> {
    let step = |side: Side| match side {
        Side::Left => (-1, 0),
        Side::Right => (1, 0),
        Side::Top => (0, -1),
        Side::Bottom => (0, 1),
    };
    let links: Vec<(String, Side, String)> = file
        .neighbors
        .iter()
        .map(|n| (canonical(&n.machine), n.side, canonical(&n.of)))
        .collect();
    let mut placed = vec![(me.to_string(), 0, 0)];
    let mut next = 0;
    while next < placed.len() {
        let (current, x, y) = placed[next].clone();
        next += 1;
        for (machine, side, of) in &links {
            let (other, (dx, dy)) = if *of == current {
                (machine, step(*side))
            } else if *machine == current {
                (of, step(side.opposite()))
            } else {
                continue;
            };
            let cell = (x + dx, y + dy);
            if !placed.iter().any(|(m, ..)| m == other) {
                placed.push((other.clone(), cell.0, cell.1));
            }
        }
    }
    placed
}

/// The machines of a desk in reading order (row by row, left to right): number 1 is the first.
/// Ctrl+Shift+Alt+number goes to that computer.
pub fn reading_order(cells: &[(String, i32, i32)]) -> Vec<String> {
    let mut sorted: Vec<_> = cells.iter().collect();
    sorted.sort_by_key(|(_, x, y)| (*y, *x));
    sorted.into_iter().map(|(m, ..)| m.clone()).collect()
}

/// The neighbors of an arrangement drawn on a grid: computers in cells next to each other become
/// neighbors (all of a shared cell). Only what is connected to `me` counts; computers on their own
/// island are left out.
pub fn from_grid(cells: &[(String, i32, i32)], me: &str) -> Vec<Neighbor> {
    let at = |x: i32, y: i32| cells.iter().filter(move |(_, cx, cy)| (*cx, *cy) == (x, y));
    let mut reached: Vec<&(String, i32, i32)> = cells.iter().filter(|(m, ..)| m == me).take(1).collect();
    let mut next = 0;
    while next < reached.len() {
        let (_, x, y) = reached[next];
        next += 1;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            for cell in at(x + dx, y + dy) {
                if !reached.iter().any(|r| r.0 == cell.0) {
                    reached.push(cell);
                }
            }
        }
    }
    let mut neighbors = Vec::new();
    for (machine, x, y) in &reached {
        for (dx, dy, side) in [(1, 0, Side::Right), (0, 1, Side::Bottom)] {
            for (other, ..) in at(x + dx, y + dy) {
                neighbors.push(Neighbor {
                    machine: other.clone(),
                    side,
                    of: machine.clone(),
                });
            }
        }
    }
    neighbors
}

pub fn set_swap(file: &mut LayoutFile, peer: &Fingerprint, swap: bool) {
    file.swap_cmd_ctrl.retain(|m| !names(m, peer, ""));
    if swap {
        file.swap_cmd_ctrl.push(machine_id(peer));
    }
}

pub fn swaps(file: &LayoutFile, peer: &Fingerprint, peer_name: &str) -> bool {
    file.swap_cmd_ctrl.iter().any(|m| names(m, peer, peer_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Fingerprint = Fingerprint([1; 32]);
    const PC: Fingerprint = Fingerprint([2; 32]);
    const TV: Fingerprint = Fingerprint([3; 32]);

    #[test]
    fn place_and_read_back() {
        let mut file = LayoutFile::default();
        place(&mut file, &ME, &PC, Some(Side::Right));
        assert_eq!(side_of(&file, &ME, "mac", &PC, "pc"), Some(Side::Right));
        // Moving it replaces the old place
        place(&mut file, &ME, &PC, Some(Side::Top));
        assert_eq!(file.neighbors.len(), 1);
        assert_eq!(side_of(&file, &ME, "mac", &PC, "pc"), Some(Side::Top));
        // Another one on the same side pushes it away
        place(&mut file, &ME, &TV, Some(Side::Top));
        assert_eq!(side_of(&file, &ME, "mac", &PC, "pc"), None);
        assert_eq!(side_of(&file, &ME, "mac", &TV, "tv"), Some(Side::Top));
        place(&mut file, &ME, &TV, None);
        assert!(file.neighbors.is_empty());
    }

    #[test]
    fn written_from_the_other_side_or_by_name() {
        let file: LayoutFile =
            serde_json::from_str(r#"{"neighbors":[{"machine":"mac","side":"left","of":"pc"}]}"#).unwrap();
        assert_eq!(side_of(&file, &ME, "Mac", &PC, "PC"), Some(Side::Right));
        assert_eq!(side_of(&file, &PC, "PC", &ME, "Mac"), Some(Side::Left));
    }

    #[test]
    fn swap_flag() {
        let mut file = LayoutFile::default();
        set_swap(&mut file, &PC, true);
        set_swap(&mut file, &PC, true);
        assert_eq!(file.swap_cmd_ctrl.len(), 1);
        assert!(swaps(&file, &PC, "pc"));
        set_swap(&mut file, &PC, false);
        assert!(!swaps(&file, &PC, "pc"));
    }

    fn resolve(entry: &str) -> Option<Fingerprint> {
        [ME, PC, TV].into_iter().find(|fp| names(entry, fp, ""))
    }

    fn known(fp: &Fingerprint) -> Option<String> {
        (*fp == PC).then(|| "PC".to_string())
    }

    fn adopt_message(file: &mut LayoutFile, message: Message) -> bool {
        let Message::Layout {
            revision,
            editor,
            placements,
            names,
        } = message
        else {
            panic!("not a layout");
        };
        adopt(file, revision, editor, placements, names)
    }

    #[test]
    fn newer_arrangements_are_taken_over() {
        let mut mine = LayoutFile::default();
        place(&mut mine, &ME, &PC, Some(Side::Right));
        bump(&mut mine, &ME);
        let mut theirs = LayoutFile::default();
        // Ours is newer: the other computer takes it, we keep ours
        assert!(adopt_message(&mut theirs, message(&mine, resolve, known)));
        assert_eq!(theirs.neighbors, mine.neighbors);
        assert_eq!(theirs.names.get(&machine_id(&PC)).map(String::as_str), Some("PC"));
        assert!(!adopt_message(&mut mine, message(&theirs, resolve, known)));
        // A change over there wins even with a clock far behind
        place(&mut theirs, &PC, &TV, Some(Side::Top));
        theirs.revision = mine.revision;
        bump(&mut theirs, &PC);
        assert!(theirs.revision > mine.revision);
        assert!(adopt_message(&mut mine, message(&theirs, resolve, known)));
        assert_eq!(side_of(&mine, &PC, "", &TV, ""), Some(Side::Top));
        assert_eq!(side_of(&mine, &ME, "", &PC, ""), Some(Side::Right));
    }

    #[test]
    fn same_revision_is_decided_by_the_editor() {
        let a = LayoutFile {
            revision: 5,
            editor: "aaaa".into(),
            ..LayoutFile::default()
        };
        let b = LayoutFile {
            revision: 5,
            editor: "bbbb".into(),
            ..LayoutFile::default()
        };
        assert!(adopt_message(&mut a.clone(), message(&b, resolve, known)));
        assert!(!adopt_message(&mut b.clone(), message(&a, resolve, known)));
        // Two old files without revision leave each other alone
        assert!(!adopt_message(
            &mut LayoutFile::default(),
            message(&LayoutFile::default(), resolve, known)
        ));
    }

    #[test]
    fn names_in_the_file_travel_as_machine_ids() {
        let file: LayoutFile =
            serde_json::from_str(r#"{"neighbors":[{"machine":"0202020202020202","side":"left","of":"somebody"}]}"#)
                .unwrap();
        let Message::Layout { placements, .. } = message(&file, resolve, known) else {
            panic!("not a layout");
        };
        assert_eq!(placements[0].machine, machine_id(&PC));
        assert_eq!(placements[0].of, "somebody");
    }

    #[test]
    fn the_whole_desk_around_this_computer() {
        let id = |fp: &Fingerprint| machine_id(fp);
        let mut file = LayoutFile::default();
        place(&mut file, &ME, &PC, Some(Side::Right));
        place(&mut file, &PC, &TV, Some(Side::Top));
        let desk = desk(&file, &id(&PC), |e| e.to_string());
        assert_eq!(desk, vec![(id(&PC), 0, 0), (id(&ME), -1, 0), (id(&TV), 0, -1)]);
    }

    #[test]
    fn grid_becomes_neighbors_and_back() {
        let cell = |m: &str, x, y| (m.to_string(), x, y);
        // Two computers in a row on the right, one above the first, one far away
        let grid = vec![
            cell("me", 0, 0),
            cell("pc", 1, 0),
            cell("tv", 2, 0),
            cell("pi", 1, -1),
            cell("far", 4, 4),
        ];
        let file = LayoutFile {
            neighbors: from_grid(&grid, "me"),
            ..LayoutFile::default()
        };
        assert_eq!(file.neighbors.len(), 3);
        let mut back = desk(&file, "me", |e| e.to_string());
        back.sort();
        let mut expected = grid[..4].to_vec();
        expected.sort();
        assert_eq!(back, expected);
    }

    #[test]
    fn laptops_sharing_a_place_stay_together() {
        let cell = |m: &str, x, y| (m.to_string(), x, y);
        let grid = vec![cell("imac", 0, 0), cell("book1", -1, 0), cell("book2", -1, 0)];
        let file = LayoutFile {
            neighbors: from_grid(&grid, "imac"),
            ..LayoutFile::default()
        };
        assert_eq!(file.neighbors.len(), 2);
        let mut back = desk(&file, "imac", |e| e.to_string());
        back.sort();
        let mut expected = grid.clone();
        expected.sort();
        assert_eq!(back, expected);
    }

    #[test]
    fn numbered_like_reading() {
        let cell = |m: &str, x, y| (m.to_string(), x, y);
        let desk = vec![cell("me", 0, 0), cell("pc", 1, 0), cell("pi", -1, 0), cell("tv", 0, -1)];
        assert_eq!(reading_order(&desk), ["tv", "pi", "me", "pc"]);
    }

    #[test]
    fn file_round_trip() {
        let dir = std::env::temp_dir().join(format!("tablecross-layout-{}", std::process::id()));
        let mut file = LayoutFile::default();
        place(&mut file, &ME, &PC, Some(Side::Left));
        save(&dir, &file).unwrap();
        assert_eq!(load(&dir).unwrap(), file);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(load(&dir).unwrap(), LayoutFile::default());
    }
}
