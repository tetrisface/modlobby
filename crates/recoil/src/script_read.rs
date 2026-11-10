//! Reading a start script back.
//!
//! The other half of [`crate::script`], which writes them. A start script is
//! the engine's own description of a game: nested `[SECTION] { key=value; }`
//! blocks, one of which is `[MODOPTIONS]` and several of which are
//! `[ALLYTEAM0]`, `[ALLYTEAM1]` and so on carrying start boxes.
//!
//! Deliberately forgiving. These are written by the engine, by SPADS, by
//! autohosts and by hand, and the differences between them are all in the
//! whitespace and the trailing semicolons. A section this does not recognise
//! is skipped rather than being a parse failure — the point is to recover a
//! room setup, not to validate somebody's file.

use std::collections::BTreeMap;

/// What a start script says, as far as we care.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
	/// `[GAME]` keys, lowercased. `mapname`, `gametype` and the rest.
	pub game: BTreeMap<String, String>,
	/// `[MODOPTIONS]` keys, lowercased, which is what `!bSet` sets.
	pub modoptions: BTreeMap<String, String>,
	/// Ally team number to its start box, in the 0-1 fractions the script
	/// uses rather than the 0-200 the lobby protocol uses.
	pub start_boxes: BTreeMap<u8, Fractions>,
	/// `[PLAYERn]`, by `n`: everyone in the game, spectators included.
	pub players: BTreeMap<u32, Player>,
	/// `[AIn]`, by `n`.
	pub ais: BTreeMap<u32, Ai>,
	/// `[TEAMn]`, by `n`: the side, colour and bonus of whoever plays team `n`.
	pub teams: BTreeMap<u32, Team>,
	/// `[HOSTOPTIONS]` keys, lowercased: what SPADS was set to, `teamsize`
	/// and `nbteams` among them. Only an autohost writes them.
	pub host_options: BTreeMap<String, String>,
}

/// A person in the game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Player {
	pub name: String,
	/// The server's id for them; `None` in a game with no server.
	pub account_id: Option<u32>,
	pub spectator: bool,
	pub team: Option<u32>,
}

/// An AI in the game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ai {
	/// What the room called it: `RaptorsAI(1)`.
	pub name: String,
	/// Which AI it is: `RaptorsAI`, `BARb`, `ScavengersAI`.
	pub short_name: String,
	/// An engine AI's version: `stable`. A Lua AI has none.
	pub version: Option<String>,
	pub team: Option<u32>,
	/// Its own options, from the `[OPTIONS]` inside it: BARb's `profile`.
	pub options: BTreeMap<String, String>,
}

/// One team: what a player or an AI plays as.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Team {
	pub ally_team: Option<u32>,
	/// `0xBBGGRR`, as the lobby protocol carries a colour.
	pub colour: Option<u32>,
	/// The faction by name: `Armada`, `Cortex`, `Random`.
	pub side: Option<String>,
	/// The bonus, in percent.
	pub handicap: u32,
}

/// `r g b` in 0-1 floats, as `rgbcolor` writes one, as `0xBBGGRR`.
fn colour(text: &str) -> Option<u32> {
	let channels: Vec<f32> = text
		.split_whitespace()
		.map(str::parse)
		.collect::<Result<_, _>>()
		.ok()?;
	let [red, green, blue] = channels[..] else {
		return None;
	};
	let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
	Some(byte(red) | (byte(green) << 8) | (byte(blue) << 16))
}

/// A start box as a start script states it: fractions of the map.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Fractions {
	pub left: f32,
	pub top: f32,
	pub right: f32,
	pub bottom: f32,
}

impl Eq for Fractions {}

impl Script {
	pub fn map(&self) -> Option<&str> {
		self.game.get("mapname").map(String::as_str)
	}

	/// The start boxes in the lobby protocol's units, which is what a preset
	/// and `ADDSTARTRECT` both use.
	pub fn boxes_out_of_200(&self) -> BTreeMap<u8, (u16, u16, u16, u16)> {
		let scale = |value: f32| (value.clamp(0.0, 1.0) * 200.0).round() as u16;
		self.start_boxes
			.iter()
			.map(|(ally, held)| {
				(
					*ally,
					(
						scale(held.left),
						scale(held.top),
						scale(held.right),
						scale(held.bottom),
					),
				)
			})
			.collect()
	}
}

/// Splits `key=value` the way the engine does: at the first `=` only, so a
/// base64 tweak with padding in it survives.
fn pair(line: &str) -> Option<(String, String)> {
	let line = line.trim().trim_end_matches(';').trim();
	let (key, value) = line.split_once('=')?;
	let key = key.trim().to_ascii_lowercase();
	if key.is_empty() {
		return None;
	}
	Some((key, value.trim().to_owned()))
}

