//! Turning the room into the script the engine plays.
//!
//! Everything the room shows has to survive this journey or the game is not
//! the one that was set up, so the awkward parts are here and each has a test:
//! ally teams renumbered without gaps, teams numbered apart from them, a
//! faction written by name with Legion's modoption honoured, and start boxes
//! written both ways the game might read them.

use std::collections::BTreeMap;

use recoil::script::{self, AllyTeam, Player, Rect, Skirmish, StartPos, Team};

use crate::Room;

/// The modoption BAR reads a room's own arrangement from.
const OVERRIDE: &str = "mapmetadata_startbox_override";
/// The map's own arrangements, keyed by team count.
const SET: &str = "mapmetadata_startboxes_set";
/// Legion is not in the game until this is on (`sidedata.lua:21`).
const LEGION: &str = "experimentallegionfaction";

/// The factions by the number the lobby carries them as, and the name the
/// script wants (`BYAR-Chobby/LuaMenu/configs/gameConfig/byar/sidedata.lua`).
const SIDES: [&str; 4] = ["Armada", "Cortex", "Random", "Legion"];

impl Room {
    /// The script this room would play.
    pub fn to_script(&self) -> Skirmish {
        let start_pos = match self.start_pos() {
            0 => StartPos::Fixed,
            1 => StartPos::Random,
            _ => StartPos::InGame,
        };
        // Somebody may have put an AI on side 4 with nothing on 2 or 3. The
        // engine counts ally teams from zero without gaps, so they are
        // renumbered — and the boxes are resolved for the count that leaves.
        let sides = self.ally_teams();
        let side_of = |ally: u8| sides.iter().position(|s| *s == ally).unwrap_or(0) as u8;
        let legion = self.modoption(LEGION) == "1";

        let mut teams: Vec<Team> = Vec::new();
        let mut players: Vec<Player> = Vec::new();
        let mut ais: Vec<script::Ai> = Vec::new();

        // We are player 0, which is what every AI's `host` points at.
        let mine = self.seat().map(|seat| {
            teams.push(Team {
                ally_team: side_of(seat.ally_team),
                leader: 0,
                side: faction(seat.side, legion, false),
                // Chobby writes one for every team; without it the engine
                // picks, and two teams can end up hard to tell apart.
                colour: Some(self.player_colour()),
                handicap: seat.handicap,
            });
            teams.len() as u8 - 1
        });
        players.push(Player {
            name: self.player.clone(),
            team: mine,
        });

        for ai in self.ais() {
            teams.push(Team {
                ally_team: side_of(ai.seat.ally_team),
                leader: 0,
                side: faction(ai.seat.side, legion, true),
                colour: Some(ai.colour),
                handicap: ai.seat.handicap,
            });
            ais.push(script::Ai {
                name: ai.name.clone(),
                short_name: ai.ai.clone(),
                version: None,
                team: teams.len() as u8 - 1,
                host: 0,
                options: ai
                    .options
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            });
        }

        let boxes = self.arrangement(sides.len() as u32);
        Skirmish {
            game: self.game.clone(),
            map: self.map.clone(),
            player: self.player.clone(),
            start_pos,
            modoptions: self.script_modoptions(),
            ally_teams: ally_teams(boxes.as_ref(), sides.len()),
            teams,
            players,
            ais,
        }
    }

    /// The arrangement the game will settle on for this many sides.
    ///
    /// [`startbox::resolve`] is the mirror of the game's own
    /// `resolveArrangement`, so asking it here is what stops the script
    /// disagreeing with what the room drew: an override with fewer boxes than
    /// there are sides is dropped by the game and the map's set used instead,
    /// and this drops it too rather than writing rectangles the game will
    /// contradict.
    fn arrangement(&self, sides: u32) -> Option<startbox::Arrangement> {
        let over = startbox::decode_override(self.modoption(OVERRIDE)).ok();
        let set = startbox::decode_set(self.modoption(SET)).unwrap_or_default();
        startbox::resolve(over.as_ref(), &set, sides).map(|(held, _)| held)
    }

