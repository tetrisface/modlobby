//! A battle room with no server behind it.
//!
//! The same room the lobby draws — teams, seats, AIs, BAR's modoptions, start
//! boxes — held here and changed by calling methods rather than by asking a
//! host. It projects into [`SkirmishView`], which is `BattleView` and
//! `MyBattleView` verbatim, so the front end cannot tell the two kinds of room
//! apart and a field added to one has to be answered for the other.
//!
//! Pure: no I/O, no clock, no engine. What the runtime does with a launched
//! game, and what content is on this machine, are its business; this decides
//! only what the room *is*.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use lobby_ui::{
    BattleStatusView, BattleView, BotView, LayoutView, MyBattleView, OptionChangeView,
    SkirmishView, SyncView, UserStatusView, UserView,
};

pub mod command;
pub mod script;

pub use command::{Outcome, command};

/// The prefix `SETSCRIPTTAGS` puts on modoptions, kept so the keys here are the
/// keys the online room has and every reader of them works unchanged.
const MODOPTION: &str = "game/modoptions/";

/// Nobody assigned this room a number, so it keeps the one that says so.
pub const ROOM_ID: u32 = 0;

/// What the engine will run without being told otherwise.
const DEFAULT_TITLE: &str = "Skirmish";

/// Colours the engine can tell apart at a glance, as `0xBBGGRR`. The same six
/// the seat bar offers online, so a room looks the same either way.
pub const COLOURS: [u32; 6] = [0x4b73f2, 0x3fd07f, 0x2fb8f0, 0x9e5ce8, 0x50a0ff, 0x8fd04b];

/// Where a participant sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    pub team: u8,
    pub ally_team: u8,
    /// 0 Armada, 1 Cortex, 2 Random, 3 Legion, as the lobby numbers them.
    pub side: u8,
    pub handicap: u8,
}

impl Seat {
    pub fn new(team: u8, ally_team: u8) -> Self {
        Self {
            team,
            ally_team,
            side: 0,
            handicap: 0,
        }
    }
}

/// An AI in the room. `ai` is what the engine is asked for — an engine AI's
/// directory, or the name a game's `luaai.lua` declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ai {
    pub name: String,
    pub ai: String,
    pub seat: Seat,
    pub colour: u32,
}

/// One thing to do to the room.
///
/// Carried across the actor boundary and over the wire to the front end, so
/// every variant is a struct: an internally tagged enum is what gives
/// TypeScript `{ type: 'setOption', key, value }` rather than a nesting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Act {
    /// An empty value clears the option rather than emptying it.
    SetOption {
        key: String,
        value: String,
    },
    SetTitle {
        title: String,
    },
    SetMap {
        map: String,
    },
    SetGame {
        game: String,
    },
    SetEngine {
        engine: String,
    },
    TakeSeat {
        team: u8,
        ally_team: u8,
    },
    ReleaseSeat,
    SetSide {
        side: u8,
    },
    AddBot {
        name: String,
        ai: String,
        team: u8,
        ally_team: u8,
        colour: u32,
    },
    RemoveBot {
        name: String,
    },
    /// A line from the console under the roster.
    Say {
        text: String,
    },
}

/// A room of one's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    pub title: String,
    /// What the player appears as. An account's name when there is one, and
    /// something rather than nothing when there is not.
    pub player: String,
    pub game: String,
    /// The map's spring name, not its archive file name.
    pub map: String,
    pub engine: String,
    /// Script tags under the same keys the online room uses.
    script_tags: BTreeMap<String, String>,
    /// Where the player sits, or `None` while watching.
    seat: Option<Seat>,
    ais: Vec<Ai>,
    /// What has been changed in this room, oldest first — the same record the
    /// tweak pane and the start-box history read online.
    history: Vec<OptionChangeView>,
    next_seq: u64,
    layout: Option<LayoutView>,
}

