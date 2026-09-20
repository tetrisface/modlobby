//! Playing on the local network, with no lobby server in sight.
//!
//! The host runs a small server here that speaks enough of the lobby protocol
//! for modlobby's own client — the host's included — to log in, see one
//! battle, join it, sit down, talk and hear the game start. Guests are the
//! same client pointed at the host's address. Nothing about a room changes
//! shape for being on the LAN: every line emitted is one BAR's server would
//! emit, which is what lets the online room be drawn unchanged.
//!
//! What the server does not do is run the game. The host's engine is the game
//! server, as it is for any hosted game; this only tells everyone when it has
//! started, and writes the script it starts on.

pub mod discover;
pub mod getmap;
pub mod room;
pub mod script;
pub mod serve;
pub mod wire;

pub use serve::Host;

pub use room::{Config, Out, Policy, Room};

/// The server id the settings list the LAN under. A host `lan` is not a name
/// any DNS resolves, which is the point: the app rewrites it to an address at
/// connect time and the session stays keyed by this.
pub const LAN_ID: &str = "lan";

/// Where the host's lobby server listens. The same port BAR's server uses,
/// so the entry in the settings reads like any other.
pub const DEFAULT_PORT: u16 = 8200;

/// Where the host's engine listens, which is what `BATTLEOPENED` announces.
/// The engine's own default (`ClientSetup.cpp`).
pub const ENGINE_PORT: u16 = 8452;

/// The one battle a LAN room holds.
pub const BATTLE_ID: u32 = 1;
