//! Turns one reduced server event (plus the effects it produced) into deltas.
//! Derived from the event and the state *after* reduction, so the core keeps
//! no dirty tracking. Nothing but phase changes is emitted before the lobby is
//! ready: the runtime sends a [`Snapshot`](crate::Snapshot) then.

use lobby_core::{Effect, LobbyState};
use spring_protocol::ServerEvent;

use crate::model::{
    BotView, ChatKind, ChatLine, Delta, GameRunningView, LayoutView, MyBattleView, NoticeLevel,
    OptionChangeView, Phase, StartRectView, UserView, VoteView,
};

#[derive(Debug, Default)]
pub struct Projector {
    seq: u64,
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
                let kind = if *announcement {
                    ChatKind::Announcement
                } else {
                    ChatKind::Chat
                };
                out.push(Delta::Chat(self.line(from, text, kind)));
            }
            Effect::PrivateChat { from, text } => {
                out.push(Delta::Chat(self.line(from, text, ChatKind::Private)))
            }
            Effect::VoteChanged => {
                let vote = state.my_battle.as_ref().and_then(|my| my.vote.as_ref());
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

    fn line(&mut self, from: &str, text: &str, kind: ChatKind) -> ChatLine {
        self.seq += 1;
        ChatLine {
            seq: self.seq,
            from: from.to_owned(),
            text: text.to_owned(),
            kind,
        }
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
