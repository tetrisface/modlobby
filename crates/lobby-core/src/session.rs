use spring_protocol::{
    Area, Envelope, LoginRequest, MyBattleStatus, ServerEvent, Sync, battle, status, telemetry,
};

use crate::spads::{self, Announcement, VoteState};
use crate::state::{Bot, LobbyState, MyBattle, Phase, StartRect};

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
    /// The host accepted us into the room, as a spectator.
    Joined {
        id: u32,
    },
    JoinFailed {
        reason: String,
    },
    /// We are out of the room: left, kicked, or it closed.
    LeftBattle {
        id: u32,
    },
    BattleChat {
        from: String,
        text: String,
        /// `SAIDBATTLEEX`: host announcements and `/me` lines.
        announcement: bool,
    },
    /// A direct message; the Coordinator uses these for refusals.
    PrivateChat {
        from: String,
        text: String,
    },
    /// The room's modoptions changed; `keys` are unprefixed, e.g. `tweakdefs1`.
    ModOptionsChanged {
        keys: Vec<String>,
    },
    /// The room's vote started, moved or ended.
    VoteChanged,
    /// A cluster manager answered `!privatehost` with a room password.
    PrivateHostOffered {
        manager: String,
        password: String,
    },
    /// The private room asked for has appeared on the list.
    PrivateHostReady {
        id: u32,
        password: String,
    },
    /// The room's game is running; the engine connects with `spring://<me>:<script_password>@<ip>:<port>`.
    GameRunning {
        id: u32,
        ip: String,
        port: u16,
        script_password: String,
    },
}

/// One logical connection: credentials, machine identity and the state they produce.
#[derive(Debug)]
pub struct Session {
    login: LoginRequest,
    hardware: Vec<(String, String)>,
    machine_hash: String,
    agreement: Vec<String>,
    /// Script password of a `JOINBATTLE` the host has not answered yet.
    pending_join: Option<String>,
    /// The seat we hold in our room; `None` is a spectator.
    seat: Option<(u8, u8)>,
    /// A `!privatehost` we asked for and the password it came back with.
    private_host: Option<String>,
    /// Whether this machine has the room's engine, game and map. Claiming to be
    /// synced when we are not makes the host start a game we cannot join.
    synced: bool,
    pub state: LobbyState,
}

/// Why taking a player slot was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SeatError {
    #[error("not in a battle")]
    NotInARoom,
    /// In a public room a seat belongs to someone else; modlobby only plays in
    /// rooms it was given, which is what a `!privatehost` password proves.
    #[error("this room is public; modlobby only takes a seat in a passworded room")]
    PublicRoom,
}

impl Session {
    /// `hardware` are the `hardware:*` telemetry properties uploaded after login (see [`telemetry`]).
    pub fn new(login: LoginRequest, hardware: Vec<(String, String)>, machine_hash: String) -> Self {
        Self {
            login,
            hardware,
            machine_hash,
            agreement: Vec::new(),
            pending_join: None,
            seat: None,
            private_host: None,
            synced: false,
            state: LobbyState {
                phase: Some(Phase::Connecting),
                ..LobbyState::default()
            },
        }
    }

    /// Asks to join room `id` as a spectator. `script_password` is the secret the
    /// engine later presents to the host; the caller supplies it so this stays pure.
    pub fn join_battle(
        &mut self,
        id: u32,
        password: Option<&str>,
        script_password: String,
    ) -> Vec<Effect> {
        let line = battle::join_battle(id, password, &script_password);
        self.pending_join = Some(script_password);
        vec![Effect::Send(Envelope::queue(Area::Other, line))]
    }

    /// Announces whether we are in a game. SPADS admits a mid-game joiner to the
    /// running game only after seeing this bit (`spads.pl` `cbClientStatus`), so it
    /// must go out before the engine connects.
    pub fn set_in_game(&mut self, in_game: bool) -> Vec<Effect> {
        vec![Effect::Send(Envelope::queue(
            Area::Status,
            status::my_status(in_game, false),
        ))]
    }

