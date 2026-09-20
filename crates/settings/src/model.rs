//! The settings shape. Every field has a default so a partial file is valid;
//! unknown keys are kept on disk and ignored here.

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const SCHEMA_FILE: &str = "settings.schema.json";

/// BAR's lobby server: the one every install starts with.
pub const DEFAULT_HOST: &str = "server4.beyondallreason.info";

/// The host of the servers entry that is not a server: whoever on the local
/// network is hosting a room. Nothing resolves it; the app points it at an
/// address when a room is hosted or joined. The same string as `lan::LAN_ID`.
pub const LAN_HOST: &str = "lan";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Settings {
	/// Points editors at the schema next to the file.
	#[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
	pub schema: Option<String>,
	/// The lobby servers, in the order the app lists them. A fresh install
	/// has BAR's.
	pub servers: Vec<ServerEntry>,
	/// The one server a file from before `servers` named; read to build that
	/// list, never written.
	#[serde(skip_serializing)]
	#[schemars(skip)]
	#[ts(skip)]
	pub server: Server,
	pub account: Account,
	pub connection: Connection,
	pub paths: Paths,
	pub battle_list: BattleList,
	pub chat: Chat,
	pub notifications: Notifications,
	pub overlay: Overlay,
	pub play: Play,
	pub tweaks: Tweaks,
	pub logging: Logging,
	pub updates: Updates,
	pub ui: Ui,
}

impl Settings {
	/// What a fresh install gets: the defaults, BAR's server, and the schema pointer.
	pub fn initial() -> Self {
		Self {
			schema: Some(format!("./{SCHEMA_FILE}")),
			servers: vec![ServerEntry::bar(), ServerEntry::lan()],
			..Self::default()
		}
	}

	/// Gives a list from before there was a LAN its row. A list emptied on
	/// purpose is left empty.
	///
	/// ponytail: put back on every load, so removing just this row does not
	/// stick; a `lan: false` setting if anyone asks for that.
	pub(crate) fn ensure_lan(&mut self) {
		if !self.servers.is_empty() && !self.servers.iter().any(ServerEntry::is_lan) {
			self.servers.push(ServerEntry::lan());
		}
	}

	/// The server list a file from before there was one meant: its one
	/// server, with the account and the channels that went with it.
	pub(crate) fn migrate(&mut self) {
		let server = &self.server;
		self.servers = vec![ServerEntry {
			host: server.host.clone(),
			name: if server.host == DEFAULT_HOST {
				"BAR".into()
			} else {
				String::new()
			},
			ports: vec![server.plain_port, server.tls_port],
			allow_unencrypted: server.encryption == Encryption::None,
			website: None,
			rapid: None,
			maps: None,
			username: self.account.username.clone(),
			channels: self.chat.channels.clone(),
		}];
	}
}

/// A lobby server, and the account on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ServerEntry {
	/// Where it is. The server is known by this, trimmed and lowercased, so
	/// changing it makes a different server: a new account, a new password.
	pub host: String,
	/// What the app calls it; the host when empty.
	pub name: String,
	/// Ports to try, each both encrypted ways (`STLS` and TLS) at once; the
	/// way that answers first is remembered and tried first next time.
	pub ports: Vec<u16>,
	/// Whether an unencrypted connection will do once every encrypted way has
	/// failed. Off unless the server has nothing else: the password and every
	/// message would cross the network readable.
	pub allow_unencrypted: bool,
	/// Where the server's web pages are — a forgotten password is reset
	/// there — when that is not `https://<host>`.
	pub website: Option<String>,
	/// The server's own rapid master index (`https://…/repos.gz`), where the
	/// games its rooms run are published. Without one its games are looked
	/// for in BAR's, which is right for a server running stock BAR. A game is
	/// only ever looked for in its own server's index: a mod's name is never
	/// sent to BAR's servers, nor to any other server's.
	pub rapid: Option<String>,
	/// The server's own map search (`https://…/find`, the springfiles API
	/// pr-downloader speaks), for maps of its own. Asked only for a map that
	/// is not one of BAR's; BAR's search is asked only for those that are.
	pub maps: Option<String>,
	/// The account on this server.
	pub username: String,
	/// Channels to rejoin at login. The server forgets you were in them the
	/// moment you disconnect, so remembering is the client's job — and keeping
	/// it here means you can also just write one in.
	pub channels: Vec<String>,
}

impl ServerEntry {
	/// BAR's own.
	pub fn bar() -> Self {
		Self {
			host: DEFAULT_HOST.into(),
			name: "BAR".into(),
			..Self::default()
		}
	}

