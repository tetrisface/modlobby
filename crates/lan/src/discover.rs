//! Announcing a room and finding one, so nobody has to type an address.
//!
//! Two ways, both always on, because each fails on some network the other
//! works on: DNS-SD over mDNS (`_modlobby._tcp`, the Bonjour way, which some
//! routers filter) and a small UDP beacon to a multicast group and to each
//! subnet's broadcast address every two seconds (which some others filter).
//! A room heard either way is one room, told apart by the id it announces.
//! The beacon also goes to loopback, so two copies on one machine find each
//! other, which is how this is tried without a second computer.
//!
//! The address a room is dialled at is never in the announcement: for the
//! beacon it is where the datagram came from, for mDNS it is the published
//! address on a network this machine is also on.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// The DNS-SD service type, also listed in the app's `Info.plist`.
pub const SERVICE: &str = "_modlobby._tcp.local.";
/// What every beacon starts with; a datagram without it is not ours.
const MAGIC: &str = "modlobby-room/1 ";
/// RFC 2365 local scope, one group and port of our own beside coilbox's.
const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 8, 251);
const BEACON_PORT: u16 = 8251;
const BEACON_EVERY: Duration = Duration::from_secs(2);
/// Three beacons missed is a room that is gone.
const BEACON_STALE: Duration = Duration::from_secs(7);
/// mDNS says goodbye on its own; this is for a host that vanished without.
const MDNS_STALE: Duration = Duration::from_secs(150);
const MAX_DATAGRAM: usize = 2048;

/// What a room says about itself, on the wire both ways.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Announce {
	/// Tells one room from another, whichever way it was heard.
	pub id: u32,
	/// Where the room's lobby listens.
	pub port: u16,
	pub title: String,
	pub host: String,
	pub engine_version: String,
	pub game: String,
	pub map: String,
	pub players: u32,
	pub max_players: u32,
	pub passworded: bool,
}

impl Announce {
	fn txt(&self) -> Vec<(String, String)> {
		[
			("id", self.id.to_string()),
			("title", self.title.clone()),
			("host", self.host.clone()),
			("engine", self.engine_version.clone()),
			("game", self.game.clone()),
			("map", self.map.clone()),
			("players", self.players.to_string()),
			("maxplayers", self.max_players.to_string()),
			("passworded", u8::from(self.passworded).to_string()),
		]
		.into_iter()
		.map(|(key, value)| (key.to_owned(), value))
		.collect()
	}

	fn from_txt(txt: &HashMap<String, String>, port: u16) -> Option<Self> {
		let get = |key: &str| txt.get(key).cloned().unwrap_or_default();
		Some(Self {
			id: get("id").parse().ok()?,
			port,
			title: get("title"),
			host: get("host"),
			engine_version: get("engine"),
			game: get("game"),
			map: get("map"),
			players: get("players").parse().unwrap_or(0),
			max_players: get("maxplayers").parse().unwrap_or(0),
			passworded: get("passworded") == "1",
		})
	}
}

/// A room heard on the network, and where to reach it.
#[derive(Debug, Clone)]
pub struct Found {
	pub addr: IpAddr,
	pub info: Announce,
	beacon_seen: Option<Instant>,
	mdns_seen: Option<Instant>,
}

impl Found {
	fn alive(&self, now: Instant) -> bool {
		self.beacon_seen
			.is_some_and(|at| now.duration_since(at) < BEACON_STALE)
			|| self
				.mdns_seen
				.is_some_and(|at| now.duration_since(at) < MDNS_STALE)
	}
}

type Directory = Arc<Mutex<HashMap<u32, Found>>>;

fn record(dir: &Directory, info: Announce, addr: IpAddr, by_beacon: bool) {
	let now = Instant::now();
	let mut held = dir.lock().unwrap_or_else(|e| e.into_inner());
	let found = held.entry(info.id).or_insert_with(|| Found {
		addr,
		info: info.clone(),
		beacon_seen: None,
		mdns_seen: None,
	});
	found.info = info;
	found.addr = addr;
	if by_beacon {
		found.beacon_seen = Some(now);
	} else {
		found.mdns_seen = Some(now);
	}
}

// --- this machine's networks -------------------------------------------------

