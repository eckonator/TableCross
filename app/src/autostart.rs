//! Starting with the system: a LaunchAgent on macOS, the `Run` key on Windows, an autostart entry
//! on Linux. The program then starts with `--background` (no window, only the status icon).

use std::path::PathBuf;

pub const BACKGROUND_FLAG: &str = "--background";

fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

#[cfg(target_os = "macos")]
fn agent_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/LaunchAgents/de.eckonator.tablecross.plist"))
}

#[cfg(target_os = "macos")]
pub fn set(enabled: bool) -> std::io::Result<()> {
    let Some(path) = agent_path() else { return Ok(()) };
    if !enabled {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let exe = exe().ok_or_else(|| std::io::Error::other("program path unknown"))?;
    let escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>de.eckonator.tablecross</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>{BACKGROUND_FLAG}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        escape(&exe.display().to_string())
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, plist)
}

#[cfg(windows)]
pub fn set(enabled: bool) -> std::io::Result<()> {
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let mut command = std::process::Command::new("reg");
    if enabled {
        let exe = exe().ok_or_else(|| std::io::Error::other("program path unknown"))?;
        let value = format!("\"{}\" {BACKGROUND_FLAG}", exe.display());
        command.args(["add", KEY, "/v", "TableCross", "/t", "REG_SZ", "/d", &value, "/f"]);
    } else {
        command.args(["delete", KEY, "/v", "TableCross", "/f"]);
    }
    use std::os::windows::process::CommandExt;
    // No console window flashing up
    command.creation_flags(0x0800_0000);
    let _ = command.status()?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn set(enabled: bool) -> std::io::Result<()> {
    let Some(dir) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    else {
        return Ok(());
    };
    let path = dir.join("autostart/tablecross.desktop");
    if !enabled {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let exe = exe().ok_or_else(|| std::io::Error::other("program path unknown"))?;
    std::fs::create_dir_all(dir.join("autostart"))?;
    std::fs::write(
        path,
        format!(
            "[Desktop Entry]\nType=Application\nName=TableCross\nExec=\"{}\" {BACKGROUND_FLAG}\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        ),
    )
}
