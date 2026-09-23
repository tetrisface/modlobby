//! Where a game is, when the room's server's rapid does not have it.
//!
//! Three lists, most trusted first: the player's own overrides, each for one
//! exact game name; the list shipped with modlobby, which we answer for; and
//! coilbox's hub list, which any lobby can read and any game can be added
//! to, and which reaches games ours does not name. Ours goes before the hub
//! so what modlobby fetches is modlobby's to govern: an entry the hub adds
//! or changes cannot redirect a game we already know. All three use the
//! hub's shape (`{kind: rapid|url|github|…, value, asset?, filename?}`), so
//! a developer learns one way to say where their game is. When none of
//! them has it, springfiles is asked by name ([`springfiles`]).
//!
//! The hub says where a game lives, never which file is which version, so
//! matching a room's version to a release is [`pick`]'s job. Everything here
//! decides without the network; fetching and checking what was fetched are
//! the caller's.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use settings::model::SPRINGRTS_RAPID;

use crate::api::Api;

/// coilbox's hub (`src/hub/config.ts` `DEFAULT_HUB_URL`).
pub const HUB_URL: &str = "https://coilbox-hub.vercel.app/api/v1/games";

/// How long a fetched hub list is trusted.
pub const FRESH_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// Under the config directory's `cache/`.
pub const CACHE_FILE: &str = "hub-games.json";

/// The list modlobby ships, in the hub's shape.
const SHIPPED: &str = include_str!("../data/games.json");

pub use settings::model::GameSource as Target;

/// A game as a list names it: the title its versions start with, and where
/// to look, best first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
	pub title: String,
	pub downloads: Vec<Target>,
}

/// Which list an answer came from, for the person reading where a game was
/// looked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
	Override,
	Shipped,
	Hub,
}

/// Where a room's game is to be had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
	/// What the version is to be matched against: what follows the title,
	/// or for an override, which knows no title, the name's last word -- the
	/// engine names a game `<name> <version>`.
	pub version: String,
	pub targets: Vec<Target>,
	pub via: Via,
}

/// Where `name` is to be had, in the order to try: an override for exactly
/// that name, alone -- it is the player's word and nothing is tried beside
/// it; else our entry, then the hub's, so the hub is still asked when ours
/// cannot produce the version. An entry is a game's when the name is its
/// title and a space and then the version, the longest such title winning.
/// An entry with nothing to download from has nothing to say.
pub fn resolve(
	name: &str,
	overrides: &[(String, Target)],
	shipped: &[Entry],
	hub: &[Entry],
) -> Vec<Found> {
	if let Some((_, target)) = overrides.iter().find(|(exact, _)| exact == name) {
		return vec![Found {
			version: name.rsplit(' ').next().unwrap_or(name).to_owned(),
			targets: vec![target.clone()],
			via: Via::Override,
		}];
	}
	let from = |list: &[Entry], via: Via| {
		list.iter()
			.filter(|entry| !entry.downloads.is_empty())
			.filter_map(|entry| Some((entry, version_of(name, &entry.title)?)))
			.max_by_key(|(entry, _)| entry.title.len())
			.map(|(entry, version)| Found {
				version: version.to_owned(),
				targets: entry.downloads.clone(),
				via,
			})
	};
	[from(shipped, Via::Shipped), from(hub, Via::Hub)]
		.into_iter()
		.flatten()
		.collect()
}

/// What follows `title` and a space in `name`, if that is how it starts.
fn version_of<'a>(name: &'a str, title: &str) -> Option<&'a str> {
	let version = name.strip_prefix(title)?.strip_prefix(' ')?;
	(!version.trim().is_empty()).then_some(version)
}

/// A release, as far as picking a file goes, in GitHub's shape: a Forgejo
/// writes the same, and a GitLab's is read into it ([`parse_gitlab`]).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
	#[serde(rename = "tag_name")]
	pub tag: String,
	pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
	pub name: String,
	#[serde(rename = "browser_download_url")]
	pub url: String,
	pub size: u64,
	/// `sha256:<hex>`, on every asset GitHub has hashed: what the file must
	/// be byte for byte once it has arrived. No other forge gives one.
	#[serde(default)]
	pub digest: Option<String>,
}

impl Asset {
	/// The SHA-256 GitHub gave, in hex.
	pub fn sha256(&self) -> Option<&str> {
		self.digest.as_deref()?.strip_prefix("sha256:")
	}
}

/// GitHub's release list for `repo` (`owner/repo`) under `api`
/// ([`crate::api::GITHUB_API`]), newest first; `None` for anything that is not one
/// repository's name, so a list entry cannot steer the request anywhere else.
pub fn releases_url(api: &str, repo: &str) -> Option<String> {
	is_repo(repo).then(|| format!("{api}/repos/{repo}/releases?per_page=100"))
}

/// Whether `repo` is one repository's name, `owner/repo`, and nothing that
/// would steer a request anywhere else.
pub(crate) fn is_repo(repo: &str) -> bool {
	let fits = |part: &str| {
		!part.is_empty()
			&& part != "."
			&& part != ".."
			&& part
				.chars()
				.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
	};
	repo.split_once('/')
		.is_some_and(|(owner, name)| fits(owner) && fits(name))
}

/// The releases in GitHub's answer; nothing for anything else.
pub fn parse_releases(body: &str) -> Vec<Release> {
	serde_json::from_str(body).unwrap_or_default()
}

/// The file among `releases` that is `version`: in the release tagged the
/// version (or `v` and the version), the one archive, else the archive whose
/// name has `fragment`, else the one whose name has the version; failing a
/// tag, any archive whose name has the version (and `fragment`, if given).
/// Only archives the engine loads count.
pub fn pick<'a>(
	releases: &'a [Release],
	version: &str,
	fragment: Option<&str>,
) -> Option<&'a Asset> {
	let archives = |release: &'a Release| {
		release
			.assets
			.iter()
			.filter(|asset| is_archive(&asset.name))
			.filter(move |asset| fragment.is_none_or(|part| asset.name.contains(part)))
	};
	let tagged = releases.iter().find(|release| {
		let tag = release.tag.strip_prefix('v').unwrap_or(&release.tag);
		tag == version
	});
	if let Some(release) = tagged {
		let candidates: Vec<&Asset> = archives(release).collect();
		if let [only] = candidates[..] {
			return Some(only);
		}
		if let Some(named) = candidates.iter().find(|asset| asset.name.contains(version)) {
			return Some(named);
		}
	}
	releases
		.iter()
		.flat_map(archives)
		.find(|asset| asset.name.contains(version))
}

