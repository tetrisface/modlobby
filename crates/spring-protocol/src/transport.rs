//! Tokio transport actor: one task reads lines into [`ServerEvent`]s, one task
//! drains the [`Scheduler`] onto the socket and keeps the heartbeat alive.
//!
//! teiserver listens plain on 8200 (what Chobby uses; `STLS` upgrades it to
//! TLS) and TLS on 8201; all carry the same line protocol, so the actor is
//! generic over the stream.
//!
//! Correlation: a request tagged `#<id>` resolves on the first reply line
//! carrying that id; every line, tagged or not, is also delivered as an event,
//! so nothing depends on correlation for state.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use crate::codec::{self, RawMessage};
use crate::event::ServerEvent;
use crate::policy::{Area, Envelope, PolicyEvent, Scheduler, ThrottlePolicy};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CHANNEL_CAPACITY: usize = 1024;
/// How long a fresh connection gets to say its first word before it counts as silent.
const GREETING_WAIT: Duration = Duration::from_secs(5);

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
}

/// Where to connect, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub plain_port: u16,
    pub tls_port: u16,
    pub security: Security,
}

impl Endpoint {
    /// teiserver's own ports: plain on 8200, TLS on 8201.
    pub fn new(host: impl Into<String>, security: Security) -> Self {
        Self {
            host: host.into(),
            plain_port: 8200,
            tls_port: 8201,
            security,
        }
    }
}

/// How the connection is encrypted. Each encrypted way falls back to the
/// other: on 2026-09-18 teiserver's TLS port hung up before greeting while
/// `STLS` on the plain port worked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// The plain port, upgraded to TLS by `STLS`, like SMTP's STARTTLS.
    Stls,
    /// The TLS port, encrypted from the first byte.
    Tls,
    /// Unencrypted on the plain port; no fallback.
    None,
}

impl std::str::FromStr for Security {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stls" => Ok(Self::Stls),
            "tls" => Ok(Self::Tls),
            "none" => Ok(Self::None),
            other => Err(format!("expected stls, tls or none, got {other}")),
        }
    }
}

/// What the transport delivers to the application.
#[derive(Debug)]
pub enum Inbound {
    Message(ServerEvent),
    /// Scheduler decisions worth surfacing (delays, coalescing, trips, drops).
    Policy(PolicyEvent),
    /// Something the transport decided on its own that the user should hear about.
    Note(String),
    Closed {
        reason: String,
    },
}

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
    /// Connects the way the endpoint asks, and spawns the reader and writer tasks.
    ///
    /// An encrypted way that fails, or that the server does not greet on, is
    /// retried the other encrypted way, and the user is told. Never unencrypted:
    /// that is only ever asked for.
    pub async fn connect(
        endpoint: &Endpoint,
        policy: ThrottlePolicy,
    ) -> Result<(Self, mpsc::Receiver<Inbound>), TransportError> {
        let (first, second) = match endpoint.security {
            Security::None => {
                let stream = tcp(&endpoint.host, endpoint.plain_port).await?;
                return Ok(Self::from_stream(stream, policy));
            }
            Security::Stls => (Security::Stls, Security::Tls),
            Security::Tls => (Security::Tls, Security::Stls),
        };
        let err = match encrypted(endpoint, first).await {
            Ok(stream) => return Ok(Self::from_stream(stream, policy)),
            Err(err) => err,
        };
        let (tried, instead) = (way(endpoint, first), way(endpoint, second));
        tracing::warn!(%err, "{tried} failed; trying {instead}");
        let stream = encrypted(endpoint, second).await?;
        let note = format!("{tried} is not working ({err}); connected encrypted via {instead}");
        Ok(Self::spawn(stream, policy, Some(note)))
    }

    /// Runs the protocol over any stream: the socket in production, an in-memory duplex in tests.
    pub fn from_stream<S>(stream: S, policy: ThrottlePolicy) -> (Self, mpsc::Receiver<Inbound>)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::spawn(stream, policy, None)
    }

    fn spawn<S>(
        stream: S,
        policy: ThrottlePolicy,
        note: Option<String>,
    ) -> (Self, mpsc::Receiver<Inbound>)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        let (in_tx, in_rx) = mpsc::channel(CHANNEL_CAPACITY);
        if let Some(note) = note {
            // The channel is new and empty, so this cannot be full.
            let _ = in_tx.try_send(Inbound::Note(note));
        }
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

/// A TLS session the server has greeted on, its greeting still unread.
/// `Security::Stls` upgrades the plain port; anything else is the TLS port.
async fn encrypted(
    endpoint: &Endpoint,
    security: Security,
) -> Result<BufReader<TlsStream<TcpStream>>, TransportError> {
    let name = ServerName::try_from(endpoint.host.clone())
        .map_err(|_| TransportError::ServerName(endpoint.host.clone()))?;
    let socket = if security == Security::Stls {
        stls_upgrade(tcp(&endpoint.host, endpoint.plain_port).await?).await?
    } else {
        tcp(&endpoint.host, endpoint.tls_port).await?
    };
    let mut stream = BufReader::new(tls_connector().connect(name, socket).await?);
    if !greets(&mut stream).await {
        return Err(TransportError::Silent);
    }
    Ok(stream)
}

/// How a way of connecting reads in a notice.
fn way(endpoint: &Endpoint, security: Security) -> String {
    match security {
        Security::Stls => format!("STLS on port {}", endpoint.plain_port),
        Security::Tls => format!("TLS on port {}", endpoint.tls_port),
        Security::None => format!("unencrypted on port {}", endpoint.plain_port),
    }
}

async fn tcp(host: &str, port: u16) -> Result<TcpStream, TransportError> {
    let stream = TcpStream::connect((host, port)).await?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// Whether the server says anything before it hangs up. The first bytes are
/// waited for but left in the buffer, so the greeting still reaches the session.
async fn greets<S: AsyncRead + Unpin>(stream: &mut BufReader<S>) -> bool {
    matches!(
        tokio::time::timeout(GREETING_WAIT, stream.fill_buf()).await,
        Ok(Ok(buf)) if !buf.is_empty()
    )
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

/// Verifies the server against the Mozilla root store bundled by `webpki-roots`.
fn tls_connector() -> TlsConnector {
    install_crypto();
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
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

    #[test]
    fn security_reads_the_names_the_cli_takes() {
        assert_eq!("stls".parse(), Ok(Security::Stls));
        assert_eq!("tls".parse(), Ok(Security::Tls));
        assert_eq!("none".parse(), Ok(Security::None));
        assert!("plain".parse::<Security>().is_err());
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
