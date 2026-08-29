//! Authoritative client-side lobby state.
//!
//! [`Session::handle`] is a pure reducer: `(state, event) -> effects`. It does
//! no I/O, so it is tested by replaying captured server output and asserting
//! the commands it wants sent.

pub mod session;
pub mod spads;
pub mod state;

pub use session::{Effect, SeatError, Session};
pub use spads::{Announcement, Proposal, VoteState};
pub use state::{Battle, Bot, LobbyState, MyBattle, OptionChange, Phase, StartRect, User};
