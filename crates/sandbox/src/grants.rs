//! The record of every path the container was granted, so what is no longer
//! in the policy can be taken back, and nothing is left on a directory
//! modlobby has forgotten about.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub struct Grants {
	path: PathBuf,
	held: BTreeSet<PathBuf>,
}

impl Grants {
	/// The record at `path`; one that is missing or unreadable holds nothing,
	/// and the next save replaces it.
	pub fn load(path: &Path) -> Self {
		let held = std::fs::read_to_string(path)
			.ok()
			.and_then(|text| serde_json::from_str::<Vec<PathBuf>>(&text).ok())
			.unwrap_or_default()
			.into_iter()
			.collect();
		Self {
			path: path.to_path_buf(),
			held,
		}
	}

	pub fn held(&self) -> &BTreeSet<PathBuf> {
		&self.held
	}

	/// Revokes what is held but no longer wanted, then grants everything
	/// wanted -- every time, since a file moved into a content directory
	/// keeps the permissions it came with -- and records the result.
	pub fn reconcile(
		&mut self,
		wanted: &[PathBuf],
		mut grant: impl FnMut(&Path) -> Result<(), String>,
		mut revoke: impl FnMut(&Path) -> Result<(), String>,
	) -> Result<(), String> {
		let wanted_set: BTreeSet<PathBuf> = wanted.iter().cloned().collect();
		for stale in self.held.difference(&wanted_set) {
			revoke(stale)?;
		}
		self.held.retain(|path| wanted_set.contains(path));
		self.save()?;
		for path in wanted {
			// On record before it is made: a grant interrupted halfway is
			// still a grant to take back.
			self.held.insert(path.clone());
			self.save()?;
			grant(path)?;
		}
		Ok(())
	}

	/// Written whole or not at all: a torn record would read as empty and
	/// forget every grant.
	fn save(&self) -> Result<(), String> {
		let held: Vec<&PathBuf> = self.held.iter().collect();
		let text = serde_json::to_string_pretty(&held).map_err(|err| err.to_string())?;
		let failed = |err: std::io::Error| {
			format!(
				"recording the sandbox grants in {}: {err}",
				self.path.display()
			)
		};
		if let Some(parent) = self.path.parent() {
			std::fs::create_dir_all(parent).map_err(failed)?;
		}
		let fresh = self.path.with_extension("json.tmp");
		std::fs::write(&fresh, text).map_err(failed)?;
		std::fs::rename(&fresh, &self.path).map_err(failed)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::RefCell;

	fn paths(names: &[&str]) -> Vec<PathBuf> {
		names.iter().map(PathBuf::from).collect()
	}

	#[test]
	fn a_grant_no_longer_in_the_policy_is_revoked() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("sandbox-grants.json");
		std::fs::write(&file, r#"["C:/old/games", "C:/kept/games"]"#).unwrap();
		let mut grants = Grants::load(&file);
		let revoked = RefCell::new(Vec::new());
		grants
			.reconcile(
				&paths(&["C:/kept/games"]),
				|_| Ok(()),
				|path| {
					revoked.borrow_mut().push(path.to_path_buf());
					Ok(())
				},
			)
			.unwrap();
		assert_eq!(revoked.into_inner(), paths(&["C:/old/games"]));
		assert_eq!(
			Grants::load(&file).held(),
			&paths(&["C:/kept/games"]).into_iter().collect()
		);
	}

	#[test]
	fn every_wanted_path_is_granted_again_each_time() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("sandbox-grants.json");
		std::fs::write(&file, r#"["C:/kept/games"]"#).unwrap();
		let mut grants = Grants::load(&file);
		let granted = RefCell::new(Vec::new());
		grants
			.reconcile(
				&paths(&["C:/kept/games", "C:/new/maps"]),
				|path| {
					granted.borrow_mut().push(path.to_path_buf());
					Ok(())
				},
				|_| Ok(()),
			)
			.unwrap();
		assert_eq!(
			granted.into_inner(),
			paths(&["C:/kept/games", "C:/new/maps"]),
			"a file moved into a content dir keeps its old permissions, so the grant is made again"
		);
	}

	#[test]
	fn a_missing_or_broken_record_holds_nothing() {
		let dir = tempfile::tempdir().unwrap();
		assert!(
			Grants::load(&dir.path().join("none.json"))
				.held()
				.is_empty()
		);
		let broken = dir.path().join("broken.json");
		std::fs::write(&broken, "{not json").unwrap();
		assert!(Grants::load(&broken).held().is_empty());
	}

	/// A grant is on record before it is made: a crash in the middle of
	/// making it must not leave an entry nobody knows to take back.
	#[test]
	fn a_grant_is_recorded_before_it_is_made() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("sandbox-grants.json");
		let mut grants = Grants::load(&file);
		let err = grants
			.reconcile(
				&paths(&["C:/new/games"]),
				|_| Err("crashed mid-grant".into()),
				|_| Ok(()),
			)
			.unwrap_err();
		assert_eq!(err, "crashed mid-grant");
		assert_eq!(
			Grants::load(&file).held(),
			&paths(&["C:/new/games"]).into_iter().collect()
		);
	}
}
