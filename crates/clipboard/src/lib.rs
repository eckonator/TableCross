//! The system clipboard: reading, writing and noticing changes, per operating system.
//!
//! Content marked by password managers as concealed or transient is never read, so it never
//! leaves the computer.

#[cfg(any(windows, test))]
mod cf_html;
mod dib;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

use std::sync::Mutex;

pub use dib::{dib_to_png, png_to_dib};
pub use tc_protocol::ClipContent;

#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    #[error("the clipboard is busy")]
    Busy,
    #[error("{0}")]
    System(String),
}

/// What reading found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Read {
    Content(ClipContent),
    /// Nothing in a format we carry.
    #[default]
    Empty,
    /// Marked by a password manager: must not leave this computer.
    Concealed,
}

pub trait SystemClipboard: Send + Sync {
    /// Changes whenever the clipboard changes (also by our own writes).
    fn change_count(&self) -> u64;
    fn read(&self) -> Result<Read, ClipError>;
    /// Replaces the content; returns the change count afterwards.
    fn write(&self, content: &ClipContent) -> Result<u64, ClipError>;
}

/// The clipboard of this system, if supported.
pub fn platform() -> Option<Box<dyn SystemClipboard>> {
    #[cfg(target_os = "macos")]
    return Some(Box::new(mac::MacClipboard::default()));
    #[cfg(windows)]
    return Some(Box::new(windows::WinClipboard));
    #[cfg(target_os = "linux")]
    return linux::LinuxClipboard::new().map(|c| Box::new(c) as Box<dyn SystemClipboard>);
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    None
}

/// A clipboard in memory, for tests.
#[derive(Default)]
pub struct MemoryClipboard {
    state: Mutex<(u64, Read)>,
}

impl MemoryClipboard {
    pub fn new() -> Self {
        Self {
            state: Mutex::new((0, Read::Empty)),
        }
    }

    /// As if the user copied something.
    pub fn set(&self, read: Read) {
        let mut state = self.state.lock().expect("clipboard lock");
        state.0 += 1;
        state.1 = read;
    }

    pub fn get(&self) -> Read {
        self.state.lock().expect("clipboard lock").1.clone()
    }
}

impl SystemClipboard for MemoryClipboard {
    fn change_count(&self) -> u64 {
        self.state.lock().expect("clipboard lock").0
    }

    fn read(&self) -> Result<Read, ClipError> {
        Ok(self.get())
    }

    fn write(&self, content: &ClipContent) -> Result<u64, ClipError> {
        let mut state = self.state.lock().expect("clipboard lock");
        state.0 += 1;
        state.1 = Read::Content(content.clone());
        Ok(state.0)
    }
}
