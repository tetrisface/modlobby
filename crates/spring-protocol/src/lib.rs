//! Legacy SpringLobbyProtocol as spoken by teiserver: line framing, typed
//! server events, login and telemetry encoding, the outbound throttle policy,
//! and a tokio transport actor that ties them together.
//!
//! Reference implementations vendored under `external/`:
//! `BYAR-Chobby/libs/liblobby/lobby/interface*.lua` (client) and
//! `teiserver/lib/teiserver/protocols/spring/*.ex` (server).

pub mod codec;
pub mod event;
pub mod hash;
pub mod login;
pub mod policy;
pub mod telemetry;
pub mod transport;

pub use codec::RawMessage;
pub use event::{BattleOpened, ServerEvent, UserStatus};
pub use login::LoginRequest;
pub use policy::{Area, Envelope, Mode, ThrottlePolicy};
pub use transport::{Inbound, Transport, TransportError};
