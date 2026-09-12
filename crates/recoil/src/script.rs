//! Start scripts, for a game with no lobby behind it.
//!
//! The engine takes a `script.txt` where it would take a `spring://` URL, and
//! plays whatever it describes. That is how every lobby runs a skirmish, and
//! the format is documented at <https://springrts.com/wiki/Script.txt>.
//!
//! What is written here follows BAR's own two lobbies, which are the only
//! authorities on what the game actually reads:
//! `BYAR-Chobby/libs/liblobby/lobby/interface_skirmish.lua:96-370` and
//! `bar-lobby/src/main/utils/start-script-converter.ts`, whose schema is
//! `bar-lobby/src/main/model/start-script.ts`.
//!
//! Three things about it are easy to get wrong, and each has a test below:
//!
//! - A team number is not an ally-team number. Teams are numbered as
//!   participants are walked; the ally team is the side they are on.
//! - `startrect*` is a fraction of the map, 0 to 1 — not the 0-200 grid the
//!   lobby protocol and the `mapmetadata_*` modoptions use.
//! - Both start-box mechanisms are written. The engine's own rectangles are
//!   the baseline, and the modoption blob is what BAR's gadget reads; a game
//!   build without the decoder still gets sensible boxes.
//!
//! # The same script hosts a LAN game
//!
//! A skirmish is already a game the engine hosts: `ishost = 1` and a server
//! listening on loopback, with one client — this one — connected to it. A LAN
//! game is that script with the listener bound somewhere reachable and the
//! other people written in as players, which is [`Host`]. Nothing else about
//! it changes, and that is the point: the room, the modoptions, the boxes and
//! the AIs are the same ones, so a LAN game cannot drift from the skirmish it
//! was set up as.

use std::fmt::Write as _;

/// Where players start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartPos {
    /// Wherever the map's own start positions say.
    Fixed = 0,
    Random = 1,
    /// Placed by each player once the game has loaded, inside their box.
    InGame = 2,
}

/// One ally team's start box, as a fraction of the map on each axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    /// From the 0-200 grid `ADDSTARTRECT` and the start-box modoptions use.
    pub fn from_200(left: u16, top: u16, right: u16, bottom: u16) -> Self {
        let scale = |v: u16| f32::from(v) / 200.0;
        Self {
            left: scale(left),
            top: scale(top),
            right: scale(right),
            bottom: scale(bottom),
        }
    }
}

/// A side. Everyone on one is allied; `numallies` stays 0 because allying
/// whole ally teams together is not something a lobby sets up.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AllyTeam {
    pub start_rect: Option<Rect>,
}

/// One team: a slot the engine gives units to. A player or an AI holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub ally_team: u8,
    /// The *player* index in charge of it. With one human in the room that is
    /// always 0, including for the teams the AIs hold.
    pub leader: u8,
    /// The faction by name — `Armada`, `Cortex`, `Legion` — not the number the
    /// lobby protocol carries. `None` leaves the game to choose.
    pub side: Option<String>,
    /// `0xBBGGRR`, as the lobby carries a team colour.
    pub colour: Option<u32>,
    pub handicap: u8,
}

impl Team {
    pub fn new(ally_team: u8) -> Self {
        Self {
            ally_team,
            leader: 0,
            side: None,
            colour: None,
            handicap: 0,
        }
    }
}

/// A person. `team` is `None` for someone who is only watching.
///
/// Everyone the game expects is written down, whether or not they are at the
/// keyboard yet: the engine's server matches a joining client to the entry
/// with its name and rejects a name it does not find
/// (`GameServer.cpp:3017-3026`, which admits an unlisted name only as a
/// tilde-prefixed spectator and only when `AllowSpectatorJoin` is on in the
/// engine's own configuration — which modlobby does not write). So on a LAN
/// the host names the guests, and they are here from the moment the script is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub name: String,
    pub team: Option<u8>,
}

