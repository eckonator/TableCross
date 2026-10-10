//! A running TableCross session: the network node, the engine and the input backend, driven by one
//! loop. The app and the command line tool control it with commands and read a status snapshot.

mod clip;
mod files;
#[cfg(target_os = "macos")]
pub mod helper_install;
pub mod layout_file;
pub mod peer_logs;
mod session;
mod transfer;

#[cfg(test)]
mod tests;

pub use clip::{Clipboard, Skip};
pub use session::{
    Command, InputState, Notice, OnChange, PairTarget, PeerView, Session, SessionConfig, SessionHandle, Status,
};
pub use transfer::{TransferState, TransferView};
