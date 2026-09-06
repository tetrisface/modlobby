//! The AI slots in a preset, read rather than carried through.
//!
//! [`crate::Preset::bots`] is a bag of JSON on purpose: its shape is whatever
//! Chobby's `AddAi` accepted at the time, and re-encoding a structure we do
//! not fully model is how you lose somebody's carefully tuned bot setup.
//!
//! A room with no server behind it is the reason to read it: a skirmish is
//! mostly its AIs, and a preset that could not put them back would be a
//! preset of the settings alone. So this reads the fields Chobby writes
//! (`gui_optionpresets_panel.lua:177-196`) and leaves every other key on the
//! object exactly where it was found.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Preset;

/// One AI, as Chobby's file names its parts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bot {
    /// What it is called in the room. The key it was filed under.
    #[serde(skip)]
    pub name: String,
    /// What the engine is asked for: `BARb`, or a game's Lua AI by name.
    pub ai_lib: String,
    /// The side it plays on.
    pub ally_number: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_version: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub ai_options: Map<String, Value>,
    /// Three floats, 0 to 1, which is how the engine and Chobby both read one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_color: Option<[f32; 3]>,
    #[serde(default)]
    pub side: u8,
    #[serde(default)]
    pub handicap: u8,
    /// Anything else the file had for this AI, kept so a round trip through
    /// here loses nothing somebody else's client put there.
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Bot {
    pub fn new(name: impl Into<String>, ai_lib: impl Into<String>, ally_number: u8) -> Self {
        Self {
            name: name.into(),
            ai_lib: ai_lib.into(),
            ally_number,
            ai_version: None,
            ai_options: Map::new(),
            team_color: None,
            side: 0,
            handicap: 0,
            rest: Map::new(),
        }
    }

    /// The colour as the lobby carries one, `0xBBGGRR`.
    pub fn colour(&self) -> Option<u32> {
        let [red, green, blue] = self.team_color?;
        let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
        Some(byte(red) | (byte(green) << 8) | (byte(blue) << 16))
    }

    pub fn with_colour(mut self, colour: u32) -> Self {
        let channel = |shift: u32| ((colour >> shift) & 0xFF) as f32 / 255.0;
        self.team_color = Some([channel(0), channel(8), channel(16)]);
        self
    }
}

/// The AIs a preset holds, in name order so applying one twice is the same
/// game twice.
pub fn read(preset: &Preset) -> Vec<Bot> {
    let mut bots: Vec<Bot> = preset
        .bots
        .iter()
        .filter_map(|(name, value)| {
            let mut bot: Bot = serde_json::from_value(value.clone()).ok()?;
            bot.name = name.clone();
            Some(bot)
        })
        .collect();
    bots.sort_by(|a, b| a.name.cmp(&b.name));
    bots
}

/// Replaces the preset's AIs with these.
pub fn write(preset: &mut Preset, bots: &[Bot]) {
    preset.bots = bots
        .iter()
        .filter_map(|bot| {
            let value = serde_json::to_value(bot).ok()?;
            Some((bot.name.clone(), value))
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset_with(json: &str) -> Preset {
        let mut preset = Preset::new("p", 0);
        preset.bots = serde_json::from_str(json).unwrap();
        preset
    }

    #[test]
    fn chobbys_own_shape_reads() {
        let preset = preset_with(
            r#"{
                "BARb": {
                    "aiLib": "BARb",
                    "allyNumber": 1,
                    "aiVersion": "0.1",
                    "aiOptions": { "difficultyLevel": 2 },
                    "teamColor": [1.0, 0.0, 0.0],
                    "side": 1,
                    "handicap": 20
                }
            }"#,
        );
        let bots = read(&preset);
        assert_eq!(bots.len(), 1);
        assert_eq!(bots[0].name, "BARb");
        assert_eq!(bots[0].ai_lib, "BARb");
        assert_eq!(bots[0].ally_number, 1);
        assert_eq!(bots[0].ai_version.as_deref(), Some("0.1"));
        assert_eq!(bots[0].side, 1);
        assert_eq!(bots[0].handicap, 20);
        // Red, as the lobby carries it: 0xBBGGRR.
        assert_eq!(bots[0].colour(), Some(0x0000FF));
    }

    #[test]
    fn what_is_missing_falls_back_rather_than_failing() {
        let preset = preset_with(r#"{ "Simple": { "aiLib": "NullAI", "allyNumber": 2 } }"#);
        let bots = read(&preset);
        assert_eq!(bots[0].side, 0);
        assert_eq!(bots[0].handicap, 0);
        assert_eq!(bots[0].colour(), None);
        assert!(bots[0].ai_options.is_empty());
    }

    #[test]
    fn an_entry_nobody_can_read_is_skipped_and_the_rest_still_come() {
        let preset = preset_with(r#"{ "bad": 7, "good": { "aiLib": "BARb", "allyNumber": 1 } }"#);
        let bots = read(&preset);
        assert_eq!(bots.len(), 1);
        assert_eq!(bots[0].name, "good");
    }

    #[test]
    fn keys_we_do_not_model_survive_a_round_trip() {
        let preset = preset_with(
            r#"{ "BARb": { "aiLib": "BARb", "allyNumber": 1, "somethingElse": "kept" } }"#,
        );
        let mut back = Preset::new("p", 0);
        write(&mut back, &read(&preset));
        assert_eq!(
            back.bots["BARb"]["somethingElse"],
            serde_json::json!("kept")
        );
    }

    #[test]
    fn a_colour_survives_both_ways() {
        let bot = Bot::new("a", "BARb", 1).with_colour(0x4b73f2);
        assert_eq!(bot.colour(), Some(0x4b73f2));
    }

    #[test]
    fn they_come_back_in_a_fixed_order_so_two_applies_are_one_game() {
        let preset = preset_with(
            r#"{
                "zeta": { "aiLib": "BARb", "allyNumber": 2 },
                "alpha": { "aiLib": "BARb", "allyNumber": 1 }
            }"#,
        );
        let bots = read(&preset);
        let names: Vec<&str> = bots.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta"]);
    }
}
