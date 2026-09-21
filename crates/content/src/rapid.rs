//! Somebody else's rapid server, read before anything is fetched from it.
//!
//! A rapid server is two kinds of gzipped text: a master index naming its
//! repos (`name,url,,`), and in each repo a `versions.gz` naming what it
//! publishes (`tag,md5,depends,name`). BAR has one; a server of mods has its
//! own, which usually lists BAR's repos beside its own so that a mod's BAR
//! dependency resolves. Nothing here is particular to any one server: the
//! format is the protocol, and reading it is how a server we have no source
//! for is understood.
//!
//! # What is checked, and why before
//!
//! Every server's games land in one data directory. That is what lets a mod
//! reuse the BAR files already on disk, and it is safe for files — a package
//! is named by its hash, so two can never overwrite each other — but not for
//! *names*: the engine finds a game by the name inside it. A rapid server
//! that published its own "Beyond All Reason test-31251" would put a second
//! game by that name on disk, and which one then runs on BAR's own server is
//! anybody's guess. So before a game is fetched from a rapid server that is
//! not BAR's, everything that server publishes itself is compared with what
//! BAR publishes, and a BAR name under a different hash refuses the server.

use std::collections::HashMap;
use std::io::Read;
use std::sync::Mutex;

use flate2::read::GzDecoder;

/// The most one index may be on the wire, and unpacked. BAR's largest is
/// 0.6 MB packed and a few MB unpacked; past this it is not an index.
const MOST_PACKED: usize = 32 << 20;
const MOST_UNPACKED: u64 = 256 << 20;

/// A repo a master index lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
	pub name: String,
	pub url: String,
}

/// A version a repo publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
	pub md5: String,
	pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("{url}: {reason}")]
	Fetch { url: String, reason: String },
	#[error("{0} is not a rapid index")]
	NotAnIndex(String),
	/// Plain `http` would let anyone between here and there put their own
	/// game code in the answer.
	#[error("{0} is not served over https")]
	Insecure(String),
	#[error(
		"this server's rapid publishes {0} under BAR's name with other contents; nothing is fetched from it"
	)]
	Shadows(String),
}

/// What a master index turned out to list, for whoever typed its address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RapidSummary {
	/// Repos the server hosts itself.
	pub own: u32,
	/// Repos of BAR's it lists beside them.
	pub bars: u32,
}

/// `name,url,,` a line; a line with fewer fields is not one of these.
pub fn parse_repos(unpacked: &str) -> Option<Vec<Repo>> {
	let repos: Option<Vec<Repo>> = unpacked
		.lines()
		.filter(|line| !line.trim().is_empty())
		.map(|line| {
			let mut fields = line.split(',');
			let (name, url) = (fields.next()?.trim(), fields.next()?.trim());
			(!name.is_empty() && url.contains("://")).then(|| Repo {
				name: name.to_owned(),
				url: url.trim_end_matches('/').to_owned(),
			})
		})
		.collect();
	repos.filter(|repos| !repos.is_empty())
}

/// `tag,md5,depends,name` a line. A line that is not one is skipped rather
/// than refused: pr-downloader reads the same file and is the judge of it.
pub fn parse_versions(unpacked: &str) -> Vec<Version> {
	unpacked
		.lines()
		.filter_map(|line| {
			let mut fields = line.splitn(4, ',');
			let (_tag, md5, _depends, name) = (
				fields.next()?,
				fields.next()?,
				fields.next()?,
				fields.next()?,
			);
			Some(Version {
				md5: md5.trim().to_owned(),
				name: name.trim().to_owned(),
			})
		})
		.filter(|version| !version.name.is_empty())
		.collect()
}

/// BAR's names and the hashes it publishes each under.
type Names = HashMap<String, Vec<String>>;

/// The first of `theirs` that carries one of BAR's names with contents BAR
/// never published under it -- other than a copy already looked at, which
/// the runtime never fetches from anyone but BAR ([`recoil::STALE_COPIES`]).
pub fn shadowed<'a>(bars: &Names, theirs: &'a [Version]) -> Option<&'a str> {
	theirs
		.iter()
		.filter(|version| {
			!recoil::STALE_COPIES.contains(&(version.name.as_str(), version.md5.as_str()))
		})
		.find(|version| {
			bars.get(&version.name)
				.is_some_and(|published| !published.contains(&version.md5))
		})
		.map(|version| version.name.as_str())
}

