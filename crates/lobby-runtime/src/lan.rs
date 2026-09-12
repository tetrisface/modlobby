//! The socket under [`lobby_core::lan`].
//!
//! One UDP socket, bound to [`lan::PORT`] on every interface, doing two things
//! that are really one: broadcasting whatever this machine is hosting, and
//! collecting what everyone else is. It runs from startup to shutdown whether
//! or not anything is being hosted, because *listening* is what makes a LAN
//! game appear on the other machines without anybody configuring anything —
//! and a listener that costs one socket and a datagram every two seconds is
//! cheaper than a setting explaining when to turn it on.
//!
//! It is deliberately one task away from everything else. The client actor
//! hands it what to announce and takes back what was heard, over channels, so
//! the reducer never waits on a socket and a network that misbehaves cannot
//! stall a room.
//!
//! # Why a broadcast rather than multicast
//!
//! Multicast is the tidier answer and the one that does not work: it needs a
//! group joined per interface, it is dropped by most consumer access points
//! between wireless clients, and on macOS it is one of the things the local
//! network permission prompt gates hardest. A directed broadcast to
//! `255.255.255.255` reaches the same machines, needs no group, and is what
//! every LAN game since the nineties has used.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Instant;

use lobby_core::lan::{self, Beacon, Found, Game, Seen};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// What this machine wants said on the network, as the client sees it.
///
/// Replaced whole every time it changes, so there is no way for the announcer
/// to hold a stale half of it. `None` is "say nothing about a game", which is
/// the state of every machine that is not hosting one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Announcing {
    /// The game to announce, when there is one.
    pub game: Option<Game>,
    /// The name to answer with when there is no game: how a host on another
    /// machine gets to pick this person out of a list rather than spell their
    /// name.
    ///
    /// Offered rather than sent. It only goes out while somebody else on this
    /// network is announcing a game -- see [`datagram`] -- because a name is a
    /// person's, and a lobby that broadcast it around the building from the
    /// moment it was opened would be doing something nobody asked for. On a
    /// network with no game on it, this machine says nothing at all.
    pub me: Option<String>,
}

/// What the listener heard, as the client wants it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Heard {
    pub games: Vec<Found>,
    pub people: Vec<String>,
    /// Whether there is a socket at all. False where the port was taken or the
    /// network refused it — worth carrying, because an empty list otherwise
    /// means two different things and only one of them is "nobody is playing".
    pub listening: bool,
}

/// The handle the client keeps.
pub struct Lan {
    /// What to announce. A watch rather than a queue: only the latest matters,
    /// and a sender that outruns the two-second tick should not pile up.
    say: tokio::sync::watch::Sender<Announcing>,
}

impl Lan {
    /// Says what to announce from now on.
    ///
    /// Cheap and idempotent: announcing the same thing again replaces it with
    /// itself and changes nothing on the wire.
    pub fn announce(&self, announcing: Announcing) {
        // A closed receiver means the task is gone, which is not worth an
        // error here: the network is the one part of this program that is
        // allowed to simply not be there.
        let _ = self.say.send(announcing);
    }
}

/// Starts the listener and the announcer, and answers with the handle and the
/// channel changes arrive on.
///
/// Never fails. A machine where the socket cannot be bound — the port taken by
/// another lobby, a network stack that refuses broadcast, a sandbox that
/// declines — gets a handle that does nothing and one `Heard` saying it is not
/// listening. That is a fact to draw, not a reason for the lobby to fail to
/// start.
pub fn start() -> (Lan, mpsc::Receiver<Heard>) {
    let (say, watch) = tokio::sync::watch::channel(Announcing::default());
    let (heard, changes) = mpsc::channel(8);
    tokio::spawn(run(watch, heard));
    (Lan { say }, changes)
}

