//! Tokio transport actor: one task reads lines into [`ServerEvent`]s, one task
//! drains the [`Scheduler`] onto the socket and keeps the heartbeat alive.
//!
//! teiserver listens plain on 8200 (what Chobby uses; `STLS` upgrades it to
//! TLS) and TLS on 8201; all carry the same line protocol, so the actor is
//! generic over the stream. Which of those ways a given server answers on is
//! found by trying them ([`Transport::connect`]), not configured.
//!
//! Correlation: a request tagged `#<id>` resolves on the first reply line
//! carrying that id; every line, tagged or not, is also delivered as an event,
//! so nothing depends on correlation for state.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;

use crate::codec::{self, RawMessage};
use crate::event::ServerEvent;
use crate::pin::{self, Fingerprint, Seen, Trust};
use crate::policy::{Area, Envelope, PolicyEvent, Scheduler, ThrottlePolicy};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CHANNEL_CAPACITY: usize = 1024;
/// How long a fresh connection gets to say its first word before it counts as silent.
const GREETING_WAIT: Duration = Duration::from_secs(5);
/// The most one way into a server may take, from the first packet to the
/// greeting. Without it a host that drops packets would hold a connect for
/// as long as the operating system cares to keep trying.
const ATTEMPT_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
	#[error("connect: {0}")]
	Connect(#[from] std::io::Error),
	#[error("not a valid TLS server name: {0}")]
	ServerName(String),
	#[error("transport closed")]
	Closed,
	#[error("no reply within {0:?}")]
	Timeout(Duration),
	#[error("server did not agree to STLS: {0:?}")]
	Stls(String),
	#[error("the server did not greet")]
	Silent,
	/// Every way in was tried and none answered — or nothing was there to
	/// answer: no network, no such host.
	#[error("could not reach the server ({0})")]
	Unreachable(String),
	/// The server is there, every encrypted way into it failed, and it is
	/// not allowed an unencrypted one. Its own case because the answer is a
	/// setting — which is why a server that was never reached is not this.
	#[error("no encrypted way in ({0})")]
	NoEncryption(String),
	/// Every way was refused outright: the host is there and nothing listens,
	/// which is a server down or restarting. Its own case because the answer
	/// is to wait, where the four socket errors read as a port problem.
	#[error("every port refused the connection; the server is down or restarting")]
	Down,
	/// The certificate is not the one trusted before, or no longer one the
	/// roots vouch for. Refused outright, and never answered by falling back
	/// to plaintext: that is what somebody in the middle would want.
	#[error("{}", changed(*.was, *.now))]
	CertificateChanged {
		was: Option<Fingerprint>,
		now: Fingerprint,
	},
}

fn changed(was: Option<Fingerprint>, now: Fingerprint) -> String {
	let before = match was {
		Some(was) => format!(
			"not the one trusted before ({} then, {} now)",
			was.short(),
			now.short()
		),
		None => format!(
			"no longer one a certificate authority vouches for ({} now)",
			now.short()
		),
	};
	format!(
		"the server's certificate is {before}, so it was not connected to. If the server replaced it on purpose, \"try every way again\" under Settings → Servers forgets the old one"
	)
}

/// How a server is known across the app — in the remembered ways, in what
/// the front end is told — however its host was typed.
pub fn server_id(host: &str) -> String {
	host.trim().to_ascii_lowercase()
}

/// Which server to reach, and what it may be reached by. Which way actually
/// works is found by trying ([`Transport::connect`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
	pub host: String,
	/// Each is tried both encrypted ways.
	pub ports: Vec<u16>,
	/// Whether an unencrypted connection will do once every encrypted way
	/// has failed. Never tried before that.
	pub allow_plain: bool,
	/// What worked last time, tried on its own before anything else. Its
	/// certificate pin, if it has one, is the one every way must show.
	pub preferred: Option<Way>,
	/// Only a certificate the roots vouch for will do, even the first time:
	/// for a server known to have one, where a self-signed certificate could
	/// only be somebody else's.
	pub roots_only: bool,
}

impl Endpoint {
	/// teiserver's own ports, plain on 8200 and TLS on 8201, encrypted only.
	pub fn new(host: impl Into<String>) -> Self {
		Self {
			host: host.into(),
			ports: vec![8200, 8201],
			allow_plain: false,
			preferred: None,
			roots_only: false,
		}
	}

	/// What a certificate the roots refuse may still be taken for: what was
	/// pinned last time, else a change if the roots vouched for it then, else
	/// -- nothing encrypted remembered -- whatever it is, this first time.
	fn trust(&self) -> Trust {
		match self.preferred.filter(|way| way.security != Security::None) {
			Some(way) => Trust::Pinned(way.pin),
			None if self.roots_only => Trust::Roots,
			None => Trust::FirstUse,
		}
	}

