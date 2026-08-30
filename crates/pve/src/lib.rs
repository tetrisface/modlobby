//! What a PvE lobby setup scores, before anyone plays it.
//!
//! BAR's PvE Stats service already answers this for a game in progress — the
//! in-game widget at `Widgets/widgets-extra/gui_pve_stats` posts the map, the
//! modoptions and the encounter to it and shows a challenge score, a win
//! chance and where the setup sits among games people have actually played.
//!
//! A lobby knows all of that before the game starts, which is when it is
//! useful: this is the number that answers "is this room going to be a
//! massacre" while there is still time to change it.
//!
//! Two things are deliberately not sent. Player names and account ids, which
//! the widget may send and this does not — the service's own README says the
//! estimate represents a generic team and does not use the identities or
//! ratings of the people present, so there is nothing to gain by naming them.
//! And nothing at all is sent for a room that is not PvE.
//!
//! The endpoint is plain HTTP, which is the service's own note in its README,
//! so the room's settings travel unencrypted. Nothing about a person does.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The service the in-game widget talks to (`include/remote.lua:16-18`).
pub const ENDPOINT: &str = "http://d29i3oohxql6zz.cloudfront.net/api/v1/stats";

/// Which PvE opponent a room is set up against.
///
/// The widget reads this from the LuaAI on each team; a lobby reads it from
/// the AI names in the room, which is the same string before the game starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum AiType {
    Raptors,
    Scavengers,
    Barbarian,
}

impl AiType {
    /// What the service calls it.
    pub fn as_str(self) -> &'static str {
        match self {
            AiType::Raptors => "Raptors",
            AiType::Scavengers => "Scavengers",
            AiType::Barbarian => "Barbarian",
        }
    }
}

/// The opponent a set of AI names describes, if they agree on one.
///
/// Mirrors the widget's `AiTypeFromText`: a name mentioning raptors is
/// raptors, one mentioning scavengers is scavengers, and something mentioning
/// both is neither — a mixed room is not a setup the model was trained on, and
/// guessing one of them would put a confident number on a game nobody played.
pub fn ai_type(ai_names: &[String]) -> Option<AiType> {
    let folded = ai_names.join(" ").to_ascii_lowercase();
    let raptors = folded.contains("raptor");
    let scavengers = folded.contains("scavenger");
    match (raptors, scavengers) {
        (true, false) => Some(AiType::Raptors),
        (false, true) => Some(AiType::Scavengers),
        (true, true) => None,
        (false, false) => folded.contains("barb").then_some(AiType::Barbarian),
    }
}

/// What the room is, as the service wants to hear it.
#[derive(Debug, Clone, Serialize)]
pub struct Ask {
    pub ai_type: &'static str,
    pub map: String,
    pub game_settings: BTreeMap<String, String>,
    pub encounter_context: Encounter,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Encounter {
    pub human_team_size: u32,
    /// Only meaningful for BARbarians: repeated raptor or scavenger slots are
    /// one controller and do not make the game harder, which the widget notes
    /// in as many words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enemy_ai_count: Option<u32>,
}

/// What the room scores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Score {
    /// 0-34, where 17 is an estimated even game for a representative team.
    /// `None` when the service could not place this setup.
    pub challenge: Option<f64>,
    /// Where that sits among games actually played, 0-100.
    pub percentile: Option<f64>,
    /// Estimated chance a representative human team wins, 0-1.
    pub win_chance: Option<f64>,
    /// Eligible games behind the model. Not a confidence score, and the widget
    /// is at pains to say so.
    pub evidence_games: Option<f64>,
    /// Set when the room uses settings the service has not catalogued, so the
    /// numbers are best-effort rather than an exact match.
    pub best_effort: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this room is not a PvE setup")]
    NotPve,
    #[error("pve stats: {0}")]
    Request(String),
    #[error("pve stats answered with something unexpected: {0}")]
    Answer(String),
}

