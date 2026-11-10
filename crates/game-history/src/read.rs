//! One replay's head, as the history keeps it: the game, and the setting it
//! was played with.
//!
//! A setting is the map and the modoptions, less what changes every game
//! without anyone choosing it. Tweak slots are kept by the hash of their blob,
//! so two games with the same tweaks are the same setting however large the
//! tweaks are, and the blobs themselves are stored once each.

use std::collections::BTreeMap;
use std::path::Path;

use content::demo::Head;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

/// Modoption keys that are not part of a setting. `date_*` is stamped on
/// every game, and `mapmetadata_*` is the map's own data written in by the
/// lobby; neither is something anybody set.
pub const IGNORED_PREFIXES: [&str; 2] = ["date_", "mapmetadata_"];

/// The SHA-256 of a blob, in lowercase hex.
pub type BlobId = String;

/// A tweak slot's blob, as a setting refers to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BlobRef {
	pub id: BlobId,
	/// The blob's length, base64 and all.
	pub bytes: usize,
	/// The tweak's leading `--` comment, which is how BAR names one.
	pub name: Option<String>,
}

/// A distinct setting: what a room would have to be set to to play it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
	pub map: String,
	/// Modoptions by key; a tweak slot's value is its [`BlobId`].
	pub values: BTreeMap<String, String>,
	/// The tweak slots among `values`.
	pub blobs: BTreeMap<String, BlobRef>,
}

/// Someone in the game. The account id only tells whether a game was ours;
/// the view shows names alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seat {
	pub name: String,
	pub account_id: Option<u32>,
	pub spectator: bool,
}

/// An AI in the game, with what it takes to seat it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSeat {
	/// What the room called it: `BARb 1`, `RaptorsAI(1)`.
	pub name: String,
	/// Which AI it is: `BARb`, `RaptorsAI`.
	pub ai: String,
	pub version: Option<String>,
	pub ally_team: u8,
	/// `0xBBGGRR`.
	pub colour: Option<u32>,
	/// The faction by name: `Armada`, `Random`.
	pub side: Option<String>,
	/// The bonus, in percent.
	pub handicap: u8,
	pub options: BTreeMap<String, String>,
}

/// Its AIs, each with its team's side, colour and bonus.
fn ai_seats(script: &recoil::script_read::Script) -> Vec<AiSeat> {
	script
		.ais
		.values()
		.map(|ai| {
			let team = ai.team.and_then(|team| script.teams.get(&team));
			AiSeat {
				name: ai.name.clone(),
				ai: ai.short_name.clone(),
				version: ai.version.clone(),
				ally_team: team
					.and_then(|team| team.ally_team)
					.and_then(|ally| u8::try_from(ally).ok())
					.unwrap_or(0),
				colour: team.and_then(|team| team.colour),
				side: team.and_then(|team| team.side.clone()),
				handicap: team.map_or(0, |team| team.handicap.min(100) as u8),
				options: ai.options.clone(),
			}
		})
		.collect()
}

/// One game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
	/// The engine's game id in hex, or `script:` and a hash of the start
	/// script for a replay whose header has none.
	pub id: String,
	/// Unix seconds, from the header.
	pub started: u64,
	/// The key of its [`Setting`].
	pub setting: String,
	pub players: Vec<Seat>,
	pub ais: Vec<AiSeat>,
	/// SPADS room settings, under the names presets use: `teamSize`, `nbTeams`.
	pub battle: BTreeMap<String, String>,
	/// Ally team to its box, `left, top, right, bottom` out of 200.
	pub start_boxes: BTreeMap<u8, [u16; 4]>,
	/// The start-box arrangement the game was played with, if one was set.
	pub start_box_override: Option<BlobId>,
	/// Seconds of game time; `None` for a game left before its end.
	pub game_seconds: Option<u32>,
	/// `Beyond All Reason test-31836-b250a16`.
	pub game_type: String,
	/// The replay file, while there is one.
	pub replay: Option<String>,
}

/// What one replay gives the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
	pub game: Game,
	pub setting: Setting,
	/// Each blob the game refers to, by id.
	pub blobs: BTreeMap<BlobId, String>,
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
	Sha256::digest(bytes)
		.iter()
		.map(|byte| format!("{byte:02x}"))
		.collect()
}

/// Whether a slot holds nothing: Chobby clears one with `0`.
fn cleared(value: &str) -> bool {
	value.is_empty() || value == "0"
}

/// A slot value that holds a tweak, as a setting refers to it; `None` for a
/// key that is not a tweak slot, or a slot that is cleared.
pub fn blob_ref(key: &str, value: &str) -> Option<BlobRef> {
	let slot = tweaks::Slot::parse(key)?;
	if cleared(value) {
		return None;
	}
	let name = match slot.kind() {
		tweaks::Kind::Boxes => None,
		kind => tweaks::base64url::decode(value, kind)
			.ok()
			.and_then(|decoded| tweaks::name::name(&decoded.text)),
	};
	Some(BlobRef {
		id: sha256_hex(value.as_bytes()),
		bytes: value.len(),
		name,
	})
}

