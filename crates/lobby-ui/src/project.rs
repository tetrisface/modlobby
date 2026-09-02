//! Turns one reduced server event (plus the effects it produced) into deltas.
//! Derived from the event and the state *after* reduction, so the core keeps
//! no dirty tracking. Nothing but phase changes is emitted before the lobby is
//! ready: the runtime sends a [`Snapshot`](crate::Snapshot) then.

use lobby_core::{Effect, LobbyState};
use spring_protocol::ServerEvent;

use crate::mention;
use crate::model::{
    AlertKind, BATTLE_ROOM, BotView, ChannelSummaryView, ChannelView, ChatKind, ChatLine, Delta,
    FriendsView, GameRunningView, LayoutView, MyBattleView, NoticeLevel, OptionChangeView, Phase,
    SERVER_ROOM, StartRectView, UserView, VoteView, private_room,
};

#[derive(Debug, Default)]
pub struct Projector {
    seq: u64,
    /// Whether a vote was already open, so only its opening raises an alert.
    vote_open: bool,
}

impl Projector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn project(
        &mut self,
        event: &ServerEvent,
        effects: &[Effect],
        state: &LobbyState,
    ) -> Vec<Delta> {
        let mut out = Vec::new();
        self.project_event(event, state, &mut out);
        self.project_effects(effects, state, &mut out);
        out
    }

    pub fn project_effects(
        &mut self,
        effects: &[Effect],
        state: &LobbyState,
        out: &mut Vec<Delta>,
    ) {
        for effect in effects {
            self.project_effect(effect, state, out);
        }
    }

    fn project_event(&mut self, event: &ServerEvent, state: &LobbyState, out: &mut Vec<Delta>) {
        use ServerEvent as E;
        match event {
            E::Welcome { .. } => out.push(Delta::Phase(Some(Phase::AwaitingLogin))),
            E::Accepted { .. } => out.push(Delta::Phase(Some(Phase::Loading))),
            _ => {}
        }
        if state.phase != Some(lobby_core::Phase::Ready) {
            return;
        }
        match event {
            E::AddUser { name, .. } => {
                if let Some(user) = state.users.get(name) {
                    let battle_id = state.user_battle.get(name).copied();
                    out.push(Delta::UserAdded(UserView::new(user, battle_id)));
                }
                // Only after the lobby is ready, which is why this is safe:
                // logging in brings seventeen hundred of these at once, and
                // nothing is projected until that flood is over.
                if state.friends.contains(name) {
                    out.push(Delta::Alert {
                        kind: AlertKind::FriendOnline,
                        text: format!("{name} is online"),
                    });
                }
            }
            E::RemoveUser { name } => out.push(Delta::UserRemoved { name: name.clone() }),
            E::ClientStatus { name, status } => out.push(Delta::UserStatus {
                name: name.clone(),
                status: (*status).into(),
            }),
            E::BattleOpened(opened) => {
                if let Some(battle) = state.battles.get(&opened.id) {
                    out.push(Delta::BattleOpened(battle.into()));
                }
            }
            E::BattleClosed { id } => out.push(Delta::BattleClosed { id: *id }),
            E::UpdateBattleInfo {
                id,
                spectator_count,
                locked,
                map_hash,
                map_name,
            } => out.push(Delta::BattleInfo {
                id: *id,
                spectator_count: *spectator_count,
                locked: *locked,
                map_hash: map_hash.clone(),
                map_name: map_name.clone(),
            }),
            E::BattleTitle { id, title } => out.push(Delta::BattleTitle {
                id: *id,
                title: title.clone(),
            }),
            E::BattleTeams { layouts } => {
                out.extend(layouts.iter().map(|(id, l)| Delta::BattleLayout {
                    id: *id,
                    layout: LayoutView {
                        teams: l.teams,
                        team_size: l.team_size,
                    },
                }))
            }
            E::JoinedBattle { id, name, .. } => out.push(Delta::Member {
                id: *id,
                name: name.clone(),
                joined: true,
            }),
            E::LeftBattle { id, name } => out.push(Delta::Member {
                id: *id,
                name: name.clone(),
                joined: false,
            }),
            E::ClientBattleStatus {
                name,
                status,
                team_colour,
            } => out.push(Delta::MemberStatus {
                name: name.clone(),
                status: (*status).into(),
                team_colour: *team_colour,
            }),
            E::AddBot { id, name, .. } | E::UpdateBot { id, name, .. } => out.push(Delta::Bot {
                id: *id,
                name: name.clone(),
                bot: state
                    .battles
                    .get(id)
                    .and_then(|b| b.bots.get(name))
                    .map(BotView::from),
            }),
            E::RemoveBot { id, name } => out.push(Delta::Bot {
                id: *id,
                name: name.clone(),
                bot: None,
            }),
            E::AddStartRect {
                ally_team,
                left,
                top,
                right,
                bottom,
            } => out.push(Delta::StartRect {
                ally_team: *ally_team,
                rect: Some(StartRectView {
                    ally_team: *ally_team,
                    left: *left,
                    top: *top,
                    right: *right,
                    bottom: *bottom,
                }),
            }),
            E::RemoveStartRect { ally_team } => out.push(Delta::StartRect {
                ally_team: *ally_team,
                rect: None,
            }),
            E::SetScriptTags { tags } => out.push(Delta::ScriptTags {
                set: tags.clone(),
                removed: Vec::new(),
            }),
            E::RemoveScriptTags { keys } => out.push(Delta::ScriptTags {
                set: Vec::new(),
                removed: keys.clone(),
            }),
            _ => {}
        }
    }

    fn project_effect(&mut self, effect: &Effect, state: &LobbyState, out: &mut Vec<Delta>) {
        match effect {
            Effect::Joined { .. } => out.push(Delta::MyBattle(
                state.my_battle.as_ref().map(MyBattleView::from),
            )),
            Effect::LeftBattle { .. } => {
                out.push(Delta::MyBattle(None));
                out.push(Delta::GameRunning(None));
            }
            Effect::GameStopped => out.push(Delta::GameRunning(None)),
            Effect::GameInProgress { id, elapsed_secs } => out.push(Delta::GameStartedAgo {
                id: *id,
                seconds: *elapsed_secs,
            }),
            Effect::GameRunning { id, ip, port, .. } => {
                out.push(Delta::GameRunning(Some(GameRunningView {
                    id: *id,
                    ip: ip.clone(),
                    port: *port,
                })))
            }
            Effect::BattleChat {
                from,
                text,
                announcement,
            } => {
                let kind = match () {
                    () if lobby_core::spads::is_machine(text) => ChatKind::Machine,
                    () if *announcement => ChatKind::Announcement,
                    () => ChatKind::Chat,
                };
                let line = self.line(state, BATTLE_ROOM, from, text, kind);
                alert_if_named(&line, out);
                out.push(Delta::Chat(line));
            }
            Effect::PrivateChat { with, from, text } => {
                let machine = lobby_core::spads::is_machine(text);
                // Our own message echoed back is not something to be told
                // about, and neither is a host answering a question we asked
                // it: the reply to `!#JSONRPC status game` is a wall of JSON,
                // and alerting on it puts that wall on screen.
                if !machine && Some(from) != state.me.as_ref() {
                    out.push(Delta::Alert {
                        kind: AlertKind::PrivateMessage,
                        text: format!("{from}: {text}"),
                    });
                }
                let kind = if machine {
                    ChatKind::Machine
                } else {
                    ChatKind::Private
                };
                out.push(Delta::Chat(self.line(
                    state,
                    &private_room(with),
                    from,
                    text,
                    kind,
                )));
            }
            Effect::ChannelChat {
                room,
                from,
                text,
                emote,
            } => {
                let kind = if *emote {
                    ChatKind::Emote
                } else {
                    ChatKind::Chat
                };
                let line = self.line(state, room, from, text, kind);
                alert_if_named(&line, out);
                out.push(Delta::Chat(line));
            }
            Effect::ChannelJoined { room } | Effect::ChannelChanged { room } => {
                out.push(channel_delta(state, room));
            }
            Effect::ChannelLeft { room } => out.push(Delta::Channel {
                name: room.clone(),
                channel: None,
            }),
            Effect::ChannelJoinFailed { room, reason } => out.push(notice(
                NoticeLevel::Warning,
                format!("cannot join {room}: {reason}"),
            )),
            Effect::FriendsChanged => out.push(Delta::Friends(FriendsView::from(state))),
            Effect::BossChanged => {
                out.push(Delta::MyBattle(
                    state.my_battle.as_ref().map(MyBattleView::from),
                ));
            }
            Effect::ServerSaid { text } => out.push(Delta::Chat(self.line(
                state,
                SERVER_ROOM,
                "server",
                text,
                ChatKind::System,
            ))),
            Effect::Rung { by } => out.push(Delta::Alert {
                kind: AlertKind::Ring,
                text: format!("{by} is asking for you"),
            }),
            Effect::ChannelsListed => out.push(Delta::Directory(
                state
                    .directory
                    .iter()
                    .map(|entry| ChannelSummaryView {
                        name: entry.name.clone(),
                        members: entry.members,
                    })
                    .collect(),
            )),
            Effect::VoteChanged => {
                let vote = state.my_battle.as_ref().and_then(|my| my.vote.as_ref());
                // Only a vote opening is worth interrupting for; its tally
                // changing every few seconds is not.
                if let Some(vote) = vote
                    && !self.vote_open
                {
                    out.push(Delta::Alert {
                        kind: AlertKind::Vote,
                        text: format!("a vote opened: {}", vote.command),
                    });
                }
                self.vote_open = vote.is_some();
                out.push(Delta::Vote(vote.map(VoteView::from)));
            }
            Effect::ModOptionsChanged { keys } => {
                let Some(my) = state.my_battle.as_ref() else {
                    return;
                };
                for key in keys {
                    let change = my
                        .history
                        .iter()
                        .rev()
                        .find(|change| &change.key == key)
                        .map(OptionChangeView::from);
                    out.push(Delta::ModOption {
                        key: key.clone(),
                        value: my.modoption(key).to_owned(),
                        change,
                    });
                }
            }
            // The runtime acts on these; the notices are the runtime's own.
            Effect::PrivateHostOffered { .. }
            | Effect::PrivateHostReady { .. }
            | Effect::Hosting { .. } => {}
            Effect::Notice(text) => out.push(notice(NoticeLevel::Info, text.clone())),
            Effect::JoinFailed { reason } => out.push(notice(
                NoticeLevel::Warning,
                format!("join failed: {reason}"),
            )),
            Effect::LoginDenied { reason } => out.push(notice(
                NoticeLevel::Error,
                format!("login denied: {reason}"),
            )),
            Effect::AgreementRequired { .. } => out.push(notice(
                NoticeLevel::Error,
                "the account must accept the user agreement first".into(),
            )),
            // Registration is reported to whoever asked for it, through the
            // command's own reply; a notice as well would say it twice.
            Effect::Registered | Effect::RegistrationDenied { .. } => {}
            Effect::Redirect { host, port } => out.push(notice(
                NoticeLevel::Error,
                format!(
                    "server redirects to {host}:{}",
                    port.map_or("?".into(), |p| p.to_string())
                ),
            )),
            Effect::Disconnected { reason, .. } => out.push(notice(
                NoticeLevel::Error,
                format!("disconnected: {reason}"),
            )),
            Effect::Send(_) | Effect::LoggedIn { .. } | Effect::Ready => {}
        }
    }

    fn line(
        &mut self,
        state: &LobbyState,
        room: &str,
        from: &str,
        text: &str,
        kind: ChatKind,
    ) -> ChatLine {
        self.seq += 1;
        // Our own words are not somebody calling us, and a private message is
        // already addressed to us by being one. Nor is a machine line, which
        // can carry your name without addressing you — SPADS lists the room's
        // bosses inside its `BattleStateChanged` JSON.
        let talking_to_us = Some(from) != state.me.as_deref()
            && !matches!(kind, ChatKind::Private | ChatKind::Machine)
            && state
                .me
                .as_deref()
                .is_some_and(|me| mention::mentions(text, me));
        ChatLine {
            seq: self.seq,
            room: room.to_owned(),
            from: from.to_owned(),
            text: text.to_owned(),
            kind,
            mention: talking_to_us,
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or_default(),
        }
    }
}