/// Reads a start script.
pub fn parse(text: &str) -> Script {
	let mut script = Script::default();
	// The section we are inside, innermost last.
	let mut path: Vec<String> = Vec::new();
	let mut pending: Option<String> = None;

	for raw in text.lines() {
		let line = raw.trim();
		if line.is_empty() || line.starts_with("//") {
			continue;
		}
		if let Some(rest) = line.strip_prefix('[')
			&& let Some((name, after)) = rest.split_once(']')
		{
			let name = name.trim().to_ascii_lowercase();
			// The engine puts the brace on the next line. People do not always.
			if after.trim_start().starts_with('{') {
				path.push(name);
			} else {
				pending = Some(name);
			}
			continue;
		}
		if line.starts_with('{') {
			path.push(pending.take().unwrap_or_default());
			continue;
		}
		if line.starts_with('}') {
			path.pop();
			continue;
		}

		let Some((key, value)) = pair(line) else {
			continue;
		};
		let Some(section) = path.last() else {
			continue;
		};
		// An AI's own options are a section inside the AI's.
		if section == "options"
			&& let Some(n) = path
				.len()
				.checked_sub(2)
				.and_then(|parent| numbered(&path[parent], "ai"))
		{
			script.ais.entry(n).or_default().options.insert(key, value);
			continue;
		}

		match section.as_str() {
			"game" => {
				script.game.insert(key, value);
			}
			"modoptions" => {
				script.modoptions.insert(key, value);
			}
			"hostoptions" => {
				script.host_options.insert(key, value);
			}
			name if name.starts_with("allyteam") => {
				let Ok(ally) = name["allyteam".len()..].parse::<u8>() else {
					continue;
				};
				let Ok(number) = value.parse::<f32>() else {
					continue;
				};
				let held = script.start_boxes.entry(ally).or_default();
				match key.as_str() {
					"startrectleft" => held.left = number,
					"startrecttop" => held.top = number,
					"startrectright" => held.right = number,
					"startrectbottom" => held.bottom = number,
					_ => {}
				}
			}
			name => seat(&mut script, name, &key, value),
		}
	}
	script
}

/// `n` from a section named `<prefix>n`.
fn numbered(section: &str, prefix: &str) -> Option<u32> {
	section.strip_prefix(prefix)?.parse().ok()
}

