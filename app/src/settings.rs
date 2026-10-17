//! App settings, `settings.json` in the settings folder.
//!
//! Loaded field by field: a field that is missing or broken gets its default and the others are
//! kept, so one bad value (or a file from another version) never resets everything.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LangSetting {
    #[default]
    System,
    De,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSetting {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub language: LangSetting,
    pub theme: ThemeSetting,
    /// Start with the system (in the background).
    pub autostart: bool,
    /// Name shown to other computers; empty: the host name.
    pub name: String,
    pub port: u16,
    pub discovery: bool,
    /// The setup steps were finished or skipped once.
    pub setup_done: bool,
    /// Share the clipboard with the other computers.
    pub clipboard: bool,
    /// Larger clipboard content stays on this computer.
    pub clipboard_max_mb: u32,
    /// Look for new versions on GitHub (`None`: not asked yet).
    pub update_check: Option<bool>,
    /// Update by itself when a paired computer runs a newer version.
    pub update_with_peers: bool,
    /// Collect the logs of the other computers here (troubleshooting).
    pub collect_logs: bool,
    /// macOS: switch off Universal Control at start (it takes the pointer at screen edges as well).
    pub universal_control_off: bool,
    /// Keep the system from going to sleep while TableCross runs (the display may still turn off).
    pub stay_awake: bool,
    /// With `stay_awake`: keep the display on as well.
    pub screen_stays_on: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: LangSetting::System,
            theme: ThemeSetting::System,
            autostart: false,
            name: String::new(),
            port: tc_net::DEFAULT_PORT,
            discovery: true,
            setup_done: false,
            clipboard: true,
            clipboard_max_mb: 50,
            update_check: None,
            update_with_peers: true,
            collect_logs: false,
            universal_control_off: true,
            stay_awake: false,
            screen_stays_on: false,
        }
    }
}

fn field<T: DeserializeOwned>(fields: &serde_json::Map<String, serde_json::Value>, key: &str, default: T) -> T {
    fields
        .get(key)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(default)
}

impl Settings {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(FILE)
    }

    pub fn load(dir: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(Self::path(dir)) else {
            return Self::default();
        };
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Self {
        let d = Self::default();
        let Ok(serde_json::Value::Object(fields)) = serde_json::from_str::<serde_json::Value>(text) else {
            tracing::warn!("settings are unreadable, using defaults");
            return d;
        };
        Self {
            language: field(&fields, "language", d.language),
            theme: field(&fields, "theme", d.theme),
            autostart: field(&fields, "autostart", d.autostart),
            name: field(&fields, "name", d.name),
            port: field(&fields, "port", d.port).max(1),
            discovery: field(&fields, "discovery", d.discovery),
            setup_done: field(&fields, "setup_done", d.setup_done),
            clipboard: field(&fields, "clipboard", d.clipboard),
            clipboard_max_mb: field(&fields, "clipboard_max_mb", d.clipboard_max_mb).clamp(1, 64),
            update_check: field(&fields, "update_check", d.update_check),
            update_with_peers: field(&fields, "update_with_peers", d.update_with_peers),
            collect_logs: field(&fields, "collect_logs", d.collect_logs),
            universal_control_off: field(&fields, "universal_control_off", d.universal_control_off),
            stay_awake: field(&fields, "stay_awake", d.stay_awake),
            screen_stays_on: field(&fields, "screen_stays_on", d.screen_stays_on),
        }
    }

    pub fn save(&self, dir: &Path) {
        let result = std::fs::create_dir_all(dir).and_then(|()| {
            let path = Self::path(dir);
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
            std::fs::rename(&tmp, &path)
        });
        if let Err(e) = result {
            tracing::warn!("cannot save the settings: {e}");
        }
    }

    /// The name used on the network.
    pub fn effective_name(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() { host_name() } else { name.to_string() }
    }
}

/// The name of this computer: on macOS the one in System Settings (General → About), which no
/// router can change; elsewhere the host name without domain ("office-pc.fritz.box" → "office-pc").
pub fn host_name() -> String {
    #[cfg(target_os = "macos")]
    if let Some(name) = mac_computer_name() {
        return name;
    }
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    name.split('.').next().unwrap_or_default().to_string()
}

#[cfg(target_os = "macos")]
fn mac_computer_name() -> Option<String> {
    use core_foundation_sys::base::{CFRelease, CFTypeRef};
    use core_foundation_sys::string::{CFStringGetCString, CFStringRef, kCFStringEncodingUTF8};
    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCDynamicStoreCopyComputerName(store: CFTypeRef, encoding: *mut u32) -> CFStringRef;
    }
    unsafe {
        let name = SCDynamicStoreCopyComputerName(std::ptr::null(), std::ptr::null_mut());
        if name.is_null() {
            return None;
        }
        let mut buf = [0 as std::ffi::c_char; 256];
        let ok = CFStringGetCString(name, buf.as_mut_ptr(), buf.len() as _, kCFStringEncodingUTF8) != 0;
        CFRelease(name.cast());
        let text = std::ffi::CStr::from_ptr(buf.as_ptr())
            .to_string_lossy()
            .trim()
            .to_string();
        (ok && !text.is_empty()).then_some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_computer_has_a_name() {
        assert!(!host_name().is_empty());
    }

    #[test]
    fn broken_fields_keep_the_others() {
        let s = Settings::parse(r#"{"language":"en","theme":"purple","port":"x","autostart":true,"future":1}"#);
        assert_eq!(s.language, LangSetting::En);
        assert_eq!(s.theme, ThemeSetting::System);
        assert_eq!(s.port, tc_net::DEFAULT_PORT);
        assert!(s.autostart);
        assert_eq!(Settings::parse("not json"), Settings::default());
    }

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("tablecross-settings-{}", std::process::id()));
        let s = Settings {
            name: "Office".into(),
            setup_done: true,
            ..Settings::default()
        };
        s.save(&dir);
        assert_eq!(Settings::load(&dir), s);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
