//! What the battle list can say about rooms nobody here is in.
//!
//! A room's modoptions reach a client only once it has joined
//! (`SETSCRIPTTAGS` after `JOINBATTLE`), so the list cannot ask the service
//! about anybody else's room. Someone in that room can, though, and every ask
//! that names its room ([`crate::lobby_game_id`]) leaves the answer on the
//! service's live games list. That list is public and cached at the edge, so
//! the battle list reads it and shows each room the challenge it last scored.
//!
//! Only the challenge: it describes the setup, not who plays it, and it is
//! the same number the in-room panel shows. The list's percentile is a
//! different one from the panel's, so showing it would contradict the panel.
//! Two challenges where the service has both: the room with the players it
//! has, and the room with every seat taken
//! ([`crate::Encounter::human_team_capacity`]). Where the service lists the
//! room's figure at every team size, "the players it has" is the room's head
//! count now, read by the battle list; otherwise it is whoever was seated when
//! the room was last asked about.
//!
//! What reads it here is deliberately forgiving. The list is the service's
//! (`live-api`'s `LiveGameSummary`, which pins these fields in a test of its
//! own); a field that is gone or of another shape means no score for that
//! room, never an error.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;
use ts_rs::TS;

use crate::Error;

/// The live games list, from the same distribution as [`crate::ENDPOINT`].
pub const LIVE_GAMES_ENDPOINT: &str = "https://d29i3oohxql6zz.cloudfront.net/api/v1/live-games";

/// How long one answer serves every caller. The edge holds the list for 5 s
/// and a room's score moves only when someone in it changes the setup, so a
/// list view, a remount and a second window can share a fetch.
pub const HOLD_FOR: Duration = Duration::from_secs(20);

/// One try, this long. The next poll is the retry: a battle list missing a
/// number for a minute is no failure worth a schedule of its own.
const DEADLINE: Duration = Duration::from_secs(5);

/// The challenge a room last scored, as the battle list shows it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ListedScore {
	/// 0-34, where 17 is an estimated even game for a representative team,
	/// with the players seated when the room was scored.
	pub challenge: f64,
	/// The same with every human seat taken. `None` for a room whose asker
	/// did not say how many seats it has (an older modlobby, or a running
	/// game) and from a service that predates the figure.
	pub full_challenge: Option<f64>,
	/// How many played when it was scored: who `challenge` is for.
	pub players: Option<u32>,
	/// The challenge at each human team size up to the seat count, by size:
	/// the same setup's table, so the list can read the room at the head count
	/// it has now. Empty from an asker that named no seat count and from a
	/// service that predates the table; `challenge` is then the room's figure.
	#[ts(type = "Record<number, number>")]
	pub sizes: BTreeMap<u32, f64>,
	/// When someone in the room last told the service about this setup, as
	/// the service's ISO timestamp. It has no newer word than that: a room
	/// whose setup has not changed since has not been asked about since.
	pub scored_at: String,
}

/// The live games list, by game id.
#[derive(Debug, Default)]
pub struct Listings {
	by_game: HashMap<String, Entry>,
}

#[derive(Debug, Clone)]
struct Entry {
	map: String,
	score: ListedScore,
}

/// A room in the battle list: as much of it as finding its score takes.
#[derive(Debug, Clone, Copy)]
pub struct Room<'a> {
	/// Its server's host, lowercased, as the lobby names it.
	pub server: &'a str,
	pub battle_id: u32,
	pub founder: &'a str,
	/// The map it is on now.
	pub map: &'a str,
}

/// What is read of one listed game; everything else on it is left alone.
#[derive(Deserialize)]
struct Game {
	game_id: Option<String>,
	map: String,
	last_seen_at: String,
	difficulty: Difficulty,
	/// Read loosely: an odd seat count costs the tooltip a line, not the room
	/// its score.
	#[serde(default)]
	encounter_context: serde_json::Value,
}

#[derive(Deserialize)]
struct Difficulty {
	status: String,
	challenge: Option<f64>,
	/// Loosely too: the current figure stands without it.
	#[serde(default)]
	full_team_challenge: serde_json::Value,
	/// Loosely as well, entry by entry.
	#[serde(default)]
	team_size_challenges: serde_json::Value,
}

