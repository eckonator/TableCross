//! `layout.json` in the settings folder: where the paired computers stand.
//!
//! Machines are stored by the first 16 hex digits of their fingerprint, so renaming a computer does
//! not break the layout; names and fingerprint prefixes are accepted when reading.
//!
//! The arrangement is shared: after every change it goes to all connected computers, and each one
//! keeps the version with the highest `(revision, editor)`. Revisions are milliseconds of the wall
//! clock but always above every revision seen, so a computer with a late clock still wins with
//! a newer change.
//!
//! Version 0.4.0 arranges tiles that may stand offset; older versions know only whole-edge
//! neighbors. They get those, derived from the tiles, and what they send is read as tiles on a grid.

use std::path::Path;

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tc_core::{LayoutFile, Neighbor, Side, Tile, tile_side, tiles_from_neighbors};
use tc_net::Fingerprint;
use tc_protocol::{MAX_PLACEMENTS, Message};

pub const LAYOUT_FILE: &str = "layout.json";
/// Copy of a layout from before tiles, made once when it is first read as tiles.
pub const OLD_LAYOUT_FILE: &str = "layout-before-0.4.json";

/// First version that knows `Message::Tiles`.
const TILES_SINCE: (u32, u32, u32) = (0, 4, 0);

pub fn load(dir: &Path) -> Result<LayoutFile> {
    let path = dir.join(LAYOUT_FILE);
    let mut file: LayoutFile = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("{} is not valid", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LayoutFile::default()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    if file.tiles.is_empty() && !file.neighbors.is_empty() {
        let old = dir.join(OLD_LAYOUT_FILE);
        if !old.exists() {
            let _ = std::fs::copy(&path, &old);
        }
        file.tiles = tiles_from_neighbors(&file.neighbors);
    }
    Ok(file)
}

/// Whether a peer running `app_version` understands `Message::Tiles`.
pub fn knows_tiles(app_version: &str) -> bool {
    let mut parts = app_version.split(['.', '-']).map(|p| p.parse::<u32>().unwrap_or(0));
    let version = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    version >= TILES_SINCE
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

fn step(side: Side) -> (f32, f32) {
    match side {
        Side::Left => (-1.0, 0.0),
        Side::Right => (1.0, 0.0),
        Side::Top => (0.0, -1.0),
        Side::Bottom => (0.0, 1.0),
    }
}

/// Puts `peer` on `side` of `me`, over the whole edge (`None` removes it). Whoever stood exactly
/// there before moves away.
pub fn place(file: &mut LayoutFile, me: &Fingerprint, peer: &Fingerprint, side: Option<Side>) {
    let mut tiles = file.effective_tiles();
    tiles.retain(|t| !names(&t.machine, peer, ""));
    if let Some(side) = side {
        let (x, y) = match tiles.iter().find(|t| names(&t.machine, me, "")) {
            Some(t) => (t.x, t.y),
            None => {
                tiles.push(Tile {
                    machine: machine_id(me),
                    x: 0.0,
                    y: 0.0,
                });
                (0.0, 0.0)
            }
        };
        let (dx, dy) = step(side);
        let (x, y) = (x + dx, y + dy);
        tiles.retain(|t| (t.x - x).abs() > 1e-3 || (t.y - y).abs() > 1e-3);
        tiles.push(Tile {
            machine: machine_id(peer),
            x,
            y,
        });
    }
    file.set_tiles(tiles);
}

/// Where `peer` stands next to `me`, if their tiles touch.
pub fn side_of(
    file: &LayoutFile,
    me: &Fingerprint,
    my_name: &str,
    peer: &Fingerprint,
    peer_name: &str,
) -> Option<Side> {
    let tiles = file.effective_tiles();
    let mine = tiles.iter().find(|t| names(&t.machine, me, my_name))?;
    let theirs = tiles.iter().find(|t| names(&t.machine, peer, peer_name))?;
    tile_side(mine, theirs)
}

/// Marks the arrangement as changed here, so other computers take it over.
pub fn bump(file: &mut LayoutFile, me: &Fingerprint) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    file.revision = now.max(file.revision + 1);
    file.editor = machine_id(me);
}

/// The arrangement for other computers: `Message::Tiles` for those that know it, else the old
/// `Message::Layout`. `resolve` turns a name from the file into a fingerprint (`None`: unknown,
/// sent as written); `known` gives display names of fingerprints.
pub fn message(
    file: &LayoutFile,
    tiles: bool,
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
    if tiles {
        let tiles = file
            .effective_tiles()
            .into_iter()
            .take(MAX_PLACEMENTS)
            .map(|t| Tile {
                machine: id(&t.machine),
                ..t
            })
            .collect();
        return Message::Tiles {
            revision: file.revision,
            editor: file.editor.clone(),
            tiles,
            names: names.into_iter().take(MAX_PLACEMENTS).collect(),
        };
    }
    let placements = tc_core::neighbors_from_tiles(&file.effective_tiles())
        .into_iter()
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

/// Takes over an arrangement from another computer if it is newer than ours: tiles from a
/// computer that knows them, else whole-edge neighbors from an older one.
pub fn adopt(
    file: &mut LayoutFile,
    revision: u64,
    editor: String,
    arrangement: Result<Vec<Tile>, Vec<Neighbor>>,
    names: Vec<(String, String)>,
) -> bool {
    if (revision, editor.as_str()) <= (file.revision, file.editor.as_str()) {
        return false;
    }
    file.revision = revision;
    file.editor = editor;
    match arrangement {
        Ok(tiles) => file.set_tiles(tiles),
        Err(neighbors) => {
            file.tiles = tiles_from_neighbors(&neighbors);
            file.neighbors = neighbors;
        }
    }
    file.names = names.into_iter().collect();
    true
}

/// Where each machine stands on the desk, one entry per machine: `canonical` maps an entry of the
/// file to one id per machine. Machines used in turn at the same place share a position.
pub fn desk(file: &LayoutFile, canonical: impl Fn(&str) -> String) -> Vec<(String, f32, f32)> {
    let mut placed: Vec<(String, f32, f32)> = Vec::new();
    for t in file.effective_tiles() {
        let id = canonical(&t.machine);
        if !placed.iter().any(|(m, ..)| *m == id) {
            placed.push((id, t.x, t.y));
        }
    }
    placed
}

/// The machines of a desk in reading order (row by row, left to right): number 1 is the first.
/// Ctrl+Shift+Alt+number goes to that computer.
pub fn reading_order(tiles: &[(String, f32, f32)]) -> Vec<String> {
    let mut sorted: Vec<_> = tiles.iter().collect();
    sorted.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.1.total_cmp(&b.1)));
    sorted.into_iter().map(|(m, ..)| m.clone()).collect()
}

