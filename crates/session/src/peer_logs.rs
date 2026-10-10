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