	/// The way worth trying on its own: the remembered one, while its port
	/// is still listed and, if it was unencrypted, while that is still allowed.
	///
	/// ponytail: a remembered unencrypted way is kept until it fails or is
	/// forgotten, so a server that gains TLS later is not noticed; the owner
	/// opted that server into plaintext, and forgetting the way re-probes it.
	fn first(&self) -> Option<(u16, Security)> {
		let way = self.preferred?;
		let listed = self.ports.contains(&way.port);
		let allowed = way.security != Security::None || self.allow_plain;
		(listed && allowed).then_some((way.port, way.security))
	}

	/// Each listed port with each of `ways`, port by port.
	fn every(&self, ways: &[Security]) -> Vec<(u16, Security)> {
		self.ports
			.iter()
			.flat_map(|port| ways.iter().map(move |security| (*port, *security)))
			.collect()
	}
}

/// How the connection is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
	/// A plain port, upgraded to TLS by `STLS`, like SMTP's STARTTLS.
	Stls,
	/// A TLS port, encrypted from the first byte.
	Tls,
	/// Unencrypted.
	None,
}

/// A way into a server that worked, and how long it took to greet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Way {
	pub port: u16,
	pub security: Security,
	/// From the first packet to the greeting.
	pub ms: u32,
	/// The certificate trusted on first use, where the roots vouched for none.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub pin: Option<Fingerprint>,
}

impl fmt::Display for Way {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{} on {}, {} ms",
			name(self.security),
			self.port,
			self.ms
		)
	}
}

/// What the transport delivers to the application.
#[derive(Debug)]
pub enum Inbound {
	Message(ServerEvent),
	/// Scheduler decisions worth surfacing (delays, coalescing, trips, drops).
	Policy(PolicyEvent),
	Closed {
		reason: String,
	},
}

/// A connection in either encryption, as one type.
trait Io: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// A connection the server has greeted on, its greeting still unread.
type Opened = BufReader<Box<dyn Io>>;

enum Outbound {
	Send(Envelope),
	Request {
		envelope: Envelope,
		id: u32,
		reply: oneshot::Sender<RawMessage>,
	},
	Trip {
		area: Area,
		until: Instant,
	},
	/// Drop what `area` still holds.
	Cancel {
		area: Area,
	},
	Shutdown,
}

type Pending = Arc<Mutex<HashMap<u32, oneshot::Sender<RawMessage>>>>;

/// Handle to a connected transport; cheap to clone.
#[derive(Clone)]
pub struct Transport {
	out: mpsc::Sender<Outbound>,
	next_id: Arc<AtomicU32>,
}

impl Transport {
	/// Finds a way in, and spawns the reader and writer tasks on it.
	///
	/// The remembered way goes first, alone. Failing that, or with none,
	/// every encrypted way on every port is tried at once and the first to
	/// greet wins — so a port that hangs costs nothing while another answers.
	/// Unencrypted comes last, only where allowed, and never raced against
	/// encryption: it has fewer round trips and would win.
	pub async fn connect(
		endpoint: &Endpoint,
		policy: ThrottlePolicy,
	) -> Result<(Self, mpsc::Receiver<Inbound>, Way), TransportError> {
		let (stream, way) = open(endpoint, ATTEMPT_WAIT).await?;
		let (transport, inbound) = Self::from_stream(stream, policy);
		Ok((transport, inbound, way))
	}

	/// Runs the protocol over any stream: the socket in production, an in-memory duplex in tests.
	pub fn from_stream<S>(stream: S, policy: ThrottlePolicy) -> (Self, mpsc::Receiver<Inbound>)
	where
		S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
	{
		let (read_half, write_half) = tokio::io::split(stream);
		let (in_tx, in_rx) = mpsc::channel(CHANNEL_CAPACITY);
		let (out_tx, out_rx) = mpsc::channel(CHANNEL_CAPACITY);
		let pending = Pending::default();

		tokio::spawn(reader(read_half, in_tx.clone(), Arc::clone(&pending)));
		tokio::spawn(writer(write_half, out_rx, in_tx, pending, policy));

		(
			Self {
				out: out_tx,
				next_id: Arc::default(),
			},
			in_rx,
		)
	}

	pub async fn send(&self, envelope: Envelope) -> Result<(), TransportError> {
		self.out
			.send(Outbound::Send(envelope))
			.await
			.map_err(|_| TransportError::Closed)
	}

