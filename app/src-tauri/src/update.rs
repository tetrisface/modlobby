//! Keeping the app current, without getting in the way of opening it.
//!
//! The look is one small request for the release manifest, made once a day
//! when the app opens (if the setting allows) or whenever the version in the
//! nav is clicked. A newer version found puts a button beside it. With
//! `updates.download` on, the look goes on to fetch the installer by itself
//! and stops there, as [`Pending::Downloaded`], so the button only restarts.
//! Otherwise the click downloads the installer and installs it at once,
//! unless a room is joined or a game is running, in which case the download
//! waits as [`Pending::Downloaded`] and the button offers the restart instead.
//!
//! A download nobody restarted into is installed as the app closes
//! ([`install_on_exit`]), with nothing relaunched: the next start is the new
//! version already, and has had nothing to wait for. Not under a game left
//! running, which an installer's window has no business appearing over, nor
//! beside another modlobby, which the Windows installer would close.
//!
//! A download is also kept on disk, under `updates/` beside the settings, for
//! the run that ends without that chance: killed, shut down with the machine,
//! closed over a game. The next start finds it as [`Pending::Stored`] and
//! installs it ahead of the app ([`resume`]): the manifest is asked whether it
//! is still the release to install, the file is held to the signature the
//! manifest carries, and only then is it run. The page draws nothing of the
//! app and logs in nowhere until that is settled. The manifest is asked
//! because `Update::install` checks nothing -- `Update::download` alone
//! verifies the signature -- and the file has been out of this process's
//! hands since it was written.
//!
//! The installer does the restart: on Windows `install` hands over to NSIS
//! and exits this process, and NSIS relaunches the app with the arguments it
//! had, unless the app was closing anyway. On Linux the AppImage is rewritten
//! in place and `install` returns, so a restart is asked for here.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use base64::prelude::*;
use lobby_runtime::orphan;
use minisign_verify::{PublicKey, Signature};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::{ApiError, Result};
use crate::state::App;

/// What this build is, for the corner of the nav.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VersionView {
	/// The number the updater compares: `Cargo.toml`'s, via `CARGO_PKG_VERSION`.
	pub version: &'static str,
	/// The short commit hash, stamped by `build.rs`.
	pub commit: &'static str,
	/// Whether the engine here may be run against somebody's hosted game.
	///
	/// False on macOS, where the only engine that exists is a third-party
	/// build its author asks not be used on the community servers. Talking in
	/// a room costs those servers nothing and is left alone; playing is what
	/// stops. This is the front end's copy of the answer, so it can draw a
	/// room you can watch rather than one whose buttons all fail —
	/// `recoil::refuse_target` is what actually enforces it.
	pub plays_online: bool,
}

#[tauri::command]
pub fn app_version() -> VersionView {
	VersionView {
		version: env!("CARGO_PKG_VERSION"),
		commit: env!("MODLOBBY_COMMIT"),
		plays_online: recoil::may_join_hosted_games(),
	}
}

/// How far along an update is. Emitted on the `app-update` event.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(
	rename_all = "camelCase",
	rename_all_fields = "camelCase",
	tag = "phase"
)]
#[ts(export)]
pub enum UpdateProgress {
	Checking,
	UpToDate,
	/// A newer release exists and nothing has been fetched: the corner's offer
	/// to download and install it.
	Available {
		version: String,
	},
	Downloading {
		#[ts(type = "number")]
		got: u64,
		#[ts(type = "number")]
		total: u64,
	},
	/// Downloaded and waiting. It installs as the app closes; the corner
	/// offers the restart before then, and says what stands in its way.
	Ready {
		version: String,
		/// What restarting now would take away — a room, a running game —
		/// while there is something; `None` once a click would install.
		held_by: Option<String>,
	},
	Failed {
		reason: String,
	},
}

/// An update the look found, and what has been done about it so far.
enum Pending {
	Found(Update),
	Downloaded(Update, Vec<u8>),
	/// An earlier run's download, on disk. No `Update` yet: the manifest has
	/// to be asked again for one before it can be installed.
	Stored {
		version: String,
		path: PathBuf,
	},
}

