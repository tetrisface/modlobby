//! The file on disk: `settings.jsonc` beside `settings.schema.json`.
//! Reads go through a JSONC parser; writes edit the concrete syntax tree so
//! the user's comments and layout survive.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use jsonc_parser::cst::{CstInputValue, CstNode, CstObject, CstRootNode};
use jsonc_parser::{ParseOptions, parse_to_serde_value};
use serde_json::Value;

use crate::model::{SCHEMA_FILE, Settings, schema_json};

pub const FILE_NAME: &str = "settings.jsonc";

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("{path}: {source}")]
	Io {
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},
	#[error("{path}: {message}")]
	Invalid { path: PathBuf, message: String },
}

/// `MODLOBBY_CONFIG_DIR`, else the platform's per-user config dir
/// (`%APPDATA%\modlobby\config` on Windows, `~/.config/modlobby` on Linux).
pub fn config_dir() -> PathBuf {
	if let Some(dir) = std::env::var_os("MODLOBBY_CONFIG_DIR") {
		return PathBuf::from(dir);
	}
	directories::ProjectDirs::from("", "", "modlobby")
		.map(|dirs| dirs.config_dir().to_path_buf())
		.unwrap_or_else(|| PathBuf::from("."))
}

/// The settings file plus the in-memory copy the app works from.
#[derive(Debug, Clone)]
pub struct Store {
	dir: PathBuf,
	current: Arc<Mutex<Settings>>,
	/// Hash of what we wrote last, so the watcher can tell our writes from the user's.
	last_written: Arc<Mutex<Option<u64>>>,
	/// What was done at this start about a file that did not parse.
	recovered: Option<String>,
}

impl Store {
	/// Creates the directory, the template and the schema on first run, then
	/// loads. A file this build cannot parse -- a hand edit gone wrong, a type
	/// a newer build changed -- does not stop the app: it is kept beside the
	/// one started from (see `recover`), and [`Store::recovered`] says so. One
	/// that cannot be read at all is left alone, and is the error.
	pub fn open(dir: impl Into<PathBuf>) -> Result<Self, Error> {
		let dir = dir.into();
		std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
			path: dir.clone(),
			source,
		})?;
		let path = dir.join(FILE_NAME);
		if !path.exists() {
			write_atomic(&path, &template())?;
		}
		let schema_path = dir.join(SCHEMA_FILE);
		let schema = schema_json();
		// For editors only: one that cannot be written costs completion, not a start.
		if std::fs::read_to_string(&schema_path).ok().as_deref() != Some(schema.as_str())
			&& let Err(err) = write_atomic(&schema_path, &schema)
		{
			tracing::warn!(%err, "settings: the schema was not written");
		}
		let (settings, recovered) = match load(&path) {
			Ok(settings) => (settings, None),
			Err(invalid @ Error::Invalid { .. }) => {
				let (settings, told) = recover(&dir, &invalid)?;
				tracing::warn!(%told, "settings: recovered");
				(settings, Some(told))
			}
			Err(unreadable) => return Err(unreadable),
		};
		Ok(Self {
			dir,
			current: Arc::new(Mutex::new(settings)),
			last_written: Arc::new(Mutex::new(None)),
			recovered,
		})
	}

	/// What was done at this start about a file that did not parse, to tell
	/// the user; `None` when it did.
	pub fn recovered(&self) -> Option<&str> {
		self.recovered.as_deref()
	}

	pub fn dir(&self) -> &Path {
		&self.dir
	}

	pub fn path(&self) -> PathBuf {
		self.dir.join(FILE_NAME)
	}

	pub fn get(&self) -> Settings {
		self.current.lock().expect("settings lock").clone()
	}

	/// Applies `change` to the file as minimal edits — comments, order and
	/// unrelated keys stay as the user left them — and to the in-memory copy.
	pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings, Error> {
		let path = self.path();
		let before = self.get();
		let mut after = before.clone();
		change(&mut after);
		// What a change implies, whoever made it: the servers every install
		// has, the LAN's row for its switch, a server's startup login for the
		// account's.
		after.ensure_builtins();
		after.ensure_lan();
		after.follow_auto_login(&before);
		if after == before {
			return Ok(after);
		}
		let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
			path: path.clone(),
			source,
		})?;
		let root =
			CstRootNode::parse(&text, &ParseOptions::default()).map_err(|err| Error::Invalid {
				path: path.clone(),
				message: err.to_string(),
			})?;
		let object = root.object_value_or_set();
		apply_changes(
			&object,
			&to_value(&before),
			&to_value(&after),
			&as_read(&text),
		);
		let edited = root.to_string();
		// A backup of the last user-visible version before we touch it.
		let _ = std::fs::copy(&path, self.dir.join(format!("{FILE_NAME}.bak")));
		write_atomic(&path, &edited)?;
		*self.last_written.lock().expect("hash lock") = Some(hash(&edited));
		*self.current.lock().expect("settings lock") = after.clone();
		Ok(after)
	}

	/// Re-reads the file after an external change. `Ok(None)` means it was our own write.
	pub(crate) fn reload(&self) -> Result<Option<Settings>, Error> {
		let path = self.path();
		let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
			path: path.clone(),
			source,
		})?;
		if *self.last_written.lock().expect("hash lock") == Some(hash(&text)) {
			return Ok(None);
		}
		let settings = parse(&path, &text)?;
		*self.current.lock().expect("settings lock") = settings.clone();
		Ok(Some(settings))
	}
}

