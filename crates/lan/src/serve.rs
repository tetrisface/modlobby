//! The sockets: one listener, one task per connection, one room behind a lock.
//!
//! Every decision is the room's (`room.rs`); this only carries lines. A peer
//! task reads a line, applies it under the lock, and hands whatever came out
//! to the peers it names. The lock is the ordering: what one peer's line
//! caused reaches everyone before the next line is read anywhere.
//!
//! # What a peer may spend
//!
//! This listens on every interface, so "on the LAN" also means anyone who can
//! route to the port -- a VPN peer, say. Nothing here trusts a connection to
//! behave, so all three of the things a socket can make us hold are bounded:
//! how long one line may be, how many connections there may be, and how far a
//! peer that never reads may let its outgoing queue grow. Past any of them the
//! peer is dropped, because an honest client reaches none of them.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::room::{Config, Out, Peer, Room};

/// The longest line read from a peer, the same bound the online protocol
/// keeps (`spring_protocol::codec::MAX_LINE_BYTES`): teiserver drops a
/// partial line past 64 KiB and so does this, so a client that is fine
/// against a server is fine here. A `SETSCRIPTTAGS` carrying tweak Lua is
/// the longest thing anyone legitimately sends, and it is far under.
const MOST_PER_LINE: usize = spring_protocol::codec::MAX_LINE_BYTES;

/// How many sockets may be open at once. A room holds at most a couple of
/// dozen people and their reconnects; past this someone is not playing.
const MOST_PEERS: usize = 64;

/// How many lines may be waiting to go out to one peer. The login flood is
/// the longest burst a room sends -- a line or two per member, per AI and
/// per start rect -- so this is many rooms' worth of slack.
const MOST_QUEUED: usize = 512;

enum Msg {
	Line(String),
	Close,
}

type Peers = Arc<Mutex<HashMap<Peer, mpsc::Sender<Msg>>>>;

/// A room being served. Dropping it stops the server and drops every peer.
pub struct Host {
	port: u16,
	room: Arc<Mutex<Room>>,
	peers: Peers,
	accept: JoinHandle<()>,
}

impl Host {
	/// Binds `0.0.0.0:port` (0 for any free port) and starts serving `config`.
	pub async fn start(config: Config, port: u16) -> std::io::Result<Self> {
		let listener = TcpListener::bind(("0.0.0.0", port)).await?;
		let port = listener.local_addr()?.port();
		let room = Arc::new(Mutex::new(Room::new(config)));
		let peers: Peers = Arc::default();
		let accept = tokio::spawn({
			let room = Arc::clone(&room);
			let peers = Arc::clone(&peers);
			// A permit per open socket, taken here and released when the peer's
			// task ends. Counted in the accept loop rather than off the peer
			// map, which is only written once a connection is under way -- a
			// flood would be through that window before the first insert.
			let room_for = Arc::new(tokio::sync::Semaphore::new(MOST_PEERS));
			async move {
				let mut next: Peer = 1;
				while let Ok((stream, from)) = listener.accept().await {
					let Ok(permit) = Arc::clone(&room_for).try_acquire_owned() else {
						// Hung up on without a word: an answer is a reply to
						// whoever is doing this, and there is nothing to say.
						tracing::warn!(%from, "lan: too many connections; dropped");
						drop(stream);
						continue;
					};
					let id = next;
					next += 1;
					let (room, peers) = (Arc::clone(&room), Arc::clone(&peers));
					tokio::spawn(async move {
						serve_peer(id, stream, room, peers).await;
						drop(permit);
					});
				}
			}
		});
		Ok(Self {
			port,
			room,
			peers,
			accept,
		})
	}

	pub fn port(&self) -> u16 {
		self.port
	}

	/// The room as it is now.
	pub fn room(&self) -> Room {
		self.room.lock().unwrap_or_else(|e| e.into_inner()).clone()
	}

	/// The room itself, for whoever has to read it after this is gone --
	/// the announcer, which outlives nothing but reads on its own clock.
	pub fn shared(&self) -> Arc<Mutex<Room>> {
		Arc::clone(&self.room)
	}