impl Room {
    /// A room built from whatever this machine has, with the player in seat one.
    pub fn new(
        player: impl Into<String>,
        game: impl Into<String>,
        map: impl Into<String>,
        engine: impl Into<String>,
    ) -> Self {
        Self {
            title: DEFAULT_TITLE.to_owned(),
            player: player.into(),
            game: game.into(),
            map: map.into(),
            engine: engine.into(),
            script_tags: BTreeMap::new(),
            seat: Some(Seat::new(0, 0)),
            ais: Vec::new(),
            history: Vec::new(),
            next_seq: 0,
            layout: None,
        }
    }

    // --- what is in it ----------------------------------------------------

    pub fn seat(&self) -> Option<Seat> {
        self.seat
    }

    pub fn ais(&self) -> &[Ai] {
        &self.ais
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

    /// Every ally team with somebody on it, in order.
    pub fn ally_teams(&self) -> Vec<u8> {
        let mut used: Vec<u8> = self
            .seat
            .iter()
            .map(|seat| seat.ally_team)
            .chain(self.ais.iter().map(|ai| ai.seat.ally_team))
            .collect();
        used.sort_unstable();
        used.dedup();
        used
    }

    /// Where a new AI goes: a side of its own, which is what makes the first
    /// one an opponent rather than a team-mate. Moving it afterwards is one
    /// click, and guessing "with you" would be wrong far more often.
    pub fn free_ally(&self) -> u8 {
        let used = self.ally_teams();
        (0u8..=u8::MAX)
            .find(|ally| !used.contains(ally))
            .unwrap_or(0)
    }

    /// The room's shape, falling back to what is actually on the field.
    pub fn layout_teams(&self) -> u32 {
        self.layout
            .map_or_else(|| self.ally_teams().len().max(2) as u32, |l| l.teams)
    }

    pub fn layout_size(&self) -> u32 {
        self.layout.map_or(1, |l| l.team_size)
    }

    /// The lowest team number nobody holds.
    pub fn free_team(&self) -> u8 {
        let mut held: Vec<u8> = self
            .seat
            .iter()
            .map(|seat| seat.team)
            .chain(self.ais.iter().map(|ai| ai.seat.team))
            .collect();
        held.sort_unstable();
        held.into_iter()
            .enumerate()
            .find(|(want, held)| *held != *want as u8)
            .map_or_else(
                || (self.ais.len() + usize::from(self.seat.is_some())) as u8,
                |(want, _)| want as u8,
            )
    }

    /// A name no AI in the room already has: `BARb`, then `BARb2`.
    pub fn unused_name(&self, base: &str) -> String {
        if !self.ais.iter().any(|ai| ai.name == base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base}{n}"))
            .find(|name| !self.ais.iter().any(|ai| &ai.name == name))
            .unwrap_or_else(|| base.to_owned())
    }

    // --- changing it ------------------------------------------------------

    /// Sets a modoption and records who did it, which is always the one person
    /// here. Returns whether it changed anything.
    pub fn set_option(&mut self, key: &str, value: &str) -> bool {
        let from = self.modoption(key).to_owned();
        if from == value {
            return false;
        }
        self.script_tags
            .insert(format!("{MODOPTION}{key}"), value.to_owned());
        self.next_seq += 1;
        self.history.push(OptionChangeView {
            seq: self.next_seq,
            key: key.to_owned(),
            from,
            to: value.to_owned(),
            by: Some(self.player.clone()),
        });
        true
    }

    /// Clears a modoption, which is not the same as setting it to nothing:
    /// what is absent falls back to the game's own default.
    pub fn clear_option(&mut self, key: &str) -> bool {
        let held = self.script_tags.remove(&format!("{MODOPTION}{key}"));
        let Some(from) = held else {
            return false;
        };
        self.next_seq += 1;
        self.history.push(OptionChangeView {
            seq: self.next_seq,
            key: key.to_owned(),
            from,
            to: String::new(),
            by: Some(self.player.clone()),
        });
        true
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_owned();
    }

    pub fn set_map(&mut self, map: &str) {
        self.map = map.to_owned();
    }

    pub fn set_game(&mut self, game: &str) {
        self.game = game.to_owned();
    }

    pub fn set_engine(&mut self, engine: &str) {
        self.engine = engine.to_owned();
    }

    pub fn set_layout(&mut self, teams: u32, team_size: u32) {
        self.layout = Some(LayoutView { teams, team_size });
    }

    /// Sits down, or moves. Taking a seat clears ready, as it does online.
    pub fn take_seat(&mut self, team: u8, ally_team: u8) {
        let side = self.seat.map_or(0, |held| held.side);
        let handicap = self.seat.map_or(0, |held| held.handicap);
        self.seat = Some(Seat {
            team,
            ally_team,
            side,
            handicap,
        });
    }

    pub fn release_seat(&mut self) {
        self.seat = None;
    }

    pub fn set_side(&mut self, side: u8) {
        if let Some(seat) = self.seat.as_mut() {
            seat.side = side;
        }
    }

    /// Adds an AI. A name already in the room is refused rather than silently
    /// renamed: the caller picked it, and two AIs with one name is a script
    /// the engine will not run.
    pub fn add_ai(&mut self, name: &str, ai: &str, team: u8, ally_team: u8, colour: u32) -> bool {
        if name.is_empty() || self.ais.iter().any(|held| held.name == name) {
            return false;
        }
        self.ais.push(Ai {
            name: name.to_owned(),
            ai: ai.to_owned(),
            seat: Seat::new(team, ally_team),
            colour,
        });
        true
    }

    pub fn remove_ai(&mut self, name: &str) -> bool {
        let before = self.ais.len();
        self.ais.retain(|ai| ai.name != name);
        self.ais.len() != before
    }

    /// One distinct colour per ally team, which is what `!fixColors` does
    /// online and what makes a screenshot readable.
    pub fn fix_colours(&mut self) {
        let teams = self.ally_teams();
        for ai in &mut self.ais {
            let place = teams
                .iter()
                .position(|ally| *ally == ai.seat.ally_team)
                .unwrap_or(0);
            ai.colour = COLOURS[place % COLOURS.len()];
        }
    }

    /// Does one thing to the room and says what it was.
    ///
    /// Every way the room can change goes through here, so the log below the
    /// roster is a complete account of what happened to it — and so the
    /// runtime has one place to decide what a change is worth telling the
    /// front end about.
    pub fn act(&mut self, act: Act) -> Outcome {
        match act {
            Act::SetOption { key, value } if value.is_empty() => {
                if self.clear_option(&key) {
                    Outcome::Did(format!("{key} cleared"))
                } else {
                    Outcome::Nothing
                }
            }
            Act::SetOption { key, value } => {
                if self.set_option(&key, &value) {
                    Outcome::Did(format!("{key} = {value}"))
                } else {
                    Outcome::Nothing
                }
            }
            Act::SetTitle { title } => {
                self.set_title(&title);
                Outcome::Did(format!("room is called {title}"))
            }
            Act::SetMap { map } => {
                self.set_map(&map);
                Outcome::Did(format!("map is {map}"))
            }
            Act::SetGame { game } => {
                self.set_game(&game);
                Outcome::Did(format!("game is {game}"))
            }
            Act::SetEngine { engine } => {
                self.set_engine(&engine);
                Outcome::Did(format!("engine is {engine}"))
            }
            Act::TakeSeat { team, ally_team } => {
                self.take_seat(team, ally_team);
                Outcome::Did(format!("you are on team {}", ally_team + 1))
            }
            Act::ReleaseSeat => {
                self.release_seat();
                Outcome::Did("you are watching".to_owned())
            }
            Act::SetSide { side } => {
                self.set_side(side);
                Outcome::Did(format!("your faction is {side}"))
            }
            Act::AddBot {
                name,
                ai,
                team,
                ally_team,
                colour,
            } => {
                if self.add_ai(&name, &ai, team, ally_team, colour) {
                    Outcome::Did(format!("{name} ({ai}) on team {}", ally_team + 1))
                } else {
                    Outcome::Said(format!("{name} is already here"))
                }
            }
            Act::RemoveBot { name } => {
                if self.remove_ai(&name) {
                    Outcome::Did(format!("{name} is out"))
                } else {
                    Outcome::Nothing
                }
            }
            Act::Say { text } => command::command(self, &text),
        }
    }

    // --- what the front end sees ------------------------------------------

    /// The room as the lobby's own view types, so every component that draws a
    /// battle draws this one too.
    pub fn view(&self, content: lobby_ui::ContentView) -> SkirmishView {
        SkirmishView {
            battle: self.battle(),
            my: self.my_battle(),
            users: vec![self.user()],
            me: self.player.clone(),
            content,
        }
    }

    fn battle(&self) -> BattleView {
        let seated = self.seat.is_some();
        BattleView {
            id: ROOM_ID,
            founder: self.player.clone(),
            ip: String::new(),
            port: 0,
            max_players: 16,
            passworded: false,
            locked: false,
            map_hash: String::new(),
            map_name: self.map.clone(),
            engine_name: "spring".to_owned(),
            engine_version: self.engine.clone(),
            title: self.title.clone(),
            game_name: self.game.clone(),
            members: vec![self.player.clone()],
            spectator_count: u32::from(!seated),
            player_count: u32::from(seated),
            layout: self.layout,
            bots: self.ais.iter().map(bot_view).collect(),
            // The engine's own rectangles are not how this room carries its
            // boxes; the `mapmetadata_*` modoptions are, and they are already
            // in `script_tags` where every reader looks for them.
            start_rects: Vec::new(),
        }
    }

    fn my_battle(&self) -> MyBattleView {
        MyBattleView {
            // There is one person here and it is you. Saying so is what lets
            // the room offer everything a boss may do.
            boss: Some(self.player.clone()),
            id: ROOM_ID,
            game_hash: String::new(),
            script_tags: self.script_tags.clone(),
            // Nothing to vote on where nobody can disagree.
            vote: None,
            history: self.history.clone(),
        }
    }

    fn user(&self) -> UserView {
        UserView {
            name: self.player.clone(),
            country: String::new(),
            user_id: None,
            lobby_client: String::new(),
            status: UserStatusView {
                in_game: false,
                away: false,
                rank: 0,
                moderator: false,
                bot: false,
            },
            battle_status: Some(status_view(self.seat, true)),
            battle_id: Some(ROOM_ID),
        }
    }
}

fn bot_view(ai: &Ai) -> BotView {
    BotView {
        name: ai.name.clone(),
        owner: String::new(),
        status: status_view(Some(ai.seat), false),
        team_colour: ai.colour,
        ai: ai.ai.clone(),
    }
}

/// A seat as the lobby reports one. `None` is a spectator, which the protocol
/// spells as a battle status with `player` unset rather than as no status.
fn status_view(seat: Option<Seat>, human: bool) -> BattleStatusView {
    let held = seat.unwrap_or(Seat::new(0, 0));
    BattleStatusView {
        ready: false,
        team: held.team,
        ally_team: held.ally_team,
        player: seat.is_some(),
        handicap: held.handicap,
        // Whatever this room names, it names because this machine has it.
        sync: if human {
            SyncView::Synced
        } else {
            SyncView::Bot
        },
        side: held.side,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> Room {
        Room::new(
            "tetrisface",
            "Beyond All Reason test-31134",
            "Comet Catcher Remake 1.8",
            "2026.07.04",
        )
    }

    fn content() -> lobby_ui::ContentView {
        lobby_ui::ContentView {
            engine: true,
            game: true,
            map: true,
        }
    }

    #[test]
    fn a_fresh_room_seats_the_player_and_nobody_else() {
        let view = room().view(content());
        assert_eq!(view.battle.members, ["tetrisface"]);
        assert_eq!(view.battle.player_count, 1);
        assert_eq!(view.battle.spectator_count, 0);
        assert!(view.battle.bots.is_empty());
        // You run your own room, which is what lets it offer everything.
        assert_eq!(view.my.boss.as_deref(), Some("tetrisface"));
        assert_eq!(view.my.vote, None);
    }

    #[test]
    fn a_modoption_is_kept_under_the_key_the_online_room_uses() {
        let mut room = room();
        assert!(room.set_option("ranked_game", "0"));
        let view = room.view(content());
        assert_eq!(
            view.my.script_tags.get("game/modoptions/ranked_game"),
            Some(&"0".to_owned())
        );
        assert_eq!(room.modoption("ranked_game"), "0");
    }

    #[test]
    fn setting_a_value_it_already_has_is_not_a_change() {
        let mut room = room();
        assert!(room.set_option("ranked_game", "0"));
        assert!(!room.set_option("ranked_game", "0"));
        assert_eq!(room.view(content()).my.history.len(), 1);
    }

    #[test]
    fn every_change_is_recorded_with_who_made_it() {
        let mut room = room();
        room.set_option("ranked_game", "0");
        room.set_option("ranked_game", "1");
        let history = room.view(content()).my.history;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].seq, 1);
        assert_eq!(history[1].from, "0");
        assert_eq!(history[1].to, "1");
        assert_eq!(history[1].by.as_deref(), Some("tetrisface"));
    }

