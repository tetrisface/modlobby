//! Outbound throttle policy.
//!
//! Two independent enforcers punish a chatty client: teiserver's per-connection
//! bucket (`DISCONNECT Flood protection`, then ~10 s of blocked logins) and
//! SPADS's per-battle windows (`KICKFROMBATTLE`, a 5-minute ban on repeat, or a
//! 2-minute command ignore). Every outbound line therefore passes through a
//! [`Scheduler`] whose limits are plain data ([`ThrottlePolicy`]) keyed by the
//! server-side enforcement point ([`Area`]), so any area can be retuned without
//! touching code.
//!
//! Time is passed in explicitly as [`Instant`] so the scheduler is deterministic
//! under test.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A server-side enforcement point. See the plan's policy table for sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Login,
    Heartbeat,
    /// `MYSTATUS`.
    Status,
    /// `MYBATTLESTATUS` (SPADS `statusFloodAutoKick`).
    BattleStatus,
    /// `SAYBATTLE`/`SAYBATTLEEX` (SPADS `msgFloodAutoKick`).
    BattleChat,
    /// `!` commands (SPADS `cmdFloodAutoIgnore`).
    BattleCommand,
    /// `SAY`/`SAYEX`.
    ChannelChat,
    Ring,
    Other,
}

/// Fixed drain order, so scheduling is deterministic.
const AREAS: [Area; 9] = [
    Area::Heartbeat,
    Area::Login,
    Area::Status,
    Area::BattleStatus,
    Area::BattleCommand,
    Area::BattleChat,
    Area::ChannelChat,
    Area::Ring,
    Area::Other,
];

/// How an area is limited.
///
/// `Bucket` mirrors teiserver's `BurstyRateLimiter`; `Window` mirrors the
/// sliding windows SPADS and teiserver's `MYSTATUS` check use, and — unlike a
/// bucket — guarantees "at most `max` in any `window_secs`".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Limit {
    Bucket { burst: u32, per_minute: f64 },
    Window { max: u32, window_secs: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginBackoff {
    /// teiserver errors on a reconnect right after a disconnect; Chobby waits 3 s.
    pub after_disconnect_secs: f64,
    /// A flood disconnect sets the server-side login counter above its limit for a 10 s TTL.
    pub after_flood_secs: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThrottlePolicy {
    /// teiserver: 200 commands/min per connection, overflow disconnects.
    pub connection: Limit,
    pub areas: HashMap<Area, Limit>,
    pub login: LoginBackoff,
    /// teiserver drops idle connections after 120 s; Chobby pings at 30 s.
    pub heartbeat_idle_secs: f64,
    /// Lines longer than this are dropped client-side (server buffer limit is 64 KiB).
    pub max_line_bytes: usize,
}

impl Default for ThrottlePolicy {
    fn default() -> Self {
        use Limit::{Bucket, Window};
        let areas = HashMap::from([
            (
                Area::Heartbeat,
                Bucket {
                    burst: 2,
                    per_minute: 4.0,
                },
            ),
            (
                Area::Login,
                Window {
                    max: 3,
                    window_secs: 10.0,
                },
            ),
            (
                Area::Status,
                Window {
                    max: 1,
                    window_secs: 1.0,
                },
            ),
            (
                Area::BattleStatus,
                Window {
                    max: 5,
                    window_secs: 8.0,
                },
            ),
            (
                Area::BattleChat,
                Window {
                    max: 4,
                    window_secs: 7.0,
                },
            ),
            (
                Area::BattleCommand,
                Window {
                    max: 3,
                    window_secs: 4.0,
                },
            ),
            (
                Area::ChannelChat,
                Window {
                    max: 4,
                    window_secs: 5.0,
                },
            ),
            (
                Area::Ring,
                Window {
                    max: 4,
                    window_secs: 10.0,
                },
            ),
            (
                Area::Other,
                Bucket {
                    burst: 20,
                    per_minute: 120.0,
                },
            ),
        ]);
        Self {
            connection: Bucket {
                burst: 40,
                per_minute: 150.0,
            },
            areas,
            login: LoginBackoff {
                after_disconnect_secs: 3.0,
                after_flood_secs: 10.0,
            },
            heartbeat_idle_secs: 30.0,
            max_line_bytes: 60 * 1024,
        }
    }
}

impl ThrottlePolicy {
    pub fn heartbeat_idle(&self) -> Duration {
        Duration::from_secs_f64(self.heartbeat_idle_secs)
    }
}

/// How a line should be scheduled within its area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Skips the area queue (still counted against the connection bucket).
    Immediate,
    /// Last write wins for lines sharing a key while queued (`MYSTATUS`, `MYBATTLESTATUS`).
    Coalesce(String),
    /// Ordered and paced.
    Queue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub area: Area,
    pub mode: Mode,
    pub line: String,
}

impl Envelope {
    pub fn immediate(area: Area, line: impl Into<String>) -> Self {
        Self {
            area,
            mode: Mode::Immediate,
            line: line.into(),
        }
    }

    pub fn queue(area: Area, line: impl Into<String>) -> Self {
        Self {
            area,
            mode: Mode::Queue,
            line: line.into(),
        }
    }

    pub fn coalesce(area: Area, key: impl Into<String>, line: impl Into<String>) -> Self {
        Self {
            area,
            mode: Mode::Coalesce(key.into()),
            line: line.into(),
        }
    }
}

/// What the scheduler did that a user might want to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyEvent {
    Delayed {
        area: Area,
        pending: usize,
        wait: Duration,
    },
    Coalesced {
        area: Area,
        key: String,
    },
    Tripped {
        area: Area,
        until: Instant,
    },
    Dropped {
        area: Area,
        bytes: usize,
    },
}