/// Reads rapid servers. Holds BAR's names once read, since they are the
/// large part and the same for every server checked against them.
pub struct Vetter {
	client: reqwest::Client,
	/// BAR's master index: whose names are protected, and whose repos are
	/// taken as read when another server lists them.
	bars_master: String,
	/// Whether everything must be https. Always, outside a test: the fake
	/// servers tests run against speak plain http.
	https_only: bool,
	bars: Mutex<Option<(Vec<Repo>, std::sync::Arc<Names>)>>,
}

impl Vetter {
	/// `bars_master` is BAR's master index, as its launcher config names it.
	pub fn new(client: reqwest::Client, bars_master: impl Into<String>) -> Self {
		Self {
			client,
			bars_master: bars_master.into(),
			https_only: true,
			bars: Mutex::new(None),
		}
	}

	/// Against a stand-in for BAR's master index, on a test's plain-http server.
	#[cfg(test)]
	fn against(client: reqwest::Client, bars_master: &str) -> Self {
		Self {
			https_only: false,
			..Self::new(client, bars_master)
		}
	}

	fn insecure(&self, url: &str) -> bool {
		self.https_only && !url.starts_with("https://")
	}

	/// Whether games may be fetched through `master`. BAR's own needs no
	/// looking at; anyone else's is read, and refused if it is not a rapid
	/// index over https or publishes one of BAR's names as its own.
	pub async fn vet(&self, master: &str) -> Result<(), Error> {
		if master == self.bars_master {
			return Ok(());
		}
		// Theirs first: an address that is refused outright costs BAR's
		// servers nothing.
		let listed = self.repos(master).await?;
		let (bars_repos, bars_names) = self.bars().await?;
		for repo in self.own_repos(listed, &bars_repos)? {
			let url = format!("{}/versions.gz", repo.url);
			let theirs = parse_versions(&self.unpacked(&url).await?);
			if let Some(name) = shadowed(&bars_names, &theirs) {
				return Err(Error::Shadows(name.to_owned()));
			}
		}
		Ok(())
	}

	/// Everything `master`'s own repos publish, repo by repo: what a server
	/// of mods has to offer, read the way any rapid client would.
	pub async fn published(&self, master: &str) -> Result<Vec<(Repo, Vec<Version>)>, Error> {
		let listed = self.repos(master).await?;
		let (bars_repos, _) = self.bars().await?;
		let mut published = Vec::new();
		for repo in self.own_repos(listed, &bars_repos)? {
			let url = format!("{}/versions.gz", repo.url);
			let versions = parse_versions(&self.unpacked(&url).await?);
			published.push((repo, versions));
		}
		Ok(published)
	}

	/// Every game name BAR's rapid publishes, newest nowhere in particular:
	/// the caller orders them. What a room with no host to keep it current
	/// checks itself against.
	pub async fn bar_names(&self) -> Result<Vec<String>, Error> {
		Ok(self.bars().await?.1.keys().cloned().collect())
	}

	/// What `master` lists, for the person setting a server up.
	pub async fn summary(&self, master: &str) -> Result<RapidSummary, Error> {
		let listed = self.repos(master).await?;
		let (bars_repos, _) = self.bars().await?;
		let bars = listed
			.iter()
			.filter(|repo| bars_repos.iter().any(|bar| bar.url == repo.url))
			.count();
		Ok(RapidSummary {
			own: (listed.len() - bars) as u32,
			bars: bars as u32,
		})
	}

	/// Those of `listed` that are not BAR's own, each over https.
	fn own_repos(&self, listed: Vec<Repo>, bars: &[Repo]) -> Result<Vec<Repo>, Error> {
		let own: Vec<Repo> = listed
			.into_iter()
			.filter(|repo| !bars.iter().any(|bar| bar.url == repo.url))
			.collect();
		match own.iter().find(|repo| self.insecure(&repo.url)) {
			Some(plain) => Err(Error::Insecure(plain.url.clone())),
			None => Ok(own),
		}
	}

	async fn repos(&self, master: &str) -> Result<Vec<Repo>, Error> {
		if self.insecure(master) {
			return Err(Error::Insecure(master.to_owned()));
		}
		parse_repos(&self.unpacked(master).await?).ok_or_else(|| Error::NotAnIndex(master.into()))
	}

