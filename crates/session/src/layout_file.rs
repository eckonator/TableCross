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
//! Since 0.4.2 the arrangement is areas of real size that may stand offset; older versions get
//! whole-edge neighbors derived from them, and what they send (tiles of 0.4.0, neighbors before) is
//! read as areas of a stand-in size on a grid.

use std::path::Path;

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tc_core::{Area, LayoutFile, Neighbor, STAND_IN, Side, Tile, area_side, areas_from_tiles, tiles_from_neighbors};
use tc_net::Fingerprint;
use tc_protocol::{MAX_PLACEMENTS, Message};

pub const LAYOUT_FILE: &str = "layout.json";
/// Copy of a layout from before areas, made once when it is first read as areas.
pub const OLD_LAYOUT_FILE: &str = "layout-before-0.4.2.json";

/// First version that knows `Message::Desk` and `Message::ShowEdges`.
const DESK_SINCE: (u32, u32, u32) = (0, 4, 2);

pub fn load(dir: &Path) -> Result<LayoutFile> {
    let path = dir.join(LAYOUT_FILE);
    let mut file: LayoutFile = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("{} is not valid", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LayoutFile::default()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    if file.areas.is_empty() && (!file.tiles.is_empty() || !file.neighbors.is_empty()) {
        let old = dir.join(OLD_LAYOUT_FILE);
        if !old.exists() {
            let _ = std::fs::copy(&path, &old);
        }
        let areas = file.effective_areas();
        file.set_areas(areas);
    }
    Ok(file)
}

/// First version that knows `Message::Screens`.
const SCREENS_SINCE: (u32, u32, u32) = (0, 4, 7);

/// First version that knows `Message::ScreenSizes`.
const SIZES_SINCE: (u32, u32, u32) = (0, 4, 10);

fn version(app_version: &str) -> (u32, u32, u32) {
    let mut parts = app_version.split(['.', '-']).map(|p| p.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Whether a peer running `app_version` understands `Message::Desk` and `Message::ShowEdges`.
pub fn knows_desk(app_version: &str) -> bool {
    version(app_version) >= DESK_SINCE
}

/// Whether a peer running `app_version` understands `Message::Screens`.
pub fn knows_screens(app_version: &str) -> bool {
    version(app_version) >= SCREENS_SINCE
}

/// Whether a peer running `app_version` understands `Message::ScreenSizes`.
pub fn knows_sizes(app_version: &str) -> bool {
    version(app_version) >= SIZES_SINCE
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

/// Puts `peer` on `side` of `me`, along the whole edge from its start (`None` removes it). Whoever
/// stood exactly there before moves away.
pub fn place(file: &mut LayoutFile, me: &Fingerprint, peer: &Fingerprint, side: Option<Side>) {
    let mut areas = file.effective_areas();
    let (w, h) = areas
        .iter()
        .find(|a| names(&a.machine, peer, ""))
        .map_or(STAND_IN, |a| (a.w, a.h));
    areas.retain(|a| !names(&a.machine, peer, ""));
    if let Some(side) = side {
        let mine = match areas.iter().find(|a| names(&a.machine, me, "")) {
            Some(a) => a.clone(),
            None => {
                let a = Area {
                    machine: machine_id(me),
                    x: 0.0,
                    y: 0.0,
                    w: STAND_IN.0,
                    h: STAND_IN.1,
                    measured: false,
                };
                areas.push(a.clone());
                a
            }
        };
        let (x, y) = match side {
            Side::Left => (mine.x - w, mine.y),
            Side::Right => (mine.x + mine.w, mine.y),
            Side::Top => (mine.x, mine.y - h),
            Side::Bottom => (mine.x, mine.y + mine.h),
        };
        areas.retain(|a| (a.x - x).abs() > 1e-3 || (a.y - y).abs() > 1e-3);
        areas.push(Area {
            machine: machine_id(peer),
            x,
            y,
            w,
            h,
            measured: false,
        });
    }
    file.set_areas(areas);
}

/// Where `peer` stands next to `me`, if their areas touch.
pub fn side_of(
    file: &LayoutFile,
    me: &Fingerprint,
    my_name: &str,
    peer: &Fingerprint,
    peer_name: &str,
) -> Option<Side> {
    let areas = file.effective_areas();
    let mine = areas.iter().find(|a| names(&a.machine, me, my_name))?;
    let theirs = areas.iter().find(|a| names(&a.machine, peer, peer_name))?;
    area_side(mine, theirs)
}

/// Marks the arrangement as changed here, so other computers take it over.
pub fn bump(file: &mut LayoutFile, me: &Fingerprint) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    file.revision = now.max(file.revision + 1);
    file.editor = machine_id(me);
}

/// The arrangement for other computers: `Message::Desk` for those that know it, else the old
/// `Message::Layout`. `resolve` turns a name from the file into a fingerprint (`None`: unknown,
/// sent as written); `known` gives display names of fingerprints.
pub fn message(
    file: &LayoutFile,
    desk: bool,
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
    let areas = file.effective_areas();
    if desk {
        let areas = areas
            .into_iter()
            .take(MAX_PLACEMENTS)
            .map(|a| Area {
                machine: id(&a.machine),
                ..a
            })
            .collect();
        return Message::Desk {
            revision: file.revision,
            editor: file.editor.clone(),
            areas,
            names: names.into_iter().take(MAX_PLACEMENTS).collect(),
        };
    }
    let placements = tc_core::neighbors_from_areas(&areas)
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

/// An arrangement as another computer sends it, by its version.
pub enum Arrangement {
    Areas(Vec<Area>),
    Tiles(Vec<Tile>),
    Neighbors(Vec<Neighbor>),
}

/// Takes over an arrangement from another computer if it is newer than ours.
pub fn adopt(
    file: &mut LayoutFile,
    revision: u64,
    editor: String,
    arrangement: Arrangement,
    names: Vec<(String, String)>,
) -> bool {
    if (revision, editor.as_str()) <= (file.revision, file.editor.as_str()) {
        return false;
    }
    file.revision = revision;
    file.editor = editor;
    let areas = match arrangement {
        Arrangement::Areas(areas) => areas,
        Arrangement::Tiles(tiles) => areas_from_tiles(&tiles),
        Arrangement::Neighbors(neighbors) => areas_from_tiles(&tiles_from_neighbors(&neighbors)),
    };
    file.set_areas(areas);
    file.names = names.into_iter().collect();
    true
}

/// Where each machine stands on the desk, one area per machine: `canonical` maps an entry of the
/// file to one id per machine. Machines used in turn at the same place share a position.
pub fn desk(file: &LayoutFile, canonical: impl Fn(&str) -> String) -> Vec<Area> {
    let mut placed: Vec<Area> = Vec::new();
    for a in file.effective_areas() {
        let id = canonical(&a.machine);
        if !placed.iter().any(|p| p.machine == id) {
            placed.push(Area { machine: id, ..a });
        }
    }
    placed
}

/// The machines of a desk in reading order (row by row, left to right): number 1 is the first.
/// Ctrl+Shift+Alt+number goes to that computer.
pub fn reading_order(areas: &[Area]) -> Vec<String> {
    let mut sorted: Vec<&Area> = areas.iter().collect();
    sorted.sort_by(|a, b| {
        let row = |a: &Area| a.y + a.h / 2.0;
        let col = |a: &Area| a.x + a.w / 2.0;
        (row(a) / 0.5)
            .round()
            .total_cmp(&(row(b) / 0.5).round())
            .then(col(a).total_cmp(&col(b)))
    });
    sorted.into_iter().map(|a| a.machine.clone()).collect()
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
            } => adopt(file, revision, editor, Arrangement::Neighbors(placements), names),
            Message::Tiles {
                revision,
                editor,
                tiles,
                names,
            } => adopt(file, revision, editor, Arrangement::Tiles(tiles), names),
            Message::Desk {
                revision,
                editor,
                areas,
                names,
            } => adopt(file, revision, editor, Arrangement::Areas(areas), names),
            _ => panic!("not a layout"),
        }
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
    fn newer_arrangements_are_taken_over() {
        for desk in [true, false] {
            let mut mine = LayoutFile::default();
            place(&mut mine, &ME, &PC, Some(Side::Right));
            bump(&mut mine, &ME);
            let mut theirs = LayoutFile::default();
            // Ours is newer: the other computer takes it, we keep ours
            assert!(adopt_message(&mut theirs, message(&mine, desk, resolve, known)));
            assert_eq!(theirs.neighbors, mine.neighbors);
            assert_eq!(theirs.names.get(&machine_id(&PC)).map(String::as_str), Some("PC"));
            assert!(!adopt_message(&mut mine, message(&theirs, desk, resolve, known)));
            // A change over there wins even with a clock far behind
            place(&mut theirs, &PC, &TV, Some(Side::Top));
            theirs.revision = mine.revision;
            bump(&mut theirs, &PC);
            assert!(theirs.revision > mine.revision);
            assert!(adopt_message(&mut mine, message(&theirs, desk, resolve, known)));
            assert_eq!(side_of(&mine, &PC, "", &TV, ""), Some(Side::Top));
            assert_eq!(side_of(&mine, &ME, "", &PC, ""), Some(Side::Right));
        }
    }

    #[test]
    fn real_sizes_travel_only_to_computers_that_know_them() {
        let mut mine = LayoutFile::default();
        mine.set_areas(vec![
            area(&machine_id(&ME), 0.0, 0.0, 5.12, 1.44),
            area(&machine_id(&PC), 1.0, 1.44, 1.728, 1.117),
        ]);
        bump(&mut mine, &ME);
        let mut new = LayoutFile::default();
        assert!(adopt_message(&mut new, message(&mine, true, resolve, known)));
        assert_eq!(new.areas, mine.areas);
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
        // Tiles from a computer with 0.4.0 become areas of the stand-in size
        let tiles = Message::Tiles {
            revision: u64::MAX,
            editor: "z".into(),
            tiles: vec![Tile {
                machine: "a".into(),
                x: 1.0,
                y: 0.0,
            }],
            names: Vec::new(),
        };
        assert!(adopt_message(&mut new, tiles));
        assert_eq!(
            new.areas,
            [Area {
                measured: false,
                ..area("a", STAND_IN.0, 0.0, STAND_IN.0, STAND_IN.1)
            }]
        );
        assert!(knows_desk("0.4.2"));
        assert!(knows_desk("1.0.0"));
        assert!(!knows_desk("0.4.1"));
        assert!(!knows_desk("0.3.10"));
        assert!(!knows_desk(""));
        assert!(knows_screens("0.4.7") && !knows_screens("0.4.6"));
        assert!(knows_sizes("0.4.10") && !knows_sizes("0.4.9"));
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
        let Message::Desk { areas, .. } = message(&file, true, resolve, known) else {
            panic!("not a desk");
        };
        assert!(areas.iter().any(|a| a.machine == machine_id(&PC)));
        assert!(areas.iter().any(|a| a.machine == "somebody"));
    }

    #[test]
    fn the_whole_desk() {
        let id = |fp: &Fingerprint| machine_id(fp);
        let mut file = LayoutFile::default();
        place(&mut file, &ME, &PC, Some(Side::Right));
        place(&mut file, &PC, &TV, Some(Side::Top));
        let (w, h) = STAND_IN;
        let desk = desk(&file, |e| e.to_string());
        let at: Vec<_> = desk.iter().map(|a| (a.machine.clone(), a.x, a.y)).collect();
        assert_eq!(at, vec![(id(&ME), 0.0, 0.0), (id(&PC), w, 0.0), (id(&TV), w, -h)]);
    }

    #[test]
    fn laptops_sharing_a_place_stay_together() {
        let file: LayoutFile = serde_json::from_str(
            r#"{"neighbors":[{"machine":"book1","side":"left","of":"imac"},
                             {"machine":"book2","side":"left","of":"imac"}]}"#,
        )
        .unwrap();
        let desk = desk(&file, |e| e.to_string());
        let at: Vec<_> = desk.iter().map(|a| (a.machine.as_str(), a.x, a.y)).collect();
        assert_eq!(
            at,
            vec![
                ("imac", 0.0, 0.0),
                ("book1", -STAND_IN.0, 0.0),
                ("book2", -STAND_IN.0, 0.0)
            ]
        );
    }

    #[test]
    fn numbered_like_reading() {
        let desk = vec![
            area("me", 0.0, 0.0, 1.0, 1.0),
            area("pc", 1.0, 0.1, 1.0, 1.0),
            area("pi", -1.0, 0.0, 1.0, 1.0),
            area("tv", 0.0, -1.0, 1.0, 1.0),
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
    fn an_old_file_is_read_as_areas_and_kept() {
        let dir = std::env::temp_dir().join(format!("tablecross-old-layout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = r#"{"tiles":[{"machine":"imac","x":0,"y":0},{"machine":"laptop","x":0.5,"y":1}],"revision":7}"#;
        std::fs::write(dir.join(LAYOUT_FILE), old).unwrap();
        let file = load(&dir).unwrap();
        assert_eq!(file.areas.len(), 2);
        assert!(file.tiles.is_empty());
        assert_eq!(file.revision, 7);
        assert_eq!(std::fs::read_to_string(dir.join(OLD_LAYOUT_FILE)).unwrap(), old);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