fn is_archive(name: &str) -> bool {
	let name = name.to_ascii_lowercase();
	name.ends_with(".sdz") || name.ends_with(".sd7")
}

/// How long a half-downloaded game is kept for resuming.
const STALE_PART_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
	/// A name that is a path, or not an archive the engine loads: never
	/// written anywhere.
	#[error("{0} is not a game archive's name")]
	NotAnArchive(String),
	#[error(transparent)]
	Fetch(#[from] crate::fetch::Error),
	#[error("{name} arrived as {got} bytes, not the {want} its release gave; it was discarded")]
	Size { name: String, got: u64, want: u64 },
	#[error("{name}'s SHA-256 ({got}) is not the one its release gave ({want}); it was discarded")]
	Digest {
		name: String,
		got: String,
		want: String,
	},
	#[error("installing {name}: {reason}")]
	Io { name: String, reason: String },
}

/// Fetches `asset` into `games`: staged as `.<name>.part`, resumed if an
/// earlier attempt left one, checked against the size and SHA-256 its
/// release gave, and only then moved into place, so the engine never finds half a
/// game or the wrong one. The installed file's path.
pub async fn install(
	http: &reqwest::Client,
	asset: &Asset,
	games: &Path,
	report: impl FnMut(u64, u64),
) -> Result<PathBuf, InstallError> {
	let name = asset.name.as_str();
	let plain = !name.starts_with('.') && !name.contains(['/', '\\', ':']) && is_archive(name);
	if !plain {
		return Err(InstallError::NotAnArchive(name.to_owned()));
	}
	let io = |err: std::io::Error| InstallError::Io {
		name: name.to_owned(),
		reason: err.to_string(),
	};
	std::fs::create_dir_all(games).map_err(io)?;
	crate::fetch::sweep_stale_parts(games, STALE_PART_AFTER);
	let staging = games.join(format!(".{name}.part"));
	crate::fetch::resumable(http, &asset.url, &staging, asset.size, report).await?;

	let discard = |refused: InstallError| {
		let _ = std::fs::remove_file(&staging);
		Err(refused)
	};
	let got = std::fs::metadata(&staging).map_err(io)?.len();
	if asset.size > 0 && got != asset.size {
		return discard(InstallError::Size {
			name: name.to_owned(),
			got,
			want: asset.size,
		});
	}
	if let Some(want) = asset.sha256() {
		let path = staging.clone();
		let got =
			tokio::task::spawn_blocking(move || crate::fetch::hash_file::<sha2::Sha256>(&path))
				.await
				.map_err(|err| InstallError::Io {
					name: name.to_owned(),
					reason: err.to_string(),
				})?
				.map_err(io)?;
		if !got.eq_ignore_ascii_case(want) {
			return discard(InstallError::Digest {
				name: name.to_owned(),
				got,
				want: want.to_owned(),
			});
		}
	}
	let installed = games.join(name);
	std::fs::rename(&staging, &installed).map_err(io)?;
	tracing::info!(path = %installed.display(), "game installed");
	Ok(installed)
}

/// A game fetched from outside rapid: where it came from, for a person to
/// read, and where it now is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
	pub from: String,
	pub path: PathBuf,
	/// Built here from a commit rather than fetched as published, and so
	/// kept only once it is proved to be the room's copy.
	pub built: bool,
}

/// Fetching the game by rapid from another master, as a list's rapid entry
/// asks: the caller's to do, since it runs pr-downloader. Where the game
/// then is.
pub type Rapid<'a> =
	dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<PathBuf, String>> + Send>> + Sync + 'a;

/// Tries each answer's downloads, in [`resolve`]'s order, until one installs
/// into `games`: what came and from where, or every reason none did. `None`
/// when there was nothing to try. `name` is the game as the room names it,
/// which a release is held to before it is downloaded.
pub async fn fetch(
	http: &reqwest::Client,
	api: &Api,
	name: &str,
	found: &[Found],
	games: &Path,
	rapid: &Rapid<'_>,
	mut report: impl FnMut(u64, u64),
) -> Option<Result<Fetched, String>> {
	let mut failures = Vec::new();
	for answer in found {
		for target in &answer.targets {
			let tried = match target {
				Target::Github { asset, .. }
				| Target::Gitlab { asset, .. }
				| Target::Forgejo { asset, .. } => {
					let want = Want {
						name,
						version: &answer.version,
						fragment: asset.as_deref(),
					};
					match listing(&api.github, target) {
						Some(listing) => {
							released(http, api, &listing, &want, games, &mut report).await
						}
						None => Err(format!("{target:?} names no repository this can ask")),
					}
				}
				Target::Url { value, filename } => {
					url(http, value, filename.as_deref(), name, games, &mut report).await
				}
				Target::Git { value, placeholder } => {
					let placeholder = placeholder.as_deref().unwrap_or(crate::git::PLACEHOLDER);
					crate::git::build(
						http,
						api,
						value,
						&answer.version,
						placeholder,
						games,
						&mut report,
					)
					.await
					.map(|path| (format!("git {value}, built from {}", answer.version), path))
				}
				Target::Rapid { value } => rapid(SPRINGRTS_RAPID.to_owned())
					.await
					.map(|path| (format!("rapid {value} at {SPRINGRTS_RAPID}"), path))
					.map_err(|err| format!("rapid {value} at {SPRINGRTS_RAPID}: {err}")),
			};
			match tried {
				Ok((from, path)) => {
					let built = matches!(target, Target::Git { .. });
					let from = format!("{from}, from {}", via_words(answer.via));
					return Some(Ok(Fetched { from, path, built }));
				}
				Err(reason) => failures.push(reason),
			}
		}
	}
	(!failures.is_empty()).then(|| Err(failures.join("; ")))
}

