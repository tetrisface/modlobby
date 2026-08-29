//! Tokio transport actor: one task reads lines into [`ServerEvent`]s, one task
//! drains the [`Scheduler`] onto the socket and keeps the heartbeat alive.
//!
//! Correlation: a request tagged `#<id>` resolves on the first reply line
//! carrying that id; every line, tagged or not, is also delivered as an event,
//! so nothing depends on correlation for state.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot};

use crate::codec::{self, RawMessage};
use crate::event::ServerEvent;
use crate::policy::{Area, Envelope, PolicyEvent, Scheduler, ThrottlePolicy};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("connect: {0}")]
    Connect(#[from] std::io::Error),
    #[error("transport closed")]
    Closed,
    #[error("no reply within {0:?}")]
    Timeout(Duration),
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
    /// Connects over plain TCP (what Chobby uses on port 8200) and spawns the reader and writer tasks.
    pub async fn connect(
        addr: &str,
        policy: ThrottlePolicy,
    ) -> Result<(Self, mpsc::Receiver<Inbound>), TransportError> {
        let stream = TcpStream::connect(addr).await?;
        stream.set_nodelay(true)?;
        let (read_half, write_half) = stream.into_split();
        let (in_tx, in_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (out_tx, out_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let pending = Pending::default();

        tokio::spawn(reader(read_half, in_tx.clone(), Arc::clone(&pending)));
        tokio::spawn(writer(write_half, out_rx, in_tx, pending, policy));

        Ok((
            Self {
                out: out_tx,
                next_id: Arc::default(),
            },
            in_rx,
        ))
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

async fn reader(read_half: OwnedReadHalf, in_tx: mpsc::Sender<Inbound>, pending: Pending) {
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

async fn writer(
    mut write_half: OwnedWriteHalf,
    mut out_rx: mpsc::Receiver<Outbound>,
    in_tx: mpsc::Sender<Inbound>,
    pending: Pending,
    policy: ThrottlePolicy,
) {
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
            tracing::trace!(target: "spring::tx", "{line}");
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
