//! Handoff of macOS carries the clipboard to other Macs of the same account (Universal Clipboard).
//! Next to TableCross a copy here reaches the other Mac twice: TableCross puts it there, Handoff
//! announces it back as new, and pasting here then waits for the other Mac. The switch lives in the
//! per-host preferences of `com.apple.coreservices.useractivityd`.

use crate::mac_prefs;

const DOMAIN: &str = "com.apple.coreservices.useractivityd";
const KEYS: [&str; 2] = ["ActivityAdvertisingAllowed", "ActivityReceivingAllowed"];

/// Handoff is switched on (it is unless switched off).
pub fn is_on() -> bool {
    KEYS.iter().any(|key| mac_prefs::get(DOMAIN, key) != Some(false))
}

/// Switches Handoff off, like the switch in System Settings → General → AirDrop & Handoff, and
/// restarts its agent so the change takes effect at once.
pub fn turn_off() -> Result<(), String> {
    mac_prefs::set(DOMAIN, &KEYS, false)?;
    // Not running is fine as well
    let _ = std::process::Command::new("/usr/bin/killall")
        .arg("useractivityd")
        .status();
    if is_on() {
        return Err("Handoff is still switched on".into());
    }
    Ok(())
}
