//! The commands behind the preset table.
//!
//! Kept out of `commands.rs` because there are a dozen of them and they share
//! two things nothing else needs: where Chobby keeps its own file, and how to
//! read the room we are standing in.

use std::path::PathBuf;

use presets::{Book, Preset, Sections, Stamp};
use tauri::State;

use crate::commands::{ApiError, Result, open};
use crate::state::App;

impl From<presets::Error> for ApiError {
	fn from(err: presets::Error) -> Self {
		let code = match err {
			presets::Error::Unknown(_) => "missing",
			presets::Error::Duplicate(_) => "duplicate",
			presets::Error::Invalid { .. } => "invalid",
			presets::Error::Io { .. } => "io",
		};
		ApiError::new(code, err.to_string())
	}
}

impl From<content::demo::Error> for ApiError {
	fn from(err: content::demo::Error) -> Self {
		ApiError::new("replay", err.to_string())
	}
}

fn now() -> Stamp {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|since| since.as_secs())
		.unwrap_or_default()
}

/// Where Chobby keeps its presets: the file wherever one already exists, so
/// importing and exporting land where the other client will see them without
/// anyone typing a path. With no Chobby install to speak of, ours.
///
/// This is the one place modlobby writes into another lobby's directory, and
/// only on an explicit export: the file is the hand-over.
fn chobby_file(app: &App) -> Option<PathBuf> {
	let dirs = lobby_runtime::launch::data_dirs(app.settings.get().paths.data_dir)?;
	let existing = dirs
		.all()
		.map(|dir| dir.join("optionsPresets.json"))
		.find(|path| path.is_file());
	Some(existing.unwrap_or_else(|| dirs.write.join("optionsPresets.json")))
}

#[tauri::command]
pub fn list_presets(app: State<'_, App>) -> Result<Book> {
	Ok(app.presets.load()?)
}

/// Where the Chobby file is, so the front end can say so before touching it.
#[tauri::command]
pub fn chobby_presets_path(app: State<'_, App>) -> Option<String> {
	chobby_file(&app).map(|path| path.display().to_string())
}

/// Our own preset file, in whatever opens `.json` here.
#[tauri::command]
pub fn open_presets_file(app: State<'_, App>) -> Result<()> {
	open(app.presets.path().to_path_buf())
}

/// The current room, as a preset.
///
/// The modoptions come from the room's script tags, which is the server's own
/// statement of what it is set to — not from anything we remember sending.
#[tauri::command]
pub async fn save_preset(app: State<'_, App>, name: String) -> Result<Book> {
	let snapshot = app.client.snapshot().await?;
	let Some((session, my)) = snapshot.room() else {
		return Err(ApiError::new("not in a room", "join a room to save it"));
	};
	let room = session
		.battles
		.iter()
		.find(|battle| battle.id == my.id)
		.ok_or_else(|| ApiError::new("not in a room", "the room is not in the list"))?;

	let mut preset = Preset::new(name, now());
	preset.map = Some(room.map_name.clone());
	preset.modoptions = my
		.script_tags
		.iter()
		.filter_map(|(key, value)| {
			Some((
				key.strip_prefix("game/modoptions/")?.to_owned(),
				value.clone(),
			))
		})
		.collect();
	if let Some(layout) = &room.layout {
		preset
			.battle
			.insert("teamSize".into(), layout.team_size.to_string());
		preset
			.battle
			.insert("nbTeams".into(), layout.teams.to_string());
	}
	preset
		.battle
		.insert("locked".into(), u8::from(room.locked).to_string());
	preset.start_boxes = room
		.start_rects
		.iter()
		.map(|rect| {
			(
				rect.ally_team,
				presets::StartBox {
					left: rect.left,
					top: rect.top,
					right: rect.right,
					bottom: rect.bottom,
				},
			)
		})
		.collect();

	Ok(app.presets.put(preset, now())?)
}

