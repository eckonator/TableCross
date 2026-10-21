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

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
    /// Files and folders that were copied.
    Files(Vec<PathBuf>),
}

/// Fetches promised files when they are pasted and returns where they are now (`None`: failed).
/// Runs on the thread that pastes and may take as long as the transfer.
pub type Fulfil = Arc<dyn Fn() -> Option<Vec<PathBuf>> + Send + Sync>;

pub trait SystemClipboard: Send + Sync {
    /// Changes whenever the clipboard changes (also by our own writes).
    fn change_count(&self) -> u64;
    fn read(&self) -> Result<Read, ClipError>;
    /// Replaces the content; returns the change count afterwards.
    fn write(&self, content: &ClipContent) -> Result<u64, ClipError>;
    /// Puts files on the clipboard, ready to be pasted in the file manager.
    fn write_files(&self, files: &[PathBuf]) -> Result<u64, ClipError>;
    /// Whether `write_file_promise` works here.
    fn can_promise(&self) -> bool {
        false
    }
    /// Puts `count` files on the clipboard that do not exist yet: pasting them in the file
    /// manager calls `fulfil` first. `None`: this system cannot (the files must exist).
    fn write_file_promise(&self, count: usize, fulfil: Fulfil) -> Option<Result<u64, ClipError>> {
        let _ = (count, fulfil);
        None
    }
}

/// Files being dragged with the mouse. Only macOS tells other programs about a drag (through the
/// drag pasteboard); elsewhere nothing is found.
pub mod drag {
    use std::path::PathBuf;

    /// A number that changes whenever a drag starts.
    pub fn change_count() -> Option<u64> {
        #[cfg(target_os = "macos")]
        return Some(crate::mac::drag_change_count());
        #[cfg(not(target_os = "macos"))]
        None
    }

    /// The files dragged since `change_count` returned `before`.
    pub fn files_since(before: u64) -> Vec<PathBuf> {
        #[cfg(target_os = "macos")]
        if crate::mac::drag_change_count() != before {
            return crate::mac::drag_files();
        }
        let _ = before;
        Vec::new()
    }
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
    promise: Mutex<Option<Fulfil>>,
}

impl MemoryClipboard {
    pub fn new() -> Self {
        Self {
            state: Mutex::new((0, Read::Empty)),
            promise: Mutex::new(None),
        }
    }

    /// As if the promised files were pasted: what the file manager would get.
    pub fn paste_promise(&self) -> Option<Vec<PathBuf>> {
        let fulfil = self.promise.lock().expect("clipboard lock").clone()?;
        fulfil()
    }

    /// As if the user copied something.
    pub fn set(&self, read: Read) {
        *self.promise.lock().expect("clipboard lock") = None;
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

    fn write_files(&self, files: &[PathBuf]) -> Result<u64, ClipError> {
        let mut state = self.state.lock().expect("clipboard lock");
        state.0 += 1;
        state.1 = Read::Files(files.to_vec());
        Ok(state.0)
    }

    fn can_promise(&self) -> bool {
        true
    }

    fn write_file_promise(&self, _count: usize, fulfil: Fulfil) -> Option<Result<u64, ClipError>> {
        let mut state = self.state.lock().expect("clipboard lock");
        state.0 += 1;
        state.1 = Read::Empty;
        *self.promise.lock().expect("clipboard lock") = Some(fulfil);
        Some(Ok(state.0))
    }
}
