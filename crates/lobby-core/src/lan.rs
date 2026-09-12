//! Finding the other people in the building.
//!
//! A LAN game needs no server: the engine hosts it, and a guest joins with
//! `spring://<name>:@<host>:<port>`. What is missing is the one thing a lobby
//! server was doing — telling everybody it exists — and on a single network
//! that is a broadcast rather than an account, a session and a protocol.
//!
//! So this is the whole of it. A host with a room open to the LAN says so
//! every [`ANNOUNCE_EVERY`] on UDP [`PORT`], and anyone listening builds the
//! list. There is no registration, no leaving message and no state to get out
//! of step: a game that stops being announced stops being listed
//! ([`GONE_AFTER`]), which is also what a host closing its laptop looks like.
//!
//! Everything here is pure — the datagrams are made and read, never sent — so
//! the format and the forgetting are tested without a network.
//!
//! # What it is not
//!
//! It is not a lobby server, and it deliberately stops well short of being
//! one. Nothing here carries chat, accounts, rankings or a room you can change
//! from the other side: the host sets the game up in its own room and the
//! announcement is a description of it. That keeps the trust model small,
//! which matters more here than anywhere else in modlobby — this is the one
//! place that reads bytes off the network from an unauthenticated stranger, so
//! what those bytes can do is held to "appear in a list, with every string
//! clamped".

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The UDP port the announcements go to.
///
/// One past the engine's own default game port ([`recoil`'s
/// `script::DEFAULT_PORT`], 8452), so the pair reads as what it is and a
/// firewall rule written for one is next to the other. It is modlobby's alone:
/// nothing in Spring or Recoil listens here.
pub const PORT: u16 = 8453;

/// How often a host says its game is there.
///
/// Short enough that a guest opening the list does not wait for it, and small
/// enough to be nothing: one datagram of a few hundred bytes every two seconds
/// is about a hundred bytes a second on a network built for megabytes.
pub const ANNOUNCE_EVERY: Duration = Duration::from_secs(2);

/// How long a game stays listed after its last announcement.
///
/// Three announcements' worth, so a single dropped datagram — which is all a
/// broadcast promises — does not make a game flicker out of the list and back.
pub const GONE_AFTER: Duration = Duration::from_secs(7);

/// The most a datagram may be.
///
/// Well inside the smallest payload IPv4 guarantees not to fragment, which is
/// what keeps an announcement a single packet on every network it might cross.
/// It is also the ceiling every field below is clamped against: a name is not
/// allowed to be the reason a game cannot be announced.
pub const MOST: usize = 512;

/// What a string from the network may be, before it is anything else.
///
/// Every text field is cut to this on both sides — written short, and read
/// short again in case the sender was not us. Long enough for a real room
/// title, short enough that a hundred of them is still a list.
pub const LONGEST: usize = 64;

/// The format, so a later one can be told from this one rather than
/// misunderstood as it. Changed only when an old build reading a new
/// announcement would get it *wrong*; a field it does not know is ignored.
pub const VERSION: u32 = 1;

/// One announcement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Beacon {
    /// A host saying its game is open.
    Game(Box<Game>),
    /// Somebody with modlobby open and no game, saying who they are.
    ///
    /// The other half of not having to type: a host adding a guest picks the
    /// name off a list rather than spelling it, and a name spelled wrong is a
    /// guest the engine's server will not let in. It carries a name and
    /// nothing else on purpose.
    Here { name: String },
}

/// A game somebody on this network is hosting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    /// The format the sender wrote. Read before anything else is believed.
    pub v: u32,
    /// New every time a room is opened to the LAN, so the same host opening a
    /// second game is a second entry rather than the first one changing under
    /// somebody who was about to join it.
    pub id: String,
    pub title: String,
    /// Whoever is hosting, as they appear in the game.
    pub host: String,
    /// Where the engine is listening. The address is not announced: it is the
    /// one the datagram came from, which is the one that can be reached.
    pub port: u16,
    pub engine: String,
    pub game: String,
    pub map: String,
    /// The names the host has written into the start script for guests.
    ///
    /// The engine's server admits a joining client only under a name the
    /// script lists (`GameServer.cpp:3017`), so this is the list of names that
    /// will be let in — which is what a guest needs to know to join at all.
    #[serde(default)]
    pub seats: Vec<String>,
    /// Whether the game has already started, which is when joining stops
    /// being possible. Listed anyway, and said so: "you are too late" is worth
    /// more than a game that vanishes as you reach for it.
    #[serde(default)]
    pub running: bool,
}

/// Cuts a string to [`LONGEST`], on a character boundary.
///
/// Written on the way out *and* on the way in. Out, so an announcement stays
/// one datagram; in, because the sender is not necessarily us and a field
/// that reaches the window is a field that has to be a reasonable size
/// whatever arrived.
pub fn clamp(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(LONGEST) {
        None => text.to_owned(),
        Some((at, _)) => text[..at].to_owned(),
    }
}

