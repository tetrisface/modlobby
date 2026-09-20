//! Whether an address looks like a map search: the springfiles `find` API
//! that pr-downloader asks for a map by name.
//!
//! The API has no "are you one" call, so the only thing to ask is for a map
//! that cannot exist. A search answers that with a 404 or an empty list;
//! a web page, a mistyped host or a server that is down answers otherwise.
//! That is all it can show — any address that 404s passes — which is enough
//! to catch what people actually get wrong when typing one in.

/// A name no server has a map under.
const NO_SUCH_MAP: &str = "modlobby-address-check-no-such-map";

/// `Ok` when `url` answers like a map search; the reason, for a person, when not.
pub async fn check(client: &reqwest::Client, url: &str) -> Result<(), String> {
	if !url.starts_with("https://") {
		return Err(format!("{url} is not served over https"));
	}
	judge(client, url).await
}

/// The asking itself, apart from the https rule so a test's plain-http
/// server can be put to it.
async fn judge(client: &reqwest::Client, url: &str) -> Result<(), String> {
	// The name needs no escaping, which is why it is what it is.
	let asked = format!("{url}?category=map&springname={NO_SUCH_MAP}");
	let response = client
		.get(&asked)
		.send()
		.await
		.map_err(|err| format!("{url} cannot be reached: {err}"))?;
	let status = response.status();
	if status == reqwest::StatusCode::NOT_FOUND {
		return Ok(());
	}
	if !status.is_success() {
		return Err(format!(
			"{url} answers {status}, which a map search does not"
		));
	}
	// Found nothing, said as a list of nothing.
	let body = response.text().await.unwrap_or_default();
	match serde_json::from_str::<Vec<serde_json::Value>>(&body) {
		Ok(_) => Ok(()),
		Err(_) => Err(format!("{url} answers with a page, not a map search")),
	}
}

#[cfg(test)]
mod tests {
	use wiremock::matchers::{method, path, query_param};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;

	async fn answers(template: ResponseTemplate) -> Result<(), String> {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/find"))
			.and(query_param("category", "map"))
			.respond_with(template)
			.mount(&server)
			.await;
		judge(
			&crate::http::client("test"),
			&format!("{}/find", server.uri()),
		)
		.await
	}

	#[tokio::test]
	async fn plain_http_is_refused_unasked() {
		let refused = check(&crate::http::client("test"), "http://maps.example/find").await;
		assert!(refused.unwrap_err().contains("https"));
	}

	#[tokio::test]
	async fn not_found_and_an_empty_list_are_a_search_and_a_page_is_not() {
		assert_eq!(answers(ResponseTemplate::new(404)).await, Ok(()));
		assert_eq!(
			answers(ResponseTemplate::new(200).set_body_string("[]")).await,
			Ok(())
		);
		assert!(
			answers(ResponseTemplate::new(200).set_body_string("<html>"))
				.await
				.is_err()
		);
		assert!(answers(ResponseTemplate::new(500)).await.is_err());
	}
}
