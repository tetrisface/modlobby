use std::collections::{BTreeSet, HashMap};

use spring_protocol::{BattleOpened, BattleStatus, TeamLayout, UserStatus};

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

/// The room we are in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyBattle {
    pub id: u32,
    /// From the `JOINBATTLE` reply; identifies the game archive.
    pub game_hash: String,
    /// Secret the engine presents to the host when connecting.
    pub script_password: String,
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
}

impl Battle {
    fn from_opened(b: BattleOpened) -> Self {
        Self {
            id: b.id,
            members: BTreeSet::from([b.founder.clone()]),
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
