//! Typed server → client messages for the login flood and battle-list flow.
//!
//! Wire formats are taken from teiserver's `spring_out.ex` `do_reply/2`
//! clauses; anything not modelled yet surfaces as [`ServerEvent::Unknown`].

use std::collections::BTreeMap;

use base64::prelude::*;
use serde::Deserialize;

use crate::battle::BattleStatus;
use crate::codec::RawMessage;

/// `CLIENTSTATUS` bit layout (teiserver `Spring.create_client_status/1`):
/// bit 0 in-game, bit 1 away, bits 2-4 rank (bit 2 least significant),
/// bit 5 moderator, bit 6 bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserStatus {
    pub in_game: bool,
    pub away: bool,
    pub rank: u8,
    pub moderator: bool,
    pub bot: bool,
}

impl UserStatus {
    pub fn from_bits(bits: u32) -> Self {
        Self {
            in_game: bits & 1 != 0,
            away: bits & 2 != 0,
            rank: ((bits >> 2) & 0b111) as u8,
            moderator: bits & (1 << 5) != 0,
            bot: bits & (1 << 6) != 0,
        }
    }
}

/// `BATTLEOPENED id type nat founder ip port maxPlayers passworded rank mapHash engine\tversion\tmap\ttitle\tgame`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleOpened {
    pub id: u32,
    pub replay: bool,
    pub nat_type: u8,
    pub founder: String,
    pub ip: String,
    pub port: u16,
    pub max_players: u32,
    pub passworded: bool,
    pub rank: u32,
    pub map_hash: String,
    pub engine_name: String,
    pub engine_version: String,
    pub map_name: String,
    pub title: String,
    pub game_name: String,
}

