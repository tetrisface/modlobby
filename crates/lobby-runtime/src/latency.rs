//! How far away a host machine is, measured by ICMP echo.
//!
//! Every room a cluster runs sits on one machine, so a handful of probes
//! covers every spare on the list. Raw ICMP sockets need privileges, which is
//! why this goes through the Windows IP Helper API instead: it works from an
//! ordinary process. Elsewhere nothing is measured and the choice of room
//! falls back to cluster headroom alone.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

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

#[cfg(test)]
mod tests {
    use super::*;

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
