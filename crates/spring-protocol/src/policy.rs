//! Outbound throttle policy.
//!
//! Every outbound line passes through a [`Scheduler`] whose limits are plain
//! data ([`ThrottlePolicy`]) keyed by the server-side enforcement point
//! ([`Area`]). The numbers ship from [`Default`]; retune them there. The
//! CLI's `--policy` takes the same shape as TOML for experiments.
//!
//! # What the servers actually enforce
//!
//! Decoded from the sources under `external/` and the teiserver checkout,
//! 2026-09. Line numbers are from those checkouts.
//!
//! ## teiserver: one permit per TCP read, not per line
//!
//! `spring_tcp_server.ex` `handle_info({:tcp, _, data})` runs `flood_protect?`
//! once per chunk the socket delivers, and only then `SpringIn.data_in` splits
//! the chunk on newlines and handles every line. The limiter is
//! `BurstyRateLimiter.per_minute(200)` (site config "teiserver.Spring rate
//! limit per minute"): 200 permits, one back every 300 ms. Going over answers
//! `DISCONNECT Flood protection`, drops the socket, and refuses login for
//! about 10 s (the login-attempt counter is set past its limit with a 10 s
//! TTL; the refusal text says 20). Admins, moderators and bot accounts are
//! exempt. The limiter grants on any positive fraction of a permit and lets
//! the store go negative, so the wait after an overshoot grows with it.
//!
//! So the unit of cost is the write, and a large write costs about as many
//! permits as it has TCP segments. The scheduler therefore charges the
//! connection bucket per write by segments ([`Paste::segment_bytes`]), which
//! is also why the burst lane joins many lines into one write. A partial line
//! is buffered up to 64 KiB ("teiserver.Spring max message buffer size"),
//! hence [`ThrottlePolicy::max_line_bytes`]; a chunk without a trailing
//! newline is buffered whole, complete lines and all, until one arrives.
//!
//! ## SPADS: two per-user counters, both skipped above access level 100
//!
//! `cbSaidBattle` (`spads.pl`, near line 14610) runs `checkUserMsgFlood` for
//! every `SAIDBATTLE` from a user, commands included; `handleRequest` (near
//! line 3205) then runs `checkCmdFlood` for any line matching `^!\w`, valid
//! command or not. The public cluster hosts run
//! `spads_config_bar/etc/spads_cluster.conf` (`docker/scripts/run-spads.sh`);
//! `barmanager_spads.conf` is the older standalone setup:
//!
//! | counter  | cluster                     | standalone | on the Nth line        |
//! |----------|-----------------------------|------------|------------------------|
//! | messages | `msgFloodAutoKick:15;7`     | same       | `KICKFROMBATTLE`       |
//! | commands | `cmdFloodAutoIgnore:20;5;1` | `8;8;4`    | ignored 1 (4) minutes  |
//! | status   | `statusFloodAutoKick:24;8`  | same       | `KICKFROMBATTLE`       |
//!
//! Both counters bucket by whole seconds (`time`), count the current line
//! before comparing, and drop a bucket only once `time - ts > window`, so a
//! window of W seconds spans W+1 buckets: between 6 and 8 real seconds for
//! the message counter. Read as "never more than": 14 messages in 8 s, and
//! 19 commands in 6 s on the cluster or 7 in 9 s standalone. Commands count
//! as messages too, so on the cluster the message counter is the one that
//! binds. The paced lane below is tuned to the standalone numbers, which fit
//! the cluster's as well.
//!
//! Both counters are skipped for a sender at `floodImmuneLevel` (100) or
//! above: moderators in `users.conf`, and any current boss, whom BarManager
//! lifts to exactly 100 (`barmanager.py`, `changeUserAccessLevel`), which is
//! also why a boss's `!bSet` runs without a vote. That immunity is what the
//! burst lane relies on. The owner reports being kicked while boss in a full
//! room, which this reading does not explain; the mechanism is still open,
//! hence [`PasteBurst`] being the first thing to tune.
//!
//! ## Chobby, for comparison
//!
//! A paste under 20000 characters goes out line by line in one frame
//! (`interface.lua`, `SayBattle`); over that, `SayBattleThrottled` sends
//! 20000-character batches of whole lines 4 s apart. It never looks at
//! SPADS's counters: as boss it is immune, otherwise the 15th line gets it
//! kicked. The 4 s is not arbitrary: a 20000-byte batch is about 14 segments,
//! and teiserver refills 14 permits in 4.2 s.
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
    /// A multi-line battle-room paste on the burst lane: whole lines are
    /// joined into one write per batch (see [`Paste`]). Only for a sender
    /// SPADS does not count, so its limit is teiserver's alone.
    BattlePaste,
    /// `!` commands (SPADS `cmdFloodAutoIgnore`).
    BattleCommand,
    /// `SAY`/`SAYEX`.
    ChannelChat,
    Ring,
    Other,
}