/// Where the engine's server listens, and therefore who can reach the game.
///
/// No passwords either way. The engine takes a joining client's password to be
/// right when the script gives that player none at all
/// (`GameServer.cpp:CheckPlayerPassword`), so `spring://name:@host:port` is
/// all a guest needs — which is what makes joining one click rather than a
/// secret to read out. What keeps a LAN game private is the network it is on,
/// which is the same thing that makes it a LAN game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Host {
    /// Loopback, on whichever port the operating system hands out. Nothing off
    /// this machine can reach it, which is what a game against AI wants — and
    /// what every skirmish has always done.
    #[default]
    Alone,
    /// Every interface, on `port`. Chosen deliberately and one room at a time:
    /// a lobby that quietly listened on the network would be a different
    /// program from the one somebody installed.
    Lan(u16),
}

impl Host {
    /// The address the engine binds its server to.
    ///
    /// Chobby writes `127.0.0.1` for a skirmish and the engine wants somewhere
    /// to bind even when nobody is joining
    /// (`interface_skirmish.lua:333-334`); `0.0.0.0` is the same key meaning
    /// every interface, so a guest reaches it at whichever address their side
    /// of the network knows this machine by.
    const fn ip(self) -> &'static str {
        match self {
            Self::Alone => "127.0.0.1",
            Self::Lan(_) => "0.0.0.0",
        }
    }

    /// The port, where `0` is the engine's spelling of "pick one".
    const fn port(self) -> u16 {
        match self {
            Self::Alone => 0,
            Self::Lan(port) => port,
        }
    }

    /// Whether anyone off this machine could join.
    pub const fn open(self) -> bool {
        matches!(self, Self::Lan(_))
    }
}

/// The port a LAN game listens on unless something else has it.
///
/// The engine's own default (`ClientSetup.cpp`, and what `spring://host` with
/// no port means), so a guest joining from anything but modlobby — another
/// lobby, a command line — reaches the game by typing the least.
pub const DEFAULT_PORT: u16 = 8452;

/// One AI opponent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ai {
    /// What it is called in game.
    pub name: String,
    /// The engine's name for it: `BARb`, `CircuitAI`, `NullAI` — or the name a
    /// game's `luaai.lua` declares, for Scavengers and Raptors.
    pub short_name: String,
    pub version: Option<String>,
    pub team: u8,
    /// The player index that owns it, which is whoever is hosting.
    pub host: u8,
    /// The AI's own options, written as an `[options]` block.
    pub options: Vec<(String, String)>,
}

impl Ai {
    pub fn new(name: impl Into<String>, short_name: impl Into<String>, team: u8) -> Self {
        Self {
            name: name.into(),
            short_name: short_name.into(),
            version: None,
            team,
            host: 0,
            options: Vec::new(),
        }
    }
}

/// A game to start without a server.
#[derive(Debug, Clone, PartialEq)]
pub struct Skirmish {
    /// The game's full version, as rapid names it.
    pub game: String,
    /// The map's spring name, not its filename.
    pub map: String,
    /// Which of `players` is us.
    pub player: String,
    pub start_pos: StartPos,
    /// `key = value` pairs written into `[modoptions]`.
    pub modoptions: Vec<(String, String)>,
    /// Where the engine's server listens. [`Host::Alone`] is a game nobody
    /// else can reach, which is every skirmish.
    pub host: Host,
    pub ally_teams: Vec<AllyTeam>,
    pub teams: Vec<Team>,
    pub players: Vec<Player>,
    pub ais: Vec<Ai>,
}

impl Skirmish {
    /// A game where everybody is on a side of their own: us on ally team 0,
    /// and one opponent per entry after. The arrangement a room falls back to
    /// before anybody puts two participants together.
    pub fn free_for_all(
        game: impl Into<String>,
        map: impl Into<String>,
        player: impl Into<String>,
        start_pos: StartPos,
        opponents: Vec<(String, String)>,
    ) -> Self {
        let player = player.into();
        let sides = 1 + opponents.len();
        Self {
            game: game.into(),
            map: map.into(),
            start_pos,
            modoptions: Vec::new(),
            host: Host::Alone,
            ally_teams: vec![AllyTeam::default(); sides],
            teams: (0..sides).map(|ally| Team::new(ally as u8)).collect(),
            players: vec![Player {
                name: player.clone(),
                team: Some(0),
            }],
            ais: opponents
                .into_iter()
                .enumerate()
                .map(|(index, (name, short_name))| Ai::new(name, short_name, index as u8 + 1))
                .collect(),
            player,
        }
    }

