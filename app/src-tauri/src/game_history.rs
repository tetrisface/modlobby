//! The settings of games played on this machine, read back out of their
//! replays: see the `game-history` crate.
//!
//! The history is read from disk the first time anything asks and held for
//! the run. A look at the replays happens only when the History tab asks for
//! one: when it opens, when a game ends, and on Rescan. Nothing scans at
//! start.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use game_history::view::ValuesView;
use game_history::{HistoryView, Identity, Index, Store};
use lobby_ui::Snapshot;
use presets::{Book, Plan, Preset, Sections};
use tauri::{Emitter, State};

use crate::commands::{ApiError, Result, data_dirs, disk_work, with_mods_demos};
use crate::presets::{now, put_in_skirmish, send_to_room};
use crate::state::App;

impl From<game_history::Error> for ApiError {
	fn from(err: game_history::Error) -> Self {
		let code = match err {
			game_history::Error::Unknown(_) => "missing",
			game_history::Error::BadId(_) => "input",
			game_history::Error::Io(_) | game_history::Error::Json(_) => "history",
		};
		ApiError::new(code, err.to_string())
	}
}

/// The history as held for the run.
pub struct Held {
	store: Store,
	/// `None` until anything asks.
	index: Mutex<Option<Arc<Index>>>,
	/// One look at the replays at a time.
	scanning: tokio::sync::Mutex<()>,
}

impl Held {
	pub fn new(store: Store) -> Self {
		Self {
			store,
			index: Mutex::new(None),
			scanning: tokio::sync::Mutex::new(()),
		}
	}

	/// The index, read from disk the first time. Blocks on the disk then, so
	/// it is asked off the main thread. A read that fails is asked again next
	/// time rather than held as an empty history.
	fn index(&self) -> std::result::Result<Arc<Index>, game_history::Error> {
		let mut held = self.index.lock().unwrap_or_else(PoisonError::into_inner);
		if let Some(index) = held.as_ref() {
			return Ok(index.clone());
		}
		let index = Arc::new(self.store.load()?);
		*held = Some(index.clone());
		Ok(index)
	}

	fn replace(&self, index: Index) -> Arc<Index> {
		let index = Arc::new(index);
		*self.index.lock().unwrap_or_else(PoisonError::into_inner) = Some(index.clone());
		index
	}
}

/// Every name and account id this machine is known by: each server's own
/// account and id, the skirmish's player, and every account in the settings.
pub(crate) fn identity_of(snapshot: &Snapshot, usernames: &[String]) -> Identity {
	let mut names: Vec<String> = usernames
		.iter()
		.filter(|name| !name.is_empty())
		.cloned()
		.collect();
	let mut ids = Vec::new();
	for server in &snapshot.servers {
		let Some(me) = &server.me else {
			continue;
		};
		names.push(me.clone());
		ids.extend(
			server
				.users
				.iter()
				.filter(|user| &user.name == me)
				.filter_map(|user| u32::try_from(user.user_id?).ok()),
		);
	}
	if let Some(skirmish) = &snapshot.skirmish {
		names.push(skirmish.me.clone());
	}
	names.sort();
	names.dedup();
	ids.sort_unstable();
	ids.dedup();
	Identity { names, ids }
}

async fn identity(app: &App) -> Identity {
	let usernames: Vec<String> = app
		.settings
		.get()
		.servers
		.iter()
		.map(|server| server.username.clone())
		.collect();
	match app.client.snapshot().await {
		Ok(snapshot) => identity_of(&snapshot, &usernames),
		Err(_) => Identity {
			names: usernames,
			ids: Vec::new(),
		},
	}
}

/// The history as last read, without looking at the replays.
#[tauri::command]
pub async fn game_history(app: State<'_, App>) -> Result<HistoryView> {
	let me = identity(&app).await;
	let held = app.game_history.clone();
	let index = disk_work(move || held.index()).await??;
	Ok(game_history::view(&index, &me))
}