fn via_words(via: Via) -> &'static str {
	match via {
		Via::Override => "your override",
		Via::Shipped => "modlobby's list",
		Via::Hub => "coilbox's hub",
	}
}

/// Which release is wanted: the room's game, its version, and a fragment of
/// the file's name when a list gave one.
struct Want<'a> {
	name: &'a str,
	version: &'a str,
	fragment: Option<&'a str>,
}

/// Where a repository's releases are listed, and how the list is written.
#[derive(Debug, PartialEq, Eq)]
struct Listing {
	label: String,
	url: String,
	gitlab: bool,
}

/// The release list a target names, under `github_api` for GitHub; `None`
/// for an address that is not one repository's, so a list entry cannot
/// steer a request anywhere else.
fn listing(github_api: &str, target: &Target) -> Option<Listing> {
	match target {
		Target::Github { value, .. } => Some(Listing {
			label: format!("github {value}"),
			url: releases_url(github_api, value)?,
			gitlab: false,
		}),
		Target::Gitlab { value, .. } => {
			let (origin, path) = forge_repo(value, "gitlab.com", false)?;
			Some(Listing {
				label: format!("gitlab {value}"),
				url: format!(
					"{origin}/api/v4/projects/{}/releases?per_page=100",
					path.replace('/', "%2F")
				),
				gitlab: true,
			})
		}
		Target::Forgejo { value, .. } => {
			let (origin, path) = forge_repo(value, "codeberg.org", true)?;
			Some(Listing {
				label: format!("forgejo {value}"),
				url: format!("{origin}/api/v1/repos/{path}/releases?limit=50"),
				gitlab: false,
			})
		}
		_ => None,
	}
}

/// `https://<host>` and a repository's path out of `value`: an https
/// address, or a bare path on `host`. Plain names only; exactly two of them
/// when `two`, else two or more, as a GitLab's subgroups nest.
fn forge_repo(value: &str, host: &str, two: bool) -> Option<(String, String)> {
	let (host, path) = match value.strip_prefix("https://") {
		Some(rest) => rest.split_once('/')?,
		None => (host, value),
	};
	let path = path.trim_end_matches('/').trim_end_matches(".git");
	let host_fits = !host.is_empty()
		&& host
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
	let parts: Vec<&str> = path.split('/').collect();
	let parts_fit = parts.iter().all(|part| {
		!part.is_empty()
			&& *part != "."
			&& *part != ".."
			&& part
				.chars()
				.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
	});
	let count_fits = if two {
		parts.len() == 2
	} else {
		parts.len() >= 2
	};
	(host_fits && parts_fit && count_fits).then(|| (format!("https://{host}"), path.to_owned()))
}

/// A GitLab release list, as the releases GitHub writes: each link a file,
/// its size unknown until it arrives.
pub fn parse_gitlab(body: &str) -> Vec<Release> {
	#[derive(Deserialize)]
	struct Listed {
		tag_name: String,
		#[serde(default)]
		assets: Assets,
	}
	#[derive(Default, Deserialize)]
	struct Assets {
		#[serde(default)]
		links: Vec<Link>,
	}
	#[derive(Deserialize)]
	struct Link {
		name: String,
		url: String,
		#[serde(default)]
		direct_asset_url: Option<String>,
	}
	let listed: Vec<Listed> = serde_json::from_str(body).unwrap_or_default();
	listed
		.into_iter()
		.map(|release| Release {
			tag: release.tag_name,
			assets: release
				.assets
				.links
				.into_iter()
				.map(|link| Asset {
					name: link.name,
					url: link.direct_asset_url.unwrap_or(link.url),
					size: 0,
					digest: None,
				})
				.collect(),
		})
		.collect()
}

async fn released(
	http: &reqwest::Client,
	api: &Api,
	listing: &Listing,
	want: &Want<'_>,
	games: &Path,
	report: &mut impl FnMut(u64, u64),
) -> Result<(String, PathBuf), String> {
	let failed = |reason: String| format!("{}: {reason}", listing.label);
	let body = api
		.text(http, &listing.url, "the release list")
		.await
		.map_err(failed)?;
	let releases = if listing.gitlab {
		parse_gitlab(&body)
	} else {
		parse_releases(&body)
	};
	let asset = pick(&releases, want.version, want.fragment)
		.ok_or_else(|| failed(format!("no release has {}", want.version)))?;
	names_itself(http, asset, want.name).await.map_err(failed)?;
	let path = install(http, asset, games, report)
		.await
		.map_err(|err| failed(err.to_string()))?;
	Ok((format!("{}, {}", listing.label, asset.name), path))
}

async fn url(
	http: &reqwest::Client,
	value: &str,
	filename: Option<&str>,
	name: &str,
	games: &Path,
	report: &mut impl FnMut(u64, u64),
) -> Result<(String, PathBuf), String> {
	if !value.starts_with("https://") {
		return Err(format!("{value} is not served over https"));
	}
	let file = filename
		.or_else(|| value.split(['?', '#']).next()?.rsplit('/').next())
		.unwrap_or_default();
	let asset = Asset {
		name: file.to_owned(),
		url: value.to_owned(),
		size: 0,
		digest: None,
	};
	let failed = |reason: String| format!("{value}: {reason}");
	names_itself(http, &asset, name).await.map_err(failed)?;
	let path = install(http, &asset, games, report)
		.await
		.map_err(|err| failed(err.to_string()))?;
	Ok((value.to_owned(), path))
}

