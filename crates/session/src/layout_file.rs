//! `layout.json` in the settings folder: where the paired computers stand.
//!
//! Machines are stored by the first 16 hex digits of their fingerprint, so renaming a computer does
//! not break the layout; names and fingerprint prefixes are accepted when reading.

use std::path::Path;

use anyhow::{Context, Result};
use tc_core::{LayoutFile, Neighbor, Side};
use tc_net::Fingerprint;

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