impl Game {
    /// An announcement with every field the size it is allowed to be.
    pub fn new(
        id: impl AsRef<str>,
        title: impl AsRef<str>,
        host: impl AsRef<str>,
        port: u16,
        engine: impl AsRef<str>,
        game: impl AsRef<str>,
        map: impl AsRef<str>,
    ) -> Self {
        Self {
            v: VERSION,
            id: clamp(id.as_ref()),
            title: clamp(title.as_ref()),
            host: clamp(host.as_ref()),
            port,
            engine: clamp(engine.as_ref()),
            game: clamp(game.as_ref()),
            map: clamp(map.as_ref()),
            seats: Vec::new(),
            running: false,
        }
    }

    /// The same, with everything from the network cut to size.
    ///
    /// The seats are capped as well as clamped: a sender claiming a thousand
    /// of them is a list nobody wants drawn, and a real game has at most the
    /// sixteen the engine plays.
    fn sanitised(mut self) -> Self {
        self.id = clamp(&self.id);
        self.title = clamp(&self.title);
        self.host = clamp(&self.host);
        self.engine = clamp(&self.engine);
        self.game = clamp(&self.game);
        self.map = clamp(&self.map);
        self.seats.truncate(MOST_SEATS);
        self.seats = self.seats.iter().map(|seat| clamp(seat)).collect();
        self
    }

    /// Whether this is worth listing at all. A game with no port cannot be
    /// joined and a game with no host has nothing to show for itself.
    fn worth_listing(&self) -> bool {
        self.v == VERSION && self.port != 0 && !self.host.is_empty()
    }
}

/// The most guests an announcement may claim. The engine plays sixteen; the
/// rest is somebody's mistake or somebody's idea of a joke.
const MOST_SEATS: usize = 16;

/// The datagram for a beacon, or `None` when it would not fit.
///
/// Not fitting is not an error worth stopping over — it is one announcement
/// missed, and there is another in two seconds — but it is worth not sending
/// a truncated one, which is what makes it an `Option` rather than a `Vec`.
pub fn encode(beacon: &Beacon) -> Option<Vec<u8>> {
    let bytes = serde_json::to_vec(beacon).ok()?;
    (bytes.len() <= MOST).then_some(bytes)
}

/// A beacon out of a datagram, or `None` for anything this does not
/// understand.
///
/// Everything about this function is written on the assumption that the bytes
/// are hostile: it is the one input modlobby takes from an unauthenticated
/// stranger. So it refuses an oversized datagram before parsing it, refuses a
/// version it does not know, and cuts every string to size whatever the sender
/// said. Nothing it returns can be longer than [`LONGEST`], and nothing it
/// returns is acted on without somebody clicking it.
pub fn decode(datagram: &[u8]) -> Option<Beacon> {
    if datagram.len() > MOST {
        return None;
    }
    match serde_json::from_slice::<Beacon>(datagram).ok()? {
        Beacon::Game(game) => {
            let game = game.sanitised();
            game.worth_listing().then(|| Beacon::Game(Box::new(game)))
        }
        Beacon::Here { name } => {
            let name = clamp(&name);
            (!name.is_empty()).then_some(Beacon::Here { name })
        }
    }
}

/// A game that was heard from, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The address the datagram came from — which is the address that can be
    /// reached, rather than whatever the sender believes about itself. A host
    /// behind two interfaces cannot get this wrong, because it does not say.
    pub from: IpAddr,
    pub game: Game,
}

impl Found {
    /// What the engine is handed to join it.
    ///
    /// No password: a script that gives a player none accepts whatever is
    /// sent (`GameServer.cpp:CheckPlayerPassword`), so there is nothing to
    /// agree on beforehand and joining is the one click it should be.
    pub fn join_url(&self, as_name: &str) -> String {
        recoil_url(as_name, self.from, self.game.port)
    }
}

/// `spring://<name>:@<host>:<port>`, with an IPv6 address in the brackets the
/// engine's parser expects.
fn recoil_url(name: &str, host: IpAddr, port: u16) -> String {
    match host {
        IpAddr::V4(ip) => format!("spring://{name}:@{ip}:{port}"),
        IpAddr::V6(ip) => format!("spring://{name}:@[{ip}]:{port}"),
    }
}

/// The games heard from lately, and the people.
///
/// Keyed by the announcement's own id rather than by address, so a host that
/// changes address mid-session replaces its entry instead of appearing twice,
/// and a machine hosting two games shows two.
#[derive(Debug, Default)]
pub struct Seen {
    games: BTreeMap<String, (Instant, Found)>,
    people: BTreeMap<String, Instant>,
}

