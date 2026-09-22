//! Asking a forge's API -- GitHub's, a GitLab's, a Forgejo's -- what a game
//! source needs to know. An answer is kept with its ETag, so asking again is
//! answered "not modified", which costs nothing against GitHub's hourly
//! allowance for an address (60 requests without an account); and GitHub's
//! API hears a token when `GITHUB_TOKEN` holds one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// GitHub's API.
pub const GITHUB_API: &str = "https://api.github.com";

/// Where GitHub serves a file as committed, outside the API's allowance.
pub const GITHUB_RAW: &str = "https://raw.githubusercontent.com";

/// Under the cache directory.
pub const CACHE_DIR: &str = "forge";

/// Where forge requests go and what they carry.
#[derive(Debug, Clone)]
pub struct Api {
	/// GitHub's API root. Only requests under it carry the token.
	pub github: String,
	/// GitHub's raw file root ([`GITHUB_RAW`]).
	pub raw: String,
	/// Where answers are kept; `None` keeps nothing.
	pub cache: Option<PathBuf>,
	pub token: Option<String>,
}

impl Api {
	/// GitHub itself, with answers kept under `cache` and the token from
	/// `GITHUB_TOKEN`.
	pub fn github(cache: Option<PathBuf>) -> Self {
		Self {
			github: GITHUB_API.into(),
			raw: GITHUB_RAW.into(),
			cache,
			token: std::env::var("GITHUB_TOKEN")
				.ok()
				.filter(|token| !token.is_empty()),
		}
	}

	/// Everything at `base`, nothing kept: a test's server.
	pub fn at(base: &str) -> Self {
		Self {
			github: base.into(),
			raw: format!("{base}/raw"),
			cache: None,
			token: None,
		}
	}

	/// `url`'s answer as text, from the cache when the forge says it has not
	/// changed. `what` names it in a refusal.
	pub async fn text(
		&self,
		http: &reqwest::Client,
		url: &str,
		what: &str,
	) -> Result<String, String> {
		let kept = self.cache.as_deref().map(|dir| kept_at(dir, url));
		let held = kept.as_deref().and_then(read);
		let mut request = http.get(url);
		if url.starts_with(&self.github) {
			request = request.header(reqwest::header::ACCEPT, "application/vnd.github+json");
			if let Some(token) = &self.token {
				request = request.bearer_auth(token);
			}
		}
		if let Some(held) = &held {
			request = request.header(reqwest::header::IF_NONE_MATCH, &held.etag);
		}
		let response = request
			.send()
			.await
			.map_err(|err| format!("{what}: {err}"))?;
		if response.status() == reqwest::StatusCode::NOT_MODIFIED
			&& let Some(held) = held
		{
			return Ok(held.body);
		}
		if !response.status().is_success() {
			return Err(refused(what, &response));
		}
		let etag = response
			.headers()
			.get(reqwest::header::ETAG)
			.and_then(|value| value.to_str().ok())
			.map(str::to_owned);
		let body = response
			.text()
			.await
			.map_err(|err| format!("{what}: {err}"))?;
		if let (Some(path), Some(etag)) = (kept, etag) {
			write(
				&path,
				&Kept {
					etag,
					body: body.clone(),
				},
			);
		}
		Ok(body)
	}

	/// `url`'s answer read as `T`.
	pub async fn json<T: serde::de::DeserializeOwned>(
		&self,
		http: &reqwest::Client,
		url: &str,
		what: &str,
	) -> Result<T, String> {
		let body = self.text(http, url, what).await?;
		serde_json::from_str(&body).map_err(|err| format!("{what}: {err}"))
	}
}

