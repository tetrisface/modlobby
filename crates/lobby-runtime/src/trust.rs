//! Whose code runs: the standing of each mod a lobby loads, decided from
//! the player's trust list, the mod's commit pin, GitHub's word on whether
//! that commit is on its branch, and what was allowed for this lobby only.
//!
//! One place decides; the content check, the download, the greeting to the
//! host and the launch all read the standing and never the list.

use content::git::Reach;
use lobby_core::Mutator;

/// What the player's settings say, pushed by the app whenever they change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trust {
	pub trusted: Vec<Trusted>,
	/// Whether a repository's trust covers one commit at a time.
	pub per_commit: bool,
}

/// One repository allowed, or one commit of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trusted {
	/// `owner/repo` on GitHub.
	pub repo: String,
	/// This commit and no other; `None` for any commit on the default branch.
	pub commit: Option<String>,
	/// Whether its games run inside the sandbox.
	pub sandboxed: bool,
}

/// Where a mod stands with the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
	/// Fetched, built and run: inside the sandbox, or with the player's own
	/// rights.
	Trusted { sandboxed: bool },
	/// Nothing happens until the player says so.
	Asks(Why),
	/// GitHub is being asked whether the commit is on its branch.
	Checking,
}

/// Why a mod asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
	/// Its repository is not on the list.
	Repo,
	/// Its repository is, one commit at a time, and this commit is new.
	Commit,
	/// No commit pin: loaded by name, with nothing to remember it by.
	Unpinned,
	/// Not on the branch it should be on, or GitHub could not be asked; the
	/// words say which.
	Branch(String),
}

/// What a mod is remembered by for one lobby: the commit it is pinned to,
/// or its name.
pub fn key(mutator: &Mutator) -> String {
	match &mutator.source {
		Some(pin) => format!("github:{}@{}", pin.repo, pin.commit),
		None => format!("name:{}", mutator.name),
	}
}

/// The standing of one mod. What was allowed for this lobby runs, sandboxed;
/// a mod with no pin asks; an entry naming this very commit stands on its
/// own, whatever GitHub says; otherwise a commit GitHub puts off its branch,
/// or could not vouch for, asks whatever the list says; a repository entry
/// stands once GitHub puts the commit on the branch, and waits until it has.
pub fn standing(
	trust: &Trust,
	mutator: &Mutator,
	reach: Option<&Reach>,
	allowed_once: bool,
) -> Standing {
	if allowed_once {
		return Standing::Trusted { sandboxed: true };
	}
	let Some(pin) = &mutator.source else {
		return Standing::Asks(Why::Unpinned);
	};
	// GitHub reads names without case; so does the list.
	let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
	let mut by_repo = None;
	let mut seen_repo = false;
	for entry in trust
		.trusted
		.iter()
		.filter(|entry| same(&entry.repo, &pin.repo))
	{
		seen_repo = true;
		match &entry.commit {
			// The player's own word on this very commit: it neither waits
			// for GitHub nor yields to it.
			Some(commit) if same(commit, &pin.commit) => {
				return Standing::Trusted {
					sandboxed: entry.sandboxed,
				};
			}
			Some(_) => {}
			None => by_repo = Some(entry),
		}
	}
	match reach {
		Some(Reach::OffBranch { branch, status }) => {
			return Standing::Asks(Why::Branch(format!(
				"not on {branch} of {}: {status}",
				pin.repo
			)));
		}
		Some(Reach::Unknown(reason)) => {
			return Standing::Asks(Why::Branch(format!("GitHub could not be asked: {reason}")));
		}
		Some(Reach::OnBranch(_)) | None => {}
	}
	// A repository known only by other commits of it asks for this one,
	// whether or not trust is per commit: that is what its entries say.
	let Some(entry) = by_repo else {
		return Standing::Asks(if seen_repo { Why::Commit } else { Why::Repo });
	};
	if trust.per_commit {
		return Standing::Asks(Why::Commit);
	}
	if reach.is_none() {
		return Standing::Checking;
	}
	Standing::Trusted {
		sandboxed: entry.sandboxed,
	}
}

/// Whether every mod runs: nothing asks and nothing is still being checked.
/// Vacuously so for a lobby that loads none.
pub fn all_trusted(standings: &[Standing]) -> bool {
	standings
		.iter()
		.all(|standing| matches!(standing, Standing::Trusted { .. }))
}

/// Whether the game may run outside the sandbox: only when every mod is
/// trusted so, and there is at least one to say it.
pub fn launch_open(standings: &[Standing]) -> bool {
	!standings.is_empty()
		&& standings
			.iter()
			.all(|standing| matches!(standing, Standing::Trusted { sandboxed: false }))
}

#[cfg(test)]
mod tests {
	use super::*;
	use content::git::Reach;
	use lobby_core::{GitPin, Mutator};

	const SHA: &str = "9108a17078f79d09925edc305ec83bc06c3a7cb3";
	const OTHER: &str = "0000000000000000000000000000000000000000";

	fn pinned(repo: &str, sha: &str) -> Mutator {
		Mutator {
			name: format!("github-{}-{}.sdd", repo.replace('/', "-"), &sha[..12]),
			checksum: None,
			source: Some(GitPin {
				repo: repo.into(),
				commit: sha.into(),
			}),
			date: None,
			track: None,
		}
	}

	fn named(name: &str) -> Mutator {
		Mutator {
			name: name.into(),
			checksum: None,
			source: None,
			date: None,
			track: None,
		}
	}