pub fn load(path: &Path) -> Result<Settings, Error> {
	let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
		path: path.to_path_buf(),
		source,
	})?;
	parse(path, &text)
}

/// Keeps a file this build cannot parse beside the one it starts from: the
/// backup of before the last write when that parses, else the defaults.
/// Answers with the settings and what to tell the user.
fn recover(dir: &Path, invalid: &Error) -> Result<(Settings, String), Error> {
	let path = dir.join(FILE_NAME);
	let seconds = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |since| since.as_secs());
	let kept = dir.join(format!("{FILE_NAME}.broken-{seconds}"));
	std::fs::rename(&path, &kept).map_err(|source| Error::Io {
		path: path.clone(),
		source,
	})?;
	let backup = dir.join(format!("{FILE_NAME}.bak"));
	let restored = std::fs::read_to_string(&backup)
		.ok()
		.and_then(|text| Some((parse(&backup, &text).ok()?, text)));
	let ((settings, text), from) = match restored {
		Some(restored) => (restored, "the version from before its last change"),
		None => (
			(Settings::initial(), template()),
			"the defaults; saved passwords are still in the OS keyring",
		),
	};
	write_atomic(&path, &text)?;
	let told = format!(
		"modlobby could not read its settings file ({invalid}). It was kept as {} and modlobby started from {from}.",
		kept.display()
	);
	Ok((settings, told))
}

/// The file as this build reads it, with nothing implied added (no
/// `ensure_*`): a list `apply_changes` edits item by item must match it.
/// `Null` when it does not read, which leaves every list to be written whole.
fn as_read(text: &str) -> Value {
	parse_to_serde_value::<Value>(text, &ParseOptions::default())
		.ok()
		.and_then(|value| serde_json::from_value::<Settings>(value).ok())
		.map_or(Value::Null, |settings| to_value(&settings))
}

/// A file from before there was a server list is read as the list it
/// meant. Only in memory: the file keeps the old keys until the list itself
/// is changed, and then gains `servers` beside them.
fn parse(path: &Path, text: &str) -> Result<Settings, Error> {
	let invalid = |message: String| Error::Invalid {
		path: path.to_path_buf(),
		message,
	};
	let value: Value = parse_to_serde_value(text, &ParseOptions::default())
		.map_err(|err| invalid(err.to_string()))?;
	let listed = value.get("servers").is_some();
	let mut settings: Settings =
		serde_json::from_value(value).map_err(|err| invalid(err.to_string()))?;
	if !listed {
		settings.migrate();
	}
	settings.ensure_builtins();
	settings.ensure_lan();
	Ok(settings)
}

/// The first-run file: a header for humans, then the defaults.
pub fn template() -> String {
	let body = serde_json::to_string_pretty(&Settings::initial()).expect("settings serialise");
	format!(
		"// modlobby settings. Edit freely: the app reloads this file when it changes and\n\
         // keeps your comments when it writes. Passwords never live here; they are in\n\
         // the OS keyring. \"$schema\" gives your editor completion and documentation.\n\
         {body}\n"
	)
}

