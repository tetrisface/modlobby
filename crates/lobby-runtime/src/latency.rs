//! How far away a host machine is, measured by ICMP echo.
//!
//! Every room a cluster runs sits on one machine, so a handful of probes
//! covers every spare on the list. Raw ICMP sockets need privileges, which is
//! why this goes through the Windows IP Helper API instead: it works from an
//! ordinary process. Elsewhere nothing is measured and the choice of room
//! falls back to cluster headroom alone.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// One round trip to a machine, or `None` when it did not answer in time.
/// Blocking: call it off the actor.
pub trait Latency: Send + Sync {
    fn probe(&self, ip: Ipv4Addr, timeout: Duration) -> Option<Duration>;
}

/// Never measures anything, for tests and platforms without a probe.
pub struct Unmeasured;

impl Latency for Unmeasured {
    fn probe(&self, _ip: Ipv4Addr, _timeout: Duration) -> Option<Duration> {
        None
    }
}

/// The platform's ICMP echo.
pub struct IcmpEcho;

impl Latency for IcmpEcho {
    fn probe(&self, ip: Ipv4Addr, timeout: Duration) -> Option<Duration> {
        // The first echo often pays for an ARP or route lookup; the second
        // is the one that says how far away the machine is.
        (0..2).filter_map(|_| echo(ip, timeout)).min()
    }
}

#[cfg(windows)]
fn echo(ip: Ipv4Addr, timeout: Duration) -> Option<Duration> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        ICMP_ECHO_REPLY, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho,
    };

    const PAYLOAD: [u8; 32] = *b"modlobby ping modlobby ping ping";

    // SAFETY: the handle is closed on every path out; the reply buffer is
    // sized as the API documents (one reply, the payload echoed back, and
    // room for an ICMP error) and read as the struct it was filled with.
    unsafe {
        let handle = IcmpCreateFile();
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut reply = vec![0_u8; std::mem::size_of::<ICMP_ECHO_REPLY>() + PAYLOAD.len() + 8];
        let replies = IcmpSendEcho(
            handle,
            u32::from_ne_bytes(ip.octets()),
            PAYLOAD.as_ptr().cast(),
            PAYLOAD.len() as u16,
            std::ptr::null(),
            reply.as_mut_ptr().cast(),
            reply.len() as u32,
            timeout.as_millis().try_into().unwrap_or(u32::MAX),
        );
        IcmpCloseHandle(handle);
        if replies == 0 {
            return None;
        }
        let first: ICMP_ECHO_REPLY = std::ptr::read_unaligned(reply.as_ptr().cast());
        // IP_SUCCESS; anything else is unreachable, timed out, or an error.
        (first.Status == 0).then(|| Duration::from_millis(u64::from(first.RoundTripTime)))
    }
}

#[cfg(not(windows))]
fn echo(_ip: Ipv4Addr, _timeout: Duration) -> Option<Duration> {
    None
}

/// Probes every address at once, off the async runtime's threads.
pub async fn measure(
    latency: Arc<dyn Latency>,
    ips: Vec<Ipv4Addr>,
    timeout: Duration,
) -> HashMap<Ipv4Addr, Option<Duration>> {
    let mut probes = tokio::task::JoinSet::new();
    for ip in ips {
        let latency = Arc::clone(&latency);
        probes.spawn_blocking(move || (ip, latency.probe(ip, timeout)));
    }
    let mut measured = HashMap::new();
    while let Some(result) = probes.join_next().await {
        if let Ok((ip, rtt)) = result {
            measured.insert(ip, rtt);
        }
    }
    measured
}

/// An answer is trusted for this long. The machines do not move; this is
/// about noticing a route that changed, not tracking jitter.
pub const GOOD_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// A machine that did not answer is asked again after this long, so a
/// moment's packet loss is not held against it for a week.
pub const RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

/// What the host machines answered, kept between runs.
///
/// Every request probes what has expired plus one address chosen at random,
/// so the set drifts back into currency one member per use instead of all
/// at once every time the app starts. Pure: the clock comes in as `now`, in
/// seconds since the Unix epoch.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    hosts: HashMap<Ipv4Addr, Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    /// `None` when the machine did not answer.
    rtt_ms: Option<u32>,
    /// When it was measured, in seconds since the Unix epoch.
    at: u64,
}

impl Entry {
    fn live(&self, now: u64) -> bool {
        let good_for = match self.rtt_ms {
            Some(_) => GOOD_FOR,
            None => RETRY_AFTER,
        };
        now.saturating_sub(self.at) <= good_for.as_secs()
    }
}

impl Cache {
    /// The file's contents, or an empty cache when there is no file yet or
    /// it cannot be read: a cache that will not load is a cache to rebuild.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                tracing::debug!(path = %path.display(), %err, "latency cache unreadable; starting over");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Writes the cache; a file that cannot be written only costs a probe.
    pub fn save(&self, path: &Path) {
        let text = serde_json::to_string_pretty(self).expect("a cache serialises");
        if let Err(err) = std::fs::write(path, text) {
            tracing::warn!(path = %path.display(), %err, "latency cache not written");
        }
    }

