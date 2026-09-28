//! Which way into each server worked last, kept between runs so the next
//! connect tries it on its own instead of racing every way again.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use spring_protocol::{Way, server_id};

use crate::json_file;

pub(crate) const FILE: &str = "ways.json";

/// By [`server_id`]: one server however its name was typed.
///
/// An entry this build cannot read -- a newer build's, say a way over a
/// security this one has no name for -- is kept as it was and written back,
/// so the certificate that build pinned is still there when it runs again.
/// Here that server is simply one without a way yet.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Ways {
	known: BTreeMap<String, Way>,
	unread: BTreeMap<String, Value>,
}

impl Ways {
	pub(crate) fn load(path: &Path) -> Self {
		let entries: BTreeMap<String, Value> = json_file::load(path, "ways");
		let mut ways = Self::default();
		for (host, entry) in entries {
			match Way::deserialize(&entry) {
				Ok(way) => {
					ways.known.insert(host, way);
				}
				Err(err) => {
					tracing::debug!(host, %err, "a way this build cannot read is kept as it is");
					ways.unread.insert(host, entry);
				}
			}
		}
		ways
	}

	pub(crate) fn save(&self, path: &Path) {
		let mut entries = self.unread.clone();
		for (host, way) in &self.known {
			entries.insert(
				host.clone(),
				serde_json::to_value(way).expect("a way serialises"),
			);
		}
		json_file::save(&entries, path, "ways");
	}

	pub(crate) fn get(&self, host: &str) -> Option<Way> {
		self.known.get(&server_id(host)).copied()
	}

	/// A way found here replaces one this build could not read.
	pub(crate) fn remember(&mut self, host: &str, way: Way) {
		let host = server_id(host);
		self.unread.remove(&host);
		self.known.insert(host, way);
	}

	pub(crate) fn forget(&mut self, host: &str) {
		let host = server_id(host);
		self.known.remove(&host);
		self.unread.remove(&host);
	}

	pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &Way)> {
		self.known.iter()
	}
}

#[cfg(test)]
mod tests {
	use spring_protocol::Security;

	use super::*;

	const STLS: Way = Way {
		port: 8200,
		security: Security::Stls,
		ms: 46,
		pin: None,
	};

	#[test]
	fn a_host_is_one_server_however_it_is_typed() {
		let mut ways = Ways::default();
		ways.remember(" Server4.BeyondAllReason.info ", STLS);
		assert_eq!(ways.get("server4.beyondallreason.info"), Some(STLS));
		ways.forget("SERVER4.beyondallreason.info");
		assert_eq!(ways.get("server4.beyondallreason.info"), None);
	}

	#[test]
	fn a_way_this_build_cannot_read_is_written_back_as_it_was() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join(FILE);
		let later =
			serde_json::json!({ "port": 8201, "security": "later", "ms": 30, "pin": "kept" });
		let file = serde_json::json!({
			"server4.beyondallreason.info": { "port": 8200, "security": "stls", "ms": 46 },
			"later.example": later,
		});
		std::fs::write(&path, file.to_string()).unwrap();

		let mut ways = Ways::load(&path);
		assert_eq!(ways.get("server4.beyondallreason.info"), Some(STLS));
		assert_eq!(ways.get("later.example"), None);
		ways.remember("other.example", STLS);
		ways.save(&path);

		let written: Value =
			serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
		assert_eq!(written["later.example"], later);
		assert_eq!(written["other.example"]["port"], 8200);
	}

	#[test]
	fn what_is_saved_loads_back_and_what_cannot_be_read_is_empty() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join(FILE);
		let mut ways = Ways::default();
		ways.remember("server4.beyondallreason.info", STLS);
		ways.save(&path);
		assert_eq!(Ways::load(&path), ways);

		std::fs::write(&path, "{ not json").unwrap();
		assert_eq!(Ways::load(&path), Ways::default());
		assert_eq!(
			Ways::load(&dir.path().join("missing.json")),
			Ways::default()
		);
	}
}
