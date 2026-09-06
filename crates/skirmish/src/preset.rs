//! Saving the room as a preset, and putting one back.
//!
//! Online, applying a preset is a list of `!bSet` lines trickled past SPADS'
//! flood limit — which is why `presets::apply` separates planning from
//! sending. Here there is nobody to ask and nothing to pace: the preset's
//! fields go straight into the room. What the two share is the file, so a
//! preset saved in a room on the server plays here, and one made from a replay
//! (`preset_from_replay`) becomes "that game again, against AI".

use presets::{Preset, Sections, Stamp, bots};

use crate::{COLOURS, Room};

/// Which ally team a preset's start boxes are keyed by, and the modoption the
/// room actually reads them from.
const OVERRIDE: &str = "mapmetadata_startbox_override";

/// What applying one did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Applied {
    /// Settings the preset named that the room already had.
    pub already_set: usize,
    /// Settings it changed.
    pub changed: usize,
    /// AIs it put in the room, replacing whatever was there.
    pub bots: usize,
    /// Whether its start boxes could be put back.
    pub boxes: bool,
}

/// The room as a preset.
///
/// Its AIs are written, which the online `save_preset` does not do — a room on
/// the server rarely has any, and a skirmish is mostly its AIs.
pub fn to_preset(room: &Room, name: impl Into<String>, now: Stamp) -> Preset {
    let mut preset = Preset::new(name, now);
    preset.map = Some(room.map.clone());
    preset.modoptions = room
        .modoptions()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    preset
        .battle
        .insert("nbTeams".into(), room.layout_teams().to_string());
    preset
        .battle
        .insert("teamSize".into(), room.layout_size().to_string());

    let held: Vec<bots::Bot> = room
        .ais()
        .iter()
        .map(|ai| {
            let mut bot =
                bots::Bot::new(&ai.name, &ai.ai, ai.seat.ally_team).with_colour(ai.colour);
            bot.side = ai.seat.side;
            bot.handicap = ai.seat.handicap;
            bot.ai_options = ai
                .options
                .iter()
                .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
                .collect();
            bot
        })
        .collect();
    bots::write(&mut preset, &held);
    preset
}

/// Puts a preset back into the room.
///
/// `reset` clears the modoptions first, which is what makes applying one
/// preset after another replace rather than combine — the same thing
/// `!preset <name>` does online.
pub fn apply(room: &mut Room, preset: &Preset, sections: Sections) -> Applied {
    let mut done = Applied::default();

    if sections.reset && sections.modoptions {
        let held: Vec<String> = room.modoptions().map(|(key, _)| key.to_owned()).collect();
        for key in held {
            room.clear_option(&key);
        }
    }

    if sections.map
        && let Some(map) = &preset.map
    {
        room.set_map(map);
    }

    if sections.modoptions {
        for (key, value) in &preset.modoptions {
            if room.set_option(key, value) {
                done.changed += 1;
            } else {
                done.already_set += 1;
            }
        }
    }

    if sections.battle {
        let number = |key: &str| preset.battle.get(key).and_then(|held| held.parse().ok());
        let teams = number("nbTeams").unwrap_or_else(|| room.layout_teams());
        let size = number("teamSize").unwrap_or_else(|| room.layout_size());
        room.set_layout(teams, size);
    }

    if sections.bots {
        done.bots = put_bots(room, preset);
    }

    // The boxes ride in a modoption, so they are already back if the settings
    // were. A preset from an older file keeps them in `startBoxes` instead,
    // and those are put back only when nothing has already said otherwise.
    if sections.start_boxes {
        done.boxes = put_boxes(room, preset);
    }

    done
}

/// Replaces the room's AIs with the preset's. Replaces rather than adds: two
/// applies in a row should be the same game twice, not twice the game.
fn put_bots(room: &mut Room, preset: &Preset) -> usize {
    let wanted = bots::read(preset);
    let held: Vec<String> = room.ais().iter().map(|ai| ai.name.clone()).collect();
    for name in held {
        room.remove_ai(&name);
    }
    for bot in &wanted {
        let colour = bot
            .colour()
            .unwrap_or(COLOURS[usize::from(bot.ally_number) % COLOURS.len()]);
        let team = room.free_team();
        room.add_ai(&bot.name, &bot.ai_lib, team, bot.ally_number, colour);
        for (key, value) in &bot.ai_options {
            let text = match value {
                serde_json::Value::String(held) => held.clone(),
                serde_json::Value::Bool(held) => u8::from(*held).to_string(),
                other => other.to_string(),
            };
            room.set_ai_option(&bot.name, key, &text);
        }
    }
    wanted.len()
}