/// A preset made from the game a replay recorded.
#[tauri::command]
pub fn preset_from_replay(app: State<'_, App>, path: String, name: String) -> Result<Book> {
	let text = content::demo::script(&path)?;
	let script = recoil::script_read::parse(&text);

	let mut preset = Preset::new(name, now());
	preset.map = script.map().map(str::to_owned);
	preset.modoptions = script.modoptions.clone();
	preset.start_boxes = script
		.boxes_out_of_200()
		.into_iter()
		.map(|(ally, (left, top, right, bottom))| {
			(
				ally,
				presets::StartBox {
					left,
					top,
					right,
					bottom,
				},
			)
		})
		.collect();

	Ok(app.presets.put(preset, now())?)
}

#[tauri::command]
pub fn delete_preset(app: State<'_, App>, name: String) -> Result<Book> {
	Ok(app.presets.remove(&name)?)
}

#[tauri::command]
pub fn rename_preset(app: State<'_, App>, from: String, to: String) -> Result<Book> {
	Ok(app.presets.rename(&from, &to)?)
}

/// What applying a preset would send, without sending it.
#[tauri::command]
pub async fn plan_preset(
	app: State<'_, App>,
	name: String,
	sections: Sections,
) -> Result<presets::Plan> {
	let preset = one(&app, &name)?;
	Ok(presets::plan(&preset, &current_room(&app).await?, sections))
}

/// Sends a preset to the room.
///
/// As one paste, the same as pasting its lines into the room's chat: that is
/// what the paste banner counts, and what takes the burst lane for a boss.
/// Said a line at a time, each was a paste of one, which shows nothing and
/// always goes out paced.
#[tauri::command]
pub async fn apply_preset(
	app: State<'_, App>,
	name: String,
	sections: Sections,
) -> Result<presets::Plan> {
	let preset = one(&app, &name)?;
	let plan = presets::plan(&preset, &current_room(&app).await?, sections);

	if !plan.lines.is_empty() {
		app.client.say(plan.lines.join("\n")).await?;
	}

	app.presets.touch(&name, now())?;
	Ok(plan)
}

/// The room with no server behind it, as a preset.
///
/// Its AIs are written too, which the online one does not do: a room on the
/// server rarely has any, and a skirmish is mostly its AIs.
#[tauri::command]
pub async fn skirmish_save_preset(app: State<'_, App>, name: String) -> Result<Book> {
	let Some(room) = app.client.skirmish_room().await? else {
		return Err(ApiError::new("no room", "open a skirmish to save it"));
	};
	let preset = skirmish::preset::to_preset(&room, name, now());
	Ok(app.presets.put(preset, now())?)
}

/// Puts one back into it.
///
/// Nothing is sent and nothing is paced: online a preset is a couple of
/// minutes of `!bSet` lines trickled past SPADS' flood limit, and here it is
/// one assignment. The answer keeps the same shape so the pane can say what
/// happened either way; the list of lines is simply empty.
#[tauri::command]
pub async fn skirmish_apply_preset(
	app: State<'_, App>,
	name: String,
	sections: Sections,
) -> Result<presets::Plan> {
	let preset = one(&app, &name)?;
	let done = app.client.skirmish_preset(preset.clone(), sections).await?;
	app.presets.touch(&name, now())?;
	Ok(presets::Plan {
		lines: Vec::new(),
		start_boxes: preset
			.start_boxes
			.iter()
			.map(|(ally, held)| presets::PlannedBox {
				ally_team: *ally,
				left: held.left,
				top: held.top,
				right: held.right,
				bottom: held.bottom,
			})
			.collect(),
		start_boxes_unsent: sections.start_boxes && !done.boxes && !preset.start_boxes.is_empty(),
		already_set: done.already_set,
	})
}

