use std::collections::{BTreeMap, BTreeSet, HashMap};

use spring_protocol::{BattleOpened, BattleStatus, TeamLayout, UserStatus};

use crate::spads::VoteState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    AwaitingLogin,
    /// `ACCEPTED` received; the login flood is streaming in.
    Loading,
    /// `LOGININFOEND` received; state is complete and live.
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub country: String,
    pub user_id: Option<u64>,
    pub lobby_client: String,
    pub status: UserStatus,
    /// `CLIENTBATTLESTATUS`; the server only sends it for members of our own room.
    pub battle_status: Option<BattleStatus>,
}

/// The prefix `SETSCRIPTTAGS` puts on modoptions.
const MODOPTION: &str = "game/modoptions/";

/// A modoption changing while we watched — the raw material for a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionChange {
    pub seq: u64,
    /// Without the `game/modoptions/` prefix, e.g. `tweakdefs1`.
    pub key: String,
    pub from: String,
    pub to: String,
    /// Who did it, once the host announces it.
    pub by: Option<String>,
}

/// The room we are in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyBattle {
    /// Who SPADS says is bossing this room, when it has said.
    pub boss: Option<String>,
    pub id: u32,
    /// From the `JOINBATTLE` reply; identifies the game archive.
    pub game_hash: String,
    /// Secret the engine presents to the host when connecting.
    pub script_password: String,
    /// The room's script tags from `SETSCRIPTTAGS`, lowercase keys (`game/modoptions/tweakdefs`, `game/hosttype`, …).
    pub script_tags: BTreeMap<String, String>,
    /// The vote the host is running, if any.
    pub vote: Option<VoteState>,
    /// Modoption changes seen this session, oldest first.
    pub history: Vec<OptionChange>,
    next_seq: u64,
}

impl MyBattle {
    pub fn new(id: u32, game_hash: String, script_password: String) -> Self {
        Self {
            boss: None,
            id,
            game_hash,
            script_password,
            script_tags: BTreeMap::new(),
            vote: None,
            history: Vec::new(),
            next_seq: 0,
        }
    }

    /// `game/modoptions/<key>` values, keyed without the prefix.
    pub fn modoptions(&self) -> impl Iterator<Item = (&str, &str)> {
        self.script_tags
            .iter()
            .filter_map(|(key, value)| Some((key.strip_prefix(MODOPTION)?, value.as_str())))
    }

    pub fn modoption(&self, key: &str) -> &str {
        self.script_tags
            .get(&format!("{MODOPTION}{key}"))
            .map_or("", String::as_str)
    }

    /// Applies `SETSCRIPTTAGS`, recording every modoption that really changed.
    /// Returns those keys, without the prefix.
    pub fn set_script_tags(&mut self, tags: Vec<(String, String)>) -> Vec<String> {
        let mut changed = Vec::new();
        for (key, value) in tags {
            let previous = self.script_tags.insert(key.clone(), value.clone());
            let Some(option) = key.strip_prefix(MODOPTION) else {
                continue;
            };
            let from = previous.unwrap_or_default();
            if from != value {
                self.record(option.to_owned(), from, value, None);
                changed.push(option.to_owned());
            }
        }
        changed
    }

    /// The host's `Battle setting changed by …`. It arrives for clears too,
    /// where SPADS sends no `SETSCRIPTTAGS` at all (`spads.pl:2625-2628`), and
    /// it is the only place the author's name appears.
    pub fn setting_changed(&mut self, key: String, value: String, by: String) -> bool {
        let from = self.modoption(&key).to_owned();
        if let Some(last) = self.history.last_mut()
            && last.key == key
            && last.to == value
        {
            last.by = Some(by);
            return false;
        }
        self.script_tags
            .insert(format!("{MODOPTION}{key}"), value.clone());
        let changed = from != value;
        if changed {
            self.record(key, from, value, Some(by));
        }
        changed
    }

    fn record(&mut self, key: String, from: String, to: String, by: Option<String>) {
        self.next_seq += 1;
        self.history.push(OptionChange {
            seq: self.next_seq,
            key,
            from,
            to,
            by,
        });
    }
}

/// An AI in the room (`ADDBOT`): owned by a member, plays on a team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bot {
    pub name: String,
    pub owner: String,
    pub status: BattleStatus,
    pub team_colour: u32,
    pub ai: String,
}

/// An ally team's start box (`ADDSTARTRECT`), in map fractions out of 200.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartRect {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battle {
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
    /// Everyone in the room, host bot and spectators included. The founder is
    /// never announced with `JOINEDBATTLE`; `BATTLEOPENED` implies it.
    pub members: BTreeSet<String>,
    pub spectator_count: u32,
    /// From `s.battle.teams`; absent until the server sends it.
    pub layout: Option<TeamLayout>,
    /// Only known for our own room; cleared when we leave it.
    pub bots: BTreeMap<String, Bot>,
    /// Keyed by ally team; only known for our own room.
    pub start_rects: BTreeMap<u8, StartRect>,
}