/// Raises an alert for a line that named us.
///
/// A private message already alerts on its own; this is for the channel and
/// battle-room lines, where a name goes past in traffic nobody reads in full.
fn alert_if_named(line: &ChatLine, out: &mut Vec<Delta>) {
    if line.mention {
        out.push(Delta::Alert {
            kind: AlertKind::Mention,
            text: format!("{}: {}", line.from, line.text),
        });
    }
}

/// A channel as it now stands, or its removal if we are no longer in it.
fn channel_delta(state: &LobbyState, room: &str) -> Delta {
    Delta::Channel {
        name: room.to_owned(),
        channel: state.channels.get(room).map(|channel| ChannelView {
            name: channel.name.clone(),
            members: channel.members.iter().cloned().collect(),
            topic_author: channel.topic_author.clone(),
        }),
    }
}

fn notice(level: NoticeLevel, text: String) -> Delta {
    Delta::Notice { level, text }
}

#[cfg(test)]
mod tests {
    use lobby_core::Session;
    use spring_protocol::{LoginRequest, RawMessage};

    use super::*;
    use crate::model::UiMessage;

    fn session() -> Session {
        Session::new(
            LoginRequest::new("me", "pw", "test", "h h"),
            vec![],
            "MH".into(),
        )
    }

    /// Reduces `line` and projects it, like the runtime does.
    fn step(s: &mut Session, p: &mut Projector, line: &str) -> Vec<Delta> {
        let event: ServerEvent = RawMessage::parse(line).into();
        let effects = s.handle(event.clone());
        p.project(&event, &effects, &s.state)
    }