#[derive(Debug)]
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(burst: u32, per_minute: f64, now: Instant) -> Self {
        Self {
            capacity: f64::from(burst),
            tokens: f64::from(burst),
            per_sec: per_minute / 60.0,
            last: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_sec).min(self.capacity);
        self.last = now;
    }

    fn wait(&mut self, now: Instant) -> Duration {
        self.refill(now);
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((1.0 - self.tokens) / self.per_sec)
        }
    }

    fn take(&mut self, now: Instant) {
        self.refill(now);
        self.tokens -= 1.0;
    }
}

#[derive(Debug)]
struct SlidingWindow {
    max: usize,
    window: Duration,
    sent: VecDeque<Instant>,
}

impl SlidingWindow {
    fn new(max: u32, window_secs: f64) -> Self {
        Self {
            max: max as usize,
            window: Duration::from_secs_f64(window_secs),
            sent: VecDeque::new(),
        }
    }

    fn prune(&mut self, now: Instant) {
        while self
            .sent
            .front()
            .is_some_and(|&t| now.saturating_duration_since(t) >= self.window)
        {
            self.sent.pop_front();
        }
    }

    fn wait(&mut self, now: Instant) -> Duration {
        self.prune(now);
        match self.sent.front() {
            Some(&oldest) if self.sent.len() >= self.max => {
                (oldest + self.window).saturating_duration_since(now)
            }
            _ => Duration::ZERO,
        }
    }

    fn take(&mut self, now: Instant) {
        self.prune(now);
        self.sent.push_back(now);
    }
}

#[derive(Debug)]
enum Limiter {
    Bucket(TokenBucket),
    Window(SlidingWindow),
}

impl Limiter {
    fn new(limit: Limit, now: Instant) -> Self {
        match limit {
            Limit::Bucket { burst, per_minute } => {
                Self::Bucket(TokenBucket::new(burst, per_minute, now))
            }
            Limit::Window { max, window_secs } => {
                Self::Window(SlidingWindow::new(max, window_secs))
            }
        }
    }

    fn wait(&mut self, now: Instant) -> Duration {
        match self {
            Self::Bucket(b) => b.wait(now),
            Self::Window(w) => w.wait(now),
        }
    }

    fn take(&mut self, now: Instant) {
        match self {
            Self::Bucket(b) => b.take(now),
            Self::Window(w) => w.take(now),
        }
    }
}

/// Gates outbound lines per area and per connection; see the module docs.
#[derive(Debug)]
pub struct Scheduler {
    policy: ThrottlePolicy,
    connection: Limiter,
    limiters: HashMap<Area, Limiter>,
    queues: HashMap<Area, VecDeque<Envelope>>,
    immediate: VecDeque<Envelope>,
    tripped: HashMap<Area, Instant>,
    events: Vec<PolicyEvent>,
}