fn ignored(key: &str) -> bool {
	IGNORED_PREFIXES
		.iter()
		.any(|prefix| key.starts_with(prefix))
}

/// The setting's identity: its map and every value, in key order.
fn setting_key(setting: &Setting) -> String {
	let mut canonical = format!("{}\n", setting.map);
	for (key, value) in &setting.values {
		canonical.push_str(&format!("{key}={value}\n"));
	}
	sha256_hex(canonical.as_bytes())[..16].to_owned()
}

/// Modoptions as a setting holds them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Normalized {
	/// What anybody set, a tweak slot's value being its blob's id.
	pub values: BTreeMap<String, String>,
	/// The tweak slots among `values`.
	pub blobs: BTreeMap<String, BlobRef>,
	/// Each blob, by id.
	pub texts: BTreeMap<BlobId, String>,
}

/// Modoptions, from a replay or from a room, by the one rule both are
/// compared under: leave out what nobody set, drop cleared tweak slots, and
/// keep each tweak by its blob's id.
pub fn normalize(modoptions: &BTreeMap<String, String>) -> Normalized {
	let mut normalized = Normalized::default();
	for (key, value) in modoptions.iter().filter(|(key, _)| !ignored(key)) {
		if tweaks::Slot::parse(key).is_none() {
			normalized.values.insert(key.clone(), value.clone());
			continue;
		}
		let Some(held) = blob_ref(key, value) else {
			continue;
		};
		normalized.values.insert(key.clone(), held.id.clone());
		normalized.texts.insert(held.id.clone(), value.clone());
		normalized.blobs.insert(key.clone(), held);
	}
	normalized
}