	/// The local network. Unencrypted on purpose: the host is in the same
	/// room, and there is no certificate anyone could check. `username` is
	/// the name to appear as; no channels, since there is no server to have
	/// them.
	pub fn lan() -> Self {
		Self {
			host: LAN_HOST.into(),
			name: "LAN".into(),
			ports: vec![8200],
			allow_unencrypted: true,
			channels: Vec::new(),
			..Self::default()
		}
	}

	pub fn is_lan(&self) -> bool {
		self.host.trim().eq_ignore_ascii_case(LAN_HOST)
	}
}

impl Default for ServerEntry {
	fn default() -> Self {
		Self {
			host: String::new(),
			name: String::new(),
			// teiserver's own: plain (and `STLS`) on 8200, TLS on 8201.
			ports: vec![8200, 8201],
			allow_unencrypted: false,
			website: None,
			rapid: None,
			maps: None,
			username: String::new(),
			// Where the server puts everyone, and where the announcements are.
			channels: vec!["main".into()],
		}
	}
}

/// Which teiserver to talk to, as files from before `servers` said it.
///
/// These keys replaced `port` and `tls`, so a file that still carries those
/// is read as the defaults below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Server {
	pub host: String,
	/// How the connection is encrypted, and what it falls back to.
	pub encryption: Encryption,
	/// Where `STLS` upgrades to TLS, and where `none` stays unencrypted.
	pub plain_port: u16,
	/// Where TLS starts from the first byte.
	pub tls_port: u16,
}

impl Default for Server {
	fn default() -> Self {
		Self {
			host: DEFAULT_HOST.into(),
			encryption: Encryption::Stls,
			plain_port: 8200,
			tls_port: 8201,
		}
	}
}

/// Both encrypted ways fall back to the other when the server does not greet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Encryption {
	/// The plain port, upgraded with `STLS`; falls back to the TLS port.
	Stls,
	/// The TLS port; falls back to `STLS` on the plain port.
	Tls,
	/// Unencrypted on the plain port, e.g. a local server without certificates.
	None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Account {
	/// The one account a file from before `servers` named; read to build that
	/// list, never written. Each server keeps its own now.
	#[serde(skip_serializing)]
	#[schemars(skip)]
	#[ts(skip)]
	pub username: String,
	/// Keep passwords in the OS keyring between runs, for every server; they
	/// are never written here.
	pub remember_password: bool,
	/// Log in on startup, to every server with a remembered password. Without
	/// one there is nothing to log in with, so this does nothing on its own.
	pub auto_login: bool,
}

/// Holding on to the server, and letting it go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Connection {
	/// Minutes without a key pressed or a click in the window before the
	/// connection is dropped and not retried; `0` keeps it however long.
	///
	/// A dropped connection otherwise comes back on its own, which is right
	/// for a window in use and wrong for one that was forgotten: it holds a
	/// seat in a room for nobody, and keeps the account logged in from a
	/// machine its owner may have left. A running game never counts as idle,
	/// whatever the lobby window sees. The window stays open, one click from
	/// logging in again.
	pub idle_disconnect_minutes: u32,
}