    /// The `[modoptions]` block.
    ///
    /// The start-box keys are carried through untouched: they are what BAR's
    /// own gadget reads, and the `startrect*` written beside them is only the
    /// baseline for a game build without the decoder. Chobby clears both
    /// before writing (`interface_skirmish.lua:350-361`) because its table
    /// outlives a launch; ours is rebuilt from the room every time.
    fn script_modoptions(&self) -> Vec<(String, String)> {
        self.modoptions()
            .filter(|(_, value)| !value.is_empty())
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }
}

/// One entry per side, each with the rectangle that contains its box.
fn ally_teams(boxes: Option<&startbox::Arrangement>, sides: usize) -> Vec<AllyTeam> {
    (0..sides)
        .map(|side| AllyTeam {
            start_rect: boxes
                .and_then(|held| held.startboxes.get(side))
                .map(|held| {
                    let (left, top, right, bottom) = held.bounds();
                    Rect {
                        left: left / 200.0,
                        top: top / 200.0,
                        right: right / 200.0,
                        bottom: bottom / 200.0,
                    }
                }),
        })
        .collect()
}

/// The faction by name, with Legion falling back where the game has not been
/// told to include it — to Armada for a person and Random for an AI, which is
/// what Chobby does (`interface_skirmish.lua:91-94`, `:161-164`).
fn faction(side: u8, legion: bool, is_ai: bool) -> Option<String> {
    let side = usize::from(side);
    if side == 3 && !legion {
        return Some(SIDES[if is_ai { 2 } else { 0 }].to_owned());
    }
    SIDES.get(side).map(|name| (*name).to_owned())
}

