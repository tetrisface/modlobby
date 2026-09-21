//! Which way into each server worked last, kept between runs so the next
//! connect tries it on its own instead of racing every way again.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use spring_protocol::{Way, server_id};

use crate::json_file;

pub(crate) const FILE: &str = "ways.json";

/// By [`server_id`]: one server however its name was typed.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Ways(BTreeMap<String, Way>);

impl Ways {
	pub(crate) fn load(path: &Path) -> Self {
		json_file::load(path, "ways")
	}

	pub(crate) fn save(&self, path: &Path) {
		json_file::save(self, path, "ways");
	}

	pub(crate) fn get(&self, host: &str) -> Option<Way> {
		self.0.get(&server_id(host)).copied()
	}

	pub(crate) fn remember(&mut self, host: &str, way: Way) {
		self.0.insert(server_id(host), way);
	}

	pub(crate) fn forget(&mut self, host: &str) {
		self.0.remove(&server_id(host));
	}

	pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &Way)> {
		self.0.iter()
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
