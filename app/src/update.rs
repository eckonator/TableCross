//! Looking for a newer TableCross on GitHub, only if the user agreed.
//!
//! The only thing sent is the request for the newest release of the project. It goes through `curl`
//! (present on macOS, Windows 10+ and Linux), so the app needs no web client of its own. Errors
//! (offline, no release) stay silent.
//!
//! Every push to main rebuilds the test build without changing the version number. So for test
//! builds a release is also new if it was built from another, younger commit than this program.

use std::process::Command;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;

/// Releases look only for releases; test builds also for newer test builds (the newest of both).
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/eckonator/TableCross/releases/latest";
/// GitHub does not list releases by date (the test build often comes last), so the newest is picked
/// from the first page by its publishing time.
const NEWEST_URL: &str = "https://api.github.com/repos/eckonator/TableCross/releases?per_page=20";

/// Built from a release tag (not a test build from main).
pub fn is_release() -> bool {
    env!("TC_CHANNEL") == "release"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVersion {
    pub version: String,
    /// Release page to download it from.
    pub url: String,
    /// The file for this system, if the app can install it itself.
    pub asset: Option<String>,
}

/// Starts the check in the background; the result comes through the receiver (`None`: nothing new).
pub fn check_in_background(ctx: egui::Context) -> Receiver<Option<NewVersion>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("tablecross-update".into())
        .spawn(move || {
            let result = check();
            if let Some(v) = &result {
                tracing::info!("newer version available: {} ({})", v.version, v.url);
            }
            let _ = tx.send(result);
            ctx.request_repaint();
        })
        .ok();
    rx
}

fn check() -> Option<NewVersion> {
    // TC_UPDATE_URL (debug builds only): another source, e.g. a local JSON file for testing
    let url = std::env::var("TC_UPDATE_URL")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .unwrap_or_else(|| if is_release() { LATEST_RELEASE_URL } else { NEWEST_URL }.into());
    let mut cmd = Command::new("curl");
    cmd.args([
        "-fsSL",
        "--max-time",
        "8",
        "-H",
        "Accept: application/vnd.github+json",
        "-A",
        "TableCross-update-check",
    ])
    .arg(&url);
    #[cfg(windows)]
    {
        // No console window flashing up
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let release = json.as_array().and_then(|a| newest(a)).unwrap_or(&json);
    newer_release(
        release,
        env!("CARGO_PKG_VERSION"),
        // A release is new only with a higher version
        if is_release() { "" } else { env!("TC_GIT_HASH") },
        env!("TC_GIT_TIME"),
        crate::self_update::asset_suffix(),
    )
}

/// The release published last ("2026-…" times sort as text).
fn newest(releases: &[serde_json::Value]) -> Option<&serde_json::Value> {
    releases
        .iter()
        .max_by_key(|r| r.get("published_at").and_then(|p| p.as_str()).unwrap_or_default())
}

/// Compares a release (GitHub API object) with this build.
fn newer_release(
    release: &serde_json::Value,
    version: &str,
    commit: &str,
    commit_time: &str,
    suffix: Option<&str>,
) -> Option<NewVersion> {
    let tag = release.get("tag_name")?.as_str()?;
    let page = release.get("html_url")?.as_str()?.to_string();
    let latest = tag.trim_start_matches('v');
    let asset = suffix.and_then(|suffix| {
        release
            .get("assets")?
            .as_array()?
            .iter()
            .filter_map(|a| a.get("browser_download_url")?.as_str())
            .find(|u| u.ends_with(suffix))
            .map(str::to_string)
    });
    let found = NewVersion {
        version: latest.to_string(),
        url: page,
        asset,
    };
    if newer(latest, version) {
        return Some(found);
    }
    if latest != version || commit.is_empty() {
        return None;
    }
    // Same version: newer if built from another commit published after ours
    let release_commit = release.get("target_commitish")?.as_str()?;
    let published = release.get("published_at")?.as_str().and_then(unix_time)?;
    let ours: i64 = commit_time.parse().ok()?;
    (!release_commit.starts_with(commit) && published > ours).then_some(found)
}

/// Is `a` newer than `b`? Compares "1.2.3" part by part as numbers.
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split(['.', '-', '+'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parse(a) > parse(b)
}

/// "2026-10-17T10:52:11Z" as seconds since 1970.
fn unix_time(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    // Days since 1970 for a date in the Gregorian calendar
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("0.10.0", "0.9.0"));
        assert!(!newer("0.1.2", "0.1.2"));
        assert!(!newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn reads_github_times() {
        assert_eq!(unix_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_time("2026-10-17T10:52:11Z"), Some(1_792_234_331));
        assert_eq!(unix_time("nonsense"), None);
    }

    #[test]
    fn same_version_counts_when_built_later_from_another_commit() {
        let release = serde_json::json!({
            "tag_name": "v0.0.1",
            "html_url": "https://example/releases/v0.0.1",
            "target_commitish": "abcdef1234",
            "published_at": "2026-10-17T12:00:00Z",
        });
        let noon = unix_time("2026-10-17T12:00:00Z").unwrap();
        let earlier = (noon - 3600).to_string();
        let later = (noon + 3600).to_string();
        assert!(newer_release(&release, "0.0.1", "1111111", &earlier, None).is_some());
        // Our own build, or a local build from a younger commit: nothing new
        assert!(newer_release(&release, "0.0.1", "abcdef1", &earlier, None).is_none());
        assert!(newer_release(&release, "0.0.1", "1111111", &later, None).is_none());
        // Without build information only the version counts
        assert!(newer_release(&release, "0.0.1", "", "", None).is_none());
        assert!(newer_release(&release, "0.0.0", "", "", None).is_some());
        assert!(newer_release(&release, "0.0.2", "1111111", &earlier, None).is_none());
    }

    #[test]
    fn the_newest_is_found_wherever_it_is_listed() {
        let list = serde_json::json!([
            {"tag_name": "v0.1.2", "published_at": "2026-10-17T16:25:09Z"},
            {"tag_name": "v0.1.1", "published_at": "2026-10-17T16:11:12Z"},
            {"tag_name": "testing", "published_at": "2026-10-17T17:44:53Z"},
        ]);
        let found = newest(list.as_array().unwrap()).unwrap();
        assert_eq!(found["tag_name"], "testing");
    }

    #[test]
    fn picks_the_file_for_this_system() {
        let release = serde_json::json!({
            "tag_name": "v0.2.0",
            "html_url": "https://example/releases/v0.2.0",
            "assets": [
                {"browser_download_url": "https://example/TableCross-0.2.0-macOS.dmg"},
                {"browser_download_url": "https://example/TableCross-0.2.0-macOS.zip"},
                {"browser_download_url": "https://example/TableCross-0.2.0-Windows-Setup.exe"},
            ],
        });
        let found = newer_release(&release, "0.1.0", "", "", Some("-macOS.zip")).unwrap();
        assert_eq!(
            found.asset.as_deref(),
            Some("https://example/TableCross-0.2.0-macOS.zip")
        );
        let found = newer_release(&release, "0.1.0", "", "", Some("-Linux.tar.gz")).unwrap();
        assert_eq!(found.asset, None);
    }
}