impl Seen {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes in one beacon. Answers whether the list a person is looking at
    /// would now be different, so a caller can push a change rather than
    /// pushing the same list every two seconds.
    pub fn heard(&mut self, now: Instant, from: IpAddr, beacon: Beacon) -> bool {
        match beacon {
            Beacon::Game(game) => {
                let found = Found { from, game: *game };
                let changed = self
                    .games
                    .get(&found.game.id)
                    .is_none_or(|(_, held)| *held != found);
                self.games.insert(found.game.id.clone(), (now, found));
                changed
            }
            Beacon::Here { name } => self.people.insert(name, now).is_none(),
        }
    }

    /// Drops whatever has not been heard from since [`GONE_AFTER`]. Answers
    /// whether anything went.
    pub fn forget_stale(&mut self, now: Instant) -> bool {
        let fresh = |at: &Instant| now.duration_since(*at) < GONE_AFTER;
        let before = self.games.len() + self.people.len();
        self.games.retain(|_, (at, _)| fresh(at));
        self.people.retain(|_, at| fresh(at));
        before != self.games.len() + self.people.len()
    }

    /// The games still being announced, in a stable order: by title, then by
    /// id, so a list does not reshuffle itself under a pointer every time a
    /// datagram arrives.
    pub fn games(&self) -> Vec<&Found> {
        let mut found: Vec<&Found> = self.games.values().map(|(_, found)| found).collect();
        found.sort_by(|a, b| {
            a.game
                .title
                .cmp(&b.game.title)
                .then_with(|| a.game.id.cmp(&b.game.id))
        });
        found
    }

    /// One by its announcement id, for a caller about to join it.
    pub fn game(&self, id: &str) -> Option<&Found> {
        self.games.get(id).map(|(_, found)| found)
    }