	/// Sends a `#<id>`-tagged line and waits for the first reply carrying that id.
	pub async fn request(&self, envelope: Envelope) -> Result<RawMessage, TransportError> {
		let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
		let (reply, receiver) = oneshot::channel();
		self.out
			.send(Outbound::Request {
				envelope,
				id,
				reply,
			})
			.await
			.map_err(|_| TransportError::Closed)?;
		match tokio::time::timeout(REQUEST_TIMEOUT, receiver).await {
			Ok(Ok(raw)) => Ok(raw),
			Ok(Err(_)) => Err(TransportError::Closed),
			Err(_) => Err(TransportError::Timeout(REQUEST_TIMEOUT)),
		}
	}

	/// Pauses an area of the throttle policy, e.g. after a flood signal.
	pub async fn trip(&self, area: Area, until: Instant) -> Result<(), TransportError> {
		self.out
			.send(Outbound::Trip { area, until })
			.await
			.map_err(|_| TransportError::Closed)
	}

	/// Drops whatever is still queued for `area`; what has left is gone.
	pub async fn cancel(&self, area: Area) -> Result<(), TransportError> {
		self.out
			.send(Outbound::Cancel { area })
			.await
			.map_err(|_| TransportError::Closed)
	}

	pub async fn shutdown(&self) {
		let _ = self.out.send(Outbound::Shutdown).await;
	}
}

/// Makes ring the process-wide crypto provider, once.
///
/// `ClientConfig::builder()` resolves the process default and *panics at
/// connect time* when the build carries more than one provider with no default
/// installed — which is exactly what a second dependency enabling rustls's
/// aws-lc-rs feature causes, and it took the whole runtime actor down with it
/// ("client stopped" on login). Installing explicitly here means no future
/// dependency change can reintroduce that panic; it also serves every other
/// rustls user in the process, reqwest included.
pub fn install_crypto() {
	// Err means someone installed one already, which is the state we want.
	let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
}

/// The first way in that greets; see [`Transport::connect`]. `wait` bounds
/// each attempt, so the whole takes at most three of them.
async fn open(endpoint: &Endpoint, wait: Duration) -> Result<(Opened, Way), TransportError> {
	let trust = endpoint.trust();
	if let Some((port, security)) = endpoint.first() {
		match attempt(endpoint.host.clone(), port, security, trust, wait).await {
			Ok(opened) => return Ok(opened),
			Err(Failed {
				error: changed @ TransportError::CertificateChanged { .. },
				..
			}) => return Err(changed),
			Err(Failed { error, .. }) => {
				tracing::warn!(host = %endpoint.host, port, ?security, %error, "the remembered way failed; trying every way")
			}
		}
	}
	let encrypted = race(
		&endpoint.host,
		endpoint.every(&[Security::Stls, Security::Tls]),
		trust,
		wait,
	)
	.await;
	let tried = match encrypted {
		Ok(opened) => return Ok(opened),
		Err(tried) => tried,
	};
	if let Some((was, now)) = tried.changed {
		return Err(TransportError::CertificateChanged { was, now });
	}
	if !endpoint.allow_plain {
		return Err(if tried.reached {
			TransportError::NoEncryption(tried.text)
		} else if tried.refused {
			TransportError::Down
		} else {
			TransportError::Unreachable(tried.text)
		});
	}
	race(
		&endpoint.host,
		endpoint.every(&[Security::None]),
		trust,
		wait,
	)
	.await
	.map_err(|plain| {
		if tried.refused && plain.refused {
			TransportError::Down
		} else {
			TransportError::Unreachable(format!("{}; {}", tried.text, plain.text))
		}
	})
}

/// What a race that nobody won ran into, and whether any of it got as far as
/// a connection: a server that answers but will not encrypt is a different
/// problem from one that is not there.
struct Tried {
	reached: bool,
	/// Every way was refused outright; see [`TransportError::Down`].
	refused: bool,
	/// A way showed a certificate other than the one trusted.
	changed: Option<(Option<Fingerprint>, Fingerprint)>,
	text: String,
}

/// One way in that failed, and whether the server was at least there.
struct Failed {
	reached: bool,
	error: TransportError,
}

/// Every way at once; the first to greet wins. Dropping the set aborts the
/// others mid-attempt, which closes their sockets. When all fail, what each
/// one ran into, for the message.
async fn race(
	host: &str,
	ways: Vec<(u16, Security)>,
	trust: Trust,
	wait: Duration,
) -> Result<(Opened, Way), Tried> {
	let mut attempts = JoinSet::new();
	for (port, security) in ways {
		let host = host.to_owned();
		attempts.spawn(async move {
			attempt(host, port, security, trust, wait)
				.await
				.map_err(|failed| {
					let text = format!("{} on {port}: {}", name(security), failed.error);
					let changed = match failed.error {
						TransportError::CertificateChanged { was, now } => Some((was, now)),
						_ => None,
					};
					(failed.reached, refused(&failed.error), changed, text)
				})
		});
	}
	let mut reached = false;
	let mut refusals = 0;
	let mut changed = None;
	let mut failures = Vec::new();
	while let Some(joined) = attempts.join_next().await {
		match joined {
			Ok(Ok(opened)) => return Ok(opened),
			Ok(Err((there, turned_away, shown, failure))) => {
				reached |= there;
				refusals += usize::from(turned_away);
				changed = changed.or(shown);
				failures.push(failure);
			}
			Err(panicked) => failures.push(panicked.to_string()),
		}
	}
	if failures.is_empty() {
		failures.push("no ports to try".to_owned());
	}
	Err(Tried {
		reached,
		refused: refusals == failures.len(),
		changed,
		text: failures.join("; "),
	})
}

