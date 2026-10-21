//! macOS Universal Control also takes the pointer at screen edges and moves it to other Macs. Next
//! to TableCross the two fight over the pointer, and with TableCross quit the pointer still crosses
//! over. The switch lives in the per-host preferences of `com.apple.universalcontrol`.

use crate::mac_prefs;

const DOMAIN: &str = "com.apple.universalcontrol";
const AGENT: &str = "/System/Library/CoreServices/UniversalControl.app";

/// Universal Control exists on this Mac and is switched on.
pub fn is_on() -> bool {
    std::path::Path::new(AGENT).exists() && mac_prefs::get(DOMAIN, "Disable") != Some(true)
}

/// Switches Universal Control off, like the switch in System Settings → Displays → Advanced, and
/// restarts its agent so the change takes effect at once.
pub fn turn_off() -> Result<(), String> {
    mac_prefs::set(DOMAIN, &["Disable", "DisableMagicEdges"], true)?;
    // Not running is fine as well
    let _ = std::process::Command::new("/usr/bin/killall")
        .arg("UniversalControl")
        .status();
    if is_on() {
        return Err("Universal Control is still switched on".into());
    }
    Ok(())
}
