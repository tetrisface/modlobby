//! Maps a search said it does not have, kept between runs so the same
//! question is not put to the same server again for a day.
//!
//! Joining a room fetches its missing map unasked, and a map nobody carries
//! is met in every room that plays it — so without this, hopping between two
//! such rooms, or restarting in one, asks somebody's server the same thing
//! over and over for an answer that has not changed. Asking by hand is never
//! held back: a person clicking Download has a reason to think it has.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::json_file;

pub(crate) const FILE: &str = "misses.json";

/// How long "not found" is taken as the answer.
const HOLDS_FOR: u64 = 24 * 60 * 60;

/// By search URL and map name: when it was last not found, in unix seconds.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Misses(BTreeMap<String, u64>);

fn key(search: &str, map: &str) -> String {
	format!("{search} {map}")
}

impl Misses {
	pub(crate) fn load(path: &Path) -> Self {
		json_file::load(path, "map misses")
	}

	pub(crate) fn save(&self, path: &Path) {
		json_file::save(self, path, "map misses");
	}

	/// Whether `search` said within the day that it has no `map`.
	pub(crate) fn holds(&self, search: &str, map: &str, now: u64) -> bool {
		self.0
			.get(&key(search, map))
			.is_some_and(|at| now.saturating_sub(*at) < HOLDS_FOR)
	}

	/// Records a miss, and lets go of those that no longer hold.
	pub(crate) fn remember(&mut self, search: &str, map: &str, now: u64) {
		self.0.retain(|_, at| now.saturating_sub(*at) < HOLDS_FOR);
		self.0.insert(key(search, map), now);
	}

	/// Whether there was one to forget.
	pub(crate) fn forget(&mut self, search: &str, map: &str) -> bool {
		self.0.remove(&key(search, map)).is_some()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const FIND: &str = "https://maps.example/find";

	#[test]
	fn a_miss_holds_for_a_day_for_that_search_alone() {
		let mut misses = Misses::default();
		misses.remember(FIND, "Nowhere v1", 1_000);
		assert!(misses.holds(FIND, "Nowhere v1", 1_000 + HOLDS_FOR - 1));
		assert!(!misses.holds(FIND, "Nowhere v1", 1_000 + HOLDS_FOR));
		assert!(!misses.holds("https://other.example/find", "Nowhere v1", 1_001));
		assert!(!misses.holds(FIND, "Somewhere v1", 1_001));
	}

	#[test]
	fn asking_by_hand_forgets_it_and_old_ones_are_let_go() {
		let mut misses = Misses::default();
		misses.remember(FIND, "Old v1", 0);
		misses.remember(FIND, "New v1", HOLDS_FOR + 5);
		assert_eq!(misses.0.len(), 1, "the old one went when the new came");
		assert!(misses.forget(FIND, "New v1"));
		assert!(!misses.forget(FIND, "New v1"));
	}

	#[test]
	fn survives_a_restart_and_a_broken_file_is_none() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join(FILE);
		let mut misses = Misses::default();
		misses.remember(FIND, "Nowhere v1", 7);
		misses.save(&path);
		assert_eq!(Misses::load(&path), misses);
		std::fs::write(&path, "not json").unwrap();
		assert_eq!(Misses::load(&path), Misses::default());
	}
}