/// What the pve.bar stats service says the current room scores.
///
/// Answers `None` rather than an error for a room that is not PvE, or when the
/// setting is off: neither is a failure, and a panel that says nothing is the
/// right outcome for both.
#[tauri::command]
pub async fn pve_score(app: State<'_, App>) -> Result<Option<pve::Score>> {
	if !app.settings.get().play.pve_stats {
		return Ok(None);
	}
	let snapshot = app.client.snapshot().await?;
	let Some((session, my)) = snapshot.room() else {
		return Ok(None);
	};
	let Some(room) = session.battles.iter().find(|battle| battle.id == my.id) else {
		return Ok(None);
	};
	// What lists the room as a game being played. Answers the service already
	// gave are memoised by body, so a room that stops changing stops
	// refreshing its row; the row expires on the service's own clock, which is
	// what a lobby that went quiet should do.
	let listed = pve::lobby_game_id(&session.server, room.id, &room.founder);
	let Some(ask) = pve_ask(my, room, &session.users, Some(listed)) else {
		return Ok(None);
	};
	ask_pve(&app, ask).await
}

/// The same question for a room with no server behind it.
///
/// Asked without a `game_id`, which is what keeps it a question rather than an
/// announcement: listing a skirmish would put a game nobody can join on a
/// service whose whole job is saying what is being played.
#[tauri::command]
pub async fn skirmish_pve_score(app: State<'_, App>) -> Result<Option<pve::Score>> {
	if !app.settings.get().play.pve_stats {
		return Ok(None);
	}
	let snapshot = app.client.snapshot().await?;
	let Some(room) = snapshot.skirmish.as_ref() else {
		return Ok(None);
	};
	let Some(ask) = pve_ask(&room.my, &room.battle, &room.users, None) else {
		return Ok(None);
	};
	ask_pve(&app, ask).await
}

/// What each room in the battle list last scored, by battle list key.
///
/// Read off the service's live games list rather than asked: the list can
/// only score a room someone in it has asked about, which is every room a
/// modlobby with PvE stats on has sat in. Nothing is fetched while the
/// setting is off -- the same switch as the panel's, so a player who turned
/// pve.bar off never hears from it.
///
/// The list is fetched before the rooms are read, so each score is checked
/// against the room as it is now.
#[tauri::command]
pub async fn pve_listed_scores(
	app: State<'_, App>,
) -> Result<std::collections::HashMap<String, pve::ListedScore>> {
	if !app.settings.get().play.pve_stats {
		return Ok(std::collections::HashMap::new());
	}
	let listings = app.pve_listed.current().await.map_err(|err| {
		// The list keeps the numbers it has; the reason lives here.
		tracing::debug!(error = %err, "pve stats: no live games list");
		ApiError::new("pve", err.to_string())
	})?;
	let snapshot = app.client.snapshot().await?;
	let rooms = snapshot.servers.iter().flat_map(|session| {
		session.battles.iter().map(move |battle| pve::Room {
			server: &session.server,
			battle_id: battle.id,
			founder: &battle.founder,
			map: &battle.map_name,
		})
	});
	Ok(listings.for_rooms(rooms))
}

/// Who the service is told is playing, and against what.
///
/// The room's own AIs whenever they name an opponent, so a room being set up
/// for its next game is scored as set up, game running or not. Failing that,
/// whoever the running game started with: a leaver's AIs leave the room and
/// play on, and a room with no opponent or no seats gets no score at all.
fn pve_roster(
	my: &lobby_ui::MyBattleView,
	room: &lobby_ui::BattleView,
	users: &[lobby_ui::UserView],
) -> Option<(pve::Opponent, lobby_ui::GameRosterView)> {
	let opponent = |bots: &[lobby_ui::BotView]| {
		let names: Vec<String> = bots.iter().map(|bot| bot.ai.clone()).collect();
		pve::opponent(&names)
	};
	if let Some(found) = opponent(&room.bots) {
		let here = lobby_ui::GameRosterView {
			bots: room.bots.clone(),
			player_count: room.player_count,
			seat_handicaps: room
				.members
				.iter()
				.filter_map(|name| users.iter().find(|user| &user.name == name))
				.filter_map(|user| user.battle_status.as_ref())
				.filter(|status| status.player)
				.map(|status| status.handicap)
				.collect(),
			guessed: false,
		};
		return Some((found, here));
	}
	let started = my.game_roster.as_ref()?;
	Some((opponent(&started.bots)?, started.clone()))
}

