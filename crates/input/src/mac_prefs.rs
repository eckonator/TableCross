//! Per-host preferences of macOS (`defaults -currentHost`), where System Settings keeps switches
//! like Universal Control and Handoff.

use std::ffi::CString;

use core_foundation_sys::base::{CFGetTypeID, CFRelease, kCFAllocatorDefault};
use core_foundation_sys::number::{CFBooleanGetTypeID, CFBooleanGetValue, kCFBooleanFalse, kCFBooleanTrue};
use core_foundation_sys::preferences::{
    CFPreferencesCopyValue, CFPreferencesSetValue, CFPreferencesSynchronize, kCFPreferencesCurrentHost,
    kCFPreferencesCurrentUser,
};
use core_foundation_sys::string::{CFStringCreateWithCString, CFStringRef, kCFStringEncodingUTF8};

fn cf_string(text: &str) -> CFStringRef {
    let c = CString::new(text).expect("no NUL");
    unsafe { CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), kCFStringEncodingUTF8) }
}

/// The value of a switch; `None` when it was never set (the system default applies).
pub(crate) fn get(domain: &str, key: &str) -> Option<bool> {
    unsafe {
        let domain = cf_string(domain);
        let key = cf_string(key);
        // Picks up a change made in System Settings meanwhile
        CFPreferencesSynchronize(domain, kCFPreferencesCurrentUser, kCFPreferencesCurrentHost);
        let value = CFPreferencesCopyValue(key, domain, kCFPreferencesCurrentUser, kCFPreferencesCurrentHost);
        let out =
            (!value.is_null() && CFGetTypeID(value) == CFBooleanGetTypeID()).then(|| CFBooleanGetValue(value.cast()));
        if !value.is_null() {
            CFRelease(value);
        }
        CFRelease(key.cast());
        CFRelease(domain.cast());
        out
    }
}

/// Sets several switches of one domain and saves them.
pub(crate) fn set(domain: &str, keys: &[&str], value: bool) -> Result<(), String> {
    unsafe {
        let domain_ref = cf_string(domain);
        let value = if value { kCFBooleanTrue } else { kCFBooleanFalse };
        for name in keys {
            let key = cf_string(name);
            CFPreferencesSetValue(
                key,
                value.cast(),
                domain_ref,
                kCFPreferencesCurrentUser,
                kCFPreferencesCurrentHost,
            );
            CFRelease(key.cast());
        }
        let saved = CFPreferencesSynchronize(domain_ref, kCFPreferencesCurrentUser, kCFPreferencesCurrentHost) != 0;
        CFRelease(domain_ref.cast());
        if saved {
            Ok(())
        } else {
            Err(format!("cannot save the preferences of {domain}"))
        }
    }
}