    /// Logged in and past the flood, which is when anything is projected.
    fn live() -> (Session, Projector) {
        let mut s = session();
        let mut p = Projector::new();
        for line in ["TASSERVER 0.38 * 8201 0", "ACCEPTED me", "LOGININFOEND"] {
            step(&mut s, &mut p, line);
        }
        (s, p)
    }

    #[test]
    fn a_hosts_json_answer_is_not_something_to_alert_about() {
        let (mut s, mut p) = live();
        // The reply to `!#JSONRPC status game`, which is what asking a host how
        // long its game has been running gets back: a wall of JSON carrying
        // every client in the room. Alerting on it puts that wall on screen.
        let deltas = step(
            &mut s,
            &mut p,
            r#"SAIDPRIVATE Host[EU2][007] !#JSONRPC {"result":{"game":{"clients":[{"Name":"someone"}]}},"id":1}"#,
        );
        assert!(
            !deltas
                .iter()
                .any(|delta| matches!(delta, Delta::Alert { .. })),
            "{deltas:?}"
        );
        let [Delta::Chat(line)] = &deltas[..] else {
            panic!("expected one chat line, got {deltas:?}")
        };
        assert_eq!(line.kind, ChatKind::Machine, "and it is filterable");
    }

    #[test]
    fn an_actual_private_message_still_alerts() {
        let (mut s, mut p) = live();
        let deltas = step(&mut s, &mut p, "SAIDPRIVATE friend are you playing?");
        assert!(deltas.iter().any(|delta| matches!(
            delta,
            Delta::Alert {
                kind: AlertKind::PrivateMessage,
                ..
            }
        )));
    }

