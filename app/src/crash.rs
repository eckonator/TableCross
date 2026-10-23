//! Crashes end up in the log: a panic is logged before the program ends, and on macOS a crash
//! report of an earlier run is summed up at the next start (with what was logged in the minute
//! before), so a computer collecting this log sees what happened even if it was off at the time.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Seconds of log before a crash that are logged with its report.
const BEFORE: i64 = 60;
/// Frames of the crashed thread that are logged.
const FRAMES: usize = 30;

/// Logs panics (with a backtrace) before the default handler runs.
pub fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            "panic in thread {}: {info}\n{backtrace}",
            thread.name().unwrap_or("unnamed")
        );
        default(info);
    }));
}

/// Looks for crash reports of this program newer than the last one seen (kept in `seen_file`)
/// and logs them, each with the lines of `log_file` (and its older part) from the minute before.
/// Looks again a little later: the report is written some seconds after the crash.
pub fn report_earlier_crashes(seen_file: PathBuf, log_file: PathBuf) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(dir) = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Logs/DiagnosticReports")) else {
        return;
    };
    let _ = std::thread::Builder::new().name("crash reports".into()).spawn(move || {
        for wait in [5, 60] {
            std::thread::sleep(Duration::from_secs(wait));
            let seen = std::fs::read_to_string(&seen_file).unwrap_or_default();
            let mut new = reports(&dir)
                .into_iter()
                .filter(|name| name.as_str() > seen.trim())
                .collect::<Vec<_>>();
            if new.is_empty() {
                continue;
            }
            new.sort();
            for name in &new {
                match std::fs::read_to_string(dir.join(name)) {
                    Ok(text) => {
                        tracing::error!("an earlier run crashed ({name}):\n{}", summary(&text));
                        if let Some(at) = crashed_at(&text) {
                            let lines = logged_before(&log_file, at);
                            if !lines.is_empty() {
                                tracing::info!("logged in the minute before that crash:\n{}", lines.join("\n"));
                            }
                        }
                    }
                    Err(e) => tracing::warn!("cannot read the crash report {name}: {e}"),
                }
            }
            if let Some(newest) = new.last() {
                let _ = std::fs::write(&seen_file, newest);
            }
        }
    });
}

/// Names of this program's crash reports (`tablecross-<date>-<time>.ips`, they sort by time).
fn reports(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.to_ascii_lowercase().starts_with("tablecross-") && name.ends_with(".ips"))
        .collect()
}