	fn trusting(repo: &str, commit: Option<&str>, sandboxed: bool, per_commit: bool) -> Trust {
		Trust {
			trusted: vec![Trusted {
				repo: repo.into(),
				commit: commit.map(str::to_owned),
				sandboxed,
			}],
			per_commit,
		}
	}

	fn on_main() -> Reach {
		Reach::OnBranch("main".into())
	}

	#[test]
	fn an_untrusted_repository_asks() {
		let mutator = pinned("dev/sphere", SHA);
		assert_eq!(
			standing(&Trust::default(), &mutator, None, false),
			Standing::Asks(Why::Repo)
		);
		// Known to be on its branch changes nothing: the list says nothing.
		assert_eq!(
			standing(&Trust::default(), &mutator, Some(&on_main()), false),
			Standing::Asks(Why::Repo)
		);
	}

	#[test]
	fn a_trusted_repository_waits_for_the_branch_then_passes() {
		let trust = trusting("dev/sphere", None, true, false);
		let mutator = pinned("dev/sphere", SHA);
		assert_eq!(standing(&trust, &mutator, None, false), Standing::Checking);
		assert_eq!(
			standing(&trust, &mutator, Some(&on_main()), false),
			Standing::Trusted { sandboxed: true }
		);
	}

	#[test]
	fn per_commit_trust_asks_for_a_commit_it_has_not_seen() {
		let by_repo = trusting("dev/sphere", None, true, true);
		let mutator = pinned("dev/sphere", SHA);
		assert_eq!(
			standing(&by_repo, &mutator, Some(&on_main()), false),
			Standing::Asks(Why::Commit)
		);
		let this_one = trusting("dev/sphere", Some(SHA), true, true);
		assert_eq!(
			standing(&this_one, &mutator, None, false),
			Standing::Trusted { sandboxed: true },
			"a commit allowed by hand needs no word from GitHub"
		);
		assert_eq!(
			standing(
				&this_one,
				&pinned("dev/sphere", OTHER),
				Some(&on_main()),
				false
			),
			Standing::Asks(Why::Commit)
		);
	}

	#[test]
	fn a_commit_off_its_branch_asks_whatever_the_list_says() {
		let off = Reach::OffBranch {
			branch: "main".into(),
			status: "ahead".into(),
		};
		let mutator = pinned("dev/sphere", SHA);
		let expected = Standing::Asks(Why::Branch("not on main of dev/sphere: ahead".into()));
		assert_eq!(
			standing(&Trust::default(), &mutator, Some(&off), false),
			expected
		);
		let trust = trusting("dev/sphere", None, true, false);
		assert_eq!(standing(&trust, &mutator, Some(&off), false), expected);
		let unknown = Reach::Unknown("GitHub's hourly allowance is used up".into());
		assert_eq!(
			standing(&trust, &mutator, Some(&unknown), false),
			Standing::Asks(Why::Branch(
				"GitHub could not be asked: GitHub's hourly allowance is used up".into()
			))
		);
		// An entry naming this very commit is the player's own word on it:
		// it neither waits for GitHub nor yields to it.
		let this_one = trusting("dev/sphere", Some(SHA), true, false);
		assert_eq!(
			standing(&this_one, &mutator, Some(&off), false),
			Standing::Trusted { sandboxed: true }
		);
		assert_eq!(
			standing(&this_one, &mutator, Some(&unknown), false),
			Standing::Trusted { sandboxed: true }
		);
	}

	#[test]
	fn a_mod_by_name_alone_asks_every_time() {
		let mutator = named("tiny maps v1");
		let trust = trusting("dev/sphere", None, true, false);
		assert_eq!(
			standing(&trust, &mutator, None, false),
			Standing::Asks(Why::Unpinned)
		);
		assert_eq!(
			standing(&trust, &mutator, None, true),
			Standing::Trusted { sandboxed: true },
			"allowed for this lobby, sandboxed"
		);
	}

	#[test]
	fn only_an_entry_says_unsandboxed_and_only_on_its_branch() {
		let open = trusting("dev/sphere", None, false, false);
		let mutator = pinned("dev/sphere", SHA);
		assert_eq!(
			standing(&open, &mutator, Some(&on_main()), false),
			Standing::Trusted { sandboxed: false }
		);
		let unknown = Reach::Unknown("offline".into());
		assert!(matches!(
			standing(&open, &mutator, Some(&unknown), false),
			Standing::Asks(Why::Branch(_))
		));
		let this_one = trusting("dev/sphere", Some(SHA), false, false);
		assert_eq!(
			standing(&this_one, &mutator, None, false),
			Standing::Trusted { sandboxed: false }
		);
		// Allowed for this lobby is never open.
		assert_eq!(
			standing(&Trust::default(), &mutator, None, true),
			Standing::Trusted { sandboxed: true }
		);
	}

	#[test]
	fn the_launch_is_open_only_when_every_mod_says_so() {
		let open = Standing::Trusted { sandboxed: false };
		let boxed = Standing::Trusted { sandboxed: true };
		assert!(!launch_open(&[]));
		assert!(launch_open(std::slice::from_ref(&open)));
		assert!(!launch_open(&[open.clone(), boxed.clone()]));
		assert!(!launch_open(&[open, Standing::Checking]));
		assert!(all_trusted(&[]));
		assert!(all_trusted(std::slice::from_ref(&boxed)));
		assert!(!all_trusted(&[boxed, Standing::Asks(Why::Repo)]));
	}

	#[test]
	fn keys_name_the_commit_or_the_name() {
		assert_eq!(
			key(&pinned("dev/sphere", SHA)),
			format!("github:dev/sphere@{SHA}")
		);
		assert_eq!(key(&named("tiny maps v1")), "name:tiny maps v1");
	}
}