	/// BAR's repos and every name in them, read once a run.
	///
	/// ponytail: held for the life of the process, so a BAR version published
	/// after the first check is not protected until the next start; re-read
	/// on an age if a rapid server is ever caught racing BAR's releases.
	async fn bars(&self) -> Result<(Vec<Repo>, std::sync::Arc<Names>), Error> {
		if let Some(held) = self.bars.lock().expect("bar's names").clone() {
			return Ok(held);
		}
		let repos = self.repos(&self.bars_master).await?;
		let mut names = Names::new();
		for repo in &repos {
			let url = format!("{}/versions.gz", repo.url);
			for version in parse_versions(&self.unpacked(&url).await?) {
				names.entry(version.name).or_default().push(version.md5);
			}
		}
		let held = (repos, std::sync::Arc::new(names));
		*self.bars.lock().expect("bar's names") = Some(held.clone());
		Ok(held)
	}

	/// One gzipped index as text, bounded both packed and unpacked.
	async fn unpacked(&self, url: &str) -> Result<String, Error> {
		let fetch = |reason: String| Error::Fetch {
			url: url.to_owned(),
			reason,
		};
		let mut response = self
			.client
			.get(url)
			.send()
			.await
			.and_then(reqwest::Response::error_for_status)
			.map_err(|err| fetch(err.to_string()))?;
		let mut packed = Vec::new();
		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|err| fetch(err.to_string()))?
		{
			packed.extend_from_slice(&chunk);
			if packed.len() > MOST_PACKED {
				return Err(fetch("too large to be an index".into()));
			}
		}
		let mut text = String::new();
		GzDecoder::new(packed.as_slice())
			.take(MOST_UNPACKED)
			.read_to_string(&mut text)
			.map_err(|_| Error::NotAnIndex(url.to_owned()))?;
		Ok(text)
	}
}

#[cfg(test)]
mod tests {
	use std::io::Write;

	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;

	fn gz(text: &str) -> Vec<u8> {
		let mut packer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
		packer.write_all(text.as_bytes()).unwrap();
		packer.finish().unwrap()
	}

	#[test]
	fn a_master_index_is_names_and_addresses() {
		let repos = parse_repos(
            "randomguy-hosting,https://rapid.example/randomguy-hosting,,\nbyar,https://repos-cdn.beyondallreason.dev/byar/,,\n",
        )
        .unwrap();
		assert_eq!(repos[0].name, "randomguy-hosting");
		assert_eq!(repos[1].url, "https://repos-cdn.beyondallreason.dev/byar");
		assert_eq!(parse_repos("<html>not found</html>"), None);
		assert_eq!(parse_repos(""), None);
	}

	#[test]
	fn a_version_is_a_name_and_the_hash_it_is_published_under() {
		let versions = parse_versions(
			"mod:test,392134f8,Beyond All Reason test-31251-0ceadc7,RandomGuy Hosting v1\nbroken line\nmod:x,aa,,\n",
		);
		assert_eq!(
			versions,
			[Version {
				md5: "392134f8".into(),
				name: "RandomGuy Hosting v1".into()
			}]
		);
	}

	#[test]
	fn bars_name_under_another_hash_is_a_shadow_and_under_its_own_is_not() {
		let bars = Names::from([(
			"Beyond All Reason test-1".to_owned(),
			vec!["aaaa".to_owned()],
		)]);
		let version = |name: &str, md5: &str| Version {
			name: name.into(),
			md5: md5.into(),
		};
		assert_eq!(
			shadowed(&bars, &[version("Somebody's Mod v1", "bbbb")]),
			None
		);
		assert_eq!(
			shadowed(&bars, &[version("Beyond All Reason test-1", "aaaa")]),
			None,
			"a mirror of what BAR published is BAR's game"
		);
		assert_eq!(
			shadowed(&bars, &[version("Beyond All Reason test-1", "bbbb")]),
			Some("Beyond All Reason test-1")
		);
	}

