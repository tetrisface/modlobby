//! Turning a preset into the lines that put a room back that way.
//!
//! Pure, and separate from sending, because what gets sent is the whole
//! question here. SPADS on the BAR cluster auto-ignores a client for four
//! minutes after eight commands in eight seconds
//! (`spads_config_bar/etc/barmanager_spads.conf:57`), so a preset with a
//! hundred settings in it is a couple of minutes of patient trickle no matter
//! who sends it — and every line that did not need sending is a second wasted
//! and a second closer to being ignored.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::bots;
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
	/// Ally teams whose start box the preset names, and the box. For showing
	/// what is about to happen; the wire form is one of the `lines`.
	pub start_boxes: Vec<PlannedBox>,
	/// Set when the preset has boxes that could not be turned into a line.
	pub start_boxes_unsent: bool,
	/// Settings the room already has, which is why they are not in `lines`.
	pub already_set: usize,
	/// AIs the load adds.
	pub bots: usize,
	/// Set when an AI added has options of its own, which a room on the
	/// server keeps nowhere.
	pub bot_options_unsent: bool,
	/// The AIs to add, for whoever carries the plan out.
	#[serde(skip)]
	#[ts(skip)]
	pub seats: Vec<Seating>,
}

/// An AI to add, as `ADDBOT` takes one, with the bonus that follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seating {
	pub name: String,
	pub ai: String,
	pub team: u8,
	pub ally_team: u8,
	pub colour: u32,
	pub handicap: u8,
}

/// An AI already in the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomBot {
	pub name: String,
	pub ai: String,
	pub ally_team: u8,
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
	/// The SPADS preset it runs under: what a reset goes back to when the
	/// preset names none of its own.
	pub preset: Option<String>,
	pub bots: Vec<RoomBot>,
	/// Team numbers held, by players and AIs alike.
	pub teams: BTreeSet<u8>,
}

/// The preset's AIs the room does not already have. One of the same kind on
/// the same side counts as there, so loading twice is the same game twice.
/// Each added one takes a team nobody holds and a name the room does not.
fn seatings(preset: &Preset, room: &Room) -> Vec<(Seating, bool)> {
	let mut present: Vec<&RoomBot> = room.bots.iter().collect();
	let mut names: BTreeSet<String> = room.bots.iter().map(|held| held.name.clone()).collect();
	let mut teams = room.teams.clone();
	let mut seats = Vec::new();
	for bot in bots::read(preset) {
		let there = present.iter().position(|held| {
			held.ally_team == bot.ally_number && held.ai.eq_ignore_ascii_case(&bot.ai_lib)
		});
		if let Some(at) = there {
			present.remove(at);
			continue;
		}
		let name = unused_name(&bot.ai_lib, &names);
		names.insert(name.clone());
		let team = (0..=u8::MAX)
			.find(|team| !teams.contains(team))
			.unwrap_or(u8::MAX);
		teams.insert(team);
		let fallback = bots::COLOURS[usize::from(bot.ally_number) % bots::COLOURS.len()];
		let seat = Seating {
			name,
			ai: bot.ai_lib.clone(),
			team,
			ally_team: bot.ally_number,
			colour: bot.colour().unwrap_or(fallback),
			handicap: bot.handicap,
		};
		seats.push((seat, !bot.ai_options.is_empty()));
	}
	seats
}

/// `BARb`, then `BARb2`, as the seat bar names them: one the room does not
/// hold, without the spaces the protocol splits a line on.
fn unused_name(ai: &str, taken: &BTreeSet<String>) -> String {
	let base: String = ai.chars().filter(|c| !c.is_whitespace()).collect();
	if !taken.contains(&base) {
		return base;
	}
	(2..)
		.map(|n| format!("{base}{n}"))
		.find(|name| !taken.contains(name))
		.unwrap_or(base)
}

/// The modoption BAR reads a custom startbox arrangement from.
pub const START_BOX_OVERRIDE: &str = "mapmetadata_startbox_override";

/// The map's per-team-count arrangements, sent after every override line.
const START_BOXES_SET: &str = "mapmetadata_startboxes_set";