/// A desk drawn on the arrangement page becomes the arrangement.
pub fn from_desk(tiles: &[(String, f32, f32)]) -> Vec<Tile> {
    tiles
        .iter()
        .map(|(machine, x, y)| Tile {
            machine: machine.clone(),
            x: *x,
            y: *y,
        })
        .collect()
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
        match message {
            Message::Layout {
                revision,
                editor,
                placements,
                names,
            } => adopt(file, revision, editor, Err(placements), names),
            Message::Tiles {
                revision,
                editor,
                tiles,
                names,
            } => adopt(file, revision, editor, Ok(tiles), names),
            _ => panic!("not a layout"),
        }
    }

    #[test]
    fn newer_arrangements_are_taken_over() {
        for tiles in [true, false] {
            let mut mine = LayoutFile::default();
            place(&mut mine, &ME, &PC, Some(Side::Right));
            bump(&mut mine, &ME);
            let mut theirs = LayoutFile::default();
            // Ours is newer: the other computer takes it, we keep ours
            assert!(adopt_message(&mut theirs, message(&mine, tiles, resolve, known)));
            assert_eq!(theirs.neighbors, mine.neighbors);
            assert_eq!(theirs.names.get(&machine_id(&PC)).map(String::as_str), Some("PC"));
            assert!(!adopt_message(&mut mine, message(&theirs, tiles, resolve, known)));
            // A change over there wins even with a clock far behind
            place(&mut theirs, &PC, &TV, Some(Side::Top));
            theirs.revision = mine.revision;
            bump(&mut theirs, &PC);
            assert!(theirs.revision > mine.revision);
            assert!(adopt_message(&mut mine, message(&theirs, tiles, resolve, known)));
            assert_eq!(side_of(&mine, &PC, "", &TV, ""), Some(Side::Top));
            assert_eq!(side_of(&mine, &ME, "", &PC, ""), Some(Side::Right));
        }
    }

    #[test]
    fn offsets_travel_only_to_computers_that_know_tiles() {
        let mut mine = LayoutFile::default();
        mine.set_tiles(vec![
            Tile {
                machine: machine_id(&ME),
                x: 0.0,
                y: 0.0,
            },
            Tile {
                machine: machine_id(&PC),
                x: 0.5,
                y: 1.0,
            },
        ]);
        bump(&mut mine, &ME);
        let mut new = LayoutFile::default();
        assert!(adopt_message(&mut new, message(&mine, true, resolve, known)));
        assert_eq!(new.tiles, mine.tiles);
        // An older computer gets the PC below this one, over the whole edge
        let Message::Layout { placements, .. } = message(&mine, false, resolve, known) else {
            panic!("not a layout");
        };
        assert_eq!(
            placements,
            [Neighbor {
                machine: machine_id(&PC),
                side: Side::Bottom,
                of: machine_id(&ME),
            }]
        );
        assert!(knows_tiles("0.4.0"));
        assert!(knows_tiles("1.0.0"));
        assert!(knows_tiles("0.4.1-test"));
        assert!(!knows_tiles("0.3.10"));
        assert!(!knows_tiles(""));
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
        assert!(adopt_message(&mut a.clone(), message(&b, true, resolve, known)));
        assert!(!adopt_message(&mut b.clone(), message(&a, true, resolve, known)));
        // Two old files without revision leave each other alone
        assert!(!adopt_message(
            &mut LayoutFile::default(),
            message(&LayoutFile::default(), false, resolve, known)
        ));
    }

    #[test]
    fn names_in_the_file_travel_as_machine_ids() {
        let file: LayoutFile =
            serde_json::from_str(r#"{"neighbors":[{"machine":"0202020202020202","side":"left","of":"somebody"}]}"#)
                .unwrap();
        let Message::Layout { placements, .. } = message(&file, false, resolve, known) else {
            panic!("not a layout");
        };
        assert_eq!(placements[0].machine, machine_id(&PC));
        assert_eq!(placements[0].of, "somebody");
        let Message::Tiles { tiles, .. } = message(&file, true, resolve, known) else {
            panic!("not tiles");
        };
        assert!(tiles.iter().any(|t| t.machine == machine_id(&PC)));
        assert!(tiles.iter().any(|t| t.machine == "somebody"));
    }

    #[test]
    fn the_whole_desk() {
        let id = |fp: &Fingerprint| machine_id(fp);
        let mut file = LayoutFile::default();
        place(&mut file, &ME, &PC, Some(Side::Right));
        place(&mut file, &PC, &TV, Some(Side::Top));
        let desk = desk(&file, |e| e.to_string());
        assert_eq!(
            desk,
            vec![(id(&ME), 0.0, 0.0), (id(&PC), 1.0, 0.0), (id(&TV), 1.0, -1.0)]
        );
        assert_eq!(from_desk(&desk), file.tiles);
    }

    #[test]
    fn laptops_sharing_a_place_stay_together() {
        let file: LayoutFile = serde_json::from_str(
            r#"{"neighbors":[{"machine":"book1","side":"left","of":"imac"},
                             {"machine":"book2","side":"left","of":"imac"}]}"#,
        )
        .unwrap();
        let desk = desk(&file, |e| e.to_string());
        assert_eq!(
            desk,
            vec![
                ("imac".to_string(), 0.0, 0.0),
                ("book1".to_string(), -1.0, 0.0),
                ("book2".to_string(), -1.0, 0.0)
            ]
        );
    }

    #[test]
    fn numbered_like_reading() {
        let tile = |m: &str, x, y| (m.to_string(), x, y);
        let desk = vec![
            tile("me", 0.0, 0.0),
            tile("pc", 1.0, 0.5),
            tile("pi", -1.0, 0.0),
            tile("tv", 0.0, -1.0),
        ];
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

    #[test]
    fn an_old_file_is_read_as_tiles_and_kept() {
        let dir = std::env::temp_dir().join(format!("tablecross-old-layout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = r#"{"neighbors":[{"machine":"laptop","side":"bottom","of":"imac"}],"revision":7}"#;
        std::fs::write(dir.join(LAYOUT_FILE), old).unwrap();
        let file = load(&dir).unwrap();
        assert_eq!(file.tiles.len(), 2);
        assert_eq!(file.revision, 7);
        assert_eq!(std::fs::read_to_string(dir.join(OLD_LAYOUT_FILE)).unwrap(), old);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