impl Default for Connection {
	fn default() -> Self {
		Self {
			// Long enough to read a thread of chat without touching anything;
			// short enough that a lobby left overnight is gone by morning.
			idle_disconnect_minutes: 60,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Paths {
	/// The BAR data directory modlobby writes (`engine/`, `games/`, `maps/`);
	/// its own when unset. The launcher's and bar-lobby's installs are read
	/// either way, so their content is never fetched twice.
	pub data_dir: Option<PathBuf>,
}

/// Which rooms the list shows. Chobby words these the other way round, as
/// "Filter out:" checkboxes, where ticking one means seeing less; stated
/// positively, a toggle that is on means that kind of room is in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct BattleList {
	pub show_passworded: bool,
	pub show_locked: bool,
	/// Rooms nobody has joined yet.
	pub show_empty: bool,
	/// Rooms whose game has already started.
	pub show_running: bool,
	/// Narrow the list to rooms with a friend in them. Off by default: it
	/// empties the list for anyone who has not added anybody.
	pub friends_only: bool,
	pub mode: ModeFilter,
	pub sort: BattleSort,
	/// Largest or latest first. Ignored by `BattleSort::Relevance`, which has
	/// a fixed order of its own.
	pub sort_descending: bool,
}

impl Default for BattleList {
	/// Everything, in Chobby's order. Narrowing the list is a choice someone
	/// makes, never the state they are dropped into.
	fn default() -> Self {
		Self {
			show_passworded: true,
			show_locked: true,
			show_empty: true,
			show_running: true,
			friends_only: false,
			mode: ModeFilter::default(),
			sort: BattleSort::default(),
			sort_descending: false,
		}
	}
}

/// Player-versus-what. Read off the room title, which is all the list has:
/// the server only sends a room's AI once you are in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ModeFilter {
	#[default]
	All,
	/// Only rooms that look like they are against AI.
	Pve,
	/// Only rooms that do not.
	Pvp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BattleSort {
	/// Chobby's own order: joinable first, then busy, then locked, then
	/// passworded, with player count deciding inside each band.
	#[default]
	Relevance,
	Players,
	Title,
	Map,
}

/// A sort this build no longer offers — `host` was one until 2026-09-03 —
/// falls back to the default rather than making the whole file unreadable.
impl<'de> Deserialize<'de> for BattleSort {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		Ok(match String::deserialize(deserializer)?.as_str() {
			"players" => BattleSort::Players,
			"title" => BattleSort::Title,
			"map" => BattleSort::Map,
			_ => BattleSort::Relevance,
		})
	}
}

/// What sitting down in a room you just joined should mean.
///
/// Chobby's three, under the same names it gives them
/// (`gui_settings_window.lua:906`): remember what you did last time, or always
/// one or the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JoinAs {
	Remember,
	Spectator,
	Player,
}

/// Playing rather than watching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Play {
	/// Whether joining a room seats you.
	pub join_as: JoinAs,
	/// Whether the engine starts on its own when your room's game does.
	///
	/// On, as it is in Chobby (`gui_settings_window.lua:888`), because a game
	/// you are in starting is the moment you want to be in it — spectating
	/// included, which is the case that otherwise means watching the room and
	/// pressing a button. Only ever fires when the content is already on disk.
	pub auto_launch: bool,
	/// Whether joining a room fetches what it needs: engine, game and map.
	///
	/// On, as bar-lobby and the launcher do it: a room you cannot play is
	/// nothing until its content is here. Off leaves a button in the room for
	/// each fetch, for a metered connection or a disk being kept small.
	pub auto_download: bool,
	/// Whether to ask the pve.bar stats service what a PvE room scores.
	///
	/// On, because the number is the point of looking at a PvE room before
	/// joining it. It sends the map, the settings and the team size to a
	/// third-party service — never a name or an account — so it stays
	/// something that can be turned off.
	pub pve_stats: bool,
	/// What [`JoinAs::Remember`] remembers: whether you played last time.
	/// Written when you take or leave a seat, never chosen directly.
	pub last_was_player: bool,
}

impl Default for Play {
	fn default() -> Self {
		Self {
			join_as: JoinAs::Remember,
			auto_launch: true,
			auto_download: true,
			pve_stats: true,
			// Nothing remembered yet, and this is a lobby.
			last_was_player: true,
		}
	}
}

/// The lobby raised over a running game by a hotkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Overlay {
	pub enabled: bool,
	/// A Tauri accelerator, e.g. `Alt+Shift+L`.
	///
	/// A global hotkey beats the focused window, so a collision does not
	/// merely conflict — it silently eats a game action for as long as a game
	/// is running. The default was checked against BAR's shipped binds: `sc_l`
	/// is taken plain and with Shift (cycling fire state), and the whole game
	/// binds only two Alt+Shift combinations, neither of them this one.
	pub hotkey: String,
	/// Whether hiding the overlay should put the game back in front.
	pub return_focus_to_game: bool,
	/// Whether Escape inside a game should raise the lobby.
	///
	/// The engine gives an outside program no way to see Escape, so this is
	/// the one feature that puts a file of ours in the BAR data directory: a
	/// small widget in `LuaUI/Widgets/`. It draws nothing, it is removed when
	/// modlobby exits, and it leaves the key alone unless modlobby answers —
	/// so a game launched from Chobby behaves exactly as it always did. It is
	/// still someone else's directory, which is why it is a setting.
	pub in_game_escape: bool,
}

impl Default for Overlay {
	fn default() -> Self {
		Self {
			enabled: true,
			hotkey: "Alt+Shift+L".into(),
			return_focus_to_game: true,
			in_game_escape: true,
		}
	}
}

