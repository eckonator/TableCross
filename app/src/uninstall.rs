//! Removing TableCross from this computer with everything it put there: system helper and its
//! permission, the Accessibility permission, starting with the system, optionally settings and
//! pairings, and the program itself.

use std::path::Path;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::PathBuf;

/// Runs on a background thread after the session stopped; the app quits afterwards.
pub fn run(dir: &Path, remove_settings: bool) -> Result<(), String> {
    if let Err(e) = crate::autostart::set(false) {
        tracing::warn!("autostart: {e}");
    }
    #[cfg(target_os = "macos")]
    {
        if tc_session::helper_install::installed() {
            tc_session::helper_install::uninstall().map_err(|e| e.to_string())?;
        }
        let _ = std::process::Command::new("tccutil")
            .args(["reset", "Accessibility", "de.eckonator.tablecross"])
            .status();
    }
    if remove_settings && dir.exists() {
        // Open log files may stay behind on Windows; nothing else does
        if let Err(e) = std::fs::remove_dir_all(dir) {
            tracing::warn!("settings folder: {e}");
        }
    }
    remove_program()
}

#[cfg(target_os = "macos")]
fn remove_program() -> Result<(), String> {
    let Some(bundle) = crate::self_update::bundle() else {
        return Ok(()); // not an installed app (development)
    };
    let trash = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".Trash"))
        .ok_or("no home folder")?;
    let mut target = trash.join("TableCross.app");
    let mut n = 2;
    while target.exists() {
        target = trash.join(format!("TableCross {n}.app"));
        n += 1;
    }
    std::fs::rename(&bundle, &target).map_err(|e| format!("{e}; please drag TableCross from Applications to the Trash"))
}

#[cfg(windows)]
fn remove_program() -> Result<(), String> {
    if !crate::self_update::installed_with_setup() {
        return Ok(()); // portable: the folder is deleted by hand
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let uninstaller = exe.with_file_name("unins000.exe");
    // It removes the service and the firewall rule as well, and asks for administrator rights itself
    std::process::Command::new(uninstaller)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("uninstaller: {e}"))
}

#[cfg(target_os = "linux")]
fn remove_program() -> Result<(), String> {
    let Some(exe) = crate::self_update::installed_for_user() else {
        return Ok(());
    };
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let mut files = vec![exe.clone(), exe.with_file_name("tablecross-cli")];
    if let Some(data) = data {
        files.push(data.join("applications/tablecross.desktop"));
        files.push(data.join("icons/hicolor/256x256/apps/tablecross.png"));
    }
    for file in files {
        let _ = std::fs::remove_file(file);
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn remove_program() -> Result<(), String> {
    Ok(())
}
