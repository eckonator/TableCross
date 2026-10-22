//! Collecting the logs of the other computers in one place, for troubleshooting.
//!
//! Every computer keeps the end of its own log in memory (`record`, called by the log writer of the
//! app). A computer that collects asks its peers with `Message::LogSubscribe`; they answer with
//! what they have and then send new lines about once a second. The collector writes them to
//! `<settings>/logs/peers/<name>.log`, rotated like its own log.

use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tc_protocol::MAX_LOG_TEXT;

/// What a new subscriber gets from before it asked.
const BACKLOG: usize = 200 * 1024;
/// Kept in memory; a subscriber that fell further behind loses the oldest part.
const KEPT: usize = 1024 * 1024;
const MAX_FILE: u64 = 1024 * 1024;
const KEEP: usize = 2;

/// The end of this computer's log, and how many bytes were written in all (positions count from
/// the start, so every subscriber reads on from its own position).
struct Outbox {
    kept: VecDeque<u8>,
    written: u64,
}

static OUTBOX: Mutex<Outbox> = Mutex::new(Outbox {
    kept: VecDeque::new(),
    written: 0,
});

/// Hands a piece of this computer's log over (called for every write of the log file).
pub fn record(bytes: &[u8]) {
    let Ok(mut outbox) = OUTBOX.lock() else { return };
    outbox.kept.extend(bytes);
    outbox.written += bytes.len() as u64;
    let over = outbox.kept.len().saturating_sub(KEPT);
    outbox.kept.drain(..over);
}

/// The last lines, for a new subscriber, and the position to read on from.
pub fn backlog() -> (Vec<String>, u64) {
    let Ok(outbox) = OUTBOX.lock() else {
        return (Vec::new(), 0);
    };
    let skip = outbox.kept.len().saturating_sub(BACKLOG);
    let bytes: Vec<u8> = outbox.kept.iter().skip(skip).copied().collect();
    // Start at a whole line if older text was cut off
    let cut_off = outbox.written > bytes.len() as u64;
    let start = if cut_off {
        bytes.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1)
    } else {
        0
    };
    (chunks(&String::from_utf8_lossy(&bytes[start..])), outbox.written)
}

/// What was written after `position`; moves `position` to the end.
pub fn since(position: &mut u64) -> Vec<String> {
    let Ok(outbox) = OUTBOX.lock() else {
        return Vec::new();
    };
    let first_kept = outbox.written - outbox.kept.len() as u64;
    let from = (*position).max(first_kept);
    let bytes: Vec<u8> = outbox.kept.iter().skip((from - first_kept) as usize).copied().collect();
    *position = outbox.written;
    drop(outbox);
    chunks(&String::from_utf8_lossy(&bytes))
}

/// Splits text into pieces that fit into one message, at line ends where possible.
fn chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.len() <= MAX_LOG_TEXT {
            out.push(rest.to_string());
            break;
        }
        let mut cut = MAX_LOG_TEXT;
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        if let Some(line_end) = rest[..cut].rfind('\n') {
            cut = line_end + 1;
        }
        out.push(rest[..cut].to_string());
        rest = &rest[cut..];
    }
    out
}