/// Binds the socket, or says why not.
///
/// Both reuse flags before the bind, because two modlobbies on one machine is
/// the ordinary case while somebody is testing a LAN game and the second one
/// silently not listening is a confusing way to find that out. `SO_REUSEADDR`
/// alone lets the second bind succeed; on the BSDs -- macOS included --
/// `SO_REUSEPORT` is what makes a broadcast reach *both* of them rather than
/// whichever bound first. Windows has no such split and takes the one flag.
///
/// Broadcast is asked for explicitly: without it the send fails with a
/// permission error rather than going nowhere quietly.
fn bind() -> std::io::Result<UdpSocket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, lan::PORT)).into())?;
    UdpSocket::from_std(socket.into())
}

/// Where an announcement goes.
///
/// The all-networks broadcast rather than a per-interface directed one: a
/// machine with a wired and a wireless interface would otherwise need the
/// broadcast address of each, which means enumerating interfaces on three
/// operating systems to solve a problem the kernel already solves.
const EVERYONE: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::BROADCAST, lan::PORT);

async fn run(mut say: tokio::sync::watch::Receiver<Announcing>, heard: mpsc::Sender<Heard>) {
    let socket = match bind() {
        Ok(socket) => socket,
        Err(err) => {
            tracing::info!(%err, port = lan::PORT, "no LAN discovery on this machine");
            let _ = heard.send(Heard::default()).await;
            return;
        }
    };
    tracing::info!(port = lan::PORT, "listening for games on this network");
    let _ = heard
        .send(Heard {
            listening: true,
            ..Heard::default()
        })
        .await;

    let mut seen = Seen::new();
    let mut tick = tokio::time::interval(lan::ANNOUNCE_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // A datagram is capped at `lan::MOST`; anything longer is not ours and is
    // refused by the decoder anyway, so the buffer is that plus enough to see
    // that it was too long.
    let mut buffer = vec![0_u8; lan::MOST + 1];
    // Our own broadcast comes straight back to us. The ids we are announcing
    // are what tells our game from somebody else's, since two machines can
    // share an address as far as a datagram is concerned (a host on loopback,
    // a second instance in development).
    let mut ours: Option<String> = None;

    loop {
        tokio::select! {
            // Everything heard, whoever sent it.
            received = socket.recv_from(&mut buffer) => {
                let Ok((len, from)) = received else {
                    // A receive error on a datagram socket is about one
                    // datagram (an ICMP unreachable from an earlier send, on
                    // Windows). The socket is still good.
                    continue;
                };
                let Some(beacon) = lan::decode(&buffer[..len]) else {
                    continue;
                };
                // Our own words coming back are not news from the network.
                if let Beacon::Game(game) = &beacon
                    && ours.as_deref() == Some(game.id.as_str())
                {
                    continue;
                }
                if seen.heard(Instant::now(), from.ip(), beacon)
                    && heard.send(collect(&seen)).await.is_err()
                {
                    return;
                }
            }
            // Ours said again, and anything gone forgotten.
            _ = tick.tick() => {
                let announcing = say.borrow().clone();
                ours = announcing.game.as_ref().map(|game| game.id.clone());
                if let Some(bytes) = datagram(&announcing, !seen.games().is_empty())
                    && let Err(err) = socket.send_to(&bytes, EVERYONE).await
                {
                    // Every network change looks like this once: a laptop
                    // between access points, a VPN coming up. The next tick is
                    // two seconds away and there is nothing to do about it.
                    tracing::debug!(%err, "the announcement did not go out");
                }
                if seen.forget_stale(Instant::now())
                    && heard.send(collect(&seen)).await.is_err()
                {
                    return;
                }
            }
            // The client is gone.
            changed = say.changed() => {
                if changed.is_err() {
                    return;
                }
                // Said at once rather than at the next tick, so opening a room
                // to the network puts it on the other machines now.
                let announcing = say.borrow().clone();
                ours = announcing.game.as_ref().map(|game| game.id.clone());
                if let Some(bytes) = datagram(&announcing, !seen.games().is_empty()) {
                    let _ = socket.send_to(&bytes, EVERYONE).await;
                }
            }
        }
    }
}

/// What to put on the wire for what this machine is doing: its game, or its
/// name, or -- most of the time -- nothing.
///
/// `anyone_hosting` is what keeps a name off the network until there is a
/// reason for it to be there. Hosting is a deliberate act and announces
/// itself; *being* somewhere is not, and a lobby that broadcast whoever was
/// logged in around the building for as long as it was open would be doing
/// something nobody asked for. So the name goes out only while somebody else
/// here is announcing a game — the one moment it is any use, since its whole
/// purpose is to let that host add this person without spelling their name.
/// A network with no game on it hears nothing from this machine at all.
fn datagram(announcing: &Announcing, anyone_hosting: bool) -> Option<Vec<u8>> {
    let beacon = match (&announcing.game, &announcing.me) {
        (Some(game), _) => Beacon::Game(Box::new(game.clone())),
        (None, Some(me)) if anyone_hosting && !me.trim().is_empty() => Beacon::Here {
            name: lan::clamp(me),
        },
        _ => return None,
    };
    lan::encode(&beacon)
}

/// The registry as the client wants it.
fn collect(seen: &Seen) -> Heard {
    Heard {
        games: seen.games().into_iter().cloned().collect(),
        people: seen.people().into_iter().map(str::to_owned).collect(),
        listening: true,
    }
}

/// Where the games heard from are kept between changes, so a join can name one
/// by its announcement id rather than carry an address through the window.
///
/// A plain map on the client actor's side of the channel: the registry itself
/// lives in the task, and this is the client's copy of the last thing it was
/// told. Keeping it means a join is a lookup rather than a round trip to a
/// socket that may be mid-tick.
#[derive(Debug, Default)]
pub struct Known {
    games: HashMap<String, Found>,
    people: Vec<String>,
    listening: bool,
}

impl Known {
    /// Takes in what the task heard.
    pub fn update(&mut self, heard: Heard) {
        self.games = heard
            .games
            .into_iter()
            .map(|found| (found.game.id.clone(), found))
            .collect();
        self.people = heard.people;
        self.listening = heard.listening;
    }

    /// One by its announcement id, for a caller about to join it.
    pub fn game(&self, id: &str) -> Option<&Found> {
        self.games.get(id)
    }

    pub fn listening(&self) -> bool {
        self.listening
    }

    pub fn people(&self) -> &[String] {
        &self.people
    }

    /// The games, in the order the list should draw them: by title, then by
    /// id, so nothing reshuffles under a pointer.
    pub fn games(&self) -> Vec<&Found> {
        let mut games: Vec<&Found> = self.games.values().collect();
        games.sort_by(|a, b| {
            a.game
                .title
                .cmp(&b.game.title)
                .then_with(|| a.game.id.cmp(&b.game.id))
        });
        games
    }
}

/// An id for one opening of a room to the network.
///
/// New every time, so a host that closes a game and opens another is a second
/// entry rather than the first one changing under somebody who was about to
/// join it. Random rather than counted: two machines counting from one would
/// collide on their first game, which is the one time this matters.
pub fn fresh_id() -> String {
    use rand::RngExt as _;
    let bits: u64 = rand::rng().random();
    format!("{bits:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_game() -> Game {
        Game::new(
            "abc123",
            "Friday",
            "tetrisface",
            8452,
            "2026.07.04",
            "BAR test-1",
            "Comet Catcher",
        )
    }

    /// A name is a person's, and it goes on the network only while there is a
    /// game here for it to be any use to.
    ///
    /// The pair is the assertion. On a quiet network this machine is silent —
    /// nobody asked for their name to be broadcast around the building — and
    /// the moment somebody opens a room, it offers itself so that host can add
    /// them without spelling it.
    #[test]
    fn a_name_goes_out_only_while_somebody_here_is_hosting() {
        let idle = Announcing {
            game: None,
            me: Some("tetrisface".into()),
        };
        assert!(
            datagram(&idle, false).is_none(),
            "a quiet network hears nothing from us"
        );

        let bytes = datagram(&idle, true).expect("a hello");
        assert_eq!(
            lan::decode(&bytes),
            Some(Beacon::Here {
                name: "tetrisface".into()
            })
        );
    }

    /// A machine with nothing to say says nothing. There is no keepalive and
    /// no empty announcement: the network stays quiet until somebody is here.
    #[test]
    fn a_machine_with_nothing_to_say_sends_nothing() {
        assert!(datagram(&Announcing::default(), true).is_none());
        assert!(
            datagram(
                &Announcing {
                    game: None,
                    me: Some("   ".into()),
                },
                true
            )
            .is_none()
        );
    }

    /// The game wins over the name: a host is not also advertising itself as
    /// somebody looking for a game. And a host says so whether or not anybody
    /// else is -- announcing a game is the deliberate act that starts all this.
    #[test]
    fn a_host_announces_its_game_rather_than_itself() {
        let hosting = Announcing {
            game: Some(a_game()),
            me: Some("tetrisface".into()),
        };
        for quiet in [false, true] {
            let bytes = datagram(&hosting, quiet).expect("an announcement");
            assert!(matches!(lan::decode(&bytes), Some(Beacon::Game(_))));
        }
    }

    #[test]
    fn every_opening_of_a_room_gets_an_id_of_its_own() {
        let ids: std::collections::HashSet<String> = (0..64).map(|_| fresh_id()).collect();
        assert_eq!(
            ids.len(),
            64,
            "an id that repeats is a game joined by mistake"
        );
        assert!(ids.iter().all(|id| id.len() == 16));
    }

    #[test]
    fn what_was_heard_is_kept_so_a_join_is_a_lookup() {
        let mut known = Known::default();
        assert!(!known.listening());
        assert!(known.games().is_empty());

        let mut second = a_game();
        second.id = "def456".into();
        second.title = "Another".into();
        known.update(Heard {
            games: vec![
                Found {
                    from: "192.168.1.20".parse().unwrap(),
                    game: a_game(),
                },
                Found {
                    from: "192.168.1.21".parse().unwrap(),
                    game: second,
                },
            ],
            people: vec!["ann".into()],
            listening: true,
        });

        assert!(known.listening());
        assert_eq!(known.people(), ["ann"]);
        // Sorted by title, so the list is stable under a pointer.
        assert_eq!(
            known
                .games()
                .iter()
                .map(|f| f.game.title.as_str())
                .collect::<Vec<_>>(),
            ["Another", "Friday"]
        );
        assert_eq!(
            known.game("abc123").map(|found| found.join_url("me")),
            Some("spring://me:@192.168.1.20:8452".to_owned())
        );
        assert!(known.game("gone").is_none());

        // A later report replaces what was there rather than adding to it: a
        // game that stopped being announced has to leave the list.
        known.update(Heard {
            games: Vec::new(),
            people: Vec::new(),
            listening: true,
        });
        assert!(known.games().is_empty());
        assert!(known.game("abc123").is_none());
    }

    /// The one thing none of the above can answer: does a datagram actually
    /// leave this machine and come back off the network?
    ///
    /// Everything else here is the format and the bookkeeping, which are
    /// tested without a socket on purpose. Broadcast is the part that is easy
    /// to get wrong and impossible to get wrong *quietly* -- a missing
    /// `SO_BROADCAST` is a permission error at send time, a missing
    /// `SO_REUSEPORT` is a second instance that hears nothing -- so it is
    /// asked for real, of a real network stack, behind `--ignored` because a
    /// build machine's may refuse it:
    ///
    /// ```text
    /// cargo test -p lobby-runtime -- --ignored lan
    /// ```
    #[tokio::test]
    #[ignore = "sends a real broadcast; needs a network stack that allows one"]
    async fn an_announcement_goes_out_and_comes_back_off_the_network() {
        // A second socket on the port, exactly as a second modlobby would be.
        let listener = bind().expect("a second socket on the same port");
        let (lan, _changes) = start();
        lan.announce(Announcing {
            game: Some(a_game()),
            me: None,
        });

        let mut buffer = vec![0_u8; lan::MOST + 1];
        let heard = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let (len, from) = listener.recv_from(&mut buffer).await.expect("a datagram");
                if let Some(Beacon::Game(game)) = lan::decode(&buffer[..len]) {
                    return (*game, from);
                }
            }
        })
        .await
        .expect("the announcement arrives within five seconds");

        assert_eq!(heard.0.id, "abc123");
        assert_eq!(heard.0.title, "Friday");
        assert_eq!(heard.0.port, 8452);
        // And the address a join would go to is the one it arrived from.
        let found = Found {
            from: heard.1.ip(),
            game: heard.0,
        };
        assert!(found.join_url("me").starts_with("spring://me:@"));
        println!("heard our own announcement from {}", heard.1);
    }

    /// Two machines, as they would be: one hosting, one listening.
    ///
    /// The one thing worth asserting end to end, because it is the only claim
    /// the parts do not make individually — a host's announcement reaches
    /// somebody else's list, under the address a join would go to, and the
    /// host does not list its own game back to itself. Two tasks on one
    /// machine is what `SO_REUSEPORT` in [`bind`] is for, and is also how
    /// anybody would try this out.
    ///
    /// ```text
    /// cargo test -p lobby-runtime -- --ignored lan
    /// ```
    #[tokio::test]
    #[ignore = "sends a real broadcast; needs a network stack that allows one"]
    async fn a_host_appears_in_somebody_elses_list_and_not_in_its_own() {
        let (host, mut host_hears) = start();
        let (_guest, mut guest_hears) = start();
        // Both say whether they are listening before anything else; a machine
        // that could not bind cannot answer the question this test asks.
        assert!(host_hears.recv().await.expect("an answer").listening);
        assert!(guest_hears.recv().await.expect("an answer").listening);

        host.announce(Announcing {
            game: Some(a_game()),
            me: None,
        });

        let heard = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                let heard = guest_hears.recv().await.expect("the guest is listening");
                if let Some(found) = heard.games.first() {
                    return found.clone();
                }
            }
        })
        .await
        .expect("the host's game reaches the other machine");

        assert_eq!(heard.game.id, "abc123");
        assert_eq!(heard.game.title, "Friday");
        assert_eq!(heard.game.port, 8452);
        assert!(!heard.game.running);
        // And the address to join at is the one it arrived from, which is the
        // one that can be reached -- never one the sender chose.
        assert_eq!(
            heard.join_url("me"),
            format!("spring://me:@{}:8452", heard.from)
        );
        assert!(
            recoil::refuse_target(&heard.join_url("me"), false).is_none(),
            "a game on this network is playable even where hosted games are not"
        );

        // The host's own words coming back are not news from the network.
        let mine = tokio::time::timeout(lan::ANNOUNCE_EVERY * 3, async {
            while let Some(heard) = host_hears.recv().await {
                if !heard.games.is_empty() {
                    return true;
                }
            }
            false
        })
        .await;
        assert!(
            matches!(mine, Err(_) | Ok(false)),
            "a host does not list its own game"
        );
        println!("the guest found the host at {}", heard.from);
    }

    /// A machine where the socket cannot be bound is a machine that says so,
    /// rather than one where the lobby failed to start.
    #[tokio::test]
    async fn no_socket_is_a_fact_rather_than_a_failure() {
        let (lan, mut changes) = start();
        // Whether it bound or not, the first thing said is which.
        let first = changes.recv().await.expect("an answer either way");
        assert!(first.games.is_empty());
        // And announcing into it is safe regardless.
        lan.announce(Announcing {
            game: Some(a_game()),
            me: None,
        });
    }
}