/// Every IPv4 interface that is not loopback: its address, netmask and
/// broadcast address.
fn local_nets() -> Vec<(Ipv4Addr, Ipv4Addr, Option<Ipv4Addr>)> {
	if_addrs::get_if_addrs()
		.unwrap_or_default()
		.into_iter()
		.filter_map(|interface| match interface.addr {
			if_addrs::IfAddr::V4(v4) if !v4.ip.is_loopback() => {
				Some((v4.ip, subnet_mask(v4.netmask), v4.broadcast))
			}
			_ => None,
		})
		.collect()
}

/// A netmask as a subnet. A zero one is an interface that reports none -- a
/// point-to-point tunnel, as `if_addrs` reads ProtonVPN's on Windows -- and
/// not a network every address is on, so it stands for its own address alone.
fn subnet_mask(reported: Ipv4Addr) -> Ipv4Addr {
	if reported.is_unspecified() {
		Ipv4Addr::BROADCAST
	} else {
		reported
	}
}

fn same_subnet(a: Ipv4Addr, b: Ipv4Addr, mask: Ipv4Addr) -> bool {
	u32::from(a) & u32::from(mask) == u32::from(b) & u32::from(mask)
}

/// Which of a host's published addresses to dial: one on a network this
/// machine is on, else any that is not loopback, else loopback — the two
/// copies on one machine case.
fn address_to_dial(addresses: &[Ipv4Addr]) -> Option<Ipv4Addr> {
	let nets = local_nets();
	let shared = |addr: &&Ipv4Addr| {
		nets.iter()
			.any(|(ip, mask, _)| same_subnet(**addr, *ip, *mask))
	};
	addresses
		.iter()
		.find(shared)
		.or_else(|| addresses.iter().find(|addr| !addr.is_loopback()))
		.or_else(|| addresses.first())
		.copied()
}

// --- announcing ------------------------------------------------------------

/// A room being announced, for as long as this is held. `current` is asked
/// every beacon for the room as it is now, so the player count and the map
/// follow the room without anyone telling this.
pub struct Advert {
	task: JoinHandle<()>,
}

impl Advert {
	pub fn start(current: impl Fn() -> Announce + Send + 'static) -> Self {
		let task = tokio::spawn(async move {
			let daemon = ServiceDaemon::new().ok();
			let mut registered: Option<(String, Vec<(String, String)>)> = None;
			loop {
				let info = current();
				send_beacon(&info);
				if let Some(daemon) = daemon.as_ref() {
					let txt = info.txt();
					let changed = registered.as_ref().is_none_or(|(_, held)| *held != txt);
					if changed {
						if let Some((fullname, _)) = registered.take() {
							let _ = daemon.unregister(&fullname);
						}
						if let Some(service) = service_info(&info, &txt)
							&& daemon.register(service.clone()).is_ok()
						{
							registered = Some((service.get_fullname().to_owned(), txt));
						}
					}
				}
				tokio::time::sleep(BEACON_EVERY).await;
			}
		});
		Self { task }
	}
}

impl Drop for Advert {
	fn drop(&mut self) {
		// The daemon goes with the task; its goodbye packets are sent from
		// its own thread as it shuts down.
		self.task.abort();
	}
}

fn service_info(info: &Announce, txt: &[(String, String)]) -> Option<ServiceInfo> {
	let addresses: Vec<IpAddr> = local_nets()
		.into_iter()
		.map(|(ip, ..)| IpAddr::V4(ip))
		.collect();
	if addresses.is_empty() {
		return None;
	}
	// The id keeps two rooms with one title apart; a label is 63 bytes.
	let mut instance = format!("{} ({})", info.title, info.id);
	while instance.len() > 63 {
		instance.pop();
	}
	ServiceInfo::new(
		SERVICE,
		&instance,
		&format!("modlobby-{}.local.", info.id),
		&addresses[..],
		info.port,
		txt,
	)
	.ok()
}

