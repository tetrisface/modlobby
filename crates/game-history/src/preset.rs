//! A setting from the history as a preset, so that loading one goes the way
//! loading a preset does.

use std::collections::BTreeMap;

use presets::apply::START_BOX_OVERRIDE;
use presets::{Bot, Preset, Stamp, StartBox};

use crate::Error;
use crate::index::Index;
use crate::read::AiSeat;
use crate::store::Store;

/// The setting `key` as a preset named `name`, leaving out the modoptions in
/// `skip`. Its blobs are put back in place of their ids, and the room
/// settings and start boxes come from the newest game played with it.
pub fn preset(
	index: &Index,
	store: &Store,
	key: &str,
	name: &str,
	now: Stamp,
	skip: &[String],
) -> Result<Preset, Error> {
	let setting = index
		.settings
		.get(key)
		.ok_or_else(|| Error::Unknown(key.to_owned()))?;
	let kept = |key: &str| !skip.iter().any(|skipped| skipped == key);

	let mut modoptions = BTreeMap::new();
	for (key, value) in setting.values.iter().filter(|(key, _)| kept(key)) {
		let value = match setting.blobs.get(key) {
			Some(held) => store.blob(&held.id)?,
			None => value.clone(),
		};
		modoptions.insert(key.clone(), value);
	}

	let newest = index
		.games
		.values()
		.filter(|game| game.setting == key)
		.max_by_key(|game| game.started);
	let Some(game) = newest else {
		return Ok(Preset {
			map: Some(setting.map.clone()),
			modoptions,
			..Preset::new(name, now)
		});
	};
	if let Some(id) = &game.start_box_override
		&& kept(START_BOX_OVERRIDE)
	{
		modoptions.insert(START_BOX_OVERRIDE.to_owned(), store.blob(id)?);
	}
	let mut preset = Preset {
		map: Some(setting.map.clone()),
		modoptions,
		battle: game.battle.clone(),
		start_boxes: game
			.start_boxes
			.iter()
			.map(|(ally, [left, top, right, bottom])| {
				(
					*ally,
					StartBox {
						left: *left,
						top: *top,
						right: *right,
						bottom: *bottom,
					},
				)
			})
			.collect(),
		..Preset::new(name, now)
	};
	presets::bots::write(&mut preset, &bots(&game.ais));
	Ok(preset)
}

