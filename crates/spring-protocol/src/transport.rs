//! Tokio transport actor: one task reads lines into [`ServerEvent`]s, one task
//! drains the [`Scheduler`] onto the socket and keeps the heartbeat alive.
//!
//! teiserver listens plain on 8200 (what Chobby uses) and TLS on 8201; both
//! carry the same line protocol, so the actor is generic over the stream.
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
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use crate::codec::{self, RawMessage};
use crate::event::ServerEvent;
use crate::policy::{Area, Envelope, PolicyEvent, Scheduler, ThrottlePolicy};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CHANNEL_CAPACITY: usize = 1024;

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
}

/// Where to connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

impl Endpoint {
    /// Splits `host:port`.
    pub fn parse(addr: &str, tls: bool) -> Option<Self> {
        let (host, port) = addr.rsplit_once(':')?;
        Some(Self {
            host: host.to_owned(),
            port: port.parse().ok()?,
            tls,
        })
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
    /// Connects, wraps the socket in TLS when the endpoint asks for it, and spawns the reader and writer tasks.
    pub async fn connect(
        endpoint: &Endpoint,
        policy: ThrottlePolicy,
    ) -> Result<(Self, mpsc::Receiver<Inbound>), TransportError> {
        let stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port)).await?;
        stream.set_nodelay(true)?;
        if !endpoint.tls {
            return Ok(Self::spawn(stream, policy));
        }
        let name = ServerName::try_from(endpoint.host.clone())
            .map_err(|_| TransportError::ServerName(endpoint.host.clone()))?;
        let stream = tls_connector().connect(name, stream).await?;
        Ok(Self::spawn(stream, policy))
    }

    fn spawn<S>(stream: S, policy: ThrottlePolicy) -> (Self, mpsc::Receiver<Inbound>)
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

    pub async fn shutdown(&self) {
        let _ = self.out.send(Outbound::Shutdown).await;
    }
}

/// Verifies the server against the Mozilla root store bundled by `webpki-roots`.
fn tls_connector() -> TlsConnector {
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
                Some(Outbound::Shutdown) | None => return,
            },
            _ = tokio::time::sleep(wakeup) => {}
        }

        let now = Instant::now();
        if now.duration_since(last_write) >= heartbeat_idle && scheduler.pending() == 0 {
            scheduler.submit(Envelope::immediate(Area::Heartbeat, "PING"));
        }
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
    fn endpoint_splits_host_and_port() {
        assert_eq!(
            Endpoint::parse("server4.beyondallreason.info:8201", true),
            Some(Endpoint {
                host: "server4.beyondallreason.info".into(),
                port: 8201,
                tls: true
            })
        );
        assert_eq!(Endpoint::parse("nope", true), None);
        assert_eq!(Endpoint::parse("host:notaport", true), None);
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
}