/// One beacon out of every interface: to the group, to the subnet's
/// broadcast address, and to loopback for a copy on this machine.
fn send_beacon(info: &Announce) {
	let Ok(json) = serde_json::to_string(info) else {
		return;
	};
	let payload = format!("{MAGIC}{json}");
	let to_group = SockAddr::from(SocketAddrV4::new(GROUP, BEACON_PORT));
	for (local, _, broadcast) in local_nets() {
		let Ok(socket) = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) else {
			continue;
		};
		let _ = socket.set_reuse_address(true);
		if socket
			.bind(&SockAddr::from(SocketAddrV4::new(local, 0)))
			.is_err()
		{
			continue;
		}
		let _ = socket.set_broadcast(true);
		let _ = socket.set_multicast_if_v4(&local);
		let _ = socket.set_multicast_ttl_v4(1);
		let _ = socket.set_multicast_loop_v4(true);
		let _ = socket.send_to(payload.as_bytes(), &to_group);
		if let Some(broadcast) = broadcast {
			let _ = socket.send_to(
				payload.as_bytes(),
				&SockAddr::from(SocketAddrV4::new(broadcast, BEACON_PORT)),
			);
		}
	}
	if let Ok(socket) = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)) {
		let _ = socket.send_to(payload.as_bytes(), (Ipv4Addr::LOCALHOST, BEACON_PORT));
	}
}

// --- finding ---------------------------------------------------------------

/// Listens for rooms, both ways, for as long as this is held.
pub struct Browser {
	dir: Directory,
	tasks: Vec<JoinHandle<()>>,
	daemon: Option<ServiceDaemon>,
}

impl Browser {
	pub fn start() -> Self {
		let dir: Directory = Arc::default();
		let mut tasks = Vec::new();
		match bind_listener() {
			Ok(socket) => tasks.push(tokio::spawn(listen(socket, Arc::clone(&dir)))),
			Err(err) => tracing::warn!(%err, "no beacon listener; rooms are found by mDNS only"),
		}
		let daemon = ServiceDaemon::new().ok();
		if let Some(events) = daemon
			.as_ref()
			.and_then(|daemon| daemon.browse(SERVICE).ok())
		{
			let dir = Arc::clone(&dir);
			tasks.push(tokio::spawn(async move {
				// Which room each service is, so a goodbye can take it out.
				let mut named: HashMap<String, u32> = HashMap::new();
				while let Ok(event) = events.recv_async().await {
					match event {
						ServiceEvent::ServiceResolved(service) => {
							let txt = service.txt_properties.clone().into_property_map_str();
							let Some(info) = Announce::from_txt(&txt, service.port) else {
								continue;
							};
							let addresses: Vec<Ipv4Addr> =
								service.get_addresses_v4().into_iter().collect();
							let Some(addr) = address_to_dial(&addresses) else {
								continue;
							};
							named.insert(service.fullname.clone(), info.id);
							record(&dir, info, IpAddr::V4(addr), false);
						}
						ServiceEvent::ServiceRemoved(_, fullname) => {
							if let Some(id) = named.remove(&fullname)
								&& let Some(found) =
									dir.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&id)
							{
								found.mdns_seen = None;
							}
						}
						_ => {}
					}
				}
			}));
		}
		Self { dir, tasks, daemon }
	}

	/// Every room heard from lately, but `except` — this machine's own —
	/// in title order.
	pub fn rooms(&self, except: Option<u32>) -> Vec<Found> {
		let now = Instant::now();
		let mut held = self.dir.lock().unwrap_or_else(|e| e.into_inner());
		held.retain(|_, found| found.alive(now));
		let mut rooms: Vec<Found> = held
			.values()
			.filter(|found| Some(found.info.id) != except)
			.cloned()
			.collect();
		rooms.sort_by(|a, b| a.info.title.cmp(&b.info.title));
		rooms
	}

	pub fn find(&self, id: u32) -> Option<Found> {
		self.rooms(None)
			.into_iter()
			.find(|found| found.info.id == id)
	}
}

impl Drop for Browser {
	fn drop(&mut self) {
		for task in &self.tasks {
			task.abort();
		}
		if let Some(daemon) = self.daemon.take() {
			let _ = daemon.shutdown();
		}
	}
}

/// The beacon socket: every interface, joined to the group on each, shared
/// with any other copy on this machine.
fn bind_listener() -> std::io::Result<UdpSocket> {
	let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
	socket.set_reuse_address(true)?;
	#[cfg(unix)]
	socket.set_reuse_port(true)?;
	socket.bind(&SockAddr::from(SocketAddrV4::new(
		Ipv4Addr::UNSPECIFIED,
		BEACON_PORT,
	)))?;
	let _ = socket.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED);
	for (local, ..) in local_nets() {
		let _ = socket.join_multicast_v4(&GROUP, &local);
	}
	socket.set_nonblocking(true)?;
	UdpSocket::from_std(socket.into())
}

