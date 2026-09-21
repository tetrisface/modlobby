//! The local network, as one more server in the list.
//!
//! The settings carry a server called `lan` that nothing resolves. What it
//! points at is decided here: this machine while it hosts a room, or the
//! machine whose room is being joined. The runtime never learns the
//! difference — its session is keyed `lan` either way, and the connector
//! below swaps the address in on the way out.
//!
//! Hosting is the `lan` crate's server run in this process, with our own
//! client logged into it like anybody else's. The one thing the host does
//! that a guest does not is start the engine as the game server, on the
//! script the room writes from itself.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use lobby_runtime::Connector;
use serde::Serialize;
use spring_protocol::{Endpoint, LoginRequest, Security, Transport, TransportError, Way};
use tauri::State;

use crate::commands::{ApiError, LOBBY_VERSION, Result, data_dirs, data_dirs_of};
use crate::state::App;

/// Where `lan` points, the room being hosted, and the ear on the network.
pub struct Lan {
	target: Arc<Mutex<Option<SocketAddr>>>,
	host: tokio::sync::Mutex<Option<Hosted>>,
	/// Started the first time the battle list asks, kept for the run.
	browser: tokio::sync::Mutex<Option<lan::discover::Browser>>,
}

/// A room this machine hosts: the server, and its announcement.
struct Hosted {
	id: u32,
	host: lan::Host,
	_advert: lan::discover::Advert,
}

impl Default for Lan {
	fn default() -> Self {
		Self {
			target: Arc::default(),
			host: tokio::sync::Mutex::new(None),
			browser: tokio::sync::Mutex::new(None),
		}
	}
}

/// A number no other room on the network is likely to announce, and never
/// the battle id the session itself shows, so the two never share a row.
fn room_id() -> u32 {
	let nanos = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.subsec_nanos())
		.unwrap_or(0);
	(std::process::id().wrapping_mul(2_654_435_761) ^ nanos).max(2)
}

impl Lan {
	/// The address the connector reads; handed to it at spawn.
	pub fn target(&self) -> Arc<Mutex<Option<SocketAddr>>> {
		Arc::clone(&self.target)
	}

	fn point_at(&self, addr: Option<SocketAddr>) {
		*self.target.lock().unwrap_or_else(|e| e.into_inner()) = addr;
	}
}

/// The app's way into every server: the plain TCP one, with the `lan` host
/// rewritten to wherever the LAN points right now. Unencrypted, first try:
/// there is no certificate a room in the next room could have.
pub fn connector(target: Arc<Mutex<Option<SocketAddr>>>) -> Connector {
	Arc::new(move |mut endpoint: Endpoint, policy| {
		if spring_protocol::server_id(&endpoint.host) == lan::LAN_ID {
			let Some(addr) = *target.lock().unwrap_or_else(|e| e.into_inner()) else {
				return Box::pin(async {
					Err(TransportError::Unreachable(
						"no room on the LAN is being hosted or joined".into(),
					))
				});
			};
			endpoint.host = addr.ip().to_string();
			endpoint.ports = vec![addr.port()];
			endpoint.allow_plain = true;
			endpoint.preferred = Some(Way {
				port: addr.port(),
				security: Security::None,
				ms: 0,
				pin: None,
			});
		}
		Box::pin(async move { Transport::connect(&endpoint, policy).await })
	})
}

/// The room this machine hosts, for the page that shows it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LanHostView {
	/// As announced on the network, which is how the battle list keys it.
	pub id: u32,
	pub port: u16,
	pub title: String,
	/// Who is waiting for `!accept`.
	pub pending: Vec<String>,
}

/// A room found on the network, for the battle list.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LanRoomView {
	pub id: u32,
	pub address: String,
	pub port: u16,
	pub title: String,
	pub host: String,
	pub engine_version: String,
	pub game: String,
	pub map: String,
	pub players: u32,
	pub max_players: u32,
	pub passworded: bool,
}

/// The name to appear as on the LAN: the LAN row's, else the first account's.
fn lan_name(app: &State<'_, App>) -> String {
	app.settings
		.get()
		.servers
		.iter()
		.find(|entry| entry.is_lan())
		.map(|entry| entry.username.trim().to_owned())
		.filter(|name| !name.is_empty())
		.unwrap_or_else(|| crate::commands::player_name(app))
}