/// What the service is told about a room, whichever kind it is.
fn pve_ask(
	my: &lobby_ui::MyBattleView,
	room: &lobby_ui::BattleView,
	users: &[lobby_ui::UserView],
	game_id: Option<String>,
) -> Option<pve::Ask> {
	let (opponent, roster) = pve_roster(my, room, users)?;
	let kind = opponent.kind;

	Some(pve::Ask {
		// A guess at the opponent stays a question: this id is what would list
		// the room as a game of that kind being played.
		game_id: game_id.filter(|_| !roster.guessed),
		ai_type: kind.as_str(),
		map: room.map_name.clone(),
		game_settings: my
			.script_tags
			.iter()
			.filter_map(|(key, value)| {
				Some((
					key.strip_prefix("game/modoptions/")?.to_owned(),
					value.clone(),
				))
			})
			.collect(),
		encounter_context: pve::Encounter {
			human_team_size: roster.player_count,
			enemy_ai_count: matches!(kind, pve::AiType::Barbarian)
				.then_some(roster.bots.len() as u32),
			// Exact names, whatever the mode: the service classifies the
			// opponent from these and says in its answer what it made of them.
			enemy_ai_names: opponent.names,
			// Each BARbarian is its own opponent with its own handicap, and
			// the service will not place a Barbarian room without hearing
			// them. Raptors and scavengers are one controller however many
			// slots they take, so for them this stays empty.
			enemy_ai_income_multipliers: matches!(kind, pve::AiType::Barbarian)
				.then(|| {
					roster
						.bots
						.iter()
						.map(|bot| pve::income_multiplier(bot.status.handicap))
						.collect()
				})
				.unwrap_or_default(),
			// One per seated human. The service derives its `Player Handicap`
			// column from the average of these, and a governed column it
			// cannot derive counts as missing rather than defaulted — which is
			// what makes it decline to score the room at all.
			human_player_income_multipliers: roster
				.seat_handicaps
				.iter()
				.map(|&handicap| pve::income_multiplier(handicap))
				.collect(),
		},
		// Everyone present, bar the autohost: the service has a record for
		// people, not for SPADS. Sorted so the same room is the same body.
		player_names: {
			let mut names: Vec<String> = room
				.members
				.iter()
				.filter(|name| {
					users
						.iter()
						.find(|user| &user.name == *name)
						.is_none_or(|user| !user.status.bot)
				})
				.cloned()
				.collect();
			names.sort();
			names
		},
		player_filter_requested: true,
	})
}

/// Puts the question, and keeps the reason for a refusal out of the panel.
async fn ask_pve(app: &State<'_, App>, ask: pve::Ask) -> Result<Option<pve::Score>> {
	tracing::debug!(
		ai = ask.ai_type,
		map = %ask.map,
		settings = ask.game_settings.len(),
		seats = ask.encounter_context.human_team_size,
		"pve stats: asking"
	);
	match app.pve.score(&ask).await {
		Ok(score) => Ok(Some(score)),
		Err(err) => {
			// The panel says "unavailable" and no more; the reason lives here.
			tracing::warn!(error = %err, "pve stats: no answer");
			Err(ApiError::new("pve", err.to_string()))
		}
	}
}

/// Brings in everything from Chobby's file.
#[tauri::command]
pub fn import_presets(app: State<'_, App>, path: Option<String>) -> Result<Imported> {
	let path = path
		.map(PathBuf::from)
		.or_else(|| chobby_file(&app))
		.ok_or_else(|| ApiError::new("no path", "no BAR data directory to look in"))?;
	let (book, skipped) = app.presets.import_chobby(&path, now())?;
	Ok(Imported { book, skipped })
}

#[derive(serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Imported {
	pub book: Book,
	/// Presets in their file whose name we already had, and so left alone.
	pub skipped: usize,
}

