use spring_protocol::{
    Area, Envelope, LoginRequest, MyBattleStatus, ServerEvent, Sync, battle, chat, friends, status,
    telemetry,
};

use crate::spads::{self, Announcement, VoteState};
use crate::state::{Bot, Channel, LobbyState, MyBattle, Phase, StartRect};

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
    /// A message in a channel we are in.
    ChannelChat {
        room: String,
        from: String,
        text: String,
        /// `SAIDEX`: an emote, rendered as an action rather than speech.
        emote: bool,
    },
    /// We are now in a channel, with its roster as far as it has arrived.
    ChannelJoined {
        room: String,
    },
    ChannelJoinFailed {
        room: String,
        reason: String,
    },
    ChannelLeft {
        room: String,
    },
    /// The roster or topic of a channel changed.
    ChannelChanged {
        room: String,
    },
    /// The server's channel directory finished arriving.
    ChannelsListed,
    /// The friend list or the pending requests changed.
    FriendsChanged,
    /// Who is bossing our room changed.
    BossChanged,
    /// The server said something to everyone: the message of the day, or a
    /// broadcast.
    ServerSaid {
        text: String,
    },
    /// Someone is summoning us; Chobby alerts on this unconditionally, because
    /// it is a person asking for you rather than the room making noise.
    Rung {
        by: String,
    },
    /// A direct message; the Coordinator uses these for refusals.
    ///
    /// `with` is the other person, which is who the conversation is filed
    /// under whichever way the message went; `from` is who wrote it.
    PrivateChat {
        with: String,
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
        /// Whether the game began just now, rather than having been under way
        /// before we got here.
        ///
        /// The two are the same connection but not the same event. Walking
        /// into a room with a game in progress is an invitation to watch;
        /// a game starting around you is the thing you came for. Only the
        /// second is a reason to do anything on somebody's behalf.
        just_started: bool,
    },
    /// Our room's host came back out of its game.
    GameStopped,
    /// How long our room's game had been going when we walked in, from SPADS's
    /// welcome message — the only place this protocol states it.
    GameInProgress {
        id: u32,
        elapsed_secs: u64,
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
    seat: Option<Seat>,
    /// Whether a seat may be taken in a public room. Off unless the owner says
    /// otherwise: in a public room a slot is a real player's game.
    allow_public_seat: bool,
    /// A `!privatehost` we asked for and the password it came back with.
    private_host: Option<String>,
    /// Whether this machine has the room's engine, game and map. Claiming to be
    /// synced when we are not makes the host start a game we cannot join.
    synced: bool,
    /// The two bits `MYSTATUS` carries. Kept together because the command
    /// carries both at once and there is no way to send one alone.
    in_game: bool,
    away: bool,
    /// A friend listing part-way through arriving. Held aside so a listing that
    /// is cut off never half-replaces the one we have.
    collecting_friends: Option<std::collections::BTreeSet<String>>,
    collecting_requests: Option<std::collections::BTreeSet<String>>,
    collecting_ignored: Option<std::collections::BTreeSet<String>>,
    pub state: LobbyState,
}

/// What to do about a friendship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendAction {
    Request,
    Accept,
    Decline,
    Remove,
    Ignore,
    Unignore,
}

#[derive(Debug, thiserror::Error)]
#[error("no such friend action: {0}")]
pub struct UnknownFriendAction(String);

impl std::str::FromStr for FriendAction {
    type Err = UnknownFriendAction;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "request" => Ok(Self::Request),
            "accept" => Ok(Self::Accept),
            "decline" => Ok(Self::Decline),
            "remove" => Ok(Self::Remove),
            "ignore" => Ok(Self::Ignore),
            "unignore" => Ok(Self::Unignore),
            other => Err(UnknownFriendAction(other.to_owned())),
        }
    }
}

/// Why taking a player slot was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SeatError {
    #[error("not in a battle")]
    NotInARoom,
    /// Seats in public rooms were declined for this session. A room that is
    /// ours — passworded, or one SPADS says we boss — is never refused.
    #[error("this session is watching only; seats are turned off in settings")]
    PublicRoom,
    /// Ready and faction belong to a player; a spectator has neither.
    #[error("you are spectating")]
    Spectating,
}

