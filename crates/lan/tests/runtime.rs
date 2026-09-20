//! The real client against the real room, over loopback TCP.
//!
//! The founder is played by hand on a raw socket, so the test drives the
//! start signal exactly; the guest is `lobby_runtime::Client` as the app
//! runs it, with the default TCP connector — which also proves that the
//! client's encrypted attempts fail fast on this server and the plain
//! fallback is what gets in.

use std::time::Duration;

use lan::{Config, Host, Policy};
use lobby_runtime::{Client, Hardware};
use lobby_ui::Snapshot;
use spring_protocol::{Endpoint, LoginRequest, ThrottlePolicy};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

fn config() -> Config {
	Config {
		founder: "ann".into(),
		title: "Ann's game".into(),
		engine_version: "2026.07.04".into(),
		game: "Beyond All Reason test-31357-b06bb1a".into(),
		map: "Supreme Isthmus v2.1".into(),
		max_players: 8,
		policy: Policy::Open,
	}
}

/// Polls the snapshot until `want` holds, or gives up.
async fn until(client: &Client, what: &str, want: impl Fn(&Snapshot) -> bool) -> Snapshot {
	for _ in 0..200 {
		let snapshot = client.snapshot().await.unwrap();
		if want(&snapshot) {
			return snapshot;
		}
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
	panic!("gave up waiting for {what}: {:?}", client.snapshot().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_logs_in_joins_talks_and_hears_the_game_start() {
	let host = Host::start(config(), 0).await.unwrap();
	let port = host.port();

	// The founder, by hand.
	let founder = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
	let (read, mut write) = founder.into_split();
	let mut founder_lines = BufReader::new(read).lines();
	assert!(
		founder_lines
			.next_line()
			.await
			.unwrap()
			.unwrap()
			.starts_with("TASSERVER ")
	);
	write
		.write_all(b"LOGIN ann * 0 * modlobby:0.1\tx y\tb sp\n")
		.await
		.unwrap();
	loop {
		let line = founder_lines.next_line().await.unwrap().unwrap();
		if line == "LOGININFOEND" {
			break;
		}
	}
	write.write_all(b"JOINBATTLE 1 empty 1111\n").await.unwrap();
	write
		.write_all(b"MYBATTLESTATUS 4195328 255\n")
		.await
		.unwrap();

	// The guest, as the app runs it.
	let client = Client::spawn(
		ThrottlePolicy::default(),
		Hardware {
			properties: Vec::new(),
			lobby_hash: "h h".into(),
			machine_hash: "m".into(),
		},
		None,
	);
	let endpoint = Endpoint {
		host: "127.0.0.1".into(),
		ports: vec![port],
		allow_plain: true,
		preferred: None,
	};
	client
		.login(endpoint, LoginRequest::new("bob", "*", "test", "h h"))
		.await
		.unwrap();
	let snapshot = until(&client, "the battle", |s| {
		s.servers
			.first()
			.is_some_and(|server| !server.battles.is_empty())
	})
	.await;
	let server = snapshot.servers[0].server.clone();
	let battle = &snapshot.servers[0].battles[0];
	assert_eq!(battle.founder, "ann");
	assert_eq!(
		battle.ip, "127.0.0.1",
		"the address the guest reached us on"
	);
	assert_eq!(battle.port, 8452);
	assert_eq!(battle.title, "Ann's game");

	client.join_battle(server.clone(), 1, None).await.unwrap();
	until(&client, "the join", |s| s.room().is_some()).await;
	client.take_seat(1, 1).await.unwrap();
	client.say("hello".into()).await.unwrap();
	// The founder hears the guest arrive with its script password, sit, and
	// talk -- the talk last, so reading up to it sees the rest.
	let mut heard = Vec::new();
	loop {
		let line = founder_lines.next_line().await.unwrap().unwrap();
		heard.push(line.clone());
		if line == "SAIDBATTLE bob hello" {
			break;
		}
		assert!(heard.len() < 50, "no chat line among {heard:?}");
	}
	assert!(
		heard.iter().any(|l| l.starts_with("JOINEDBATTLE 1 bob ")),
		"{heard:?}"
	);
	assert!(
		heard
			.iter()
			.any(|l| l.starts_with("CLIENTBATTLESTATUS bob ")),
		"{heard:?}"
	);

	// The founder's engine starts: its in-game bit is the whole signal.
	write.write_all(b"MYSTATUS 1\n").await.unwrap();
	let snapshot = until(&client, "the game", |s| {
		s.room()
			.is_some_and(|(server, _)| server.game_running.is_some())
	})
	.await;
	let running = snapshot.room().unwrap().0.game_running.clone().unwrap();
	assert_eq!(
		(running.id, running.ip.as_str(), running.port),
		(1, "127.0.0.1", 8452)
	);

	// And stops.
	write.write_all(b"MYSTATUS 0\n").await.unwrap();
	until(&client, "the end", |s| {
		s.room()
			.is_some_and(|(server, _)| server.game_running.is_none())
	})
	.await;

	client.shutdown().await;
	drop(host);
}