/// The rectangles an older preset carries, as the arrangement the room reads.
///
/// Only when the preset did not bring the modoption itself: that one is what
/// the game enforces, and a rectangle drawn from a sparse `startBoxes` table
/// would quietly disagree with it.
fn put_boxes(room: &mut Room, preset: &Preset) -> bool {
    if preset.modoptions.contains_key(OVERRIDE) {
        return true;
    }
    if preset.start_boxes.is_empty() {
        return false;
    }
    // Chobby's table is sparse -- it can name team 2 without naming team 1 --
    // and the game drops an arrangement that does not cover every side. Dense
    // from zero or not at all.
    let highest = preset.start_boxes.keys().copied().max().unwrap_or(0);
    let boxes: Option<Vec<startbox::Box>> = (0..=highest)
        .map(|ally| preset.start_boxes.get(&ally).map(rect))
        .collect();
    let Some(startboxes) = boxes else {
        return false;
    };
    let Ok(blob) = startbox::encode_override(&startbox::Arrangement { startboxes }) else {
        return false;
    };
    room.set_option(OVERRIDE, &blob);
    true
}

fn rect(held: &presets::StartBox) -> startbox::Box {
    let at = |x: u16, y: u16| startbox::Point {
        x: f32::from(x),
        y: f32::from(y),
        strength: None,
    };
    startbox::Box {
        poly: vec![at(held.left, held.top), at(held.right, held.bottom)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> Room {
        Room::new("me", "BAR test-1", "Comet Catcher", "2026.07.04")
    }

    fn all() -> Sections {
        Sections {
            bots: true,
            ..Sections::default()
        }
    }

    #[test]
    fn a_room_saves_its_map_settings_and_ais() {
        let mut room = room();
        room.set_option("ranked_game", "0");
        room.add_ai("BARb", "BARb", 1, 1, COLOURS[1]);
        room.set_layout(2, 4);

        let preset = to_preset(&room, "mine", 7);
        assert_eq!(preset.map.as_deref(), Some("Comet Catcher"));
        assert_eq!(preset.modoptions["ranked_game"], "0");
        assert_eq!(preset.battle["nbTeams"], "2");
        assert_eq!(preset.battle["teamSize"], "4");
        let saved = bots::read(&preset);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].ai_lib, "BARb");
        assert_eq!(saved[0].ally_number, 1);
        assert_eq!(saved[0].colour(), Some(COLOURS[1]));
    }

    #[test]
    fn a_preset_goes_back_into_a_room_whole() {
        let mut source = room();
        source.set_option("ranked_game", "0");
        source.set_option("experimentallegionfaction", "1");
        source.add_ai("Left", "BARb", 1, 1, COLOURS[1]);
        source.add_ai("Right", "CircuitAI", 2, 2, COLOURS[2]);
        let preset = to_preset(&source, "mine", 7);

        let mut room = room();
        let done = apply(&mut room, &preset, all());
        assert_eq!(done.changed, 2);
        assert_eq!(done.bots, 2);
        assert_eq!(room.modoption("ranked_game"), "0");
        assert_eq!(room.ais().len(), 2);
        assert_eq!(room.ais()[0].name, "Left");
        assert_eq!(room.ais()[1].ai, "CircuitAI");
        assert_eq!(room.ais()[1].seat.ally_team, 2);
    }

    #[test]
    fn an_ais_own_options_survive_a_preset() {
        let mut source = room();
        source.add_ai("BARb", "BARb", 1, 1, 0);
        source.set_ai_option("BARb", "cheating", "1");
        let preset = to_preset(&source, "mine", 7);

        let mut room = room();
        apply(&mut room, &preset, all());
        assert_eq!(room.ais()[0].options["cheating"], "1");
    }

    #[test]
    fn applying_the_same_preset_twice_is_the_same_game_twice() {
        let mut source = room();
        source.add_ai("Left", "BARb", 1, 1, 0);
        let preset = to_preset(&source, "mine", 7);

        let mut room = room();
        apply(&mut room, &preset, all());
        let done = apply(&mut room, &preset, all());
        assert_eq!(room.ais().len(), 1);
        assert_eq!(done.bots, 1);
        // Nothing moved the second time, and it says so.
        assert_eq!(done.changed, 0);
    }

    #[test]
    fn a_reset_replaces_the_settings_rather_than_adding_to_them() {
        let mut room = room();
        room.set_option("leftover", "1");
        let mut preset = Preset::new("mine", 7);
        preset.modoptions.insert("ranked_game".into(), "0".into());

        apply(&mut room, &preset, Sections::default());
        assert_eq!(room.modoption("leftover"), "");
        assert_eq!(room.modoption("ranked_game"), "0");
    }

    #[test]
    fn without_a_reset_two_presets_combine() {
        let mut room = room();
        room.set_option("leftover", "1");
        let mut preset = Preset::new("mine", 7);
        preset.modoptions.insert("ranked_game".into(), "0".into());

        apply(
            &mut room,
            &preset,
            Sections {
                reset: false,
                ..Sections::default()
            },
        );
        assert_eq!(room.modoption("leftover"), "1");
        assert_eq!(room.modoption("ranked_game"), "0");
    }

    #[test]
    fn a_section_left_out_is_left_alone() {
        let mut source = room();
        source.set_option("ranked_game", "0");
        source.add_ai("Left", "BARb", 1, 1, 0);
        source.set_map("Somewhere Else");
        let preset = to_preset(&source, "mine", 7);

        let mut room = room();
        apply(
            &mut room,
            &preset,
            Sections {
                map: false,
                bots: false,
                ..Sections::default()
            },
        );
        assert_eq!(room.map, "Comet Catcher");
        assert!(room.ais().is_empty());
        assert_eq!(room.modoption("ranked_game"), "0");
    }

    #[test]
    fn an_older_presets_rectangles_become_the_arrangement_the_room_reads() {
        let mut preset = Preset::new("mine", 7);
        preset.start_boxes.insert(
            0,
            presets::StartBox {
                left: 0,
                top: 0,
                right: 50,
                bottom: 200,
            },
        );
        preset.start_boxes.insert(
            1,
            presets::StartBox {
                left: 150,
                top: 0,
                right: 200,
                bottom: 200,
            },
        );

        let mut room = room();
        assert!(apply(&mut room, &preset, all()).boxes);
        let held = startbox::decode_override(room.modoption(OVERRIDE)).unwrap();
        assert_eq!(held.startboxes.len(), 2);
        assert_eq!(held.startboxes[0].poly[1].x, 50.0);
    }

    /// The game drops an arrangement that does not cover every side, so a
    /// table with a gap in it is not turned into one.
    #[test]
    fn a_sparse_table_of_rectangles_is_not_guessed_at() {
        let mut preset = Preset::new("mine", 7);
        preset.start_boxes.insert(
            2,
            presets::StartBox {
                left: 0,
                top: 0,
                right: 50,
                bottom: 200,
            },
        );
        let mut room = room();
        assert!(!apply(&mut room, &preset, all()).boxes);
        assert_eq!(room.modoption(OVERRIDE), "");
    }

    #[test]
    fn the_modoption_wins_over_the_rectangles_when_a_preset_has_both() {
        let mut source = room();
        let blob = startbox::encode_override(&startbox::Arrangement {
            startboxes: vec![rect(&presets::StartBox {
                left: 10,
                top: 10,
                right: 20,
                bottom: 20,
            })],
        })
        .unwrap();
        source.set_option(OVERRIDE, &blob);
        let mut preset = to_preset(&source, "mine", 7);
        preset.start_boxes.insert(
            0,
            presets::StartBox {
                left: 0,
                top: 0,
                right: 200,
                bottom: 200,
            },
        );

        let mut room = room();
        assert!(apply(&mut room, &preset, all()).boxes);
        // The blob the preset carried, not one built from its rectangles.
        assert_eq!(room.modoption(OVERRIDE), blob);
    }
}