    #[test]
    fn clearing_an_option_is_not_the_same_as_emptying_it() {
        let mut room = room();
        room.set_option("tweakdefs1", "abc");
        assert!(room.clear_option("tweakdefs1"));
        assert!(
            !room
                .view(content())
                .my
                .script_tags
                .contains_key("game/modoptions/tweakdefs1")
        );
        // Nothing to clear the second time.
        assert!(!room.clear_option("tweakdefs1"));
    }

    #[test]
    fn an_ai_takes_a_seat_and_shows_as_a_bot() {
        let mut room = room();
        assert!(room.add_ai("BARb", "BARb", 1, 1, COLOURS[1]));
        let view = room.view(content());
        assert_eq!(view.battle.bots.len(), 1);
        assert_eq!(view.battle.bots[0].ai, "BARb");
        assert_eq!(view.battle.bots[0].status.ally_team, 1);
        // A bot holds a player seat, as `ADDBOT` sends one
        // (`spring-protocol/src/battle.rs:250`); what marks it out is the sync.
        assert!(view.battle.bots[0].status.player);
        assert_eq!(view.battle.bots[0].status.sync, SyncView::Bot);
    }

    #[test]
    fn two_ais_may_not_share_a_name() {
        let mut room = room();
        assert!(room.add_ai("BARb", "BARb", 1, 1, 0));
        assert!(!room.add_ai("BARb", "BARb", 2, 1, 0));
        assert_eq!(room.unused_name("BARb"), "BARb2");
        assert!(room.add_ai("BARb2", "BARb", 2, 1, 0));
        assert_eq!(room.unused_name("BARb"), "BARb3");
    }

