use spring_protocol::telemetry;
use spring_protocol::{Area, Envelope, LoginRequest, ServerEvent};

use crate::state::{LobbyState, Phase};

/// What the application must do in response to an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Send(Envelope),
    LoggedIn {
        username: String,
    },
    /// The post-login flood has finished; state is complete.
    Ready,
    LoginDenied {
        reason: String,
    },
    /// The account must confirm the user agreement (email code) before it can log in.
    AgreementRequired {
        text: Vec<String>,
    },
    Redirect {
        host: String,
        port: Option<u16>,
    },
    /// Server-initiated disconnect; `flood` means re-login is blocked for ~10 s.
    Disconnected {
        reason: String,
        flood: bool,
    },
    Notice(String),
}

/// One logical connection: credentials, machine identity and the state they produce.
#[derive(Debug)]
pub struct Session {
    login: LoginRequest,
    hardware: Vec<(String, String)>,
    machine_hash: String,
    agreement: Vec<String>,
    pub state: LobbyState,
}

impl Session {
    /// `hardware` are the `hardware:*` telemetry properties uploaded after login (see [`telemetry`]).
    pub fn new(login: LoginRequest, hardware: Vec<(String, String)>, machine_hash: String) -> Self {
        Self {
            login,
            hardware,
            machine_hash,
            agreement: Vec::new(),
            state: LobbyState {
                phase: Some(Phase::Connecting),
                ..LobbyState::default()
            },
        }
    }