/// Team layout teiserver attaches to a battle (`s.battle.teams`), e.g. 2 x 8 for an 8v8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct TeamLayout {
    #[serde(rename = "nbTeams")]
    pub teams: u32,
    #[serde(rename = "teamSize")]
    pub team_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEvent {
    /// `TASSERVER <server version> <spring version> <udp port> <server mode>`, the greeting.
    Welcome {
        server_version: String,
        spring_version: String,
        udp_port: u16,
        server_mode: u8,
    },
    Accepted {
        username: String,
    },
    Denied {
        reason: String,
    },
    /// Login is throttled server-side; the server retries on its own.
    Queued,
    Agreement {
        line: String,
    },
    AgreementEnd,
    /// The account was created; it still has to log in and confirm.
    RegistrationAccepted,
    RegistrationDenied {
        reason: String,
    },
    Motd {
        line: String,
    },
    ServerMsg {
        text: String,
    },
    CompFlags {
        flags: Vec<String>,
    },
    Redirect {
        host: String,
        port: Option<u16>,
    },
    Pong,
    /// End of the post-login flood.
    LoginInfoEnd,
    /// `ADDUSER name country userid lobbyClient` (teiserver puts the client name in the 4th field).
    AddUser {
        name: String,
        country: String,
        user_id: Option<u64>,
        lobby_client: String,
    },
    RemoveUser {
        name: String,
    },
    ClientStatus {
        name: String,
        status: UserStatus,
    },
    BattleOpened(Box<BattleOpened>),
    BattleClosed {
        id: u32,
    },
    UpdateBattleInfo {
        id: u32,
        spectator_count: u32,
        locked: bool,
        map_hash: String,
        map_name: String,
    },
    JoinedBattle {
        id: u32,
        name: String,
        script_password: Option<String>,
    },
    LeftBattle {
        id: u32,
        name: String,
    },
    ClientBattleStatus {
        name: String,
        status: BattleStatus,
        team_colour: u32,
    },
    JoinBattle {
        id: u32,
        game_hash: String,
    },
    JoinBattleFailed {
        reason: String,
    },
    RequestBattleStatus,
    SaidBattle {
        name: String,
        text: String,
    },
    SaidBattleEx {
        name: String,
        text: String,
    },
    /// `SAIDPRIVATE <name> <text>`: direct messages, including the Coordinator's refusals.
    SaidPrivate {
        name: String,
        text: String,
    },
    /// `SAYPRIVATE <name> <text>`: the server echoing a message we sent, which
    /// is the only confirmation it went anywhere (`spring_out.ex:458`).
    SentPrivate {
        name: String,
        text: String,
    },
    /// `JOIN <room>`: the join was accepted.
    JoinedChannel {
        room: String,
    },
    /// `JOINFAILED <room>\t<reason>`.
    JoinChannelFailed {
        room: String,
        reason: String,
    },
    /// `CLIENTS <room> <name> <name> …`: who is in a channel, sent on join and
    /// then in batches, so a later line adds to the roster rather than replacing it.
    Clients {
        room: String,
        names: Vec<String>,
    },
    /// `JOINED <room> <name>`.
    JoinedRoom {
        room: String,
        name: String,
    },
    /// `LEFT <room> <name>`.
    LeftRoom {
        room: String,
        name: String,
    },
    /// `SAID <room> <name> <text>`.
    Said {
        room: String,
        name: String,
        text: String,
    },
    /// `SAIDEX <room> <name> <text>`: an emote, rendered as an action.
    SaidEx {
        room: String,
        name: String,
        text: String,
    },
    /// `CHANNELTOPIC <room> <author>`. teiserver sends no topic text with it
    /// (`spring_out.ex:438`), so this is only ever "who set it".
    ChannelTopic {
        room: String,
        author: String,
    },
    /// `CHANNEL <name> <members>`: one line of the channel listing.
    ChannelListed {
        name: String,
        members: u32,
    },
    /// `ENDOFCHANNELS`.
    EndOfChannels,
    /// `FRIENDLISTBEGIN`: the friend list that follows replaces the last one.
    FriendListBegin,
    /// `FRIENDLIST userName=<name>`.
    Friend {
        name: String,
    },
    /// `FRIENDLISTEND`.
    FriendListEnd,
    /// `FRIENDREQUESTLISTBEGIN`.
    FriendRequestListBegin,
    /// `FRIENDREQUESTLIST userName=<name>`: someone wants to be friends.
    FriendRequest {
        name: String,
    },
    /// `FRIENDREQUESTLISTEND`.
    FriendRequestListEnd,
    /// `IGNORELISTBEGIN`: the list that follows replaces the last one.
    IgnoreListBegin,
    /// `IGNORELIST userName=<name>`.
    Ignored {
        name: String,
    },
    /// `IGNORELISTEND`.
    IgnoreListEnd,
    /// `RING <name>`: someone is summoning us to a room.
    Ring {
        name: String,
    },
    /// `FORCEQUITBATTLE`: we have been removed from the room we were in. The
    /// server has already forgotten us (`spring_tcp_server.ex:1046`), so this
    /// is a statement rather than a request.
    ForceQuitBattle,
    /// `SETSCRIPTTAGS k=v\tk=v`: the room's script tags (`game/modoptions/*` among them),
    /// one full line on join and then per change. teiserver lowercases the keys.
    SetScriptTags {
        tags: Vec<(String, String)>,
    },
    /// `REMOVESCRIPTTAGS k k`.
    RemoveScriptTags {
        keys: Vec<String>,
    },
    /// `ADDBOT <battle> <name> <owner> <status> <colour> <ai>`.
    AddBot {
        id: u32,
        name: String,
        owner: String,
        status: BattleStatus,
        team_colour: u32,
        ai: String,
    },
    UpdateBot {
        id: u32,
        name: String,
        status: BattleStatus,
        team_colour: u32,
    },
    RemoveBot {
        id: u32,
        name: String,
    },
    /// `ADDSTARTRECT <ally team> <left> <top> <right> <bottom>`, map fractions out of 200.
    AddStartRect {
        ally_team: u8,
        left: u16,
        top: u16,
        right: u16,
        bottom: u16,
    },
    RemoveStartRect {
        ally_team: u8,
    },
    /// `s.battle.update_lobby_title <id>\t<title>`.
    BattleTitle {
        id: u32,
        title: String,
    },
    /// `s.battle.teams <base64 json>` with `{"<id>": {"nbTeams", "teamSize"}}` for one or more battles.
    BattleTeams {
        layouts: Vec<(u32, TeamLayout)>,
    },
    /// `s.system.disconnect <reason>`; reason `Flood protection` also blocks re-login for ~10 s.
    Disconnect {
        reason: String,
    },
    /// `s.system.shutdown`.
    Shutdown,
    Unknown(RawMessage),
    Malformed(RawMessage),
}