async fn listen(socket: UdpSocket, dir: Directory) {
	let mut buf = vec![0u8; MAX_DATAGRAM];
	loop {
		let Ok((len, from)) = socket.recv_from(&mut buf).await else {
			continue;
		};
		let Ok(text) = std::str::from_utf8(&buf[..len]) else {
			continue;
		};
		let Some(json) = text.strip_prefix(MAGIC) else {
			continue;
		};
		let Ok(info) = serde_json::from_str::<Announce>(json) else {
			continue;
		};
		record(&dir, info, from.ip(), true);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn announce(id: u32) -> Announce {
		Announce {
			id,
			port: 8200,
			title: "Ann's game".into(),
			host: "ann".into(),
			engine_version: "2026.07.04".into(),
			game: "BAR test-1".into(),
			map: "Comet Catcher".into(),
			players: 2,
			max_players: 8,
			passworded: true,
		}
	}

	#[test]
	fn the_txt_record_round_trips() {
		let info = announce(7);
		let txt: HashMap<String, String> = info.txt().into_iter().collect();
		assert_eq!(Announce::from_txt(&txt, 8200), Some(info));
		assert!(
			Announce::from_txt(&HashMap::new(), 8200).is_none(),
			"no id, no room"
		);
	}

	#[test]
	fn an_interface_reporting_no_netmask_holds_its_own_address_only() {
		let vpn = Ipv4Addr::new(10, 2, 0, 2);
		let mask = subnet_mask(Ipv4Addr::UNSPECIFIED);
		assert!(same_subnet(vpn, vpn, mask));
		assert!(!same_subnet(Ipv4Addr::new(192, 0, 2, 2), vpn, mask));
		let lan = Ipv4Addr::new(255, 255, 255, 0);
		assert_eq!(subnet_mask(lan), lan);
	}

	#[test]
	fn a_shared_subnet_wins_over_a_stray_address_and_loopback_is_last() {
		let nets = local_nets();
		let mine = nets.first().map(|(ip, ..)| *ip);
		// Not a fixed address: 172.17.0.0/16 is Docker's default bridge, so on
		// any machine running Docker -- every CI runner -- the stray was on a
		// local subnet and won the first `find` instead of losing it. RFC 5737's
		// documentation ranges are never assigned to an interface, and the check
		// covers the tunnel whose netmask is broad enough to swallow one anyway.
		let stray = [
			Ipv4Addr::new(192, 0, 2, 2),
			Ipv4Addr::new(198, 51, 100, 2),
			Ipv4Addr::new(203, 0, 113, 2),
		]
		.into_iter()
		.find(|addr| {
			!nets
				.iter()
				.any(|(ip, mask, _)| same_subnet(*addr, *ip, *mask))
		})
		.expect("a documentation address off every local subnet");
		if let Some(mine) = mine {
			assert_eq!(
				address_to_dial(&[Ipv4Addr::LOCALHOST, stray, mine]),
				Some(mine)
			);
		}
		assert_eq!(address_to_dial(&[Ipv4Addr::LOCALHOST, stray]), Some(stray));
		assert_eq!(
			address_to_dial(&[Ipv4Addr::LOCALHOST]),
			Some(Ipv4Addr::LOCALHOST)
		);
		assert_eq!(address_to_dial(&[]), None);
	}

	/// Two copies on one machine hear each other through the beacon alone.
	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn a_room_announced_here_is_found_here() {
		let browser = Browser::start();
		let id = 0xC0FFEE;
		let _advert = Advert::start(move || announce(id));
		let mut found = None;
		for _ in 0..40 {
			tokio::time::sleep(Duration::from_millis(100)).await;
			found = browser.find(id);
			if found.is_some() {
				break;
			}
		}
		let found = found.expect("the beacon reached the listener");
		assert_eq!(found.info, announce(id));
		assert_eq!(
			browser.rooms(Some(id)).len(),
			0,
			"our own room is not offered back"
		);
	}
}
