//! Authoritative client-side lobby state.
//!
//! [`Session::handle`] is a pure reducer: `(state, event) -> effects`. It does
//! no I/O, so it is tested by replaying captured server output and asserting
//! the commands it wants sent.

pub mod hosting;
pub mod lan;
pub mod session;
pub mod spads;
pub mod state;

pub use hosting::{Rtts, SpareRoom};
pub use lan::{Beacon, Found, Seen};
pub use session::{Effect, FriendAction, SeatError, Session, UnknownFriendAction};
pub use spads::{Announcement, Proposal, VoteState};
pub use state::{Battle, Bot, LobbyState, MyBattle, OptionChange, Phase, StartRect, User};