    pub fn handle(&mut self, event: ServerEvent) -> Vec<Effect> {
        use ServerEvent as E;
        let state = &mut self.state;
        match event {
            E::Welcome { server_version, .. } => {
                tracing::info!(server_version, "connected");
                state.phase = Some(Phase::AwaitingLogin);
                vec![Effect::Send(Envelope::queue(
                    Area::Login,
                    self.login.line(),
                ))]
            }
            E::Accepted { username } => {
                state.phase = Some(Phase::Loading);
                state.me = Some(username.clone());
                vec![Effect::LoggedIn { username }]
            }
            E::Denied { reason } => vec![Effect::LoginDenied { reason }],
            E::Queued => vec![Effect::Notice("login queued by server, waiting".into())],
            E::Agreement { line } => {
                self.agreement.push(line);
                vec![]
            }
            E::AgreementEnd => vec![Effect::AgreementRequired {
                text: std::mem::take(&mut self.agreement),
            }],
            E::Motd { line } => {
                state.motd.push(line);
                vec![]
            }
            E::CompFlags { flags } => {
                state.comp_flags = flags;
                vec![]
            }
            E::ServerMsg { text } => vec![Effect::Notice(text)],
            E::LoginInfoEnd => {
                state.phase = Some(Phase::Ready);
                let mut effects: Vec<Effect> = self
                    .hardware
                    .iter()
                    .map(|(name, value)| {
                        Effect::Send(Envelope::queue(
                            Area::Other,
                            telemetry::update_client_property(name, value, &self.machine_hash),
                        ))
                    })
                    .collect();
                effects.push(Effect::Ready);
                effects
            }
            E::AddUser {
                name,
                country,
                user_id,
                lobby_client,
            } => {
                state.add_user(name, country, user_id, lobby_client);
                vec![]
            }
            E::RemoveUser { name } => {
                state.remove_user(&name);
                vec![]
            }
            E::ClientStatus { name, status } => {
                state.set_status(&name, status);
                vec![]
            }
            E::BattleOpened(opened) => {
                state.open_battle(*opened);
                vec![]
            }
            E::BattleClosed { id } => {
                state.close_battle(id);
                vec![]
            }
            E::UpdateBattleInfo {
                id,
                spectator_count,
                locked,
                map_hash,
                map_name,
            } => {
                if let Some(battle) = state.battles.get_mut(&id) {
                    battle.spectator_count = spectator_count;
                    battle.locked = locked;
                    battle.map_hash = map_hash;
                    battle.map_name = map_name;
                }
                vec![]
            }
            E::JoinedBattle { id, name, .. } => {
                state.join_battle(id, name);
                vec![]
            }
            E::LeftBattle { id, name } => {
                state.leave_battle(id, &name);
                vec![]
            }
            E::ClientBattleStatus { name, status, .. } => {
                if let Some(user) = state.users.get_mut(&name) {
                    user.battle_status = Some(status);
                }
                vec![]
            }
            E::Redirect { host, port } => vec![Effect::Redirect { host, port }],
            E::Disconnect { reason } => {
                let flood = reason.contains("Flood");
                vec![Effect::Disconnected { reason, flood }]
            }
            E::Shutdown => vec![Effect::Disconnected {
                reason: "server shutdown".into(),
                flood: false,
            }],
            E::Pong => vec![],
            E::JoinBattle { .. }
            | E::JoinBattleFailed { .. }
            | E::RequestBattleStatus
            | E::SaidBattle { .. }
            | E::SaidBattleEx { .. } => {
                tracing::debug!("battle-room event not handled yet");
                vec![]
            }
            E::Unknown(raw) => {
                tracing::debug!(command = raw.command, "unhandled command");
                vec![]
            }
            E::Malformed(raw) => {
                tracing::warn!(command = raw.command, args = raw.args, "malformed command");
                vec![]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use spring_protocol::RawMessage;

    use super::*;

    fn session() -> Session {
        Session::new(
            LoginRequest::new("me", "pw", "test", "h h"),
            vec![("hardware:cpuinfo".into(), "cpu".into())],
            "MH".into(),
        )
    }

    fn feed(session: &mut Session, lines: &[&str]) -> Vec<Effect> {
        lines
            .iter()
            .flat_map(|l| session.handle(RawMessage::parse(l).into()))
            .collect()
    }

    #[test]
    fn greeting_triggers_login() {
        let mut s = session();
        let effects = feed(&mut s, &["TASSERVER 0.38 * 8201 0"]);
        assert!(
            matches!(&effects[..], [Effect::Send(env)] if env.area == Area::Login && env.line.starts_with("LOGIN me "))
        );
        assert_eq!(s.state.phase, Some(Phase::AwaitingLogin));
    }

    #[test]
    fn login_flood_builds_state_and_ready_uploads_telemetry() {
        let mut s = session();
        let effects = feed(
            &mut s,
            &[
                "TASSERVER 0.38 * 8201 0",
                "ACCEPTED me",
                "MOTD hi",
                "ADDUSER me SE 1 LuaLobby Chobby",
                "ADDUSER bot GB 2 SPADS",
                "ADDUSER alice DE 3 LuaLobby Chobby",
                "BATTLEOPENED 5 0 0 bot 1.2.3.4 8452 16 0 0 h Recoil\t2026.07.04\tMap One\tTitle\tBeyond All Reason test-1",
                "UPDATEBATTLEINFO 5 1 0 h Map One",
                "JOINEDBATTLE 5 bot",
                "JOINEDBATTLE 5 alice",
                "CLIENTSTATUS bot 64",
                "LOGININFOEND",
            ],
        );
        assert_eq!(s.state.phase, Some(Phase::Ready));
        assert_eq!(s.state.users.len(), 3);
        assert!(s.state.users["bot"].status.bot);
        let battle = &s.state.battles[&5];
        assert_eq!(battle.members.len(), 2);
        assert_eq!(battle.spectator_count, 1);
        assert_eq!(battle.player_count(), 1);
        assert_eq!(s.state.user_battle["alice"], 5);
        let sends: Vec<&Envelope> = effects
            .iter()
            .filter_map(|e| {
                if let Effect::Send(env) = e {
                    Some(env)
                } else {
                    None
                }
            })
            .collect();
        assert!(sends.iter().any(|e| {
            e.line
                .starts_with("c.telemetry.update_client_property hardware:cpuinfo ")
        }));
        assert_eq!(effects.last(), Some(&Effect::Ready));
    }

    #[test]
    fn leaving_and_closing_clean_up_membership() {
        let mut s = session();
        feed(
            &mut s,
            &[
                "BATTLEOPENED 5 0 0 bot 1.2.3.4 8452 16 0 0 h R\tv\tm\tt\tg",
                "ADDUSER alice DE 3 x",
                "JOINEDBATTLE 5 alice",
                "LEFTBATTLE 5 alice",
                "JOINEDBATTLE 5 alice",
                "BATTLECLOSED 5",
            ],
        );
        assert!(s.state.battles.is_empty());
        assert!(s.state.user_battle.is_empty());
    }

    #[test]
    fn flood_disconnect_is_flagged() {
        let mut s = session();
        let effects = feed(&mut s, &["s.system.disconnect Flood protection"]);
        assert_eq!(
            effects,
            vec![Effect::Disconnected {
                reason: "Flood protection".into(),
                flood: true
            }]
        );
    }
}
