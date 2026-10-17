//! TableCross logic without OS code: screen geometry, which machine sits where, and the state
//! machine that decides where input goes. Everything here is tested without a second computer.

mod engine;
mod geometry;
mod layout;

#[cfg(test)]
mod sim;

pub use engine::{Action, Captured, CornerBlock, EDGE_DWELL, Engine, MachineId, Role, is_modifier};
pub use geometry::{Point, Rect, Screens, Side};
pub use layout::{
    Area, Layout, LayoutError, LayoutFile, Neighbor, STAND_IN, Tile, area_side, areas_from_tiles, neighbors_from_areas,
    resized, tiles_from_neighbors,
};