/// The update between the look and the install, and the directory a
/// download waits in between runs.
pub struct Staged {
	dir: PathBuf,
	held: Mutex<Option<Pending>>,
	/// The version a download was kept as when this run opened: what the
	/// start installs ahead of the app.
	kept: Option<String>,
	/// What that install came to, worked out once. The start asks as soon as
	/// there is a runtime to ask on, and the page waits on the same answer
	/// before it draws the app or logs in.
	resumed: tokio::sync::OnceCell<Result<Option<UpdateProgress>>>,
}

/// What a kept download is called: the version, so a start can tell whether
/// it is still worth anything without opening it.
const STORED_PREFIX: &str = "modlobby-";
const STORED_SUFFIX: &str = ".update";

impl Staged {
	/// Opens `updates/` under the config directory and picks up a download an
	/// earlier run left there. One for the version running now is what that
	/// run installed, and is removed; anything else waits for the manifest
	/// to say whether it is still the release to install.
	pub fn open(config_dir: &Path) -> Self {
		Self::open_as(config_dir, env!("CARGO_PKG_VERSION"))
	}

	fn open_as(config_dir: &Path, running: &str) -> Self {
		let dir = config_dir.join("updates");
		let mut stored = None;
		for path in stored_files(&dir) {
			let version = stored_version(&path);
			if version == running || stored.is_some() {
				remove(&path);
				continue;
			}
			tracing::info!(version, path = %path.display(), "update kept from an earlier run");
			stored = Some((version, path));
		}
		Self {
			dir,
			kept: stored.as_ref().map(|(version, _)| version.clone()),
			held: Mutex::new(stored.map(|(version, path)| Pending::Stored { version, path })),
			resumed: tokio::sync::OnceCell::new(),
		}
	}

	/// The version of a download waiting on disk, if one is.
	pub fn stored_version(&self) -> Option<String> {
		match self.held.lock().expect("staged update").as_ref() {
			Some(Pending::Stored { version, .. }) => Some(version.clone()),
			_ => None,
		}
	}

	/// The version this start installs ahead of the app: the download it
	/// opened on, in a build that updates itself.
	fn resuming(&self) -> Option<&str> {
		self.kept.as_deref().filter(|_| enabled())
	}

	/// Keeps a download for a later run. Losable: a download that cannot be
	/// written is still installable from memory this run, and is fetched
	/// again next time, which is what happened before anything was kept.
	fn keep(&self, version: &str, bytes: &[u8]) {
		let path = self.dir.join(format!(
			"{STORED_PREFIX}{}{STORED_SUFFIX}",
			version.replace(['/', '\\'], "_")
		));
		let tmp = path.with_extension("part");
		let written = std::fs::create_dir_all(&self.dir)
			.and_then(|()| std::fs::write(&tmp, bytes))
			.and_then(|()| std::fs::rename(&tmp, &path));
		match written {
			Ok(()) => {
				tracing::info!(version, path = %path.display(), "update kept for the next start")
			}
			Err(err) => tracing::warn!(%err, path = %path.display(), "could not keep the update"),
		}
	}

	/// Removes every kept download: installed, or no longer the release.
	fn discard(&self) {
		for path in stored_files(&self.dir) {
			remove(&path);
		}
	}
}

fn stored_files(dir: &Path) -> Vec<PathBuf> {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return Vec::new();
	};
	let mut paths: Vec<PathBuf> = entries
		.filter_map(|entry| entry.ok().map(|entry| entry.path()))
		.filter(|path| {
			path.file_name()
				.and_then(|name| name.to_str())
				.is_some_and(|name| {
					name.starts_with(STORED_PREFIX) && name.ends_with(STORED_SUFFIX)
				})
		})
		.collect();
	paths.sort();
	paths
}

fn stored_version(path: &Path) -> String {
	path.file_name()
		.and_then(|name| name.to_str())
		.and_then(|name| name.strip_prefix(STORED_PREFIX))
		.and_then(|name| name.strip_suffix(STORED_SUFFIX))
		.unwrap_or_default()
		.to_owned()
}

