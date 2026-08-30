//! Where the teams start, as the game will actually decide it.
//!
//! There are two systems and a room can be using either. The old one is the
//! protocol's own `ADDSTARTRECT`, which is what `!split` sends and what the
//! battle already carries. The new one is a pair of modoptions holding
//! base64url(zlib(json)) — `mapmetadata_startboxes_set`, the map's own
//! arrangements keyed by team count, and `mapmetadata_startbox_override`,
//! somebody's deliberate choice for this room.
//!
//! Which one wins is not a matter of taste: `resolve` mirrors the game's
//! `resolveArrangement`, and when it answers `None` no modoption applies and
//! the engine's start rects stand. So both are drawn, and the one that counts
//! is labelled.

use std::collections::BTreeMap;

use serde::Serialize;
use tauri::State;

use crate::commands::{ApiError, Result};
use crate::state::App;

const OVERRIDE: &str = "game/modoptions/mapmetadata_startbox_override";
const SET: &str = "game/modoptions/mapmetadata_startboxes_set";

/// The boxes to draw, already flattened into polygons in the 0-200 space the
/// minimap uses.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BoxesView {
    /// One polygon per ally team, in ally-team order.
    pub polys: Vec<Vec<[f32; 2]>>,
    /// `override` when somebody chose these for this room, `set` when they are
    /// the map's own for this many teams.
    pub source: String,
    /// The team count they were resolved for.
    pub teams: u32,
    /// Whether the map offers arrangements for other team counts too, which is
    /// what makes a "boxes for N teams" choice meaningful.
    pub available: Vec<u32>,
}

/// What the game will use for `teams` ally teams, or `None` when the modoptions
/// say nothing and the room's own start rects are the whole story.
#[tauri::command]
pub async fn start_boxes(app: State<'_, App>, teams: u32) -> Result<Option<BoxesView>> {
    let snapshot = app.client.snapshot().await.map_err(ApiError::from)?;
    let Some(my) = snapshot.my_battle else {
        return Ok(None);
    };
    Ok(from_tags(&my.script_tags, teams))
}

/// The decision, given the room's script tags. Separated from the command so
/// it can be tested without a lobby.
fn from_tags(tags: &BTreeMap<String, String>, teams: u32) -> Option<BoxesView> {
    // SPADS clears a modoption by setting it to `0`, so an empty slot arrives
    // as a value rather than as an absent key.
    let held = |key: &str| {
        tags.get(key)
            .filter(|raw| !raw.is_empty() && raw.as_str() != "0")
    };

    let over = held(OVERRIDE).and_then(|raw| startbox::decode_override(raw).ok());
    let set: BTreeMap<u32, startbox::Arrangement> = held(SET)
        .and_then(|raw| startbox::decode_set(raw).ok())
        .unwrap_or_default();

    let available: Vec<u32> = set.keys().copied().collect();
    let (arrangement, source) = startbox::resolve(over.as_ref(), &set, teams)?;

    Some(BoxesView {
        polys: arrangement
            .startboxes
            .iter()
            .map(|shape| {
                shape
                    .corners()
                    .into_iter()
                    .map(|point| [point.x, point.y])
                    .collect()
            })
            .collect(),
        source: match source {
            startbox::Source::Override => "override",
            startbox::Source::Set => "set",
        }
        .to_owned(),
        teams,
        available,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// One arrangement of `n` boxes, encoded the way a room carries it.
    fn arrangement(n: usize) -> startbox::Arrangement {
        startbox::Arrangement {
            startboxes: (0..n)
                .map(|i| startbox::Box {
                    poly: vec![
                        startbox::Point {
                            x: i as f32,
                            y: 0.0,
                            strength: None,
                        },
                        startbox::Point {
                            x: i as f32 + 10.0,
                            y: 20.0,
                            strength: None,
                        },
                    ],
                })
                .collect(),
        }
    }

    #[test]
    fn a_room_with_no_startbox_modoptions_says_nothing() {
        assert!(from_tags(&tags(&[]), 2).is_none());
    }

    #[test]
    fn a_cleared_modoption_is_not_an_arrangement() {
        // SPADS writes `0` to empty a slot, and `0` decodes to nothing useful;
        // treating it as data would draw an empty box set over a good one.
        let cleared = tags(&[(OVERRIDE, "0"), (SET, "")]);
        assert!(from_tags(&cleared, 2).is_none());
    }

    #[test]
    fn an_override_wins_and_says_so() {
        let raw = startbox::encode_override(&arrangement(2)).unwrap();
        let view = from_tags(&tags(&[(OVERRIDE, &raw)]), 2).expect("boxes");
        assert_eq!(view.source, "override");
        assert_eq!(view.polys.len(), 2);
        // Two points are opposite corners, so each becomes a four-sided shape.
        assert_eq!(view.polys[0].len(), 4);
    }

    #[test]
    fn the_maps_own_boxes_are_chosen_by_team_count() {
        let mut set = BTreeMap::new();
        set.insert(2, arrangement(2));
        set.insert(4, arrangement(4));
        let raw = startbox::encode_set(&set).unwrap();

        let two = from_tags(&tags(&[(SET, &raw)]), 2).expect("boxes");
        assert_eq!(two.source, "set");
        assert_eq!(two.polys.len(), 2);
        assert_eq!(two.available, vec![2, 4], "and what else is on offer");

        let four = from_tags(&tags(&[(SET, &raw)]), 4).expect("boxes");
        assert_eq!(four.polys.len(), 4);
    }

    #[test]
    fn an_override_too_small_for_the_room_gives_way_to_the_set() {
        // "Will not leave any teams without a box": a two-box override cannot
        // serve four teams, so the map's own four-team arrangement is used.
        let mut set = BTreeMap::new();
        set.insert(4, arrangement(4));
        let view = from_tags(
            &tags(&[
                (
                    OVERRIDE,
                    &startbox::encode_override(&arrangement(2)).unwrap(),
                ),
                (SET, &startbox::encode_set(&set).unwrap()),
            ]),
            4,
        )
        .expect("boxes");
        assert_eq!(view.source, "set");
        assert_eq!(view.polys.len(), 4);
    }

    #[test]
    fn nonsense_in_a_modoption_is_ignored_rather_than_fatal() {
        // A room can carry anything; a bad blob must not blank the minimap.
        let view = from_tags(&tags(&[(OVERRIDE, "not-base64!!"), (SET, "also-not")]), 2);
        assert!(view.is_none());
    }
}