/// The per-size table as listed, `{"1": 21.5, …}`: an entry whose size or
/// figure does not read as one is dropped, the rest kept.
fn sizes(listed: &serde_json::Value) -> BTreeMap<u32, f64> {
	let Some(by_size) = listed.as_object() else {
		return BTreeMap::new();
	};
	by_size
		.iter()
		.filter_map(|(size, challenge)| {
			let challenge = challenge.as_f64().filter(|value| value.is_finite())?;
			Some((size.parse().ok()?, challenge))
		})
		.collect()
}

impl Listings {
	/// The list as the service answered it.
	///
	/// Each game is read on its own, so one of an unexpected shape costs
	/// that game and not the list. Kept: games that name their game id and
	/// that the service placed. An abstention or a setup it could not score
	/// says nothing about difficulty, so it is left out rather than shown as
	/// a number.
	pub fn read(body: &serde_json::Value) -> Self {
		let games = body
			.get("games")
			.and_then(serde_json::Value::as_array)
			.map(Vec::as_slice)
			.unwrap_or_default();
		let mut by_game = HashMap::new();
		for game in games {
			let Ok(game) = serde_json::from_value::<Game>(game.clone()) else {
				continue;
			};
			let (Some(id), Some(challenge)) = (game.game_id, game.difficulty.challenge) else {
				continue;
			};
			if game.difficulty.status != "estimated" || !challenge.is_finite() {
				continue;
			}
			let full_challenge = game
				.difficulty
				.full_team_challenge
				.as_f64()
				.filter(|full| full.is_finite());
			let players = game
				.encounter_context
				.get("human_count")
				.and_then(serde_json::Value::as_u64)
				.and_then(|count| u32::try_from(count).ok());
			// The service lists the newest first; a duplicate is older news.
			by_game.entry(id).or_insert(Entry {
				map: game.map,
				score: ListedScore {
					challenge,
					full_challenge,
					players,
					sizes: sizes(&game.difficulty.team_size_challenges),
					scored_at: game.last_seen_at,
				},
			});
		}
		Self { by_game }
	}

	/// What the room listed under this id last scored, if it still plays
	/// the map it was scored on.
	///
	/// The map is the one part of a room's setup the battle list can see,
	/// so it is the check that a room has not been changed since whoever
	/// scored it left. A score for another map is another setup's score.
	pub fn score_for(&self, game_id: &str, map: &str) -> Option<ListedScore> {
		self.by_game
			.get(game_id)
			.filter(|entry| entry.map == map)
			.map(|entry| entry.score.clone())
	}

	/// What each of these rooms last scored, keyed `"{server}/{battle id}"`.
	///
	/// That key is the battle list's own `battleKey` (`app/src/lib/battles.ts`),
	/// two servers each having a battle 12; keep the two in step. A room is
	/// found under the id its own players' asks list it as,
	/// [`crate::lobby_game_id`], and only on the map it was scored on.
	pub fn for_rooms<'a>(
		&self,
		rooms: impl IntoIterator<Item = Room<'a>>,
	) -> HashMap<String, ListedScore> {
		if self.is_empty() {
			return HashMap::new();
		}
		rooms
			.into_iter()
			.filter_map(|room| {
				let id = crate::lobby_game_id(room.server, room.battle_id, room.founder);
				let score = self.score_for(&id, room.map)?;
				Some((format!("{}/{}", room.server, room.battle_id), score))
			})
			.collect()
	}

	pub fn len(&self) -> usize {
		self.by_game.len()
	}

	pub fn is_empty(&self) -> bool {
		self.by_game.is_empty()
	}
}

/// The live games list as one client sees it: the transport it was handed,
/// where the list lives, and the last answer while it is fresh.
///
/// The client and endpoint are handed in for the reason [`crate::Service`]'s
/// are: one connection pool for the process, and a test server for tests.
pub struct Listed {
	client: reqwest::Client,
	endpoint: String,
	hold: Duration,
	/// Held across the fetch, so callers arriving together share one.
	held: Mutex<Option<(Instant, Arc<Listings>)>>,
}