/// Logs the `lan` session in at `addr` as `name`, over from wherever it was.
async fn connect(app: &State<'_, App>, addr: SocketAddr, name: &str) -> Result<()> {
	let _ = app.client.logout(Some(lan::LAN_ID.into())).await;
	app.lan.point_at(Some(addr));
	let endpoint = Endpoint {
		host: lan::LAN_ID.into(),
		ports: vec![addr.port()],
		allow_plain: true,
		preferred: None,
		roots_only: false,
	};
	let request = LoginRequest::new(name, "*", LOBBY_VERSION, app.hardware.lobby_hash.clone());
	app.client.login(endpoint, request).await?;
	// The name is what the LAN row shows, and what the next room is joined as.
	let _ = app.settings.update(|s| {
		if let Some(entry) = s.servers.iter_mut().find(|entry| entry.is_lan()) {
			entry.username = name.to_owned();
		}
	});
	Ok(())
}

/// Opens a room on this machine and sits down in it.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn lan_host(
	app: State<'_, App>,
	title: String,
	password: Option<String>,
	approve: bool,
	max_players: u32,
	engine_version: String,
	game: String,
	map: String,
) -> Result<LanHostView> {
	if !lan_on(&app) {
		return Err(ApiError::new(
			"off",
			"games on the local network are switched off",
		));
	}
	let name = lan_name(&app);
	let password = password.filter(|p| !p.trim().is_empty());
	let policy = match (&password, approve) {
		(Some(password), _) => lan::Policy::Password(password.clone()),
		(None, true) => lan::Policy::Approve,
		(None, false) => lan::Policy::Open,
	};
	let config = lan::Config {
		founder: name.clone(),
		title: if title.trim().is_empty() {
			format!("{name}'s game")
		} else {
			title.trim().to_owned()
		},
		engine_version,
		game,
		map,
		max_players: max_players.clamp(1, 32),
		policy,
	};
	lan_stop(app.clone()).await?;
	// Where this machine keeps its maps, so the room can hand its own over to
	// a member whose search found it nowhere. Resolved per request, because
	// the room's map can change while it is open.
	let files: lan::serve::MapFiles = match data_dirs_of(&app) {
		Some(dirs) => std::sync::Arc::new(move |map: &str| {
			content::Library::new(dirs.clone()).map_archive(map)
		}),
		None => std::sync::Arc::new(|_: &str| None),
	};
	// The usual port first, so a guest typing an address can leave it off;
	// any port when another room on this machine already has it.
	let host = match lan::Host::start(config.clone(), lan::DEFAULT_PORT, files.clone()).await {
		Ok(host) => host,
		Err(_) => lan::Host::start(config, 0, files)
			.await
			.map_err(|err| ApiError::new("io", format!("opening the room: {err}")))?,
	};
	let port = host.port();
	let id = room_id();
	let view = LanHostView {
		id,
		port,
		title: host.room().config().title.clone(),
		pending: Vec::new(),
	};
	let shared = host.shared();
	let advert = lan::discover::Advert::start(move || {
		shared
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.announce(id, port)
	});
	*app.lan.host.lock().await = Some(Hosted {
		id,
		host,
		_advert: advert,
	});
	let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
	connect(&app, addr, &name).await?;
	app.client
		.join_battle(lan::LAN_ID.into(), lan::BATTLE_ID, password)
		.await?;
	// A host waiting for people is not idle, however long it takes them.
	let _ = app.client.keep_awake(true).await;
	Ok(view)
}

/// Starts the game: the engine as its server, on the room as it stands.
#[tauri::command]
pub async fn lan_start(app: State<'_, App>) -> Result<()> {
	let (engine_version, script) = {
		let held = app.lan.host.lock().await;
		let host = held
			.as_ref()
			.map(|hosted| &hosted.host)
			.ok_or_else(|| ApiError::new("input", "no room is being hosted"))?;
		let room = host.room();
		(
			room.config().engine_version.clone(),
			room.host_script().script(),
		)
	};
	app.client
		.launch_hosted(data_dirs(&app)?, engine_version, script)
		.await?;
	Ok(())
}

