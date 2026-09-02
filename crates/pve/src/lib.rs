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
//! The widget reaches the service over plain HTTP because the engine's Lua has
//! no TLS. The same CloudFront distribution answers over TLS, and this is Rust
//! with rustls in the room, so this asks over `https://` -- the settings of a
//! room are nobody's secret, but there is no reason to send them in the clear.
//!
//! The HTTP client is handed in, not built here: modlobby has one client for
//! everything it asks BAR, so that every request carries the same name and
//! shares one connection pool.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

/// The service the in-game widget talks to (`include/remote.lua:16-18`),
/// over TLS rather than the widget's plain HTTP.
pub const ENDPOINT: &str = "https://d29i3oohxql6zz.cloudfront.net/api/v1/stats";

/// The request body limit the widget documents for this service.
///
/// Worth knowing because a heavily modded room gets close to it: the biggest
/// room in a real presets file on this machine serialises to 237 KiB, of which
/// 233 KiB is base64 tweak Lua. The settings are what the service matches on,
/// so they cannot simply be dropped — but a room that would not fit is worth
/// saying so about rather than sending and reading back a bare 413.
pub const BODY_LIMIT: usize = 256 * 1024;

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
    /// One income multiplier per seated human, `1.0` being no handicap.
    ///
    /// Not optional in practice. The service derives its `Player Handicap`
    /// column from the average of these, and a governed column it cannot
    /// derive is a *missing* column rather than a defaulted one — which
    /// suppresses the difficulty estimate entirely. Sending an empty list is
    /// how a request comes back with a histogram, a closest match, and no
    /// score at all.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub human_player_income_multipliers: Vec<f64>,
    /// One income multiplier per BARbarian, `1.0` being no handicap.
    ///
    /// The Barbarian counterpart of the list above, and just as necessary:
    /// the service derives its `Barbarian Handicap` column from these, and
    /// without them every BARbarian room comes back unplaced — a histogram,
    /// a closest match, and no score. Meaningless for raptors and scavengers,
    /// which have no per-slot handicap, so it stays empty for them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub enemy_ai_income_multipliers: Vec<f64>,
}

/// The income multiplier a seated player's handicap amounts to.
///
/// `MYBATTLESTATUS` carries handicap as a percentage in bits 11-17; the
/// service works in multipliers, and derives the column as the average of
/// `(multiplier - 1) * 100` — so a room where nobody is handicapped is a list
/// of `1.0`, which is not the same thing as no list at all.
pub fn income_multiplier(handicap_percent: u8) -> f64 {
    1.0 + f64::from(handicap_percent) / 100.0
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
    #[error("this room's settings are {size} KiB, past the {limit} KiB the service takes")]
    TooBig { size: usize, limit: usize },
    #[error("pve stats did not answer within {0:?}")]
    Timeout(Duration),
    #[error("pve stats: {0}")]
    Request(String),
    #[error("pve stats answered {0}")]
    Status(u16),
    /// Someone else is being answered. A Lambda at its concurrency limit says
    /// 429, and a room full of clients asking at once is exactly how it gets
    /// there; when the service names a wait, that wait is kept.
    #[error("pve stats is busy")]
    Throttled { retry_after: Option<Duration> },
    #[error("pve stats answered with something unexpected: {0}")]
    Answer(String),
}

impl Error {
    /// Whether asking again could reasonably go differently.
    ///
    /// The service is a Lambda behind CloudFront, and a Lambda nobody has
    /// asked for a while takes some twenty seconds to wake up. The first
    /// question of the evening can therefore run out the clock — CloudFront's
    /// as well as ours — while the answer is being prepared, and by the time
    /// that failure comes back the function is warm and the same question
    /// takes half a second. Timeouts, dropped connections, gateway errors and
    /// a busy signal are all that story. A rejected request or an unreadable
    /// answer is not: asking the same thing again would get the same thing.
    pub fn retryable(&self) -> bool {
        match self {
            Error::Timeout(_) | Error::Request(_) | Error::Throttled { .. } => true,
            Error::Status(code) => (500..600).contains(code),
            Error::NotPve | Error::TooBig { .. } | Error::Answer(_) => false,
        }
    }
}