/// Reads one replay's head.
pub fn read(replay: &Path, head: &Head) -> Parsed {
	let script = recoil::script_read::parse(&head.script);

	let Normalized {
		values,
		blobs: refs,
		texts: mut blobs,
	} = normalize(&script.modoptions);
	let setting = Setting {
		map: script.map().unwrap_or_default().to_owned(),
		values,
		blobs: refs,
	};

	let start_box_override = script
		.modoptions
		.get(presets::apply::START_BOX_OVERRIDE)
		.filter(|value| !cleared(value))
		.map(|value| {
			let id = sha256_hex(value.as_bytes());
			blobs.insert(id.clone(), value.clone());
			id
		});

	let battle = [("teamsize", "teamSize"), ("nbteams", "nbTeams")]
		.into_iter()
		.filter_map(|(theirs, ours)| {
			let value = script.host_options.get(theirs)?;
			Some((ours.to_owned(), value.clone()))
		})
		.collect();

	let id = match head.game_id {
		Some(id) => id.iter().map(|byte| format!("{byte:02x}")).collect(),
		None => format!("script:{}", &sha256_hex(head.script.as_bytes())[..32]),
	};

	let game = Game {
		id,
		started: head.started,
		setting: setting_key(&setting),
		players: script
			.players
			.values()
			.map(|player| Seat {
				name: player.name.clone(),
				account_id: player.account_id,
				spectator: player.spectator,
			})
			.collect(),
		ais: ai_seats(&script),
		battle,
		start_boxes: script
			.boxes_out_of_200()
			.into_iter()
			.map(|(ally, (left, top, right, bottom))| (ally, [left, top, right, bottom]))
			.collect(),
		start_box_override,
		game_seconds: head.length.map(|length| length.game),
		game_type: script.game.get("gametype").cloned().unwrap_or_default(),
		replay: Some(replay.display().to_string()),
	};

	Parsed {
		game,
		setting,
		blobs,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn head(modoptions: &str) -> Head {
		head_with("Comet Catcher", "", modoptions)
	}

	fn head_with(map: &str, game: &str, modoptions: &str) -> Head {
		Head {
			game_id: Some([7; 16]),
			started: 1_762_000_000,
			length: None,
			script: format!(
				"[GAME]\n{{\n\tmapname={map};\n\tgametype=Beyond All Reason test-1;\n{game}\
				\t[MODOPTIONS]\n\t{{\n{modoptions}\t}}\n}}\n"
			),
		}
	}

	fn tweak(lua: &str) -> String {
		tweaks::base64url::encode(lua, tweaks::Kind::Defs).unwrap()
	}

	fn setting_of(head: &Head) -> Setting {
		read(Path::new("demos/a.sdfz"), head).setting
	}

	fn key_of(head: &Head) -> String {
		read(Path::new("demos/a.sdfz"), head).game.setting
	}

	#[test]
	fn what_changes_every_game_is_not_part_of_the_setting() {
		let one = head("\t\tdate_day=1;\n\t\tmapmetadata_type=land;\n\t\tdebugcommands=0;\n");
		let two = head("\t\tdate_day=2;\n\t\tdebugcommands=0;\n");
		assert_eq!(key_of(&one), key_of(&two));
		assert_eq!(
			setting_of(&one).values,
			BTreeMap::from([("debugcommands".into(), "0".into())])
		);
	}

	#[test]
	fn a_cleared_tweak_slot_is_no_slot_at_all() {
		let cleared = head("\t\ttweakdefs1=0;\n\t\ttweakunits2=;\n\t\tx=1;\n");
		let absent = head("\t\tx=1;\n");
		assert_eq!(key_of(&cleared), key_of(&absent));
		assert!(setting_of(&cleared).blobs.is_empty());
	}

	#[test]
	fn a_tweak_is_kept_by_the_hash_of_its_blob_and_named() {
		let blob = tweak("-- Sphere spawner\nlocal x = 1\n");
		let parsed = read(
			Path::new("demos/a.sdfz"),
			&head(&format!("\t\ttweakdefs={blob};\n")),
		);

		let held = &parsed.setting.blobs["tweakdefs"];
		assert_eq!(held.id, sha256_hex(blob.as_bytes()));
		assert_eq!(held.bytes, blob.len());
		assert_eq!(held.name.as_deref(), Some("Sphere spawner"));
		assert_eq!(parsed.setting.values["tweakdefs"], held.id);
		assert_eq!(parsed.blobs[&held.id], blob);
	}

	#[test]
	fn the_order_the_keys_were_written_in_does_not_matter() {
		assert_eq!(
			key_of(&head("\t\ta=1;\n\t\tb=2;\n")),
			key_of(&head("\t\tb=2;\n\t\ta=1;\n"))
		);
	}

	#[test]
	fn another_map_or_value_is_another_setting() {
		let here = head_with("Comet Catcher", "", "\t\ta=1;\n");
		assert_ne!(key_of(&here), key_of(&head_with("Forge", "", "\t\ta=1;\n")));
		assert_ne!(key_of(&here), key_of(&head("\t\ta=2;\n")));
	}

	#[test]
	fn the_game_is_known_by_its_engine_id_or_else_its_script() {
		let with_id = head("");
		assert_eq!(read(Path::new("a.sdfz"), &with_id).game.id, "07".repeat(16));

		let without = Head {
			game_id: None,
			..head("\t\ta=1;\n")
		};
		let id = read(Path::new("a.sdfz"), &without).game.id;
		assert!(id.starts_with("script:"));
		assert_eq!(id, read(Path::new("b.sdfz"), &without).game.id);
	}

	#[test]
	fn the_room_settings_and_seats_come_along() {
		let game = read(
			Path::new("demos/a.sdfz"),
			&head_with(
				"Forge",
				"\t[hostoptions]\n\t{\n\t\tteamsize=12;\n\t\tnbteams=1;\n\t}\n\
				 \t[player0]\n\t{\n\t\tname=tetrisface;\n\t\taccountid=42;\n\t\tspectator=0;\n\t}\n\
				 \t[ai0]\n\t{\n\t\tshortname=RaptorsAI;\n\t}\n\
				 \t[allyteam0]\n\t{\n\t\tstartrectleft=0;\n\t\tstartrecttop=0;\n\t\tstartrectright=0.5;\n\t\tstartrectbottom=1;\n\t}\n",
				"\t\tmapmetadata_startbox_override=QUJD;\n",
			),
		)
		.game;
		assert_eq!(game.battle["teamSize"], "12");
		assert_eq!(game.battle["nbTeams"], "1");
		assert_eq!(game.ais[0].ai, "RaptorsAI");
		assert_eq!(
			game.players,
			[Seat {
				name: "tetrisface".into(),
				account_id: Some(42),
				spectator: false,
			}]
		);
		assert_eq!(game.start_boxes[&0], [0, 0, 100, 200]);
		assert_eq!(game.start_box_override, Some(sha256_hex(b"QUJD")));
		assert_eq!(game.game_type, "Beyond All Reason test-1");
		assert_eq!(game.replay.as_deref(), Some("demos/a.sdfz"));
	}

	#[test]
	fn an_ai_comes_along_with_everything_needed_to_seat_it_again() {
		let game = read(
			Path::new("demos/a.sdfz"),
			&head_with(
				"Forge",
				"\t[ai1]\n\t{\n\t\tname=BARb 1;\n\t\tshortname=BARb;\n\t\tversion=stable;\n\t\tteam=9;\n\
				 \t\t[options]\n\t\t{\n\t\t\tprofile=hard;\n\t\t}\n\t}\n\
				 \t[team9]\n\t{\n\t\tallyteam=1;\n\t\trgbcolor=0.10980 0.36471 0.71765;\n\
				 \t\tside=Cortex;\n\t\thandicap=60;\n\t}\n",
				"",
			),
		)
		.game;
		assert_eq!(
			game.ais,
			[AiSeat {
				name: "BARb 1".into(),
				ai: "BARb".into(),
				version: Some("stable".into()),
				ally_team: 1,
				colour: Some(0xB75D1C),
				side: Some("Cortex".into()),
				handicap: 60,
				options: BTreeMap::from([("profile".into(), "hard".into())]),
			}]
		);
	}

	#[test]
	fn a_value_that_is_not_a_tweak_has_no_blob() {
		assert_eq!(blob_ref("debugcommands", "1"), None);
		assert_eq!(blob_ref("tweakdefs", "0"), None);
		assert!(blob_ref("tweakunits1", &tweak("{}")).is_some());
	}
}