fn remove(path: &Path) {
	if let Err(err) = std::fs::remove_file(path) {
		tracing::warn!(%err, path = %path.display(), "could not remove a kept update");
	}
}

/// Whether this build may update itself. Unset means yes in a release and no
/// in a `tauri dev` run, which is always a local build the released one would
/// replace; `0`, `false`, `off` or `no` means no look at all, anything else
/// means look, and the on-demand look says why it will not. For a build that
/// has to stay put — a local one behind the released version, or one under
/// test — or, set on, for a dev run testing the update round trip.
pub const AUTO_UPDATE_ENV: &str = "MODLOBBY_AUTO_UPDATE";

pub fn enabled() -> bool {
	allows(std::env::var_os(AUTO_UPDATE_ENV), !tauri::is_dev())
}

fn allows(value: Option<std::ffi::OsString>, unset: bool) -> bool {
	let Some(value) = value else {
		return unset;
	};
	let value = value.to_string_lossy().trim().to_ascii_lowercase();
	!matches!(value.as_str(), "0" | "false" | "off" | "no")
}

/// How much has to arrive before the front end is told again. An installer
/// is tens of megabytes, so a megabyte is a visible step.
const REPORT_EVERY: u64 = 1024 * 1024;

fn disabled() -> ApiError {
	ApiError::new(
		"update",
		format!("updates are off: {AUTO_UPDATE_ENV} says so, or is unset in a dev run"),
	)
}

/// Looks for a newer release. The answer is `Available` with the version,
/// `UpToDate`, or `Ready` when that version has already been downloaded and
/// is waiting for a restart -- or `Downloading` when `updates.download` sent
/// a fetch after what was found, which [`stage`] carries on with. A completed
/// look is remembered so the daily one knows when it is due.
#[tauri::command]
pub async fn check_update(
	app: State<'_, App>,
	staged: State<'_, Staged>,
	handle: AppHandle,
) -> Result<UpdateProgress> {
	if !enabled() {
		return Err(disabled());
	}

	let say = |progress: UpdateProgress| {
		let _ = handle.emit("app-update", progress);
	};
	say(UpdateProgress::Checking);

	let outcome = match look(&handle).await {
		Ok(found) => {
			app.update_memory.record(SystemTime::now());
			let held_by = busy(&app).await;
			Ok(settle(&staged, found, held_by))
		}
		Err(err) => Err(err),
	};
	// Answered as `Downloading` rather than `Available`, so the button is
	// never offered enabled for the moment before the fetch says so itself: a
	// click then would find the update already taken by `stage`.
	let outcome = outcome.map(|progress| match progress {
		UpdateProgress::Available { .. } if app.settings.get().updates.download => {
			tauri::async_runtime::spawn(stage(handle.clone()));
			UpdateProgress::Downloading { got: 0, total: 0 }
		}
		other => other,
	});

	match &outcome {
		Ok(progress) => say(progress.clone()),
		Err(err) => say(UpdateProgress::Failed {
			reason: err.message.clone(),
		}),
	}
	outcome
}

/// Reconciles what the manifest says with what is held: the same version
/// already downloaded stays downloaded and is `Ready`; anything else the
/// look found replaces it as `Available`; nothing found clears it.
fn settle(staged: &Staged, found: Option<Update>, held_by: Option<&str>) -> UpdateProgress {
	let mut held = staged.held.lock().expect("staged update");
	match (found, held.take()) {
		(None, _) => {
			staged.discard();
			UpdateProgress::UpToDate
		}
		(Some(update), Some(Pending::Downloaded(done, bytes)))
			if done.version == update.version =>
		{
			let version = done.version.clone();
			*held = Some(Pending::Downloaded(done, bytes));
			ready(version, held_by)
		}
		(Some(update), Some(Pending::Stored { version, path })) if version == update.version => {
			*held = Some(Pending::Stored {
				version: version.clone(),
				path,
			});
			ready(version, held_by)
		}
		(Some(update), _) => {
			// Whatever was kept is not this release.
			staged.discard();
			let version = update.version.clone();
			*held = Some(Pending::Found(update));
			UpdateProgress::Available { version }
		}
	}
}