/// Walks both trees; a leaf that differs is set in place, a key that vanished
/// is removed. A list is written whole -- unless `read`, the file as this
/// build reads it, shows its items line up with `before`'s one for one: then
/// each item is walked like an object, so the keys and values in it this
/// build has no name for, a newer build's, stay as they were.
fn apply_changes(object: &CstObject, before: &Value, after: &Value, read: &Value) {
	let (Value::Object(before), Value::Object(after)) = (before, after) else {
		return;
	};
	for (key, new) in after {
		let old = before.get(key);
		if old == Some(new) {
			continue;
		}
		let read = read.get(key).unwrap_or(&Value::Null);
		if let (Some(Value::Array(old)), Value::Array(new), Value::Array(read)) = (old, new, read)
			&& let Some(items) = items_in_step(object, key, old, new, read)
		{
			for (item, (old, (new, read))) in items.iter().zip(old.iter().zip(new.iter().zip(read)))
			{
				apply_changes(item, old, new, read);
			}
			continue;
		}
		match (old, new) {
			(Some(Value::Object(_)), Value::Object(_)) => {
				let child = object.object_value_or_set(key);
				apply_changes(&child, old.expect("checked"), new, read);
			}
			_ => match object.get(key) {
				Some(prop) => prop.set_value(input_value(new)),
				None => {
					object.append(key, input_value(new));
				}
			},
		}
	}
	for key in before.keys() {
		if !after.contains_key(key)
			&& let Some(prop) = object.get(key)
		{
			prop.remove();
		}
	}
}

/// The list under `key` as objects to walk one by one, when that is safe:
/// as long before as after, every item an object, and the file's items the
/// ones `before` holds, in order. Anything else -- an item added or taken
/// away, a list this build reordered or read short -- is written whole.
fn items_in_step(
	object: &CstObject,
	key: &str,
	old: &[Value],
	new: &[Value],
	read: &[Value],
) -> Option<Vec<CstObject>> {
	let objects = |items: &[Value]| items.iter().all(Value::is_object);
	if old.len() != new.len() || read != old || !objects(old) || !objects(new) {
		return None;
	}
	let items = object
		.array_value(key)?
		.elements()
		.iter()
		.map(CstNode::as_object)
		.collect::<Option<Vec<_>>>()?;
	(items.len() == old.len()).then_some(items)
}

fn input_value(value: &Value) -> CstInputValue {
	match value {
		Value::Null => CstInputValue::Null,
		Value::Bool(b) => CstInputValue::Bool(*b),
		Value::Number(n) => CstInputValue::Number(n.to_string()),
		Value::String(s) => CstInputValue::String(s.clone()),
		Value::Array(items) => CstInputValue::Array(items.iter().map(input_value).collect()),
		Value::Object(map) => CstInputValue::Object(
			map.iter()
				.map(|(k, v)| (k.clone(), input_value(v)))
				.collect(),
		),
	}
}

fn to_value(settings: &Settings) -> Value {
	serde_json::to_value(settings).expect("settings serialise")
}

fn hash(text: &str) -> u64 {
	let mut hasher = DefaultHasher::new();
	text.hash(&mut hasher);
	hasher.finish()
}