impl Listed {
	pub fn new(client: reqwest::Client, endpoint: impl Into<String>) -> Self {
		Self {
			client,
			endpoint: endpoint.into(),
			hold: HOLD_FOR,
			held: Mutex::new(None),
		}
	}

	/// The same, holding an answer for `hold` instead of [`HOLD_FOR`].
	pub fn holding(self, hold: Duration) -> Self {
		Self { hold, ..self }
	}

	/// The list, fetched unless an answer young enough is held.
	///
	/// A failure leaves what was held alone and is the caller's to shrug off:
	/// the list keeps the numbers it already shows.
	pub async fn current(&self) -> Result<Arc<Listings>, Error> {
		let mut held = self.held.lock().await;
		if let Some((at, listings)) = held.as_ref()
			&& at.elapsed() < self.hold
		{
			return Ok(Arc::clone(listings));
		}
		let listings = Arc::new(fetch(&self.client, &self.endpoint).await?);
		*held = Some((Instant::now(), Arc::clone(&listings)));
		Ok(listings)
	}
}

async fn fetch(client: &reqwest::Client, endpoint: &str) -> Result<Listings, Error> {
	let failed = |err: reqwest::Error| {
		if err.is_timeout() {
			Error::Timeout(DEADLINE)
		} else {
			Error::Request(err.to_string())
		}
	};
	let response = client
		.get(endpoint)
		.timeout(DEADLINE)
		.send()
		.await
		.map_err(failed)?;
	let status = response.status();
	if !status.is_success() {
		return Err(Error::Status(status.as_u16()));
	}
	let body: serde_json::Value = response.json().await.map_err(|err| {
		if err.is_timeout() {
			Error::Timeout(DEADLINE)
		} else {
			Error::Answer(err.to_string())
		}
	})?;
	Ok(Listings::read(&body))
}

#[cfg(test)]
mod tests {
	use super::*;

	const ROOM: &str = "a24d99de2eb0fb9af8987df826fdf94b";

	/// A game as `live-api` lists it (schema 3), every field it sends.
	fn game(
		game_id: Option<&str>,
		map: &str,
		status: &str,
		challenge: Option<f64>,
	) -> serde_json::Value {
		serde_json::json!({
			"id": "f".repeat(64),
			"game_id": game_id,
			"state": "observed_active",
			"outcome": null,
			"observed_at": "2026-11-07T18:00:00Z",
			"last_seen_at": "2026-11-07T18:12:00Z",
			"finished_at": null,
			"visible_until": "2026-11-14T18:12:00Z",
			"ai_type": "Raptors",
			"map": map,
			"player_count": 8,
			"encounter_context": {"human_count": 3, "enemy_ai_count": null},
			"setting_id": "0123456789abcdef0123456789abcdef",
			"difficulty": {
				"status": status,
				"challenge": challenge,
				"full_team_challenge": challenge.map(|now| now - 4.0),
				"team_size_challenges": {},
				"context_percentile": 71.5,
				"representative_team_win_probability": 0.41,
				"historical_games": 120,
				"historical_wins": 50
			},
			"setting_summary": {"raptor_difficulty": "hard"},
			"partial": false
		})
	}

	fn page(games: Vec<serde_json::Value>) -> serde_json::Value {
		serde_json::json!({
			"schema_version": 3,
			"generated_at": "2026-11-07T18:13:00Z",
			"games": games
		})
	}

	#[test]
	fn a_scored_room_is_read_with_when_it_was_scored() {
		let listings = Listings::read(&page(vec![game(
			Some(ROOM),
			"Full Metal Plate 1.5",
			"estimated",
			Some(16.4),
		)]));
		assert_eq!(
			listings.score_for(ROOM, "Full Metal Plate 1.5"),
			Some(ListedScore {
				challenge: 16.4,
				full_challenge: Some(16.4 - 4.0),
				players: Some(3),
				sizes: BTreeMap::new(),
				scored_at: "2026-11-07T18:12:00Z".into(),
			})
		);
	}