/// Writes presets back into Chobby's file, leaving its other entries alone.
#[tauri::command]
pub fn export_presets(
	app: State<'_, App>,
	path: Option<String>,
	names: Vec<String>,
) -> Result<usize> {
	let path = path
		.map(PathBuf::from)
		.or_else(|| chobby_file(&app))
		.ok_or_else(|| ApiError::new("no path", "no BAR data directory to write to"))?;
	Ok(app.presets.export_chobby(&path, &names)?)
}

fn one(app: &App, name: &str) -> Result<Preset> {
	app.presets
		.load()?
		.presets
		.into_iter()
		.find(|preset| preset.name == name)
		.ok_or_else(|| ApiError::new("missing", format!("no preset called {name}")))
}

/// The room as it stands, so a plan can leave out what is already true.
async fn current_room(app: &App) -> Result<presets::Room> {
	let snapshot = app.client.snapshot().await?;
	let Some((session, my)) = snapshot.room() else {
		return Ok(presets::Room::default());
	};
	Ok(presets::Room {
		map: session
			.battles
			.iter()
			.find(|battle| battle.id == my.id)
			.map(|battle| battle.map_name.clone()),
		modoptions: my
			.script_tags
			.iter()
			.filter_map(|(key, value)| {
				Some((
					key.strip_prefix("game/modoptions/")?.to_owned(),
					value.clone(),
				))
			})
			.collect(),
	})
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use lobby_ui::{
		BattleStatusView, BattleView, BotView, GameRosterView, IntentView, MyBattleView, SyncView,
		UserStatusView, UserView,
	};

	use super::pve_ask;

	fn status(handicap: u8, sync: SyncView) -> BattleStatusView {
		BattleStatusView {
			ready: false,
			team: 0,
			ally_team: 0,
			player: true,
			handicap,
			sync,
			side: 0,
		}
	}

	fn bot(ai: &str, handicap: u8) -> BotView {
		BotView {
			name: ai.into(),
			owner: "host".into(),
			status: status(handicap, SyncView::Bot),
			team_colour: 0,
			ai: ai.into(),
			options: BTreeMap::new(),
		}
	}

	/// A room of one seated human, `me`, on the map `Now`.
	fn room(bots: Vec<BotView>) -> BattleView {
		BattleView {
			id: 5,
			founder: "host".into(),
			ip: String::new(),
			port: 0,
			max_players: 16,
			passworded: false,
			locked: false,
			map_hash: String::new(),
			map_name: "Now".into(),
			engine_name: String::new(),
			engine_version: String::new(),
			title: String::new(),
			game_name: String::new(),
			members: vec!["host".into(), "me".into()],
			spectator_count: 1,
			player_count: 1,
			layout: None,
			bots,
			start_rects: Vec::new(),
			queue: Vec::new(),
		}
	}

	fn user(name: &str, bot: bool, battle_status: Option<BattleStatusView>) -> UserView {
		UserView {
			name: name.into(),
			country: String::new(),
			user_id: None,
			lobby_client: String::new(),
			status: UserStatusView {
				in_game: false,
				away: false,
				rank: 0,
				moderator: false,
				bot,
			},
			battle_status,
			battle_id: Some(5),
		}
	}

	fn users() -> Vec<UserView> {
		vec![
			user("host", true, None),
			user("me", false, Some(status(0, SyncView::Synced))),
		]
	}

	fn my(game_roster: Option<GameRosterView>) -> MyBattleView {
		MyBattleView {
			boss: None,
			auto_balance: None,
			preset: None,
			id: 5,
			game_hash: String::new(),
			script_tags: BTreeMap::from([("game/modoptions/now".into(), "1".into())]),
			vote: None,
			history: Vec::new(),
			intent: IntentView::Spectate,
			ready_on_its_way: None,
			seat_on_its_way: None,
			held_until_ms: None,
			settled: true,
			game_roster,
		}
	}

	/// Three humans against what is given, one of them on a handicap of 20.
	fn started_with(bots: Vec<BotView>) -> Option<GameRosterView> {
		Some(GameRosterView {
			bots,
			player_count: 3,
			seat_handicaps: vec![0, 0, 20],
			guessed: false,
		})
	}

	#[test]
	fn no_ai_and_no_game_is_nothing_to_ask() {
		assert!(pve_ask(&my(None), &room(vec![]), &users(), None).is_none());
	}

	#[test]
	fn a_game_whose_ais_left_is_asked_about_as_it_started() {
		let roster = started_with(vec![bot("BARb", 10), bot("BARb", 30)]);
		let ask = pve_ask(&my(roster), &room(vec![]), &users(), None).unwrap();

		assert_eq!(ask.ai_type, "Barbarian");
		assert_eq!(ask.encounter_context.enemy_ai_count, Some(2));
		assert_eq!(ask.encounter_context.enemy_ai_names, ["BARb", "BARb"]);
		assert_eq!(
			ask.encounter_context.enemy_ai_income_multipliers,
			[1.1, 1.3]
		);
		assert_eq!(ask.encounter_context.human_team_size, 3);
		assert_eq!(
			ask.encounter_context.human_player_income_multipliers,
			[1.0, 1.0, 1.2]
		);
	}

	#[test]
	fn the_roster_fills_in_the_people_and_leaves_the_settings_to_the_room() {
		let roster = started_with(vec![bot("RaptorsAI", 0)]);
		let ask = pve_ask(&my(roster), &room(vec![]), &users(), None).unwrap();

		assert_eq!(ask.map, "Now");
		assert_eq!(ask.game_settings["now"], "1");
		assert_eq!(ask.player_names, ["me"]);
	}

	#[test]
	fn an_opponent_in_the_room_overrides_the_game_being_played() {
		let roster = started_with(vec![bot("RaptorsAI", 0)]);
		let here = room(vec![bot("ScavengersAI", 0)]);
		let ask = pve_ask(&my(roster), &here, &users(), None).unwrap();

		assert_eq!(ask.ai_type, "Scavengers");
		assert_eq!(ask.encounter_context.enemy_ai_names, ["ScavengersAI"]);
		assert_eq!(ask.encounter_context.human_team_size, 1);
		assert_eq!(ask.encounter_context.human_player_income_multipliers, [1.0]);
	}

	#[test]
	fn a_game_we_saw_goes_on_being_listed_after_its_ais_leave() {
		let roster = started_with(vec![bot("RaptorsAI", 0)]);
		let listed = Some("id".to_string());
		let ask = pve_ask(&my(roster), &room(vec![]), &users(), listed).unwrap();
		assert_eq!(ask.game_id.as_deref(), Some("id"));
	}

	#[test]
	fn a_guess_from_the_title_is_scored_and_never_listed() {
		let guess = started_with(vec![bot("RaptorsAI", 0)]).map(|roster| GameRosterView {
			guessed: true,
			..roster
		});
		let listed = Some("id".to_string());
		let ask = pve_ask(&my(guess), &room(vec![]), &users(), listed).unwrap();
		assert_eq!(ask.ai_type, "Raptors");
		assert_eq!(ask.game_id, None);
	}

	#[test]
	fn a_game_started_without_a_pve_opponent_has_nothing_to_fall_back_on() {
		let roster = started_with(vec![bot("Circuit", 0)]);
		assert!(pve_ask(&my(roster), &room(vec![]), &users(), None).is_none());
	}

	#[test]
	fn a_host_built_barbarian_is_asked_about_under_its_own_name() {
		// The service is the one that knows whether it can score SMRTBARb; the
		// room is sent as it is, and the answer carries the verdict.
		let here = room(vec![bot("SMRTBARb", 0), bot("SMRTBARb", 0)]);
		let ask = pve_ask(&my(None), &here, &users(), None).unwrap();
		assert_eq!(ask.ai_type, "Barbarian");
		assert_eq!(ask.encounter_context.enemy_ai_count, Some(2));
		assert_eq!(
			ask.encounter_context.enemy_ai_names,
			["SMRTBARb", "SMRTBARb"]
		);
	}
}