/// Turned away at the socket: the host answered and nothing listens there.
fn refused(error: &TransportError) -> bool {
	matches!(error, TransportError::Connect(io) if io.kind() == std::io::ErrorKind::ConnectionRefused)
}

/// One way in, from the first packet to the greeting, within `wait`.
async fn attempt(
	host: String,
	port: u16,
	security: Security,
	trust: Trust,
	wait: Duration,
) -> Result<(Opened, Way), Failed> {
	let started = Instant::now();
	let deadline = tokio::time::Instant::now() + wait;
	let within = |reached| move |error| Failed { reached, error };
	let socket = tokio::time::timeout_at(deadline, tcp(&host, port))
		.await
		.map_err(|_| TransportError::Timeout(wait))
		.flatten()
		.map_err(within(false))?;
	let (stream, pin) = tokio::time::timeout_at(deadline, secure(&host, socket, security, trust))
		.await
		.map_err(|_| TransportError::Timeout(wait))
		.flatten()
		.map_err(within(true))?;
	let ms = started.elapsed().as_millis().try_into().unwrap_or(u32::MAX);
	Ok((
		stream,
		Way {
			port,
			security,
			ms,
			pin,
		},
	))
}

/// A connected socket made into a stream the server has greeted on, and the
/// certificate pin it was trusted by, if the roots vouched for none.
async fn secure(
	host: &str,
	socket: TcpStream,
	security: Security,
	trust: Trust,
) -> Result<(Opened, Option<Fingerprint>), TransportError> {
	let (stream, pin): (Box<dyn Io>, _) = match security {
		Security::None => (Box::new(socket), None),
		Security::Stls => {
			let (stream, pin) = tls(host, stls_upgrade(socket).await?, trust).await?;
			(Box::new(stream), pin)
		}
		Security::Tls => {
			let (stream, pin) = tls(host, socket, trust).await?;
			(Box::new(stream), pin)
		}
	};
	let mut stream = BufReader::new(stream);
	if !greets(&mut stream).await {
		return Err(TransportError::Silent);
	}
	Ok((stream, pin))
}

async fn tls(
	host: &str,
	socket: TcpStream,
	trust: Trust,
) -> Result<(TlsStream<TcpStream>, Option<Fingerprint>), TransportError> {
	let name = ServerName::try_from(host.to_owned())
		.map_err(|_| TransportError::ServerName(host.to_owned()))?;
	let (connector, outcome) = pin::connector(trust);
	let handshake = connector.connect(name, socket).await;
	let seen = *outcome.lock().expect("pin outcome lock");
	match (handshake, seen) {
		(Ok(stream), Some(Seen::Pinned(pin))) => Ok((stream, Some(pin))),
		(Ok(stream), _) => Ok((stream, None)),
		(Err(_), Some(Seen::Changed { was, now })) => {
			Err(TransportError::CertificateChanged { was, now })
		}
		(Err(io), _) => Err(io.into()),
	}
}

/// How a way of connecting reads in a message.
fn name(security: Security) -> &'static str {
	match security {
		Security::Stls => "STLS",
		Security::Tls => "TLS",
		Security::None => "unencrypted",
	}
}

async fn tcp(host: &str, port: u16) -> Result<TcpStream, TransportError> {
	let stream = TcpStream::connect((host, port)).await?;
	stream.set_nodelay(true)?;
	Ok(stream)
}

/// Whether the server greets as a lobby server before it hangs up: a port
/// that answers is not enough, since whatever else listens there — a mail
/// server, a web server — would win a race and be remembered as the way in.
/// The first bytes are waited for but left in the buffer, so the greeting
/// still reaches the session.
async fn greets<S: AsyncRead + Unpin>(stream: &mut BufReader<S>) -> bool {
	const GREETING: &[u8] = b"TASSERVER";
	let Ok(Ok(buf)) = tokio::time::timeout(GREETING_WAIT, stream.fill_buf()).await else {
		return false;
	};
	// As much of the word as has arrived: a greeting may come in pieces.
	let arrived = buf.len().min(GREETING.len());
	arrived > 0 && buf[..arrived] == GREETING[..arrived]
}