/// Fixed drain order, so scheduling is deterministic.
const AREAS: [Area; 10] = [
    Area::Heartbeat,
    Area::Login,
    Area::Status,
    Area::BattleStatus,
    Area::BattleCommand,
    Area::BattleChat,
    Area::BattlePaste,
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
    /// How a multi-line battle-room paste is sent.
    #[serde(default)]
    pub paste: Paste,
}

/// How a multi-line battle-room paste is sent. Single lines never take the
/// burst lane: one `!vote` should not wait on a batch interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Paste {
    pub burst: PasteBurst,
    /// Most bytes joined into one write on the burst lane. Chobby's 20000
    /// (`interface.lua`, `THROTTLED_SAY_MAX_CHARS_PER_BATCH`): about 14
    /// segments, which the connection bucket gets back in one batch interval.
    pub batch_bytes: usize,
    /// Bytes per TCP segment assumed when charging a write against the
    /// connection bucket: the usual Ethernet MSS. teiserver counts reads, and
    /// a read is one segment when the client writes faster than it reads.
    pub segment_bytes: usize,
}

impl Default for Paste {
    fn default() -> Self {
        Self {
            burst: PasteBurst::Boss,
            batch_bytes: 20_000,
            segment_bytes: 1460,
        }
    }
}

/// When a paste takes the burst lane rather than being paced line by line
/// under SPADS's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteBurst {
    /// Burst while SPADS reports us as boss, the one state known to lift a
    /// sender past `floodImmuneLevel`; pace otherwise. The owner has been
    /// kicked as boss in a full room, so this is the setting to change first
    /// when tuning.
    Boss,
    /// Always burst: what Chobby does. Kicks a sender SPADS is counting.
    Always,
    /// Never burst, even as boss.
    Never,
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
            // The paced lane, shaped by SPADS's counters (module docs): at most
            // 7 commands in any 9 s and 14 messages in any 8 s, commands
            // included. The windows are a second wider than the counters so a
            // burst landing just across a whole-second boundary still fits,
            // and chat plus commands together stay under 14 in 8 s.
            (
                Area::BattleChat,
                Window {
                    max: 4,
                    window_secs: 9.0,
                },
            ),
            (
                Area::BattleCommand,
                Window {
                    max: 7,
                    window_secs: 10.0,
                },
            ),
            // The burst lane: one batch per interval, Chobby's 4 s. Nothing
            // else limits it but the connection bucket.
            (
                Area::BattlePaste,
                Window {
                    max: 1,
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
            paste: Paste::default(),
        }
    }
}

impl ThrottlePolicy {
    pub fn heartbeat_idle(&self) -> Duration {
        Duration::from_secs_f64(self.heartbeat_idle_secs)
    }

