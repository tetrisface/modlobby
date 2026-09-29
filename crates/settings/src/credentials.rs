//! Where passwords go: the OS keyring (Windows Credential Manager here), never
//! the settings file. The trait is the seam; tests and the CLI use memory.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::model::DEFAULT_HOST;

const SERVICE: &str = "modlobby";

/// Where an account's password is kept: its name on its server, by the
/// server's id. The same name on two servers is two accounts.
pub fn account(server: &str, username: &str) -> String {
	format!("{username}@{server}")
}

/// The password for `username` on `server`, if one is kept.
///
/// Before there were several servers a password was kept under the bare
/// name, and that one can only be BAR's; it is still found there.
pub fn password(
	store: &dyn CredentialStore,
	server: &str,
	username: &str,
) -> Result<Option<String>, CredentialError> {
	if let Some(found) = store.get(&account(server, username))? {
		return Ok(Some(found));
	}
	if server != DEFAULT_HOST {
		return Ok(None);
	}
	store.get(username)
}

/// Keeps the password under the new key — and BAR's under the bare name as
/// well, in step, because a build from before servers reads only that one
/// and may be installed beside this one, sharing the keyring.
///
/// ponytail: the bare key is written for as long as such builds may run;
/// stop writing it (and reading it in `password`) once none do.
pub fn keep(
	store: &dyn CredentialStore,
	server: &str,
	username: &str,
	password: &str,
) -> Result<(), CredentialError> {
	store.set(&account(server, username), password)?;
	if server == DEFAULT_HOST {
		store.set(username, password)?;
	}
	Ok(())
}

/// Forgets the password under either key.
pub fn forget(
	store: &dyn CredentialStore,
	server: &str,
	username: &str,
) -> Result<(), CredentialError> {
	store.delete(&account(server, username))?;
	if server == DEFAULT_HOST {
		store.delete(username)?;
	}
	Ok(())
}

/// The usernames with a password kept for `server`: what a login form can
/// start from when the settings name nobody -- a fresh config directory on a
/// machine whose keyring remembers. A bare key is BAR's, as in `password`.
pub fn usernames(
	store: &dyn CredentialStore,
	server: &str,
) -> Result<Vec<String>, CredentialError> {
	let mut names: Vec<String> = store
		.keys()?
		.into_iter()
		.filter_map(|key| match key.rsplit_once('@') {
			Some((name, host)) => (host == server).then(|| name.to_owned()),
			None => (server == DEFAULT_HOST).then_some(key),
		})
		.collect();
	names.sort();
	names.dedup();
	Ok(names)
}

#[derive(Debug, thiserror::Error)]
#[error("credential store: {0}")]
pub struct CredentialError(pub String);

pub trait CredentialStore: Send + Sync {
	fn get(&self, username: &str) -> Result<Option<String>, CredentialError>;
	fn set(&self, username: &str, password: &str) -> Result<(), CredentialError>;
	fn delete(&self, username: &str) -> Result<(), CredentialError>;
	/// Every key a password is kept under.
	fn keys(&self) -> Result<Vec<String>, CredentialError>;
}

/// The platform keyring, keyed by `modlobby` / `<username>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
	fn entry(username: &str) -> Result<keyring::Entry, CredentialError> {
		keyring::Entry::new(SERVICE, username).map_err(|err| CredentialError(err.to_string()))
	}
}

impl CredentialStore for KeyringStore {
	fn get(&self, username: &str) -> Result<Option<String>, CredentialError> {
		match Self::entry(username)?.get_password() {
			Ok(password) => Ok(Some(password)),
			Err(keyring::Error::NoEntry) => Ok(None),
			Err(err) => Err(CredentialError(err.to_string())),
		}
	}

	fn set(&self, username: &str, password: &str) -> Result<(), CredentialError> {
		Self::entry(username)?
			.set_password(password)
			.map_err(|err| CredentialError(err.to_string()))
	}

	fn delete(&self, username: &str) -> Result<(), CredentialError> {
		match Self::entry(username)?.delete_credential() {
			Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
			Err(err) => Err(CredentialError(err.to_string())),
		}
	}