/// The seat we hold, and what we have said about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    pub team: u8,
    pub ally_team: u8,
    pub ready: bool,
    pub side: u8,
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
            collecting_friends: None,
            collecting_requests: None,
            collecting_ignored: None,
            seat: None,
            allow_public_seat: false,
            private_host: None,
            synced: false,
            in_game: false,
            away: false,
            state: LobbyState {
                phase: Some(Phase::Connecting),
                ..LobbyState::default()
            },
        }
    }

    /// Asks to join room `id` as a spectator. `script_password` is the secret the
    /// engine later presents to the host; the caller supplies it so this stays pure.
    /// Joins a room, leaving the one we are in first.
    ///
    /// teiserver keeps one room per client, so a `JOINBATTLE` sent while still
    /// in another is simply ignored — which looks exactly like nothing
    /// happening.
    pub fn join_battle(
        &mut self,
        id: u32,
        password: Option<&str>,
        script_password: String,
    ) -> Vec<Effect> {
        let mut effects = match self.state.my_battle.as_ref() {
            Some(my) if my.id == id => return vec![],
            Some(_) => self.leave_battle(),
            None => Vec::new(),
        };
        let line = battle::join_battle(id, password, &script_password);
        self.pending_join = Some(script_password);
        effects.push(Effect::Send(Envelope::queue(Area::Other, line)));
        effects
    }

    /// Announces whether we are in a game. SPADS admits a mid-game joiner to the
    /// running game only after seeing this bit (`spads.pl` `cbClientStatus`), so it
    /// must go out before the engine connects.
    pub fn set_in_game(&mut self, in_game: bool) -> Vec<Effect> {
        self.in_game = in_game;
        vec![self.status()]
    }

    /// Marks us away, or back. One `MYSTATUS` carries both bits, so each is
    /// remembered here — sending one without the other would silently claim we
    /// had left the game we are in.
    pub fn set_away(&mut self, away: bool) -> Vec<Effect> {
        self.away = away;
        vec![self.status()]
    }

    fn status(&self) -> Effect {
        Effect::Send(Envelope::queue(
            Area::Status,
            status::my_status(self.in_game, self.away),
        ))
    }

    /// Takes a player slot.
    ///
    /// Refused only when seats in public rooms have been turned off, which is
    /// how a client with nobody at the keyboard says it is watching; a room
    /// that is ours is always ours to sit in. Nothing here does it on its own —
    /// it is always a deliberate action from the user.
    pub fn take_seat(&mut self, team: u8, ally_team: u8) -> Result<Vec<Effect>, SeatError> {
        let room = self
            .state
            .my_battle
            .as_ref()
            .and_then(|my| self.state.battles.get(&my.id))
            .ok_or(SeatError::NotInARoom)?;
        // Ours if it was given to us (passworded), or if SPADS says we are
        // bossing it — which is what joining an empty autohost makes you.
        let ours = room.passworded
            || self
                .state
                .my_battle
                .as_ref()
                .and_then(|my| my.boss.as_deref())
                .is_some_and(|boss| Some(boss) == self.state.me.as_deref());
        if !ours && !self.allow_public_seat {
            return Err(SeatError::PublicRoom);
        }
        // Ready never survives sitting down: a seat taken is not a game agreed to.
        self.seat = Some(Seat {
            team,
            ally_team,
            ready: false,
            side: self.seat.map_or(0, |seat| seat.side),
        });
        Ok(vec![self.battle_status()])
    }

    /// Whether a seat may be taken in a public room at all.
    pub fn allow_public_seat(&mut self, allow: bool) {
        self.allow_public_seat = allow;
    }

    /// Says we are ready, or not. Only a player can be either.
    pub fn set_ready(&mut self, ready: bool) -> Result<Vec<Effect>, SeatError> {
        let seat = self.seat.as_mut().ok_or(SeatError::Spectating)?;
        if seat.ready == ready {
            return Ok(vec![]);
        }
        seat.ready = ready;
        Ok(vec![self.battle_status()])
    }

    /// Picks a faction: 0 Armada, 1 Cortex, 2 Random, 3 Legion.
    pub fn set_side(&mut self, side: u8) -> Result<Vec<Effect>, SeatError> {
        let seat = self.seat.as_mut().ok_or(SeatError::Spectating)?;
        if seat.side == side {
            return Ok(vec![]);
        }
        seat.side = side;
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

    pub fn seat(&self) -> Option<Seat> {
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

    /// Joins a channel. The server answers with `JOIN` or `JOINFAILED`, so
    /// nothing is added to state here.
    pub fn join_channel(
        &mut self,
        room: &str,
        key: Option<&str>,
    ) -> Result<Vec<Effect>, chat::BadChannel> {
        Ok(vec![Effect::Send(chat::join(room, key)?)])
    }

    /// Leaves a channel. teiserver reports our own departure as `LEFT`, the
    /// same as anyone else's, which is where the channel is dropped from state.
    pub fn leave_channel(&mut self, room: &str) -> Result<Vec<Effect>, chat::BadChannel> {
        Ok(vec![Effect::Send(chat::leave(room)?)])
    }

    /// Says something in a channel. A leading `/me ` becomes an emote, which is
    /// how every lobby client has spelled it since the protocol was written.
    pub fn say_channel(&mut self, room: &str, text: &str) -> Result<Vec<Effect>, chat::SayError> {
        let envelope = match text.strip_prefix("/me ") {
            Some(action) => chat::say_ex(room, action)?,
            None => chat::say(room, text)?,
        };
        Ok(vec![Effect::Send(envelope)])
    }

    /// Sends a direct message. Nothing appears until the server echoes it back.
    pub fn say_private(&mut self, user: &str, text: &str) -> Result<Vec<Effect>, battle::TooLong> {
        Ok(vec![Effect::Send(battle::say_private(user, text)?)])
    }

    /// Asks a host how long its game has been going.
    ///
    /// The only way to learn that about a room you are not in: SPADS answers a
    /// JSON-RPC request carried on a private message. Ask sparingly — it is a
    /// message to a real account, and hovering a list would otherwise send one
    /// per row.
    pub fn request_game_status(&mut self, founder: &str) -> Result<Vec<Effect>, battle::TooLong> {
        Ok(vec![Effect::Send(battle::say_private(
            founder,
            spads::GAME_STATUS_REQUEST,
        )?)])
    }

    /// The battle a host runs, if we can see one.
    fn battle_hosted_by(&self, founder: &str) -> Option<u32> {
        self.state
            .battles
            .values()
            .find(|battle| battle.founder == founder)
            .map(|battle| battle.id)
    }

    /// Rings someone: the lobby's way of saying the game is waiting on you.
    pub fn ring(&mut self, user: &str) -> Vec<Effect> {
        vec![Effect::Send(battle::ring(user))]
    }

    /// Adds an AI to the room, seated and ready.
    ///
    /// The AI runs on this machine when the game starts, so the caller names
    /// one that is installed here. Whether the room lets us is the server's
    /// call — SPADS answers a refusal in chat, where it will be seen.
    pub fn add_bot(
        &mut self,
        name: &str,
        ai: &str,
        team: u8,
        ally_team: u8,
        colour: u32,
    ) -> Vec<Effect> {
        let status =
            battle::MyBattleStatus::player(battle::Sync::Synced, team, ally_team).ready(true);
        vec![Effect::Send(battle::add_bot(name, ai, status, colour))]
    }

    /// Removes an AI by name; the server decides whether we may.
    pub fn remove_bot(&mut self, name: &str) -> Vec<Effect> {
        vec![Effect::Send(battle::remove_bot(name))]
    }

    /// Asks for both friend listings; each replaces what it had.
    pub fn refresh_friends(&mut self) -> Vec<Effect> {
        vec![
            Effect::Send(friends::list()),
            Effect::Send(friends::list_requests()),
            Effect::Send(friends::list_ignored()),
        ]
    }

    /// Acts on a friendship, then asks for the listings again — the server
    /// sends nothing of its own accord when one changes.
    pub fn friend_action(&mut self, action: FriendAction, user: &str) -> Vec<Effect> {
        let envelope = match action {
            FriendAction::Request => friends::request(user),
            FriendAction::Accept => friends::accept(user),
            FriendAction::Decline => friends::decline(user),
            FriendAction::Remove => friends::remove(user),
            FriendAction::Ignore => friends::ignore(user),
            FriendAction::Unignore => friends::unignore(user),
        };
        let mut effects = vec![Effect::Send(envelope)];
        effects.extend(self.refresh_friends());
        effects
    }

    /// Asks for the server's channel directory, which replaces the last one.
    pub fn list_channels(&mut self) -> Vec<Effect> {
        self.state.directory.clear();
        vec![Effect::Send(chat::list())]
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
            Some(seat) => MyBattleStatus::player(sync, seat.team, seat.ally_team)
                .ready(seat.ready)
                .side(seat.side),
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
                // Kept as well as flashed: a broadcast you were away for is
                // still worth being able to scroll back to.
                None => vec![
                    Effect::ServerSaid { text: text.clone() },
                    Effect::Notice(text),
                ],
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
                // The message of the day arrives before the login is complete,
                // so it is replayed here — in order, once there is somewhere
                // for it to go.
                effects.extend(
                    state
                        .motd
                        .iter()
                        .map(|line| Effect::ServerSaid { text: line.clone() }),
                );
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
                if !self.hosts_my_battle(&name) || status.in_game == was_in_game {
                    return vec![];
                }
                if status.in_game {
                    // The host's bit went up while we were standing here.
                    return self.game_running(true).into_iter().collect();
                }
                // The bit going the other way is the only sign a game ended.
                // Without this the room goes on offering to connect you to one
                // that finished, for as long as you stay in it.
                vec![Effect::GameStopped]
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
                // Already under way before we arrived: worth saying, not worth
                // acting on.
                effects.extend(self.game_running(false));
                effects
            }
            E::JoinBattleFailed { reason } => {
                self.pending_join = None;
                vec![Effect::JoinFailed { reason }]
            }
            E::RequestBattleStatus => vec![self.battle_status()],
            E::SentPrivate { name, text } => {
                // The server echoing us back is the only confirmation the
                // message left, so it is what puts our own words on screen.
                let me = state.me.clone().unwrap_or_default();
                vec![Effect::PrivateChat {
                    with: name,
                    from: me,
                    text,
                }]
            }
            E::JoinedChannel { room } => {
                self.state
                    .channels
                    .entry(room.clone())
                    .or_insert_with(|| Channel {
                        name: room.clone(),
                        ..Channel::default()
                    });
                vec![Effect::ChannelJoined { room }]
            }
            E::JoinChannelFailed { room, reason } => {
                vec![Effect::ChannelJoinFailed { room, reason }]
            }
            E::Clients { room, names } => {
                // Sent in batches, so this adds to the roster rather than
                // replacing it; a second batch must not erase the first.
                let Some(channel) = self.state.channels.get_mut(&room) else {
                    return vec![];
                };
                channel.members.extend(names);
                vec![Effect::ChannelChanged { room }]
            }
            E::JoinedRoom { room, name } => {
                let Some(channel) = self.state.channels.get_mut(&room) else {
                    return vec![];
                };
                channel.members.insert(name);
                vec![Effect::ChannelChanged { room }]
            }
            E::LeftRoom { room, name } => {
                let Some(channel) = self.state.channels.get_mut(&room) else {
                    return vec![];
                };
                // The server tells us about our own departure the same way it
                // tells us about anyone else's.
                if Some(&name) == self.state.me.as_ref() {
                    self.state.channels.remove(&room);
                    return vec![Effect::ChannelLeft { room }];
                }
                channel.members.remove(&name);
                vec![Effect::ChannelChanged { room }]
            }
            E::ChannelTopic { room, author } => {
                let Some(channel) = self.state.channels.get_mut(&room) else {
                    return vec![];
                };
                channel.topic_author = Some(author);
                vec![Effect::ChannelChanged { room }]
            }
            E::Said { room, name, text } => vec![Effect::ChannelChat {
                room,
                from: name,
                text,
                emote: false,
            }],
            E::SaidEx { room, name, text } => vec![Effect::ChannelChat {
                room,
                from: name,
                text,
                emote: true,
            }],
            E::ChannelListed { name, members } => {
                self.state
                    .directory
                    .push(crate::state::ChannelSummary { name, members });
                vec![]
            }
            E::EndOfChannels => vec![Effect::ChannelsListed],
            // Both listings arrive whole, so each one is collected into a
            // fresh set and swapped in at the end: a friend removed elsewhere
            // has to disappear here too.
            E::Ring { name } => vec![Effect::Rung { by: name }],
            E::ForceQuitBattle => {
                // The server has already dropped us; keeping the room on screen
                // would leave every action in it silently doing nothing.
                let Some(my) = state.my_battle.take() else {
                    return vec![];
                };
                state.forget_room_details(my.id);
                self.seat = None;
                vec![
                    Effect::Notice("you were removed from the room".into()),
                    Effect::LeftBattle { id: my.id },
                ]
            }
            E::FriendListBegin => {
                self.collecting_friends = Some(std::collections::BTreeSet::new());
                vec![]
            }
            E::Friend { name } => {
                if let Some(collecting) = self.collecting_friends.as_mut() {
                    collecting.insert(name);
                }
                vec![]
            }
            E::FriendListEnd => {
                let Some(collected) = self.collecting_friends.take() else {
                    return vec![];
                };
                self.state.friends = collected;
                vec![Effect::FriendsChanged]
            }
            E::IgnoreListBegin => {
                self.collecting_ignored = Some(std::collections::BTreeSet::new());
                vec![]
            }
            E::Ignored { name } => {
                if let Some(collecting) = self.collecting_ignored.as_mut() {
                    collecting.insert(name);
                }
                vec![]
            }
            E::IgnoreListEnd => {
                let Some(collected) = self.collecting_ignored.take() else {
                    return vec![];
                };
                self.state.ignored = collected;
                vec![Effect::FriendsChanged]
            }
            E::FriendRequestListBegin => {
                self.collecting_requests = Some(std::collections::BTreeSet::new());
                vec![]
            }
            E::FriendRequest { name } => {
                if let Some(collecting) = self.collecting_requests.as_mut() {
                    collecting.insert(name);
                }
                vec![]
            }
            E::FriendRequestListEnd => {
                let Some(collected) = self.collecting_requests.take() else {
                    return vec![];
                };
                self.state.friend_requests = collected;
                vec![Effect::FriendsChanged]
            }
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
                // A host answering what we asked about its game, on the side
                // channel SPADS carries over private messages.
                if let Some(spads::RpcStatus::Game { seconds, .. }) = spads::parse_rpc(&text)
                    && let Some(id) = self.battle_hosted_by(&name)
                {
                    effects.push(Effect::GameInProgress {
                        id,
                        elapsed_secs: seconds,
                    });
                }
                if let Some(password) = private_host_password(&text) {
                    self.private_host = Some(password.clone());
                    effects.push(Effect::PrivateHostOffered {
                        manager: name.clone(),
                        password,
                    });
                }
                effects.push(Effect::PrivateChat {
                    with: name.clone(),
                    from: name,
                    text,
                });
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
            // The one statement of a game's age this protocol carries, and it
            // is addressed to us alone as we walk in.
            Announcement::GameInProgress { elapsed_secs } => {
                vec![Effect::GameInProgress {
                    id: my.id,
                    elapsed_secs,
                }]
            }
            Announcement::SettingChanged { by, key, value } => {
                if my.setting_changed(key.clone(), value, by) {
                    return vec![Effect::ModOptionsChanged { keys: vec![key] }];
                }
                vec![]
            }
            // The BAR plugin's JSON duplicates what the text already told us.
            Announcement::BarManager { json } => {
                // The room's own statement of who is in charge of it.
                let boss = spads::boss(&json);
                let Some(my) = self.state.my_battle.as_mut() else {
                    return vec![];
                };
                if my.boss == boss {
                    return vec![];
                }
                my.boss = boss;
                vec![Effect::BossChanged]
            }
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

    /// How many rooms a cluster is already running.
    ///
    /// A manager `Host[EU1]` runs instances named `Host[EU1][012]`, and each
    /// instance is one room, so counting the instances counts the load.
    fn cluster_load(&self, manager: &str) -> u32 {
        let prefix = format!("{}[", manager.trim_end_matches(']'));
        self.state
            .users
            .values()
            .filter(|user| {
                user.status.bot
                    && user.name.starts_with(&prefix)
                    && user.name.ends_with(']')
                    && user.name.len() > prefix.len() + 1
            })
            .count() as u32
    }

    /// Picks a cluster manager in a region, favouring the emptiest.
    ///
    /// Chobby weights by `(1 - current/limit) * (limit - current)` and draws
    /// against the total (`battle_list_window.lua:1453`), which spreads rooms
    /// across clusters instead of piling every request onto whichever name
    /// happens to sort first. `roll` is a fresh number in `0.0..1.0`.
    pub fn pick_cluster_manager(&self, region: &str, roll: f64) -> Option<&str> {
        /// Chobby's default when a cluster reports no capacity of its own.
        const LIMIT: f64 = 80.0;

        let weighted: Vec<(&str, f64)> = self
            .cluster_managers(region)
            .into_iter()
            .map(|manager| {
                let current = f64::from(self.cluster_load(manager)).min(LIMIT);
                let weight = (1.0 - current / LIMIT) * (LIMIT - current);
                (manager, weight.max(0.0))
            })
            .collect();

        let total: f64 = weighted.iter().map(|(_, weight)| weight).sum();
        if total <= 0.0 {
            // Every cluster is full, or none reported capacity; the caller
            // still deserves an answer rather than silence.
            return weighted.first().map(|(manager, _)| *manager);
        }

        let mut drawn = roll.clamp(0.0, 1.0) * total;
        for (manager, weight) in &weighted {
            drawn -= weight;
            if drawn <= 0.0 {
                return Some(manager);
            }
        }
        weighted.last().map(|(manager, _)| *manager)
    }

    /// An autohost room in this region that nobody is in.
    ///
    /// Joining one is how Chobby's Host button makes a *public* room: the
    /// autohost is already listed, and the first person in it becomes its boss.
    pub fn empty_public_host(&self, region: &str) -> Option<u32> {
        let prefix = format!("Host[{}", region.to_ascii_uppercase());
        self.state
            .battles
            .values()
            .filter(|battle| {
                battle.founder.starts_with(&prefix)
                    && !battle.passworded
                    && !battle.locked
                    && battle.player_count() == 0
            })
            .map(|battle| battle.id)
            .min()
    }

    fn hosts_my_battle(&self, name: &str) -> bool {
        self.state
            .my_battle
            .as_ref()
            .and_then(|my| self.state.battles.get(&my.id))
            .is_some_and(|battle| battle.founder == name)
    }

    /// [`Effect::GameRunning`] when our room's host is in game right now.
    fn game_running(&self, just_started: bool) -> Option<Effect> {
        let my = self.state.my_battle.as_ref()?;
        let battle = self.state.battles.get(&my.id)?;
        let host = self.state.users.get(&battle.founder)?;
        host.status.in_game.then(|| Effect::GameRunning {
            id: my.id,
            ip: battle.ip.clone(),
            port: battle.port,
            script_password: my.script_password.clone(),
            just_started,
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

    /// A session that believes it is logged in as `me`, without the flood.
    fn joined_session() -> Session {
        let mut s = session();
        s.state.me = Some("me".into());
        s
    }

    #[test]
    fn a_channel_roster_is_built_from_batches_rather_than_replaced() {
        let mut s = joined_session();
        feed(&mut s, &["JOIN main"]);
        // teiserver sends CLIENTS in batches; a later one must not erase the
        // earlier one (`spring_out.ex:442`).
        feed(&mut s, &["CLIENTS main alice bob", "CLIENTS main carol"]);

        let channel = &s.state.channels["main"];
        assert_eq!(
            channel.members.iter().cloned().collect::<Vec<_>>(),
            vec!["alice", "bob", "carol"]
        );
    }

    #[test]
    fn people_arriving_and_leaving_move_the_roster() {
        let mut s = joined_session();
        feed(&mut s, &["JOIN main", "CLIENTS main alice bob"]);
        feed(&mut s, &["JOINED main carol", "LEFT main alice"]);

        let members = &s.state.channels["main"].members;
        assert!(members.contains("carol"));
        assert!(!members.contains("alice"));
    }

    #[test]
    fn our_own_departure_drops_the_channel_entirely() {
        let mut s = joined_session();
        feed(&mut s, &["JOIN main", "CLIENTS main me alice"]);

        // The server reports us leaving exactly as it reports anyone else.
        let effects = feed(&mut s, &["LEFT main me"]);
        assert!(matches!(
            effects.as_slice(),
            [Effect::ChannelLeft { room }] if room == "main"
        ));
        assert!(!s.state.channels.contains_key("main"));
    }

    #[test]
    fn traffic_for_a_channel_we_are_not_in_is_ignored() {
        let mut s = joined_session();
        assert!(feed(&mut s, &["CLIENTS other alice"]).is_empty());
        assert!(feed(&mut s, &["JOINED other alice"]).is_empty());
        assert!(s.state.channels.is_empty());
    }

    #[test]
    fn a_message_and_an_emote_are_told_apart() {
        let mut s = joined_session();
        let effects = feed(
            &mut s,
            &["SAID main alice hello", "SAIDEX main alice waves"],
        );
        assert_eq!(
            effects,
            vec![
                Effect::ChannelChat {
                    room: "main".into(),
                    from: "alice".into(),
                    text: "hello".into(),
                    emote: false,
                },
                Effect::ChannelChat {
                    room: "main".into(),
                    from: "alice".into(),
                    text: "waves".into(),
                    emote: true,
                },
            ]
        );
    }

    #[test]
    fn a_private_conversation_is_filed_under_the_other_person_either_way() {
        let mut s = joined_session();
        let effects = feed(
            &mut s,
            &["SAIDPRIVATE bob you there?", "SAYPRIVATE bob on my way"],
        );
        assert_eq!(
            effects,
            vec![
                Effect::PrivateChat {
                    with: "bob".into(),
                    from: "bob".into(),
                    text: "you there?".into(),
                },
                // Our own message comes back from the server; both sides of the
                // conversation file under `bob`.
                Effect::PrivateChat {
                    with: "bob".into(),
                    from: "me".into(),
                    text: "on my way".into(),
                },
            ]
        );
    }

    #[test]
    fn slash_me_becomes_an_emote_and_the_cap_is_enforced() {
        let mut s = joined_session();
        assert_eq!(
            sent_lines(&s.say_channel("main", "hello").unwrap()),
            vec!["SAY main hello"]
        );
        assert_eq!(
            sent_lines(&s.say_channel("main", "/me waves").unwrap()),
            vec!["SAYEX main waves"]
        );
        assert!(s.say_channel("main", &"x".repeat(258)).is_err());
        assert!(s.join_channel("#main", None).is_err());
    }

    #[test]
    fn the_directory_replaces_itself_rather_than_growing() {
        let mut s = joined_session();
        s.list_channels();
        feed(
            &mut s,
            &["CHANNEL main 412", "CHANNEL bar 30", "ENDOFCHANNELS"],
        );
        assert_eq!(s.state.directory.len(), 2);

        // Asking again starts over, so a shrinking server list shrinks here.
        s.list_channels();
        feed(&mut s, &["CHANNEL main 400", "ENDOFCHANNELS"]);
        assert_eq!(s.state.directory.len(), 1);
        assert_eq!(s.state.directory[0].members, 400);
    }

    #[test]
    fn a_friend_listing_replaces_the_last_one_whole() {
        let mut s = joined_session();
        feed(
            &mut s,
            &[
                "FRIENDLISTBEGIN",
                "FRIENDLIST userName=alice",
                "FRIENDLIST userName=bob",
                "FRIENDLISTEND",
            ],
        );
        assert_eq!(
            s.state.friends.iter().cloned().collect::<Vec<_>>(),
            vec!["alice", "bob"]
        );

        // Someone unfriended elsewhere has to disappear here too, which only
        // works because the listing replaces rather than merges.
        feed(
            &mut s,
            &[
                "FRIENDLISTBEGIN",
                "FRIENDLIST userName=alice",
                "FRIENDLISTEND",
            ],
        );
        assert_eq!(
            s.state.friends.iter().cloned().collect::<Vec<_>>(),
            vec!["alice"]
        );
    }

    #[test]
    fn a_listing_cut_off_part_way_leaves_the_old_one_standing() {
        let mut s = joined_session();
        feed(
            &mut s,
            &[
                "FRIENDLISTBEGIN",
                "FRIENDLIST userName=alice",
                "FRIENDLISTEND",
            ],
        );
        // Begin, one name, and then nothing: without the end marker the
        // half-built set must not be swapped in.
        feed(&mut s, &["FRIENDLISTBEGIN", "FRIENDLIST userName=zoe"]);
        assert_eq!(
            s.state.friends.iter().cloned().collect::<Vec<_>>(),
            vec!["alice"]
        );
    }

    #[test]
    fn requests_are_kept_apart_from_friends() {
        let mut s = joined_session();
        feed(
            &mut s,
            &[
                "FRIENDREQUESTLISTBEGIN",
                "FRIENDREQUESTLIST userName=carol",
                "FRIENDREQUESTLISTEND",
            ],
        );
        assert!(s.state.friends.is_empty());
        assert!(s.state.friend_requests.contains("carol"));
    }

    #[test]
    fn acting_on_a_friendship_asks_for_the_listings_again() {
        let mut s = joined_session();
        // The server announces nothing when a friendship changes, so the only
        // way to see the result is to ask.
        assert_eq!(
            sent_lines(&s.friend_action(FriendAction::Accept, "carol")),
            vec![
                "ACCEPTFRIENDREQUEST userName=carol",
                "FRIENDLIST",
                "FRIENDREQUESTLIST",
                "IGNORELIST"
            ]
        );
    }

    #[test]
    fn a_kick_drops_the_room_rather_than_leaving_it_on_screen() {
        let mut s = joined_session();
        feed(
            &mut s,
            &[
                "BATTLEOPENED 7 0 0 host 1.2.3.4 8452 16 0 0 -1 Recoil	2026.07.04	Supreme Isthmus v2.1	a room	BAR test",
                "JOINBATTLE 7 hash",
            ],
        );
        assert!(s.state.my_battle.is_some());

        // The server has already forgotten us by the time this arrives, so
        // there is nothing to send back — only state to correct.
        let effects = feed(&mut s, &["FORCEQUITBATTLE"]);
        assert!(s.state.my_battle.is_none());
        assert!(sent_lines(&effects).is_empty(), "a kick is not a request");
        assert!(matches!(
            effects.as_slice(),
            [Effect::Notice(_), Effect::LeftBattle { id: 7 }]
        ));
    }

    #[test]
    fn a_kick_when_we_are_in_no_room_changes_nothing() {
        let mut s = joined_session();
        assert!(feed(&mut s, &["FORCEQUITBATTLE"]).is_empty());
    }

    /// The `MYBATTLESTATUS` a set of effects carries, decoded.
    #[test]
    fn the_emptiest_cluster_is_favoured_over_the_first_by_name() {
        let mut s = joined_session();
        // Two managers: EU1 already running three rooms, EU2 running none.
        for name in [
            "Host[EU1]",
            "Host[EU1][001]",
            "Host[EU1][002]",
            "Host[EU1][003]",
            "Host[EU2]",
        ] {
            feed(&mut s, &[&format!("ADDUSER {name} EU 0 modlobby")]);
            feed(&mut s, &[&format!("CLIENTSTATUS {name} 64")]);
        }

        // Sorting by name alone would always answer EU1; weighting by how
        // loaded each one is puts most of the draw on EU2.
        assert_eq!(s.pick_cluster_manager("EU", 0.99), Some("Host[EU2]"));
        assert_eq!(s.pick_cluster_manager("EU", 0.0), Some("Host[EU1]"));
    }

    #[test]
    fn a_region_with_no_managers_offers_nothing() {
        let s = joined_session();
        assert_eq!(s.pick_cluster_manager("EU", 0.5), None);
    }

    #[test]
    fn an_empty_public_autohost_is_one_you_can_take_over() {
        let mut s = joined_session();
        feed(
            &mut s,
            &[
                // Busy: someone is already in it.
                "BATTLEOPENED 1 0 0 Host[EU1][001] 1.2.3.4 8452 16 0 0 -1 R	v	m	t	g",
                "JOINEDBATTLE 1 alice",
                // Passworded: not a public room.
                "BATTLEOPENED 2 0 0 Host[EU1][002] 1.2.3.4 8452 16 1 0 -1 R	v	m	t	g",
                // Empty and open.
                "BATTLEOPENED 3 0 0 Host[EU1][003] 1.2.3.4 8452 16 0 0 -1 R	v	m	t	g",
                // Another region.
                "BATTLEOPENED 4 0 0 Host[US1][001] 1.2.3.4 8452 16 0 0 -1 R	v	m	t	g",
            ],
        );

        assert_eq!(s.empty_public_host("EU"), Some(3));
        assert_eq!(s.empty_public_host("US"), Some(4));
        assert_eq!(s.empty_public_host("AU"), None);
    }

    fn sent_status(effects: &[Effect]) -> spring_protocol::BattleStatus {
        let [Effect::Send(env)] = effects else {
            panic!("expected one status, got {effects:?}")
        };
        let rest = env
            .line
            .strip_prefix("MYBATTLESTATUS ")
            .expect("a status line");
        let bits: u32 = rest.split(' ').next().unwrap().parse().unwrap();
        spring_protocol::BattleStatus::from_bits(bits)
    }

    /// A session sitting in a public room, ready to try for a seat.
    fn in_a_public_room() -> Session {
        let mut s = joined_session();
        feed(
            &mut s,
            &[
                "ADDUSER host EU 0 SPADS",
                "BATTLEOPENED 3 0 0 host 1.2.3.4 8452 16 0 0 -1 R	v	m	t	g",
                "JOINBATTLE 3 hash",
            ],
        );
        s
    }

    #[test]
    fn joining_a_second_room_leaves_the_first() {
        let mut s = in_a_public_room();
        feed(
            &mut s,
            &["BATTLEOPENED 4 0 0 other 1.2.3.4 8452 16 0 0 -1 R	v	m	t	g"],
        );

        // Without the LEAVEBATTLE the server ignores the join outright, and
        // the app sits in the old room believing it moved.
        let effects = s.join_battle(4, None, "pw".into());
        let lines = sent_lines(&effects);
        assert_eq!(lines.len(), 2, "leave then join, got {lines:?}");
        assert_eq!(lines[0], "LEAVEBATTLE");
        assert!(lines[1].starts_with("JOINBATTLE 4"));
    }

    #[test]
    fn joining_the_room_we_are_already_in_does_nothing() {
        let mut s = in_a_public_room();
        assert!(s.join_battle(3, None, "pw".into()).is_empty());
    }

    #[test]
    fn a_seat_may_be_taken_while_the_current_game_runs() {
        let mut s = in_a_public_room();
        s.allow_public_seat(true);

        // Sitting down during a game is how you join the *next* one: SPADS has
        // the lineup ready when this one ends. Refusing it would make the
        // commonest thing anyone does in a busy room impossible.
        feed(&mut s, &["CLIENTSTATUS host 1"]);
        assert!(s.take_seat(0, 0).is_ok());
    }

    #[test]
    fn a_room_we_boss_is_ours_without_any_licence() {
        let mut s = in_a_public_room();
        assert!(matches!(s.take_seat(0, 0), Err(SeatError::PublicRoom)));

        // Joining an empty autohost makes you its boss, and SPADS says so.
        feed(
            &mut s,
            &[r#"SAIDBATTLEEX host * BarManager|{"BattleStateChanged": {"boss": "me"}}"#],
        );
        assert!(s.take_seat(0, 0).is_ok(), "our own room needs no licence");

        // Somebody else bossing it does not make it ours.
        feed(
            &mut s,
            &[r#"SAIDBATTLEEX host * BarManager|{"BattleStateChanged": {"boss": "alice"}}"#],
        );
        s.release_seat();
        assert!(matches!(s.take_seat(0, 0), Err(SeatError::PublicRoom)));
    }

    #[test]
    fn a_public_seat_is_refused_until_the_owner_allows_it() {
        let mut s = in_a_public_room();
        assert!(matches!(s.take_seat(0, 0), Err(SeatError::PublicRoom)));

        // The guard is a decision, not a law of nature.
        s.allow_public_seat(true);
        assert!(s.take_seat(0, 0).is_ok());
    }

    #[test]
    fn ready_and_faction_ride_on_the_battle_status() {
        let mut s = in_a_public_room();
        s.allow_public_seat(true);
        s.take_seat(3, 1).unwrap();

        let ready = sent_status(&s.set_ready(true).unwrap());
        assert!(ready.ready);
        assert_eq!((ready.team, ready.ally_team), (3, 1));

        let side = sent_status(&s.set_side(2).unwrap());
        assert_eq!(side.side, 2);
        // Setting ready did not lose the faction, nor the other way round.
        assert!(side.ready);
    }

    #[test]
    fn saying_the_same_thing_twice_sends_nothing() {
        let mut s = in_a_public_room();
        s.allow_public_seat(true);
        s.take_seat(0, 0).unwrap();
        s.set_ready(true).unwrap();
        assert!(
            s.set_ready(true).unwrap().is_empty(),
            "the room already knows"
        );
        assert!(s.set_side(0).unwrap().is_empty());
    }

    #[test]
    fn a_spectator_is_neither_ready_nor_a_faction() {
        let mut s = in_a_public_room();
        assert!(matches!(s.set_ready(true), Err(SeatError::Spectating)));
        assert!(matches!(s.set_side(1), Err(SeatError::Spectating)));
    }

    #[test]
    fn sitting_down_again_is_not_a_game_agreed_to() {
        let mut s = in_a_public_room();
        s.allow_public_seat(true);
        s.take_seat(0, 0).unwrap();
        s.set_side(3).unwrap();
        s.set_ready(true).unwrap();

        // Moving to another team clears ready but keeps the faction: one is a
        // statement about this arrangement, the other is a preference.
        let moved = sent_status(&s.take_seat(1, 1).unwrap());
        assert!(!moved.ready);
        assert_eq!(moved.side, 3);
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
    fn the_host_leaving_its_game_is_what_says_the_game_ended() {
        let mut s = ready_with_room();
        s.join_battle(5, None, "4242".into());
        feed(&mut s, &["JOINBATTLE 5 -1", "CLIENTSTATUS host 65"]);

        // The same bit going back down. Nothing else on the wire says a game
        // finished, and a room that never hears it goes on offering to connect
        // you to one that is over.
        let effects = feed(&mut s, &["CLIENTSTATUS host 64"]);
        assert!(effects.contains(&Effect::GameStopped));

        // Said once: a status line that changes something else is not news.
        assert!(!feed(&mut s, &["CLIENTSTATUS host 64"]).contains(&Effect::GameStopped));
    }

    #[test]
    fn walking_into_a_running_game_is_told_apart_from_one_starting() {
        // The distinction the whole auto-launch behaviour rests on. Both are
        // the same running game and the same connection details; only one is
        // a reason to start an engine for somebody.
        let started = |effects: &[Effect]| {
            effects.iter().find_map(|effect| match effect {
                Effect::GameRunning { just_started, .. } => Some(*just_started),
                _ => None,
            })
        };

        // Joining a room whose host is already in game.
        let mut s = ready_with_room();
        feed(&mut s, &["CLIENTSTATUS host 65"]);
        s.join_battle(5, None, "4242".into());
        assert_eq!(started(&feed(&mut s, &["JOINBATTLE 5 -1"])), Some(false));

        // The same room, the game starting while we stand in it.
        let mut s = ready_with_room();
        s.join_battle(5, None, "4242".into());
        feed(&mut s, &["JOINBATTLE 5 -1", "CLIENTSTATUS host 64"]);
        assert_eq!(
            started(&feed(&mut s, &["CLIENTSTATUS host 65"])),
            Some(true)
        );
    }

    #[test]
    fn host_in_game_reports_game_running_on_join_and_on_change() {
        // Already running when we join: bot (64) + in-game (1).
        let mut s = ready_with_room();
        feed(&mut s, &["CLIENTSTATUS host 65"]);
        s.join_battle(5, None, "4242".into());
        let effects = feed(&mut s, &["JOINBATTLE 5 -1"]);
        // Reported, but marked as something that was already under way: it is
        // an invitation to watch, not a game starting around us, and nothing
        // should be launched on somebody's behalf for it.
        assert!(effects.contains(&Effect::GameRunning {
            id: 5,
            ip: "1.2.3.4".into(),
            port: 8452,
            script_password: "4242".into(),
            just_started: false,
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
    fn away_and_in_game_are_sent_together_because_the_command_carries_both() {
        let mut s = session();
        assert_eq!(sent_lines(&s.set_away(true)), ["MYSTATUS 2"]);
        // Going into a game must not quietly say we came back.
        assert_eq!(sent_lines(&s.set_in_game(true)), ["MYSTATUS 3"]);
        assert_eq!(sent_lines(&s.set_away(false)), ["MYSTATUS 1"]);
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
        assert_eq!(
            s.seat().map(|seat| (seat.team, seat.ally_team)),
            Some((2, 1))
        );
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
                with: "Coordinator".into(),
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
