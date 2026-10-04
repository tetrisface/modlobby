//! Logging in as the app opens, without being asked.
//!
//! Begun as soon as there is an app to do it with, rather than once the page
//! is up: the window and its webview take a fifth of a second to appear, and a
//! connection takes about as long to open, so the two are done side by side
//! and the lobby is on its way before there is a page to show it. The page
//! asks how it went ([`auto_login`]) and what is being waited out
//! ([`login_holds`]); it starts nothing.
//!
//! Never ahead of a kept update. Installing one ends in a restart, and a login
//! on each side of a restart is two inside teiserver's twenty seconds; the
//! caller sees to the order (`lib.rs`).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::join_all;
use serde::Serialize;
use settings::{CredentialStore, Settings, credentials};
use spring_protocol::server_id;
use tauri::{AppHandle, Manager, State};

use crate::commands::log_in;
use crate::state::App;

/// The start's logins: made once, whoever asks and however often, so a page
/// reloaded after a logout is told how the start went and logs nobody back in.
#[derive(Default)]
pub struct StartLogins {
	went: tokio::sync::OnceCell<Vec<LoginFailure>>,
	/// When each login the server's limit is holding back goes out, while it
	/// is held.
	holds: Mutex<BTreeMap<String, SystemTime>>,
}

/// A server the start could not log in to, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LoginFailure {
	pub server: String,
	pub message: String,
}

/// How the start's logins went.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutoLogin {
	/// The settings as the logins left them: a login writes the account back,
	/// and our own write raises no change event.
	pub settings: Settings,
	pub failures: Vec<LoginFailure>,
}

/// Logs in to every server that logs in without being asked. The first call
/// does it; every other waits on that one and is told the same.
pub async fn run(handle: &AppHandle) -> Vec<LoginFailure> {
	let logins = handle.state::<StartLogins>();
	let app = handle.state::<App>();
	let went = logins.went.get_or_init(|| all(&app, &logins));
	went.await.clone()
}

/// Every one of them, side by side: each waits out its own login limit, and
/// one being slow holds up none.
async fn all(app: &App, logins: &StartLogins) -> Vec<LoginFailure> {
	let settings = app.settings.get();
	let going = unattended(&settings, &*app.credentials);
	tracing::debug!(
		ms = crate::since_start(),
		servers = going.len(),
		"startup: logging in"
	);
	let attempts = going
		.into_iter()
		.map(|(server, username)| one(app, logins, server, username, settings.account.auto_login));
	join_all(attempts).await.into_iter().flatten().collect()
}

/// The servers logged in to without being asked, and who as: set to log in at
/// startup, with an account, and a password kept for it. A keyring that
/// cannot be read has no password to give.
fn unattended(settings: &Settings, kept: &dyn CredentialStore) -> Vec<(String, String)> {
	settings
		.servers
		.iter()
		.filter(|entry| entry.logs_in_at_start(&settings.account))
		.map(|entry| (server_id(&entry.host), entry.username.trim().to_owned()))
		.filter(|(server, username)| {
			!username.is_empty()
				&& matches!(credentials::password(kept, server, username), Ok(Some(_)))
		})
		.collect()
}

/// One login, once the server's limit allows it.
///
/// teiserver refuses a login within twenty seconds of the account's last, and
/// that clock is kept across restarts, so the start after an update or a
/// rebuild arrives here already held. Waiting it out beats being refused: a
/// refusal starts the twenty seconds again.
async fn one(
	app: &App,
	logins: &StartLogins,
	server: String,
	username: String,
	auto_login: bool,
) -> Option<LoginFailure> {
	let asked = SystemTime::now();
	if let Some(wait) = app.login_guard.wait(&server, asked) {
		// A second past the server's own count, so the allowance has lapsed.
		let wait = wait + Duration::from_secs(1);
		hold(logins, &server, Some(asked + wait));
		tokio::time::sleep(wait).await;
		hold(logins, &server, None);
	}
	// No password given: the keyring's is used. The account's own answer goes
	// back as it was, since a login writes it: a server that logs in at
	// startup on its own say turns it on for no other.
	let went = log_in(app, &server, username, None, true, auto_login).await;
	went.err().map(|err| LoginFailure {
		server,
		message: err.message,
	})
}

fn hold(logins: &StartLogins, server: &str, until: Option<SystemTime>) {
	let mut holds = logins.holds.lock().expect("login holds");
	match until {
		Some(until) => holds.insert(server.to_owned(), until),
		None => holds.remove(server),
	};
}

/// How the start's logins went, once they are over.
#[tauri::command]
pub async fn auto_login(handle: AppHandle) -> AutoLogin {
	let failures = run(&handle).await;
	AutoLogin {
		settings: handle.state::<App>().settings.get(),
		failures,
	}
}