/// How loudly to say that something happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Alert {
	/// Say nothing.
	Off,
	/// A message in the lobby's own corner, which you see when you look.
	Lobby,
	/// A desktop notification and a flashing taskbar entry, and only while
	/// the window is in the background: a toast for something already on
	/// screen is noise, which is the line Chobby draws too. Never the
	/// lobby's corner — that is what `Lobby` is for, and a choice that did
	/// both would not be a choice.
	Desktop,
}

/// Accepts the `true`/`false` this used to be.
///
/// Every setting under `notifications` was a boolean before there was anywhere
/// but the desktop to put one. A file written then must keep working: an
/// unreadable value here is not a field that falls back to its default, it is
/// a settings file that will not parse at all.
impl<'de> Deserialize<'de> for Alert {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		#[serde(untagged)]
		enum Written {
			Named(String),
			Legacy(bool),
		}

		Ok(match Written::deserialize(deserializer)? {
			Written::Legacy(true) => Alert::Desktop,
			Written::Legacy(false) => Alert::Off,
			Written::Named(name) => match name.to_ascii_lowercase().as_str() {
				"off" | "none" | "false" => Alert::Off,
				"lobby" => Alert::Lobby,
				_ => Alert::Desktop,
			},
		})
	}
}

/// What is worth saying something about, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Notifications {
	/// Someone messaged you directly.
	pub private_message: Alert,
	/// Someone said your name in a channel or in your room.
	pub mention: Alert,
	/// Someone rang you.
	pub ring: Alert,
	/// A friend logged in.
	pub friend_online: Alert,
	/// A vote opened in your room.
	pub vote: Alert,
	/// Your room's game started.
	pub game_starting: Alert,
	/// Your room's game finished.
	pub game_ended: Alert,
	/// Silences every kind above, without forgetting how each was set.
	///
	/// Chobby has the same switch (`doNotDisturb`), and it is the one people
	/// reach for: turning seven rows off to get an hour's quiet, and then
	/// remembering how each of them stood, is not a thing anyone does.
	pub do_not_disturb: bool,
}