fn ready(version: String, held_by: Option<&str>) -> UpdateProgress {
	UpdateProgress::Ready {
		version,
		held_by: held_by.map(str::to_owned),
	}
}

/// Takes the corner's offer: downloads what the look found and installs it,
/// or stages it as `Ready` when a room or a game would be lost. Installs at
/// once what an earlier click — or an earlier run — already downloaded.
/// Returns only when there is nothing to install: a successful install ends
/// the process.
#[tauri::command]
pub async fn install_update(
	app: State<'_, App>,
	staged: State<'_, Staged>,
	handle: AppHandle,
) -> Result<UpdateProgress> {
	let taken = staged.held.lock().expect("staged update").take();
	let Some(pending) = taken else {
		return Err(ApiError::new(
			"update",
			"no update has been found; look for one first",
		));
	};

	let say = |progress: UpdateProgress| {
		let _ = handle.emit("app-update", progress);
	};

	let (update, bytes) = match pending {
		Pending::Downloaded(update, bytes) => (update, bytes),
		Pending::Found(update) => match download(&update, &say).await {
			Ok(bytes) => {
				staged.keep(&update.version, &bytes);
				(update, bytes)
			}
			Err(err) => {
				// Still found, still on offer; the next click tries again.
				*staged.held.lock().expect("staged update") = Some(Pending::Found(update));
				say(UpdateProgress::Failed {
					reason: err.message.clone(),
				});
				return Err(err);
			}
		},
		Pending::Stored { version, path } => match reopen(&staged, &handle, version, path).await {
			Ok(Reopened::Installable(update, bytes)) => (*update, bytes),
			Ok(Reopened::Otherwise(progress)) => {
				say(progress.clone());
				return Ok(progress);
			}
			Err(err) => {
				say(UpdateProgress::Failed {
					reason: err.message.clone(),
				});
				return Err(err);
			}
		},
	};

	if let Some(held_by) = busy(&app).await {
		let version = update.version.clone();
		*staged.held.lock().expect("staged update") = Some(Pending::Downloaded(update, bytes));
		let progress = ready(version, Some(held_by));
		say(progress.clone());
		return Ok(progress);
	}

	if let Err(err) = install(&handle, &staged, &update, &bytes) {
		say(UpdateProgress::Failed {
			reason: err.message.clone(),
		});
		return Err(err);
	}
	// Reached where the installer does not end the process itself.
	handle.restart()
}

/// Fetches what the look found and keeps it, without installing: fetching by
/// itself stops at `Ready`, because the restart is the user's to ask for.
/// Left unasked, the download is installed as the app closes.
async fn stage(handle: AppHandle) {
	let app = handle.state::<App>();
	let staged = handle.state::<Staged>();
	let say = |progress: UpdateProgress| {
		let _ = handle.emit("app-update", progress);
	};

	let update = {
		let mut held = staged.held.lock().expect("staged update");
		match held.take() {
			Some(Pending::Found(update)) => update,
			other => {
				// A click took it first, or it is already here.
				*held = other;
				return;
			}
		}
	};

	match download(&update, &say).await {
		Ok(bytes) => {
			staged.keep(&update.version, &bytes);
			let version = update.version.clone();
			*staged.held.lock().expect("staged update") = Some(Pending::Downloaded(update, bytes));
			say(ready(version, busy(&app).await));
		}
		Err(err) => {
			// Still found, still on offer: the button fetches it on a click.
			tracing::warn!(reason = %err.message, "update: fetching ahead failed");
			let version = update.version.clone();
			*staged.held.lock().expect("staged update") = Some(Pending::Found(update));
			say(UpdateProgress::Available { version });
		}
	}
}

/// What a start does about updates: installs a download an earlier run kept,
/// ahead of the app, and otherwise looks when `look` says a look is wanted.
/// In that order, so the look finds the kept download settled rather than
/// half taken.
pub async fn startup(handle: AppHandle, look: bool) {
	let _ = resume(&handle).await;
	if look {
		daily(handle).await;
	}
}

