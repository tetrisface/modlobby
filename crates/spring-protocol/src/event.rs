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
    fn unknown_and_malformed_are_preserved() {
        assert!(matches!(event("s.user.whois 12"), ServerEvent::Unknown(_)));
        assert!(matches!(
            event("CLIENTSTATUS Alice notanumber"),
            ServerEvent::Malformed(_)
        ));
    }
}