/// Reads the numbers out of whatever the service replied.
///
/// Every field is optional on purpose. The service is a moving target, older
/// deployments omit things, and a missing number should leave a blank in the
/// panel rather than turn the whole answer into an error.
pub fn read(body: &serde_json::Value) -> Score {
    let number = |parent: &str, key: &str| -> Option<f64> { body.get(parent)?.get(key)?.as_f64() };
    Score {
        challenge: number("difficulty_histogram", "current_difficulty"),
        percentile: number("difficulty_histogram", "current_percentile"),
        win_chance: number("difficulty_estimate", "player_win_probability"),
        evidence_games: number("difficulty_estimate", "evidence_games"),
        best_effort: body
            .get("degradation")
            .and_then(|held| held.get("reason"))
            .and_then(serde_json::Value::as_str)
            == Some("unknown_modoptions"),
    }
}

/// Asks the service what a setup scores.
pub async fn fetch(ask: &Ask) -> Result<Score, Error> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        // The widget does not follow redirects either; there is one endpoint.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| Error::Request(err.to_string()))?;

    let response = client
        .post(ENDPOINT)
        .json(ask)
        .send()
        .await
        .map_err(|err| Error::Request(err.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::Request(format!("{}", response.status())));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|err| Error::Answer(err.to_string()))?;
    Ok(read(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_opponent_is_read_from_the_ai_names_in_the_room() {
        assert_eq!(ai_type(&names(&["RaptorsAI"])), Some(AiType::Raptors));
        assert_eq!(ai_type(&names(&["ScavengersAI"])), Some(AiType::Scavengers));
        assert_eq!(ai_type(&names(&["BARb"])), Some(AiType::Barbarian));
        // However it happens to be spelled.
        assert_eq!(ai_type(&names(&["raptor"])), Some(AiType::Raptors));
    }

    #[test]
    fn a_room_with_both_is_not_guessed_at() {
        // Nothing the model was trained on, so a confident number here would
        // describe a game nobody has played.
        assert_eq!(ai_type(&names(&["RaptorsAI", "ScavengersAI"])), None);
    }

    #[test]
    fn a_room_with_no_pve_opponent_scores_nothing() {
        assert_eq!(ai_type(&names(&[])), None);
        assert_eq!(ai_type(&names(&["NullAI", "SimpleAI"])), None);
    }

    #[test]
    fn the_numbers_are_read_from_where_the_widget_reads_them() {
        let body = serde_json::json!({
            "difficulty_histogram": {"current_difficulty": 21.5, "current_percentile": 88},
            "difficulty_estimate": {"player_win_probability": 0.31, "evidence_games": 4200},
        });
        let score = read(&body);
        assert_eq!(score.challenge, Some(21.5));
        assert_eq!(score.percentile, Some(88.0));
        assert_eq!(score.win_chance, Some(0.31));
        assert_eq!(score.evidence_games, Some(4200.0));
        assert!(!score.best_effort);
    }

    #[test]
    fn an_answer_missing_pieces_leaves_blanks_rather_than_failing() {
        let score = read(&serde_json::json!({"difficulty_estimate": {}}));
        assert_eq!(score.challenge, None);
        assert_eq!(score.win_chance, None);
    }

    #[test]
    fn a_setup_the_service_does_not_recognise_is_marked_best_effort() {
        let body = serde_json::json!({
            "degradation": {"reason": "unknown_modoptions", "unknown_setting_count": 3}
        });
        assert!(read(&body).best_effort);
        // Anything else it might say is not that.
        let other = serde_json::json!({"degradation": {"reason": "something else"}});
        assert!(!read(&other).best_effort);
    }

    #[test]
    fn nothing_about_a_person_is_in_what_we_send() {
        let ask = Ask {
            ai_type: AiType::Raptors.as_str(),
            map: "Comet Catcher Remake 1.8".into(),
            game_settings: BTreeMap::from([("raptor_endless".to_owned(), "1".to_owned())]),
            encounter_context: Encounter {
                human_team_size: 8,
                enemy_ai_count: None,
            },
        };
        let json = serde_json::to_string(&ask).unwrap();
        assert!(!json.contains("player"));
        assert!(!json.contains("account"));
        assert!(json.contains("human_team_size"));
    }
}