impl Scheduler {
    pub fn new(policy: ThrottlePolicy, now: Instant) -> Self {
        let connection = Limiter::new(policy.connection, now);
        let limiters = AREAS
            .iter()
            .map(|&area| {
                let limit = policy.areas.get(&area).copied().unwrap_or(Limit::Bucket {
                    burst: 20,
                    per_minute: 120.0,
                });
                (area, Limiter::new(limit, now))
            })
            .collect();
        Self {
            policy,
            connection,
            limiters,
            queues: AREAS.iter().map(|&a| (a, VecDeque::new())).collect(),
            immediate: VecDeque::new(),
            tripped: HashMap::new(),
            events: Vec::new(),
        }
    }

    pub fn policy(&self) -> &ThrottlePolicy {
        &self.policy
    }

    pub fn submit(&mut self, envelope: Envelope) {
        if envelope.line.len() > self.policy.max_line_bytes {
            self.events.push(PolicyEvent::Dropped {
                area: envelope.area,
                bytes: envelope.line.len(),
            });
            return;
        }
        match &envelope.mode {
            Mode::Immediate => self.immediate.push_back(envelope),
            Mode::Coalesce(key) => {
                let queue = self.queues.entry(envelope.area).or_default();
                match queue
                    .iter_mut()
                    .find(|e| matches!(&e.mode, Mode::Coalesce(k) if k == key))
                {
                    Some(existing) => {
                        existing.line = envelope.line;
                        self.events.push(PolicyEvent::Coalesced {
                            area: envelope.area,
                            key: key.clone(),
                        });
                    }
                    None => queue.push_back(envelope),
                }
            }
            Mode::Queue => self
                .queues
                .entry(envelope.area)
                .or_default()
                .push_back(envelope),
        }
    }

    /// Pauses an area until `until`, e.g. after a flood signal from the server.
    pub fn trip(&mut self, area: Area, until: Instant) {
        self.tripped.insert(area, until);
        self.events.push(PolicyEvent::Tripped { area, until });
    }

    /// Lines that may be written now, in send order.
    pub fn drain(&mut self, now: Instant) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(front) = self.immediate.front() {
            if !self.connection.wait(now).is_zero() {
                self.events.push(PolicyEvent::Delayed {
                    area: front.area,
                    pending: self.immediate.len(),
                    wait: self.connection.wait(now),
                });
                return out;
            }
            self.connection.take(now);
            out.push(self.immediate.pop_front().expect("front exists").line);
        }
        for area in AREAS {
            loop {
                let pending = match self.queues.get(&area) {
                    Some(queue) if !queue.is_empty() => queue.len(),
                    _ => break,
                };
                let wait = self.area_wait(area, now);
                if !wait.is_zero() {
                    self.events.push(PolicyEvent::Delayed {
                        area,
                        pending,
                        wait,
                    });
                    break;
                }
                self.connection.take(now);
                self.limiters
                    .get_mut(&area)
                    .expect("limiter exists")
                    .take(now);
                let envelope = self
                    .queues
                    .get_mut(&area)
                    .and_then(VecDeque::pop_front)
                    .expect("queue non-empty");
                out.push(envelope.line);
            }
        }
        out
    }

    /// How long until [`Scheduler::drain`] could produce another line, if anything is pending.
    pub fn next_wakeup(&mut self, now: Instant) -> Option<Duration> {
        let mut waits = Vec::new();
        if !self.immediate.is_empty() {
            waits.push(self.connection.wait(now));
        }
        for area in AREAS {
            if self.queues.get(&area).is_some_and(|q| !q.is_empty()) {
                waits.push(self.area_wait(area, now));
            }
        }
        waits.into_iter().min()
    }

    pub fn pending(&self) -> usize {
        self.immediate.len() + self.queues.values().map(VecDeque::len).sum::<usize>()
    }

    pub fn take_events(&mut self) -> Vec<PolicyEvent> {
        std::mem::take(&mut self.events)
    }

    fn area_wait(&mut self, area: Area, now: Instant) -> Duration {
        let tripped = self.tripped.get(&area).map_or(Duration::ZERO, |&until| {
            until.saturating_duration_since(now)
        });
        let limiter = self
            .limiters
            .get_mut(&area)
            .expect("limiter exists")
            .wait(now);
        tripped.max(limiter).max(self.connection.wait(now))
    }
}