impl Battle {
    fn from_opened(b: BattleOpened) -> Self {
        Self {
            id: b.id,
            members: BTreeSet::from([b.founder.clone()]),
            bots: BTreeMap::new(),
            start_rects: BTreeMap::new(),
            founder: b.founder,
            ip: b.ip,
            port: b.port,
            max_players: b.max_players,
            passworded: b.passworded,
            locked: false,
            map_hash: b.map_hash,
            map_name: b.map_name,
            engine_name: b.engine_name,
            engine_version: b.engine_version,
            title: b.title,
            game_name: b.game_name,
            // The founder spectates its own room until UPDATEBATTLEINFO says otherwise.
            spectator_count: 1,
            layout: None,
        }
    }

    /// Members minus spectators; the host bot counts itself as a spectator.
    pub fn player_count(&self) -> usize {
        self.members
            .len()
            .saturating_sub(self.spectator_count as usize)
    }
}

#[derive(Debug, Default)]
pub struct LobbyState {
    pub phase: Option<Phase>,
    pub me: Option<String>,
    pub users: HashMap<String, User>,
    pub battles: HashMap<u32, Battle>,
    /// Which battle each user is in, for O(1) leave/cleanup.
    pub user_battle: HashMap<String, u32>,
    pub my_battle: Option<MyBattle>,
    pub motd: Vec<String>,
    pub comp_flags: Vec<String>,
    /// Channels we are in, by name.
    pub channels: BTreeMap<String, Channel>,
    /// The server's channel directory, from the last `CHANNELS` request.
    pub directory: Vec<ChannelSummary>,
    /// Who we are friends with.
    pub friends: BTreeSet<String>,
    /// Who has asked to be, and is waiting on an answer.
    pub friend_requests: BTreeSet<String>,
    /// Who the server no longer relays anything from.
    pub ignored: BTreeSet<String>,
}

/// A channel we have joined.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Channel {
    pub name: String,
    /// Sorted, so the roster does not reshuffle between `CLIENTS` batches.
    pub members: BTreeSet<String>,
    /// Who last set the topic. teiserver sends no topic text with it
    /// (`spring_out.ex:438`), so there is nothing else to keep.
    pub topic_author: Option<String>,
}

/// One line of the server's channel directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSummary {
    pub name: String,
    pub members: u32,
}

impl LobbyState {
    pub fn add_user(
        &mut self,
        name: String,
        country: String,
        user_id: Option<u64>,
        lobby_client: String,
    ) {
        let user = User {
            name: name.clone(),
            country,
            user_id,
            lobby_client,
            status: UserStatus::default(),
            battle_status: None,
        };
        self.users.insert(name, user);
    }

    pub fn remove_user(&mut self, name: &str) {
        self.users.remove(name);
        if let Some(id) = self.user_battle.remove(name)
            && let Some(battle) = self.battles.get_mut(&id)
        {
            battle.members.remove(name);
        }
    }

    pub fn set_status(&mut self, name: &str, status: UserStatus) {
        match self.users.get_mut(name) {
            Some(user) => user.status = status,
            None => tracing::debug!(name, "CLIENTSTATUS for unknown user"),
        }
    }

    pub fn open_battle(&mut self, opened: BattleOpened) {
        self.user_battle.insert(opened.founder.clone(), opened.id);
        self.battles.insert(opened.id, Battle::from_opened(opened));
    }

    pub fn close_battle(&mut self, id: u32) {
        if let Some(battle) = self.battles.remove(&id) {
            for member in battle.members {
                self.user_battle.remove(&member);
            }
        }
    }

    pub fn join_battle(&mut self, id: u32, name: String) {
        if let Some(battle) = self.battles.get_mut(&id) {
            battle.members.insert(name.clone());
            self.user_battle.insert(name, id);
        } else {
            tracing::debug!(id, name, "JOINEDBATTLE for unknown battle");
        }
    }

    pub fn leave_battle(&mut self, id: u32, name: &str) {
        if let Some(battle) = self.battles.get_mut(&id) {
            battle.members.remove(name);
        }
        self.user_battle.remove(name);
    }

    /// The battle we are in, if it is still on the list.
    pub fn my_room_mut(&mut self) -> Option<&mut Battle> {
        let id = self.my_battle.as_ref()?.id;
        self.battles.get_mut(&id)
    }

    /// Drops the room-only details (bots, start boxes) the server stops updating once we leave.
    pub fn forget_room_details(&mut self, id: u32) {
        if let Some(battle) = self.battles.get_mut(&id) {
            battle.bots.clear();
            battle.start_rects.clear();
        }
    }

    /// Battles ordered by player count, most populated first.
    pub fn battles_by_players(&self) -> Vec<&Battle> {
        let mut battles: Vec<&Battle> = self.battles.values().collect();
        battles.sort_by(|a, b| {
            b.player_count()
                .cmp(&a.player_count())
                .then_with(|| a.id.cmp(&b.id))
        });
        battles
    }
}