    /// The script the engine reads.
    pub fn script(&self) -> String {
        let mut out = String::from("[game] {\n");
        let _ = writeln!(out, "\tgametype = {};", self.game);
        let _ = writeln!(out, "\tmapname = {};", self.map);
        let _ = writeln!(out, "\tishost = 1;");
        let _ = writeln!(out, "\tmyplayername = {};", self.player);
        // Chobby writes both, and the engine wants somewhere to bind even when
        // nobody is joining (`interface_skirmish.lua:333-334`). A LAN game is
        // the same two keys pointed outward.
        let _ = writeln!(out, "\thostip = {};", self.host.ip());
        let _ = writeln!(out, "\thostport = {};", self.host.port());
        let _ = writeln!(out, "\tnohelperais = 0;");
        let _ = writeln!(out, "\tstartpostype = {};", self.start_pos as u8);
        let _ = writeln!(out, "\tnumplayers = {};", self.players.len());
        let _ = writeln!(out, "\tnumusers = {};", self.players.len() + self.ais.len());

        if !self.modoptions.is_empty() {
            out.push_str("\n\t[modoptions] {\n");
            for (key, value) in &self.modoptions {
                let _ = writeln!(out, "\t\t{key} = {value};");
            }
            out.push_str("\t}\n");
        }

        for (index, ally) in self.ally_teams.iter().enumerate() {
            let _ = write!(out, "\n\t[allyteam{index}] {{\n\t\tnumallies = 0;\n");
            if let Some(rect) = ally.start_rect {
                let _ = writeln!(out, "\t\tstartrectleft = {};", rect.left);
                let _ = writeln!(out, "\t\tstartrecttop = {};", rect.top);
                let _ = writeln!(out, "\t\tstartrectright = {};", rect.right);
                let _ = writeln!(out, "\t\tstartrectbottom = {};", rect.bottom);
            }
            out.push_str("\t}\n");
        }

        for (index, team) in self.teams.iter().enumerate() {
            let _ = write!(out, "\n\t[team{index}] {{\n");
            let _ = writeln!(out, "\t\tallyteam = {};", team.ally_team);
            let _ = writeln!(out, "\t\tteamleader = {};", team.leader);
            let _ = writeln!(out, "\t\thandicap = {};", team.handicap);
            if let Some(side) = &team.side {
                let _ = writeln!(out, "\t\tside = {side};");
            }
            if let Some(colour) = team.colour {
                let _ = writeln!(out, "\t\trgbcolor = {};", rgb(colour));
            }
            out.push_str("\t}\n");
        }

        for (index, player) in self.players.iter().enumerate() {
            let _ = write!(out, "\n\t[player{index}] {{\n");
            let _ = writeln!(out, "\t\tname = {};", player.name);
            let _ = writeln!(out, "\t\tisfromdemo = 0;");
            match player.team {
                Some(team) => {
                    let _ = writeln!(out, "\t\tteam = {team};");
                }
                None => {
                    let _ = writeln!(out, "\t\tspectator = 1;");
                }
            }
            out.push_str("\t}\n");
        }

        for (index, ai) in self.ais.iter().enumerate() {
            let _ = write!(out, "\n\t[ai{index}] {{\n");
            let _ = writeln!(out, "\t\tname = {};", ai.name);
            let _ = writeln!(out, "\t\tshortname = {};", ai.short_name);
            if let Some(version) = &ai.version {
                let _ = writeln!(out, "\t\tversion = {version};");
            }
            let _ = writeln!(out, "\t\tteam = {};", ai.team);
            let _ = writeln!(out, "\t\thost = {};", ai.host);
            if !ai.options.is_empty() {
                out.push_str("\n\t\t[options] {\n");
                for (key, value) in &ai.options {
                    let _ = writeln!(out, "\t\t\t{key} = {value};");
                }
                out.push_str("\t\t}\n");
            }
            out.push_str("\t}\n");
        }

        out.push_str("}\n");
        out
    }
}

