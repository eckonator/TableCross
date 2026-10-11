//! macOS Universal Control also takes the pointer at screen edges and moves it to other Macs. Next
//! to TableCross the two fight over the pointer, and with TableCross quit the pointer still crosses
//! over. The switch lives in the per-host preferences of `com.apple.universalcontrol`.

use std::ffi::CString;

use core_foundation_sys::base::{CFGetTypeID, CFRelease, kCFAllocatorDefault};
use core_foundation_sys::number::{CFBooleanGetTypeID, CFBooleanGetValue, kCFBooleanTrue};
use core_foundation_sys::preferences::{
    CFPreferencesCopyValue, CFPreferencesSetValue, CFPreferencesSynchronize, kCFPreferencesCurrentHost,
    kCFPreferencesCurrentUser,
};
use core_foundation_sys::string::{CFStringCreateWithCString, CFStringRef, kCFStringEncodingUTF8};

const DOMAIN: &str = "com.apple.universalcontrol";
const AGENT: &str = "/System/Library/CoreServices/UniversalControl.app";

fn cf_string(text: &str) -> CFStringRef {
    let c = CString::new(text).expect("no NUL");
    unsafe { CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), kCFStringEncodingUTF8) }
}

/// Universal Control exists on this Mac and is switched on.
pub fn is_on() -> bool {
    if !std::path::Path::new(AGENT).exists() {
        return false;
    }
    unsafe {
        let domain = cf_string(DOMAIN);
        let key = cf_string("Disable");
        // Picks up a change made in System Settings meanwhile
        CFPreferencesSynchronize(domain, kCFPreferencesCurrentUser, kCFPreferencesCurrentHost);
        let value = CFPreferencesCopyValue(key, domain, kCFPreferencesCurrentUser, kCFPreferencesCurrentHost);
        let disabled =
            !value.is_null() && CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value.cast());
        if !value.is_null() {
            CFRelease(value);
        }
        CFRelease(key.cast());
        CFRelease(domain.cast());
        !disabled
    }
}

/// Switches Universal Control off, like the switch in System Settings → Displays → Advanced, and
/// restarts its agent so the change takes effect at once.
pub fn turn_off() -> Result<(), String> {
    unsafe {
        let domain = cf_string(DOMAIN);
        for name in ["Disable", "DisableMagicEdges"] {
            let key = cf_string(name);
            CFPreferencesSetValue(
                key,
                kCFBooleanTrue.cast(),
                domain,
                kCFPreferencesCurrentUser,
                kCFPreferencesCurrentHost,
            );
            CFRelease(key.cast());
        }
        let saved = CFPreferencesSynchronize(domain, kCFPreferencesCurrentUser, kCFPreferencesCurrentHost) != 0;
        CFRelease(domain.cast());
        if !saved {
            return Err("cannot save the Universal Control setting".into());
        }
    }
    // Not running is fine as well
    let _ = std::process::Command::new("/usr/bin/killall")
        .arg("UniversalControl")
        .status();
    if is_on() {
        return Err("Universal Control is still switched on".into());
    }
    Ok(())
}