/// A `.sdz` whose `modinfo.lua` names another game than `name` is refused
/// before any more of it is downloaded. One that cannot be looked into --
/// a `.sd7`, or a server without ranges -- is fetched and held to the
/// room's hash as ever.
async fn names_itself(http: &reqwest::Client, asset: &Asset, name: &str) -> Result<(), String> {
	if !asset.name.to_ascii_lowercase().ends_with(".sdz") {
		return Ok(());
	}
	let modinfo = match crate::peek::root_file(http, &asset.url, "modinfo.lua").await {
		Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
		Err(why) => {
			tracing::debug!(asset = asset.name, %why, "not looked into before downloading");
			return Ok(());
		}
	};
	match crate::map_name::compose(&modinfo) {
		Some(own) if own != name => Err(format!(
			"{} is {own}, not {name}, by its modinfo.lua; not downloaded",
			asset.name
		)),
		_ => Ok(()),
	}
}

/// The file springfiles' search (`json.php`, pr-downloader's default) lists
/// as the game named exactly `name`, from its first mirror served over
/// https, with the size it gave. An entry this cannot read is skipped, not
/// the answer.
pub fn parse_springfiles(body: &str, name: &str) -> Option<Asset> {
	#[derive(Deserialize)]
	struct Listed {
		springname: String,
		category: String,
		filename: String,
		size: u64,
		mirrors: Vec<String>,
	}
	let listed: Vec<Value> = serde_json::from_str(body).unwrap_or_default();
	listed
		.into_iter()
		.filter_map(|one| serde_json::from_value::<Listed>(one).ok())
		.filter(|file| file.category == "game" && file.springname == name)
		.find_map(|file| {
			let url = file
				.mirrors
				.into_iter()
				.find(|mirror| mirror.starts_with("https://"))?;
			Some(Asset {
				name: file.filename,
				url,
				size: file.size,
				digest: None,
			})
		})
}

/// `name` from springfiles' search at `search`: nobody's server, and where
/// Recoil's games have been uploaded for fifteen years. It is asked by name,
/// so the caller asks only for a name known not to be BAR's.
pub async fn springfiles(
	http: &reqwest::Client,
	search: &str,
	name: &str,
	games: &Path,
	report: impl FnMut(u64, u64),
) -> Result<Fetched, String> {
	let failed = |reason: String| format!("springfiles: {reason}");
	let url =
		reqwest::Url::parse_with_params(search, [("springname", name), ("category", "game")])
			.map_err(|err| failed(err.to_string()))?;
	let body = async { http.get(url).send().await?.error_for_status()?.text().await }
		.await
		.map_err(|err| failed(err.to_string()))?;
	let asset =
		parse_springfiles(&body, name).ok_or_else(|| format!("springfiles has no {name}"))?;
	names_itself(http, &asset, name).await.map_err(failed)?;
	let path = install(http, &asset, games, report)
		.await
		.map_err(|err| failed(err.to_string()))?;
	Ok(Fetched {
		from: format!("springfiles, {}", asset.name),
		path,
		built: false,
	})
}

/// Where a game that is not the room's copy is kept: out of every folder
/// the engine scans (`games/`, `maps/`, `packages/`, `base/`), so it is
/// never loaded, and still there to look at or delete.
pub const QUARANTINE: &str = "modlobby-quarantine";

/// Moves `game` out of the engine's sight, under `data_dir`'s
/// [`QUARANTINE`]; where it went.
pub fn set_aside(game: &Path, data_dir: &Path) -> std::io::Result<PathBuf> {
	let dir = data_dir.join(QUARANTINE);
	std::fs::create_dir_all(&dir)?;
	let aside = dir.join(game.file_name().unwrap_or_default());
	std::fs::rename(game, &aside)?;
	Ok(aside)
}

/// The entries in a list in the hub's shape. A download of a kind this build
/// does not know is left out rather than failing the list: the hub may learn
/// new kinds before modlobby does.
pub fn parse(text: &str) -> Vec<Entry> {
	let Ok(list) = serde_json::from_str::<Value>(text) else {
		return Vec::new();
	};
	let Some(games) = list["games"].as_array() else {
		return Vec::new();
	};
	games
		.iter()
		.filter_map(|game| {
			let title = game["title"].as_str()?.trim();
			let downloads = game["downloads"]
				.as_array()
				.map(|all| {
					all.iter()
						.filter_map(|one| serde_json::from_value(one.clone()).ok())
						.collect()
				})
				.unwrap_or_default();
			(!title.is_empty()).then(|| Entry {
				title: title.to_owned(),
				downloads,
			})
		})
		.collect()
}

/// The list shipped with this build.
pub fn shipped() -> Vec<Entry> {
	parse(SHIPPED)
}

/// What the cache file holds: the hub's answer as it came, and when.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
	fetched_at: u64,
	body: String,
}

/// The hub list held on disk, and whether it is fresh; nothing, not fresh,
/// when there is none.
pub fn cached(cache_dir: &Path, now: SystemTime) -> (Vec<Entry>, bool) {
	match read(&cache_dir.join(CACHE_FILE)) {
		Some(held) => (
			parse(&held.body),
			seconds(now).saturating_sub(held.fetched_at) < FRESH_FOR.as_secs(),
		),
		None => (Vec::new(), false),
	}
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error(transparent)]
	Http(#[from] reqwest::Error),
	#[error("HTTP {0}")]
	Status(u16),
	#[error("not the hub's games list")]
	NotAList,
}

/// Fetches the hub's list, keeps it, and says what it holds. Not kept unless
/// it reads as a list, so a portal's page never replaces a good copy.
pub async fn refresh(
	client: &reqwest::Client,
	url: &str,
	cache_dir: &Path,
	now: SystemTime,
) -> Result<Vec<Entry>, Error> {
	let response = client.get(url).send().await?;
	let status = response.status();
	if !status.is_success() {
		return Err(Error::Status(status.as_u16()));
	}
	let body = response.text().await?;
	let is_list = serde_json::from_str::<Value>(&body).is_ok_and(|list| list["games"].is_array());
	if !is_list {
		return Err(Error::NotAList);
	}
	let entries = parse(&body);
	write(
		&cache_dir.join(CACHE_FILE),
		&Cached {
			fetched_at: seconds(now),
			body,
		},
	);
	Ok(entries)
}