/// Closes the room this machine hosts, if it hosts one.
#[tauri::command]
pub async fn lan_stop(app: State<'_, App>) -> Result<()> {
	let Some(hosted) = app.lan.host.lock().await.take() else {
		return Ok(());
	};
	hosted.host.stop("the host closed the room");
	let _ = app.client.keep_awake(false).await;
	let _ = app.client.logout(Some(lan::LAN_ID.into())).await;
	app.lan.point_at(None);
	Ok(())
}

/// The room this machine hosts, or nothing.
#[tauri::command]
pub async fn lan_status(app: State<'_, App>) -> Result<Option<LanHostView>> {
	Ok(app.lan.host.lock().await.as_ref().map(|hosted| {
		let room = hosted.host.room();
		LanHostView {
			id: hosted.id,
			port: hosted.host.port(),
			title: room.config().title.clone(),
			pending: room.pending(),
		}
	}))
}

/// Joins the room at `address` (`host` or `host:port`) as a guest.
#[tauri::command]
pub async fn lan_join_address(
	app: State<'_, App>,
	address: String,
	password: Option<String>,
) -> Result<()> {
	let addr = parse_address(&address)?;
	connect(&app, addr, &lan_name(&app)).await?;
	app.client
		.join_battle(lan::LAN_ID.into(), lan::BATTLE_ID, password)
		.await?;
	Ok(())
}

/// Whether the local network is switched on.
///
/// Every socket this file opens is behind this, and not merely behind the
/// front end declining to ask: the first `lan_rooms` is what starts
/// listening, and a listener is what the firewall asks about. Off, modlobby
/// binds nothing and nobody is asked anything.
fn lan_on(app: &App) -> bool {
	app.settings.get().lan.enabled
}

/// Rooms found on the network right now, this machine's own left out.
#[tauri::command]
pub async fn lan_rooms(app: State<'_, App>) -> Result<Vec<LanRoomView>> {
	// Nothing heard rather than a refusal: this is polled, and a poll that
	// answers with an error while the answer is simply "none" is noise.
	if !lan_on(&app) {
		return Ok(Vec::new());
	}
	let mine = app.lan.host.lock().await.as_ref().map(|hosted| hosted.id);
	let mut held = app.lan.browser.lock().await;
	let browser = held.get_or_insert_with(lan::discover::Browser::start);
	Ok(browser
		.rooms(mine)
		.into_iter()
		.map(|found| LanRoomView {
			id: found.info.id,
			address: found.addr.to_string(),
			port: found.info.port,
			title: found.info.title,
			host: found.info.host,
			engine_version: found.info.engine_version,
			game: found.info.game,
			map: found.info.map,
			players: found.info.players,
			max_players: found.info.max_players,
			passworded: found.info.passworded,
		})
		.collect())
}

/// Points `lan` at the room announced as `id` and logs in there, for a join
/// from the battle list. The join itself is the ordinary one that follows.
pub async fn ensure_connected(app: &State<'_, App>, id: u32) -> Result<()> {
	if !lan_on(app) {
		return Err(ApiError::new(
			"off",
			"games on the local network are switched off",
		));
	}
	let found = {
		let mut held = app.lan.browser.lock().await;
		let browser = held.get_or_insert_with(lan::discover::Browser::start);
		browser
			.find(id)
			.ok_or_else(|| ApiError::new("notFound", "that room is no longer on the network"))?
	};
	let addr = SocketAddr::new(found.addr, found.info.port);
	let already = *app.lan.target.lock().unwrap_or_else(|e| e.into_inner()) == Some(addr)
		&& app
			.client
			.snapshot()
			.await
			.ok()
			.and_then(|s| {
				s.servers
					.into_iter()
					.find(|server| server.server == lan::LAN_ID)
			})
			.is_some_and(|server| server.phase == Some(lobby_ui::Phase::Ready));
	if already {
		return Ok(());
	}
	connect(app, addr, &lan_name(app)).await
}

