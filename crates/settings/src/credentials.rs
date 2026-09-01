//! Where passwords go: the OS keyring (Windows Credential Manager here), never
//! the settings file. The trait is the seam; tests and the CLI use memory.

use std::collections::HashMap;
use std::sync::Mutex;

const SERVICE: &str = "modlobby";

#[derive(Debug, thiserror::Error)]
#[error("credential store: {0}")]
pub struct CredentialError(pub String);

pub trait CredentialStore: Send + Sync {
    fn get(&self, username: &str) -> Result<Option<String>, CredentialError>;
    fn set(&self, username: &str, password: &str) -> Result<(), CredentialError>;
    fn delete(&self, username: &str) -> Result<(), CredentialError>;
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
}
