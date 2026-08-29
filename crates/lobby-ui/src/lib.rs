//! What a front end sees: one [`Snapshot`] when the lobby is ready, then
//! [`Delta`]s. Pure — no I/O, no Tauri — so the contract is testable and the
//! TypeScript side is generated from it (`ts-rs`, on `cargo test`).

pub mod batch;
pub mod model;
pub mod project;
pub mod transport;

pub use batch::Batcher;
pub use model::{
    BATTLE_ROOM, BattleStatusView, BattleView, BotView, ChannelSummaryView, ChannelView, ChatKind,
    ChatLine, Delta, EngineStatus, GameRunningView, LayoutView, MyBattleView, NoticeLevel,
    OptionChangeView, Phase, ProposalView, Snapshot, StartRectView, SyncView, UiMessage,
    UserStatusView, UserView, VoteView, private_room,
};
pub use project::Projector;
pub use transport::{Collector, UiClosed, UiTransport};