/// The parts of a crash report that tell what happened: exception, termination reason, messages
/// of the system libraries and the crashed thread's frames (offsets, for symbolicating).
fn summary(report: &str) -> String {
    // A line of JSON about the report, then the report itself as JSON
    let Some((_, body)) = report.split_once('\n') else {
        return report.chars().take(4000).collect();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return body.chars().take(4000).collect();
    };
    let mut out = Vec::new();
    for key in ["osVersion", "exception", "termination", "asi"] {
        if let Some(value) = json.get(key) {
            out.push(format!("{key}: {value}"));
        }
    }
    let images = json.get("usedImages").and_then(|v| v.as_array());
    let faulting = json.get("faultingThread").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let thread = json.get("threads").and_then(|t| t.get(faulting));
    if let Some(name) = thread.and_then(|t| t.get("name").or_else(|| t.get("queue"))) {
        out.push(format!("crashed thread {faulting}: {name}"));
    }
    let frames = thread.and_then(|t| t.get("frames")).and_then(|f| f.as_array());
    for frame in frames.into_iter().flatten().take(FRAMES) {
        let image = frame
            .get("imageIndex")
            .and_then(|i| i.as_u64())
            .and_then(|i| images?.get(i as usize))
            .and_then(|i| i.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("?");
        let offset = frame.get("imageOffset").and_then(|o| o.as_u64()).unwrap_or(0);
        let symbol = frame.get("symbol").and_then(|s| s.as_str()).unwrap_or("");
        out.push(format!("  {image} +0x{offset:x} {symbol}"));
    }
    if let Some(main) = images.and_then(|i| i.first()) {
        out.push(format!("main image: {main}"));
    }
    out.join("\n")
}

/// When the crash happened (seconds since 1970), from the report's first line
/// (`"timestamp":"2026-10-23 13:23:41.00 +0200"`).
fn crashed_at(report: &str) -> Option<i64> {
    let header: serde_json::Value = serde_json::from_str(report.lines().next()?).ok()?;
    let stamp = header.get("timestamp")?.as_str()?;
    let (local, zone) = stamp.rsplit_once(' ')?;
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let zone = zone.trim_start_matches(['+', '-']);
    let offset = sign * (zone.get(..2)?.parse::<i64>().ok()? * 3600 + zone.get(2..4)?.parse::<i64>().ok()? * 60);
    Some(seconds(local)? - offset)
}

/// Seconds since 1970 of `YYYY-MM-DD?HH:MM:SS` at the start of `text` (any separator, no zone).
fn seconds(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    // Days from the civil date (proleptic Gregorian)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Lines logged (in UTC) from `BEFORE` seconds before `at` up to just after it, from the log and
/// its older part; lines without a time of their own go with the line before.
fn logged_before(log_file: &Path, at: i64) -> Vec<String> {
    let older = log_file.with_extension("log.1");
    let mut out = Vec::new();
    for path in [older.as_path(), log_file] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut inside = false;
        for line in text.lines() {
            if let Some(time) = seconds(line).filter(|_| line.as_bytes().get(10) == Some(&b'T')) {
                inside = (at - BEFORE..=at + 1).contains(&time);
            }
            if inside {
                out.push(line.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_up_a_report() {
        let report = r#"{"app_name":"tablecross"}
{"osVersion":{"train":"macOS 15.8"},"exception":{"type":"EXC_BAD_ACCESS","signal":"SIGSEGV"},
"faultingThread":1,"threads":[{"frames":[]},{"queue":"com.apple.main-thread","frames":[
{"imageOffset":4660,"symbol":"objc_msgSend","imageIndex":1},{"imageOffset":255,"imageIndex":0}]}],
"usedImages":[{"name":"tablecross","uuid":"abc"},{"name":"libobjc.A.dylib"}]}"#;
        let text = summary(report);
        assert!(text.contains("EXC_BAD_ACCESS"), "{text}");
        assert!(text.contains("crashed thread 1: \"com.apple.main-thread\""), "{text}");
        assert!(text.contains("libobjc.A.dylib +0x1234 objc_msgSend"), "{text}");
        assert!(text.contains("tablecross +0xff"), "{text}");
    }

    #[test]
    fn finds_the_lines_before_a_crash() {
        let at = crashed_at("{\"timestamp\":\"2026-10-23 13:23:41.00 +0200\"}\n{}").unwrap();
        assert_eq!(at, seconds("2026-10-23T11:23:41").unwrap());
        assert_eq!(seconds("1970-01-02T00:00:01"), Some(86_401));
        let dir = std::env::temp_dir().join(format!("tablecross-crash-lines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("tablecross.log");
        std::fs::write(dir.join("tablecross.log.1"), "2026-10-23T11:21:00.1Z INFO too early\n").unwrap();
        std::fs::write(
            &log,
            "2026-10-23T11:22:30.0Z INFO too early\n2026-10-23T11:23:39.8Z INFO drag starts\n[helper] 1 note\n\
             2026-10-23T11:24:15.0Z INFO starting\n[helper] 2 later\n",
        )
        .unwrap();
        assert_eq!(
            logged_before(&log, at),
            vec!["2026-10-23T11:23:39.8Z INFO drag starts", "[helper] 1 note"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_only_own_reports() {
        let dir = std::env::temp_dir().join(format!("tablecross-crash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "tablecross-2026-10-23-132342.ips",
            "node-2026-10-22-160207.ips",
            "tablecross.log",
        ] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        assert_eq!(reports(&dir), vec!["tablecross-2026-10-23-132342.ips".to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