    /// Which of `ips` to probe now: those without a live answer, plus one
    /// live one picked by `roll` (a number in `0.0..1.0`) so that what is
    /// remembered is also, now and then, checked.
    pub fn due(&self, ips: &[Ipv4Addr], now: u64, roll: f64) -> Vec<Ipv4Addr> {
        let (live, mut due): (Vec<Ipv4Addr>, Vec<Ipv4Addr>) = ips
            .iter()
            .copied()
            .partition(|ip| self.hosts.get(ip).is_some_and(|entry| entry.live(now)));
        if !live.is_empty() {
            let index = ((roll.clamp(0.0, 1.0) * live.len() as f64) as usize).min(live.len() - 1);
            due.push(live[index]);
        }
        due
    }

    /// How many of `ips` need no probe right now.
    pub fn remembered(&self, ips: &[Ipv4Addr], now: u64) -> usize {
        ips.iter()
            .filter(|ip| self.hosts.get(ip).is_some_and(|entry| entry.live(now)))
            .count()
    }

    pub fn record(&mut self, measured: HashMap<Ipv4Addr, Option<Duration>>, now: u64) {
        for (ip, rtt) in measured {
            let rtt_ms = rtt.map(|rtt| rtt.as_millis().try_into().unwrap_or(u32::MAX));
            self.hosts.insert(ip, Entry { rtt_ms, at: now });
        }
    }

    /// The answers still worth trusting.
    pub fn known(&self, now: u64) -> lobby_core::Rtts {
        self.hosts
            .iter()
            .filter(|(_, entry)| entry.live(now))
            .filter_map(|(ip, entry)| Some((*ip, Duration::from_millis(u64::from(entry.rtt_ms?)))))
            .collect()
    }
}

/// Seconds since the Unix epoch, for [`Cache`].
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(10, 0, 0, last)
    }

    fn cache_with(entries: &[(Ipv4Addr, Option<u32>, u64)]) -> Cache {
        let mut cache = Cache::default();
        for (ip, rtt_ms, at) in entries {
            cache.hosts.insert(
                *ip,
                Entry {
                    rtt_ms: *rtt_ms,
                    at: *at,
                },
            );
        }
        cache
    }

    #[test]
    fn what_is_not_remembered_is_due_plus_one_that_is() {
        let now = 1_000_000;
        let cache = cache_with(&[(ip(1), Some(20), now - 60), (ip(2), Some(30), now - 60)]);
        let ips = [ip(1), ip(2), ip(3)];
        // The unknown one, and one of the two live ones by the roll.
        assert_eq!(cache.due(&ips, now, 0.0), [ip(3), ip(1)]);
        assert_eq!(cache.due(&ips, now, 0.99), [ip(3), ip(2)]);
        assert_eq!(cache.remembered(&ips, now), 2);
    }

    #[test]
    fn with_nothing_live_only_the_unknown_are_due() {
        let cache = Cache::default();
        assert_eq!(cache.due(&[ip(1)], 5, 0.5), [ip(1)]);
        assert_eq!(cache.remembered(&[ip(1)], 5), 0);
    }

    #[test]
    fn an_answer_lasts_a_week_and_a_silence_an_hour() {
        let now = 10_000_000;
        let cache = cache_with(&[
            (ip(1), Some(20), now - GOOD_FOR.as_secs()),
            (ip(2), Some(20), now - GOOD_FOR.as_secs() - 1),
            (ip(3), None, now - RETRY_AFTER.as_secs()),
            (ip(4), None, now - RETRY_AFTER.as_secs() - 1),
        ]);
        let ips = [ip(1), ip(2), ip(3), ip(4)];
        let mut due = cache.due(&ips, now, 0.0);
        due.sort();
        // 2 and 4 expired; 1 is the live one the roll lands on (3 is live
        // but, being a silence, is never an answer).
        assert_eq!(due, [ip(1), ip(2), ip(4)]);
        let known = cache.known(now);
        assert_eq!(known.len(), 1);
        assert_eq!(known[&ip(1)], Duration::from_millis(20));
    }

    #[test]
    fn recording_replaces_what_was_there() {
        let mut cache = cache_with(&[(ip(1), Some(20), 1)]);
        cache.record(
            HashMap::from([(ip(1), None), (ip(2), Some(Duration::from_millis(44)))]),
            2,
        );
        let known = cache.known(2);
        assert!(!known.contains_key(&ip(1)));
        assert_eq!(known[&ip(2)], Duration::from_millis(44));
    }

    #[test]
    fn the_file_round_trips_and_a_missing_one_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("latency.json");
        assert_eq!(Cache::load(&path), Cache::default());

        let cache = cache_with(&[(ip(1), Some(20), 7), (ip(2), None, 8)]);
        cache.save(&path);
        assert_eq!(Cache::load(&path), cache);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("\"rttMs\": 20")
        );

        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Cache::load(&path), Cache::default());
    }

    #[cfg(windows)]
    #[test]
    fn the_local_machine_answers_an_echo() {
        let rtt = IcmpEcho.probe(Ipv4Addr::LOCALHOST, Duration::from_secs(1));
        assert!(rtt.is_some());
    }

    #[tokio::test]
    async fn every_address_asked_about_gets_an_entry() {
        let ips = vec![Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2)];
        let measured = measure(Arc::new(Unmeasured), ips.clone(), Duration::from_secs(1)).await;
        assert_eq!(measured.len(), 2);
        assert!(ips.iter().all(|ip| measured[ip].is_none()));
    }
}
