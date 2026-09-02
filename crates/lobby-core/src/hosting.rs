//! Which spare autohost to take when someone wants a public room of their own.
//!
//! Chobby takes the first empty room in hash order inside a region the user
//! picked by hand (`battle_list_window.lua:1707-1735`). Here the choice is
//! made from two things the client can observe on its own: how far away each
//! cluster is, and how many spare rooms it has. Every SPADS cluster keeps one
//! spare (`ClusterManager.conf` `targetSpares`), so taking a cluster's only
//! spare makes it start another instance, while taking one of several costs
//! nothing and trims the idle pool every login is sent. Nothing here does I/O:
//! the latencies come in measured, and a `roll` stands in for randomness.

use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;
use std::time::Duration;

/// Latencies within this much of the best are treated as the same. Below it
/// the difference is not something a lockstep game shows, and ranking by it
/// would pile everyone onto one machine for nothing.
pub const NEAR: Duration = Duration::from_millis(30);

/// Round-trip times by host address, as far as they were measured.
pub type Rtts = HashMap<Ipv4Addr, Duration>;

/// An autohost room nobody is in, as a candidate for taking over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpareRoom {
    pub id: u32,
    pub founder: String,
    /// The manager that runs it: `Host[EU6]` for `Host[EU6][012]`.
    pub cluster: String,
    /// The machine the game would run on; `None` when the server sent
    /// something that is not an address.
    pub ip: Option<Ipv4Addr>,
}

/// `Host[EU6]` is a cluster manager; `Host[EU6][012]` is one of its rooms.
/// Chobby's `^Host%[%a+%d+%]$` (`battle_list_window.lua:1434`).
pub fn is_manager(name: &str) -> bool {
    manager_body(name).is_some_and(|body| body.is_empty())
}

/// The manager a room's host belongs to: `Host[EU6][012]` gives `Host[EU6]`.
/// A manager itself, or any other name, gives `None`.
pub fn cluster_of(founder: &str) -> Option<&str> {
    let rest = manager_body(founder)?;
    let instance = rest.strip_prefix('[')?.strip_suffix(']')?;
    if instance.is_empty() || !instance.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(&founder[..founder.len() - rest.len()])
}

/// What follows a well-formed `Host[<letters><digits>]` prefix, if there is one.
fn manager_body(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("Host[")?;
    let close = rest.find(']')?;
    let (tag, rest) = rest.split_at(close);
    let letters = tag.chars().take_while(char::is_ascii_alphabetic).count();
    let digits = tag.len() - letters;
    if letters == 0 || digits == 0 || !tag[letters..].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(&rest[1..])
}

/// The ones worth considering: measured within [`NEAR`] of the best. With no
/// measurements at all every one is, since there is nothing to tell them apart.
/// Order is kept, so a caller's own ordering survives.
pub fn near<T>(items: Vec<(T, Option<Duration>)>) -> Vec<T> {
    let Some(best) = items.iter().filter_map(|(_, rtt)| *rtt).min() else {
        return items.into_iter().map(|(item, _)| item).collect();
    };
    items
        .into_iter()
        .filter(|(_, rtt)| rtt.is_some_and(|rtt| rtt <= best + NEAR))
        .map(|(item, _)| item)
        .collect()
}

/// The room to take, or `None` when there is nothing to take.
///
/// Clusters near enough are ranked by how many spares they have; `roll`, a
/// number in `0.0..1.0`, breaks ties and picks the room inside the cluster,
/// so two clients pressing at once do not both walk into the same one.
pub fn pick(spares: &[SpareRoom], rtts: &Rtts, roll: f64) -> Option<u32> {
    let mut clusters: BTreeMap<&str, Vec<&SpareRoom>> = BTreeMap::new();
    for room in spares {
        clusters.entry(&room.cluster).or_default().push(room);
    }
    let measured: Vec<(Vec<&SpareRoom>, Option<Duration>)> = clusters
        .into_values()
        .map(|rooms| {
            let rtt = rooms
                .iter()
                .filter_map(|room| room.ip.and_then(|ip| rtts.get(&ip)))
                .min()
                .copied();
            (rooms, rtt)
        })
        .collect();
    let mut candidates = near(measured);
    candidates.sort_by_key(|rooms| std::cmp::Reverse(rooms.len()));
    let most = candidates.first()?.len();
    let tied: Vec<&Vec<&SpareRoom>> = candidates
        .iter()
        .take_while(|rooms| rooms.len() == most)
        .collect();
    // One roll serves twice: its coarse part chooses the cluster, the
    // remainder the room, which is as good as two draws for this purpose.
    let (cluster, rest) = draw(roll, tied.len());
    let (room, _) = draw(rest, tied[cluster].len());
    Some(tied[cluster][room].id)
}