/// A key of a `[PLAYERn]`, `[AIn]` or `[TEAMn]` section, kept on that seat.
/// Any other section is not ours to read.
fn seat(script: &mut Script, section: &str, key: &str, value: String) {
	if let Some(n) = numbered(section, "team") {
		let team = script.teams.entry(n).or_default();
		match key {
			"allyteam" => team.ally_team = value.parse().ok(),
			"rgbcolor" => team.colour = colour(&value),
			"side" => team.side = Some(value),
			"handicap" => team.handicap = value.parse().unwrap_or(0),
			_ => {}
		}
		return;
	}
	if let Some(n) = numbered(section, "player") {
		let player = script.players.entry(n).or_default();
		match key {
			"name" => player.name = value,
			"accountid" => player.account_id = value.parse().ok(),
			"spectator" => player.spectator = value == "1",
			"team" => player.team = value.parse().ok(),
			_ => {}
		}
		return;
	}
	if let Some(n) = numbered(section, "ai") {
		let ai = script.ais.entry(n).or_default();
		match key {
			"name" => ai.name = value,
			"shortname" => ai.short_name = value,
			"version" => ai.version = Some(value),
			"team" => ai.team = value.parse().ok(),
			_ => {}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const SCRIPT: &str = r#"
[GAME]
{
	MapName=Supreme Isthmus v2.1;
	GameType=Beyond All Reason test-31115;
	HostIP=203.0.113.7;

	[MODOPTIONS]
	{
		raptor_endless=1;
		tweakdefs1=LS1OdXR0eUIgdjEuNTI=;
		scav_spawncountmult=2;
	}

	[ALLYTEAM0]
	{
		numallies=0;
		startrectleft=0;
		startrecttop=0;
		startrectright=0.25;
		startrectbottom=1;
	}

	[ALLYTEAM1]
	{
		numallies=0;
		startrectleft=0.75;
		startrecttop=0;
		startrectright=1;
		startrectbottom=1;
	}

	[PLAYER0]
	{
		Name=tetrisface;
	}
}
"#;

	#[test]
	fn the_map_and_the_modoptions_come_back() {
		let script = parse(SCRIPT);
		assert_eq!(script.map(), Some("Supreme Isthmus v2.1"));
		assert_eq!(script.modoptions["raptor_endless"], "1");
		assert_eq!(script.modoptions["scav_spawncountmult"], "2");
		// A tweak's base64 keeps its padding: the split is at the first `=`.
		assert_eq!(script.modoptions["tweakdefs1"], "LS1OdXR0eUIgdjEuNTI=");
	}

	#[test]
	fn a_players_name_is_not_a_modoption() {
		let script = parse(SCRIPT);
		assert!(!script.modoptions.contains_key("name"));
		assert!(
			!script.game.contains_key("name"),
			"PLAYER0 is its own section"
		);
	}

	#[test]
	fn start_boxes_come_back_in_the_units_the_lobby_uses() {
		let boxes = parse(SCRIPT).boxes_out_of_200();
		assert_eq!(boxes[&0], (0, 0, 50, 200));
		assert_eq!(boxes[&1], (150, 0, 200, 200));
	}

	#[test]
	fn a_script_written_on_one_line_per_brace_reads_the_same() {
		let compact = "[GAME] {\nMapName=X;\n[MODOPTIONS] {\na=1;\nb=2;\n}\n}\n";
		let script = parse(compact);
		assert_eq!(script.map(), Some("X"));
		assert_eq!(script.modoptions["b"], "2");
	}

	const SEATS: &str = "[GAME]\n{\n\tautohostname=Host[US1][002];\n\
		\t[PLAYER0]\n\t{\n\t\tname=tetrisface;\n\t\taccountid=1234;\n\t\tteam=0;\n\t\tspectator=0;\n\t}\n\
		\t[player3]\n\t{\n\t\tname=Watcher;\n\t\tspectator=1;\n\t}\n\
		\t[AI0]\n\t{\n\t\tname=RaptorsAI(1);\n\t\tshortname=RaptorsAI;\n\t\tteam=12;\n\t\thost=0;\n\t}\n\
		\t[hostoptions]\n\t{\n\t\tteamsize=12;\n\t\tnbteams=1;\n\t\tbalancemode=clan:skill;\n\t}\n}\n";

	#[test]
	fn the_players_come_back_with_their_seat_and_account() {
		let script = parse(SEATS);
		assert_eq!(
			script.players[&0],
			Player {
				name: "tetrisface".into(),
				account_id: Some(1234),
				spectator: false,
				team: Some(0),
			}
		);
		assert_eq!(
			script.players[&3],
			Player {
				name: "Watcher".into(),
				account_id: None,
				spectator: true,
				team: None,
			}
		);
		assert!(
			!script.game.contains_key("name"),
			"a player's keys stay in the player"
		);
	}

	#[test]
	fn the_ais_come_back_with_their_kind_and_team() {
		assert_eq!(
			parse(SEATS).ais[&0],
			Ai {
				name: "RaptorsAI(1)".into(),
				short_name: "RaptorsAI".into(),
				team: Some(12),
				..Ai::default()
			}
		);
	}

	const TEAMS: &str = "[GAME]\n{\n\
		\t[ai1]\n\t{\n\t\tname=BARb 1;\n\t\tshortname=BARb;\n\t\tversion=stable;\n\t\tteam=9;\n\
		\t\t[options]\n\t\t{\n\t\t\tprofile=hard;\n\t\t}\n\t}\n\
		\t[team9]\n\t{\n\t\tallyteam=1;\n\t\tteamleader=0;\n\t\trgbcolor=0.10980 0.36471 0.71765;\n\
		\t\tside=Cortex;\n\t\thandicap=60;\n\t}\n}\n";

	#[test]
	fn an_ais_version_and_options_come_back_with_it() {
		let ai = &parse(TEAMS).ais[&1];
		assert_eq!(ai.version.as_deref(), Some("stable"));
		assert_eq!(
			ai.options,
			BTreeMap::from([("profile".to_owned(), "hard".to_owned())])
		);
		assert_eq!(ai.team, Some(9), "the options are not the AI's own keys");
	}

	#[test]
	fn a_team_comes_back_with_its_side_colour_and_bonus() {
		assert_eq!(
			parse(TEAMS).teams[&9],
			Team {
				ally_team: Some(1),
				colour: Some(0xB75D1C),
				side: Some("Cortex".into()),
				handicap: 60,
			}
		);
	}

	#[test]
	fn the_host_options_come_back() {
		let script = parse(SEATS);
		assert_eq!(script.host_options["teamsize"], "12");
		assert_eq!(script.host_options["balancemode"], "clan:skill");
		assert_eq!(script.game["autohostname"], "Host[US1][002]");
	}

	#[test]
	fn nothing_recognisable_yields_nothing_rather_than_failing() {
		let script = parse("this is not a start script at all");
		assert!(script.game.is_empty());
		assert!(script.modoptions.is_empty());
	}
}