/// Asks a plain connection to switch to TLS and hands it back ready for the
/// handshake. Anything but `OK cmd=STLS` is an error: carrying on unencrypted
/// is exactly what an attacker stripping the upgrade hopes for.
async fn stls_upgrade<S>(stream: S) -> Result<S, TransportError>
where
	S: AsyncRead + AsyncWrite + Unpin,
{
	let mut stream = BufReader::new(stream);
	let mut line = String::new();
	// The plaintext TASSERVER; the server greets again once encrypted.
	read_line_within(&mut stream, &mut line).await?;
	stream.write_all(b"STLS\n").await?;
	line.clear();
	read_line_within(&mut stream, &mut line).await?;
	// Bytes behind the OK would be plaintext mistaken for the TLS handshake.
	if line.trim_end() != "OK cmd=STLS" || !stream.buffer().is_empty() {
		return Err(TransportError::Stls(line.trim_end().to_owned()));
	}
	Ok(stream.into_inner())
}

async fn read_line_within<S: AsyncRead + Unpin>(
	stream: &mut BufReader<S>,
	line: &mut String,
) -> Result<(), TransportError> {
	tokio::time::timeout(GREETING_WAIT, stream.read_line(line))
		.await
		.map_err(|_| TransportError::Timeout(GREETING_WAIT))??;
	Ok(())
}

/// Keeps the password hash out of the transmit trace.
fn redacted(line: &str) -> Cow<'_, str> {
	let start = match line.strip_prefix('#') {
		Some(_) => line.find(' ').map_or(line.len(), |i| i + 1),
		None => 0,
	};
	let Some(after) = line[start..].strip_prefix("LOGIN ") else {
		return Cow::Borrowed(line);
	};
	let mut parts = after.splitn(3, ' ');
	let user = parts.next().unwrap_or("");
	let rest = parts.nth(1).unwrap_or("");
	Cow::Owned(format!("{}LOGIN {user} <redacted> {rest}", &line[..start]))
}

async fn reader<R>(read_half: R, in_tx: mpsc::Sender<Inbound>, pending: Pending)
where
	R: AsyncRead + Unpin + Send + 'static,
{
	let mut lines = BufReader::new(read_half).lines();
	let reason = loop {
		match lines.next_line().await {
			Ok(Some(line)) => {
				tracing::trace!(target: "spring::rx", "{line}");
				let raw = RawMessage::parse(&line);
				if let Some(id) = raw.id
					&& let Some(reply) = pending.lock().expect("pending map lock").remove(&id)
				{
					let _ = reply.send(raw.clone());
				}
				if in_tx.send(Inbound::Message(raw.into())).await.is_err() {
					return;
				}
			}
			Ok(None) => break "connection closed by server".to_owned(),
			Err(err) => break err.to_string(),
		}
	};
	let _ = in_tx.send(Inbound::Closed { reason }).await;
}