/// When each of the start's logins that is being held back goes out, by
/// server: milliseconds since the epoch, which is what the page's clock
/// counts in.
#[tauri::command]
pub fn login_holds(logins: State<'_, StartLogins>) -> BTreeMap<String, u64> {
	let holds = logins.holds.lock().expect("login holds");
	holds
		.iter()
		.map(|(server, until)| {
			let since_epoch = until.duration_since(UNIX_EPOCH).unwrap_or_default();
			(server.clone(), since_epoch.as_millis() as u64)
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use settings::MemoryStore;
	use settings::model::ServerEntry;
	use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
	use tokio::net::{TcpListener, TcpStream};

	use super::*;

	const BAR: &str = "server4.beyondallreason.info";
	const OTHER: &str = "server.example.com";

	fn server(host: &str, username: &str, auto_login: Option<bool>) -> ServerEntry {
		ServerEntry {
			host: host.into(),
			username: username.into(),
			auto_login,
			..ServerEntry::bar()
		}
	}

	/// Settings that remember passwords and log in at startup, unless told
	/// otherwise, with the given servers.
	fn remembered(servers: Vec<ServerEntry>) -> Settings {
		let mut settings = Settings::default();
		settings.account.remember_password = true;
		settings.account.auto_login = true;
		settings.servers = servers;
		settings
	}

	fn kept(accounts: &[(&str, &str)]) -> MemoryStore {
		let store = MemoryStore::default();
		for (server, username) in accounts {
			credentials::keep(&store, server, username, "secret").unwrap();
		}
		store
	}

	fn both() -> Vec<ServerEntry> {
		vec![server(BAR, "me", None), server(OTHER, "other", None)]
	}

	#[test]
	fn every_server_whose_password_is_kept_is_logged_in_to() {
		let store = kept(&[(BAR, "me"), (OTHER, "other")]);
		assert_eq!(
			unattended(&remembered(both()), &store),
			[(BAR.into(), "me".into()), (OTHER.into(), "other".into())]
		);
	}

	#[test]
	fn a_server_whose_password_is_not_kept_is_passed_over() {
		let store = kept(&[(OTHER, "other")]);
		assert_eq!(
			unattended(&remembered(both()), &store),
			[(OTHER.into(), "other".into())]
		);
	}

	#[test]
	fn nothing_goes_out_without_a_remembered_password_or_an_account() {
		let store = kept(&[(BAR, "me"), (OTHER, "other")]);

		let mut forgetful = remembered(both());
		forgetful.account.remember_password = false;
		assert_eq!(unattended(&forgetful, &store), []);

		let mut asked = remembered(both());
		asked.account.auto_login = false;
		assert_eq!(unattended(&asked, &store), []);

		let nobody = remembered(vec![server(BAR, "   ", None)]);
		assert_eq!(unattended(&nobody, &store), []);
	}

	#[test]
	fn a_servers_own_answer_goes_before_the_accounts() {
		let store = kept(&[(BAR, "me"), (OTHER, "other")]);

		let mut one_on = remembered(vec![
			server(BAR, "me", Some(true)),
			server(OTHER, "other", None),
		]);
		one_on.account.auto_login = false;
		assert_eq!(unattended(&one_on, &store), [(BAR.into(), "me".into())]);

		let one_off = remembered(vec![
			server(BAR, "me", Some(false)),
			server(OTHER, "other", None),
		]);
		assert_eq!(
			unattended(&one_off, &store),
			[(OTHER.into(), "other".into())]
		);
	}

	/// A lobby without encryption: it greets, takes any login and says the
	/// flood is over. Whatever else opens the conversation -- `STLS`, a TLS
	/// hello -- is hung up on, as such a server does to the encrypted ways in.
	async fn plain_lobby(mut socket: TcpStream) {
		let (read, mut write) = socket.split();
		let _ = write.write_all(b"TASSERVER 0.38 * 8201 0\n").await;
		let mut lines = BufReader::new(read).lines();
		let mut logged_in = false;
		while let Ok(Some(line)) = lines.next_line().await {
			let Some(login) = line.strip_prefix("LOGIN ") else {
				if logged_in {
					continue;
				}
				return;
			};
			let name = login.split(' ').next().unwrap_or_default();
			let accepted = format!("ACCEPTED {name}\nLOGININFOEND\n");
			let _ = write.write_all(accepted.as_bytes()).await;
			logged_in = true;
		}
	}

	/// The port of a local listener serving every connection as `plain_lobby`.
	async fn listening() -> u16 {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let port = listener.local_addr().unwrap().port();
		tokio::spawn(async move {
			while let Ok((socket, _)) = listener.accept().await {
				tokio::spawn(plain_lobby(socket));
			}
		});
		port
	}

	/// The whole way, with nothing but the app to do it: the settings say
	/// where, the kept password says as whom, and nobody is asked.
	#[test]
	fn the_start_logs_in_with_the_kept_password() {
		const LOCAL: &str = "127.0.0.1";
		let dir = tempfile::tempdir().unwrap();
		let port = tauri::async_runtime::block_on(listening());
		let settings = settings::Store::open(dir.path()).unwrap();
		settings
			.update(|settings| {
				settings.account.remember_password = true;
				settings.account.auto_login = true;
				settings.servers.push(ServerEntry {
					builtin: None,
					ports: vec![port],
					allow_unencrypted: true,
					..server(LOCAL, "me", None)
				});
			})
			.unwrap();
		let mut app = App::open(settings);
		app.credentials = Arc::new(kept(&[(LOCAL, "me")]));
		let logins = StartLogins::default();

		let failures = tauri::async_runtime::block_on(all(&app, &logins));

		assert_eq!(failures, []);
		let snapshot = tauri::async_runtime::block_on(app.client.snapshot()).unwrap();
		let session = snapshot.servers.iter().find(|held| held.server == LOCAL);
		assert_eq!(session.and_then(|held| held.me.as_deref()), Some("me"));
		assert!(logins.holds.lock().unwrap().is_empty());
	}
}
