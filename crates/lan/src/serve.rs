//! The sockets: one listener, one task per connection, one room behind a lock.
//!
//! Every decision is the room's (`room.rs`); this only carries lines. A peer
//! task reads a line, applies it under the lock, and hands whatever came out
//! to the peers it names. The lock is the ordering: what one peer's line
//! caused reaches everyone before the next line is read anywhere.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::room::{Config, Out, Peer, Room};

enum Msg {
	Line(String),
	Close,
}

type Peers = Arc<Mutex<HashMap<Peer, mpsc::UnboundedSender<Msg>>>>;

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
			async move {
				let mut next: Peer = 1;
				while let Ok((stream, _)) = listener.accept().await {
					let id = next;
					next += 1;
					tokio::spawn(serve_peer(
						id,
						stream,
						Arc::clone(&room),
						Arc::clone(&peers),
					));
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
		for tx in peers.into_values() {
			let _ = tx.send(Msg::Line(format!("SERVERMSG {reason}")));
			let _ = tx.send(Msg::Close);
		}
	}
}

impl Drop for Host {
	fn drop(&mut self) {
		self.stop("the host closed the room");
	}
}

fn deliver(peers: &Peers, out: Vec<Out>, me: Peer) -> bool {
	let held = peers.lock().unwrap_or_else(|e| e.into_inner());
	let mut closing = false;
	for item in out {
		match item {
			Out::To(peer, line) => {
				if let Some(tx) = held.get(&peer) {
					let _ = tx.send(Msg::Line(line));
				}
			}
			Out::All(line) => {
				for tx in held.values() {
					let _ = tx.send(Msg::Line(line.clone()));
				}
			}
			Out::Close(peer) => {
				if let Some(tx) = held.get(&peer) {
					let _ = tx.send(Msg::Close);
				}
				closing |= peer == me;
			}
		}
	}
	closing
}

async fn serve_peer(id: Peer, stream: TcpStream, room: Arc<Mutex<Room>>, peers: Peers) {
	let reached_at: IpAddr = stream
		.local_addr()
		.map(|addr| addr.ip())
		.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
	let _ = stream.set_nodelay(true);
	let (read, mut write) = stream.into_split();
	let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
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
	let mut lines = BufReader::new(read).lines();
	while !closing {
		// A TLS hello is not UTF-8 and ends the line reader, which ends the
		// peer: that is the answer the client's TLS attempt needs.
		let Ok(Some(line)) = lines.next_line().await else {
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