/// Installs the download an earlier run kept, ahead of the app. Does not come
/// back when it installs. `None` when nothing was kept, or when this build
/// does not update itself; otherwise what `install_update` answers when it
/// does not install: the manifest moved on, or could not be reached.
///
/// Asked twice and worked out once: by the start, so the manifest is asked
/// before the page has loaded, and by the page, which holds the app and the
/// login back until this has answered.
async fn resume(handle: &AppHandle) -> Result<Option<UpdateProgress>> {
	let staged = handle.state::<Staged>();
	let resumed = staged.resumed.get_or_init(|| async {
		if staged.resuming().is_none() {
			return Ok(None);
		}
		install_update(handle.state(), handle.state(), handle.clone())
			.await
			.map(Some)
	});
	resumed.await.clone()
}

/// The page's wait on [`resume`]: the app is drawn, and the login made, once
/// this has answered. A restart into the new version after either would take
/// the app away as it appeared and spend a second login on the server's count.
#[tauri::command]
pub async fn resume_update(handle: AppHandle) -> Result<Option<UpdateProgress>> {
	resume(&handle).await
}

/// The version the start is installing ahead of the app, for the page that
/// waits on it to say so.
#[tauri::command]
pub fn kept_update(staged: State<'_, Staged>) -> Option<String> {
	staged.resuming().map(str::to_owned)
}

/// The front end raised an error: the app's own failing, so the next start
/// looks for a fix sooner. Kept locally, sent nowhere.
#[tauri::command]
pub fn note_trouble(app: State<'_, App>) {
	app.update_memory.note_trouble();
}

enum Reopened {
	/// Boxed for the size: an `Update` carries the whole manifest response.
	Installable(Box<Update>, Vec<u8>),
	Otherwise(UpdateProgress),
}

/// How long the manifest may take to answer for a kept download. The start
/// waits on this with the app undrawn and nobody logged in, so a slow answer
/// counts as none: the file keeps waiting, and the app opens as it is.
const KEPT_LOOK_DEADLINE: Duration = Duration::from_secs(3);

/// Turns a kept download back into something installable: the manifest for
/// the `Update` (and the signature in it), the file for the bytes, and the
/// one held to the other. A manifest that has moved on makes the kept file
/// worthless, and a file that cannot be read, or is not what the release
/// signed, is fetched again as if never kept.
async fn reopen(
	staged: &Staged,
	handle: &AppHandle,
	version: String,
	path: PathBuf,
) -> Result<Reopened> {
	let asked = tokio::time::timeout(KEPT_LOOK_DEADLINE, look(handle)).await;
	let late = |_| {
		Err(ApiError::new(
			"update",
			"looking for a release: no answer in time",
		))
	};
	let found = match asked.unwrap_or_else(late) {
		Ok(found) => found,
		Err(err) => {
			// Offline, most likely: the file keeps waiting.
			*staged.held.lock().expect("staged update") = Some(Pending::Stored { version, path });
			return Err(err);
		}
	};
	let Some(update) = found else {
		staged.discard();
		return Ok(Reopened::Otherwise(UpdateProgress::UpToDate));
	};
	if update.version != version {
		staged.discard();
		let version = update.version.clone();
		*staged.held.lock().expect("staged update") = Some(Pending::Found(update));
		return Ok(Reopened::Otherwise(UpdateProgress::Available { version }));
	}
	match kept_bytes(handle, &update, &path) {
		Ok(bytes) => Ok(Reopened::Installable(Box::new(update), bytes)),
		Err(reason) => {
			tracing::warn!(reason, path = %path.display(), "kept update unusable; fetching again");
			staged.discard();
			let version = update.version.clone();
			*staged.held.lock().expect("staged update") = Some(Pending::Found(update));
			Ok(Reopened::Otherwise(UpdateProgress::Available { version }))
		}
	}
}

/// A kept download's bytes, once they are shown to be what the release
/// signed. `Update::install` runs whatever it is handed, and this file has
/// been on disk, out of this process's hands, since it was fetched.
fn kept_bytes(
	handle: &AppHandle,
	update: &Update,
	path: &Path,
) -> std::result::Result<Vec<u8>, String> {
	let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
	verify(&bytes, &update.signature, release_key(handle.config()))?;
	Ok(bytes)
}

