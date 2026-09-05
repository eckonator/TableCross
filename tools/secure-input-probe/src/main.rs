//! Checks whether key presses still arrive on macOS while Secure Input is active.
//!
//! Listens on two paths at once and logs every key with a timestamp:
//! - `TAP`: CGEventTap at HID level (the approach most similar tools use)
//! - `HID`: raw keyboard and media key values via IOHIDManager
//!
//! `STATUS` reports whenever a process enables or releases Secure Input.
//!
//! Usage:
//!   secure-input-probe [--seconds N]     listen (default 60 s)
//!   secure-input-probe --hold N          hold Secure Input for N seconds
//!   secure-input-probe --service FILE    test service (root): seize the keyboard, see `run_service`

#[cfg(target_os = "macos")]
mod mac;

fn main() {
    #[cfg(target_os = "macos")]
    mac::run();
    #[cfg(not(target_os = "macos"))]
    eprintln!("This probe only runs on macOS.");
}
