//! Capturing the local mouse and keyboard and injecting input from other machines, per operating
//! system, behind two small traits. The logic of *where* input goes lives in `tc-core`; a backend
//! only reports events and carries out what it is told.

pub mod keymap {
    //! Native key codes ↔ USB HID usages. Plain tables, compiled and tested on every system.
    pub mod mac;
    pub mod windows;
}

pub mod helper_protocol;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod mac_helper;
mod virtual_input;
#[cfg(windows)]
mod windows;

use std::sync::Arc;

use tc_core::{Captured, Point};
use tc_protocol::{Message, Screen};

pub use virtual_input::{VirtualCapture, VirtualDevice, VirtualInject, virtual_backend};

/// Called for every event of the local devices. Returns whether the local system gets the event
/// (`true`) or it is suppressed (`false`). Runs on the capture thread and must be quick.
pub type Handler = Box<dyn FnMut(Captured) -> bool + Send>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Health {
    /// Events of the local devices arrive.
    pub capturing: bool,
    /// What is wrong, in words for the user.
    pub problems: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("permission missing: {0}")]
    Permission(String),
    #[error("{0}")]
    System(String),
    #[error("not supported on this system yet")]
    Unsupported,
}

/// The local devices.
pub trait Capture: Send {
    /// Starts capturing on a thread of its own.
    fn start(&mut self, handler: Handler) -> Result<(), InputError>;
    fn stop(&mut self);
    fn health(&self) -> Health;
}

/// The local system's pointer and keyboard.
pub trait Inject: Send + Sync {
    /// Hide the pointer and keep it at `park`; the capture reports motion as deltas from now on.
    fn grab(&self, park: Point) -> Result<(), InputError>;
    /// Show the pointer again at `at`.
    fn ungrab(&self, at: Point) -> Result<(), InputError>;
    /// `Enter`, `MouseMove`, `Key`, `MouseButton`, `Wheel`.
    fn inject(&self, message: &Message) -> Result<(), InputError>;
    /// Releases everything injected so far (safety net when shutting down).
    fn release_all(&self);
    fn screens(&self) -> Vec<Screen>;
}

/// Capture and injection of one system.
pub struct Backend {
    pub capture: Box<dyn Capture>,
    pub inject: Arc<dyn Inject>,
}

/// The backend of this system.
pub fn platform() -> Result<Backend, InputError> {
    #[cfg(target_os = "macos")]
    {
        mac::backend()
    }
    #[cfg(windows)]
    {
        windows::backend()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err(InputError::Unsupported)
    }
}
