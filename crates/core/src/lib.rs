//! TableCross logic without OS code: screen geometry, which machine sits where, and the state
//! machine that decides where input goes. Everything here is tested without a second computer.

mod engine;
mod geometry;
mod layout;

#[cfg(test)]
mod sim;

pub use engine::{Action, Captured, Engine, MachineId, Role, is_modifier};
pub use geometry::{Point, Rect, Screens, Side};
pub use layout::{Layout, LayoutError, LayoutFile, Neighbor};