    /// Takes a player slot, which is refused unless the room is passworded —
    /// in a public room the slot is someone else's. Pair it with a deliberate
    /// action from the user; nothing here does it on its own.
    pub fn take_seat(&mut self, team: u8, ally_team: u8) -> Result<Vec<Effect>, SeatError> {
        let room = self
            .state
            .my_battle
            .as_ref()
            .and_then(|my| self.state.battles.get(&my.id))
            .ok_or(SeatError::NotInARoom)?;
        if !room.passworded {
            return Err(SeatError::PublicRoom);
        }
        self.seat = Some((team, ally_team));
        Ok(vec![self.battle_status()])
    }

    /// Goes back to spectating; always allowed.
    pub fn release_seat(&mut self) -> Vec<Effect> {
        self.seat = None;
        if self.state.my_battle.is_none() {
            return vec![];
        }
        vec![self.battle_status()]
    }

    pub fn seat(&self) -> Option<(u8, u8)> {
        self.seat
    }

    /// Asks a cluster manager for a room of our own (`!privatehost`); it
    /// answers privately with a password (`battle_list_window.lua:1632-1645`).
    pub fn request_private_host(&mut self, manager: &str) -> Result<Vec<Effect>, battle::TooLong> {
        self.private_host = None;
        Ok(vec![Effect::Send(battle::say_private(
            manager,
            "!privatehost",
        )?)])
    }

    /// Reports whether the room's content is installed. Only a caller that has
    /// actually looked at the disk should say `true`.
    pub fn set_synced(&mut self, synced: bool) -> Vec<Effect> {
        if self.synced == synced {
            return vec![];
        }
        self.synced = synced;
        if self.state.my_battle.is_none() {
            return vec![];
        }
        vec![self.battle_status()]
    }

    pub fn is_synced(&self) -> bool {
        self.synced
    }

    fn battle_status(&self) -> Effect {
        let sync = if self.synced {
            Sync::Synced
        } else {
            Sync::Unsynced
        };
        let status = match self.seat {
            Some((team, ally_team)) => MyBattleStatus::player(sync, team, ally_team),
            None => MyBattleStatus::spectator(sync),
        };
        Effect::Send(Envelope::queue(Area::BattleStatus, status.line()))
    }