fn read(path: &Path) -> Option<Cached> {
	let text = std::fs::read_to_string(path).ok()?;
	match serde_json::from_str(&text) {
		Ok(cached) => Some(cached),
		Err(err) => {
			tracing::warn!(%err, path = %path.display(), "hub games cache not readable");
			None
		}
	}
}

/// Temp file and rename, so a crash never leaves half a list behind.
fn write(path: &Path, cached: &Cached) {
	let written = (|| {
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let tmp: PathBuf = path.with_extension("json.tmp");
		std::fs::write(&tmp, serde_json::to_vec(cached)?)?;
		std::fs::rename(&tmp, path)
	})();
	if let Err(err) = written {
		tracing::warn!(%err, path = %path.display(), "hub games cache not written");
	}
}

fn seconds(time: SystemTime) -> u64 {
	time.duration_since(UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

#[cfg(test)]
mod tests {
	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;
	use crate::api::GITHUB_API;

	fn rapid_nowhere(
		master: String,
	) -> Pin<Box<dyn Future<Output = Result<PathBuf, String>> + Send>> {
		Box::pin(async move { Err(format!("nothing at {master}")) })
	}

	fn github(repo: &str) -> Target {
		Target::Github {
			value: repo.into(),
			asset: None,
		}
	}

	fn entry(title: &str, downloads: Vec<Target>) -> Entry {
		Entry {
			title: title.into(),
			downloads,
		}
	}

	fn asset(name: &str) -> Asset {
		Asset {
			name: name.into(),
			url: format!("https://github.com/x/y/releases/download/t/{name}"),
			size: 1,
			digest: None,
		}
	}

	fn release(tag: &str, assets: &[&str]) -> Release {
		Release {
			tag: tag.into(),
			assets: assets.iter().map(|name| asset(name)).collect(),
		}
	}

	#[test]
	fn the_shipped_list_reads_and_knows_splinterfaction() {
		let [found] = &resolve("SplinterFaction 0.1.86", &[], &shipped(), &[])[..] else {
			panic!("one answer");
		};
		assert_eq!(found.version, "0.1.86");
		assert_eq!(found.via, Via::Shipped);
		assert_eq!(found.targets, [github("SplinterFaction/SplinterFaction")]);
	}

	#[test]
	fn an_override_for_the_exact_name_wins_and_nothing_else_is_one() {
		let mine = github("fork/SplinterFaction");
		let overrides = vec![("SplinterFaction 0.1.86".to_owned(), mine.clone())];
		let shipped = [entry(
			"SplinterFaction",
			vec![github("SplinterFaction/SplinterFaction")],
		)];

		let found = resolve("SplinterFaction 0.1.86", &overrides, &shipped, &shipped);
		assert_eq!(
			found,
			[Found {
				version: "0.1.86".into(),
				targets: vec![mine],
				via: Via::Override
			}],
			"alone: nothing is tried beside it"
		);
		// Strict: a neighbouring version is not the override's.
		let next = resolve("SplinterFaction 0.1.87", &overrides, &shipped, &[]);
		assert_eq!(next[0].via, Via::Shipped);
	}

	#[test]
	fn ours_is_tried_before_the_hub_and_the_hub_after_it() {
		let hub_says = vec![Target::Rapid {
			value: "sf:stable".into(),
		}];
		let ours = vec![github("SplinterFaction/SplinterFaction")];
		let shipped = [entry("SplinterFaction", ours.clone())];
		let hub = [entry("SplinterFaction", hub_says.clone())];

		let found = resolve("SplinterFaction 0.1.86", &[], &shipped, &hub);
		let order: Vec<_> = found.iter().map(|f| (f.via, f.targets.clone())).collect();
		assert_eq!(order, [(Via::Shipped, ours), (Via::Hub, hub_says)]);

		// A game only the hub knows is still found.
		let found = resolve("SplinterFaction 0.1.86", &[], &[], &hub);
		assert_eq!(found[0].via, Via::Hub);

		// The hub as it was on 2026-09-22: the game listed, nothing to fetch.
		let empty = [entry("SplinterFaction", vec![])];
		let found = resolve("SplinterFaction 0.1.86", &[], &shipped, &empty);
		assert_eq!(found.len(), 1);
	}

	#[test]
	fn a_title_is_a_whole_word_and_the_longest_one_wins() {
		let list = [
			entry("Tech", vec![github("a/tech")]),
			entry("Tech Annihilation", vec![github("b/ta")]),
		];
		let found = &resolve("Tech Annihilation test-12065-72535dc", &[], &list, &[])[0];
		assert_eq!(found.targets, [github("b/ta")]);
		assert_eq!(found.version, "test-12065-72535dc");
		assert!(resolve("Techno 1.0", &[], &list, &[]).is_empty());
		assert!(resolve("Tech", &[], &list, &[]).is_empty(), "no version");
	}

	#[test]
	fn the_release_tagged_the_version_gives_its_archive() {
		// SplinterFaction's own, 2026-09-18: one release, one file.
		let releases = [
			release("0.1.87", &["SplinterFaction_0.1.87.sdz"]),
			release("0.1.86", &["SplinterFaction_0.1.86.sdz"]),
		];
		assert_eq!(
			pick(&releases, "0.1.86", None).map(|a| a.name.as_str()),
			Some("SplinterFaction_0.1.86.sdz")
		);
		let prefixed = [release("v2.1", &["notes.txt", "game-2.1.sd7"])];
		assert_eq!(
			pick(&prefixed, "2.1", None).map(|a| a.name.as_str()),
			Some("game-2.1.sd7")
		);
	}

	#[test]
	fn several_archives_are_told_apart_by_the_fragment_then_the_version() {
		let releases = [release(
			"1.0",
			&["mod-1.0-lite.sdz", "mod-1.0-full.sdz", "source.zip"],
		)];
		assert_eq!(
			pick(&releases, "1.0", Some("full")).map(|a| a.name.as_str()),
			Some("mod-1.0-full.sdz")
		);
		assert_eq!(
			pick(&releases, "1.0", None).map(|a| a.name.as_str()),
			Some("mod-1.0-lite.sdz")
		);
	}

	#[test]
	fn without_a_tag_any_archive_named_for_the_version_will_do_and_nothing_else() {
		let releases = [release("nightly", &["mod-test-5-abc1234.sdz"])];
		assert_eq!(
			pick(&releases, "test-5-abc1234", None).map(|a| a.name.as_str()),
			Some("mod-test-5-abc1234.sdz")
		);
		assert_eq!(pick(&releases, "test-6-def5678", None), None);
		assert_eq!(pick(&[release("1.0", &["source.zip"])], "1.0", None), None);
	}

	#[test]
	fn a_release_list_reads_as_github_gives_it_and_only_a_repo_is_asked_for() {
		// SplinterFaction 0.1.86 as the API gave it on 2026-09-22, trimmed.
		let releases = parse_releases(
			r#"[{ "tag_name": "0.1.86", "name": "0.1.86", "assets": [ {
				"name": "SplinterFaction_0.1.86.sdz", "size": 806387536,
				"browser_download_url": "https://github.com/SplinterFaction/SplinterFaction/releases/download/0.1.86/SplinterFaction_0.1.86.sdz",
				"digest": "sha256:ab12" } ] }]"#,
		);
		let found = pick(&releases, "0.1.86", None).unwrap();
		assert_eq!(found.size, 806_387_536);
		assert_eq!(found.sha256(), Some("ab12"));
		assert!(parse_releases("<html>").is_empty());

		assert_eq!(
			releases_url(GITHUB_API, "SplinterFaction/SplinterFaction").as_deref(),
			Some(
				"https://api.github.com/repos/SplinterFaction/SplinterFaction/releases?per_page=100"
			)
		);
		for steered in [
			"a/b/c", "a", "../x", "a/..", "a/b?x=1", "a/b#c", "a /b", "/b",
		] {
			assert_eq!(releases_url(GITHUB_API, steered), None, "{steered}");
		}
	}

	#[test]
	fn a_download_of_a_kind_not_known_yet_is_left_out_not_the_list() {
		let list = parse(
			r#"{ "format": "coilbox-hub-games", "version": 1, "games": [
				{ "shortname": "EvoRTS", "title": "Evolution RTS",
				  "downloads": [ { "kind": "torrent", "value": "magnet:x" }, { "kind": "rapid", "value": "evo:stable" } ] },
				{ "shortname": "SF", "title": "SplinterFaction", "downloads": [] },
				{ "shortname": "nameless", "downloads": [] }
			] }"#,
		);
		assert_eq!(
			list,
			[
				entry(
					"Evolution RTS",
					vec![Target::Rapid {
						value: "evo:stable".into()
					}]
				),
				entry("SplinterFaction", vec![]),
			]
		);
		assert!(parse("<html>").is_empty());
	}

	/// A game served the way GitHub serves a release asset.
	async fn served(body: &'static [u8]) -> (MockServer, Asset) {
		use md5::Digest;
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/download/Game_1.0.sdz"))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(body))
			.mount(&server)
			.await;
		let asset = Asset {
			name: "Game_1.0.sdz".into(),
			url: format!("{}/download/Game_1.0.sdz", server.uri()),
			size: body.len() as u64,
			digest: Some(format!(
				"sha256:{}",
				crate::fetch::hex(&sha2::Sha256::digest(body))
			)),
		};
		(server, asset)
	}

	#[tokio::test]
	async fn a_game_is_installed_only_once_it_is_what_github_said() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		let client = crate::http::client("test");

		let (_server, asset) = served(b"a game").await;
		let installed = install(&client, &asset, &games, |_, _| {}).await.unwrap();
		assert_eq!(installed, games.join("Game_1.0.sdz"));
		assert_eq!(std::fs::read(&installed).unwrap(), b"a game");
		assert!(!games.join(".Game_1.0.sdz.part").exists());

		// A file the server does not have leaves nothing behind.
		let (_server, mut missing) = served(b"a game").await;
		missing.name = "Other_1.0.sdz".into();
		missing.url = missing.url.replace("Game_1.0", "Other_1.0");
		let refused = install(&client, &missing, &games, |_, _| {}).await;
		assert!(
			matches!(
				refused,
				Err(InstallError::Fetch(crate::fetch::Error::Status(_)))
			),
			"{refused:?}"
		);
		assert!(!games.join("Other_1.0.sdz").exists());

		let (_server, mut bad_sha) = served(b"a game").await;
		bad_sha.digest = Some("sha256:00".into());
		assert!(matches!(
			install(&client, &bad_sha, &dir.path().join("elsewhere"), |_, _| {}).await,
			Err(InstallError::Digest { .. })
		));
		assert!(!dir.path().join("elsewhere").join("Game_1.0.sdz").exists());
	}

	#[tokio::test]
	async fn each_answer_is_tried_in_turn_and_the_first_that_installs_says_where_from() {
		use md5::Digest;
		let dir = tempfile::tempdir().unwrap();
		let api = MockServer::start().await;
		let body: &[u8] = b"splinter";
		let sha = crate::fetch::hex(&sha2::Sha256::digest(body));
		// Ours points at a repo without the version; the hub's has it.
		Mock::given(method("GET"))
			.and(path("/repos/ours/SF/releases"))
			.respond_with(ResponseTemplate::new(200).set_body_string("[]"))
			.expect(1)
			.mount(&api)
			.await;
		Mock::given(method("GET"))
			.and(path("/repos/theirs/SF/releases"))
			.respond_with(ResponseTemplate::new(200).set_body_string(format!(
				r#"[{{ "tag_name": "0.1.86", "assets": [ {{ "name": "SF_0.1.86.sdz", "size": {}, "digest": "sha256:{sha}",
				"browser_download_url": "{}/files/SF_0.1.86.sdz" }} ] }}]"#,
				body.len(),
				api.uri()
			)))
			.expect(1)
			.mount(&api)
			.await;
		// Asked twice: looked into first, which a server without ranges
		// refuses, and then downloaded.
		Mock::given(method("GET"))
			.and(path("/files/SF_0.1.86.sdz"))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(body))
			.expect(2)
			.mount(&api)
			.await;
		let found = resolve(
			"SplinterFaction 0.1.86",
			&[],
			&[entry("SplinterFaction", vec![github("ours/SF")])],
			&[entry("SplinterFaction", vec![github("theirs/SF")])],
		);
		let games = dir.path().join("games");
		let client = crate::http::client("test");
		let at = crate::api::Api::at(&api.uri());
		let name = "SplinterFaction 0.1.86";
		let got = fetch(
			&client,
			&at,
			name,
			&found,
			&games,
			&rapid_nowhere,
			|_, _| {},
		)
		.await;
		assert_eq!(
			got,
			Some(Ok(Fetched {
				from: "github theirs/SF, SF_0.1.86.sdz, from coilbox's hub".into(),
				path: games.join("SF_0.1.86.sdz"),
				built: false,
			}))
		);
		assert_eq!(std::fs::read(games.join("SF_0.1.86.sdz")).unwrap(), body);

		assert_eq!(
			fetch(&client, &at, name, &[], &games, &rapid_nowhere, |_, _| {}).await,
			None
		);
		let refused = fetch(
			&client,
			&at,
			"X 1",
			&resolve(
				"X 1",
				&[(
					"X 1".into(),
					Target::Url {
						value: "http://x/X.sdz".into(),
						filename: None,
					},
				)],
				&[],
				&[],
			),
			&games,
			&rapid_nowhere,
			|_, _| {},
		)
		.await;
		assert_eq!(
			refused,
			Some(Err("http://x/X.sdz is not served over https".into()))
		);
	}

	#[tokio::test]
	async fn a_rapid_entry_is_fetched_from_the_community_master_by_the_caller() {
		let dir = tempfile::tempdir().unwrap();
		let client = crate::http::client("test");
		let found = resolve(
			"Evolution RTS v18.13",
			&[],
			&[],
			&[entry(
				"Evolution RTS",
				vec![Target::Rapid {
					value: "evo:stable".into(),
				}],
			)],
		);
		let package = dir.path().join("packages").join("e.sdp");
		let asked = std::sync::Mutex::new(Vec::new());
		let rapid =
			|master: String| -> Pin<Box<dyn Future<Output = Result<PathBuf, String>> + Send>> {
				asked.lock().unwrap().push(master);
				let package = package.clone();
				Box::pin(async move { Ok(package) })
			};
		let got = fetch(
			&client,
			&crate::api::Api::at("http://unused"),
			"Evolution RTS v18.13",
			&found,
			dir.path(),
			&rapid,
			|_, _| {},
		)
		.await;
		assert_eq!(
			got,
			Some(Ok(Fetched {
				from: format!("rapid evo:stable at {SPRINGRTS_RAPID}, from coilbox's hub"),
				path: package.clone(),
				built: false,
			}))
		);
		assert_eq!(*asked.lock().unwrap(), [SPRINGRTS_RAPID]);
	}

	#[test]
	fn each_forge_lists_its_releases_where_it_does_and_nowhere_else() {
		let target = |kind: &str, value: &str| match kind {
			"gitlab" => Target::Gitlab {
				value: value.into(),
				asset: None,
			},
			_ => Target::Forgejo {
				value: value.into(),
				asset: None,
			},
		};
		assert_eq!(
			listing(GITHUB_API, &target("gitlab", "group/sub/game")).map(|found| found.url),
			Some(
				"https://gitlab.com/api/v4/projects/group%2Fsub%2Fgame/releases?per_page=100"
					.into()
			)
		);
		assert_eq!(
			listing(
				GITHUB_API,
				&target("gitlab", "https://git.example.org/dev/game.git")
			)
			.map(|found| found.url),
			Some("https://git.example.org/api/v4/projects/dev%2Fgame/releases?per_page=100".into())
		);
		assert_eq!(
			listing(GITHUB_API, &target("forgejo", "dev/game")).map(|found| found.url),
			Some("https://codeberg.org/api/v1/repos/dev/game/releases?limit=50".into())
		);
		for steered in [
			"dev",
			"dev/game/extra",
			"http://codeberg.org/dev/game",
			"https://codeberg.org/dev/../x",
			"dev/game?x=1",
			"https://host@evil/dev/game",
		] {
			assert_eq!(
				listing(GITHUB_API, &target("forgejo", steered)),
				None,
				"{steered}"
			);
		}
		assert_eq!(
			listing(GITHUB_API, &target("gitlab", "https://x/../y")),
			None
		);
	}

	#[test]
	fn a_gitlab_release_list_reads_as_a_github_one() {
		let releases = parse_gitlab(
			r#"[{ "tag_name": "v2.58", "assets": { "count": 1, "sources": [], "links": [
				{ "name": "mf-2.58.sdz", "url": "https://gitlab.com/x", "direct_asset_url": "https://gitlab.com/direct" },
				{ "name": "notes.txt", "url": "https://gitlab.com/notes" } ] } }]"#,
		);
		assert_eq!(
			pick(&releases, "2.58", None).map(|asset| asset.url.as_str()),
			Some("https://gitlab.com/direct")
		);
		assert!(parse_gitlab("{}").is_empty());
	}

	/// springfiles' answer as it gave it for Vroom RTS on 2026-09-23, beside
	/// a map of that name, a plain-http mirror and an entry it cannot read.
	#[test]
	fn springfiles_gives_the_game_of_exactly_that_name_from_an_https_mirror() {
		let body = r#"[
			{"springname": "Vroom RTS v0.1.9.3.2", "category": "map", "filename": "vroom.sd7", "size": 1, "mirrors": ["https://springfiles.springrts.com/files/maps/vroom.sd7"]},
			{"springname": "Vroom RTS v0.1.9.3.2", "category": "game", "size": null},
			{"mainQueryTime": 0.04, "fid": 38142, "name": "Vroom RTS", "filename": "vroom_rts-v0.1.9.3.2.sdz", "path": "games", "md5": "4bcad4ca513da1cd9ce1124d4e573653", "version": "v0.1.9.3.2", "category": "game", "size": 96006889, "keywords": null, "mirrors": ["http://mirror.example/vroom_rts-v0.1.9.3.2.sdz", "https://springfiles.springrts.com/files/games/vroom_rts-v0.1.9.3.2.sdz"], "tags": [], "springname": "Vroom RTS v0.1.9.3.2"}
		]"#;
		assert_eq!(
			parse_springfiles(body, "Vroom RTS v0.1.9.3.2"),
			Some(Asset {
				name: "vroom_rts-v0.1.9.3.2.sdz".into(),
				url: "https://springfiles.springrts.com/files/games/vroom_rts-v0.1.9.3.2.sdz".into(),
				size: 96_006_889,
				digest: None,
			})
		);
		assert_eq!(parse_springfiles(body, "Vroom RTS v0.1.9.3"), None);
		assert_eq!(parse_springfiles("[]", "Vroom RTS v0.1.9.3.2"), None);
		assert_eq!(parse_springfiles("<html>", "Vroom RTS v0.1.9.3.2"), None);
	}

	#[tokio::test]
	async fn a_release_naming_another_game_is_refused_before_it_is_downloaded() {
		use crate::peek::tests::{Ranged, game_zip};
		let dir = tempfile::tempdir().unwrap();
		let server = MockServer::start().await;
		let wrong = game_zip("name = 'SplinterFaction'\nversion = '0.1.85'\n", true);
		Mock::given(method("GET"))
			.and(path("/releases"))
			.respond_with(ResponseTemplate::new(200).set_body_string(format!(
				r#"[{{ "tag_name": "v0.1.86", "assets": {{ "links": [ {{ "name": "SF.sdz", "url": "{}/files/SF.sdz" }} ] }} }}]"#,
				server.uri()
			)))
			.mount(&server)
			.await;
		Mock::given(method("GET"))
			.and(path("/files/SF.sdz"))
			.respond_with(Ranged(wrong))
			.mount(&server)
			.await;
		let listing = Listing {
			label: "gitlab dev/SF".into(),
			url: format!("{}/releases", server.uri()),
			gitlab: true,
		};
		let want = Want {
			name: "SplinterFaction 0.1.86",
			version: "0.1.86",
			fragment: None,
		};
		let client = crate::http::client("test");
		let api = crate::api::Api::at(&server.uri());
		let refused = released(&client, &api, &listing, &want, dir.path(), &mut |_, _| {}).await;
		assert_eq!(
			refused,
			Err("gitlab dev/SF: SF.sdz is SplinterFaction 0.1.85, not SplinterFaction 0.1.86, by its modinfo.lua; not downloaded".into())
		);
		assert!(
			std::fs::read_dir(dir.path()).unwrap().next().is_none(),
			"nothing written"
		);

		let right = Want {
			name: "SplinterFaction 0.1.85",
			version: "0.1.86",
			fragment: None,
		};
		let (from, installed) =
			released(&client, &api, &listing, &right, dir.path(), &mut |_, _| {})
				.await
				.unwrap();
		assert_eq!(from, "gitlab dev/SF, SF.sdz");
		assert_eq!(installed, dir.path().join("SF.sdz"));
	}

	#[test]
	fn a_game_set_aside_is_out_of_every_folder_the_engine_scans() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		std::fs::write(games.join("Wrong_1.0.sdz"), b"not the room's").unwrap();
		let aside = set_aside(&games.join("Wrong_1.0.sdz"), dir.path()).unwrap();
		assert_eq!(aside, dir.path().join(QUARANTINE).join("Wrong_1.0.sdz"));
		assert!(!games.join("Wrong_1.0.sdz").exists());
		assert_eq!(std::fs::read(aside).unwrap(), b"not the room's");
	}

	#[tokio::test]
	async fn a_name_that_is_a_path_or_not_a_game_is_never_written() {
		let dir = tempfile::tempdir().unwrap();
		let client = crate::http::client("test");
		for name in [
			"../evil.sdz",
			"sub/x.sdz",
			"c:x.sdz",
			".hidden.sdz",
			"readme.txt",
		] {
			let asset = Asset {
				name: name.into(),
				url: "https://example.invalid/x".into(),
				size: 1,
				digest: None,
			};
			assert!(
				matches!(
					install(&client, &asset, dir.path(), |_, _| {}).await,
					Err(InstallError::NotAnArchive(_))
				),
				"{name}"
			);
		}
	}

	#[tokio::test]
	async fn the_hub_list_is_kept_a_day_and_a_page_that_is_not_one_changes_nothing() {
		let dir = tempfile::tempdir().unwrap();
		let hub = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/api/v1/games"))
			.respond_with(ResponseTemplate::new(200).set_body_string(SHIPPED))
			.expect(1)
			.mount(&hub)
			.await;
		let client = crate::http::client("test");
		let now = SystemTime::now();

		assert_eq!(cached(dir.path(), now), (Vec::new(), false));
		let fetched = refresh(
			&client,
			&format!("{}/api/v1/games", hub.uri()),
			dir.path(),
			now,
		)
		.await
		.unwrap();
		assert_eq!(fetched, shipped());
		assert_eq!(cached(dir.path(), now), (shipped(), true));
		assert!(!cached(dir.path(), now + FRESH_FOR).1);

		let portal = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
			.expect(1)
			.mount(&portal)
			.await;
		let refused = refresh(&client, &portal.uri(), dir.path(), now).await;
		assert!(matches!(refused, Err(Error::NotAList)));
		assert_eq!(cached(dir.path(), now).0, shipped(), "the good copy stays");
	}
}
