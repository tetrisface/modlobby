//! A value kept between runs as one small JSON file. A file that is missing
//! or cannot be read is a value to start over from, and one that cannot be
//! written costs only what the next run has to find out again.

use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub(crate) fn load<T: DeserializeOwned + Default>(path: &Path, what: &str) -> T {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
            tracing::debug!(path = %path.display(), %err, "{what} unreadable; starting over");
            T::default()
        }),
        Err(_) => T::default(),
    }
}

pub(crate) fn save<T: Serialize>(value: &T, path: &Path, what: &str) {
    let text = serde_json::to_string_pretty(value).expect("plain data serialises");
    if let Err(err) = std::fs::write(path, text) {
        tracing::warn!(path = %path.display(), %err, "{what} not written");
    }
}