    /// Permits one write of `bytes` costs on the connection: one per segment
    /// it will arrive in, since teiserver counts reads.
    pub fn permits(&self, bytes: usize) -> usize {
        1 + bytes / self.paste.segment_bytes.max(1)
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
    /// One write left, carrying `lines` protocol lines; `pending` is what
    /// the area still holds. How a paste's progress is counted.
    Sent {
        area: Area,
        lines: usize,
        pending: usize,
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

    /// A write needing more than the bucket holds waits for a full bucket;
    /// it can never wait for more.
    fn wait_for(&mut self, permits: usize, now: Instant) -> Duration {
        self.refill(now);
        let needed = (permits as f64).min(self.capacity);
        if self.tokens >= needed {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((needed - self.tokens) / self.per_sec)
        }
    }

    fn take_n(&mut self, permits: usize, now: Instant) {
        self.refill(now);
        self.tokens -= permits as f64;
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

    fn wait_for(&mut self, permits: usize, now: Instant) -> Duration {
        self.prune(now);
        let permits = permits.clamp(1, self.max.max(1));
        let over = self.sent.len() + permits;
        if over <= self.max {
            return Duration::ZERO;
        }
        // The write fits once this many of the oldest have left the window.
        let oldest = self.sent[over - self.max - 1];
        (oldest + self.window).saturating_duration_since(now)
    }

    fn take_n(&mut self, permits: usize, now: Instant) {
        self.prune(now);
        for _ in 0..permits {
            self.sent.push_back(now);
        }
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

    fn wait_for(&mut self, permits: usize, now: Instant) -> Duration {
        match self {
            Self::Bucket(b) => b.wait_for(permits, now),
            Self::Window(w) => w.wait_for(permits, now),
        }
    }

    fn take_n(&mut self, permits: usize, now: Instant) {
        match self {
            Self::Bucket(b) => b.take_n(permits, now),
            Self::Window(w) => w.take_n(permits, now),
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

    /// Drops everything still queued in `area`, returning how many lines that
    /// was. What has already left cannot be recalled.
    pub fn cancel(&mut self, area: Area) -> usize {
        self.queues.get_mut(&area).map_or(0, |queue| {
            let dropped = queue.len();
            queue.clear();
            dropped
        })
    }

    /// Pauses an area until `until`, e.g. after a flood signal from the server.
    pub fn trip(&mut self, area: Area, until: Instant) {
        self.tripped.insert(area, until);
        self.events.push(PolicyEvent::Tripped { area, until });
    }

    /// Writes that may go out now, in send order. Each is one write on the
    /// socket: a single protocol line, or on [`Area::BattlePaste`] several
    /// joined with newlines, which teiserver splits again on arrival.
    pub fn drain(&mut self, now: Instant) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(front) = self.immediate.front() {
            let permits = self.policy.permits(front.line.len() + 1);
            let wait = self.connection.wait_for(permits, now);
            if !wait.is_zero() {
                self.events.push(PolicyEvent::Delayed {
                    area: front.area,
                    pending: self.immediate.len(),
                    wait,
                });
                return out;
            }
            self.connection.take_n(permits, now);
            let envelope = self.immediate.pop_front().expect("front exists");
            out.push(envelope.line);
            self.events.push(PolicyEvent::Sent {
                area: envelope.area,
                lines: 1,
                pending: self.immediate.len(),
            });
        }
        for area in AREAS {
            while let Some(write) = self.next_write(area) {
                let wait = self.area_wait(area, write.permits, now);
                if !wait.is_zero() {
                    self.events.push(PolicyEvent::Delayed {
                        area,
                        pending: self.queues[&area].len(),
                        wait,
                    });
                    break;
                }
                self.connection.take_n(write.permits, now);
                self.limiters
                    .get_mut(&area)
                    .expect("limiter exists")
                    .take_n(1, now);
                let queue = self.queues.get_mut(&area).expect("queue exists");
                let lines: Vec<String> = queue.drain(..write.envelopes).map(|e| e.line).collect();
                let pending = queue.len();
                out.push(lines.join("\n"));
                self.events.push(PolicyEvent::Sent {
                    area,
                    lines: write.envelopes,
                    pending,
                });
            }
        }
        out
    }

    /// The next write an area would make: one line, or on the burst lane as
    /// many whole lines as fit in [`Paste::batch_bytes`] (always at least one).
    fn next_write(&self, area: Area) -> Option<NextWrite> {
        let queue = self.queues.get(&area)?;
        let first = queue.front()?;
        let mut envelopes = 1;
        let mut bytes = first.line.len() + 1;
        if area == Area::BattlePaste {
            for envelope in queue.iter().skip(1) {
                let more = envelope.line.len() + 1;
                if bytes + more > self.policy.paste.batch_bytes {
                    break;
                }
                envelopes += 1;
                bytes += more;
            }
        }
        Some(NextWrite {
            envelopes,
            permits: self.policy.permits(bytes),
        })
    }

    /// How long until [`Scheduler::drain`] could produce another line, if anything is pending.
    pub fn next_wakeup(&mut self, now: Instant) -> Option<Duration> {
        let mut waits = Vec::new();
        if let Some(front) = self.immediate.front() {
            let permits = self.policy.permits(front.line.len() + 1);
            waits.push(self.connection.wait_for(permits, now));
        }
        for area in AREAS {
            if let Some(write) = self.next_write(area) {
                waits.push(self.area_wait(area, write.permits, now));
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

    fn area_wait(&mut self, area: Area, permits: usize, now: Instant) -> Duration {
        let tripped = self.tripped.get(&area).map_or(Duration::ZERO, |&until| {
            until.saturating_duration_since(now)
        });
        let limiter = self
            .limiters
            .get_mut(&area)
            .expect("limiter exists")
            .wait_for(1, now);
        tripped
            .max(limiter)
            .max(self.connection.wait_for(permits, now))
    }
}

/// What [`Scheduler::next_write`] found at the front of a queue.
struct NextWrite {
    envelopes: usize,
    permits: usize,
}

/// teiserver's `SAYBATTLE` cap for a message (`spring_in.ex`): `String.slice(0..n)` is an
/// inclusive range, so each class allows `n + 1` characters. The slice counts
/// graphemes where this crate counts scalars, so this side is the stricter one.
/// `!bset mapmetadata` is not in the 2026-07 teiserver checkout; if the live
/// server does not know it, such a line past 257 is cut there without a word.
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
    fn a_write_costs_one_permit_per_segment() {
        let policy = ThrottlePolicy::default();
        assert_eq!(policy.permits(10), 1);
        assert_eq!(policy.permits(1460), 2);
        assert_eq!(policy.permits(20_000), 14);
        // A tweak blob alone: what teiserver sees as a dozen reads.
        assert_eq!(policy.permits(16_000), 11);
    }

    #[test]
    fn the_burst_lane_joins_whole_lines_into_one_write_per_interval() {
        let mut policy = ThrottlePolicy::default();
        policy.paste.batch_bytes = 30;
        let t0 = Instant::now();
        let mut s = Scheduler::new(policy, t0);
        for line in ["!bSet a 1", "!bSet b 22", "!bSet c 333", "hello"] {
            s.submit(Envelope::queue(Area::BattlePaste, line));
        }
        // 10 + 11 = 21 fits, 21 + 12 = 33 does not.
        assert_eq!(s.drain(t0), vec!["!bSet a 1\n!bSet b 22"]);
        assert!(s.drain(t0 + secs(3.9)).is_empty(), "one batch per interval");
        assert_eq!(s.drain(t0 + secs(4.0)), vec!["!bSet c 333\nhello"]);
        assert_eq!(s.pending(), 0);
    }

    #[test]
    fn a_big_write_waits_for_the_permits_it_needs() {
        let policy = ThrottlePolicy {
            connection: Limit::Bucket {
                burst: 20,
                per_minute: 60.0,
            },
            ..ThrottlePolicy::default()
        };
        let t0 = Instant::now();
        let mut s = Scheduler::new(policy, t0);
        let blob = "x".repeat(20_000);
        s.submit(Envelope::queue(Area::BattlePaste, blob.clone()));
        s.submit(Envelope::queue(Area::BattlePaste, blob));
        // 14 permits of 20: the first goes at once; the second needs 8 more
        // at one a second, so the batch interval is not what it waits on.
        assert_eq!(s.drain(t0).len(), 1);
        assert!(s.drain(t0 + secs(4.0)).is_empty());
        assert!(s.drain(t0 + secs(7.9)).is_empty());
        assert_eq!(s.drain(t0 + secs(8.0)).len(), 1);
    }

    #[test]
    fn cancelling_an_area_drops_what_has_not_left() {
        let t0 = Instant::now();
        let mut s = Scheduler::new(ThrottlePolicy::default(), t0);
        for line in ["!bSet a 1", "!bSet b 2", "!bSet c 3"] {
            s.submit(Envelope::queue(Area::BattleCommand, line));
        }
        s.submit(Envelope::queue(Area::BattleChat, "hi"));
        assert_eq!(s.cancel(Area::BattleCommand), 3);
        assert_eq!(s.cancel(Area::BattleCommand), 0);
        assert_eq!(s.drain(t0), vec!["hi"]);
    }

    #[test]
    fn an_older_policy_file_without_paste_still_loads() {
        let text = toml::to_string(&ThrottlePolicy::default()).expect("serialise");
        let without: String = text
            .lines()
            .take_while(|line| !line.starts_with("[paste]"))
            .collect::<Vec<_>>()
            .join("\n");
        let policy: ThrottlePolicy = toml::from_str(&without).expect("parse");
        assert_eq!(policy.paste, Paste::default());
    }

    #[test]
    fn policy_round_trips_through_toml() {
        let policy = ThrottlePolicy::default();
        let text = toml::to_string(&policy).expect("serialise");
        let back: ThrottlePolicy = toml::from_str(&text).expect("parse");
        assert_eq!(back, policy);
    }
}