impl From<RawMessage> for ServerEvent {
    fn from(raw: RawMessage) -> Self {
        parse(&raw).unwrap_or(ServerEvent::Malformed(raw))
    }
}

fn parse(raw: &RawMessage) -> Option<ServerEvent> {
    let a = raw.args.as_str();
    let event = match raw.command.as_str() {
        "TASSERVER" => {
            let f = fields(a, 4)?;
            ServerEvent::Welcome {
                server_version: f[0].into(),
                spring_version: f[1].into(),
                udp_port: f[2].parse().ok()?,
                server_mode: f[3].parse().ok()?,
            }
        }
        "ACCEPTED" => ServerEvent::Accepted { username: a.into() },
        "DENIED" => ServerEvent::Denied { reason: a.into() },
        "QUEUED" => ServerEvent::Queued,
        "AGREEMENT" => ServerEvent::Agreement { line: a.into() },
        "AGREEMENTEND" => ServerEvent::AgreementEnd,
        "REGISTRATIONACCEPTED" => ServerEvent::RegistrationAccepted,
        "REGISTRATIONDENIED" => ServerEvent::RegistrationDenied { reason: a.into() },
        "MOTD" => ServerEvent::Motd { line: a.into() },
        "SERVERMSG" => ServerEvent::ServerMsg { text: a.into() },
        "COMPFLAGS" => ServerEvent::CompFlags {
            flags: a.split_whitespace().map(str::to_owned).collect(),
        },
        "REDIRECT" => {
            let mut it = a.split_whitespace();
            ServerEvent::Redirect {
                host: it.next()?.into(),
                port: it.next().and_then(|p| p.parse().ok()),
            }
        }
        "PONG" => ServerEvent::Pong,
        "LOGININFOEND" => ServerEvent::LoginInfoEnd,
        "ADDUSER" => {
            let (f, rest) = split_fields(a, 3)?;
            ServerEvent::AddUser {
                name: f[0].into(),
                country: f[1].into(),
                user_id: f[2].parse().ok(),
                lobby_client: rest.into(),
            }
        }
        "REMOVEUSER" => ServerEvent::RemoveUser {
            name: a.trim().into(),
        },
        "CLIENTSTATUS" => {
            let f = fields(a, 2)?;
            ServerEvent::ClientStatus {
                name: f[0].into(),
                status: UserStatus::from_bits(f[1].parse().ok()?),
            }
        }
        "BATTLEOPENED" => ServerEvent::BattleOpened(Box::new(parse_battle_opened(a)?)),
        "BATTLECLOSED" => ServerEvent::BattleClosed {
            id: a.trim().parse().ok()?,
        },
        "UPDATEBATTLEINFO" => {
            let (f, rest) = split_fields(a, 4)?;
            ServerEvent::UpdateBattleInfo {
                id: f[0].parse().ok()?,
                spectator_count: f[1].parse().ok()?,
                locked: f[2] == "1",
                map_hash: f[3].into(),
                map_name: rest.into(),
            }
        }
        "JOINEDBATTLE" => {
            let mut it = a.split_whitespace();
            ServerEvent::JoinedBattle {
                id: it.next()?.parse().ok()?,
                name: it.next()?.into(),
                script_password: it.next().map(str::to_owned),
            }
        }
        "LEFTBATTLE" => {
            let f = fields(a, 2)?;
            ServerEvent::LeftBattle {
                id: f[0].parse().ok()?,
                name: f[1].into(),
            }
        }
        "CLIENTBATTLESTATUS" => {
            let f = fields(a, 3)?;
            ServerEvent::ClientBattleStatus {
                name: f[0].into(),
                status: BattleStatus::from_bits(f[1].parse().ok()?),
                team_colour: f[2].parse().ok()?,
            }
        }
        "JOINBATTLE" => {
            let f = fields(a, 2)?;
            ServerEvent::JoinBattle {
                id: f[0].parse().ok()?,
                game_hash: f[1].into(),
            }
        }
        "JOINBATTLEFAILED" => ServerEvent::JoinBattleFailed { reason: a.into() },
        "REQUESTBATTLESTATUS" => ServerEvent::RequestBattleStatus,
        "SAIDBATTLE" => {
            let (f, rest) = split_fields(a, 1)?;
            ServerEvent::SaidBattle {
                name: f[0].into(),
                text: rest.into(),
            }
        }
        "SAIDBATTLEEX" => {
            let (f, rest) = split_fields(a, 1)?;
            ServerEvent::SaidBattleEx {
                name: f[0].into(),
                text: rest.into(),
            }
        }
        "SAIDPRIVATE" => {
            let (f, rest) = split_fields(a, 1)?;
            ServerEvent::SaidPrivate {
                name: f[0].into(),
                text: rest.into(),
            }
        }
        "SAYPRIVATE" => {
            let (f, rest) = split_fields(a, 1)?;
            ServerEvent::SentPrivate {
                name: f[0].into(),
                text: rest.into(),
            }
        }
        "JOIN" => ServerEvent::JoinedChannel { room: a.into() },
        "JOINFAILED" => {
            let (room, reason) = a.split_once('\t').unwrap_or((a, ""));
            ServerEvent::JoinChannelFailed {
                room: room.into(),
                reason: reason.into(),
            }
        }
        "CLIENTS" => {
            let (f, rest) = split_fields(a, 1)?;
            ServerEvent::Clients {
                room: f[0].into(),
                names: rest.split_whitespace().map(Into::into).collect(),
            }
        }
        "JOINED" => {
            let (f, rest) = split_fields(a, 1)?;
            ServerEvent::JoinedRoom {
                room: f[0].into(),
                name: rest.into(),
            }
        }
        "LEFT" => {
            let (f, rest) = split_fields(a, 1)?;
            ServerEvent::LeftRoom {
                room: f[0].into(),
                name: rest.into(),
            }
        }
        "SAID" => {
            let (f, rest) = split_fields(a, 2)?;
            ServerEvent::Said {
                room: f[0].into(),
                name: f[1].into(),
                text: rest.into(),
            }
        }
        "SAIDEX" => {
            let (f, rest) = split_fields(a, 2)?;
            ServerEvent::SaidEx {
                room: f[0].into(),
                name: f[1].into(),
                text: rest.into(),
            }
        }
        "CHANNELTOPIC" => {
            let (f, rest) = split_fields(a, 1)?;
            ServerEvent::ChannelTopic {
                room: f[0].into(),
                author: rest.into(),
            }
        }
        "CHANNEL" => {
            let (f, rest) = split_fields(a, 1)?;
            ServerEvent::ChannelListed {
                name: f[0].into(),
                members: rest.split_whitespace().next()?.parse().ok()?,
            }
        }
        "ENDOFCHANNELS" => ServerEvent::EndOfChannels,
        "RING" => ServerEvent::Ring { name: a.into() },
        "FORCEQUITBATTLE" => ServerEvent::ForceQuitBattle,
        "IGNORELISTBEGIN" => ServerEvent::IgnoreListBegin,
        "IGNORELISTEND" => ServerEvent::IgnoreListEnd,
        "IGNORELIST" => ServerEvent::Ignored {
            name: named_user(a)?.into(),
        },
        "FRIENDLISTBEGIN" => ServerEvent::FriendListBegin,
        "FRIENDLISTEND" => ServerEvent::FriendListEnd,
        "FRIENDREQUESTLISTBEGIN" => ServerEvent::FriendRequestListBegin,
        "FRIENDREQUESTLISTEND" => ServerEvent::FriendRequestListEnd,
        "FRIENDLIST" => ServerEvent::Friend {
            name: named_user(a)?.into(),
        },
        "FRIENDREQUESTLIST" => ServerEvent::FriendRequest {
            name: named_user(a)?.into(),
        },
        "SETSCRIPTTAGS" => ServerEvent::SetScriptTags {
            tags: a
                .split('\t')
                .filter_map(|tag| tag.split_once('='))
                .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.to_owned()))
                .collect(),
        },
        "REMOVESCRIPTTAGS" => ServerEvent::RemoveScriptTags {
            keys: a.split_whitespace().map(str::to_ascii_lowercase).collect(),
        },
        "ADDBOT" => {
            let (f, rest) = split_fields(a, 5)?;
            ServerEvent::AddBot {
                id: f[0].parse().ok()?,
                name: f[1].into(),
                owner: f[2].into(),
                status: BattleStatus::from_bits(f[3].parse().ok()?),
                team_colour: f[4].parse().ok()?,
                ai: rest.into(),
            }
        }
        "UPDATEBOT" => {
            let f = fields(a, 4)?;
            ServerEvent::UpdateBot {
                id: f[0].parse().ok()?,
                name: f[1].into(),
                status: BattleStatus::from_bits(f[2].parse().ok()?),
                team_colour: f[3].parse().ok()?,
            }
        }
        "REMOVEBOT" => {
            let f = fields(a, 2)?;
            ServerEvent::RemoveBot {
                id: f[0].parse().ok()?,
                name: f[1].into(),
            }
        }
        "ADDSTARTRECT" => {
            let f = fields(a, 5)?;
            ServerEvent::AddStartRect {
                ally_team: f[0].parse().ok()?,
                left: f[1].parse().ok()?,
                top: f[2].parse().ok()?,
                right: f[3].parse().ok()?,
                bottom: f[4].parse().ok()?,
            }
        }
        "REMOVESTARTRECT" => ServerEvent::RemoveStartRect {
            ally_team: a.trim().parse().ok()?,
        },
        "s.battle.update_lobby_title" => {
            let (id, title) = a.split_once('\t')?;
            ServerEvent::BattleTitle {
                id: id.parse().ok()?,
                title: title.into(),
            }
        }
        "s.battle.teams" => ServerEvent::BattleTeams {
            layouts: parse_battle_teams(a.trim())?,
        },
        "s.system.disconnect" => ServerEvent::Disconnect { reason: a.into() },
        "s.system.shutdown" => ServerEvent::Shutdown,
        _ => ServerEvent::Unknown(raw.clone()),
    };
    Some(event)
}

