//! Copying files and folders between computers.
//!
//! Copied files are announced like clipboard content. They are transferred when the pointer arrives
//! at a computer that needs them (not merely because something was copied somewhere), into
//! `Downloads/TableCross`, and then put on that computer's clipboard, so pasting in the file manager
//! works as usual. Unfinished transfers live in a hidden part folder that is removed when a transfer
//! is cancelled or fails.
//!
//! On the stream every entry is a length-prefixed header (`Entry`) followed by the file's bytes.
//! Programs keep their permission to run, and symbolic links travel where they point inside what was
//! copied (an app is full of them); links that lead out of it are left behind.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

const HEADER: &[u8; 4] = b"file";
/// Longest entry header on the stream.
const MAX_ENTRY_HEADER: usize = 16 * 1024;
/// Top-level names announced for display.
const SHOWN_NAMES: usize = 8;
/// At most this many top-level names travel in an offer (each becomes an item to paste).
const OFFERED_NAMES: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Dir,
    File,
    /// A file that runs as a program. Only for peers that know it (see `layout_file::knows_links`).
    Executable,
    /// A symbolic link to this target, which stays inside what was copied (see `link_ok`). Only for
    /// peers that know it.
    Link(String),
}

/// One file or folder, with its path relative to the copied top level ('/' separated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub kind: Kind,
    pub size: u64,
}

impl Entry {
    /// Whether bytes follow this entry on the stream.
    pub fn has_content(&self) -> bool {
        matches!(self.kind, Kind::File | Kind::Executable)
    }

    /// The entry as a peer before 0.4.26 understands it: a program as a plain file, no links.
    pub fn plain(&self) -> Option<Entry> {
        match self.kind {
            Kind::Link(_) => None,
            Kind::Executable => Some(Entry {
                kind: Kind::File,
                ..self.clone()
            }),
            _ => Some(self.clone()),
        }
    }
}

/// Whether a link at `link_path` may point to `target`: a relative path with `..` only at its
/// start, rising no higher than the copied top level, then at least one name; never the top level
/// itself a link. Followed on the disk it stays inside, as no entry lies below a link.
pub fn link_ok(link_path: &str, target: &str) -> bool {
    let depth = link_path.split('/').count();
    if depth < 2 {
        return false;
    }
    let (mut ups, mut names) = (0, 0);
    for part in target.split('/') {
        match part {
            ".." if names == 0 => ups += 1,
            "" | "." | ".." => return false,
            part if part.contains(['\\', ':']) => return false,
            _ => names += 1,
        }
    }
    names > 0 && ups <= depth - 2
}

/// What was copied: the entries in order (folders before their content) and where they are.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub entries: Vec<(Entry, PathBuf)>,
    pub roots: Vec<String>,
}

impl Listing {
    pub fn count(&self) -> u64 {
        self.entries.iter().filter(|(e, _)| e.has_content()).count() as u64
    }

    pub fn size(&self) -> u64 {
        self.entries.iter().map(|(e, _)| e.size).sum()
    }

    pub fn shown_names(&self) -> Vec<String> {
        self.roots.iter().take(SHOWN_NAMES).cloned().collect()
    }

    /// The top-level names for an offer: the receiver pastes one item per name.
    pub fn offered_names(&self) -> Vec<String> {
        self.roots.iter().take(OFFERED_NAMES).cloned().collect()
    }
}

/// The first names of an offer, for showing.
pub fn label(names: &[String]) -> String {
    let mut label = names.iter().take(SHOWN_NAMES).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > SHOWN_NAMES {
        label.push_str(", …");
    }
    label
}

/// Where files promised to the clipboard are fetched to when pasted (emptied at start).
pub fn promise_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Caches/TableCross/clipboard");
    }
    std::env::temp_dir().join("tablecross-clipboard")
}

