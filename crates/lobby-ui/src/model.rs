//! Serialisable views of the core state. Field names are camelCase on the wire;
//! integers that JSON carries as numbers are typed `number` for TypeScript.

use std::collections::BTreeMap;

use lobby_core::{
    Battle, Bot, LobbyState, MyBattle, OptionChange, Proposal, StartRect, User, VoteState,
};
use serde::{Deserialize, Serialize};
use spring_protocol::{BattleStatus, Sync, UserStatus};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Phase {
    Connecting,
    AwaitingLogin,
    Loading,
    Ready,
}

impl From<lobby_core::Phase> for Phase {
    fn from(phase: lobby_core::Phase) -> Self {
        match phase {
            lobby_core::Phase::Connecting => Self::Connecting,
            lobby_core::Phase::AwaitingLogin => Self::AwaitingLogin,
            lobby_core::Phase::Loading => Self::Loading,
            lobby_core::Phase::Ready => Self::Ready,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SyncView {
    Bot,
    Synced,
    Unsynced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BattleStatusView {
    pub ready: bool,
    pub team: u8,
    pub ally_team: u8,
    pub player: bool,
    pub handicap: u8,
    pub sync: SyncView,
    pub side: u8,
}

impl From<BattleStatus> for BattleStatusView {
    fn from(s: BattleStatus) -> Self {
        Self {
            ready: s.ready,
            team: s.team,
            ally_team: s.ally_team,
            player: s.player,
            handicap: s.handicap,
            sync: match s.sync {
                Sync::Bot => SyncView::Bot,
                Sync::Synced => SyncView::Synced,
                Sync::Unsynced => SyncView::Unsynced,
            },
            side: s.side,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UserStatusView {
    pub in_game: bool,
    pub away: bool,
    pub rank: u8,
    pub moderator: bool,
    pub bot: bool,
}

impl From<UserStatus> for UserStatusView {
    fn from(s: UserStatus) -> Self {
        Self {
            in_game: s.in_game,
            away: s.away,
            rank: s.rank,
            moderator: s.moderator,
            bot: s.bot,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UserView {
    pub name: String,
    pub country: String,
    #[ts(type = "number | null")]
    pub user_id: Option<u64>,
    pub lobby_client: String,
    pub status: UserStatusView,
    pub battle_status: Option<BattleStatusView>,
    /// The room the user is in.
    pub battle_id: Option<u32>,
}

impl UserView {
    pub fn new(user: &User, battle_id: Option<u32>) -> Self {
        Self {
            name: user.name.clone(),
            country: user.country.clone(),
            user_id: user.user_id,
            lobby_client: user.lobby_client.clone(),
            status: user.status.into(),
            battle_status: user.battle_status.map(Into::into),
            battle_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BotView {
    pub name: String,
    pub owner: String,
    pub status: BattleStatusView,
    pub team_colour: u32,
    pub ai: String,
}

impl From<&Bot> for BotView {
    fn from(bot: &Bot) -> Self {
        Self {
            name: bot.name.clone(),
            owner: bot.owner.clone(),
            status: bot.status.into(),
            team_colour: bot.team_colour,
            ai: bot.ai.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartRectView {
    pub ally_team: u8,
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

impl StartRectView {
    pub fn new(ally_team: u8, rect: StartRect) -> Self {
        Self {
            ally_team,
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LayoutView {
    pub teams: u32,
    pub team_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BattleView {
    pub id: u32,
    pub founder: String,
    pub ip: String,
    pub port: u16,
    pub max_players: u32,
    pub passworded: bool,
    pub locked: bool,
    pub map_hash: String,
    pub map_name: String,
    pub engine_name: String,
    pub engine_version: String,
    pub title: String,
    pub game_name: String,
    /// Sorted; includes the host bot and spectators.
    pub members: Vec<String>,
    pub spectator_count: u32,
    pub player_count: u32,
    pub layout: Option<LayoutView>,
    pub bots: Vec<BotView>,
    pub start_rects: Vec<StartRectView>,
}

impl From<&Battle> for BattleView {
    fn from(b: &Battle) -> Self {
        Self {
            id: b.id,
            founder: b.founder.clone(),
            ip: b.ip.clone(),
            port: b.port,
            max_players: b.max_players,
            passworded: b.passworded,
            locked: b.locked,
            map_hash: b.map_hash.clone(),
            map_name: b.map_name.clone(),
            engine_name: b.engine_name.clone(),
            engine_version: b.engine_version.clone(),
            title: b.title.clone(),
            game_name: b.game_name.clone(),
            members: b.members.iter().cloned().collect(),
            spectator_count: b.spectator_count,
            player_count: b.player_count() as u32,
            layout: b.layout.map(|l| LayoutView {
                teams: l.teams,
                team_size: l.team_size,
            }),
            bots: b.bots.values().map(BotView::from).collect(),
            start_rects: b
                .start_rects
                .iter()
                .map(|(ally, rect)| StartRectView::new(*ally, *rect))
                .collect(),
        }
    }
}

impl From<&LobbyState> for FriendsView {
    fn from(state: &LobbyState) -> Self {
        Self {
            friends: state.friends.iter().cloned().collect(),
            requests: state.friend_requests.iter().cloned().collect(),
        }
    }
}

/// What a vote would do, when the room can tell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum ProposalView {
    /// A modoption change — this is what the tweak diff hangs off.
    SetOption {
        key: String,
        value: String,
    },
    Other,
}

impl From<&Proposal> for ProposalView {
    fn from(proposal: &Proposal) -> Self {
        match proposal {
            Proposal::SetOption { key, value } => ProposalView::SetOption {
                key: key.clone(),
                value: value.clone(),
            },
            Proposal::Other => ProposalView::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VoteView {
    pub command: String,
    pub by: Option<String>,
    pub proposal: ProposalView,
    pub yes: u32,
    pub yes_needed: u32,
    pub no: u32,
    pub no_needed: u32,
    pub remaining_secs: u32,
}

impl From<&VoteState> for VoteView {
    fn from(vote: &VoteState) -> Self {
        Self {
            command: vote.command.clone(),
            by: vote.by.clone(),
            proposal: (&vote.proposal).into(),
            yes: vote.yes,
            yes_needed: vote.yes_needed,
            no: vote.no,
            no_needed: vote.no_needed,
            remaining_secs: vote.remaining_secs,
        }
    }
}

/// A modoption that changed while we watched — one side of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OptionChangeView {
    #[ts(type = "number")]
    pub seq: u64,
    pub key: String,
    pub from: String,
    pub to: String,
    pub by: Option<String>,
}

impl From<&OptionChange> for OptionChangeView {
    fn from(change: &OptionChange) -> Self {
        Self {
            seq: change.seq,
            key: change.key.clone(),
            from: change.from.clone(),
            to: change.to.clone(),
            by: change.by.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MyBattleView {
    pub id: u32,
    pub game_hash: String,
    /// Lowercase script-tag keys (`game/modoptions/tweakdefs`, `game/hosttype`, …).
    pub script_tags: BTreeMap<String, String>,
    pub vote: Option<VoteView>,
    /// Modoption changes seen this session, oldest first.
    pub history: Vec<OptionChangeView>,
}

impl From<&MyBattle> for MyBattleView {
    fn from(my: &MyBattle) -> Self {
        Self {
            id: my.id,
            game_hash: my.game_hash.clone(),
            script_tags: my.script_tags.clone(),
            vote: my.vote.as_ref().map(VoteView::from),
            history: my.history.iter().map(OptionChangeView::from).collect(),
        }
    }
}

/// The room's game is running; the script password stays in the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GameRunningView {
    pub id: u32,
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum EngineStatus {
    Idle,
    Running,
    Exited { code: Option<i32> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Snapshot {
    pub phase: Option<Phase>,
    pub me: Option<String>,
    pub users: Vec<UserView>,
    pub battles: Vec<BattleView>,
    pub my_battle: Option<MyBattleView>,
    pub game_running: Option<GameRunningView>,
    pub engine: EngineStatus,
    /// Channels we are in. Chat lines are not replayed — a reload keeps
    /// whichever backlog the front end still holds — but membership is, so the
    /// channel list is right the moment the window comes back.
    pub channels: Vec<ChannelView>,
    pub friends: FriendsView,
}

/// Who we are friends with, and who is waiting on an answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FriendsView {
    pub friends: Vec<String>,
    pub requests: Vec<String>,
}

impl Snapshot {
    pub fn disconnected() -> Self {
        Self {
            phase: None,
            me: None,
            users: Vec::new(),
            battles: Vec::new(),
            my_battle: None,
            game_running: None,
            engine: EngineStatus::Idle,
            channels: Vec::new(),
            friends: FriendsView::default(),
        }
    }

    /// Users sorted by name and battles by id, so two snapshots of one state are equal.
    pub fn from_state(
        state: &LobbyState,
        game_running: Option<GameRunningView>,
        engine: EngineStatus,
    ) -> Self {
        let mut users: Vec<UserView> = state
            .users
            .values()
            .map(|u| UserView::new(u, state.user_battle.get(&u.name).copied()))
            .collect();
        users.sort_by(|a, b| a.name.cmp(&b.name));
        let mut battles: Vec<BattleView> = state.battles.values().map(BattleView::from).collect();
        battles.sort_by_key(|b| b.id);
        Self {
            phase: state.phase.map(Into::into),
            me: state.me.clone(),
            users,
            battles,
            my_battle: state.my_battle.as_ref().map(MyBattleView::from),
            game_running,
            engine,
            channels: state
                .channels
                .values()
                .map(|channel| ChannelView {
                    name: channel.name.clone(),
                    members: channel.members.iter().cloned().collect(),
                    topic_author: channel.topic_author.clone(),
                })
                .collect(),
            friends: FriendsView::from(state),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChatKind {
    Chat,
    /// `SAIDBATTLEEX`: host announcements and `/me` lines.
    Announcement,
    Private,
    /// `SAIDEX`: an emote, written as an action rather than speech.
    Emote,
    /// Said by the app rather than by anyone on the server.
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatLine {
    #[ts(type = "number")]
    pub seq: u64,
    /// Where this was said. `#battle` is the room we are in, `@name` is a
    /// private conversation, and anything else is a channel — which cannot
    /// collide, because teiserver only accepts `\w+` as a channel name.
    pub room: String,
    pub from: String,
    pub text: String,
    pub kind: ChatKind,
}

/// The room key for the battle we are in.
pub const BATTLE_ROOM: &str = "#battle";

/// The room key for a private conversation with someone.
pub fn private_room(user: &str) -> String {
    format!("@{user}")
}

/// A channel we are in, as the front end needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChannelView {
    pub name: String,
    pub members: Vec<String>,
    pub topic_author: Option<String>,
}

/// One line of the server's channel directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChannelSummaryView {
    pub name: String,
    pub members: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// One change to apply to the mirrored state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Delta {
    Phase(Option<Phase>),
    UserAdded(UserView),
    UserRemoved {
        name: String,
    },
    UserStatus {
        name: String,
        status: UserStatusView,
    },
    BattleOpened(BattleView),
    BattleClosed {
        id: u32,
    },
    BattleInfo {
        id: u32,
        spectator_count: u32,
        locked: bool,
        map_hash: String,
        map_name: String,
    },
    BattleTitle {
        id: u32,
        title: String,
    },
    BattleLayout {
        id: u32,
        layout: LayoutView,
    },
    Member {
        id: u32,
        name: String,
        joined: bool,
    },
    MemberStatus {
        name: String,
        status: BattleStatusView,
        team_colour: u32,
    },
    /// `bot == None` removes it.
    Bot {
        id: u32,
        name: String,
        bot: Option<BotView>,
    },
    /// `rect == None` removes it.
    StartRect {
        ally_team: u8,
        rect: Option<StartRectView>,
    },
    ScriptTags {
        set: Vec<(String, String)>,
        removed: Vec<String>,
    },
    /// One modoption's current value, plus the change that produced it.
    ModOption {
        key: String,
        value: String,
        change: Option<OptionChangeView>,
    },
    Vote(Option<VoteView>),
    /// Whether the room's engine, game and map are installed here.
    Content {
        engine: bool,
        game: bool,
        map: bool,
    },
    MyBattle(Option<MyBattleView>),
    GameRunning(Option<GameRunningView>),
    Engine(EngineStatus),
    Chat(ChatLine),
    /// A channel we are in was added, changed, or removed (`None`).
    Channel {
        name: String,
        channel: Option<ChannelView>,
    },
    /// The server's channel directory, replaced whole.
    Directory(Vec<ChannelSummaryView>),
    /// The friend list and pending requests, replaced whole.
    Friends(FriendsView),
    Notice {
        level: NoticeLevel,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
#[ts(export)]
pub enum UiMessage {
    /// Boxed: a snapshot dwarfs a delta batch, and this enum is moved per message.
    Snapshot(Box<Snapshot>),
    Deltas(Vec<Delta>),
}
