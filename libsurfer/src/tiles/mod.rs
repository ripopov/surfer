//! Workspace identity, layout and command boundaries.
//!
//! Application IDs are independent of the layout library's runtime node IDs.

pub mod commands;
pub(crate) use workspace::history;
pub mod input;
pub mod kind;
pub mod layout;
pub(crate) use workspace::legacy;
mod placement;
pub mod render;
pub mod resources;
pub mod runtime;
pub mod serde;
pub mod view;
pub mod workspace;

use ::serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId(pub u64);

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemListId(pub u64);

/// Input intent. Resolve at the input boundary; queued commands carry `TileId`.
#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum TileTarget {
    Id(TileId),
    Focused,
}