/// teiserver's `SAYBATTLE` cap for a message (`spring_in.ex`): `String.slice(0..n)` is an
/// inclusive range, so each class allows `n + 1` characters.
pub fn saybattle_max_len(message: &str) -> usize {
    const LONG_PREFIXES: [&str; 4] = [
        "$welcome-message",
        "!welcome-message",
        "!mode ",
        "!bset mapmetadata",
    ];
    let lower = message.to_ascii_lowercase();
    if lower.starts_with("!bset tweakdefs") || lower.starts_with("!bset tweakunits") {
        16_385
    } else if LONG_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        1025
    } else {
        257
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler() -> (Scheduler, Instant) {
        let now = Instant::now();
        (Scheduler::new(ThrottlePolicy::default(), now), now)
    }

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    #[test]
    fn window_never_exceeds_max_per_window() {
        let (mut s, t0) = scheduler();
        for i in 0..30 {
            s.submit(Envelope::queue(
                Area::BattleChat,
                format!("SAYBATTLE line {i}"),
            ));
        }
        let mut sent = Vec::new();
        for tick in 0..240 {
            let now = t0 + secs(f64::from(tick) * 0.5);
            for _ in s.drain(now) {
                sent.push(now);
            }
        }
        assert_eq!(sent.len(), 30);
        for (i, &start) in sent.iter().enumerate() {
            let in_window = sent[i..]
                .iter()
                .filter(|&&t| t.duration_since(start) < secs(7.0))
                .count();
            assert!(
                in_window <= 4,
                "{in_window} lines within 7 s starting at {i}"
            );
        }
    }

    #[test]
    fn coalesce_keeps_only_the_latest_line() {
        let (mut s, t0) = scheduler();
        for status in [
            "MYBATTLESTATUS 1 0",
            "MYBATTLESTATUS 2 0",
            "MYBATTLESTATUS 3 0",
        ] {
            s.submit(Envelope::coalesce(Area::BattleStatus, "me", status));
        }
        assert_eq!(s.drain(t0), vec!["MYBATTLESTATUS 3 0"]);
        let coalesced = s
            .take_events()
            .into_iter()
            .filter(|e| matches!(e, PolicyEvent::Coalesced { .. }))
            .count();
        assert_eq!(coalesced, 2);
    }

    #[test]
    fn immediate_respects_connection_bucket() {
        let (mut s, t0) = scheduler();
        for _ in 0..50 {
            s.submit(Envelope::immediate(Area::Heartbeat, "PING"));
        }
        assert_eq!(
            s.drain(t0).len(),
            40,
            "burst of the default connection bucket"
        );
        assert_eq!(s.pending(), 10);
        assert!(s.next_wakeup(t0).is_some_and(|w| w > Duration::ZERO));
        assert_eq!(s.drain(t0 + secs(60.0)).len(), 10);
    }

    #[test]
    fn tripped_area_waits_until_deadline() {
        let (mut s, t0) = scheduler();
        s.submit(Envelope::queue(
            Area::BattleCommand,
            "SAYBATTLE !bset foo 1",
        ));
        s.trip(Area::BattleCommand, t0 + secs(10.0));
        assert!(s.drain(t0 + secs(9.0)).is_empty());
        assert_eq!(s.next_wakeup(t0 + secs(9.0)), Some(secs(1.0)));
        assert_eq!(s.drain(t0 + secs(10.0)).len(), 1);
    }

    #[test]
    fn oversized_lines_are_dropped() {
        let (mut s, t0) = scheduler();
        s.submit(Envelope::queue(Area::BattleChat, "x".repeat(70 * 1024)));
        assert!(s.drain(t0).is_empty());
        assert!(matches!(
            s.take_events().as_slice(),
            [PolicyEvent::Dropped { .. }]
        ));
    }

    #[test]
    fn saybattle_length_classes() {
        assert_eq!(saybattle_max_len("hello"), 257);
        assert_eq!(saybattle_max_len("!mode ffa"), 1025);
        assert_eq!(saybattle_max_len("!bSet tweakDefs abc"), 16_385);
    }

    #[test]
    fn policy_round_trips_through_toml() {
        let policy = ThrottlePolicy::default();
        let text = toml::to_string(&policy).expect("serialise");
        let back: ThrottlePolicy = toml::from_str(&text).expect("parse");
        assert_eq!(back, policy);
    }
}