/// The key releases are signed with, as the updater is configured with it.
fn release_key(config: &tauri::Config) -> &str {
	config
		.plugins
		.0
		.get("updater")
		.and_then(|updater| updater["pubkey"].as_str())
		.unwrap_or_default()
}

/// Holds `bytes` to a release signature, read the way the updater reads one.
fn verify(bytes: &[u8], signature: &str, key: &str) -> std::result::Result<(), String> {
	let key = PublicKey::decode(&minisign_text(key)?).map_err(|err| err.to_string())?;
	let signature = Signature::decode(&minisign_text(signature)?).map_err(|err| err.to_string())?;
	key.verify(bytes, &signature, true)
		.map_err(|err| err.to_string())
}

/// Minisign's own text out of the base64 that the manifest and
/// `tauri.conf.json` carry a signature and a key in.
fn minisign_text(encoded: &str) -> std::result::Result<String, String> {
	let decoded = BASE64_STANDARD
		.decode(encoded)
		.map_err(|err| err.to_string())?;
	String::from_utf8(decoded).map_err(|err| err.to_string())
}

/// The look on opening, when it is due: daily, or sooner after a session
/// that went wrong (see `UpdateMemory::interval`). Quiet about being
/// offline: an update is not something to be told about failing to look
/// for. Not while a download still waits on disk: [`resume`] has just asked
/// the manifest about it and had no answer.
pub async fn daily(handle: AppHandle) {
	let app = handle.state::<App>();
	let staged = handle.state::<Staged>();
	if staged.stored_version().is_some() {
		tracing::debug!("update check: a kept download is still waiting, not looking");
		return;
	}
	let every = app.update_memory.interval();
	if !app.update_memory.due(SystemTime::now(), every) {
		tracing::debug!(
			?every,
			"update check: looked within the interval, not again"
		);
		return;
	}
	match check_update(app, staged, handle.clone()).await {
		Ok(progress) => tracing::info!(?progress, "update check"),
		Err(err) => tracing::info!(reason = %err.message, "update check"),
	}
}

/// The release manifest, compared with this build. One small request.
async fn look(handle: &AppHandle) -> Result<Option<Update>> {
	let updater = handle
		.updater()
		.map_err(|err| ApiError::new("update", err.to_string()))?;
	updater
		.check()
		.await
		.map_err(|err| ApiError::new("update", format!("looking for a release: {err}")))
}

async fn download(update: &Update, say: &impl Fn(UpdateProgress)) -> Result<Vec<u8>> {
	let mut got = 0_u64;
	let mut reported = 0_u64;
	say(UpdateProgress::Downloading { got: 0, total: 0 });
	update
		.download(
			|chunk, total| {
				got += chunk as u64;
				let total = total.unwrap_or(0);
				if got - reported >= REPORT_EVERY {
					reported = got;
					say(UpdateProgress::Downloading { got, total });
				}
			},
			|| {},
		)
		.await
		.map_err(|err| ApiError::new("update", format!("fetching {}: {err}", update.version)))
}

/// What restarting now would take away: a room we are in, a game that is
/// running, an engine we launched that is still alive, or another modlobby.
/// `None` when nothing would be lost. A runtime that cannot answer has
/// nothing to lose.
async fn busy(app: &App) -> Option<&'static str> {
	if let Ok(snapshot) = app.client.snapshot().await {
		if snapshot.servers.iter().any(|s| s.game_running.is_some()) {
			return Some("the game that is running");
		}
		if snapshot.room().is_some() {
			return Some("the room you are in");
		}
	}
	if matches!(app.client.engine_pid().await, Ok(Some(_))) {
		return Some("the engine that is still running");
	}
	closes_another_lobby().then_some("another modlobby that is running")
}

/// Whether installing now would end another modlobby. The Windows installer
/// closes every process run from an executable named as this one is, whatever
/// it is in the middle of; elsewhere an install ends nothing.
fn closes_another_lobby() -> bool {
	cfg!(windows)
		&& std::env::current_exe().is_ok_and(|exe| {
			exe.file_name()
				.is_some_and(|name| another_named(&orphan::processes(), std::process::id(), name))
		})
}