async fn writer<W>(
	mut write_half: W,
	mut out_rx: mpsc::Receiver<Outbound>,
	in_tx: mpsc::Sender<Inbound>,
	pending: Pending,
	policy: ThrottlePolicy,
) where
	W: AsyncWrite + Unpin + Send + 'static,
{
	let heartbeat_idle = policy.heartbeat_idle();
	let mut scheduler = Scheduler::new(policy, Instant::now());
	let mut last_write = Instant::now();

	loop {
		let now = Instant::now();
		let heartbeat_in = heartbeat_idle.saturating_sub(now.duration_since(last_write));
		let wakeup = scheduler
			.next_wakeup(now)
			.map_or(heartbeat_in, |w| w.min(heartbeat_in));

		tokio::select! {
			outbound = out_rx.recv() => match outbound {
				Some(Outbound::Send(envelope)) => scheduler.submit(envelope),
				Some(Outbound::Request { mut envelope, id, reply }) => {
					pending.lock().expect("pending map lock").insert(id, reply);
					envelope.line = format!("#{id} {}", envelope.line);
					scheduler.submit(envelope);
				}
				Some(Outbound::Trip { area, until }) => scheduler.trip(area, until),
				Some(Outbound::Cancel { area }) => {
					scheduler.cancel(area);
				}
				Some(Outbound::Shutdown) | None => return,
			},
			_ = tokio::time::sleep(wakeup) => {}
		}

		let now = Instant::now();
		if now.duration_since(last_write) >= heartbeat_idle && scheduler.pending() == 0 {
			scheduler.submit(Envelope::immediate(Area::Heartbeat, "PING"));
		}
		// One drained string is one write, and a write is what teiserver's
		// flood limiter counts (policy module docs). On the burst lane a
		// string holds several lines joined by newlines; the newline appended
		// here ends the last of them.
		for line in scheduler.drain(now) {
			tracing::trace!(target: "spring::tx", "{}", redacted(&line));
			if let Err(err) = write_half
				.write_all(codec::encode(None, &line).as_bytes())
				.await
			{
				let _ = in_tx
					.send(Inbound::Closed {
						reason: format!("write failed: {err}"),
					})
					.await;
				return;
			}
			last_write = now;
		}
		for event in scheduler.take_events() {
			if in_tx.send(Inbound::Policy(event)).await.is_err() {
				return;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use std::future::Future;
	use std::sync::atomic::AtomicUsize;

	use tokio_rustls::TlsAcceptor;
	use tokio_rustls::rustls::ServerConfig;
	use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

	/// A certificate no root vouches for, and its key; test-only, made with
	/// openssl for these tests.
	const SELF_SIGNED: &[u8] = include_bytes!("../testdata/self-signed.der");
	const SELF_SIGNED_KEY: &[u8] = include_bytes!("../testdata/self-signed.key.der");

	fn local(ports: Vec<u16>, allow_plain: bool, preferred: Option<Way>) -> Endpoint {
		Endpoint {
			host: "127.0.0.1".into(),
			ports,
			allow_plain,
			preferred,
			roots_only: false,
		}
	}

	fn way(port: u16, security: Security) -> Way {
		Way {
			port,
			security,
			ms: 0,
			pin: None,
		}
	}

	/// A TLS port behind [`SELF_SIGNED`] that greets once encrypted, as an
	/// uberserver does on the far side of `STLS`.
	async fn self_signed(socket: TcpStream) {
		install_crypto();
		let config = ServerConfig::builder()
			.with_no_client_auth()
			.with_single_cert(
				vec![CertificateDer::from(SELF_SIGNED)],
				PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(SELF_SIGNED_KEY)),
			)
			.unwrap();
		if let Ok(mut stream) = TlsAcceptor::from(Arc::new(config)).accept(socket).await {
			let _ = stream.write_all(b"TASSERVER 0.38 * 8201 0\n").await;
			let _ = stream.flush().await;
			tokio::time::sleep(Duration::from_secs(5)).await;
		}
	}

	async fn greets_plainly(mut socket: TcpStream) {
		let _ = socket.write_all(b"TASSERVER 0.38 * 8201 0\n").await;
		tokio::time::sleep(Duration::from_secs(5)).await;
	}

	/// A listener on a free local port serving each connection with `serve`,
	/// and a count of the connections it took.
	async fn serving<F, Fut>(serve: F) -> (u16, Arc<AtomicUsize>)
	where
		F: Fn(TcpStream) -> Fut + Send + 'static,
		Fut: Future<Output = ()> + Send + 'static,
	{
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let port = listener.local_addr().unwrap().port();
		let accepted = Arc::new(AtomicUsize::new(0));
		let count = Arc::clone(&accepted);
		tokio::spawn(async move {
			while let Ok((socket, _)) = listener.accept().await {
				count.fetch_add(1, Ordering::Relaxed);
				tokio::spawn(serve(socket));
			}
		});
		(port, accepted)
	}

	/// server.example.com on 2026-09-19: greets in plaintext, agrees
	/// to `STLS`, then hangs up where the handshake should be.
	async fn agrees_then_hangs_up(mut socket: TcpStream) {
		let _ = socket.write_all(b"TASSERVER 0.38 * 8201 0\n").await;
		let mut line = String::new();
		let _ = BufReader::new(&mut socket).read_line(&mut line).await;
		if line == "STLS\n" {
			let _ = socket.write_all(b"OK cmd=STLS\n").await;
		}
	}

	async fn hangs_up(_socket: TcpStream) {}

	async fn says_nothing(socket: TcpStream) {
		tokio::time::sleep(Duration::from_secs(60)).await;
		drop(socket);
	}

	#[test]
	fn the_remembered_way_goes_first_only_while_it_is_still_on_offer() {
		let tls = Some(way(8201, Security::Tls));
		let plain = Some(way(8200, Security::None));
		assert_eq!(
			local(vec![8200, 8201], false, tls).first(),
			Some((8201, Security::Tls))
		);
		assert_eq!(
			local(vec![8200], false, tls).first(),
			None,
			"port no longer listed"
		);
		assert_eq!(
			local(vec![8200], false, plain).first(),
			None,
			"plaintext no longer allowed"
		);
		assert_eq!(
			local(vec![8200], true, plain).first(),
			Some((8200, Security::None))
		);
		assert_eq!(local(vec![8200], true, None).first(), None);
	}

	#[test]
	fn every_way_is_every_port_each_way() {
		assert_eq!(
			local(vec![8200, 8201], false, None).every(&[Security::Stls, Security::Tls]),
			vec![
				(8200, Security::Stls),
				(8200, Security::Tls),
				(8201, Security::Stls),
				(8201, Security::Tls),
			]
		);
	}

	#[test]
	fn a_certificate_is_taken_as_the_remembered_way_says() {
		let pin = Fingerprint([1; 32]);
		let pinned = Way {
			pin: Some(pin),
			..way(8200, Security::Stls)
		};
		assert_eq!(
			local(vec![8200], false, Some(pinned)).trust(),
			Trust::Pinned(Some(pin))
		);
		assert_eq!(
			local(vec![8200], false, Some(way(8200, Security::Stls))).trust(),
			Trust::Pinned(None),
			"the roots vouched for it last time"
		);
		assert_eq!(
			local(vec![8200], true, Some(way(8200, Security::None))).trust(),
			Trust::FirstUse,
			"nothing encrypted remembered"
		);
		let bar = Endpoint {
			roots_only: true,
			..local(vec![8200], false, None)
		};
		assert_eq!(bar.trust(), Trust::Roots);
	}

	#[tokio::test]
	async fn a_certificate_nobody_vouches_for_is_trusted_the_first_time_and_held_to() {
		let (port, _) = serving(self_signed).await;
		let wait = Duration::from_secs(5);
		let (_, first) = open(&local(vec![port], false, None), wait).await.unwrap();
		assert_eq!(first.security, Security::Tls);
		assert_eq!(first.pin, Some(Fingerprint::of(SELF_SIGNED)));

		let (_, again) = open(&local(vec![port], false, Some(first)), wait)
			.await
			.unwrap();
		assert_eq!(again.pin, first.pin);
	}

	#[tokio::test]
	async fn a_changed_certificate_is_refused_and_never_answered_with_plaintext() {
		let (tls, _) = serving(self_signed).await;
		let (plain, plain_taken) = serving(greets_plainly).await;
		let trusted_before = Way {
			pin: Some(Fingerprint([7; 32])),
			..way(tls, Security::Tls)
		};
		let refused = open(
			&local(vec![tls, plain], true, Some(trusted_before)),
			Duration::from_secs(5),
		)
		.await;
		assert!(matches!(
			refused,
			Err(TransportError::CertificateChanged { was: Some(_), .. })
		));
		assert_eq!(plain_taken.load(Ordering::Relaxed), 0);
	}

	#[tokio::test]
	async fn a_server_held_to_the_roots_is_not_trusted_on_first_use() {
		let (port, _) = serving(self_signed).await;
		let bar = Endpoint {
			roots_only: true,
			..local(vec![port], false, None)
		};
		let refused = open(&bar, Duration::from_secs(5)).await;
		assert!(matches!(refused, Err(TransportError::NoEncryption(_))));
	}

	#[test]
	fn a_way_reads_as_what_it_was() {
		assert_eq!(
			Way {
				port: 8200,
				security: Security::Stls,
				ms: 46,
				pin: None,
			}
			.to_string(),
			"STLS on 8200, 46 ms"
		);
		assert_eq!(
			way(8200, Security::None).to_string(),
			"unencrypted on 8200, 0 ms"
		);
	}

	#[tokio::test]
	async fn a_server_without_encryption_is_refused_unless_plaintext_is_allowed() {
		let (agrees, _) = serving(agrees_then_hangs_up).await;
		let (resets, _) = serving(hangs_up).await;
		let wait = Duration::from_secs(2);

		let refused = open(&local(vec![agrees, resets], false, None), wait).await;
		let Err(TransportError::NoEncryption(tried)) = refused else {
			panic!("expected NoEncryption");
		};
		for port in [agrees, resets] {
			assert!(tried.contains(&format!("STLS on {port}")), "{tried}");
			assert!(tried.contains(&format!("TLS on {port}")), "{tried}");
		}

		let (mut stream, way) = open(&local(vec![agrees, resets], true, None), wait)
			.await
			.expect("plaintext is allowed");
		assert_eq!((way.port, way.security), (agrees, Security::None));
		let mut greeting = String::new();
		stream.read_line(&mut greeting).await.unwrap();
		assert_eq!(
			greeting, "TASSERVER 0.38 * 8201 0\n",
			"the greeting is kept"
		);
	}

	#[tokio::test]
	async fn every_way_is_tried_at_once_so_a_silent_port_costs_one_wait() {
		let (silent, _) = serving(says_nothing).await;
		let (agrees, _) = serving(agrees_then_hangs_up).await;
		let wait = Duration::from_millis(400);

		let started = Instant::now();
		let (_, way) = open(&local(vec![silent, agrees], true, None), wait)
			.await
			.expect("the plain way answers");
		assert_eq!(way.port, agrees);
		// In turn it would be at least the four encrypted attempts' waits.
		assert!(started.elapsed() < wait * 2, "took {:?}", started.elapsed());
	}

	#[tokio::test]
	async fn one_attempt_is_bounded_by_the_wait() {
		let (silent, _) = serving(says_nothing).await;
		let started = Instant::now();
		let attempted = attempt(
			"127.0.0.1".into(),
			silent,
			Security::Stls,
			Trust::FirstUse,
			Duration::from_millis(200),
		)
		.await;
		assert!(matches!(
			attempted,
			Err(Failed {
				reached: true,
				error: TransportError::Timeout(_)
			})
		));
		assert!(started.elapsed() < Duration::from_secs(1));
	}

	#[tokio::test]
	async fn the_remembered_way_is_tried_alone_and_a_failed_one_falls_back_to_every_way() {
		let (agrees, accepted) = serving(agrees_then_hangs_up).await;
		let wait = Duration::from_secs(2);

		let remembered = Some(way(agrees, Security::None));
		let (_, found) = open(&local(vec![agrees], true, remembered), wait)
			.await
			.unwrap();
		assert_eq!(found.security, Security::None);
		assert_eq!(
			accepted.load(Ordering::Relaxed),
			1,
			"no race when the memory holds"
		);

		let (resets, _) = serving(hangs_up).await;
		let stale = Some(way(resets, Security::None));
		let (_, found) = open(&local(vec![resets, agrees], true, stale), wait)
			.await
			.unwrap();
		assert_eq!((found.port, found.security), (agrees, Security::None));
	}

	#[test]
	fn login_hash_is_redacted_in_traces() {
		assert_eq!(
			redacted("LOGIN alice X03MO1qnZdYdgyfeuILPmQ== 0 * LuaLobby Chobby:x\ta b\tb sp"),
			"LOGIN alice <redacted> 0 * LuaLobby Chobby:x\ta b\tb sp"
		);
		assert_eq!(
			redacted("#3 LOGIN alice hash rest"),
			"#3 LOGIN alice <redacted> rest"
		);
		assert_eq!(
			redacted("JOINBATTLE 5 empty 4242"),
			"JOINBATTLE 5 empty 4242"
		);
		assert_eq!(redacted("PING"), "PING");
	}

	#[tokio::test]
	async fn a_port_that_hangs_up_does_not_greet_and_one_that_speaks_keeps_its_words() {
		let (client, server) = tokio::io::duplex(64);
		drop(server);
		assert!(!greets(&mut BufReader::new(client)).await);

		let (client, mut server) = tokio::io::duplex(64);
		server.write_all(b"TASSERVER 0.38\n").await.unwrap();
		let mut client = BufReader::new(client);
		assert!(greets(&mut client).await);
		let mut line = String::new();
		client.read_line(&mut line).await.unwrap();
		assert_eq!(line, "TASSERVER 0.38\n");

		let (client, mut server) = tokio::io::duplex(64);
		server.write_all(b"220 mail.example ESMTP\n").await.unwrap();
		assert!(
			!greets(&mut BufReader::new(client)).await,
			"something else lives on that port"
		);
	}

	#[tokio::test]
	async fn a_server_that_refuses_every_way_is_down_rather_than_unencrypted() {
		// A port nothing listens on: bound to learn a free one, then let go.
		let closed = {
			let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
			listener.local_addr().unwrap().port()
		};
		// Windows retries a refused connect for about two seconds before it
		// says so; a shorter wait would read as a timeout.
		let wait = Duration::from_secs(5);
		for allow_plain in [false, true] {
			let refused = open(&local(vec![closed], allow_plain, None), wait).await;
			assert!(
				matches!(refused, Err(TransportError::Down)),
				"allowing plaintext would not have helped: {:?}",
				refused.err()
			);
		}
	}

	#[tokio::test]
	async fn stls_asks_for_the_upgrade_and_takes_nothing_but_ok() {
		let (client, server) = tokio::io::duplex(256);
		let serve = tokio::spawn(async move {
			let mut server = BufReader::new(server);
			server.write_all(b"TASSERVER 0.38\n").await.unwrap();
			let mut asked = String::new();
			server.read_line(&mut asked).await.unwrap();
			server.write_all(b"OK cmd=STLS\n").await.unwrap();
			asked
		});
		stls_upgrade(client).await.unwrap();
		assert_eq!(serve.await.unwrap(), "STLS\n");

		let (client, mut server) = tokio::io::duplex(256);
		server
			.write_all(b"TASSERVER 0.38\nNO cmd=STLS\n")
			.await
			.unwrap();
		assert!(matches!(
			stls_upgrade(client).await,
			Err(TransportError::Stls(reply)) if reply == "NO cmd=STLS"
		));
	}
}
