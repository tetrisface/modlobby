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

/// A host with no map files to hand over, which is most of these tests.
fn no_maps() -> lan::serve::MapFiles {
	std::sync::Arc::new(|_| None)
}

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
	let host = Host::start(config(), 0, no_maps()).await.unwrap();
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
		roots_only: false,
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

/// A peer that will not behave cannot make the host hold more and more on its
/// account. Three ways to try it, one test: the bounds are one policy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_peer_cannot_spend_the_host_without_limit() {
	let host = Host::start(config(), 0, no_maps()).await.unwrap();
	let port = host.port();
	let at = format!("127.0.0.1:{port}");

	// A line that never ends: the host reads its fill and hangs up, rather
	// than buffering whatever arrives until there is no memory left.
	let mut flood = TcpStream::connect(&at).await.unwrap();
	let chunk = vec![b'x'; 8 * 1024];
	let mut sent = 0usize;
	let hung_up = loop {
		if flood.write_all(&chunk).await.is_err() {
			break true;
		}
		sent += chunk.len();
		// Well past the 64 KiB a line may be; a host that took this is one
		// that would take anything.
		if sent > 4 * 1024 * 1024 {
			break false;
		}
	};
	assert!(hung_up, "an endless line was read forever");

	// Sixty-four sockets at once is the most; the next is dropped on the
	// floor, and the room is still there for everyone already in it.
	let mut held = Vec::new();
	for _ in 0..80 {
		if let Ok(socket) = TcpStream::connect(&at).await {
			held.push(socket);
		}
	}
	let mut greeted = 0;
	for socket in &mut held {
		let mut line = String::new();
		let read = tokio::time::timeout(
			Duration::from_millis(200),
			BufReader::new(socket).read_line(&mut line),
		)
		.await;
		if matches!(read, Ok(Ok(n)) if n > 0) {
			greeted += 1;
		}
	}
	assert!(
		greeted <= 64,
		"every one of {} sockets was served",
		held.len()
	);
	assert!(greeted > 0, "nobody was served at all");
	drop(held);
	drop(flood);

	// And the room still works for a client that behaves.
	tokio::time::sleep(Duration::from_millis(200)).await;
	let mut fresh = TcpStream::connect(&at).await.unwrap();
	let mut hello = String::new();
	let mut reader = BufReader::new(&mut fresh);
	tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut hello))
		.await
		.expect("the host still answers")
		.unwrap();
	assert!(hello.starts_with("TASSERVER"), "got {hello:?}");
}

/// The last resort: the map nobody publishes, from the one machine that
/// demonstrably has it.
///
/// The request names no file. It says who is asking and proves it with the
/// script password that member gave at `JOINBATTLE`, and the host answers
/// with whatever its own room is hosting — so there is nothing in it that
/// could reach another file, and nobody outside the room gets an answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_host_hands_its_map_to_a_member_and_to_nobody_else() {
	let held = tempfile::tempdir().unwrap();
	let maps = held.path().join("maps");
	std::fs::create_dir_all(&maps).unwrap();
	// Big enough to cross the chunk boundary, so the streaming is exercised.
	let body: Vec<u8> = (0..200_000_u32).map(|n| (n % 251) as u8).collect();
	std::fs::write(maps.join("supreme_isthmus_v2.1.sd7"), &body).unwrap();

	let owned = maps.clone();
	let files: lan::serve::MapFiles = std::sync::Arc::new(move |map: &str| {
		let stem = map.trim().to_lowercase().replace(' ', "_");
		let path = owned.join(format!("{stem}.sd7"));
		path.is_file().then_some(path)
	});
	let host = Host::start(config(), 0, files).await.unwrap();
	let at: std::net::SocketAddr = format!("127.0.0.1:{}", host.port()).parse().unwrap();

	// A member of the room, joined the way a guest joins.
	let mut guest = TcpStream::connect(at).await.unwrap();
	let mut lines = BufReader::new(&mut guest);
	let mut greeting = String::new();
	lines.read_line(&mut greeting).await.unwrap();
	guest
		.write_all(b"LOGIN bob * 0 * modlobby:0.1\tx y\tb sp\nJOINBATTLE 1 empty 4242\n")
		.await
		.unwrap();
	tokio::time::sleep(Duration::from_millis(200)).await;

	let into = tempfile::tempdir().unwrap();
	let quiet: &lan::getmap::Progress = &|_, _| {};

	// A stranger, and a member with the wrong password, are told the same
	// thing: which of the two it was is not the asker's business.
	for (name, password) in [("bob", "wrong"), ("mallory", "4242")] {
		let refused = lan::getmap::fetch(at, name, password, into.path(), quiet)
			.await
			.expect_err("served a stranger");
		assert!(refused.contains("not in this room"), "{name}: {refused}");
	}
	// Saying nothing at all does not get in either.
	assert!(
		lan::getmap::fetch(at, "bob", "", into.path(), quiet)
			.await
			.is_err(),
		"served a request that proved nothing"
	);
	assert_eq!(
		std::fs::read_dir(into.path()).unwrap().count(),
		0,
		"a refusal left something behind"
	);

	// The member, who gets the file the room is playing, byte for byte.
	let seen = std::sync::Arc::new(std::sync::Mutex::new((0_u64, 0_u64)));
	let noted = std::sync::Arc::clone(&seen);
	let watch: &lan::getmap::Progress = &move |done, total| {
		*noted.lock().unwrap() = (done, total);
	};
	let archive = lan::getmap::fetch(at, "bob", "4242", into.path(), watch)
		.await
		.unwrap();
	assert_eq!(archive, "supreme_isthmus_v2.1.sd7");
	assert_eq!(std::fs::read(into.path().join(&archive)).unwrap(), body);
	assert_eq!(
		*seen.lock().unwrap(),
		(body.len() as u64, body.len() as u64)
	);
	// Nothing half-written left over.
	assert!(!into.path().join(format!("{archive}.part")).exists());

	// And the room is as it was: four file connections came and went, and
	// not one of them became a member of it.
	assert_eq!(
		host.room().counts().0,
		1,
		"a connection that came for the map joined the room"
	);
	drop(host);
}