/// Whether a process other than `me` runs from an executable called `name`,
/// in whatever directory and case: how the installer picks what to close.
fn another_named(processes: &[orphan::Process], me: u32, name: &OsStr) -> bool {
	processes.iter().any(|process| {
		let named = process.exe.as_deref().and_then(Path::file_name);
		process.pid != me && named.is_some_and(|other| other.eq_ignore_ascii_case(name))
	})
}

/// Installs the download this run holds, as the app closes: the next start is
/// then the new version already, with nothing to install on its way in.
/// Nothing is relaunched, the app was closing. Not under a game left running,
/// which an installer's window has no business appearing over, nor while
/// another modlobby runs, which the installer would close. Left like that, or
/// after a failure, the next start finds the download where it was kept.
pub fn install_on_exit(handle: &AppHandle, game_running: bool) {
	let Some(staged) = handle.try_state::<Staged>() else {
		return;
	};
	let taken = staged.held.lock().expect("staged update").take();
	let Some(Pending::Downloaded(update, bytes)) = taken else {
		return;
	};
	if game_running || closes_another_lobby() {
		tracing::info!(version = %update.version, game_running, "update: left for the next start");
		return;
	}
	tracing::info!(version = %update.version, "update: installing on the way out");
	let update = update.restart_after_install(false);
	if let Err(err) = install(handle, &staged, &update, &bytes) {
		tracing::warn!(reason = %err.message, "update: not installed on the way out");
	}
}

