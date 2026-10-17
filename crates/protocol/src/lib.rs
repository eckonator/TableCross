//! The common language of TableCross peers: messages, their wire format and key state tracking.
//!
//! Deliberately free of networking and OS code, so everything here behaves the same on every
//! platform and can be tested without a second computer.

mod clipboard;
mod keys;
mod message;

pub use clipboard::{ClipContent, MAX_CLIP_LEN};
pub use keys::{PressedKeys, is_key_usage};
pub use message::{
    ClipFormat, DecodeError, MAX_LOG_TEXT, MAX_MESSAGE_LEN, MAX_PLACEMENTS, MAX_TILE_POS, MediaKey, Message,
    MouseButton, PROTOCOL_VERSION, Placement, Screen, Side, Tile, decode, encode,
};
