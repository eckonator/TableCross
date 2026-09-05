//! Installing the macOS system helper as a LaunchDaemon (administrator password via the system
//! dialog). Later the app installs it the same way, or via `SMAppService` once it is signed.

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tc_input::helper_protocol::{FromHelper, HELPER_VERSION, LABEL, ToHelper, read_frame, socket_path, write_frame};

const INSTALLED_BINARY: &str = "/Library/PrivilegedHelperTools/de.eckonator.tablecross.helper";
const PLIST: &str = "/Library/LaunchDaemons/de.eckonator.tablecross.helper.plist";
const LOG: &str = "/Library/Logs/TableCross/helper.log";

unsafe extern "C" {
    fn getuid() -> u32;
}

/// The helper binary next to this program.
fn built_helper() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let path = exe.with_file_name("tablecross-helper");
    if !path.exists() {
        bail!(
            "{} not found; build it first (cargo build -p tablecross-helper)",
            path.display()
        );
    }
    Ok(path)
}

fn plist(uid: u32) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{INSTALLED_BINARY}</string>
        <string>--user</string>
        <string>{uid}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>{LOG}</string>
</dict>
</plist>
"#
    )
}

/// Single quotes for the shell.
fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

/// Runs a shell script as root after the system's password dialog.
fn as_admin(script: &str) -> Result<()> {
    let apple_script = format!(
        "do shell script \"{}\" with administrator privileges",
        script.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let status = std::process::Command::new("osascript")
        .args(["-e", &apple_script])
        .status()
        .context("cannot run osascript")?;
    if !status.success() {
        bail!("cancelled or failed");
    }
    Ok(())
}

pub fn install() -> Result<()> {
    let source = built_helper()?;
    let plist_file = std::env::temp_dir().join("de.eckonator.tablecross.helper.plist");
    std::fs::write(&plist_file, plist(unsafe { getuid() }))?;
    let script = format!(
        "set -e
mkdir -p /Library/PrivilegedHelperTools /Library/Logs/TableCross
launchctl bootout system/{LABEL} 2>/dev/null || true
cp {source} {INSTALLED_BINARY}
chown root:wheel {INSTALLED_BINARY}
chmod 755 {INSTALLED_BINARY}
cp {plist} {PLIST}
chown root:wheel {PLIST}
chmod 644 {PLIST}
launchctl bootstrap system {PLIST}",
        source = quote(&source),
        plist = quote(&plist_file),
    );
    as_admin(&script)?;
    let _ = std::fs::remove_file(&plist_file);
    // The place to grant it, and the file in case it has to be added with "+"
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
        .status();
    let _ = std::process::Command::new("open")
        .args(["-R", INSTALLED_BINARY])
        .status();
    println!("Installed. The helper now asks for the Input Monitoring permission:");
    println!("System Settings → Privacy & Security → Input Monitoring → enable “de.eckonator.tablecross.helper”.");
    println!("It picks the permission up by itself within half a minute; check with `tablecross-cli helper status`.");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    as_admin(&format!(
        "launchctl bootout system/{LABEL} 2>/dev/null || true
rm -f {PLIST} {INSTALLED_BINARY}"
    ))?;
    println!("Removed.");
    Ok(())
}

pub fn restart() -> Result<()> {
    as_admin(&format!("launchctl kickstart -k system/{LABEL}"))?;
    println!("Restarted.");
    Ok(())
}

pub fn status() -> Result<()> {
    let path = socket_path();
    let Ok(mut stream) = UnixStream::connect(&path) else {
        println!("Not running (no {path}). Install with `tablecross-cli helper install`.");
        return Ok(());
    };
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    write_frame(
        &mut stream,
        &ToHelper::Hello {
            version: HELPER_VERSION,
        },
    )?;
    match read_frame::<FromHelper>(&mut reader) {
        Ok(FromHelper::Hello { version, access }) => {
            println!(
                "Running, version {version}{}.",
                if version == HELPER_VERSION {
                    ""
                } else {
                    " (outdated, install again)"
                }
            );
            if access {
                println!("Input Monitoring: granted.");
            } else {
                println!("Input Monitoring: missing. System Settings → Privacy & Security → Input Monitoring.");
            }
        }
        Ok(other) => println!("Unexpected answer: {other:?}"),
        Err(e) => println!("Running, but it did not answer ({e}); is it this user's helper?"),
    }
    println!("Log: {LOG}");
    Ok(())
}
