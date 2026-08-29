//! Authoritative client-side lobby state.
//!
//! [`Session::handle`] is a pure reducer: `(state, event) -> effects`. It does
//! no I/O, so it is tested by replaying captured server output and asserting
//! the commands it wants sent.

pub mod session;
pub mod state;

pub use session::{Effect, Session};
pub use state::{Battle, LobbyState, Phase, User};