	/// Stops listening and hangs up on everyone, with a word first.
	pub fn stop(&self, reason: &str) {
		self.accept.abort();
		let peers = std::mem::take(&mut *self.peers.lock().unwrap_or_else(|e| e.into_inner()));
		// `try_send`, not `send`: this is not a task and cannot wait, and a
		// bounded channel's `send` is a future that does nothing until it is
		// awaited. Dropping the sender afterwards ends the writer anyway, so
		// a peer whose queue is full is still hung up on -- it just does not
		// get the reason.
		for tx in peers.into_values() {
			let _ = tx.try_send(Msg::Line(format!("SERVERMSG {reason}")));
			let _ = tx.try_send(Msg::Close);
		}
	}
}

impl Drop for Host {
	fn drop(&mut self) {
		self.stop("the host closed the room");
	}
}

fn deliver(peers: &Peers, out: Vec<Out>, me: Peer) -> bool {
	let mut held = peers.lock().unwrap_or_else(|e| e.into_inner());
	let mut closing = false;
	// Peers whose queue is full: they are not reading, and the room is not
	// waiting for them.
	let mut stuck: Vec<Peer> = Vec::new();
	let push = |stuck: &mut Vec<Peer>, peer: Peer, tx: &mpsc::Sender<Msg>, msg: Msg| {
		if tx.try_send(msg).is_err() {
			stuck.push(peer);
		}
	};
	for item in out {
		match item {
			Out::To(peer, line) => {
				if let Some(tx) = held.get(&peer) {
					push(&mut stuck, peer, tx, Msg::Line(line));
				}
			}
			Out::All(line) => {
				for (peer, tx) in held.iter() {
					push(&mut stuck, *peer, tx, Msg::Line(line.clone()));
				}
			}
			Out::Close(peer) => {
				if let Some(tx) = held.get(&peer) {
					push(&mut stuck, peer, tx, Msg::Close);
				}
				closing |= peer == me;
			}
		}
	}
	// Dropping the sender ends that peer's writer, which shuts its socket
	// down; its own task then sees the end and tells the room it left.
	for peer in stuck {
		if held.remove(&peer).is_some() {
			tracing::warn!(peer, "lan: not reading; dropped");
		}
		closing |= peer == me;
	}
	closing
}

/// One line, or `None` for a peer there is no reading any more of.
///
/// `None` covers every way that happens, because they want the same answer:
/// the socket ended, or the peer sent [`MOST_PER_LINE`] bytes without a
/// newline, or what it sent is not text. A TLS hello is the third of those --
/// the client trying encryption on a room that speaks none -- and hanging up
/// is the answer it needs.
///
/// `buffer` is the caller's so the allocation is reused rather than made per
/// line; `take` puts the ceiling on how far it can ever grow.
async fn read_line(
	reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
	buffer: &mut Vec<u8>,
) -> Option<String> {
	buffer.clear();
	let read = reader
		.take(MOST_PER_LINE as u64)
		.read_until(b'\n', buffer)
		.await
		.ok()?;
	if read == 0 || !buffer.ends_with(b"\n") {
		return None;
	}
	buffer.pop();
	// The CR of a CRLF, which some clients send and which the line
	// reader this replaced took off here too.
	if buffer.ends_with(b"\r") {
		buffer.pop();
	}
	String::from_utf8(std::mem::take(buffer)).ok()
}

async fn serve_peer(id: Peer, stream: TcpStream, room: Arc<Mutex<Room>>, peers: Peers) {
	let reached_at: IpAddr = stream
		.local_addr()
		.map(|addr| addr.ip())
		.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
	let _ = stream.set_nodelay(true);
	let (read, mut write) = stream.into_split();
	let (tx, mut rx) = mpsc::channel::<Msg>(MOST_QUEUED);
	peers
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.insert(id, tx);
	let writer = tokio::spawn(async move {
		while let Some(msg) = rx.recv().await {
			match msg {
				Msg::Line(line) => {
					if write
						.write_all(format!("{line}\n").as_bytes())
						.await
						.is_err()
					{
						break;
					}
				}
				Msg::Close => break,
			}
		}
		let _ = write.shutdown().await;
	});

	let out = room
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.connect(id, reached_at);
	let mut closing = deliver(&peers, out, id);
	let mut reader = BufReader::new(read);
	let mut buffer = Vec::new();
	while !closing {
		let Some(line) = read_line(&mut reader, &mut buffer).await else {
			break;
		};
		let out = room
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.apply(id, &line, reached_at);
		closing = deliver(&peers, out, id);
	}
	let out = room
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.disconnect(id);
	peers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
	deliver(&peers, out, id);
	writer.abort();
}
