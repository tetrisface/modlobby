//! The confinement a platform has when it has none: it says why.

use crate::{Confined, Confinement, Policy};

pub struct Refuse {
	pub reason: String,
}

impl Confinement for Refuse {
	fn spawn(
		&self,
		_command: &std::process::Command,
		_policy: &Policy,
	) -> Result<Confined, String> {
		Err(self.reason.clone())
	}

	fn unavailable(&self) -> Option<String> {
		Some(self.reason.clone())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Network;

	#[test]
	fn a_refusal_names_its_reason() {
		let refuse = Refuse {
			reason: "no sandbox here".into(),
		};
		assert_eq!(refuse.unavailable().as_deref(), Some("no sandbox here"));
		let policy = Policy {
			exe_dirs: vec!["C:/engine".into()],
			roots: Vec::new(),
			write: "C:/mods".into(),
			network: Network::None,
			env: Vec::new(),
			grants_file: "C:/grants.json".into(),
		};
		let err = refuse
			.spawn(&std::process::Command::new("spring"), &policy)
			.err();
		assert_eq!(err.as_deref(), Some("no sandbox here"));
	}
}