impl Default for Notifications {
	fn default() -> Self {
		Self {
			// Addressed to you by name, or your game starting or ending
			// around you: worth pulling you back for.
			private_message: Alert::Desktop,
			mention: Alert::Desktop,
			ring: Alert::Desktop,
			game_starting: Alert::Desktop,
			game_ended: Alert::Desktop,
			// True, but not worth taking over the screen for.
			vote: Alert::Lobby,
			// A friend's comings and goings are on the friends list already.
			friend_online: Alert::Off,
			// Off: a lobby that says nothing until it is configured is a
			// lobby that looks broken.
			do_not_disturb: false,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Chat {
	/// Whether to drop the host's machine-readable lines rather than show them.
	///
	/// SPADS rides structured state on battle chat as `BarManager|{…}`, which
	/// this already parses into the room. Chobby hides the same lines behind
	/// the same default, calling it "filter bot chatter"
	/// (`gui_settings_window.lua:1146`).
	pub filter_host_chatter: bool,
	/// Lines kept per room before the oldest are dropped.
	pub max_lines: u32,
	/// The channels a file from before `servers` rejoined; read to build that
	/// list, never written. Each server keeps its own now.
	#[serde(skip_serializing)]
	#[schemars(skip)]
	#[ts(skip)]
	pub channels: Vec<String>,
	/// Rooms left out of the unread count on the Chat tab, by name: a
	/// channel's name, or `@name` for a person — on every server that has
	/// one. A line that names you still counts.
	pub muted: Vec<String>,
}

impl Default for Chat {
	fn default() -> Self {
		Self {
			filter_host_chatter: true,
			max_lines: 3000,
			// Where the server puts everyone, and where the announcements are.
			channels: vec!["main".into()],
			// Everyone is in it, so a count that includes it is never zero.
			muted: vec!["main".into()],
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Tweaks {
	/// A `stylua.toml` to format tweak Lua with; StyLua's defaults when unset.
	pub stylua_config: Option<PathBuf>,
	/// Slot offered when exporting a tweak, e.g. `tweakdefs1`.
	pub default_slot: String,
}

impl Default for Tweaks {
	fn default() -> Self {
		Self {
			stylua_config: None,
			default_slot: "tweakdefs1".into(),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Logging {
	/// A `tracing` filter, e.g. `info,spring::rx=trace`.
	pub filter: String,
}

impl Default for Logging {
	fn default() -> Self {
		Self {
			filter: "info".into(),
		}
	}
}

/// Keeping the app current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Updates {
	/// Whether to look for a newer release once a day, when the app opens.
	///
	/// On, because an out-of-date lobby is one that quietly talks to a server
	/// that has moved on. Looking is one small request for the release
	/// manifest and nothing is installed by itself: a newer version puts a
	/// button in the nav, and a click on it restarts into that version. Off,
	/// the nav still looks when the version is clicked.
	///
	/// After a session that ended badly the look comes round more often for a
	/// while -- hourly at first, easing back to daily -- so a fix reaches a
	/// broken client sooner. Nothing about the failure is sent anywhere.
	pub automatic: bool,
	/// Whether to fetch a newer release as soon as a look finds one.
	///
	/// On. Nothing is installed by itself: the download is kept beside the
	/// settings and the app goes on running the version it started with, so
	/// the offer in the nav is one restart rather than a restart and a wait on
	/// a link that may be slow. Off, nothing is fetched until the button is
	/// clicked, and that click fetches before it restarts.
	pub download: bool,
}

impl Default for Updates {
	fn default() -> Self {
		Self {
			automatic: true,
			download: true,
		}
	}
}

/// How large the interface is drawn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Ui {
	/// Interface scale as a percentage, per screen size.
	///
	/// Keyed by a coarse bucket of the display's pixel size, so a laptop and
	/// the monitor it is plugged into each keep their own answer instead of
	/// the last one used winning — Chobby remembers it the same way
	/// (`chobby/components/configuration.lua:593`). A screen with no entry is
	/// drawn at a size derived from it, which is what makes a large display
	/// legible without anyone opening this file.
	pub scale: BTreeMap<String, u16>,
}

/// The JSON Schema editors use for completion, pretty-printed.
pub fn schema_json() -> String {
	let schema = schemars::schema_for!(Settings);
	serde_json::to_string_pretty(&schema).expect("schema serialises")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn partial_file_fills_in_defaults() {
		let s: Settings = serde_json::from_str(r#"{"chat":{"maxLines":7}}"#).unwrap();
		assert_eq!(s.chat.max_lines, 7);
		assert_eq!(s.server, Server::default());
		assert_eq!(s.tweaks.default_slot, "tweakdefs1");
		assert_eq!(s.connection.idle_disconnect_minutes, 60);
	}

	#[test]
	fn a_file_written_before_alerts_had_places_still_parses() {
		// What every one of these was until there was somewhere other than the
		// desktop to put a notification. A file like this must not be the
		// reason the app refuses to start.
		let s: Settings = serde_json::from_str(
			r#"{"notifications":{"enabled":true,"privateMessage":true,"vote":false}}"#,
		)
		.unwrap();
		assert_eq!(s.notifications.private_message, Alert::Desktop);
		assert_eq!(s.notifications.vote, Alert::Off);
		// A field that was never written keeps its default rather than the
		// reading of whatever the file happened to say about its neighbours.
		assert_eq!(s.notifications.game_ended, Alert::Desktop);
	}

	#[test]
	fn a_place_is_read_by_name_however_it_is_written() {
		let s: Settings = serde_json::from_str(
			r#"{"notifications":{"mention":"lobby","ring":"OFF","vote":"desktop"}}"#,
		)
		.unwrap();
		assert_eq!(s.notifications.mention, Alert::Lobby);
		assert_eq!(s.notifications.ring, Alert::Off);
		assert_eq!(s.notifications.vote, Alert::Desktop);
	}

	#[test]
	fn a_sort_that_was_withdrawn_reads_as_the_default() {
		let s: Settings =
			serde_json::from_str(r#"{"battleList":{"sort":"host","sortDescending":true}}"#)
				.unwrap();
		assert_eq!(s.battle_list.sort, BattleSort::Relevance);
		assert!(s.battle_list.sort_descending);
		let s: Settings = serde_json::from_str(r#"{"battleList":{"sort":"map"}}"#).unwrap();
		assert_eq!(s.battle_list.sort, BattleSort::Map);
	}

	/// `schema/settings.schema.json` is what editors read; keep it in sync.
	/// Regenerate with `SETTINGS_WRITE_SCHEMA=1 cargo test -p settings`.
	#[test]
	fn committed_schema_is_current() {
		let path = concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/schema/",
			"settings.schema.json"
		);
		let current = schema_json();
		if std::env::var_os("SETTINGS_WRITE_SCHEMA").is_some() {
			std::fs::write(path, &current).unwrap();
			return;
		}
		let committed = std::fs::read_to_string(path).unwrap_or_default();
		assert_eq!(
			committed.trim(),
			current.trim(),
			"schema drifted; regenerate"
		);
	}
}