	/// BAR's rapid and somebody else's on one fake host, and a vetter that
	/// takes that host's plain http.
	async fn served(theirs: &str) -> (MockServer, Vetter) {
		let server = MockServer::start().await;
		let at = server.uri();
		let files = [
			("/bar/repos.gz", format!("byar,{at}/bar/byar,,\n")),
			(
				"/bar/byar/versions.gz",
				"byar:test,aaaa,,Beyond All Reason test-1\n".into(),
			),
			(
				"/theirs/repos.gz",
				format!("mods,{at}/theirs/mods,,\nbyar,{at}/bar/byar,,\n"),
			),
			("/theirs/mods/versions.gz", theirs.to_owned()),
		];
		for (at, text) in files {
			Mock::given(method("GET"))
				.and(path(at))
				.respond_with(ResponseTemplate::new(200).set_body_bytes(gz(&text)))
				.mount(&server)
				.await;
		}
		let vetter = Vetter::against(crate::http::client("test"), &format!("{at}/bar/repos.gz"));
		(server, vetter)
	}

	#[tokio::test]
	async fn plain_http_is_refused_before_anything_is_asked() {
		let (server, _) = served("mods:test,bbbb,,Somebody's Mod v1\n").await;
		let vetter = Vetter::new(crate::http::client("test"), recoil::RAPID_REPO_MASTER);
		let refused = vetter
			.vet(&format!("{}/theirs/repos.gz", server.uri()))
			.await;
		assert!(matches!(refused, Err(Error::Insecure(_))), "{refused:?}");
		assert!(server.received_requests().await.unwrap().is_empty());
	}

	#[tokio::test]
	async fn a_server_of_its_own_mods_beside_bars_games_passes() {
		let (server, vetter) =
			served("mods:test,bbbb,Beyond All Reason test-1,Somebody's Mod v1\n").await;
		let master = format!("{}/theirs/repos.gz", server.uri());
		vetter.vet(&master).await.unwrap();
		assert_eq!(
			vetter.summary(&master).await.unwrap(),
			RapidSummary { own: 1, bars: 1 }
		);

		// BAR's names are read once, however many servers are checked.
		vetter.vet(&master).await.unwrap();
		let asked = server.received_requests().await.unwrap();
		let bars_index = asked
			.iter()
			.filter(|request| request.url.path() == "/bar/byar/versions.gz")
			.count();
		assert_eq!(bars_index, 1);
	}

	#[test]
	fn a_stale_copy_already_looked_at_is_not_held_against_its_server() {
		let (name, md5) = recoil::STALE_COPIES[0];
		let bars: Names = [(
			name.to_owned(),
			vec!["f7abf7328ee9025a7ba3e63d6f0f3b4e".to_owned()],
		)]
		.into();
		let stale = Version {
			md5: md5.to_owned(),
			name: name.to_owned(),
		};
		assert_eq!(shadowed(&bars, std::slice::from_ref(&stale)), None);
		let other = Version {
			md5: "0000".into(),
			..stale
		};
		assert_eq!(shadowed(&bars, &[other]), Some(name));
	}

	#[tokio::test]
	async fn a_server_publishing_bars_name_as_its_own_is_refused() {
		let (server, vetter) = served("mods:test,bbbb,,Beyond All Reason test-1\n").await;
		let refused = vetter
			.vet(&format!("{}/theirs/repos.gz", server.uri()))
			.await;
		assert!(
			matches!(&refused, Err(Error::Shadows(name)) if name == "Beyond All Reason test-1"),
			"{refused:?}"
		);
	}

	#[tokio::test]
	async fn an_address_that_is_not_a_rapid_index_says_so() {
		let (server, vetter) = served("").await;
		Mock::given(method("GET"))
			.and(path("/page"))
			.respond_with(ResponseTemplate::new(200).set_body_string("<html>hello</html>"))
			.mount(&server)
			.await;
		let page = vetter.vet(&format!("{}/page", server.uri())).await;
		assert!(matches!(page, Err(Error::NotAnIndex(_))), "{page:?}");
		let missing = vetter
			.vet(&format!("{}/nothing/repos.gz", server.uri()))
			.await;
		assert!(matches!(missing, Err(Error::Fetch { .. })), "{missing:?}");
	}

	#[tokio::test]
	async fn bars_own_master_needs_no_looking_at() {
		let (server, vetter) = served("").await;
		vetter
			.vet(&format!("{}/bar/repos.gz", server.uri()))
			.await
			.unwrap();
		assert!(server.received_requests().await.unwrap().is_empty());
	}
}