    #[test]
    fn a_machine_line_naming_you_is_not_a_mention() {
        let (mut s, mut p) = live();
        // SPADS lists the room's bosses inside its own JSON, so a machine line
        // carries your name without addressing you.
        let deltas = step(
            &mut s,
            &mut p,
            r#"SAIDBATTLEEX host * BarManager|{"BattleStateChanged": {"boss": "me,someone"}}"#,
        );
        assert!(
            !deltas
                .iter()
                .any(|delta| matches!(delta, Delta::Alert { .. })),
            "{deltas:?}"
        );
        for delta in &deltas {
            if let Delta::Chat(line) = delta {
                assert!(!line.mention, "a boss list is not somebody calling you");
            }
        }
    }

    #[test]
    fn silent_during_the_flood_then_live() {
        let mut s = session();
        let mut p = Projector::new();
        assert_eq!(
            step(&mut s, &mut p, "TASSERVER 0.38 * 8201 0"),
            [Delta::Phase(Some(Phase::AwaitingLogin))]
        );
        assert_eq!(
            step(&mut s, &mut p, "ACCEPTED me"),
            [Delta::Phase(Some(Phase::Loading))]
        );
        assert!(step(&mut s, &mut p, "ADDUSER alice SE 1 LuaLobby Chobby").is_empty());
        assert!(
            step(
                &mut s,
                &mut p,
                "BATTLEOPENED 5 0 0 host 1.2.3.4 8452 16 0 0 h R\tv\tm\tt\tg"
            )
            .is_empty()
        );
        assert!(step(&mut s, &mut p, "LOGININFOEND").is_empty());

        let [Delta::UserAdded(user)] = &step(&mut s, &mut p, "ADDUSER bob DE 2 x")[..] else {
            panic!("expected UserAdded")
        };
        assert_eq!(user.name, "bob");
        assert_eq!(
            step(&mut s, &mut p, "JOINEDBATTLE 5 bob"),
            [Delta::Member {
                id: 5,
                name: "bob".into(),
                joined: true
            }]
        );
        let deltas = step(&mut s, &mut p, "SAIDBATTLEEX host * hi");
        assert!(matches!(
            &deltas[..],
            [Delta::Chat(ChatLine {
                seq: 1,
                kind: ChatKind::Announcement,
                ..
            })]
        ));
    }

    #[test]
    fn json_shape_is_tag_and_data() {
        let value = serde_json::to_value(UiMessage::Deltas(vec![Delta::UserStatus {
            name: "bob".into(),
            status: spring_protocol::UserStatus::from_bits(1).into(),
        }]))
        .unwrap();
        assert_eq!(value["type"], "deltas");
        assert_eq!(value["data"][0]["type"], "userStatus");
        assert_eq!(value["data"][0]["data"]["status"]["inGame"], true);
    }
}