pub fn header(id: u64) -> Vec<u8> {
    let mut h = HEADER.to_vec();
    h.extend_from_slice(&id.to_le_bytes());
    h
}

pub fn parse_header(header: &[u8]) -> Option<u64> {
    let id = header.strip_prefix(HEADER)?;
    Some(u64::from_le_bytes(id.try_into().ok()?))
}

/// Names as Windows and Linux expect them (macOS hands out decomposed umlauts).
fn nfc(name: &str) -> String {
    name.nfc().collect()
}

/// Walks the copied files and folders. Symbolic links come along where they stay inside (`link_ok`).
pub fn list(paths: &[PathBuf]) -> std::io::Result<Listing> {
    let mut entries = Vec::new();
    let mut roots = Vec::new();
    for path in paths {
        let Some(name) = path.file_name().map(|n| nfc(&n.to_string_lossy())) else {
            continue;
        };
        roots.push(name.clone());
        walk(path, name, &mut entries)?;
    }
    Ok(Listing { entries, roots })
}

fn walk(path: &Path, relative: String, out: &mut Vec<(Entry, PathBuf)>) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        if let Some(target) = std::fs::read_link(path).ok().and_then(|t| t.to_str().map(nfc))
            && link_ok(&relative, &target)
        {
            out.push((
                Entry {
                    path: relative,
                    kind: Kind::Link(target),
                    size: 0,
                },
                path.to_path_buf(),
            ));
        }
        return Ok(());
    }
    if meta.is_dir() {
        out.push((
            Entry {
                path: relative.clone(),
                kind: Kind::Dir,
                size: 0,
            },
            path.to_path_buf(),
        ));
        let mut children: Vec<_> = std::fs::read_dir(path)?.filter_map(Result::ok).collect();
        children.sort_by_key(|c| c.file_name());
        for child in children {
            let name = nfc(&child.file_name().to_string_lossy());
            walk(&child.path(), format!("{relative}/{name}"), out)?;
        }
    } else if meta.is_file() {
        #[cfg(unix)]
        let runs = std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 != 0;
        #[cfg(not(unix))]
        let runs = false;
        out.push((
            Entry {
                path: relative,
                kind: if runs { Kind::Executable } else { Kind::File },
                size: meta.len(),
            },
            path.to_path_buf(),
        ));
    }
    Ok(())
}

/// The relative path from the stream as a path below `base`, or `None` if it tries to leave it.
pub fn safe_join(base: &Path, relative: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    let mut any = false;
    for part in relative.split('/') {
        let component = Path::new(part).components().next()?;
        match component {
            Component::Normal(name) if !part.contains(['\\', ':']) && part == name.to_string_lossy() => {
                out.push(name);
                any = true;
            }
            _ => return None,
        }
    }
    any.then_some(out)
}

/// `name`, or `name (2)`, `name (3)` … if it exists in `dir` already.
pub fn unique_name(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("some number is free")
}