/// `192.168.1.5`, `192.168.1.5:8200`, `[fd12::1]:8200`, `mac.local`.
fn parse_address(text: &str) -> Result<SocketAddr> {
	let text = text.trim();
	if text.is_empty() {
		return Err(ApiError::new("input", "an address is needed"));
	}
	if let Ok(addr) = text.parse::<SocketAddr>() {
		return Ok(addr);
	}
	if let Ok(ip) = text.parse::<IpAddr>() {
		return Ok(SocketAddr::new(ip, lan::DEFAULT_PORT));
	}
	let with_port = if text.contains(':') {
		text.to_owned()
	} else {
		format!("{text}:{}", lan::DEFAULT_PORT)
	};
	std::net::ToSocketAddrs::to_socket_addrs(&with_port)
		.ok()
		.and_then(|mut addrs| addrs.next())
		.ok_or_else(|| {
			ApiError::new(
				"input",
				format!("{text} is not an address anyone answers at"),
			)
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_settings_row_and_the_crate_agree_on_the_name() {
		assert_eq!(settings::model::LAN_HOST, lan::LAN_ID);
	}

	#[test]
	fn an_address_takes_the_usual_port_unless_told() {
		assert_eq!(
			parse_address("192.168.1.5").unwrap(),
			"192.168.1.5:8200".parse().unwrap()
		);
		assert_eq!(
			parse_address(" 192.168.1.5:8201 ").unwrap(),
			"192.168.1.5:8201".parse().unwrap()
		);
		assert_eq!(
			parse_address("[fd12::1]:8200").unwrap(),
			"[fd12::1]:8200".parse().unwrap()
		);
		assert!(parse_address("").is_err());
	}

	/// The session stays keyed `lan`; only the way out changes.
	#[tokio::test]
	async fn the_connector_points_lan_at_the_target_and_nothing_else() {
		let target = Arc::new(Mutex::new(None));
		let connect = connector(Arc::clone(&target));
		let unset = connect(
			Endpoint::new("lan"),
			spring_protocol::ThrottlePolicy::default(),
		)
		.await
		.err()
		.unwrap();
		assert!(matches!(unset, TransportError::Unreachable(_)));
		// A real address, with nothing listening: the error is the socket's,
		// which proves the endpoint was rewritten rather than refused here.
		*target.lock().unwrap() = Some("127.0.0.1:1".parse().unwrap());
		let refused = connect(
			Endpoint::new("LAN"),
			spring_protocol::ThrottlePolicy::default(),
		)
		.await
		.err()
		.unwrap();
		assert!(
			!matches!(refused, TransportError::Unreachable(ref why) if why.contains("LAN")),
			"{refused}"
		);
	}
}

/// The map from the room's own host, when no search on the internet had it.
///
/// The one room this answers for is the one on the local network: anywhere
/// else there is no host holding the file, and a server's rooms are served by
/// the searches that came first. What proves the asking is `ask`'s own
/// credentials, which the runtime keeps and hands over for this one fetch.
pub async fn map_from_host(
	app: &tauri::AppHandle,
	ask: lobby_runtime::Ask,
	say: tokio::sync::mpsc::Sender<recoil::Progress>,
) -> std::result::Result<(), String> {
	use tauri::Manager;
	if ask.server.as_deref() != Some(settings::model::LAN_HOST) {
		return Err("this room has no host to ask".into());
	}
	let state = app
		.try_state::<App>()
		.ok_or_else(|| "the app is not up".to_owned())?;
	let addr = (*state.lan.target.lock().unwrap_or_else(|e| e.into_inner()))
		.ok_or_else(|| "there is no host to ask".to_owned())?;
	let dirs = crate::commands::data_dirs_of(&state)
		.ok_or_else(|| "there is no BAR data directory to write the map into".to_owned())?;
	let maps = dirs.write.join("maps");

	// Blocking sends would hold the transfer up behind the bar; a step lost
	// because the last one has not been drawn yet costs nothing.
	let tell: Box<lan::getmap::Progress> = Box::new(move |current, total| {
		let _ = say.try_send(recoil::Progress { current, total });
	});
	let archive =
		lan::getmap::fetch(addr, &ask.me, &ask.script_password, &maps, tell.as_ref()).await?;
	tracing::info!(%archive, from = %addr, "lan: the host handed over the map");
	Ok(())
}