/// Temp file + rename, so a crash never leaves a half-written settings file.
fn write_atomic(path: &Path, text: &str) -> Result<(), Error> {
	let tmp = path.with_extension("tmp");
	let io = |source| Error::Io {
		path: path.to_path_buf(),
		source,
	};
	std::fs::write(&tmp, text).map_err(io)?;
	std::fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn first_run_writes_template_and_schema_then_round_trips() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::open(dir.path()).unwrap();
		assert!(dir.path().join(FILE_NAME).is_file());
		assert!(dir.path().join(SCHEMA_FILE).is_file());
		assert_eq!(store.get(), Settings::initial());

		let updated = store.update(|s| s.chat.max_lines = 42).unwrap();
		assert_eq!(updated.chat.max_lines, 42);
		assert_eq!(load(&store.path()).unwrap().chat.max_lines, 42);
		assert_eq!(Store::open(dir.path()).unwrap().get().chat.max_lines, 42);
	}

	#[test]
	fn user_comments_and_order_survive_app_writes() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::open(dir.path()).unwrap();
		let hand_edited = "// my notes\n{\n  // trailing commas are fine,\n  \"chat\": { \"maxLines\": 9 }, // keep me\n  \"server\": { \"plainPort\": 8200, \"encryption\": \"none\" },\n}\n";
		std::fs::write(store.path(), hand_edited).unwrap();
		let reloaded = store.reload().unwrap().unwrap();
		assert!(reloaded.servers[0].allow_unencrypted);

		store
			.update(|s| {
				s.chat.max_lines = 10;
				s.servers[0].username = "alice".into();
			})
			.unwrap();
		let text = std::fs::read_to_string(store.path()).unwrap();
		assert!(text.starts_with("// my notes"));
		assert!(text.contains("// keep me"));
		assert!(text.contains("\"maxLines\": 10"));
		assert!(text.contains("\"username\": \"alice\""));
		// The old key is left as it was: nothing writes it any more.
		assert!(text.contains("\"plainPort\": 8200"));
		let parsed = load(&store.path()).unwrap();
		assert_eq!(parsed.servers[0].username, "alice");
		assert!(parsed.servers[0].allow_unencrypted);
	}

	/// The owner's file of 2026-09-19, in the shape it had before servers.
	const BEFORE_SERVERS: &str = r#"{
      "server": {
        "host": "server4.beyondallreason.info",
        "port": 8201,
        "tls": true,
        "encryption": "stls",
        // "host": "localhost",
      },
      "account": { "username": "tetrisface", "rememberPassword": true, "autoLogin": true },
      "chat": { "channels": ["main", "newbies"] },
    }"#;

	#[test]
	fn a_file_from_before_servers_is_read_as_its_one_server() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join(FILE_NAME);
		std::fs::write(&path, BEFORE_SERVERS).unwrap();
		let settings = load(&path).unwrap();
		assert_eq!(
			settings.servers,
			vec![
				crate::model::ServerEntry {
					username: "tetrisface".into(),
					channels: vec!["main".into(), "newbies".into()],
					..crate::model::ServerEntry::bar()
				},
				crate::model::ServerEntry::mods(),
				crate::model::ServerEntry::recoil(),
				// No local network: it is off until it is asked for, and a
				// file from before it existed never asked.
			]
		);
		assert!(settings.account.remember_password && settings.account.auto_login);
	}

	#[test]
	fn nothing_is_written_until_the_server_list_itself_changes() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::open(dir.path()).unwrap();
		std::fs::write(store.path(), BEFORE_SERVERS).unwrap();
		store.reload().unwrap().unwrap();

		store.update(|s| s.chat.max_lines = 7).unwrap();
		let text = std::fs::read_to_string(store.path()).unwrap();
		assert!(
			!text.contains("\"servers\""),
			"an unrelated change leaves the old shape"
		);
		assert!(
			text.contains("// \"host\": \"localhost\""),
			"and its comments"
		);

		store.update(|s| s.servers[0].name = "Main".into()).unwrap();
		let text = std::fs::read_to_string(store.path()).unwrap();
		assert!(text.contains("\"servers\""));
		assert!(text.contains("\"tetrisface\""));
		assert!(text.contains("\"port\": 8201"), "the old keys stay, unread");
		let reread = load(&store.path()).unwrap();
		assert_eq!(reread.servers[0].name, "Main");
		assert_eq!(reread.servers[0].username, "tetrisface");
	}

	#[test]
	fn an_emptied_list_still_has_the_servers_every_install_has() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join(FILE_NAME);
		std::fs::write(
			&path,
			r#"{ "servers": [{ "host": "mods.example" }], "server": { "host": "x" } }"#,
		)
		.unwrap();
		let hosts: Vec<String> = load(&path)
			.unwrap()
			.servers
			.into_iter()
			.map(|entry| entry.host)
			.collect();
		assert_eq!(
			hosts,
			[
				"server4.beyondallreason.info",
				"server.pve.bar",
				"lobby.recoilengine.org",
				"mods.example"
			]
		);
	}

	#[test]
	fn the_mods_servers_first_name_is_replaced_but_not_a_name_of_the_users_own() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join(FILE_NAME);
		std::fs::write(
			&path,
			r#"{ "servers": [
				{ "builtin": "mods", "host": "server.pve.bar", "name": "pve.bar" }
			] }"#,
		)
		.unwrap();
		assert_eq!(load(&path).unwrap().servers[1].name, "modserver");

		std::fs::write(
			&path,
			r#"{ "servers": [
				{ "builtin": "mods", "host": "server.pve.bar", "name": "mine" }
			] }"#,
		)
		.unwrap();
		let names: Vec<String> = load(&path)
			.unwrap()
			.servers
			.into_iter()
			.map(|entry| entry.name)
			.collect();
		assert_eq!(names, ["BAR", "mine", "Recoil Official"]);
	}

	#[test]
	fn the_mods_servers_first_ports_are_replaced_but_not_ports_of_the_users_own() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join(FILE_NAME);
		let ports = |json: &str| {
			std::fs::write(&path, json).unwrap();
			load(&path).unwrap().servers[1].ports.clone()
		};
		let entry = |ports: &str| {
			format!(
				r#"{{ "servers": [ {{ "builtin": "mods", "host": "server.pve.bar", "ports": {ports} }} ] }}"#
			)
		};
		assert_eq!(ports(&entry("[8200, 8201]")), [8201]);
		assert_eq!(ports(&entry("[8200]")), [8200]);
	}

	#[test]
	fn the_old_port_and_tls_keys_fall_back_to_the_defaults() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::open(dir.path()).unwrap();
		std::fs::write(
			store.path(),
			"{ \"server\": { \"port\": 8201, \"tls\": true } }",
		)
		.unwrap();
		let reloaded = store.reload().unwrap().unwrap();
		assert_eq!(
			reloaded.servers,
			vec![
				crate::model::ServerEntry::bar(),
				crate::model::ServerEntry::mods(),
				crate::model::ServerEntry::recoil()
			]
		);
	}

	/// A file this build cannot parse is kept, and the app starts from the
	/// backup of before the last change -- and says so.
	#[test]
	fn a_file_that_does_not_parse_is_kept_and_the_backup_used() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::open(dir.path()).unwrap();
		store.update(|s| s.chat.max_lines = 5).unwrap();
		store.update(|s| s.chat.max_lines = 6).unwrap();
		let broken = "{ \"chat\": { \"maxLines\": \"many\" } }";
		std::fs::write(store.path(), broken).unwrap();

		let reopened = Store::open(dir.path()).unwrap();
		assert_eq!(reopened.get().chat.max_lines, 5, "the backup");
		assert!(
			reopened
				.recovered()
				.unwrap()
				.contains("before its last change")
		);
		assert_eq!(
			load(&reopened.path()).unwrap().chat.max_lines,
			5,
			"and it reads again"
		);
		let kept: Vec<_> = std::fs::read_dir(dir.path())
			.unwrap()
			.map(|entry| entry.unwrap().path())
			.filter(|path| {
				path.file_name().is_some_and(|name| {
					name.to_string_lossy().starts_with("settings.jsonc.broken-")
				})
			})
			.collect();
		assert_eq!(kept.len(), 1);
		assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), broken);
	}

	#[test]
	fn with_no_backup_that_parses_the_defaults_are_used() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(FILE_NAME), "{ not json").unwrap();
		let store = Store::open(dir.path()).unwrap();
		assert_eq!(store.get(), Settings::initial());
		assert!(store.recovered().unwrap().contains("the defaults"));
	}

	/// Nothing is known about a file that cannot be read at all, and it may
	/// be fine: it is left where it is, and opening fails.
	#[test]
	fn a_file_that_cannot_be_read_is_left_alone() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::create_dir(dir.path().join(FILE_NAME)).unwrap();
		assert!(matches!(Store::open(dir.path()), Err(Error::Io { .. })));
		assert!(dir.path().join(FILE_NAME).is_dir());
	}

	/// A newer build's server, of a kind and with a key this build has no
	/// name for. A change to it here edits it in place, so both stay.
	#[test]
	fn an_older_build_keeps_what_it_cannot_read_in_a_server_it_changes() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::open(dir.path()).unwrap();
		let mut file = serde_json::to_value(Settings::initial()).unwrap();
		let later = serde_json::json!({ "builtin": "later", "host": "later.example", "later": 1 });
		file["servers"].as_array_mut().unwrap().push(later);
		std::fs::write(store.path(), file.to_string()).unwrap();
		store.reload().unwrap().unwrap();

		store
			.update(|s| s.servers[3].username = "me".into())
			.unwrap();
		let text = std::fs::read_to_string(store.path()).unwrap();
		let written: Value = parse_to_serde_value(&text, &ParseOptions::default()).unwrap();
		let entry = &written["servers"][3];
		assert_eq!(entry["builtin"], "later");
		assert_eq!(entry["later"], 1);
		assert_eq!(entry["username"], "me");
	}

	#[test]
	fn own_writes_are_recognised_and_bad_files_are_reported() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::open(dir.path()).unwrap();
		store.update(|s| s.chat.max_lines = 3).unwrap();
		assert!(store.reload().unwrap().is_none(), "our own write");

		std::fs::write(store.path(), "{ not json").unwrap();
		assert!(matches!(store.reload(), Err(Error::Invalid { .. })));
		assert_eq!(
			store.get().chat.max_lines,
			3,
			"invalid input never clobbers"
		);
	}
}
