//! Installing the macOS system helper as a LaunchDaemon (administrator password via the system
//! dialog), used by the app and the command line tool. `SMAppService` needs a signed app; until then
//! this is the way.

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tc_input::helper_protocol::{
    FromHelper, HELPER_VERSION, LABEL, LOGIN_BINARY, LOGIN_DIR, LOGIN_PLIST, ToHelper, read_frame, socket_path,
    write_frame,
};

pub const INSTALLED_BINARY: &str = "/Library/PrivilegedHelperTools/de.eckonator.tablecross.helper";
const PLIST: &str = "/Library/LaunchDaemons/de.eckonator.tablecross.helper.plist";
const LOG: &str = tc_input::helper_protocol::HELPER_LOG;

unsafe extern "C" {
    fn getuid() -> u32;
}

/// Where to grant the helper its permission.
pub const INPUT_MONITORING_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";
/// Where to grant the program for the login screen its permission.
pub const ACCESSIBILITY_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

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

/// The command line tool next to `helper`, which also runs on the login screen.
fn login_program(helper: &Path) -> Option<PathBuf> {
    Some(helper.with_file_name("tablecross-cli")).filter(|p| p.exists())
}

/// Shell lines that put the program `from` at `to`, owned by root. A signed program must not be
/// overwritten in place: a running copy keeps the file mapped, and macOS then kills the next start
/// for an invalid code signature. So the new one goes next to it and replaces it in one step.
fn put_program(from: &str, to: &str) -> String {
    format!(
        "cp {from} {to}.new
chown root:wheel {to}.new
chmod 755 {to}.new
mv -f {to}.new {to}
"
    )
}

/// Shell lines that put the program for the login screen in place (none without it).
fn copy_login_program(helper: &Path) -> String {
    match login_program(helper) {
        Some(program) => put_program(&quote(&program), LOGIN_BINARY),
        None => String::new(),
    }
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
{helper}{login}cp {plist} {PLIST}
chown root:wheel {PLIST}
chmod 644 {PLIST}
launchctl bootstrap system {PLIST}",
        helper = put_program(&quote(&source), INSTALLED_BINARY),
        login = copy_login_program(&source),
        plist = quote(&plist_file),
    );
    as_admin(&script)?;
    let _ = std::fs::remove_file(&plist_file);
    show_permission_place();
    Ok(())
}

/// Whether the installed helper is another build than the one next to this program (after an
/// update of the app). Compared by content; cached while the installed file stays the same.
pub fn outdated() -> bool {
    static CACHE: std::sync::Mutex<Option<(std::time::SystemTime, bool)>> = std::sync::Mutex::new(None);
    let Ok(modified) = std::fs::metadata(INSTALLED_BINARY).and_then(|m| m.modified()) else {
        return false;
    };
    let mut cache = CACHE.lock().expect("cache lock");
    if let Some((when, result)) = *cache
        && when == modified
    {
        return result;
    }
    let differs = match (built_helper().ok().map(std::fs::read), std::fs::read(INSTALLED_BINARY)) {
        (Some(Ok(built)), Ok(installed)) => built != installed,
        _ => false,
    };
    // The program for the login screen came with the helper later and changes with the app
    let login_differs = built_helper()
        .ok()
        .and_then(|helper| login_program(&helper))
        .and_then(|program| std::fs::read(program).ok())
        .is_some_and(|built| {
            std::fs::read(LOGIN_BINARY)
                .ok()
                .is_none_or(|installed| installed != built)
        });
    let differs = differs || login_differs;
    *cache = Some((modified, differs));
    differs
}

/// Replaces the installed helper with `new` and restarts it; keeps its permission when both are
/// signed by the same certificate. Asks for the administrator password.
pub fn update_from(new: &Path) -> Result<()> {
    if !Path::new(INSTALLED_BINARY).exists() {
        return Ok(());
    }
    as_admin(&format!(
        "set -e
{helper}{login}launchctl kickstart -k system/{LABEL}",
        helper = put_program(&quote(new), INSTALLED_BINARY),
        login = copy_login_program(new),
    ))
}

/// Replaces the installed helper with the one next to this program.
pub fn update() -> Result<()> {
    update_from(&built_helper()?)
}

/// Whether the helper is installed at all.
pub fn installed() -> bool {
    Path::new(INSTALLED_BINARY).exists()
}

/// Whether `new` (or the program for the login screen next to it) differs from what is installed.
pub fn differs_from(new: &Path) -> bool {
    let helper = match (std::fs::read(new), std::fs::read(INSTALLED_BINARY)) {
        (Ok(a), Ok(b)) => a != b,
        _ => false,
    };
    let login = login_program(new)
        .and_then(|program| std::fs::read(program).ok())
        .is_some_and(|built| {
            std::fs::read(LOGIN_BINARY)
                .ok()
                .is_none_or(|installed| installed != built)
        });
    helper || login
}

/// Opens the Input Monitoring settings and a Finder window with the helper file selected, so it
/// can be switched on there or dragged into the list.
pub fn show_permission_place() {
    let _ = std::process::Command::new("open")
        .arg(INPUT_MONITORING_SETTINGS)
        .status();
    let _ = std::process::Command::new("open")
        .args(["-R", INSTALLED_BINARY])
        .status();
}

/// The Accessibility settings and a Finder window with the program for the login screen, which
/// has to be dragged into that list.
pub fn show_login_permission_place() {
    let _ = std::process::Command::new("open").arg(ACCESSIBILITY_SETTINGS).status();
    let _ = std::process::Command::new("open").args(["-R", LOGIN_BINARY]).status();
}

/// A Finder window with the program for the login screen, e.g. for a rule of a network filter.
pub fn show_login_program() {
    let _ = std::process::Command::new("open").args(["-R", LOGIN_BINARY]).status();
}

pub fn uninstall() -> Result<()> {
    as_admin(&format!(
        "launchctl bootout system/{LABEL} 2>/dev/null || true
rm -f {PLIST} {INSTALLED_BINARY} {INSTALLED_BINARY}.new {LOGIN_BINARY} {LOGIN_BINARY}.new {LOGIN_PLIST}
rm -rf '{LOGIN_DIR}'
rm -rf /Library/Logs/TableCross
tccutil reset ListenEvent {LABEL} 2>/dev/null || true"
    ))
}

pub fn restart() -> Result<()> {
    as_admin(&format!("launchctl kickstart -k system/{LABEL}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperState {
    NotRunning,
    Running {
        version: u16,
        access: bool,
    },
    /// Something answers on the socket but not as expected.
    Unknown(String),
}

pub fn status() -> HelperState {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return HelperState::NotRunning;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(clone) = stream.try_clone() else {
        return HelperState::NotRunning;
    };
    let mut reader = BufReader::new(clone);
    if let Err(e) = write_frame(
        &mut stream,
        &ToHelper::Hello {
            version: HELPER_VERSION,
        },
    ) {
        return HelperState::Unknown(e.to_string());
    }
    match read_frame::<FromHelper>(&mut reader) {
        Ok(FromHelper::Hello { version, access }) => HelperState::Running { version, access },
        Ok(other) => HelperState::Unknown(format!("{other:?}")),
        Err(e) => HelperState::Unknown(e.to_string()),
    }
}

pub const LOG_FILE: &str = LOG;