    #[test]
    fn a_free_team_is_the_lowest_nobody_holds() {
        let mut room = room();
        assert_eq!(room.free_team(), 1);
        room.add_ai("a", "BARb", 1, 1, 0);
        assert_eq!(room.free_team(), 2);
        room.remove_ai("a");
        assert_eq!(room.free_team(), 1);
    }

    #[test]
    fn watching_leaves_the_room_with_no_players_in_it() {
        let mut room = room();
        room.release_seat();
        let view = room.view(content());
        assert_eq!(view.battle.player_count, 0);
        assert_eq!(view.battle.spectator_count, 1);
        let status = view.users[0].battle_status.unwrap();
        assert!(!status.player);
    }

    #[test]
    fn moving_sides_keeps_the_faction_you_picked() {
        let mut room = room();
        room.set_side(3);
        room.take_seat(0, 1);
        assert_eq!(room.seat().unwrap().ally_team, 1);
        assert_eq!(room.seat().unwrap().side, 3);
    }

    #[test]
    fn fixing_colours_gives_each_side_its_own() {
        let mut room = room();
        room.add_ai("a", "BARb", 1, 1, 0);
        room.add_ai("b", "BARb", 2, 1, 0);
        room.add_ai("c", "BARb", 3, 2, 0);
        room.fix_colours();
        let ais = room.ais();
        assert_eq!(ais[0].colour, ais[1].colour);
        assert_ne!(ais[0].colour, ais[2].colour);
    }

    #[test]
    fn ally_teams_are_whoever_is_actually_on_one() {
        let mut room = room();
        room.add_ai("a", "BARb", 1, 3, 0);
        assert_eq!(room.ally_teams(), [0, 3]);
        room.release_seat();
        assert_eq!(room.ally_teams(), [3]);
    }
}