/// A game's AIs as a preset holds them. The side stays behind: a preset
/// keeps one by the game's own numbering, which a replay does not record.
fn bots(seats: &[AiSeat]) -> Vec<Bot> {
	seats
		.iter()
		.map(|seat| {
			let mut bot = Bot::new(&seat.name, &seat.ai, seat.ally_team);
			if let Some(colour) = seat.colour {
				bot = bot.with_colour(colour);
			}
			bot.ai_version = seat.version.clone();
			bot.handicap = seat.handicap;
			bot.ai_options = seat
				.options
				.iter()
				.map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
				.collect();
			bot
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	use content::demo::Head;

	use super::*;
	use crate::index::{Read, merge};

	struct Fixture {
		_dir: tempfile::TempDir,
		store: Store,
		index: Index,
		blob: String,
	}

	fn fixture() -> Fixture {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::new(dir.path().join("history"));
		let blob = tweaks::base64url::encode("-- Mine\nlocal x = 1\n", tweaks::Kind::Defs).unwrap();
		let game = |id: u8, teamsize: u8, right: &str| {
			let profile = if id == 2 { "hard" } else { "easy" };
			let head = Head {
				game_id: Some([id; 16]),
				started: 1_762_000_000 + u64::from(id),
				length: None,
				script: format!(
					"[GAME]\n{{\n\tmapname=Forge;\n\
					 \t[hostoptions]\n\t{{\n\t\tteamsize={teamsize};\n\t\tnbteams=2;\n\t}}\n\
					 \t[ai0]\n\t{{\n\t\tname=BARb 1;\n\t\tshortname=BARb;\n\t\tversion=stable;\n\t\tteam=1;\n\
					 \t\t[options]\n\t\t{{\n\t\t\tprofile={profile};\n\t\t}}\n\t}}\n\
					 \t[team1]\n\t{{\n\t\tallyteam=1;\n\t\trgbcolor=1 0 0;\n\t\thandicap=60;\n\t}}\n\
					 \t[allyteam0]\n\t{{\n\t\tstartrectleft=0;\n\t\tstartrecttop=0;\n\t\tstartrectright={right};\n\t\tstartrectbottom=1;\n\t}}\n\
					 \t[MODOPTIONS]\n\t{{\n\t\ttweakdefs={blob};\n\t\tdebugcommands=1;\n\t\tdate_day=3;\n\
					 \t\tmapmetadata_startboxes_set=1;\n\t\tmapmetadata_startbox_override=QUJD{id};\n\t}}\n}}\n"
				),
			};
			let parsed = crate::read::read(Path::new("a.sdfz"), &head);
			for (id, text) in &parsed.blobs {
				store.put_blob(id, text).unwrap();
			}
			Read {
				path: format!("{id}.sdfz"),
				bytes: 1,
				modified: None,
				parsed: Some(parsed),
			}
		};
		let index = merge(
			&Index::default(),
			&content::ReplayListing::default(),
			&[game(1, 4, "0.25"), game(2, 8, "0.5")],
		);
		Fixture {
			_dir: dir,
			store,
			index,
			blob,
		}
	}

	fn key(fixture: &Fixture) -> String {
		fixture.index.settings.keys().next().unwrap().clone()
	}

	#[test]
	fn the_tweaks_go_back_in_as_their_blobs() {
		let fixture = fixture();
		let preset = preset(
			&fixture.index,
			&fixture.store,
			&key(&fixture),
			"Old",
			5,
			&[],
		)
		.unwrap();
		assert_eq!(preset.name, "Old");
		assert_eq!(preset.map.as_deref(), Some("Forge"));
		assert_eq!(preset.modoptions["tweakdefs"], fixture.blob);
		assert_eq!(preset.modoptions["debugcommands"], "1");
		assert_eq!(preset.created, 5);
	}

	#[test]
	fn what_the_room_never_set_stays_out() {
		let fixture = fixture();
		let preset = preset(
			&fixture.index,
			&fixture.store,
			&key(&fixture),
			"Old",
			5,
			&[],
		)
		.unwrap();
		assert!(!preset.modoptions.contains_key("date_day"));
		assert!(!preset.modoptions.contains_key("mapmetadata_startboxes_set"));
	}

	#[test]
	fn the_room_and_its_boxes_come_from_the_newest_game() {
		let fixture = fixture();
		let preset = preset(
			&fixture.index,
			&fixture.store,
			&key(&fixture),
			"Old",
			5,
			&[],
		)
		.unwrap();
		assert_eq!(preset.battle["teamSize"], "8");
		assert_eq!(preset.battle["nbTeams"], "2");
		assert_eq!(
			preset.start_boxes[&0],
			StartBox {
				left: 0,
				top: 0,
				right: 100,
				bottom: 200
			}
		);
		assert_eq!(
			preset.modoptions[presets::apply::START_BOX_OVERRIDE],
			"QUJD2"
		);
	}

	#[test]
	fn the_ais_come_from_the_newest_game_with_their_bonus_colour_and_options() {
		let fixture = fixture();
		let preset = preset(
			&fixture.index,
			&fixture.store,
			&key(&fixture),
			"Old",
			5,
			&[],
		)
		.unwrap();
		let bots = presets::bots::read(&preset);
		assert_eq!(bots.len(), 1);
		let bot = &bots[0];
		assert_eq!(
			(bot.name.as_str(), bot.ai_lib.as_str(), bot.ally_number),
			("BARb 1", "BARb", 1)
		);
		assert_eq!(bot.ai_version.as_deref(), Some("stable"));
		assert_eq!(bot.colour(), Some(0x0000FF));
		assert_eq!(bot.handicap, 60);
		assert_eq!(bot.ai_options["profile"], "hard");
	}

	#[test]
	fn skipped_options_are_left_out() {
		let fixture = fixture();
		let skip = ["debugcommands".to_owned()];
		let preset = preset(
			&fixture.index,
			&fixture.store,
			&key(&fixture),
			"Old",
			5,
			&skip,
		)
		.unwrap();
		assert!(!preset.modoptions.contains_key("debugcommands"));
		assert!(preset.modoptions.contains_key("tweakdefs"));
	}

	#[test]
	fn a_setting_the_history_does_not_have_says_so() {
		let fixture = fixture();
		assert!(matches!(
			preset(&fixture.index, &fixture.store, "nope", "Old", 5, &[]),
			Err(Error::Unknown(_))
		));
	}
}