/// How long to keep asking a service that may still be waking up.
///
/// The in-game widget's own policy is a 30-second deadline per attempt and a
/// doubling pause between attempts starting at two seconds; this is that with
/// fewer attempts, because a lobby panel showing dots for a minute and a half
/// has stopped being useful.
#[derive(Debug, Clone, Copy)]
pub struct Patience {
    /// The deadline for one attempt, connection and answer included.
    pub attempt: Duration,
    /// How many attempts at most; only retryable failures use the extra ones.
    pub attempts: u32,
    /// The pause before the second attempt; each later one waits twice as long.
    pub first_pause: Duration,
    /// Spreads a pause so a room's clients do not retry in lockstep.
    pub jitter: fn(Duration) -> Duration,
}

impl Patience {
    /// Widget parity on the deadline, three attempts: a cold start that
    /// outruns the first attempt is warm by the second.
    pub const DEFAULT: Self = Self {
        attempt: Duration::from_secs(30),
        attempts: 3,
        first_pause: Duration::from_secs(2),
        jitter: jittered,
    };

    /// The scheduled pause before the given attempt, `None` once there are
    /// no more.
    pub fn pause_before(self, attempt: u32) -> Option<Duration> {
        if attempt < 2 || attempt > self.attempts {
            return None;
        }
        Some(self.first_pause * 2u32.pow(attempt - 2))
    }

    /// How long to wait before the given attempt after the given failure,
    /// `None` when there is no point or no attempt left.
    ///
    /// A wait the service named is kept as it is; a pause of our own is
    /// jittered.
    pub fn pause_after(self, failed: &Error, attempt: u32) -> Option<Duration> {
        let scheduled = self.pause_before(attempt).filter(|_| failed.retryable())?;
        Some(match failed {
            Error::Throttled {
                retry_after: Some(said),
            } => *said,
            _ => (self.jitter)(scheduled),
        })
    }
}

/// The longest wait a `Retry-After` is taken at its word for.
pub const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);

/// A pause somewhere between three quarters and five quarters of itself.
///
/// Enough to keep a room's clients from retrying in lockstep, sourced from
/// the clock's nanoseconds because that is all the randomness this needs.
pub fn jittered(pause: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos())
        .unwrap_or(0);
    let spread = f64::from(nanos % 1000) / 1000.0;
    pause.mul_f64(0.75 + spread * 0.5)
}

/// No jitter: a pause as scheduled, for tests that want to know.
pub fn exact(pause: Duration) -> Duration {
    pause
}