/// Looks at the replays for games new or changed since the last look, and
/// answers with the history after. Progress arrives on `game-history-scan`.
#[tauri::command]
pub async fn game_history_scan(app: State<'_, App>, window: tauri::Window) -> Result<HistoryView> {
	let held = app.game_history.clone();
	let _one_at_a_time = held.scanning.lock().await;
	// Every directory the Replays page lists, the mods' own included.
	let library = content::Library::new(with_mods_demos(data_dirs(&app)?));
	let me = identity(&app).await;
	let work = held.clone();
	disk_work(move || {
		let before = work.index()?;
		let after = game_history::refresh(&before, &work.store, &library, |step| {
			let _ = window.emit("game-history-scan", step);
		})?;
		Ok(game_history::view(&work.replace(after), &me))
	})
	.await?
}

/// One tweak's blob, for the Lua diff.
#[tauri::command]
pub async fn game_history_blob(app: State<'_, App>, id: String) -> Result<String> {
	let held = app.game_history.clone();
	Ok(disk_work(move || held.store.blob(&id)).await??)
}

/// The room's modoptions held to the rule a game's are, to compare with one.
#[tauri::command]
pub async fn game_history_values(values: BTreeMap<String, String>) -> Result<ValuesView> {
	Ok(game_history::view::values_view(&values))
}

/// The setting `key` as a preset, leaving out `skip`.
async fn setting(app: &App, key: String, name: String, skip: Vec<String>) -> Result<Preset> {
	let held = app.game_history.clone();
	Ok(disk_work(move || {
		let index = held.index()?;
		game_history::preset::preset(&index, &held.store, &key, &name, now(), &skip)
	})
	.await??)
}

/// Puts a setting into the room the way loading a preset does.
#[tauri::command]
pub async fn game_history_apply(
	app: State<'_, App>,
	key: String,
	sections: Sections,
	skip: Vec<String>,
) -> Result<Plan> {
	let preset = setting(&app, key.clone(), key, skip).await?;
	send_to_room(&app, &preset, sections).await
}

#[tauri::command]
pub async fn skirmish_game_history_apply(
	app: State<'_, App>,
	key: String,
	sections: Sections,
	skip: Vec<String>,
) -> Result<Plan> {
	let preset = setting(&app, key.clone(), key, skip).await?;
	put_in_skirmish(&app, preset, sections).await
}

/// Keeps a setting as a preset.
#[tauri::command]
pub async fn game_history_save_preset(
	app: State<'_, App>,
	key: String,
	name: String,
) -> Result<Book> {
	let preset = setting(&app, key, name, Vec::new()).await?;
	Ok(app.presets.put(preset, now())?)
}

#[cfg(test)]
mod tests {
	use lobby_ui::{ServerSnapshot, UserStatusView, UserView};

	use super::*;

	fn user(name: &str, id: Option<u64>) -> UserView {
		UserView {
			name: name.into(),
			country: String::new(),
			user_id: id,
			lobby_client: String::new(),
			status: UserStatusView {
				in_game: false,
				away: false,
				rank: 0,
				moderator: false,
				bot: false,
			},
			battle_status: None,
			battle_id: None,
		}
	}

	#[test]
	fn i_am_every_account_this_machine_logs_in_with() {
		let snapshot = Snapshot {
			servers: vec![
				ServerSnapshot {
					me: Some("tetrisface".into()),
					users: vec![user("tetrisface", Some(4242)), user("Other", Some(7))],
					..ServerSnapshot::default()
				},
				ServerSnapshot {
					me: None,
					users: vec![user("Nobody", Some(9))],
					..ServerSnapshot::default()
				},
			],
			..Snapshot::default()
		};
		let me = identity_of(
			&snapshot,
			&["tetrisface".into(), "alt".into(), String::new()],
		);
		assert_eq!(me.names, ["alt", "tetrisface"]);
		assert_eq!(me.ids, [4242]);
	}
}
