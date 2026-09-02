//! The one HTTP client modlobby talks to BAR's services with.
//!
//! One client rather than one per call: it owns the connection pool and the
//! TLS session cache, so a second request to the same host reuses a connection
//! instead of handshaking again. And it says who is asking — the people who run
//! those services should be able to tell modlobby's traffic from everyone
//! else's in their logs, which is what a `User-Agent` is for.

use std::time::Duration;

/// Where to look modlobby up, for whoever reads the server logs.
pub const REPO_URL: &str = "https://github.com/tetrisface/modlobby";

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest wait for the *next* byte, not for the whole body: an engine
/// archive takes minutes to arrive and must not be cut off for taking them.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// `modlobby/<version> (+<repo>)`, the shape crawlers and tools have used for
/// decades: name, version, and where to complain.
pub fn user_agent(version: &str) -> String {
    format!("modlobby/{version} (+{REPO_URL})")
}

/// Built once per process and cloned wherever a request is made; a
/// `reqwest::Client` is an `Arc` inside.
pub fn client(version: &str) -> reqwest::Client {
    install_crypto();
    reqwest::Client::builder()
        .user_agent(user_agent(version))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("a client from static settings")
}

/// reqwest is built with `rustls-no-provider`, so it panics on `build()` unless
/// a crypto provider has been installed first. Installing here, and not only
/// in the lobby transport, means any process that builds a client — a test
/// binary for this crate included — is safe. `Err` means one is installed
/// already, which is the state wanted.
pub fn install_crypto() {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_names_the_version_and_where_to_look() {
        assert_eq!(
            user_agent("0.1.2"),
            "modlobby/0.1.2 (+https://github.com/tetrisface/modlobby)"
        );
    }

    #[test]
    fn a_client_can_be_built_without_anyone_else_installing_crypto() {
        let _ = client("test");
    }

    #[tokio::test]
    async fn every_request_says_who_is_asking() {
        use wiremock::matchers::{header, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("User-Agent", user_agent("0.1.2").as_str()))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let status = client("0.1.2")
            .get(server.uri())
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 204);
    }
}
