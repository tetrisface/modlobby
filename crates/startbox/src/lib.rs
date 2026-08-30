//! Where each team starts.
//!
//! There are two mechanisms and BAR is in the middle of moving between them,
//! so both are here.
//!
//! The old one is the lobby protocol's `ADDSTARTRECT`: one rectangle per ally
//! team, in 0-200 coordinates. It is a **mod command** on teiserver — founder,
//! moderator or coordinator only (`lobby.ex:746`) — so in a SPADS room a boss
//! cannot send it at all, and anything else it sends is dropped in silence.
//! What a boss gets instead is `!split h|v|c1|c2|c|s <percent>`, which can only
//! make symmetric halves and corners.
//!
//! The new one is two modoptions, which a boss *can* set, and which carry
//! arbitrary polygons:
//!
//! - `mapmetadata_startboxes_set` — `base64url(zlib(json))` of
//!   `{ "<team count>": arrangement, … }`
//! - `mapmetadata_startbox_override` — the same encoding, one arrangement
//!
//! The game's own reader is
//! `luarules/gadgets/include/startbox_utilities.lua`, which opens by calling
//! itself "the canonical contract for lobby implementations" and asking that
//! any lobby-side resolution mirror its `resolveArrangement` so that what a
//! lobby draws matches what the game enforces. [`resolve`] is that mirror, and
//! the tests below are written against its rules rather than against what
//! seemed reasonable.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A point, in the same 0-200 space as `ADDSTARTRECT`.
///
/// `y` is the map's Z axis; the game reads these as `p.x, p.y` and scales both
/// by `mapSize / 200`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    /// Spline weight, when the box is a curve rather than straight edges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
}

/// One team's box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Box {
    /// Two points are opposite corners of a rectangle; three or more are a
    /// polygon in order.
    pub poly: Vec<Point>,
}

/// One team-count's worth of boxes. The index in `startboxes` is the ally team.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Arrangement {
    pub startboxes: Vec<Box>,
}

/// Which modoption an arrangement came from, which is worth showing: an
/// override is somebody's deliberate choice, a set entry is the map's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Source {
    Override,
    Set,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not base64url: {0}")]
    Base64(String),
    #[error("not zlib data: {0}")]
    Zlib(String),
    #[error("not a startbox arrangement: {0}")]
    Json(String),
}

fn base64url() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

/// Decodes one of the two modoptions into whatever JSON it holds.
///
/// The padding is tolerated because encoders disagree about it, and the
/// standard alphabet is tolerated because a value pasted through something
/// that re-encoded it will have `+` and `/` in it.
fn decode_json(raw: &str) -> Result<serde_json::Value, Error> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .filter(|c| *c != '=')
        .collect();
    let bytes = base64url()
        .decode(cleaned.as_bytes())
        .map_err(|err| Error::Base64(err.to_string()))?;

    use std::io::Read as _;
    let mut text = String::new();
    flate2::read::ZlibDecoder::new(&bytes[..])
        .read_to_string(&mut text)
        .map_err(|err| Error::Zlib(err.to_string()))?;

    serde_json::from_str(&text).map_err(|err| Error::Json(err.to_string()))
}

/// One arrangement, from `mapmetadata_startbox_override`.
pub fn decode_override(raw: &str) -> Result<Arrangement, Error> {
    serde_json::from_value(decode_json(raw)?).map_err(|err| Error::Json(err.to_string()))
}

/// The per-team-count table, from `mapmetadata_startboxes_set`.
pub fn decode_set(raw: &str) -> Result<BTreeMap<u32, Arrangement>, Error> {
    let value = decode_json(raw)?;
    let table: BTreeMap<String, Arrangement> =
        serde_json::from_value(value).map_err(|err| Error::Json(err.to_string()))?;
    Ok(table
        .into_iter()
        .filter_map(|(key, arrangement)| Some((key.trim().parse().ok()?, arrangement)))
        .collect())
}

/// Encodes an arrangement the way the game reads it back.
pub fn encode_override(arrangement: &Arrangement) -> Result<String, Error> {
    encode(&serde_json::to_value(arrangement).map_err(|err| Error::Json(err.to_string()))?)
}

/// Encodes a per-team-count table.
pub fn encode_set(set: &BTreeMap<u32, Arrangement>) -> Result<String, Error> {
    let table: BTreeMap<String, &Arrangement> = set
        .iter()
        .map(|(count, arrangement)| (count.to_string(), arrangement))
        .collect();
    encode(&serde_json::to_value(table).map_err(|err| Error::Json(err.to_string()))?)
}

fn encode(value: &serde_json::Value) -> Result<String, Error> {
    use std::io::Write as _;
    let text = serde_json::to_string(value).map_err(|err| Error::Json(err.to_string()))?;
    let mut writer = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    writer
        .write_all(text.as_bytes())
        .map_err(|err| Error::Zlib(err.to_string()))?;
    let bytes = writer
        .finish()
        .map_err(|err| Error::Zlib(err.to_string()))?;
    Ok(base64url().encode(bytes))
}

