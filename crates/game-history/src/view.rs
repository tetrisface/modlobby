//! The history as the webview gets it: one row per setting, each with its
//! games and who else played them, by name only.
//!
//! Most settings share most values, so each row carries only where it
//! departs from the most common value of each key (`usual`). That is
//! lossless, takes the list from about 1.9 MB to about 0.6 MB, and is also
//! what makes a row's "unusual here" line.

use std::collections::BTreeMap;

use serde::Serialize;
use ts_rs::TS;

use crate::index::Index;
use crate::read::{Game, IGNORED_PREFIXES, Seat, Setting};

/// Who "me" is: every name and account id this machine logs in with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
	pub names: Vec<String>,
	pub ids: Vec<u32>,
}

/// What I was in a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Role {
	Played,
	Spectated,
	/// Joined after the game began. A start script lists who was in the room
	/// at the start, so a replay recorded here that does not name me is a
	/// game I came into later: to watch, or to take a seat a leaver left.
	Late,
	/// Nobody to compare with: no account known, or no players recorded.
	Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GameRow {
	/// Unix seconds.
	#[ts(type = "number")]
	pub started: u64,
	pub role: Role,
	pub game_seconds: Option<u32>,
	/// The AIs' short names.
	pub ais: Vec<String>,
	/// Who else had a seat, by name, in the start script's order. What makes
	/// a game memorable; account ids stay behind.
	pub players: Vec<String>,
	pub game_type: String,
	/// The replay, while it is still on the disk.
	pub replay: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SettingRow {
	pub key: String,
	pub map: String,
	/// Each key whose value is not the usual one: its value, or `null` where
	/// this setting lacks a key the usual setting has. A tweak slot's value is
	/// its blob's id; see [`HistoryView::blobs`].
	pub differs: BTreeMap<String, Option<String>>,
	/// Newest first.
	pub games: Vec<GameRow>,
}

/// A tweak blob, described once however many settings use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BlobInfo {
	pub bytes: usize,
	pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryView {
	/// Most recently played first.
	pub settings: Vec<SettingRow>,
	/// Each key's most common value across the settings.
	pub usual: BTreeMap<String, String>,
	/// Every blob a setting refers to, by id.
	pub blobs: BTreeMap<String, BlobInfo>,
	/// Keys that are never part of a setting, by prefix.
	pub ignored_prefixes: Vec<String>,
}

/// A room's modoptions as a setting would hold them, to compare with one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValuesView {
	/// A tweak slot's value is its blob's id.
	pub values: BTreeMap<String, String>,
	/// The room's blobs, by id.
	pub blobs: BTreeMap<String, BlobInfo>,
}

/// `modoptions` held to the rule a game's are: see [`crate::read::normalize`].
pub fn values_view(modoptions: &BTreeMap<String, String>) -> ValuesView {
	let normalized = crate::read::normalize(modoptions);
	ValuesView {
		blobs: normalized
			.blobs
			.into_values()
			.map(|held| {
				(
					held.id,
					BlobInfo {
						bytes: held.bytes,
						name: held.name,
					},
				)
			})
			.collect(),
		values: normalized.values,
	}
}

/// What `me` was in `game`.
///
/// A seat is mine by name, or by account id where a rename left the name
/// behind. A replay does not record its server, so an id from one server
/// can match a stranger's on another; that only ever marks a game, it never
/// decides anything.
pub fn role(game: &Game, me: &Identity) -> Role {
	if game.players.is_empty() || (me.names.is_empty() && me.ids.is_empty()) {
		return Role::Unknown;
	}
	match game.players.iter().find(|seat| is_me(seat, me)) {
		Some(seat) if seat.spectator => Role::Spectated,
		Some(_) => Role::Played,
		None => Role::Late,
	}
}

fn is_me(seat: &Seat, me: &Identity) -> bool {
	me.names
		.iter()
		.any(|name| name.eq_ignore_ascii_case(&seat.name))
		|| seat.account_id.is_some_and(|id| me.ids.contains(&id))
}

fn game_row(game: &Game, me: &Identity) -> GameRow {
	GameRow {
		started: game.started,
		role: role(game, me),
		game_seconds: game.game_seconds,
		ais: game.ais.iter().map(|seat| seat.ai.clone()).collect(),
		players: game
			.players
			.iter()
			.filter(|seat| !seat.spectator && !is_me(seat, me))
			.map(|seat| seat.name.clone())
			.collect(),
		game_type: game.game_type.clone(),
		replay: game.replay.clone(),
	}
}