    /// Who is on this network with modlobby open and no game of their own,
    /// alphabetically.
    pub fn people(&self) -> Vec<&str> {
        self.people.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> Instant {
        // A fixed origin so the arithmetic below is about the durations rather
        // than about when the test ran.
        *ORIGIN + Duration::from_secs(seconds)
    }

    static ORIGIN: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

    fn here() -> IpAddr {
        "192.168.1.20".parse().unwrap()
    }

    fn a_game() -> Game {
        let mut game = Game::new(
            "abc123",
            "Friday",
            "tetrisface",
            8452,
            "2026.07.04",
            "Beyond All Reason test-31134",
            "Supreme Isthmus v2.1",
        );
        game.seats = vec!["ann".into(), "bo".into()];
        game
    }

    #[test]
    fn an_announcement_survives_the_round_trip() {
        let sent = Beacon::Game(Box::new(a_game()));
        let bytes = encode(&sent).expect("it fits");
        assert!(bytes.len() <= MOST);
        assert_eq!(decode(&bytes), Some(sent));

        let hello = Beacon::Here {
            name: "tetrisface".into(),
        };
        assert_eq!(decode(&encode(&hello).unwrap()), Some(hello));
    }

    /// The one input modlobby takes from a stranger, so the refusals are the
    /// interesting half of this module.
    #[test]
    fn nothing_from_the_network_is_taken_on_trust() {
        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"not json"), None);
        // A datagram bigger than a beacon may be is refused before it is read.
        assert_eq!(decode(&vec![b'{'; MOST + 1]), None);
        // A format this build does not know is not guessed at.
        let mut future = a_game();
        future.v = VERSION + 1;
        assert_eq!(
            decode(&serde_json::to_vec(&Beacon::Game(Box::new(future))).unwrap()),
            None
        );
        // Nothing to join, or nobody to say it is.
        let mut portless = a_game();
        portless.port = 0;
        assert_eq!(
            decode(&serde_json::to_vec(&Beacon::Game(Box::new(portless))).unwrap()),
            None
        );
        let mut nameless = a_game();
        nameless.host = "   ".into();
        assert_eq!(
            decode(&serde_json::to_vec(&Beacon::Game(Box::new(nameless))).unwrap()),
            None
        );
        assert_eq!(
            decode(&encode(&Beacon::Here { name: " ".into() }).unwrap()),
            None
        );
    }

    /// A sender is not us, so the sizes are enforced on the way in as well.
    #[test]
    fn a_sender_cannot_decide_how_long_a_name_is() {
        let mut shouting = a_game();
        shouting.title = "t".repeat(4_000);
        shouting.seats = (0..500).map(|n| format!("guest{n}")).collect();
        // It would not fit in a datagram, so it is fed in directly: the
        // clamping has to be the decoder's doing rather than the encoder's.
        let bytes = serde_json::to_vec(&Beacon::Game(Box::new(shouting))).unwrap();
        assert!(bytes.len() > MOST, "too big to have been sent");
        assert_eq!(decode(&bytes), None, "and so not read at all");

        // Inside the size limit, an over-long field is cut rather than fatal.
        let mut long = a_game();
        long.title = "t".repeat(200);
        let read = match decode(&serde_json::to_vec(&Beacon::Game(Box::new(long))).unwrap()) {
            Some(Beacon::Game(game)) => game,
            other => panic!("a game, not {other:?}"),
        };
        assert_eq!(read.title.chars().count(), LONGEST);
    }

    #[test]
    fn a_multibyte_name_is_cut_on_a_character() {
        let cut = clamp(&"ü".repeat(200));
        assert_eq!(cut.chars().count(), LONGEST);
        assert_eq!(clamp("  spaced  "), "spaced");
    }

    /// A game that stops being announced stops being listed, which is also
    /// what a host closing their laptop looks like. There is no goodbye
    /// message, and there is deliberately nothing to get out of step.
    #[test]
    fn a_game_nobody_is_announcing_any_more_goes_away() {
        let mut seen = Seen::new();
        assert!(seen.heard(at(0), here(), Beacon::Game(Box::new(a_game()))));
        assert_eq!(seen.games().len(), 1);

        // Re-announced unchanged: still there, and nothing for a window to
        // redraw over.
        assert!(!seen.heard(at(2), here(), Beacon::Game(Box::new(a_game()))));
        assert!(!seen.forget_stale(at(6)));
        assert_eq!(seen.games().len(), 1);

        // Seven seconds after the last one, with nothing since.
        assert!(seen.forget_stale(at(10)));
        assert!(seen.games().is_empty());
        assert!(!seen.forget_stale(at(11)), "and stays gone quietly");
    }

    /// A change worth redrawing is told apart from the same game said again,
    /// because the alternative is pushing a list to the window every two
    /// seconds forever.
    #[test]
    fn only_a_real_change_is_worth_telling_anybody_about() {
        let mut seen = Seen::new();
        seen.heard(at(0), here(), Beacon::Game(Box::new(a_game())));

        let mut started = a_game();
        started.running = true;
        assert!(seen.heard(at(2), here(), Beacon::Game(Box::new(started))));
        assert!(seen.games()[0].game.running);

        // A host that moved to another address is the same game, reachable
        // somewhere else — and that is a change, because it is where a join
        // would go.
        let elsewhere: IpAddr = "192.168.1.44".parse().unwrap();
        let mut moved = a_game();
        moved.running = true;
        assert!(seen.heard(at(4), elsewhere, Beacon::Game(Box::new(moved))));
        assert_eq!(seen.games().len(), 1, "one game, not two");
        assert_eq!(seen.games()[0].from, elsewhere);
    }

    #[test]
    fn two_games_on_one_machine_are_two_entries() {
        let mut seen = Seen::new();
        let mut second = a_game();
        second.id = "def456".into();
        second.title = "Another".into();
        second.port = 8460;
        seen.heard(at(0), here(), Beacon::Game(Box::new(a_game())));
        seen.heard(at(0), here(), Beacon::Game(Box::new(second)));

        let games = seen.games();
        assert_eq!(games.len(), 2);
        // Sorted by title, so the list does not reshuffle under a pointer.
        assert_eq!(games[0].game.title, "Another");
        assert_eq!(games[1].game.title, "Friday");
        assert!(seen.game("def456").is_some());
        assert!(seen.game("nope").is_none());
    }

    /// The address a join goes to is the one the datagram came from, never one
    /// the sender chose: a host behind two interfaces cannot announce the
    /// wrong one, and nobody can announce somebody else's.
    #[test]
    fn a_join_goes_where_the_announcement_came_from() {
        let found = Found {
            from: here(),
            game: a_game(),
        };
        assert_eq!(found.join_url("me"), "spring://me:@192.168.1.20:8452");
        // And the engine's bracketed spelling for v6.
        let six = Found {
            from: "fe80::1".parse().unwrap(),
            game: a_game(),
        };
        assert_eq!(six.join_url("me"), "spring://me:@[fe80::1]:8452");
    }

    #[test]
    fn people_with_no_game_are_listed_so_a_host_need_not_type_a_name() {
        let mut seen = Seen::new();
        assert!(seen.heard(at(0), here(), Beacon::Here { name: "ann".into() }));
        // Said again is not news.
        assert!(!seen.heard(at(2), here(), Beacon::Here { name: "ann".into() }));
        seen.heard(at(2), here(), Beacon::Here { name: "bo".into() });
        assert_eq!(seen.people(), ["ann", "bo"]);

        // ann keeps saying so; bo has gone.
        seen.heard(at(8), here(), Beacon::Here { name: "ann".into() });
        assert!(seen.forget_stale(at(10)));
        assert_eq!(seen.people(), ["ann"]);
    }
}