/// The SPADS command for one room setting, where there is one.
///
/// These are not `!bSet` keys: SPADS keeps the room's own settings behind
/// their own commands, which is how Chobby sets them too.
fn battle_command(key: &str, value: &str) -> Option<String> {
	Some(match key {
		"autoBalance" => format!("!autoBalance {value}"),
		"balanceMode" => format!("!balanceMode {value}"),
		"teamSize" => format!("!set teamSize {value}"),
		"nbTeams" => format!("!nbTeams {value}"),
		// `preset` is the reset itself and is handled first, or not at all.
		// `locked` is the saver's room state, not a setting: a preset never
		// locks or unlocks the room it is loaded into.
		"preset" | "locked" => return None,
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
			&& let Some(name) = preset.battle.get("preset").or(room.preset.as_ref())
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

	// `!preset` deletes every modoption before setting its own (see
	// `resets_settings` in lobby-core), so after one nothing is already set.
	let wiped = lines
		.first()
		.is_some_and(|line| line.starts_with("!preset "));

	if sections.modoptions {
		for (key, value) in &preset.modoptions {
			// Goes out with the start boxes, below, as the one arrangement.
			if key == START_BOX_OVERRIDE {
				continue;
			}
			// The one optimisation that matters: at roughly a command per
			// second, a setting the room already has costs a second for
			// nothing.
			if !wiped && room.modoptions.get(key).map(String::as_str) == Some(value.as_str()) {
				already_set += 1;
				continue;
			}
			lines.push(format!("!bSet {key} {value}"));
		}
	}

	let mut start_boxes = Vec::new();
	let mut start_boxes_unsent = false;
	// The preset's own arrangement, polygons and all. Its rectangles are
	// Chobby's copy of the room's rectangles, which are not what BAR plays.
	let own = preset
		.modoptions
		.get(START_BOX_OVERRIDE)
		.filter(|blob| !blob.is_empty());
	if sections.start_boxes && (own.is_some() || !preset.start_boxes.is_empty()) {
		start_boxes = preset
			.start_boxes
			.iter()
			.map(|(ally, held)| PlannedBox {
				ally_team: *ally,
				left: held.left,
				top: held.top,
				right: held.right,
				bottom: held.bottom,
			})
			.collect();

		// Not `ADDSTARTRECT`. That is a mod command on teiserver — founder,
		// moderator or coordinator only (`lobby.ex:746`) — so from a boss it
		// is dropped in silence. The modoption BAR added for custom
		// arrangements is an ordinary `!bSet`, which a boss may send.
		let arrangement =
			startbox::Arrangement {
				startboxes: (0..=preset.start_boxes.keys().copied().max().unwrap_or(0))
					.map(|ally| {
						let held = preset.start_boxes.get(&ally).copied().unwrap_or(
							crate::model::StartBox {
								left: 0,
								top: 0,
								right: 200,
								bottom: 200,
							},
						);
						startbox::Box {
							poly: vec![
								startbox::Point {
									x: f32::from(held.left),
									y: f32::from(held.top),
									strength: None,
								},
								startbox::Point {
									x: f32::from(held.right),
									y: f32::from(held.bottom),
									strength: None,
								},
							],
						}
					})
					.collect(),
			};

		// Without the reset, an arrangement the room already has is left
		// alone: combining presets should not have the second one's boxes
		// silently replace the first one's.
		let occupied = room
			.modoptions
			.get(START_BOX_OVERRIDE)
			.is_some_and(|held| !held.is_empty());
		let encoded = match own {
			Some(blob) => Ok(blob.clone()),
			None => startbox::encode_override(&arrangement),
		};
		match encoded {
			Ok(encoded) if sections.reset || !occupied => {
				if !wiped && room.modoptions.get(START_BOX_OVERRIDE) == Some(&encoded) {
					already_set += 1;
				} else {
					lines.push(format!("!bSet {START_BOX_OVERRIDE} {encoded}"));
				}
			}
			Ok(_) => start_boxes_unsent = true,
			Err(_) => start_boxes_unsent = true,
		}
	}

	// Stable, so only the set moves: behind the override.
	let set = format!("!bSet {START_BOXES_SET} ");
	lines.sort_by_key(|line| line.starts_with(&set));

	let seated = if sections.bots {
		seatings(preset, room)
	} else {
		Vec::new()
	};
	Plan {
		start_boxes,
		start_boxes_unsent,
		lines,
		already_set,
		bots: seated.len(),
		bot_options_unsent: seated.iter().any(|(_, options)| *options),
		seats: seated.into_iter().map(|(seat, _)| seat).collect(),
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
		let sections = Sections {
			reset: false,
			..Sections::default()
		};
		let plan = plan(&preset(), &room, sections);

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
	fn after_the_reset_nothing_is_already_set() {
		let mut room = Room::default();
		room.modoptions.insert("raptor_endless".into(), "1".into());
		let plan = plan(&preset(), &room, Sections::default());
		// `!preset coop` deletes it before this line lands.
		assert!(plan.lines.contains(&"!bSet raptor_endless 1".into()));
		assert_eq!(plan.already_set, 0);
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
	fn a_preset_naming_no_preset_resets_the_room_to_the_one_it_runs() {
		let mut held = preset();
		held.battle.remove("preset");
		let room = Room {
			preset: Some("team".into()),
			..Room::default()
		};
		assert_eq!(
			plan(&held, &room, Sections::default()).lines[0],
			"!preset team"
		);
	}

	fn with_bots(bots: &[crate::Bot]) -> Preset {
		let mut held = preset();
		crate::bots::write(&mut held, bots);
		held
	}

	fn bots_on() -> Sections {
		Sections {
			bots: true,
			..Sections::default()
		}
	}

	#[test]
	fn the_ais_go_to_free_teams_under_names_the_room_does_not_hold() {
		let mut barb = crate::Bot::new("BARb 1", "BARb", 1).with_colour(0x0000FF);
		barb.handicap = 60;
		let room = Room {
			bots: vec![RoomBot {
				name: "BARb".into(),
				ai: "BARb".into(),
				ally_team: 0,
			}],
			teams: BTreeSet::from([0, 1]),
			..Room::default()
		};
		let raptors = crate::Bot::new("Raptors", "RaptorsAI", 1);
		let plan = plan(&with_bots(&[barb, raptors]), &room, bots_on());
		assert_eq!(plan.bots, 2);
		assert_eq!(
			plan.seats,
			[
				Seating {
					name: "BARb2".into(),
					ai: "BARb".into(),
					team: 2,
					ally_team: 1,
					colour: 0x0000FF,
					handicap: 60,
				},
				Seating {
					name: "RaptorsAI".into(),
					ai: "RaptorsAI".into(),
					team: 3,
					ally_team: 1,
					colour: crate::bots::COLOURS[1],
					handicap: 0,
				},
			]
		);
	}

	#[test]
	fn an_ai_the_room_already_has_on_that_side_is_not_added_again() {
		let room = Room {
			bots: vec![RoomBot {
				name: "RaptorsAI".into(),
				ai: "raptorsai".into(),
				ally_team: 1,
			}],
			..Room::default()
		};
		let two = [
			crate::Bot::new("RaptorsAI(1)", "RaptorsAI", 1),
			crate::Bot::new("RaptorsAI(2)", "RaptorsAI", 1),
		];
		let plan = plan(&with_bots(&two), &room, bots_on());
		assert_eq!(plan.bots, 1, "one is there already; the second is added");
		assert_eq!(plan.seats[0].name, "RaptorsAI2");
	}

	#[test]
	fn an_added_ais_own_options_are_said_to_stay_behind() {
		let mut barb = crate::Bot::new("BARb", "BARb", 1);
		barb.ai_options.insert("profile".into(), "hard".into());
		assert!(plan(&with_bots(&[barb]), &Room::default(), bots_on()).bot_options_unsent);
		let plain = crate::Bot::new("BARb", "BARb", 1);
		assert!(!plan(&with_bots(&[plain]), &Room::default(), bots_on()).bot_options_unsent);
	}

	#[test]
	fn the_ais_stay_out_unless_asked_for() {
		let held = with_bots(&[crate::Bot::new("BARb", "BARb", 1)]);
		let plan = plan(&held, &Room::default(), Sections::default());
		assert_eq!(plan.bots, 0);
		assert!(plan.seats.is_empty());
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
		assert!(!plan.start_boxes_unsent);
		// But its own settings still go out.
		assert!(plan.lines.contains(&"!bSet raptor_endless 1".into()));
		assert_eq!(plan.start_boxes.len(), 1, "its own boxes are still added");
	}

	#[test]
	fn room_settings_use_their_own_commands_rather_than_bset() {
		let plan = plan(&preset(), &Room::default(), Sections::default());
		assert!(plan.lines.contains(&"!set teamSize 8".into()));
		assert!(
			!plan
				.lines
				.iter()
				.any(|line| line.contains("!bSet teamSize"))
		);
	}

	#[test]
	fn a_preset_never_locks_or_unlocks_the_room() {
		for locked in ["1", "0"] {
			let mut preset = preset();
			preset.battle.insert("locked".into(), locked.into());
			let plan = plan(&preset, &Room::default(), Sections::default());
			assert!(
				!plan
					.lines
					.iter()
					.any(|line| line == "!lock" || line == "!unlock")
			);
		}
	}

	#[test]
	fn the_presets_own_arrangement_wins_over_its_rectangles() {
		let mut preset = preset();
		preset
			.modoptions
			.insert(START_BOX_OVERRIDE.into(), "theirs".into());
		let plan = plan(&preset, &Room::default(), Sections::default());
		let sent: Vec<_> = plan
			.lines
			.iter()
			.filter(|line| line.contains(START_BOX_OVERRIDE))
			.collect();
		assert_eq!(sent, [&format!("!bSet {START_BOX_OVERRIDE} theirs")]);
	}

	#[test]
	fn the_map_set_goes_after_every_override() {
		let mut preset = preset();
		preset
			.modoptions
			.insert(START_BOX_OVERRIDE.into(), "theirs".into());
		preset
			.modoptions
			.insert(START_BOXES_SET.into(), "set".into());
		let plan = plan(&preset, &Room::default(), Sections::default());
		let at = |prefix: String| {
			plan.lines
				.iter()
				.rposition(|line| line.starts_with(&prefix))
				.expect("a line")
		};
		let set = at(format!("!bSet {START_BOXES_SET} "));
		assert_eq!(set, plan.lines.len() - 1);
		assert!(at(format!("!bSet {START_BOX_OVERRIDE} ")) < set);
	}

	#[test]
	fn start_boxes_go_out_as_the_modoption_a_boss_may_set() {
		let plan = plan(&preset(), &Room::default(), Sections::default());
		let line = plan
			.lines
			.iter()
			.find(|line| line.contains(START_BOX_OVERRIDE))
			.expect("an arrangement line");
		// Not ADDSTARTRECT: teiserver takes that from the founder only, and
		// drops it from anybody else without a word.
		assert!(line.starts_with("!bSet "));
		assert!(!plan.start_boxes_unsent);
		assert_eq!(plan.start_boxes.len(), 1, "and still shown as boxes");
	}

	#[test]
	fn an_arrangement_the_room_already_has_is_not_sent_again() {
		let first = plan(&preset(), &Room::default(), Sections::default());
		let encoded = first
			.lines
			.iter()
			.find_map(|line| line.strip_prefix(&format!("!bSet {START_BOX_OVERRIDE} ")))
			.expect("an arrangement")
			.to_owned();

		let mut room = Room::default();
		room.modoptions.insert(START_BOX_OVERRIDE.into(), encoded);
		// No `!preset` to wipe it first.
		let mut preset = preset();
		preset.battle.remove("preset");
		let again = plan(&preset, &room, Sections::default());
		assert!(
			!again
				.lines
				.iter()
				.any(|line| line.contains(START_BOX_OVERRIDE))
		);
	}

	#[test]
	fn combining_leaves_an_arrangement_the_room_already_has_alone() {
		let mut room = Room::default();
		room.modoptions
			.insert(START_BOX_OVERRIDE.into(), "somebody else's".into());
		let sections = Sections {
			reset: false,
			..Sections::default()
		};
		let plan = plan(&preset(), &room, sections);
		assert!(
			!plan
				.lines
				.iter()
				.any(|line| line.contains(START_BOX_OVERRIDE))
		);
		// And says so, rather than looking as though it applied.
		assert!(plan.start_boxes_unsent);
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