	fn keys(&self) -> Result<Vec<String>, CredentialError> {
		// Asking after the store is what sets it up; a search before that
		// finds no store at all.
		keyring::Entry::store_status()
			.as_ref()
			.map_err(|err| CredentialError(err.to_string()))?;
		// Each platform's store searches in its own terms: Windows by a
		// pattern over the credential's target name, `<user>.<service>`, the
		// others by the service attribute. `SERVICE` has nothing a pattern
		// would read as more than itself.
		#[cfg(windows)]
		let pattern = format!(r"\.{SERVICE}$");
		#[cfg(windows)]
		let spec = HashMap::from([("pattern", pattern.as_str())]);
		#[cfg(not(windows))]
		let spec = HashMap::from([("service", SERVICE)]);
		let found =
			keyring_core::Entry::search(&spec).map_err(|err| CredentialError(err.to_string()))?;
		Ok(found
			.iter()
			.filter_map(keyring_core::Entry::get_specifiers)
			.filter(|(service, _)| service == SERVICE)
			.map(|(_, key)| key)
			.collect())
	}
}

/// In-memory store for tests.
///
/// Not a headless fallback: the CLI binds no store at all, reading
/// `MODLOBBY_PASSWORD` from the environment instead.
#[derive(Debug, Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl CredentialStore for MemoryStore {
	fn get(&self, username: &str) -> Result<Option<String>, CredentialError> {
		Ok(self.0.lock().expect("store lock").get(username).cloned())
	}

	fn set(&self, username: &str, password: &str) -> Result<(), CredentialError> {
		self.0
			.lock()
			.expect("store lock")
			.insert(username.to_owned(), password.to_owned());
		Ok(())
	}

	fn delete(&self, username: &str) -> Result<(), CredentialError> {
		self.0.lock().expect("store lock").remove(username);
		Ok(())
	}

	fn keys(&self) -> Result<Vec<String>, CredentialError> {
		Ok(self.0.lock().expect("store lock").keys().cloned().collect())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn memory_store_round_trips() {
		let store = MemoryStore::default();
		assert_eq!(store.get("alice").unwrap(), None);
		store.set("alice", "pw").unwrap();
		assert_eq!(store.get("alice").unwrap().as_deref(), Some("pw"));
		store.delete("alice").unwrap();
		assert_eq!(store.get("alice").unwrap(), None);
	}

	const BAR: &str = DEFAULT_HOST;

	#[test]
	fn the_same_name_on_two_servers_is_two_accounts() {
		let store = MemoryStore::default();
		keep(&store, BAR, "alice", "one").unwrap();
		keep(&store, "rapid", "alice", "two").unwrap();
		assert_eq!(
			password(&store, BAR, "alice").unwrap().as_deref(),
			Some("one")
		);
		assert_eq!(
			password(&store, "rapid", "alice").unwrap().as_deref(),
			Some("two")
		);
	}

	#[test]
	fn a_password_kept_before_servers_is_bars() {
		let store = MemoryStore::default();
		store.set("tetrisface", "old").unwrap();
		assert_eq!(
			password(&store, BAR, "tetrisface").unwrap().as_deref(),
			Some("old")
		);
		assert_eq!(
			password(&store, "rapid", "tetrisface").unwrap(),
			None,
			"only BAR's"
		);
	}

	#[test]
	fn bars_password_stays_where_an_older_build_reads_it() {
		// An installed build from before servers shares this keyring and
		// knows only the bare name; logging in here must not lock it out.
		let store = MemoryStore::default();
		store.set("tetrisface", "old").unwrap();
		keep(&store, BAR, "tetrisface", "new").unwrap();
		assert_eq!(store.get("tetrisface").unwrap().as_deref(), Some("new"));
		assert_eq!(
			password(&store, BAR, "tetrisface").unwrap().as_deref(),
			Some("new")
		);

		keep(&store, "rapid", "tetrisface", "other").unwrap();
		assert_eq!(
			store.get("tetrisface").unwrap().as_deref(),
			Some("new"),
			"only BAR's"
		);
	}

	#[test]
	fn forgetting_clears_both_keys() {
		let store = MemoryStore::default();
		keep(&store, BAR, "tetrisface", "new").unwrap();
		forget(&store, BAR, "tetrisface").unwrap();
		assert_eq!(password(&store, BAR, "tetrisface").unwrap(), None);
		assert_eq!(store.get("tetrisface").unwrap(), None);
	}

	#[test]
	fn usernames_are_read_back_from_the_keys() {
		let store = MemoryStore::default();
		keep(&store, BAR, "tetrisface", "pw").unwrap();
		keep(&store, "rapid", "tetrisface2", "pw").unwrap();
		keep(&store, "rapid", "alice", "pw").unwrap();
		store.set("old", "pw").unwrap();
		assert_eq!(
			usernames(&store, "rapid").unwrap(),
			["alice", "tetrisface2"]
		);
		// BAR's once, though kept under two keys, and the bare one from
		// before servers beside it.
		assert_eq!(usernames(&store, BAR).unwrap(), ["old", "tetrisface"]);
		assert!(usernames(&store, "nowhere").unwrap().is_empty());
	}
}