/// Hands the installer its bytes. On Windows this does not return when it
/// works: NSIS takes over and this process exits, the kept file left for the
/// next start to recognise as its own version and remove. Elsewhere the app
/// was rewritten in place and the kept file has served; what runs next is the
/// caller's to say.
fn install(handle: &AppHandle, staged: &Staged, update: &Update, bytes: &[u8]) -> Result<()> {
	// The exit that follows is not Tauri's, so the exit handler that takes the
	// in-game widget back out of the user's data directory will not run --
	// nor the one that marks the session as ended, without which every
	// update would read as a crash and the updated client would look hourly.
	if let Some(held) = handle.try_state::<crate::InGameHandle>() {
		drop(held.lock().expect("in-game").take());
	}
	if let Some(app) = handle.try_state::<App>() {
		app.update_memory.ended();
	}
	update
		.install(bytes)
		.map_err(|err| ApiError::new("update", format!("installing {}: {err}", update.version)))?;
	staged.discard();
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::ffi::OsStr;

	use super::{
		PublicKey, Staged, allows, another_named, minisign_text, orphan, release_key, verify,
	};

	#[test]
	fn another_lobby_is_any_other_process_run_from_an_executable_named_as_this_one() {
		let process = |pid, exe: &str| orphan::Process {
			pid,
			parent: None,
			started: 0,
			exe: Some(exe.into()),
		};
		let running = [
			process(1, "C:/installed/modlobby-app.exe"),
			process(2, "C:/installed/data/engine/spring.exe"),
			process(3, "C:/built/target/debug/Modlobby-App.exe"),
		];
		let name = OsStr::new("modlobby-app.exe");

		assert!(
			!another_named(&running[..2], 1, name),
			"itself and its game"
		);
		assert!(another_named(&running, 1, name), "the build beside it");
	}

	/// A throwaway key from `tauri signer generate` and its signature over
	/// `SIGNED`, each in the shape `tauri.conf.json` and the manifest carry.
	const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDM0M0ZDNjk4NDVCRTk4MjAKUldRZ21MNUZtTVkvTkRWV05LNGRBTE91Y3BsNlU0eG1TV0FQeWREZXdPY1hvRmN1R2tCZ2xCVmgK";
	const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRZ21MNUZtTVkvTkFYTU01dTZKOENrRGFPbHE3K0g4RUFqODU2ci84dnZFRkd6emlhc0tQcDc5TXpSUkFPb1ZzVFhSRkR1MGFiOHZYNEcrZVh0citELzNPWFFKdWJibHdZPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxMTE5NDUwCWZpbGU6aW5zdGFsbGVyLmJpbgpVN2dtblZJRUYrUGNGYjhNTmwrR0hyVGJ0RENZdzFZL3hpcTUrWTZ4TWlLbmtrN01TTDl6UW9DSlJ5RTdDQTRnREJnQzlVQjRjaDhwZHlxNVV4SUlBdz09Cg==";
	const SIGNED: &[u8] = b"installer";

	#[test]
	fn what_the_release_signed_passes() {
		assert_eq!(verify(SIGNED, SIGNATURE, KEY), Ok(()));
	}

	#[test]
	fn a_kept_download_changed_on_disk_is_refused() {
		assert!(verify(b"installer, and something riding along", SIGNATURE, KEY).is_err());
	}

	#[test]
	fn a_signature_or_key_that_cannot_be_read_refuses_rather_than_panics() {
		assert!(verify(SIGNED, "", KEY).is_err());
		assert!(verify(SIGNED, SIGNATURE, "").is_err());
		assert!(verify(SIGNED, "not base64", KEY).is_err());
	}

	/// A key not found, or not read, would refuse every kept download, and
	/// nothing but a start that always fetches again would say so.
	#[test]
	fn the_key_the_app_is_configured_with_is_found_and_reads_as_one() {
		let config: tauri::Config =
			serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
		let text = minisign_text(release_key(&config)).unwrap();
		assert!(PublicKey::decode(&text).is_ok(), "{text}");
	}

	#[test]
	fn unset_takes_the_build_default() {
		assert!(allows(None, true));
		assert!(!allows(None, false));
	}

	#[test]
	fn anything_but_the_off_words_means_on_even_in_a_dev_run() {
		for value in ["1", "true", "on", "yes", "", "whatever"] {
			assert!(allows(Some(value.into()), false), "{value:?}");
		}
	}

	#[test]
	fn the_four_off_words_mean_off_in_any_case() {
		for value in ["0", "false", "off", "no", " OFF ", "False"] {
			assert!(!allows(Some(value.into()), true), "{value:?}");
		}
	}

	#[test]
	fn a_kept_download_is_found_by_the_next_start() {
		let dir = tempfile::tempdir().unwrap();
		let staged = Staged::open_as(dir.path(), "0.1.10");
		assert_eq!(staged.stored_version(), None, "nothing kept yet");
		assert_eq!(staged.kept, None);

		staged.keep("0.1.11", b"installer");
		assert_eq!(staged.kept, None, "kept for the next start, not this one");
		let restarted = Staged::open_as(dir.path(), "0.1.10");
		assert_eq!(restarted.stored_version(), Some("0.1.11".into()));
		assert_eq!(restarted.kept.as_deref(), Some("0.1.11"));
		assert!(!dir.path().join("updates/modlobby-0.1.11.part").exists());
	}

	#[test]
	fn the_start_that_installed_it_removes_it() {
		let dir = tempfile::tempdir().unwrap();
		Staged::open_as(dir.path(), "0.1.10").keep("0.1.11", b"installer");
		let updated = Staged::open_as(dir.path(), "0.1.11");
		assert_eq!(updated.stored_version(), None);
		assert!(!dir.path().join("updates/modlobby-0.1.11.update").exists());
	}

	#[test]
	fn discarding_leaves_nothing_for_the_next_start() {
		let dir = tempfile::tempdir().unwrap();
		let staged = Staged::open_as(dir.path(), "0.1.10");
		staged.keep("0.1.11", b"installer");
		staged.discard();
		assert_eq!(Staged::open_as(dir.path(), "0.1.10").stored_version(), None);
	}

	#[test]
	fn a_name_that_is_not_a_kept_download_is_left_alone() {
		let dir = tempfile::tempdir().unwrap();
		let updates = dir.path().join("updates");
		std::fs::create_dir_all(&updates).unwrap();
		std::fs::write(updates.join("notes.txt"), "mine").unwrap();
		let staged = Staged::open_as(dir.path(), "0.1.10");
		assert_eq!(staged.stored_version(), None);
		staged.discard();
		assert!(updates.join("notes.txt").exists());
	}
}
