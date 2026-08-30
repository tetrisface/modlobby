//! Saved room setups.
//!
//! A preset is a map, a pile of modoptions (tweak slots included), the SPADS
//! room settings, the start boxes and the AI slots — everything it takes to
//! put a room back the way it was. Chobby keeps these in
//! `optionsPresets.json`; this keeps its own file so it can also record when
//! each was made, changed and last used, and interoperates with that one in
//! both directions.

pub mod apply;
pub mod chobby;
pub mod model;
pub mod store;

pub use apply::{Plan, PlannedBox, Room, Sections, plan};
pub use model::{Book, Preset, Stamp, StartBox, VERSION};
pub use store::{Error, Store};