/// The wait a `Retry-After` header names, in the seconds form only, capped.
pub fn retry_after(value: Option<&str>) -> Option<Duration> {
    let seconds: u64 = value?.trim().parse().ok()?;
    Some(Duration::from_secs(seconds).min(RETRY_AFTER_CAP))
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

/// The service as one client sees it: the transport it was handed, where the
/// service lives, and what it has already answered.
///
/// The client is injected rather than built here so the process has one
/// connection pool and one `User-Agent` for everything it asks BAR; the
/// endpoint is a parameter so a test can stand up a server of its own.
pub struct Service {
    client: reqwest::Client,
    endpoint: String,
    memo: Mutex<Memo>,
}

impl Service {
    pub fn new(client: reqwest::Client, endpoint: impl Into<String>) -> Self {
        Self {
            client,
            endpoint: endpoint.into(),
            memo: Mutex::new(Memo::default()),
        }
    }

    /// What a setup scores, with [`Patience::DEFAULT`].
    pub async fn score(&self, ask: &Ask) -> Result<Score, Error> {
        self.score_with(ask, Patience::DEFAULT).await
    }

    /// What a setup scores, trying again as the patience allows.
    ///
    /// A setup this process has already been answered about is answered from
    /// memory: the service scores a setup, not a moment, so the same question
    /// gets the same number, and the panel re-mounting or a setting toggled
    /// and toggled back is not worth a Lambda's cold start.
    pub async fn score_with(&self, ask: &Ask, patience: Patience) -> Result<Score, Error> {
        let body = serde_json::to_vec(ask).map_err(|err| Error::Request(err.to_string()))?;
        if body.len() > BODY_LIMIT {
            return Err(Error::TooBig {
                size: body.len() / 1024,
                limit: BODY_LIMIT / 1024,
            });
        }
        let key = Memo::key(&body);
        if let Some(held) = self.memo.lock().expect("pve memo").get(&key) {
            tracing::debug!("pve stats: answered from memory");
            return Ok(held);
        }

        let mut attempt = 1;
        loop {
            let asked =
                ask_once(&self.client, &self.endpoint, body.clone(), patience.attempt).await;
            let failed = match asked {
                Ok(score) => {
                    self.memo.lock().expect("pve memo").put(key, score.clone());
                    return Ok(score);
                }
                Err(err) => err,
            };
            let Some(pause) = patience.pause_after(&failed, attempt + 1) else {
                return Err(failed);
            };
            tracing::info!(error = %failed, attempt, ?pause, "pve stats: asking again after a pause");
            tokio::time::sleep(pause).await;
            attempt += 1;
        }
    }
}

/// How many distinct setups are remembered before the oldest is forgotten.
///
/// A room's evening is a handful of setups revisited; the number is small
/// because an entry can carry a few hundred kilobytes of tweak Lua.
pub const MEMO_CAP: usize = 32;

/// Answers already received, keyed by the exact request body.
#[derive(Debug, Default)]
pub struct Memo {
    held: VecDeque<([u8; 32], Score)>,
}

impl Memo {
    /// The body's hash: the service scores the body and nothing else, so two
    /// equal bodies are one question.
    pub fn key(body: &[u8]) -> [u8; 32] {
        Sha256::digest(body).into()
    }

    pub fn get(&self, key: &[u8; 32]) -> Option<Score> {
        self.held
            .iter()
            .find(|(held, _)| held == key)
            .map(|(_, score)| score.clone())
    }

    /// Remembers an answer; the oldest goes once the cap is reached.
    pub fn put(&mut self, key: [u8; 32], score: Score) {
        if self.get(&key).is_some() {
            return;
        }
        self.held.push_back((key, score));
        while self.held.len() > MEMO_CAP {
            self.held.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

/// One attempt, classified so the caller can tell a cold service from a
/// refusal. The deadline covers the connection and the whole answer.
async fn ask_once(
    client: &reqwest::Client,
    endpoint: &str,
    body: Vec<u8>,
    deadline: Duration,
) -> Result<Score, Error> {
    let response = client
        .post(endpoint)
        .timeout(deadline)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                Error::Timeout(deadline)
            } else {
                Error::Request(err.to_string())
            }
        })?;
    let status = response.status();
    let busy = status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::SERVICE_UNAVAILABLE;
    if busy {
        let said = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok());
        return Err(Error::Throttled {
            retry_after: retry_after(said),
        });
    }
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    let body: serde_json::Value = response.json().await.map_err(|err| {
        if err.is_timeout() {
            Error::Timeout(deadline)
        } else {
            Error::Answer(err.to_string())
        }
    })?;
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
    fn a_room_too_big_for_the_service_is_named_as_such() {
        // Twenty filled tweak slots is what gets a room near the limit, and
        // 233 KiB of the 237 in the largest real one is exactly that.
        let mut settings = BTreeMap::new();
        for slot in 0..20 {
            settings.insert(format!("tweakdefs{slot}"), "A".repeat(16_000));
        }
        let ask = Ask {
            ai_type: AiType::Raptors.as_str(),
            map: "Full Metal Plate 1.7".into(),
            game_settings: settings,
            encounter_context: Encounter {
                human_team_size: 8,
                enemy_ai_count: None,
                human_player_income_multipliers: vec![1.0; 8],
                enemy_ai_income_multipliers: vec![],
            },
        };
        assert!(serde_json::to_vec(&ask).unwrap().len() > BODY_LIMIT);
    }

    #[test]
    fn a_handicap_becomes_the_multiplier_the_service_averages() {
        assert_eq!(income_multiplier(0), 1.0);
        assert_eq!(income_multiplier(50), 1.5);
        assert_eq!(income_multiplier(100), 2.0);
    }

    #[test]
    fn an_unhandicapped_room_still_sends_a_multiplier_for_every_seat() {
        // The distinction that decides whether a score comes back at all: a
        // room where nobody is handicapped is a list of 1.0, not an empty list.
        let ask = Ask {
            ai_type: AiType::Raptors.as_str(),
            map: "Comet Catcher Remake 1.8".into(),
            game_settings: BTreeMap::new(),
            encounter_context: Encounter {
                human_team_size: 3,
                enemy_ai_count: None,
                human_player_income_multipliers: vec![1.0, 1.0, 1.0],
                enemy_ai_income_multipliers: vec![],
            },
        };
        let json = serde_json::to_string(&ask).unwrap();
        assert!(json.contains("human_player_income_multipliers"));
        assert!(json.contains("[1.0,1.0,1.0]"));
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
                human_player_income_multipliers: vec![1.0],
                enemy_ai_income_multipliers: vec![],
            },
        };
        let json = serde_json::to_string(&ask).unwrap();
        // Named for what it guards, not for the word: the encounter context
        // legitimately talks about players in the aggregate. What must never
        // appear is anything identifying one.
        assert!(!json.contains("player_names"));
        assert!(!json.contains("player_ids"));
        assert!(!json.contains("account"));
        assert!(!json.contains("game_id"));
        assert!(json.contains("human_team_size"));
    }

    #[test]
    fn a_barbarian_room_sends_a_multiplier_for_every_barbarian() {
        // The Barbarian counterpart of the seat multipliers, and the difference
        // between a score and "unplaced": the service derives its `Barbarian
        // Handicap` column from these and declines to place a room without it.
        let ask = Ask {
            ai_type: AiType::Barbarian.as_str(),
            map: "Comet Catcher Remake 1.8".into(),
            game_settings: BTreeMap::new(),
            encounter_context: Encounter {
                human_team_size: 2,
                enemy_ai_count: Some(2),
                human_player_income_multipliers: vec![1.0, 1.0],
                enemy_ai_income_multipliers: vec![income_multiplier(0), income_multiplier(50)],
            },
        };
        let json = serde_json::to_string(&ask).unwrap();
        assert!(json.contains("\"enemy_ai_count\":2"));
        assert!(json.contains("\"enemy_ai_income_multipliers\":[1.0,1.5]"));
    }

    #[test]
    fn waking_the_service_is_worth_another_try_but_a_refusal_is_not() {
        assert!(Error::Timeout(Duration::from_secs(30)).retryable());
        assert!(Error::Request("connection reset".into()).retryable());
        assert!(Error::Status(502).retryable());
        assert!(Error::Status(504).retryable());
        assert!(!Error::Status(400).retryable());
        assert!(!Error::Status(413).retryable());
        assert!(!Error::Answer("not json".into()).retryable());
        assert!(
            !Error::TooBig {
                size: 300,
                limit: 256
            }
            .retryable()
        );
    }

    #[test]
    fn the_pauses_double_and_then_stop() {
        let patience = Patience::DEFAULT;
        assert_eq!(patience.pause_before(1), None, "nothing before the first");
        assert_eq!(patience.pause_before(2), Some(Duration::from_secs(2)));
        assert_eq!(patience.pause_before(3), Some(Duration::from_secs(4)));
        assert_eq!(patience.pause_before(4), None, "three attempts is the lot");
    }

    #[test]
    fn a_busy_service_is_asked_again_after_the_wait_it_named() {
        let exactly = Patience {
            jitter: exact,
            ..Patience::DEFAULT
        };
        let busy = Error::Throttled {
            retry_after: Some(Duration::from_secs(1)),
        };
        assert!(busy.retryable());
        assert_eq!(
            exactly.pause_after(&busy, 2),
            Some(Duration::from_secs(1)),
            "the service's word beats the schedule"
        );
        let unsaid = Error::Throttled { retry_after: None };
        assert_eq!(
            exactly.pause_after(&unsaid, 2),
            Some(Duration::from_secs(2)),
            "no word, so the schedule"
        );
        assert_eq!(exactly.pause_after(&busy, 4), None, "no attempts left");
        assert_eq!(exactly.pause_after(&Error::Status(400), 2), None);
    }

    #[test]
    fn retry_after_is_read_in_seconds_and_capped() {
        assert_eq!(retry_after(Some("1")), Some(Duration::from_secs(1)));
        assert_eq!(retry_after(Some(" 5 ")), Some(Duration::from_secs(5)));
        assert_eq!(retry_after(Some("900")), Some(RETRY_AFTER_CAP));
        // The HTTP-date form is not worth parsing for a pause of seconds.
        assert_eq!(retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")), None);
        assert_eq!(retry_after(None), None);
    }

    #[test]
    fn jitter_stays_within_a_quarter_either_way() {
        let base = Duration::from_secs(4);
        for _ in 0..200 {
            let held = jittered(base);
            assert!(held >= Duration::from_secs(3), "{held:?}");
            assert!(held <= Duration::from_secs(5), "{held:?}");
        }
    }

    fn ask() -> Ask {
        Ask {
            ai_type: AiType::Raptors.as_str(),
            map: "Comet Catcher Remake 1.8".into(),
            game_settings: BTreeMap::from([("raptor_endless".to_owned(), "1".to_owned())]),
            encounter_context: Encounter {
                human_team_size: 2,
                enemy_ai_count: None,
                human_player_income_multipliers: vec![1.0, 1.0],
                enemy_ai_income_multipliers: vec![],
            },
        }
    }

    #[test]
    fn the_memo_answers_the_same_body_and_forgets_the_oldest() {
        let mut memo = Memo::default();
        let score = read(&serde_json::json!({"difficulty_histogram": {"current_difficulty": 1.0}}));
        let key = Memo::key(b"a");
        assert_eq!(memo.get(&key), None);
        memo.put(key, score.clone());
        memo.put(key, score.clone());
        assert_eq!(memo.len(), 1, "the same key is one entry");
        assert_eq!(memo.get(&key), Some(score.clone()));

        for n in 0..MEMO_CAP {
            memo.put(Memo::key(&n.to_le_bytes()), score.clone());
        }
        assert_eq!(memo.len(), MEMO_CAP);
        assert_eq!(
            memo.get(&key),
            None,
            "the first one in is the first one out"
        );
    }

    #[tokio::test]
    async fn the_same_setup_is_asked_about_once() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/stats"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "difficulty_histogram": {"current_difficulty": 21.5, "current_percentile": 88},
                "difficulty_estimate": {"player_win_probability": 0.31, "evidence_games": 4200}
            })))
            .expect(2)
            .mount(&server)
            .await;
        let service = Service::new(
            content::http::client("test"),
            format!("{}/api/v1/stats", server.uri()),
        );

        let first = service.score(&ask()).await.unwrap();
        let again = service.score(&ask()).await.unwrap();
        assert_eq!(first, again);
        assert_eq!(first.challenge, Some(21.5));

        // A different setup is a different question.
        let mut other = ask();
        other.encounter_context.human_team_size = 3;
        service.score(&other).await.unwrap();
    }

    #[tokio::test]
    async fn a_refusal_is_not_remembered() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400))
            .expect(2)
            .mount(&server)
            .await;
        let service = Service::new(content::http::client("test"), server.uri());
        assert!(matches!(
            service.score(&ask()).await,
            Err(Error::Status(400))
        ));
        assert!(matches!(
            service.score(&ask()).await,
            Err(Error::Status(400))
        ));
        assert!(service.memo.lock().unwrap().is_empty());
    }
}