/// The map's own arrangements, encoded as the modoption carries them. Only
/// used by the tests below and by anything that wants to plant a set.
pub fn encode_set(set: &BTreeMap<u32, startbox::Arrangement>) -> Option<String> {
    startbox::encode_set(set).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::COLOURS;

    fn room() -> Room {
        Room::new("me", "BAR test-1", "Comet Catcher", "2026.07.04")
    }

    fn square(left: f32, top: f32, right: f32, bottom: f32) -> startbox::Box {
        let at = |x: f32, y: f32| startbox::Point {
            x,
            y,
            strength: None,
        };
        startbox::Box {
            poly: vec![at(left, top), at(right, bottom)],
        }
    }

    #[test]
    fn the_room_becomes_a_script_that_names_everyone_in_it() {
        let mut room = room();
        room.add_ai("BARb", "BARb", 1, 1, COLOURS[1]);
        let script = room.to_script().script();

        assert!(script.contains("myplayername = me;"));
        assert!(script.contains("gametype = BAR test-1;"));
        assert!(script.contains("mapname = Comet Catcher;"));
        assert!(script.contains("numplayers = 1;"));
        assert!(script.contains("numusers = 2;"));
        assert!(script.contains("shortname = BARb;"));
        assert!(script.contains("[team0] {\n\t\tallyteam = 0;"));
        assert!(script.contains("[team1] {\n\t\tallyteam = 1;"));
    }

    #[test]
    fn ally_teams_are_renumbered_so_the_engine_finds_no_gaps() {
        let mut room = room();
        // Two AIs put on sides 4 and 7, with nothing between.
        room.add_ai("a", "BARb", 1, 4, 0);
        room.add_ai("b", "BARb", 2, 7, 0);
        let skirmish = room.to_script();
        assert_eq!(skirmish.ally_teams.len(), 3);
        assert_eq!(
            skirmish
                .teams
                .iter()
                .map(|t| t.ally_team)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        let script = skirmish.script();
        assert!(script.contains("[allyteam2]"));
        assert!(!script.contains("[allyteam3]"));
    }

    #[test]
    fn two_ais_on_one_side_share_it_and_still_hold_teams_of_their_own() {
        let mut room = room();
        room.add_ai("a", "BARb", 1, 1, 0);
        room.add_ai("b", "BARb", 2, 1, 0);
        let skirmish = room.to_script();
        assert_eq!(skirmish.ally_teams.len(), 2);
        assert_eq!(skirmish.teams.len(), 3);
        assert_eq!(skirmish.teams[1].ally_team, 1);
        assert_eq!(skirmish.teams[2].ally_team, 1);
        assert_eq!(skirmish.ais[0].team, 1);
        assert_eq!(skirmish.ais[1].team, 2);
    }

    #[test]
    fn watching_writes_a_spectator_and_leaves_the_ais_playing() {
        let mut room = room();
        room.add_ai("a", "BARb", 1, 1, 0);
        room.add_ai("b", "BARb", 2, 2, 0);
        room.release_seat();
        let skirmish = room.to_script();
        assert_eq!(skirmish.players[0].team, None);
        assert_eq!(skirmish.teams.len(), 2);
        assert!(skirmish.script().contains("spectator = 1;"));
    }

    #[test]
    fn legion_falls_back_until_the_game_has_been_told_to_include_it() {
        let mut room = room();
        room.set_side(3);
        room.add_ai("a", "BARb", 1, 1, 0);
        // The AI is on Legion too.
        let mut with_legion = room.clone();

        let script = room.to_script();
        assert_eq!(script.teams[0].side.as_deref(), Some("Armada"));

        with_legion.set_option(LEGION, "1");
        let script = with_legion.to_script();
        assert_eq!(script.teams[0].side.as_deref(), Some("Legion"));
    }

    #[test]
    fn an_override_that_covers_every_side_is_what_the_script_carries() {
        let mut room = room();
        room.add_ai("a", "BARb", 1, 1, 0);
        let over = startbox::encode_override(&startbox::Arrangement {
            startboxes: vec![
                square(0.0, 0.0, 50.0, 200.0),
                square(150.0, 0.0, 200.0, 200.0),
            ],
        })
        .unwrap();
        room.set_option(OVERRIDE, &over);

        let skirmish = room.to_script();
        let first = skirmish.ally_teams[0].start_rect.unwrap();
        assert_eq!((first.left, first.right), (0.0, 0.25));
        let second = skirmish.ally_teams[1].start_rect.unwrap();
        assert_eq!((second.left, second.right), (0.75, 1.0));
        // And the modoption rides along, because that is what the game reads.
        assert!(skirmish.modoptions.iter().any(|(key, _)| key == OVERRIDE));
    }

    /// The game drops an override that does not cover every ally team and
    /// resolves the set instead (`startbox_utilities.lua` `matchOverride`).
    /// Writing its rectangles anyway would put the script at odds with the
    /// game, so a short one is not written at all.
    #[test]
    fn an_override_too_short_for_the_sides_is_not_written_as_rectangles() {
        let mut room = room();
        room.add_ai("a", "BARb", 1, 1, 0);
        room.add_ai("b", "BARb", 2, 2, 0);
        let over = startbox::encode_override(&startbox::Arrangement {
            startboxes: vec![
                square(0.0, 0.0, 50.0, 200.0),
                square(150.0, 0.0, 200.0, 200.0),
            ],
        })
        .unwrap();
        room.set_option(OVERRIDE, &over);

        // Three sides, two boxes: the game would use the map's set.
        let skirmish = room.to_script();
        assert_eq!(skirmish.ally_teams.len(), 3);
        assert!(skirmish.ally_teams.iter().all(|a| a.start_rect.is_none()));
    }

    #[test]
    fn the_maps_own_set_is_used_when_there_is_no_override() {
        let mut room = room();
        room.add_ai("a", "BARb", 1, 1, 0);
        let set = BTreeMap::from([(
            2,
            startbox::Arrangement {
                startboxes: vec![
                    square(0.0, 0.0, 40.0, 200.0),
                    square(160.0, 0.0, 200.0, 200.0),
                ],
            },
        )]);
        room.set_option(SET, &encode_set(&set).unwrap());

        let skirmish = room.to_script();
        let first = skirmish.ally_teams[0].start_rect.unwrap();
        assert_eq!((first.left, first.right), (0.0, 0.2));
    }

    #[test]
    fn a_polygon_is_squared_off_to_the_rectangle_that_holds_it() {
        let mut room = room();
        room.add_ai("a", "BARb", 1, 1, 0);
        let at = |x: f32, y: f32| startbox::Point {
            x,
            y,
            strength: None,
        };
        let over = startbox::encode_override(&startbox::Arrangement {
            startboxes: vec![
                startbox::Box {
                    poly: vec![at(10.0, 20.0), at(60.0, 5.0), at(30.0, 90.0)],
                },
                square(150.0, 0.0, 200.0, 200.0),
            ],
        })
        .unwrap();
        room.set_option(OVERRIDE, &over);

        let rect = room.to_script().ally_teams[0].start_rect.unwrap();
        assert_eq!(rect.left, 10.0 / 200.0);
        assert_eq!(rect.top, 5.0 / 200.0);
        assert_eq!(rect.right, 60.0 / 200.0);
        assert_eq!(rect.bottom, 90.0 / 200.0);
    }

    #[test]
    fn an_ais_own_options_ride_into_its_block() {
        let mut room = room();
        room.add_ai("BARb", "BARb", 1, 1, 0);
        room.set_ai_option("BARb", "cheating", "1");
        let script = room.to_script().script();
        assert!(
            script.contains(
                "		[options] {
			cheating = 1;"
            ),
            "{script}"
        );
    }

    #[test]
    fn the_rooms_start_positions_are_what_the_script_says() {
        let mut room = room();
        assert!(room.to_script().script().contains("startpostype = 2;"));
        room.set_start_pos(0);
        assert!(room.to_script().script().contains("startpostype = 0;"));
        room.set_start_pos(1);
        assert!(room.to_script().script().contains("startpostype = 1;"));
    }

    /// What the room writes, read back by the parser the app uses on scripts the
    /// engine itself wrote (`recoil::script_read`, behind `preset_from_replay`).
    /// A writer and a reader that disagree is exactly the bug nobody notices
    /// until a game will not start.
    #[test]
    fn a_written_script_reads_back_as_what_the_room_was() {
        let mut room = room();
        room.set_option("ranked_game", "0");
        room.set_option("tweakdefs1", "LS1OdXR0eUIgdjEuNTI");
        room.add_ai("BARb", "BARb", 1, 1, COLOURS[1]);
        let over = startbox::encode_override(&startbox::Arrangement {
            startboxes: vec![
                square(0.0, 0.0, 50.0, 200.0),
                square(150.0, 0.0, 200.0, 200.0),
            ],
        })
        .unwrap();
        room.set_option(OVERRIDE, &over);

        let text = room.to_script().script();
        let back = recoil::script_read::parse(&text);

        assert_eq!(
            back.game.get("gametype").map(String::as_str),
            Some("BAR test-1")
        );
        assert_eq!(
            back.game.get("mapname").map(String::as_str),
            Some("Comet Catcher")
        );
        assert_eq!(
            back.game.get("myplayername").map(String::as_str),
            Some("me")
        );
        assert_eq!(back.game.get("startpostype").map(String::as_str), Some("2"));
        assert_eq!(back.game.get("numusers").map(String::as_str), Some("2"));
        assert_eq!(
            back.modoptions.get("ranked_game").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            back.modoptions.get("tweakdefs1").map(String::as_str),
            Some("LS1OdXR0eUIgdjEuNTI")
        );

        // The rectangles come back in the 0-200 the lobby draws in, which is
        // the units they were drawn in before the script halved them to
        // fractions and this doubled them again.
        let boxes = back.boxes_out_of_200();
        assert_eq!(boxes.len(), 2);
        let (left, _, right, _) = boxes[&0];
        assert_eq!((left, right), (0, 50));
        let (left, _, right, _) = boxes[&1];
        assert_eq!((left, right), (150, 200));
    }

    #[test]
    fn a_cleared_modoption_is_not_written_as_an_empty_one() {
        let mut room = room();
        room.set_option("ranked_game", "0");
        let skirmish = room.to_script();
        assert_eq!(
            skirmish.modoptions,
            [("ranked_game".to_owned(), "0".to_owned())]
        );
    }
}
