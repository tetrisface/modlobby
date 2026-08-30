//! Turning a preset into the lines that put a room back that way.
//!
//! Pure, and separate from sending, because what gets sent is the whole
//! question here. SPADS on the BAR cluster auto-ignores a client for four
//! minutes after eight commands in eight seconds
//! (`spads_config_bar/etc/barmanager_spads.conf:57`), so a preset with a
//! hundred settings in it is a couple of minutes of patient trickle no matter
//! who sends it — and every line that did not need sending is a second wasted
//! and a second closer to being ignored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::Preset;

/// Which parts of a preset to put back. Chobby offers the same five
/// (`gui_optionpresets_panel.lua`), because wanting somebody's modoptions
/// without their map is the normal case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Sections {
    pub map: bool,
    pub modoptions: bool,
    pub battle: bool,
    pub start_boxes: bool,
    pub bots: bool,
    /// Whether to reset the room to its SPADS preset first.
    ///
    /// `!preset <name>` is what clears everything the room already had, and
    /// Chobby always sends it. Turning it off is what lets two presets be
    /// applied one after the other and combine, rather than the second
    /// wiping the first.
    pub reset: bool,
}

impl Default for Sections {
    fn default() -> Self {
        Self {
            map: true,
            modoptions: true,
            battle: true,
            start_boxes: true,
            bots: false,
            reset: true,
        }
    }
}