/// The Downloads folder, where saved files go unless another one is chosen (`TC_DOWNLOAD_DIR`
/// overrides it, for tests).
pub fn downloads() -> PathBuf {
    if let Some(dir) = std::env::var_os("TC_DOWNLOAD_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join("Downloads")
}

pub fn encode_entry(entry: &Entry) -> Vec<u8> {
    let bytes = postcard::to_allocvec(entry).expect("entries always encode");
    let mut out = (bytes.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&bytes);
    out
}

pub fn decode_entry(bytes: &[u8]) -> Option<Entry> {
    match postcard::take_from_bytes(bytes) {
        Ok((entry, [])) => Some(entry),
        _ => None,
    }
}

pub fn entry_header_ok(len: usize) -> bool {
    len <= MAX_ENTRY_HEADER
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tablecross-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn links_stay_inside() {
        assert!(link_ok(
            "App.app/Contents/Frameworks/X.framework/X",
            "Versions/Current/X"
        ));
        assert!(link_ok("App.app/Contents/Frameworks/X.framework/Versions/Current", "A"));
        assert!(link_ok("Root/a/link", "../b"));
        for (link, target) in [
            ("Root/link", "../outside"),
            ("Root/a/link", "../../outside"),
            ("Root/link", "/etc"),
            ("Root/link", ".."),
            ("Root/link", "a/../b"),
            ("Root/link", "./a"),
            ("Root/link", "a//b"),
            ("Root/link", "C:\\x"),
            ("Root", "a"),
        ] {
            assert!(!link_ok(link, target), "{link} -> {target}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn programs_and_inner_links_come_along() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("app");
        std::fs::create_dir_all(dir.join("Tool/bin")).unwrap();
        std::fs::write(dir.join("Tool/bin/run"), b"#!/bin/sh").unwrap();
        std::fs::set_permissions(dir.join("Tool/bin/run"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("bin/run", dir.join("Tool/start")).unwrap();
        std::os::unix::fs::symlink("/etc", dir.join("Tool/etc")).unwrap();
        let listing = list(&[dir.join("Tool")]).unwrap();
        let kinds: Vec<_> = listing
            .entries
            .iter()
            .map(|(e, _)| (e.path.as_str(), e.kind.clone()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("Tool", Kind::Dir),
                ("Tool/bin", Kind::Dir),
                ("Tool/bin/run", Kind::Executable),
                ("Tool/start", Kind::Link("bin/run".into())),
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn listing_folders_first_without_links() {
        let dir = temp("list");
        std::fs::create_dir_all(dir.join("Fotos/Urlaub")).unwrap();
        std::fs::write(dir.join("Fotos/a.jpg"), b"12345").unwrap();
        std::fs::write(dir.join("Fotos/Urlaub/b.jpg"), b"123").unwrap();
        std::fs::write(dir.join("notes.txt"), b"hi").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", dir.join("Fotos/link")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("Fotos"), dir.join("Fotos-Link")).unwrap();
        let listing = list(&[dir.join("Fotos"), dir.join("notes.txt"), dir.join("Fotos-Link")]).unwrap();
        let paths: Vec<_> = listing.entries.iter().map(|(e, _)| e.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "Fotos",
                "Fotos/Urlaub",
                "Fotos/Urlaub/b.jpg",
                "Fotos/a.jpg",
                "notes.txt"
            ]
        );
        assert_eq!(listing.count(), 3);
        assert_eq!(listing.size(), 10);
        assert_eq!(listing.shown_names()[..2], ["Fotos", "notes.txt"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_cannot_escape() {
        let base = Path::new("/base");
        assert_eq!(safe_join(base, "a/b.txt"), Some(PathBuf::from("/base/a/b.txt")));
        for bad in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            "",
            "a//b",
            "C:\\x",
            "a\\..\\b",
            ".",
            "a/./b",
        ] {
            assert_eq!(safe_join(base, bad), None, "{bad}");
        }
    }

    #[test]
    fn names_are_made_unique() {
        let dir = temp("unique");
        assert_eq!(unique_name(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"").unwrap();
        std::fs::write(dir.join("a (2).txt"), b"").unwrap();
        assert_eq!(unique_name(&dir, "a.txt"), dir.join("a (3).txt"));
        std::fs::create_dir(dir.join("Ordner")).unwrap();
        assert_eq!(unique_name(&dir, "Ordner"), dir.join("Ordner (2)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn umlauts_become_composed() {
        assert_eq!(nfc("Gru\u{308}ße"), "Grüße");
    }

    #[test]
    fn entries_and_headers() {
        let e = Entry {
            path: "a/b".into(),
            kind: Kind::File,
            size: 9,
        };
        let bytes = encode_entry(&e);
        assert_eq!(decode_entry(&bytes[4..]), Some(e));
        assert_eq!(parse_header(&header(5)), Some(5));
        assert_eq!(parse_header(b"clip12345678"), None);
    }
}