    pub fn leave_battle(&mut self) -> Vec<Effect> {
        self.pending_join = None;
        self.seat = None;
        let Some(my) = self.state.my_battle.take() else {
            return vec![];
        };
        self.state.forget_room_details(my.id);
        vec![
            Effect::Send(Envelope::queue(Area::Other, battle::LEAVE_BATTLE)),
            Effect::LeftBattle { id: my.id },
        ]
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
            E::ServerMsg { text } => match machine_marker(&text) {
                // teiserver rides machine data on SERVERMSG behind an
                // `@MARKER@` prefix (`spring_out.ex:82`). It is protocol, not
                // prose, and a person should never be shown it.
                Some(marker) => {
                    tracing::debug!(marker, "server extension message");
                    vec![]
                }
                None => vec![Effect::Notice(text)],
            },
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
                let was_in_game = state.users.get(&name).is_some_and(|u| u.status.in_game);
                state.set_status(&name, status);
                if status.in_game && !was_in_game && self.hosts_my_battle(&name) {
                    return self.game_running().into_iter().collect();
                }
                vec![]
            }
            E::BattleOpened(opened) => {
                // The private room a cluster manager spun up for us carries our
                // name in its title (`battle_list_window.lua:1664`).
                let mine = self.private_host.as_ref().and_then(|password| {
                    let me = state.me.as_deref()?;
                    opened
                        .title
                        .starts_with(me)
                        .then(|| (opened.id, password.clone()))
                });
                state.open_battle(*opened);
                match mine {
                    Some((id, password)) => {
                        self.private_host = None;
                        vec![Effect::PrivateHostReady { id, password }]
                    }
                    None => vec![],
                }
            }
            E::BattleClosed { id } => {
                state.close_battle(id);
                if state.my_battle.as_ref().is_some_and(|my| my.id == id) {
                    state.my_battle = None;
                    return vec![Effect::LeftBattle { id }];
                }
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
                let me = state.me.as_deref() == Some(name.as_str());
                if me && state.my_battle.as_ref().is_some_and(|my| my.id == id) {
                    state.my_battle = None;
                    state.forget_room_details(id);
                    return vec![Effect::LeftBattle { id }];
                }
                vec![]
            }
            E::ClientBattleStatus { name, status, .. } => {
                if let Some(user) = state.users.get_mut(&name) {
                    user.battle_status = Some(status);
                }
                vec![]
            }
            E::JoinBattle { id, game_hash } => {
                let script_password = self.pending_join.take().unwrap_or_default();
                state.my_battle = Some(MyBattle::new(id, game_hash, script_password));
                let mut effects = vec![Effect::Joined { id }];
                effects.extend(self.game_running());
                effects
            }
            E::JoinBattleFailed { reason } => {
                self.pending_join = None;
                vec![Effect::JoinFailed { reason }]
            }
            E::RequestBattleStatus => vec![self.battle_status()],
            E::SaidBattle { name, text } => vec![Effect::BattleChat {
                from: name,
                text,
                announcement: false,
            }],
            E::SaidBattleEx { name, text } => {
                let mut effects = self.host_announcement(&name, &text);
                effects.push(Effect::BattleChat {
                    from: name,
                    text,
                    announcement: true,
                });
                effects
            }
            E::SaidPrivate { name, text } => {
                let mut effects = Vec::new();
                if let Some(password) = private_host_password(&text) {
                    self.private_host = Some(password.clone());
                    effects.push(Effect::PrivateHostOffered {
                        manager: name.clone(),
                        password,
                    });
                }
                effects.push(Effect::PrivateChat { from: name, text });
                effects
            }
            E::SetScriptTags { tags } => {
                let Some(my) = state.my_battle.as_mut() else {
                    return vec![];
                };
                let keys = my.set_script_tags(tags);
                if keys.is_empty() {
                    return vec![];
                }
                vec![Effect::ModOptionsChanged { keys }]
            }
            E::RemoveScriptTags { keys } => {
                if let Some(my) = state.my_battle.as_mut() {
                    for key in &keys {
                        my.script_tags.remove(key);
                    }
                }
                vec![]
            }
            E::AddBot {
                id,
                name,
                owner,
                status,
                team_colour,
                ai,
            } => {
                if let Some(battle) = state.battles.get_mut(&id) {
                    let bot = Bot {
                        name: name.clone(),
                        owner,
                        status,
                        team_colour,
                        ai,
                    };
                    battle.bots.insert(name, bot);
                }
                vec![]
            }
            E::UpdateBot {
                id,
                name,
                status,
                team_colour,
            } => {
                let bot = state
                    .battles
                    .get_mut(&id)
                    .and_then(|battle| battle.bots.get_mut(&name));
                if let Some(bot) = bot {
                    bot.status = status;
                    bot.team_colour = team_colour;
                }
                vec![]
            }
            E::RemoveBot { id, name } => {
                if let Some(battle) = state.battles.get_mut(&id) {
                    battle.bots.remove(&name);
                }
                vec![]
            }
            E::AddStartRect {
                ally_team,
                left,
                top,
                right,
                bottom,
            } => {
                if let Some(battle) = state.my_room_mut() {
                    let rect = StartRect {
                        left,
                        top,
                        right,
                        bottom,
                    };
                    battle.start_rects.insert(ally_team, rect);
                }
                vec![]
            }
            E::RemoveStartRect { ally_team } => {
                if let Some(battle) = state.my_room_mut() {
                    battle.start_rects.remove(&ally_team);
                }
                vec![]
            }
            E::BattleTitle { id, title } => {
                if let Some(battle) = state.battles.get_mut(&id) {
                    battle.title = title;
                }
                vec![]
            }
            E::BattleTeams { layouts } => {
                for (id, layout) in layouts {
                    if let Some(battle) = state.battles.get_mut(&id) {
                        battle.layout = Some(layout);
                    }
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

    /// SPADS announces votes and setting changes as chat. Only the room's
    /// founder is believed — Chobby gates on the same thing
    /// (`gui_battle_room_window.lua:4848`).
    fn host_announcement(&mut self, from: &str, text: &str) -> Vec<Effect> {
        if !self.hosts_my_battle(from) {
            return vec![];
        }
        let Some(announcement) = spads::parse(text) else {
            return vec![];
        };
        let Some(my) = self.state.my_battle.as_mut() else {
            return vec![];
        };
        match announcement {
            Announcement::VoteCalled { by, command } => {
                my.vote = Some(VoteState::called(by, command));
                vec![Effect::VoteChanged]
            }
            Announcement::VoteProgress {
                command,
                yes,
                yes_needed,
                no,
                no_needed,
                remaining_secs,
            } => {
                let vote = my
                    .vote
                    .get_or_insert_with(|| VoteState::called(String::new(), command));
                vote.yes = yes;
                vote.yes_needed = yes_needed;
                vote.no = no;
                vote.no_needed = no_needed;
                vote.remaining_secs = remaining_secs;
                vec![Effect::VoteChanged]
            }
            Announcement::VoteEnded { .. } | Announcement::VoteCancelled => {
                my.vote = None;
                vec![Effect::VoteChanged]
            }
            Announcement::SettingChanged { by, key, value } => {
                if my.setting_changed(key.clone(), value, by) {
                    return vec![Effect::ModOptionsChanged { keys: vec![key] }];
                }
                vec![]
            }
            // The BAR plugin's JSON duplicates what the text already told us.
            Announcement::BarManager { .. } => vec![],
        }
    }

    /// Cluster managers, the bots that spin up rooms: `Host[EU1]`, `Host[AU2]`.
    /// A `region` of `"EU"` matches any of that region's managers.
    pub fn cluster_managers(&self, region: &str) -> Vec<&str> {
        let prefix = format!("Host[{}", region.to_ascii_uppercase());
        // Chobby's `^Host%[%a+%d+%]$`: a manager, not one of its instances
        // (`Host[EU1][03]`), which is why nothing may follow the bracket.
        let is_manager = |name: &str| {
            name.strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(']'))
                .is_some_and(|digits| {
                    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
                })
        };
        let mut names: Vec<&str> = self
            .state
            .users
            .values()
            .filter(|user| user.status.bot && is_manager(&user.name))
            .map(|user| user.name.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    fn hosts_my_battle(&self, name: &str) -> bool {
        self.state
            .my_battle
            .as_ref()
            .and_then(|my| self.state.battles.get(&my.id))
            .is_some_and(|battle| battle.founder == name)
    }

    /// [`Effect::GameRunning`] when our room's host is in game right now.
    fn game_running(&self) -> Option<Effect> {
        let my = self.state.my_battle.as_ref()?;
        let battle = self.state.battles.get(&my.id)?;
        let host = self.state.users.get(&battle.founder)?;
        host.status.in_game.then(|| Effect::GameRunning {
            id: my.id,
            ip: battle.ip.clone(),
            port: battle.port,
            script_password: my.script_password.clone(),
        })
    }
}

/// `… Starting a new private instance in …, password=XXXX` — the reply Chobby
/// scrapes at `battle_list_window.lua:1634-1637`.
fn private_host_password(text: &str) -> Option<String> {
    let (_, rest) = text.split_once("password=")?;
    let password: String = rest
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    (!password.is_empty()).then_some(password)
}

/// `@NAME@ …`: teiserver's convention for data addressed to the client rather
/// than to the person using it.
fn machine_marker(text: &str) -> Option<&str> {
    let (name, _) = text.strip_prefix('@')?.split_once('@')?;
    let machine = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    machine.then_some(name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_server_extension_line_is_not_shown_to_anyone() {
        use super::machine_marker;
        // What teiserver actually sends on connect (`spring_out.ex:82`).
        assert_eq!(
            machine_marker("@PROTOCOL_EXTENSIONS@ {\"ring:originator\":1}"),
            Some("PROTOCOL_EXTENSIONS")
        );
        // Prose keeps its notice, including prose that merely mentions an `@`.
        assert_eq!(machine_marker("Welcome to teiserver"), None);
        assert_eq!(machine_marker("ask @someone about it"), None);
        assert_eq!(machine_marker("@@"), None);
    }
    use spring_protocol::{BattleStatus, RawMessage};

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

    fn sent_lines(effects: &[Effect]) -> Vec<&str> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Send(env) => Some(env.line.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Logged in, with one SPADS-hosted room (id 5, host `host`) on the list.
    fn ready_with_room() -> Session {
        let mut s = session();
        feed(
            &mut s,
            &[
                "TASSERVER 0.38 * 8201 0",
                "ACCEPTED me",
                "ADDUSER me SE 1 LuaLobby Chobby",
                "ADDUSER host GB 2 SPADS",
                "BATTLEOPENED 5 0 0 host 1.2.3.4 8452 16 0 0 h R\tv\tm\tt\tg",
                "LOGININFOEND",
            ],
        );
        s
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
        assert_eq!(s.state.user_battle["bot"], 5);
        assert!(sent_lines(&effects).iter().any(|line| {
            line.starts_with("c.telemetry.update_client_property hardware:cpuinfo ")
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
    fn founder_counts_without_a_joinedbattle_line() {
        // Captured 2026-08-29: a full 8v8 with three human spectators. The host
        // never gets a JOINEDBATTLE but is counted in UPDATEBATTLEINFO's spectators.
        let mut s = session();
        let mut lines = vec![
            "BATTLEOPENED 57 0 0 Host[US4][000] 144.126.147.151 53200 16 0 0 -590370561 spring\t2026.07.04\tSupreme Isthmus v2.1\tSuPrEmE MuFF | 8v8\tBeyond All Reason test-31115-21dbf79".to_owned(),
            "UPDATEBATTLEINFO 57 4 0 -590370561 Supreme Isthmus v2.1".to_owned(),
        ];
        lines.extend((0..19).map(|i| format!("JOINEDBATTLE 57 player{i}")));
        for line in &lines {
            s.handle(RawMessage::parse(line).into());
        }
        let battle = &s.state.battles[&57];
        assert_eq!(battle.members.len(), 20);
        assert_eq!(battle.player_count(), 16);
        assert_eq!(s.state.user_battle["Host[US4][000]"], 57);
    }

    #[test]
    fn battle_title_and_teams_update_the_room() {
        let mut s = session();
        feed(
            &mut s,
            &[
                "BATTLEOPENED 22 0 0 bot 1.2.3.4 8452 16 0 0 h R\tv\tm\told title\tg",
                "s.battle.update_lobby_title 22\tBeginner Players | 4v4",
                "s.battle.teams eyIyMiI6eyJuYlRlYW1zIjoyLCJ0ZWFtU2l6ZSI6OH19",
            ],
        );
        let battle = &s.state.battles[&22];
        assert_eq!(battle.title, "Beginner Players | 4v4");
        assert_eq!(battle.layout.map(|l| (l.teams, l.team_size)), Some((2, 8)));
    }

    #[test]
    fn join_flow_answers_the_status_request_as_a_spectator() {
        let mut s = ready_with_room();
        let effects = s.join_battle(5, None, "4242".into());
        assert_eq!(sent_lines(&effects), ["JOINBATTLE 5 empty 4242"]);

        // What teiserver sends once SPADS accepts (spring_out.ex do_join_battle).
        let effects = feed(
            &mut s,
            &[
                "JOINBATTLE 5 -1",
                "JOINEDBATTLE 5 me 4242",
                "REQUESTBATTLESTATUS",
            ],
        );
        assert!(effects.contains(&Effect::Joined { id: 5 }));
        let [status_line] = sent_lines(&effects)[..] else {
            panic!("expected exactly one MYBATTLESTATUS, got {effects:?}")
        };
        let bits: u32 = status_line
            .strip_prefix("MYBATTLESTATUS ")
            .and_then(|rest| rest.split(' ').next())
            .and_then(|bits| bits.parse().ok())
            .expect("MYBATTLESTATUS <bits> <colour>");
        let status = BattleStatus::from_bits(bits);
        assert!(!status.player, "must never take a player slot");
        assert!(!status.ready);
        assert_eq!(
            s.state
                .my_battle
                .as_ref()
                .map(|m| (m.id, m.script_password.as_str())),
            Some((5, "4242"))
        );
        assert_eq!(s.state.user_battle["me"], 5);
    }

    #[test]
    fn host_in_game_reports_game_running_on_join_and_on_change() {
        // Already running when we join: bot (64) + in-game (1).
        let mut s = ready_with_room();
        feed(&mut s, &["CLIENTSTATUS host 65"]);
        s.join_battle(5, None, "4242".into());
        let effects = feed(&mut s, &["JOINBATTLE 5 -1"]);
        assert!(effects.contains(&Effect::GameRunning {
            id: 5,
            ip: "1.2.3.4".into(),
            port: 8452,
            script_password: "4242".into()
        }));

        // Starts after we joined; repeated in-game statuses do not repeat the effect.
        let mut s = ready_with_room();
        s.join_battle(5, None, "4242".into());
        let effects = feed(
            &mut s,
            &[
                "JOINBATTLE 5 -1",
                "CLIENTSTATUS host 64",
                "CLIENTSTATUS host 65",
                "CLIENTSTATUS host 65",
            ],
        );
        let running = effects
            .iter()
            .filter(|e| matches!(e, Effect::GameRunning { .. }))
            .count();
        assert_eq!(running, 1);
    }

    #[test]
    fn join_failure_and_leaving_clear_the_room() {
        let mut s = ready_with_room();
        s.join_battle(5, None, "1".into());
        assert_eq!(
            feed(&mut s, &["JOINBATTLEFAILED Battle locked"]),
            vec![Effect::JoinFailed {
                reason: "Battle locked".into()
            }]
        );
        assert!(s.state.my_battle.is_none());

        s.join_battle(5, None, "1".into());
        feed(&mut s, &["JOINBATTLE 5 -1", "JOINEDBATTLE 5 me 1"]);
        assert_eq!(
            feed(&mut s, &["LEFTBATTLE 5 me"]),
            vec![Effect::LeftBattle { id: 5 }]
        );
        assert!(s.state.my_battle.is_none());

        s.join_battle(5, None, "1".into());
        feed(&mut s, &["JOINBATTLE 5 -1"]);
        let effects = s.leave_battle();
        assert_eq!(sent_lines(&effects), ["LEAVEBATTLE"]);
        assert!(effects.contains(&Effect::LeftBattle { id: 5 }));
        assert!(s.leave_battle().is_empty());
    }

    #[test]
    fn in_game_status_is_a_mystatus_line() {
        let mut s = ready_with_room();
        assert_eq!(sent_lines(&s.set_in_game(true)), ["MYSTATUS 1"]);
        assert_eq!(sent_lines(&s.set_in_game(false)), ["MYSTATUS 0"]);
    }

    #[test]
    fn battle_chat_becomes_effects() {
        let mut s = ready_with_room();
        assert_eq!(
            feed(
                &mut s,
                &[
                    "SAIDBATTLE host hello there",
                    "SAIDBATTLEEX host * welcomes me"
                ]
            ),
            vec![
                Effect::BattleChat {
                    from: "host".into(),
                    text: "hello there".into(),
                    announcement: false
                },
                Effect::BattleChat {
                    from: "host".into(),
                    text: "* welcomes me".into(),
                    announcement: true
                },
            ]
        );
    }

    #[test]
    fn room_details_follow_script_tags_bots_and_start_boxes() {
        let mut s = ready_with_room();
        s.join_battle(5, None, "1".into());
        feed(
            &mut s,
            &[
                "JOINBATTLE 5 -1",
                "JOINEDBATTLE 5 me 1",
                "SETSCRIPTTAGS game/modoptions/tweakdefs=abc\tgame/hosttype=SPADS",
                "ADDBOT 5 RaptorsAI host 4195330 16777215 BARb",
                "ADDSTARTRECT 1 0 0 200 40",
            ],
        );
        let my = s.state.my_battle.as_ref().unwrap();
        assert_eq!(my.modoptions().collect::<Vec<_>>(), [("tweakdefs", "abc")]);
        assert_eq!(my.script_tags["game/hosttype"], "SPADS");
        let room = &s.state.battles[&5];
        assert_eq!(room.bots["RaptorsAI"].ai, "BARb");
        assert!(room.bots["RaptorsAI"].status.player);
        assert_eq!(room.start_rects[&1].bottom, 40);

        feed(
            &mut s,
            &[
                "SETSCRIPTTAGS game/modoptions/tweakdefs=def",
                "UPDATEBOT 5 RaptorsAI 0 16777215",
                "REMOVESTARTRECT 1",
            ],
        );
        assert_eq!(
            s.state.my_battle.as_ref().unwrap().script_tags["game/modoptions/tweakdefs"],
            "def"
        );
        assert!(!s.state.battles[&5].bots["RaptorsAI"].status.player);
        assert!(s.state.battles[&5].start_rects.is_empty());

        feed(&mut s, &["REMOVESCRIPTTAGS game/modoptions/tweakdefs"]);
        assert_eq!(s.state.my_battle.as_ref().unwrap().modoptions().count(), 0);

        // Leaving drops what the server stops telling us about.
        feed(&mut s, &["LEFTBATTLE 5 me"]);
        assert!(s.state.battles[&5].bots.is_empty());
    }

    /// A tweak vote as the room actually delivers it: the host announces, the
    /// tally moves, it passes, and only then does the value land.
    #[test]
    fn a_tweak_vote_is_followed_from_call_to_setting() {
        let mut s = ready_with_room();
        s.join_battle(5, None, "1".into());
        feed(&mut s, &["JOINBATTLE 5 -1", "JOINEDBATTLE 5 me 1"]);

        let effects = feed(
            &mut s,
            &[
                "SAIDBATTLEEX host * Bob called a vote for command \"bSet tweakdefs1 QUJD\" [!vote y, !vote n, !vote b]",
            ],
        );
        assert!(effects.contains(&Effect::VoteChanged));
        let vote = s.state.my_battle.as_ref().unwrap().vote.clone().unwrap();
        assert_eq!(vote.by.as_deref(), Some("Bob"));
        assert_eq!(
            vote.proposal,
            crate::Proposal::SetOption {
                key: "tweakdefs1".into(),
                value: "QUJD".into()
            }
        );

        feed(
            &mut s,
            &[
                "SAIDBATTLEEX host * Vote in progress: \"bSet tweakdefs1 QUJD\" [y:2/3, n:1/4(5)] (17s remaining)",
            ],
        );
        let vote = s.state.my_battle.as_ref().unwrap().vote.clone().unwrap();
        assert_eq!((vote.yes, vote.yes_needed, vote.no), (2, 3, 1));
        assert_eq!(vote.remaining_secs, 17);

        // Someone else's chat must not move the room's state.
        feed(&mut s, &["SAIDBATTLEEX alice * Vote cancelled by alice"]);
        assert!(s.state.my_battle.as_ref().unwrap().vote.is_some());

        let effects = feed(
            &mut s,
            &[
                "SAIDBATTLEEX host * Vote for command \"bSet tweakdefs1 QUJD\" passed.",
                "SETSCRIPTTAGS game/modoptions/tweakdefs1=QUJD",
                "SAIDBATTLEEX host * Battle setting changed by Bob (tweakdefs1=QUJD)",
            ],
        );
        assert!(s.state.my_battle.as_ref().unwrap().vote.is_none());
        assert!(effects.contains(&Effect::ModOptionsChanged {
            keys: vec!["tweakdefs1".into()]
        }));

        let my = s.state.my_battle.as_ref().unwrap();
        assert_eq!(my.modoption("tweakdefs1"), "QUJD");
        // One change, attributed once the host said who did it.
        assert_eq!(my.history.len(), 1);
        assert_eq!(my.history[0].by.as_deref(), Some("Bob"));
        assert_eq!(
            (my.history[0].from.as_str(), my.history[0].to.as_str()),
            ("", "QUJD")
        );
    }

    /// Clearing a slot never reaches SETSCRIPTTAGS (`spads.pl:2625-2628`), so
    /// the announcement has to carry it.
    #[test]
    fn a_cleared_slot_is_recorded_from_the_announcement_alone() {
        let mut s = ready_with_room();
        s.join_battle(5, None, "1".into());
        feed(
            &mut s,
            &[
                "JOINBATTLE 5 -1",
                "SETSCRIPTTAGS game/modoptions/tweakdefs1=QUJD",
                "SAIDBATTLEEX host * Battle setting changed by Bob (tweakdefs1=)",
            ],
        );
        let my = s.state.my_battle.as_ref().unwrap();
        assert_eq!(my.modoption("tweakdefs1"), "");
        assert_eq!(my.history.len(), 2);
        assert_eq!(my.history[1].from, "QUJD");
        assert!(my.history[1].to.is_empty());
    }

    /// The safety property this project runs on: a seat is only ever taken in a
    /// room we were given, and only when asked for.
    #[test]
    fn a_seat_is_refused_in_a_public_room_and_granted_in_a_private_one() {
        let mut s = ready_with_room();
        assert_eq!(s.take_seat(0, 0), Err(SeatError::NotInARoom));

        s.join_battle(5, None, "1".into());
        feed(&mut s, &["JOINBATTLE 5 -1", "JOINEDBATTLE 5 me 1"]);
        assert_eq!(s.take_seat(0, 0), Err(SeatError::PublicRoom));
        assert_eq!(s.seat(), None);

        // What the room answers while we are a spectator.
        let status = |effects: &[Effect]| {
            let [Effect::Send(env)] = effects else {
                panic!("expected one status, got {effects:?}")
            };
            let rest = env
                .line
                .strip_prefix("MYBATTLESTATUS ")
                .expect("a status line");
            let bits: u32 = rest.split(' ').next().unwrap().parse().unwrap();
            BattleStatus::from_bits(bits)
        };
        assert!(!status(&feed(&mut s, &["REQUESTBATTLESTATUS"])).player);

        // A passworded room is one a cluster manager gave us.
        feed(
            &mut s,
            &["BATTLEOPENED 9 0 0 host2 1.2.3.4 8452 16 1 0 h R\tv\tm\tt\tg"],
        );
        s.leave_battle();
        s.join_battle(9, Some("pw"), "1".into());
        feed(&mut s, &["JOINBATTLE 9 -1", "JOINEDBATTLE 9 me 1"]);

        let taken = s.take_seat(2, 1).expect("a passworded room is ours");
        assert_eq!(s.seat(), Some((2, 1)));
        let decoded = status(&taken);
        assert!(decoded.player);
        assert_eq!((decoded.team, decoded.ally_team), (2, 1));
        // And the room's later question gets the same answer.
        assert!(status(&feed(&mut s, &["REQUESTBATTLESTATUS"])).player);

        assert!(!status(&s.release_seat()).player);
        assert_eq!(s.seat(), None);
        // Leaving forgets the seat, so the next room starts as a spectator.
        s.take_seat(2, 1).unwrap();
        s.leave_battle();
        assert_eq!(s.seat(), None);
    }

    #[test]
    fn a_private_host_is_asked_for_and_recognised_when_it_appears() {
        let mut s = ready_with_room();
        feed(
            &mut s,
            &[
                "ADDUSER Host[EU1] DE 9 SPADS",
                "ADDUSER Host[EU2] DE 10 SPADS",
                "ADDUSER Host[EU1][03] DE 11 SPADS",
                "ADDUSER Host[AU1] AU 12 SPADS",
                "CLIENTSTATUS Host[EU1] 64",
                "CLIENTSTATUS Host[EU2] 64",
                "CLIENTSTATUS Host[EU1][03] 64",
                "CLIENTSTATUS Host[AU1] 64",
            ],
        );
        assert_eq!(s.cluster_managers("eu"), ["Host[EU1]", "Host[EU2]"]);
        assert_eq!(s.cluster_managers("AU"), ["Host[AU1]"]);

        let asked = s.request_private_host("Host[EU1]").unwrap();
        assert!(
            matches!(&asked[..], [Effect::Send(env)] if env.line == "SAYPRIVATE Host[EU1] !privatehost")
        );

        let offered = feed(
            &mut s,
            &[
                "SAIDPRIVATE Host[EU1] Starting a new private instance in EU, password=ab12 - please wait",
            ],
        );
        assert!(offered.contains(&Effect::PrivateHostOffered {
            manager: "Host[EU1]".into(),
            password: "ab12".into()
        }));

        // Someone else's room is not ours; ours carries our name in the title.
        assert!(
            feed(
                &mut s,
                &["BATTLEOPENED 20 0 0 Host[EU1][04] 1.2.3.4 8452 16 1 0 h R\tv\tm\tsomeone else\tg"],
            )
            .is_empty()
        );
        let ready = feed(
            &mut s,
            &[
                "BATTLEOPENED 21 0 0 Host[EU1][05] 1.2.3.4 8452 16 1 0 h R\tv\tm\tme's private room\tg",
            ],
        );
        assert!(ready.contains(&Effect::PrivateHostReady {
            id: 21,
            password: "ab12".into()
        }));
    }

    #[test]
    fn private_messages_become_effects() {
        let mut s = ready_with_room();
        assert_eq!(
            feed(
                &mut s,
                &["SAIDPRIVATE Coordinator Setting tweakdefs requires boss privileges"]
            ),
            vec![Effect::PrivateChat {
                from: "Coordinator".into(),
                text: "Setting tweakdefs requires boss privileges".into()
            }]
        );
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