/// `0xBBGGRR` as the three floats the engine reads
/// (`interface_skirmish.lua:70-82`).
fn rgb(colour: u32) -> String {
    let channel = |shift: u32| f32::from(((colour >> shift) & 0xFF) as u8) / 255.0;
    format!("{} {} {}", channel(0), channel(8), channel(16))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_opponent() -> Skirmish {
        Skirmish::free_for_all(
            "Beyond All Reason test-31134",
            "Supreme Isthmus v2.1",
            "tetrisface",
            StartPos::InGame,
            vec![("BARbarian".into(), "BARb".into())],
        )
    }

    /// A skirmish listens where nothing can reach it; a LAN game listens where
    /// the people on this network can. The pair is the assertion — a room that
    /// bound the network by default would be a different program.
    #[test]
    fn a_lan_game_is_the_same_script_bound_outward() {
        let alone = one_opponent().script();
        assert!(alone.contains("hostip = 127.0.0.1;"));
        assert!(alone.contains("hostport = 0;"), "the engine picks one");
        assert!(!Host::Alone.open());

        let mut open = one_opponent();
        open.host = Host::Lan(DEFAULT_PORT);
        open.players.push(Player {
            name: "friend".into(),
            team: Some(2),
        });
        let script = open.script();
        assert!(script.contains("hostip = 0.0.0.0;"), "every interface");
        assert!(script.contains("hostport = 8452;"));
        assert!(Host::Lan(DEFAULT_PORT).open());

        // The guest is in the script, because the engine's server rejects a
        // name it cannot find. No password, because a script that sets none
        // accepts whatever a joining client sends.
        assert!(script.contains("[player1] {\n\t\tname = friend;"));
        assert!(!script.contains("password"));
        assert!(script.contains("numplayers = 2;"));

        // And nothing else about the game moved.
        assert!(script.contains("gametype = Beyond All Reason test-31134;"));
        assert!(script.contains("shortname = BARb;"));
    }

    /// The default is the one that cannot surprise anybody.
    #[test]
    fn a_room_is_private_unless_it_was_opened() {
        assert_eq!(Host::default(), Host::Alone);
        assert!(!Host::default().open());
        assert_eq!(one_opponent().host, Host::Alone);
    }

    #[test]
    fn a_one_on_one_names_both_sides_and_who_owns_the_ai() {
        let script = one_opponent().script();
        assert!(script.contains("gametype = Beyond All Reason test-31134;"));
        assert!(script.contains("mapname = Supreme Isthmus v2.1;"));
        assert!(script.contains("myplayername = tetrisface;"));
        assert!(script.contains("startpostype = 2;"));
        assert!(script.contains("numplayers = 1;"));
        assert!(script.contains("numusers = 2;"));

        assert!(script.contains("[player0] {\n\t\tname = tetrisface;"));
        assert!(script.contains("\t\tteam = 0;"));
        assert!(script.contains("shortname = BARb;"));
        // The AI holds team 1 and we own it.
        assert!(script.contains("\t\tteam = 1;\n\t\thost = 0;"));
    }

    #[test]
    fn everyone_gets_their_own_ally_team() {
        let mut skirmish = one_opponent();
        skirmish.ally_teams.push(AllyTeam::default());
        skirmish.teams.push(Team::new(2));
        skirmish.ais.push(Ai::new("Circuit", "CircuitAI", 2));
        let script = skirmish.script();

        for ally in ["[allyteam0]", "[allyteam1]", "[allyteam2]"] {
            assert!(script.contains(ally), "missing {ally}");
        }
        assert!(!script.contains("[allyteam3]"));
        assert!(script.contains("numusers = 3;"));
    }

    #[test]
    fn modoptions_are_written_only_when_there_are_some() {
        assert!(!one_opponent().script().contains("[modoptions]"));

        let mut skirmish = one_opponent();
        skirmish.modoptions = vec![("ranked_game".into(), "0".into())];
        let script = skirmish.script();
        assert!(script.contains("[modoptions] {"));
        assert!(script.contains("ranked_game = 0;"));
    }

    #[test]
    fn an_opponent_free_game_is_still_a_valid_script() {
        let skirmish = Skirmish::free_for_all(
            "BAR test-1",
            "Comet Catcher",
            "tetrisface",
            StartPos::InGame,
            Vec::new(),
        );
        let script = skirmish.script();
        assert!(script.contains("numusers = 1;"));
        assert!(script.contains("[allyteam0]"));
        assert!(!script.contains("[ai0]"));
        assert!(script.starts_with("[game] {"));
        assert!(script.ends_with("}\n"));
    }

    /// Two people on one side and an AI on the other: three teams across two
    /// ally teams, which is the arrangement the old writer could not express.
    #[test]
    fn a_team_number_is_not_an_ally_team_number() {
        let skirmish = Skirmish {
            game: "BAR test-1".into(),
            map: "Comet Catcher".into(),
            player: "me".into(),
            start_pos: StartPos::InGame,
            modoptions: Vec::new(),
            host: Host::Alone,
            ally_teams: vec![AllyTeam::default(); 2],
            teams: vec![Team::new(0), Team::new(0), Team::new(1)],
            players: vec![
                Player {
                    name: "me".into(),
                    team: Some(0),
                },
                Player {
                    name: "friend".into(),
                    team: Some(1),
                },
            ],
            ais: vec![Ai::new("BARbarian", "BARb", 2)],
        };
        let script = skirmish.script();

        // Teams 0 and 1 share ally team 0; team 2 is the other side.
        assert!(script.contains("[team0] {\n\t\tallyteam = 0;"));
        assert!(script.contains("[team1] {\n\t\tallyteam = 0;"));
        assert!(script.contains("[team2] {\n\t\tallyteam = 1;"));
        assert!(script.contains("[allyteam1]"));
        assert!(!script.contains("[allyteam2]"));
        assert!(script.contains("numplayers = 2;"));
        assert!(script.contains("numusers = 3;"));
    }

    #[test]
    fn a_start_rect_is_a_fraction_of_the_map_not_a_two_hundredth() {
        let mut skirmish = one_opponent();
        skirmish.ally_teams[0].start_rect = Some(Rect::from_200(0, 0, 50, 200));
        let script = skirmish.script();
        assert!(script.contains("startrectleft = 0;"), "{script}");
        assert!(script.contains("startrectright = 0.25;"), "{script}");
        assert!(script.contains("startrectbottom = 1;"), "{script}");
        // The side with no box of its own is left without one rather than
        // given a wrong one.
        assert!(!script.contains("[allyteam1] {\n\t\tnumallies = 0;\n\t\tstartrect"));
    }

    #[test]
    fn a_spectator_holds_no_team() {
        let mut skirmish = one_opponent();
        skirmish.players[0].team = None;
        let script = skirmish.script();
        assert!(script.contains("spectator = 1;"));
        assert!(
            !script.contains("[player0] {\n\t\tname = tetrisface;\n\t\tisfromdemo = 0;\n\t\tteam")
        );
    }

    #[test]
    fn a_faction_is_a_name_and_a_colour_is_three_floats() {
        let mut skirmish = one_opponent();
        skirmish.teams[0].side = Some("Cortex".into());
        // 0xBBGGRR: pure red is 0x0000FF.
        skirmish.teams[0].colour = Some(0x0000FF);
        skirmish.teams[1].handicap = 20;
        let script = skirmish.script();
        assert!(script.contains("side = Cortex;"));
        assert!(script.contains("rgbcolor = 1 0 0;"), "{script}");
        assert!(script.contains("handicap = 20;"));
    }

    #[test]
    fn an_ai_carries_its_own_options_when_it_has_any() {
        let mut skirmish = one_opponent();
        assert!(!skirmish.script().contains("[options]"));
        skirmish.ais[0].version = Some("0.1".into());
        skirmish.ais[0].options = vec![("difficultyLevel".into(), "2".into())];
        let script = skirmish.script();
        assert!(script.contains("version = 0.1;"));
        assert!(script.contains("\t\t[options] {\n\t\t\tdifficultyLevel = 2;"));
    }
}