fn parse_battle_opened(args: &str) -> Option<BattleOpened> {
    let (f, rest) = split_fields(args, 10)?;
    let mut tabs = rest.split('\t');
    let mut tab = || tabs.next().map(str::to_owned);
    Some(BattleOpened {
        id: f[0].parse().ok()?,
        replay: f[1] == "1",
        nat_type: f[2].parse().ok()?,
        founder: f[3].into(),
        ip: f[4].into(),
        port: f[5].parse().ok()?,
        max_players: f[6].parse().ok()?,
        passworded: f[7] == "1",
        rank: f[8].parse().ok()?,
        map_hash: f[9].into(),
        engine_name: tab()?,
        engine_version: tab()?,
        map_name: tab()?,
        title: tab()?,
        game_name: tab()?,
    })
}

/// teiserver encodes the JSON without padding (`Base.encode64(padding: false)`).
fn parse_battle_teams(encoded: &str) -> Option<Vec<(u32, TeamLayout)>> {
    let json = BASE64_STANDARD_NO_PAD
        .decode(encoded.trim_end_matches('='))
        .ok()?;
    let layouts: BTreeMap<u32, TeamLayout> = serde_json::from_slice(&json).ok()?;
    Some(layouts.into_iter().collect())
}

/// The first `n` space-separated fields and the untouched remainder.
/// `userName=<name>`, the shape every friend line uses. teiserver itself only
/// looks at what follows the `=`, so this does the same.
fn named_user(args: &str) -> Option<&str> {
    let name = args.split_once('=').map(|(_, name)| name)?.trim();
    (!name.is_empty()).then_some(name)
}

