//! The settings of games already played, read back out of their replays.
//!
//! Every replay opens with the start script that made its game: the map, the
//! modoptions, the tweak slots. Indexed, those scripts become one row per
//! distinct setting, so a room can be compared with any game played before,
//! two past games with each other, and a past setting loaded back.
//!
//! Nothing here is sent anywhere. Player names and account ids are read to
//! tell whether a game was ours, and they stay on this side: the view the
//! webview gets carries only a [`view::Role`].

pub mod index;
pub mod preset;
pub mod read;
pub mod store;
pub mod view;

pub use index::{Index, ScanProgress, Source, refresh};
pub use read::IGNORED_PREFIXES;
pub use store::Store;
pub use view::{HistoryView, Identity, Role, view};

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("{0}")]
	Io(#[from] std::io::Error),
	#[error("history index: {0}")]
	Json(#[from] serde_json::Error),
	#[error("no setting {0} in the history")]
	Unknown(String),
	#[error("not a blob id: {0}")]
	BadId(String),
}