/// What applying a preset would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Plan {
    /// Chat lines, in order. Every one is a SPADS command.
    pub lines: Vec<String>,
    /// Ally teams whose start box the preset names, and the box.
    pub start_boxes: Vec<PlannedBox>,
    /// Whether every start box in the room should be cleared first.
    pub clear_start_boxes: bool,
    /// Settings the room already has, which is why they are not in `lines`.
    pub already_set: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlannedBox {
    pub ally_team: u8,
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

/// The room as it stands, so a plan can leave out what is already true.
#[derive(Debug, Clone, Default)]
pub struct Room {
    pub map: Option<String>,
    pub modoptions: BTreeMap<String, String>,
}

/// The SPADS command for one room setting, where there is one.
///
/// These are not `!bSet` keys: SPADS keeps the room's own settings behind
/// their own commands, which is how Chobby sets them too.
fn battle_command(key: &str, value: &str) -> Option<String> {
    Some(match key {
        "locked" => match value {
            "1" | "true" | "locked" => "!lock".into(),
            _ => "!unlock".into(),
        },
        "autoBalance" => format!("!autoBalance {value}"),
        "balanceMode" => format!("!balanceMode {value}"),
        "teamSize" => format!("!set teamSize {value}"),
        "nbTeams" => format!("!nbTeams {value}"),
        // `preset` is the reset itself and is handled first, or not at all.
        "preset" => return None,
        _ => return None,
    })
}

/// Plans the application of a preset to a room.
pub fn plan(preset: &Preset, room: &Room, sections: Sections) -> Plan {
    let mut lines = Vec::new();
    let mut already_set = 0;

    if sections.battle {
        // First, because it is what clears the room.
        if sections.reset
            && let Some(name) = preset.battle.get("preset")
        {
            lines.push(format!("!preset {name}"));
        }
        for (key, value) in &preset.battle {
            if let Some(line) = battle_command(key, value) {
                lines.push(line);
            }
        }
    }

    if sections.map
        && let Some(map) = &preset.map
        && room.map.as_deref() != Some(map.as_str())
    {
        lines.push(format!("!map {map}"));
    }

    if sections.modoptions {
        for (key, value) in &preset.modoptions {
            // The one optimisation that matters: at roughly a command per
            // second, a setting the room already has costs a second for
            // nothing.
            if room.modoptions.get(key).map(String::as_str) == Some(value.as_str()) {
                already_set += 1;
                continue;
            }
            lines.push(format!("!bSet {key} {value}"));
        }
    }

    let start_boxes = if sections.start_boxes {
        preset
            .start_boxes
            .iter()
            .map(|(ally, held)| PlannedBox {
                ally_team: *ally,
                left: held.left,
                top: held.top,
                right: held.right,
                bottom: held.bottom,
            })
            .collect()
    } else {
        Vec::new()
    };

    Plan {
        // Clearing the old boxes belongs with resetting the room. Combining
        // two presets should add a side, not throw away the other one's.
        clear_start_boxes: sections.start_boxes && sections.reset && !start_boxes.is_empty(),
        start_boxes,
        lines,
        already_set,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StartBox;

    fn preset() -> Preset {
        let mut preset = Preset::new("raptors", 1);
        preset.map = Some("Comet Catcher Remake 1.8".into());
        preset
            .modoptions
            .insert("raptor_endless".into(), "1".into());
        preset
            .modoptions
            .insert("scav_spawncountmult".into(), "2".into());
        preset.battle.insert("preset".into(), "coop".into());
        preset.battle.insert("teamSize".into(), "8".into());
        preset.battle.insert("locked".into(), "1".into());
        preset.start_boxes.insert(
            0,
            StartBox {
                left: 0,
                top: 0,
                right: 50,
                bottom: 200,
            },
        );
        preset
    }

    #[test]
    fn a_setting_the_room_already_has_is_not_sent() {
        let mut room = Room::default();
        room.modoptions.insert("raptor_endless".into(), "1".into());
        let plan = plan(&preset(), &room, Sections::default());

        assert!(
            !plan
                .lines
                .iter()
                .any(|line| line.contains("raptor_endless"))
        );
        assert!(plan.lines.contains(&"!bSet scav_spawncountmult 2".into()));
        assert_eq!(plan.already_set, 1);
    }

    #[test]
    fn the_map_is_left_alone_when_the_room_is_already_on_it() {
        let room = Room {
            map: Some("Comet Catcher Remake 1.8".into()),
            ..Room::default()
        };
        let plan = plan(&preset(), &room, Sections::default());
        assert!(!plan.lines.iter().any(|line| line.starts_with("!map")));
    }

    #[test]
    fn resetting_is_the_first_thing_that_happens() {
        let plan = plan(&preset(), &Room::default(), Sections::default());
        assert_eq!(plan.lines[0], "!preset coop");
    }

    #[test]
    fn without_the_reset_two_presets_can_be_stacked() {
        let sections = Sections {
            reset: false,
            ..Sections::default()
        };
        let plan = plan(&preset(), &Room::default(), sections);
        // Nothing that would clear what a previous preset just put in.
        assert!(!plan.lines.iter().any(|line| line.starts_with("!preset")));
        assert!(!plan.clear_start_boxes);
        // But its own settings still go out.
        assert!(plan.lines.contains(&"!bSet raptor_endless 1".into()));
        assert_eq!(plan.start_boxes.len(), 1, "its own boxes are still added");
    }

    #[test]
    fn room_settings_use_their_own_commands_rather_than_bset() {
        let plan = plan(&preset(), &Room::default(), Sections::default());
        assert!(plan.lines.contains(&"!set teamSize 8".into()));
        assert!(plan.lines.contains(&"!lock".into()));
        assert!(
            !plan
                .lines
                .iter()
                .any(|line| line.contains("!bSet teamSize"))
        );
    }

    #[test]
    fn unlocking_is_what_a_preset_that_is_not_locked_asks_for() {
        let mut preset = preset();
        preset.battle.insert("locked".into(), "0".into());
        let plan = plan(&preset, &Room::default(), Sections::default());
        assert!(plan.lines.contains(&"!unlock".into()));
    }

    #[test]
    fn a_section_turned_off_contributes_nothing() {
        let sections = Sections {
            map: false,
            modoptions: false,
            battle: false,
            start_boxes: false,
            bots: false,
            reset: true,
        };
        let plan = plan(&preset(), &Room::default(), sections);
        assert!(plan.lines.is_empty());
        assert!(plan.start_boxes.is_empty());
    }
}