fn split_fields(args: &str, n: usize) -> Option<(Vec<&str>, &str)> {
    let mut parts = args.splitn(n + 1, ' ');
    let fields: Vec<&str> = parts.by_ref().take(n).collect();
    (fields.len() == n).then(|| (fields, parts.next().unwrap_or("")))
}

/// Exactly `n` whitespace-separated fields (trailing content is ignored).
fn fields(args: &str, n: usize) -> Option<Vec<&str>> {
    let fields: Vec<&str> = args.split_whitespace().take(n).collect();
    (fields.len() == n).then_some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(line: &str) -> ServerEvent {
        RawMessage::parse(line).into()
    }

    #[test]
    fn channel_traffic_parses_into_its_own_events() {
        assert_eq!(
            event("JOIN main"),
            ServerEvent::JoinedChannel {
                room: "main".into()
            }
        );
        assert_eq!(
            event("JOINFAILED secret	You are not allowed"),
            ServerEvent::JoinChannelFailed {
                room: "secret".into(),
                reason: "You are not allowed".into(),
            }
        );
        assert_eq!(
            event("CLIENTS main alice bob carol"),
            ServerEvent::Clients {
                room: "main".into(),
                names: vec!["alice".into(), "bob".into(), "carol".into()],
            }
        );
        assert_eq!(
            event("JOINED main dave"),
            ServerEvent::JoinedRoom {
                room: "main".into(),
                name: "dave".into(),
            }
        );
        assert_eq!(
            event("LEFT main dave"),
            ServerEvent::LeftRoom {
                room: "main".into(),
                name: "dave".into(),
            }
        );
        assert_eq!(
            event("CHANNELTOPIC main alice"),
            ServerEvent::ChannelTopic {
                room: "main".into(),
                author: "alice".into(),
            }
        );
        assert_eq!(
            event("CHANNEL main 412"),
            ServerEvent::ChannelListed {
                name: "main".into(),
                members: 412,
            }
        );
        assert_eq!(event("ENDOFCHANNELS"), ServerEvent::EndOfChannels);
        assert_eq!(event("FORCEQUITBATTLE"), ServerEvent::ForceQuitBattle);
    }

    #[test]
    fn a_message_keeps_every_space_after_the_fields_it_has() {
        assert_eq!(
            event("SAID main alice hello  there  friend"),
            ServerEvent::Said {
                room: "main".into(),
                name: "alice".into(),
                text: "hello  there  friend".into(),
            }
        );
        assert_eq!(
            event("SAIDEX main alice waves slowly"),
            ServerEvent::SaidEx {
                room: "main".into(),
                name: "alice".into(),
                text: "waves slowly".into(),
            }
        );
    }

    #[test]
    fn our_own_private_message_comes_back_as_its_own_event() {
        // The echo is the only confirmation a direct message was delivered, so
        // it must not be mistaken for one arriving from someone else.
        assert_eq!(
            event("SAYPRIVATE bob on my way"),
            ServerEvent::SentPrivate {
                name: "bob".into(),
                text: "on my way".into(),
            }
        );
        assert_eq!(
            event("SAIDPRIVATE bob see you there"),
            ServerEvent::SaidPrivate {
                name: "bob".into(),
                text: "see you there".into(),
            }
        );
    }

    #[test]
    fn battle_opened_splits_space_and_tab_fields() {
        let line = "BATTLEOPENED 42 0 0 [teh]cluster1[03] 78.46.100.74 20303 16 1 0 -123 Recoil\t2026.07.04\tSupreme Isthmus v1.9\t[A] All Welcome\tBeyond All Reason test-30673-8bf91e9";
        let ServerEvent::BattleOpened(b) = event(line) else {
            panic!("not BattleOpened")
        };
        assert_eq!(b.id, 42);
        assert_eq!(b.founder, "[teh]cluster1[03]");
        assert_eq!(b.port, 20303);
        assert!(b.passworded);
        assert_eq!(b.engine_version, "2026.07.04");
        assert_eq!(b.map_name, "Supreme Isthmus v1.9");
        assert_eq!(b.title, "[A] All Welcome");
        assert_eq!(b.game_name, "Beyond All Reason test-30673-8bf91e9");
    }

    #[test]
    fn update_battle_info_keeps_spaces_in_map_name() {
        assert_eq!(
            event("UPDATEBATTLEINFO 42 3 1 -123 Supreme Isthmus v1.9"),
            ServerEvent::UpdateBattleInfo {
                id: 42,
                spectator_count: 3,
                locked: true,
                map_hash: "-123".into(),
                map_name: "Supreme Isthmus v1.9".into()
            }
        );
    }

    #[test]
    fn add_user_keeps_client_name_with_spaces() {
        assert_eq!(
            event("ADDUSER Alice SE 1234 LuaLobby Chobby"),
            ServerEvent::AddUser {
                name: "Alice".into(),
                country: "SE".into(),
                user_id: Some(1234),
                lobby_client: "LuaLobby Chobby".into()
            }
        );
    }

    #[test]
    fn client_status_bits() {
        let s = UserStatus::from_bits(0b110_1011);
        assert!(s.in_game && s.away && s.bot && s.moderator);
        assert_eq!(s.rank, 0b010);
    }

    #[test]
    fn joined_battle_with_and_without_password() {
        assert_eq!(
            event("JOINEDBATTLE 7 Bob s3cret"),
            ServerEvent::JoinedBattle {
                id: 7,
                name: "Bob".into(),
                script_password: Some("s3cret".into())
            }
        );
        assert_eq!(
            event("JOINEDBATTLE 7 Bob"),
            ServerEvent::JoinedBattle {
                id: 7,
                name: "Bob".into(),
                script_password: None
            }
        );
    }

    #[test]
    fn battle_title_splits_on_tab() {
        assert_eq!(
            event(
                "s.battle.update_lobby_title 22\tBeginner Players 6 vs 1 EPIC Scavenger MetalMap | 4v4"
            ),
            ServerEvent::BattleTitle {
                id: 22,
                title: "Beginner Players 6 vs 1 EPIC Scavenger MetalMap | 4v4".into()
            }
        );
    }

    #[test]
    fn battle_teams_decodes_unpadded_base64_json() {
        // Captured 2026-08-29: {"22":{"nbTeams":2,"teamSize":8}}
        assert_eq!(
            event("s.battle.teams eyIyMiI6eyJuYlRlYW1zIjoyLCJ0ZWFtU2l6ZSI6OH19"),
            ServerEvent::BattleTeams {
                layouts: vec![(
                    22,
                    TeamLayout {
                        teams: 2,
                        team_size: 8
                    }
                )]
            }
        );
        // A length that would carry padding in the padded alphabet.
        let unpadded = BASE64_STANDARD_NO_PAD.encode(r#"{"7":{"nbTeams":1,"teamSize":16}}"#);
        assert!(matches!(
            event(&format!("s.battle.teams {unpadded}")),
            ServerEvent::BattleTeams { layouts } if layouts == [(7, TeamLayout { teams: 1, team_size: 16 })]
        ));
        assert!(matches!(
            event("s.battle.teams !!!"),
            ServerEvent::Malformed(_)
        ));
    }

    #[test]
    fn script_tags_split_on_tab_and_lowercase_keys() {
        // Shape of the 177-tag line captured on joining room 3843 (2026-08-29).
        let line = "SETSCRIPTTAGS game/modoptions/allowpausegameplay=1\tgame/modoptions/tweakdefs=\tGAME/HostType=SPADS\tgame/players/tetrisface/skill=[14.61]\tbroken";
        assert_eq!(
            event(line),
            ServerEvent::SetScriptTags {
                tags: vec![
                    ("game/modoptions/allowpausegameplay".into(), "1".into()),
                    ("game/modoptions/tweakdefs".into(), String::new()),
                    ("game/hosttype".into(), "SPADS".into()),
                    ("game/players/tetrisface/skill".into(), "[14.61]".into()),
                ]
            }
        );
        assert_eq!(
            event("REMOVESCRIPTTAGS game/modoptions/Foo game/modoptions/bar"),
            ServerEvent::RemoveScriptTags {
                keys: vec!["game/modoptions/foo".into(), "game/modoptions/bar".into()]
            }
        );
    }

    #[test]
    fn bots_and_start_rects() {
        let ServerEvent::AddBot {
            id,
            name,
            owner,
            status,
            ai,
            ..
        } = event("ADDBOT 5 RaptorsAI Host[EU2][001] 4195330 16777215 BARb")
        else {
            panic!("not AddBot")
        };
        assert_eq!(
            (id, name.as_str(), owner.as_str(), ai.as_str()),
            (5, "RaptorsAI", "Host[EU2][001]", "BARb")
        );
        assert!(status.player && status.ready);
        assert_eq!(
            event("ADDSTARTRECT 1 0 0 200 40"),
            ServerEvent::AddStartRect {
                ally_team: 1,
                left: 0,
                top: 0,
                right: 200,
                bottom: 40
            }
        );
        assert_eq!(
            event("REMOVEBOT 5 RaptorsAI"),
            ServerEvent::RemoveBot {
                id: 5,
                name: "RaptorsAI".into()
            }
        );
        assert_eq!(
            event("SAIDPRIVATE Coordinator Setting tweakdefs requires boss privileges"),
            ServerEvent::SaidPrivate {
                name: "Coordinator".into(),
                text: "Setting tweakdefs requires boss privileges".into()
            }
        );
    }

    #[test]
    fn unknown_and_malformed_are_preserved() {
        assert!(matches!(event("s.user.whois 12"), ServerEvent::Unknown(_)));
        assert!(matches!(
            event("CLIENTSTATUS Alice notanumber"),
            ServerEvent::Malformed(_)
        ));
    }
}