/// Each key's most common value across `settings`. A key most settings lack
/// is left out, so that those settings need say nothing about it.
fn usual(settings: &[&Setting]) -> BTreeMap<String, String> {
	let mut counts: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
	for setting in settings {
		for (key, value) in &setting.values {
			*counts.entry(key).or_default().entry(value).or_default() += 1;
		}
	}
	counts
		.into_iter()
		.filter_map(|(key, values)| {
			let absent = settings.len() - values.values().sum::<usize>();
			// The most common, and the first in order among equals.
			let (value, count) = values
				.into_iter()
				.max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))?;
			(count > absent).then(|| (key.to_owned(), value.to_owned()))
		})
		.collect()
}

/// Where `values` departs from `usual`.
fn differs(
	values: &BTreeMap<String, String>,
	usual: &BTreeMap<String, String>,
) -> BTreeMap<String, Option<String>> {
	let changed = values
		.iter()
		.filter(|(key, value)| usual.get(*key) != Some(value))
		.map(|(key, value)| (key.clone(), Some(value.clone())));
	let lacking = usual
		.keys()
		.filter(|key| !values.contains_key(*key))
		.map(|key| (key.clone(), None));
	changed.chain(lacking).collect()
}

/// The history, for `me`.
pub fn view(index: &Index, me: &Identity) -> HistoryView {
	let mut games: BTreeMap<&str, Vec<&Game>> = BTreeMap::new();
	for game in index.games.values() {
		games.entry(&game.setting).or_default().push(game);
	}
	let played: Vec<(&str, &Setting, Vec<&Game>)> = games
		.into_iter()
		.filter_map(|(key, mut games)| {
			games.sort_by_key(|game| std::cmp::Reverse(game.started));
			Some((key, index.settings.get(key)?, games))
		})
		.collect();

	let settings: Vec<&Setting> = played.iter().map(|(_, setting, _)| *setting).collect();
	let usual = usual(&settings);
	let blobs = settings
		.iter()
		.flat_map(|setting| setting.blobs.values())
		.map(|held| {
			(
				held.id.clone(),
				BlobInfo {
					bytes: held.bytes,
					name: held.name.clone(),
				},
			)
		})
		.collect();

	let mut rows: Vec<SettingRow> = played
		.into_iter()
		.map(|(key, setting, games)| SettingRow {
			key: key.to_owned(),
			map: setting.map.clone(),
			differs: differs(&setting.values, &usual),
			games: games.into_iter().map(|game| game_row(game, me)).collect(),
		})
		.collect();
	let newest = |row: &SettingRow| row.games.first().map_or(0, |game| game.started);
	rows.sort_by_key(|row| std::cmp::Reverse(newest(row)));

	HistoryView {
		settings: rows,
		usual,
		blobs,
		ignored_prefixes: IGNORED_PREFIXES.map(str::to_owned).to_vec(),
	}
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	use content::demo::Head;

	use super::*;
	use crate::index::{Read, merge};

	fn index(games: &[(u8, &str, &str)]) -> Index {
		let read: Vec<Read> = games
			.iter()
			.map(|(id, players, modoptions)| {
				let head = Head {
					game_id: Some([*id; 16]),
					started: 1_762_000_000 + u64::from(*id),
					length: None,
					script: format!(
						"[GAME]\n{{\n\tmapname=Forge;\n\thosttype=SPADS;\n\tautohostname=Host[EU6][012];\n\
						 {players}\t[MODOPTIONS]\n\t{{\n{modoptions}\t}}\n}}\n"
					),
				};
				Read {
					path: format!("{id}.sdfz"),
					bytes: 1,
					modified: None,
					parsed: Some(crate::read::read(Path::new(&format!("{id}.sdfz")), &head)),
				}
			})
			.collect();
		merge(&Index::default(), &content::ReplayListing::default(), &read)
	}

	fn player(name: &str, id: u32, spectator: u8) -> String {
		format!(
			"\t[player{id}]\n\t{{\n\t\tname={name};\n\t\taccountid={id};\n\t\tspectator={spectator};\n\t}}\n"
		)
	}

	fn me() -> Identity {
		Identity {
			names: vec!["tetrisface".into()],
			ids: vec![424_242],
		}
	}

	/// A row's values, put back together from `usual` and its departures.
	fn rebuilt(row: &SettingRow, usual: &BTreeMap<String, String>) -> BTreeMap<String, String> {
		let mut values = usual.clone();
		for (key, value) in &row.differs {
			match value {
				Some(value) => values.insert(key.clone(), value.clone()),
				None => values.remove(key),
			};
		}
		values
	}

	#[test]
	fn usual_values_and_departures_rebuild_every_setting_exactly() {
		let index = index(&[
			(1, "", "\t\ta=1;\n\t\tb=1;\n"),
			(2, "", "\t\ta=1;\n\t\tb=2;\n"),
			(3, "", "\t\ta=1;\n\t\tc=9;\n"),
		]);
		let view = view(&index, &me());
		assert_eq!(view.settings.len(), 3);
		for row in &view.settings {
			assert_eq!(rebuilt(row, &view.usual), index.settings[&row.key].values);
		}
		assert_eq!(view.usual["a"], "1");
		assert!(
			view.settings
				.iter()
				.all(|row| !row.differs.contains_key("a"))
		);
	}

	#[test]
	fn each_blob_is_described_once_for_the_whole_view() {
		let blob = tweaks::base64url::encode("-- Mine\nlocal x = 1\n", tweaks::Kind::Defs).unwrap();
		let index = index(&[
			(1, "", &format!("\t\ttweakdefs={blob};\n\t\ta=1;\n")),
			(2, "", &format!("\t\ttweakdefs={blob};\n\t\ta=2;\n")),
		]);
		let view = view(&index, &me());
		let id = crate::read::sha256_hex(blob.as_bytes());
		assert_eq!(view.blobs.len(), 1);
		assert_eq!(
			view.blobs[&id],
			BlobInfo {
				bytes: blob.len(),
				name: Some("Mine".into())
			}
		);
		assert_eq!(view.usual["tweakdefs"], id, "the value is the id");
	}

	#[test]
	fn a_rooms_values_are_held_to_the_same_rule_as_a_games() {
		let blob = tweaks::base64url::encode("-- Mine\nlocal x = 1\n", tweaks::Kind::Defs).unwrap();
		let room = BTreeMap::from([
			("tweakdefs".to_owned(), blob.clone()),
			("tweakdefs1".to_owned(), "0".to_owned()),
			("date_day".to_owned(), "9".to_owned()),
			("a".to_owned(), "1".to_owned()),
		]);
		let index = index(&[(1, "", &format!("\t\ttweakdefs={blob};\n\t\ta=1;\n"))]);

		let seen = values_view(&room);

		assert_eq!(seen.values, index.settings.values().next().unwrap().values);
		let id = crate::read::sha256_hex(blob.as_bytes());
		assert_eq!(seen.blobs[&id].name.as_deref(), Some("Mine"));
	}

	#[test]
	fn a_game_names_the_others_who_played_it_and_no_account() {
		let index = index(&[(
			1,
			&format!(
				"{}{}{}{}",
				player("Zed", 987_654, 0),
				player("renamed", 424_242, 0),
				player("Watcher", 555_555, 1),
				player("Amy", 123_456, 0),
			),
			"",
		)]);
		let view = view(&index, &me());
		assert_eq!(
			view.settings[0].games[0].players,
			["Amy", "Zed"],
			"in the script's order, without me or the spectators"
		);
		let json = serde_json::to_string(&view).unwrap();
		for private in ["Watcher", "renamed", "987654", "424242", "123456"] {
			assert!(
				!json.contains(private),
				"{private} reached the view: {json}"
			);
		}
	}

	#[test]
	fn the_newest_setting_and_game_come_first() {
		let index = index(&[
			(1, "", "\t\ta=1;\n"),
			(3, "", "\t\ta=2;\n"),
			(2, "", "\t\ta=1;\n"),
		]);
		let view = view(&index, &me());
		let started: Vec<Vec<u64>> = view
			.settings
			.iter()
			.map(|row| row.games.iter().map(|game| game.started).collect())
			.collect();
		assert_eq!(
			started,
			[vec![1_762_000_003], vec![1_762_000_002, 1_762_000_001]]
		);
		assert_eq!(view.ignored_prefixes, IGNORED_PREFIXES);
	}

	#[test]
	fn a_game_says_nothing_about_what_hosted_it() {
		let json = serde_json::to_string(&view(&index(&[(1, "", "")]), &me())).unwrap();
		assert!(!json.contains("Host[EU6][012]"), "{json}");
		assert!(!json.contains("autohost"), "{json}");
	}

	fn role_in(players: &str, me: &Identity) -> Role {
		let index = index(&[(1, players, "")]);
		role(index.games.values().next().unwrap(), me)
	}

	#[test]
	fn a_seat_of_mine_is_played_and_a_spectators_is_spectated() {
		assert_eq!(role_in(&player("tetrisface", 1, 0), &me()), Role::Played);
		assert_eq!(role_in(&player("TetrisFace", 1, 1), &me()), Role::Spectated);
		assert_eq!(
			role_in(&player("renamed", 424_242, 0), &me()),
			Role::Played,
			"the account id is enough"
		);
	}

	#[test]
	fn a_game_that_does_not_name_me_was_joined_late_and_one_nobody_can_judge_is_unknown() {
		// A start script lists who was in the room when the game began; a
		// replay recorded here that does not name me is one I joined after.
		assert_eq!(role_in(&player("Stranger", 7, 0), &me()), Role::Late);
		assert_eq!(
			role_in(&player("Stranger", 7, 0), &Identity::default()),
			Role::Unknown
		);
		assert_eq!(role_in("", &me()), Role::Unknown);
	}
}