/// An index under `n` from a roll, and what is left of the roll.
fn draw(roll: f64, n: usize) -> (usize, f64) {
    let scaled = roll.clamp(0.0, 1.0) * n as f64;
    let index = (scaled as usize).min(n.saturating_sub(1));
    (index, scaled.fract())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: u32, cluster: &str, ip: &str) -> SpareRoom {
        SpareRoom {
            id,
            founder: format!("{cluster}[{id:03}]"),
            cluster: cluster.into(),
            ip: ip.parse().ok(),
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn hosts_are_told_from_the_managers_that_run_them() {
        assert!(is_manager("Host[EU1]"));
        assert!(!is_manager("Host[EU1][003]"));
        assert!(!is_manager("Host[EU]"));
        assert!(!is_manager("[teh]host"));

        assert_eq!(cluster_of("Host[EU1][003]"), Some("Host[EU1]"));
        assert_eq!(cluster_of("Host[US12][3]"), Some("Host[US12]"));
        assert_eq!(cluster_of("Host[EU1]"), None);
        assert_eq!(cluster_of("Host[EU1][abc]"), None);
        assert_eq!(cluster_of("Host[EU1][003]x"), None);
        assert_eq!(cluster_of("alice"), None);
    }

    #[test]
    fn near_keeps_what_is_within_reach_of_the_best() {
        let items = vec![
            ("far", Some(ms(120))),
            ("best", Some(ms(20))),
            ("close", Some(ms(45))),
            ("edge", Some(ms(50))),
            ("unknown", None),
        ];
        assert_eq!(near(items), ["best", "close", "edge"]);
    }

    #[test]
    fn with_nothing_measured_everything_is_near() {
        assert_eq!(near(vec![("a", None), ("b", None)]), ["a", "b"]);
    }

    #[test]
    fn the_nearest_clusters_are_ranked_by_spares() {
        let spares = [
            room(1, "Host[EU1]", "10.0.0.1"),
            room(2, "Host[EU2]", "10.0.0.2"),
            room(3, "Host[EU2]", "10.0.0.2"),
            room(4, "Host[US1]", "10.0.0.3"),
            room(5, "Host[US1]", "10.0.0.3"),
            room(6, "Host[US1]", "10.0.0.3"),
        ];
        let rtts: Rtts = [
            ("10.0.0.1".parse().unwrap(), ms(20)),
            ("10.0.0.2".parse().unwrap(), ms(35)),
            ("10.0.0.3".parse().unwrap(), ms(140)),
        ]
        .into();
        // US has the most spares but is far; EU2 beats EU1 on spares.
        for roll in [0.0, 0.5, 0.99] {
            assert!([2, 3].contains(&pick(&spares, &rtts, roll).unwrap()));
        }
    }

    #[test]
    fn the_roll_spreads_the_choice() {
        let spares = [
            room(1, "Host[EU1]", "10.0.0.1"),
            room(2, "Host[EU1]", "10.0.0.1"),
            room(3, "Host[EU2]", "10.0.0.2"),
            room(4, "Host[EU2]", "10.0.0.2"),
        ];
        let rtts = Rtts::new();
        let picked: std::collections::BTreeSet<u32> = (0..20)
            .map(|i| pick(&spares, &rtts, f64::from(i) / 20.0).unwrap())
            .collect();
        assert_eq!(picked.into_iter().collect::<Vec<_>>(), [1, 2, 3, 4]);
    }

    #[test]
    fn a_cluster_nobody_could_reach_comes_after_ones_that_answered() {
        let spares = [
            room(1, "Host[EU1]", "10.0.0.1"),
            room(2, "Host[EU1]", "10.0.0.1"),
            room(3, "Host[EU2]", "10.0.0.2"),
        ];
        let rtts: Rtts = [("10.0.0.2".parse().unwrap(), ms(30))].into();
        assert_eq!(pick(&spares, &rtts, 0.5), Some(3));
    }

    #[test]
    fn nothing_to_take_is_none() {
        assert_eq!(pick(&[], &Rtts::new(), 0.5), None);
    }
}