/// macOS: the system helper and the program for the login screen write logs of their own (as
/// root). Their new lines go out with this computer's log, each marked with where it comes from,
/// so a collecting computer sees them too. How far each file was passed on is kept in
/// `<settings>/logs/forwarded.txt`, so lines written while no one was logged in follow later.
pub fn forward_system_logs(settings_dir: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let sources = [
        ("helper", tc_input::helper_protocol::HELPER_LOG),
        ("loginwindow", tc_input::helper_protocol::LOGIN_LOG),
    ];
    let state = settings_dir.join("logs").join("forwarded.txt");
    let _ = std::thread::Builder::new().name("system logs".into()).spawn(move || {
        let mut positions = read_positions(&state);
        loop {
            let before = positions.clone();
            for (label, path) in sources {
                let position = positions.entry(path.to_string()).or_insert(u64::MAX);
                if let Some(text) = new_text(Path::new(path), position) {
                    record(marked(label, &text).as_bytes());
                }
            }
            if positions != before {
                let lines: String = positions.iter().map(|(p, at)| format!("{at} {p}\n")).collect();
                let _ = std::fs::write(&state, lines);
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

/// How far each file was passed on (`<position> <path>` per line).
fn read_positions(state: &Path) -> std::collections::BTreeMap<String, u64> {
    std::fs::read_to_string(state)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (at, path) = line.split_once(' ')?;
            Some((path.to_string(), at.parse().ok()?))
        })
        .collect()
}

/// What `path` holds after `position` (whole lines only), moving `position` on. A file seen for the
/// first time gives its last part; one that got shorter was started anew and is read from the
/// beginning.
fn new_text(path: &Path, position: &mut u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const FIRST_LOOK: u64 = 20 * 1024;
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if *position == u64::MAX {
        *position = len.saturating_sub(FIRST_LOOK);
    } else if len < *position {
        *position = 0;
    }
    if len == *position {
        return None;
    }
    file.seek(SeekFrom::Start(*position)).ok()?;
    let mut bytes = Vec::new();
    file.take(len - *position).read_to_end(&mut bytes).ok()?;
    let end = bytes.iter().rposition(|b| *b == b'\n')? + 1;
    *position += end as u64;
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

/// `text` with every line marked by `label`, without terminal colors.
fn marked(label: &str, text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for line in text.lines() {
        out.push('[');
        out.push_str(label);
        out.push_str("] ");
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Skip a color sequence up to its final letter
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out.push('\n');
    }
    out
}

/// Where the logs of the peers go.
pub fn folder(settings_dir: &Path) -> PathBuf {
    settings_dir.join("logs").join("peers")
}

/// Appends lines from `name` to its file in `folder`, rotating at 1 MB.
pub fn store(folder: &Path, name: &str, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = if safe.is_empty() { "unknown".to_string() } else { safe };
    let path = folder.join(format!("{safe}.log"));
    if std::fs::metadata(&path).is_ok_and(|m| m.len() + text.len() as u64 > MAX_FILE) {
        for i in (1..KEEP).rev() {
            let _ = std::fs::rename(
                folder.join(format!("{safe}.log.{i}")),
                folder.join(format!("{safe}.log.{}", i + 1)),
            );
        }
        std::fs::rename(&path, folder.join(format!("{safe}.log.1")))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_logs_are_read_on_in_whole_lines_and_marked() {
        let dir = std::env::temp_dir().join(format!("tc-system-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent.log");
        std::fs::write(&path, "old line\n\x1b[2mtime\x1b[0m warn: one\nhalf").unwrap();
        let mut position = 0;
        let text = new_text(&path, &mut position).unwrap();
        assert_eq!(
            marked("loginwindow", &text),
            "[loginwindow] old line\n[loginwindow] time warn: one\n"
        );
        // The unfinished line follows once it is complete
        assert_eq!(new_text(&path, &mut position), None);
        std::fs::write(&path, "old line\n\x1b[2mtime\x1b[0m warn: one\nhalf done\n").unwrap();
        assert_eq!(new_text(&path, &mut position).as_deref(), Some("half done\n"));
        // Started anew: from the beginning
        std::fs::write(&path, "new\n").unwrap();
        assert_eq!(new_text(&path, &mut position).as_deref(), Some("new\n"));
        // Seen for the first time: only the last part
        let mut first = u64::MAX;
        assert_eq!(new_text(&path, &mut first).as_deref(), Some("new\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn long_text_is_cut_at_line_ends() {
        let line = "x".repeat(99) + "\n";
        let text = line.repeat(1000);
        let parts = chunks(&text);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.len() <= MAX_LOG_TEXT && p.ends_with('\n')));
        assert_eq!(parts.concat(), text);
        // Without line ends, still at character boundaries
        let wide = "ä".repeat(MAX_LOG_TEXT);
        assert!(chunks(&wide).iter().all(|p| p.len() <= MAX_LOG_TEXT));
        assert_eq!(chunks(&wide).concat(), wide);
        assert!(chunks("").is_empty());
    }

    #[test]
    fn peer_logs_are_stored_and_rotated() {
        let dir = std::env::temp_dir().join(format!("tc-peer-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        store(&dir, "iMac-4", "first\n").unwrap();
        store(&dir, "iMac-4", "second\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("iMac-4.log")).unwrap(),
            "first\nsecond\n"
        );
        let big = "y".repeat(MAX_FILE as usize);
        store(&dir, "iMac-4", &big).unwrap();
        assert!(dir.join("iMac-4.log.1").exists());
        store(&dir, "../evil name", "z\n").unwrap();
        assert!(dir.join("___evil_name.log").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