/// Why a forge said no, in words. GitHub's allowance for an address without
/// an account is the usual reason, and its answer says when that comes back.
pub(crate) fn refused(what: &str, response: &reqwest::Response) -> String {
	let header = |name: &str| {
		response
			.headers()
			.get(name)
			.and_then(|value| value.to_str().ok())
	};
	let status = response.status();
	let spent =
		matches!(status.as_u16(), 403 | 429) && header("x-ratelimit-remaining") == Some("0");
	if !spent {
		return format!("{what} answered {status}");
	}
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |since| since.as_secs());
	let wait = header("x-ratelimit-reset")
		.and_then(|at| at.parse::<u64>().ok())
		.map(|at| at.saturating_sub(now).div_ceil(60).max(1))
		.map(|minutes| format!(" for another {minutes} min"))
		.unwrap_or_default();
	format!("GitHub's hourly allowance of requests for this address is used up{wait}")
}

/// An answer as kept.
#[derive(Serialize, Deserialize)]
struct Kept {
	etag: String,
	body: String,
}

/// One file per address, named by its hash.
fn kept_at(dir: &Path, url: &str) -> PathBuf {
	let hash = crate::fetch::hex(&Sha256::digest(url.as_bytes()));
	dir.join(format!("{}.json", &hash[..24]))
}

fn read(path: &Path) -> Option<Kept> {
	serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Temp file and rename, so a crash never leaves half an answer.
fn write(path: &Path, kept: &Kept) {
	let written = (|| {
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let tmp = path.with_extension("json.tmp");
		std::fs::write(&tmp, serde_json::to_vec(kept)?)?;
		std::fs::rename(&tmp, path)
	})();
	if let Err(err) = written {
		tracing::warn!(%err, path = %path.display(), "forge answer not kept");
	}
}

#[cfg(test)]
mod tests {
	use wiremock::matchers::{header, method};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;

	#[tokio::test]
	async fn an_unchanged_answer_comes_from_the_cache() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(header("if-none-match", "\"v1\""))
			.respond_with(ResponseTemplate::new(304))
			.expect(1)
			.mount(&server)
			.await;
		Mock::given(method("GET"))
			.respond_with(
				ResponseTemplate::new(200)
					.insert_header("etag", "\"v1\"")
					.set_body_string("[1]"),
			)
			.expect(1)
			.mount(&server)
			.await;
		let dir = tempfile::tempdir().unwrap();
		let api = Api {
			cache: Some(dir.path().to_path_buf()),
			..Api::at(&server.uri())
		};
		let http = crate::http::client("test");
		let url = format!("{}/repos/a/b/releases", server.uri());

		assert_eq!(api.text(&http, &url, "releases").await.unwrap(), "[1]");
		assert_eq!(api.text(&http, &url, "releases").await.unwrap(), "[1]");
	}

	#[tokio::test]
	async fn only_githubs_api_hears_the_token() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(header("authorization", "Bearer secret"))
			.respond_with(ResponseTemplate::new(200).set_body_string("with"))
			.mount(&server)
			.await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_string("without"))
			.mount(&server)
			.await;
		let http = crate::http::client("test");
		let api = Api {
			token: Some("secret".into()),
			..Api::at(&server.uri())
		};
		let elsewhere = Api {
			github: "https://api.github.com".into(),
			..api.clone()
		};
		let url = format!("{}/x", server.uri());

		assert_eq!(api.text(&http, &url, "x").await.unwrap(), "with");
		assert_eq!(elsewhere.text(&http, &url, "x").await.unwrap(), "without");
	}

	#[tokio::test]
	async fn a_spent_allowance_is_said_as_one() {
		let server = MockServer::start().await;
		let reset = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_secs()
			+ 600;
		Mock::given(method("GET"))
			.respond_with(
				ResponseTemplate::new(403)
					.insert_header("x-ratelimit-remaining", "0")
					.insert_header("x-ratelimit-reset", reset.to_string().as_str()),
			)
			.mount(&server)
			.await;
		let http = crate::http::client("test");
		let said = Api::at(&server.uri())
			.text(
				&http,
				&format!("{}/repos/a/b/commits/c", server.uri()),
				"the commit",
			)
			.await
			.unwrap_err();
		assert_eq!(
			said,
			"GitHub's hourly allowance of requests for this address is used up for another 10 min"
		);
	}
}