	#[test]
	fn a_room_listed_with_its_table_is_read_at_every_size() {
		let mut filling = game(Some(ROOM), "Map", "estimated", Some(21.5));
		filling["difficulty"]["team_size_challenges"] =
			serde_json::json!({"1": 21.5, "2": 17.0, "3": 14.2, "4": 12.0});
		let listings = Listings::read(&page(vec![filling]));
		assert_eq!(
			listings.score_for(ROOM, "Map").unwrap().sizes,
			BTreeMap::from([(1, 21.5), (2, 17.0), (3, 14.2), (4, 12.0)])
		);
	}

	#[test]
	fn an_odd_entry_in_the_table_costs_that_entry_and_not_the_room() {
		let mut odd = game(Some(ROOM), "Map", "estimated", Some(21.5));
		odd["difficulty"]["team_size_challenges"] =
			serde_json::json!({"1": 21.5, "two": 17.0, "3": "hard", "4": 12.0});
		let mut older = game(Some(&"b".repeat(32)), "Map", "estimated", Some(9.0));
		older["difficulty"]
			.as_object_mut()
			.unwrap()
			.remove("team_size_challenges");
		let listings = Listings::read(&page(vec![odd, older]));
		assert_eq!(
			listings.score_for(ROOM, "Map").unwrap().sizes,
			BTreeMap::from([(1, 21.5), (4, 12.0)])
		);
		let older = listings.score_for(&"b".repeat(32), "Map").unwrap();
		assert!(older.sizes.is_empty());
		assert_eq!(older.challenge, 9.0);
	}

	#[test]
	fn a_room_without_a_full_team_figure_keeps_its_current_one() {
		let mut older = game(Some(ROOM), "Map", "estimated", Some(16.4));
		older["difficulty"]
			.as_object_mut()
			.unwrap()
			.remove("full_team_challenge");
		older.as_object_mut().unwrap().remove("encounter_context");
		let mut nameless = game(Some(&"b".repeat(32)), "Map", "estimated", Some(9.0));
		nameless["difficulty"]["full_team_challenge"] = serde_json::Value::Null;
		let mut odd = game(Some(&"c".repeat(32)), "Map", "estimated", Some(5.0));
		odd["difficulty"]["full_team_challenge"] = serde_json::json!("full");
		odd["encounter_context"]["human_count"] = serde_json::json!(-1);
		let listings = Listings::read(&page(vec![older, nameless, odd]));
		for (id, now) in [
			(ROOM.to_owned(), 16.4),
			("b".repeat(32), 9.0),
			("c".repeat(32), 5.0),
		] {
			let score = listings.score_for(&id, "Map").expect("still scored");
			assert_eq!(score.challenge, now);
			assert_eq!(score.full_challenge, None);
		}
		assert_eq!(listings.score_for(ROOM, "Map").unwrap().players, None);
		assert_eq!(
			listings.score_for(&"c".repeat(32), "Map").unwrap().players,
			None
		);
	}

	#[test]
	fn a_room_on_another_map_than_it_was_scored_on_has_no_score() {
		let listings = Listings::read(&page(vec![game(
			Some(ROOM),
			"Full Metal Plate 1.5",
			"estimated",
			Some(16.4),
		)]));
		assert_eq!(listings.score_for(ROOM, "Supreme Isthmus 1.2"), None);
	}

	#[test]
	fn only_a_placed_setup_with_a_game_id_is_kept() {
		let listings = Listings::read(&page(vec![
			game(Some(ROOM), "Map", "abstained", Some(16.4)),
			game(Some(&"b".repeat(32)), "Map", "estimated", None),
			game(None, "Map", "estimated", Some(16.4)),
			game(Some(&"c".repeat(32)), "Map", "unavailable", None),
		]));
		assert!(listings.is_empty(), "{listings:?}");
	}

	#[test]
	fn a_game_of_another_shape_costs_that_game_and_not_the_list() {
		let mut odd = game(Some(&"b".repeat(32)), "Map", "estimated", Some(3.0));
		odd["difficulty"]["challenge"] = serde_json::json!("hard");
		let listings = Listings::read(&page(vec![
			odd,
			game(Some(ROOM), "Map", "estimated", Some(20.0)),
		]));
		assert_eq!(listings.len(), 1);
		assert_eq!(listings.score_for(ROOM, "Map").unwrap().challenge, 20.0);
	}