/// Which arrangement the game will use for a given team count.
///
/// Mirrors `resolveArrangement`: an override wins if it has at least as many
/// boxes as there are teams; otherwise the set is asked for an exact match,
/// then the smallest larger, then the largest smaller. `None` means no
/// modoption applies and the engine's own start rects stand — which is what
/// makes it right to keep drawing those as well.
pub fn resolve(
    over: Option<&Arrangement>,
    set: &BTreeMap<u32, Arrangement>,
    teams: u32,
) -> Option<(Arrangement, Source)> {
    // "Will match any spare boxes, but will not leave any teams without a box."
    if let Some(held) = over
        && held.startboxes.len() as u32 >= teams
    {
        return Some((held.clone(), Source::Override));
    }
    if let Some(held) = set.get(&teams) {
        return Some((held.clone(), Source::Set));
    }
    if let Some((_, held)) = set.range(teams + 1..).next() {
        return Some((held.clone(), Source::Set));
    }
    if let Some((_, held)) = set.range(..teams).next_back() {
        return Some((held.clone(), Source::Set));
    }
    None
}

impl Box {
    /// The corners to draw, in order.
    ///
    /// Two points are opposite corners and become four; anything else is
    /// already a polygon. Mirrors `expandPoly`.
    pub fn corners(&self) -> Vec<Point> {
        if self.poly.len() != 2 {
            return self.poly.clone();
        }
        let (a, b) = (self.poly[0], self.poly[1]);
        let at = |x: f32, y: f32| Point {
            x,
            y,
            strength: None,
        };
        vec![at(a.x, a.y), at(b.x, a.y), at(b.x, b.y), at(a.x, b.y)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Box {
        Box {
            poly: vec![
                Point {
                    x: left,
                    y: top,
                    strength: None,
                },
                Point {
                    x: right,
                    y: bottom,
                    strength: None,
                },
            ],
        }
    }

    fn arrangement(count: usize) -> Arrangement {
        Arrangement {
            startboxes: (0..count)
                .map(|n| rect(n as f32 * 10.0, 0.0, n as f32 * 10.0 + 5.0, 200.0))
                .collect(),
        }
    }

    #[test]
    fn an_arrangement_survives_the_encoding_the_game_reads() {
        let one = arrangement(2);
        let encoded = encode_override(&one).unwrap();
        // base64url, so none of the characters a modoption value cannot carry.
        assert!(!encoded.contains('+') && !encoded.contains('/') && !encoded.contains('='));
        assert_eq!(decode_override(&encoded).unwrap(), one);
    }

    #[test]
    fn a_set_is_keyed_by_team_count_as_a_string_on_the_wire() {
        let mut set = BTreeMap::new();
        set.insert(2, arrangement(2));
        set.insert(4, arrangement(4));
        let encoded = encode_set(&set).unwrap();
        assert_eq!(decode_set(&encoded).unwrap(), set);
    }

    #[test]
    fn an_override_wins_when_it_has_a_box_for_every_team() {
        let mut set = BTreeMap::new();
        set.insert(2, arrangement(2));
        let over = arrangement(4);
        let (held, source) = resolve(Some(&over), &set, 2).unwrap();
        assert_eq!(source, Source::Override);
        assert_eq!(held.startboxes.len(), 4, "spare boxes are fine");
    }

    #[test]
    fn an_override_too_small_for_the_room_is_passed_over() {
        let mut set = BTreeMap::new();
        set.insert(4, arrangement(4));
        let over = arrangement(2);
        // Four teams and an override with two boxes would leave two teams
        // without one, so the set answers instead.
        let (_, source) = resolve(Some(&over), &set, 4).unwrap();
        assert_eq!(source, Source::Set);
    }

    #[test]
    fn the_set_is_asked_for_exact_then_larger_then_smaller() {
        let mut set = BTreeMap::new();
        set.insert(2, arrangement(2));
        set.insert(8, arrangement(8));

        assert_eq!(resolve(None, &set, 2).unwrap().0.startboxes.len(), 2);
        // Nothing for 4, and 8 is the smallest larger.
        assert_eq!(resolve(None, &set, 4).unwrap().0.startboxes.len(), 8);
        // Nothing at or above 16, so the largest smaller.
        assert_eq!(resolve(None, &set, 16).unwrap().0.startboxes.len(), 8);
    }

    #[test]
    fn nothing_at_all_defers_to_the_engine_rects() {
        assert!(resolve(None, &BTreeMap::new(), 2).is_none());
    }

    #[test]
    fn two_points_are_corners_and_become_a_rectangle() {
        let corners = rect(0.0, 0.0, 50.0, 200.0).corners();
        assert_eq!(corners.len(), 4);
        assert_eq!((corners[1].x, corners[1].y), (50.0, 0.0));
        assert_eq!((corners[3].x, corners[3].y), (0.0, 200.0));
    }

    #[test]
    fn a_real_polygon_is_left_as_it_was_drawn() {
        let triangle = Box {
            poly: vec![
                Point {
                    x: 0.0,
                    y: 0.0,
                    strength: None,
                },
                Point {
                    x: 100.0,
                    y: 0.0,
                    strength: None,
                },
                Point {
                    x: 50.0,
                    y: 90.0,
                    strength: Some(0.5),
                },
            ],
        };
        assert_eq!(triangle.corners(), triangle.poly);
    }

    #[test]
    fn rubbish_is_refused_at_the_layer_that_recognises_it() {
        assert!(matches!(decode_override("!!!!"), Err(Error::Base64(_))));
        // Valid base64url, but not compressed data.
        assert!(matches!(decode_override("aGVsbG8"), Err(Error::Zlib(_))));
        assert!(decode_override("").is_err());
    }
}