	#[test]
	fn a_list_of_another_shape_is_an_empty_list() {
		assert!(Listings::read(&serde_json::json!({"schema_version": 9})).is_empty());
		assert!(Listings::read(&serde_json::json!([])).is_empty());
	}

	#[test]
	fn the_newest_of_two_rows_for_one_game_wins() {
		let listings = Listings::read(&page(vec![
			game(Some(ROOM), "Map", "estimated", Some(20.0)),
			game(Some(ROOM), "Map", "estimated", Some(5.0)),
		]));
		assert_eq!(listings.score_for(ROOM, "Map").unwrap().challenge, 20.0);
	}

	const SERVER: &str = "server4.beyondallreason.info";

	fn room(map: &str) -> Room<'_> {
		Room {
			server: SERVER,
			battle_id: 5,
			founder: "host",
			map,
		}
	}

	/// The list holding one scored room, listed as `room`'s own asks list it.
	fn listing_room(map: &str) -> Listings {
		let id = crate::lobby_game_id(SERVER, 5, "host");
		Listings::read(&page(vec![game(Some(&id), map, "estimated", Some(16.4))]))
	}

	#[test]
	fn a_listed_room_is_found_under_its_battle_list_key() {
		let scores = listing_room("Now").for_rooms([room("Now")]);
		assert_eq!(scores.len(), 1);
		let score = &scores["server4.beyondallreason.info/5"];
		assert_eq!(score.challenge, 16.4);
		assert_eq!(score.scored_at, "2026-11-07T18:12:00Z");
	}

	#[test]
	fn a_room_is_scored_only_on_its_own_server_and_its_current_map() {
		let listed = listing_room("Now");
		let elsewhere = Room {
			server: "lan",
			..room("Now")
		};
		assert!(
			listed.for_rooms([elsewhere]).is_empty(),
			"battle 5 on another server is another room"
		);
		let other_host = Room {
			founder: "someone else",
			..room("Now")
		};
		assert!(
			listed.for_rooms([other_host]).is_empty(),
			"a battle id handed to another host is another room"
		);
		assert!(
			listed.for_rooms([room("Then")]).is_empty(),
			"a score for another map is another setup's"
		);
	}

	async fn serving(expected: u64) -> wiremock::MockServer {
		use wiremock::matchers::{method, path};
		use wiremock::{Mock, MockServer, ResponseTemplate};

		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/api/v1/live-games"))
			.respond_with(ResponseTemplate::new(200).set_body_json(page(vec![game(
				Some(ROOM),
				"Map",
				"estimated",
				Some(16.4),
			)])))
			.expect(expected)
			.mount(&server)
			.await;
		server
	}

	#[tokio::test]
	async fn callers_inside_the_hold_share_one_fetch() {
		let server = serving(1).await;
		let listed = Listed::new(
			content::http::client("test"),
			format!("{}/api/v1/live-games", server.uri()),
		);
		let first = listed.current().await.unwrap();
		let again = listed.current().await.unwrap();
		assert!(Arc::ptr_eq(&first, &again));
		assert_eq!(first.len(), 1);
	}

	#[tokio::test]
	async fn an_answer_past_its_hold_is_fetched_again() {
		let server = serving(2).await;
		let listed = Listed::new(
			content::http::client("test"),
			format!("{}/api/v1/live-games", server.uri()),
		)
		.holding(Duration::ZERO);
		listed.current().await.unwrap();
		listed.current().await.unwrap();
	}

	#[tokio::test]
	async fn a_refusal_is_an_error_and_keeps_nothing() {
		use wiremock::matchers::method;
		use wiremock::{Mock, MockServer, ResponseTemplate};

		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(503))
			.expect(2)
			.mount(&server)
			.await;
		let listed = Listed::new(content::http::client("test"), server.uri());
		assert!(matches!(listed.current().await, Err(Error::Status(503))));
		assert!(matches!(listed.current().await, Err(Error::Status(503))));
	}
}
