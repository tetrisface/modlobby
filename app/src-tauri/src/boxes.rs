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

/// The same, for the room with no server behind it. Its modoptions are kept
/// under the keys the online room uses, so the reading is the same reading.
#[tauri::command]
pub async fn skirmish_start_boxes(app: State<'_, App>, teams: u32) -> Result<Option<BoxesView>> {
    let snapshot = app.client.snapshot().await.map_err(ApiError::from)?;
    let Some(room) = snapshot.skirmish else {
        return Ok(None);
    };
    Ok(from_tags(&room.my.script_tags, teams))
}

/// The boxes in one blob, for showing what a vote or a past change did.
///
/// Takes either modoption's value: an override is one arrangement, a set is
/// many and has to be asked for a team count, exactly as the game would.
/// `None` covers a cleared slot and a blob that will not decode — a proposal
/// nobody can read is worth showing as unreadable rather than as empty.
#[tauri::command]
pub fn decode_boxes(raw: String, teams: u32) -> Option<Vec<Vec<[f32; 2]>>> {
    if raw.is_empty() || raw == "0" {
        return None;
    }
    let arrangement = startbox::decode_override(&raw).ok().or_else(|| {
        let set = startbox::decode_set(&raw).ok()?;
        startbox::resolve(None, &set, teams).map(|(held, _)| held)
    })?;
    Some(polygons(&arrangement))
}

/// One arrangement flattened into drawable polygons in the 0-200 space.
fn polygons(arrangement: &startbox::Arrangement) -> Vec<Vec<[f32; 2]>> {
    arrangement
        .startboxes
        .iter()
        .map(|shape| {
            shape
                .corners()
                .into_iter()
                .map(|point| [point.x, point.y])
                .collect()
        })
        .collect()
}

/// The two modoptions decoded, each absent when unset, cleared or unreadable.
fn decoded(
    tags: &BTreeMap<String, String>,
) -> (
    Option<startbox::Arrangement>,
    BTreeMap<u32, startbox::Arrangement>,
) {
    // SPADS clears a modoption by setting it to `0`, so an empty slot arrives
    // as a value rather than as an absent key.
    let held = |key: &str| {
        tags.get(key)
            .filter(|raw| !raw.is_empty() && raw.as_str() != "0")
    };
    let over = held(OVERRIDE).and_then(|raw| startbox::decode_override(raw).ok());
    let set = held(SET)
        .and_then(|raw| startbox::decode_set(raw).ok())
        .unwrap_or_default();
    (over, set)
}

/// The decision, given the room's script tags. Separated from the command so
/// it can be tested without a lobby.
fn from_tags(tags: &BTreeMap<String, String>, teams: u32) -> Option<BoxesView> {
    let (over, set) = decoded(tags);
    let available: Vec<u32> = set.keys().copied().collect();
    let (arrangement, source) = startbox::resolve(over.as_ref(), &set, teams)?;

    Some(BoxesView {
        polys: polygons(&arrangement),
        source: match source {
            startbox::Source::Override => "override",
            startbox::Source::Set => "set",
        }
        .to_owned(),
        teams,
        available,
    })
}

/// The arrangement the game will use, as the anchors it was written with --
/// curvature included, which the drawn polygons above have already spent.
/// What an editor starts from.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArrangementView {
    pub arrangement: startbox::Arrangement,
    pub source: startbox::Source,
}

#[tauri::command]
pub async fn current_arrangement(
    app: State<'_, App>,
    teams: u32,
) -> Result<Option<ArrangementView>> {
    let snapshot = app.client.snapshot().await.map_err(ApiError::from)?;
    let Some(my) = snapshot.my_battle else {
        return Ok(None);
    };
    Ok(arrangement_from_tags(&my.script_tags, teams))
}

#[tauri::command]
pub async fn skirmish_current_arrangement(
    app: State<'_, App>,
    teams: u32,
) -> Result<Option<ArrangementView>> {
    let snapshot = app.client.snapshot().await.map_err(ApiError::from)?;
    let Some(room) = snapshot.skirmish else {
        return Ok(None);
    };
    Ok(arrangement_from_tags(&room.my.script_tags, teams))
}

fn arrangement_from_tags(tags: &BTreeMap<String, String>, teams: u32) -> Option<ArrangementView> {
    let (over, set) = decoded(tags);
    let (arrangement, source) = startbox::resolve(over.as_ref(), &set, teams)?;
    Some(ArrangementView {
        arrangement,
        source,
    })
}

/// What the override rides behind in chat. The line's cap less this is the
/// blob's budget.
const OVERRIDE_PREFIX: &str = "!bSet mapmetadata_startbox_override ";

/// How long an override blob may be: teiserver cuts `!bset mapmetadata…` at
/// the long-command cap (`spring_in.ex`), and the prefix takes its share.
pub fn override_limit() -> u32 {
    (spring_protocol::policy::saybattle_max_len(OVERRIDE_PREFIX) - OVERRIDE_PREFIX.len()) as u32
}

/// An arrangement as the wire carries it, with how long it was allowed to be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Encoded {
    pub value: String,
    pub limit: u32,
}

/// Encodes a drawn arrangement for `!bSet mapmetadata_startbox_override`.
/// Over the limit is still returned -- the caller shows how far over -- but
/// a shape the game could not read is refused here, before it costs a vote.
#[tauri::command]
pub fn encode_boxes(arrangement: startbox::Arrangement) -> Result<Encoded> {
    check(&arrangement)?;
    let value = startbox::encode_override(&arrangement)
        .map_err(|err| ApiError::new("boxes", err.to_string()))?;
    Ok(Encoded {
        value,
        limit: override_limit(),
    })
}

/// What the game's decoder insists on (`decodeStartboxOverride`): boxes, each
/// of two or more points, every point on the map.
fn check(arrangement: &startbox::Arrangement) -> Result<()> {
    let refuse = |why: &str| Err(ApiError::new("boxes", why));
    if arrangement.startboxes.is_empty() {
        return refuse("an arrangement has at least one box");
    }
    for held in &arrangement.startboxes {
        if held.poly.len() < 2 {
            return refuse("a box has at least two points");
        }
        for p in &held.poly {
            let on_map = |v: f32| v.is_finite() && (0.0..=200.0).contains(&v);
            if !on_map(p.x) || !on_map(p.y) {
                return refuse("a point is on the map: 0-200 on both axes");
            }
            if p.strength
                .is_some_and(|s| !s.is_finite() || !(0.0..=1.0).contains(&s))
            {
                return refuse("curvature is 0-1");
            }
        }
    }
    Ok(())
}

/// One `mapmetadata_*` modoption in words, for a row in the Setup pane: the
/// blobs are base64url(zlib(json)) and say nothing on their own. `""` for a
/// key this does not know; `none` for a cleared slot, since SPADS empties one
/// by writing `0`; `unreadable` for a blob the game could not read either.
#[tauri::command]
pub fn describe_map_option(key: String, raw: String) -> String {
    let empty = raw.is_empty() || raw == "0";
    match key.as_str() {
        "mapmetadata_startboxes_set" if empty => "none".to_owned(),
        "mapmetadata_startboxes_set" => match startbox::decode_set(&raw) {
            Ok(set) if !set.is_empty() => {
                let counts: Vec<String> = set.keys().map(u32::to_string).collect();
                format!("map's own \u{b7} {} teams", counts.join(", "))
            }
            _ => "unreadable".to_owned(),
        },
        "mapmetadata_startbox_override" if empty => "none".to_owned(),
        "mapmetadata_startbox_override" => match startbox::decode_override(&raw) {
            Ok(held) if !held.startboxes.is_empty() => {
                format!(
                    "custom \u{b7} {}",
                    plural(held.startboxes.len(), "box", "boxes")
                )
            }
            _ => "unreadable".to_owned(),
        },
        "mapmetadata_startpos" if empty => "none".to_owned(),
        "mapmetadata_startpos" => match startbox::decode_start_pos(&raw) {
            Ok(held) => {
                let layouts: Vec<String> = held
                    .team
                    .iter()
                    .map(|layout| format!("{}\u{d7}{}", layout.team_count, layout.players_per_team))
                    .collect();
                let mut text = plural(held.positions.len(), "position", "positions");
                if !layouts.is_empty() {
                    text = format!("{text} \u{b7} {}", layouts.join(", "));
                }
                text
            }
            Err(_) => "unreadable".to_owned(),
        },
        _ => String::new(),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
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
    fn the_map_options_are_described_in_words() {
        let set = startbox::encode_set(&BTreeMap::from([(2, arrangement(2)), (4, arrangement(4))]))
            .unwrap();
        assert_eq!(
            describe_map_option("mapmetadata_startboxes_set".into(), set),
            "map's own \u{b7} 2, 4 teams"
        );
        let over = startbox::encode_override(&arrangement(3)).unwrap();
        assert_eq!(
            describe_map_option("mapmetadata_startbox_override".into(), over),
            "custom \u{b7} 3 boxes"
        );
        let one = startbox::encode_override(&arrangement(1)).unwrap();
        assert_eq!(
            describe_map_option("mapmetadata_startbox_override".into(), one),
            "custom \u{b7} 1 box"
        );
        for key in [
            "mapmetadata_startboxes_set",
            "mapmetadata_startbox_override",
            "mapmetadata_startpos",
        ] {
            assert_eq!(describe_map_option(key.into(), "0".into()), "none");
            assert_eq!(describe_map_option(key.into(), String::new()), "none");
            assert_eq!(
                describe_map_option(key.into(), "nonsense".into()),
                "unreadable"
            );
        }
        assert_eq!(describe_map_option("startmetal".into(), "1000".into()), "");
    }

    #[test]
    fn start_positions_are_counted_with_their_layouts() {
        let json = serde_json::json!({
            "positions": { "a": {"x": 1, "y": 2}, "b": {"x": 3, "y": 4}, "c": {"x": 5, "y": 6} },
            "team": [
                { "playersPerTeam": 8, "teamCount": 2, "sides": [] },
                { "playersPerTeam": 4, "teamCount": 4, "sides": [] }
            ]
        });
        let raw = startbox::encode_json(&json).unwrap();
        assert_eq!(
            describe_map_option("mapmetadata_startpos".into(), raw),
            "3 positions \u{b7} 2\u{d7}8, 4\u{d7}4"
        );
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
    fn the_editor_starts_from_the_anchors_the_game_will_use() {
        let mut curved = arrangement(2);
        curved.startboxes[0].poly[0].strength = Some(0.5);
        let raw = startbox::encode_override(&curved).unwrap();
        let view = arrangement_from_tags(&tags(&[(OVERRIDE, &raw)]), 2).expect("arrangement");
        assert_eq!(view.source, startbox::Source::Override);
        // Anchors, not drawn corners: two points and the curvature survive.
        assert_eq!(view.arrangement, curved);
        assert!(arrangement_from_tags(&tags(&[]), 2).is_none());
    }

    #[test]
    fn what_is_drawn_goes_out_as_the_game_reads_it_back() {
        let drawn = arrangement(3);
        let encoded = encode_boxes(drawn.clone()).expect("encodes");
        assert_eq!(startbox::decode_override(&encoded.value).unwrap(), drawn);
        // `!bSet mapmetadata_startbox_override ` is 36 of teiserver's 1025.
        assert_eq!(encoded.limit, 989);
    }

    #[test]
    fn a_shape_the_game_could_not_read_is_refused_before_it_costs_a_vote() {
        let off_map = startbox::Arrangement {
            startboxes: vec![startbox::Box {
                poly: vec![
                    startbox::Point {
                        x: -1.0,
                        y: 0.0,
                        strength: None,
                    },
                    startbox::Point {
                        x: 10.0,
                        y: 10.0,
                        strength: None,
                    },
                ],
            }],
        };
        assert!(encode_boxes(off_map).is_err());
        let one_point = startbox::Arrangement {
            startboxes: vec![startbox::Box {
                poly: vec![startbox::Point {
                    x: 1.0,
                    y: 1.0,
                    strength: None,
                }],
            }],
        };
        assert!(encode_boxes(one_point).is_err());
        assert!(encode_boxes(startbox::Arrangement { startboxes: vec![] }).is_err());
        let mut too_curved = arrangement(1);
        too_curved.startboxes[0].poly[0].strength = Some(1.5);
        assert!(encode_boxes(too_curved).is_err());
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
    fn one_blob_can_be_read_back_for_a_diff() {
        let raw = startbox::encode_override(&arrangement(3)).unwrap();
        let polys = decode_boxes(raw, 3).expect("boxes");
        assert_eq!(polys.len(), 3);

        // A set needs the team count, the same way the game asks for one.
        let mut set = BTreeMap::new();
        set.insert(2, arrangement(2));
        let raw = startbox::encode_set(&set).unwrap();
        assert_eq!(decode_boxes(raw, 2).map(|p| p.len()), Some(2));
    }

    #[test]
    fn an_empty_or_unreadable_blob_decodes_to_nothing() {
        assert!(decode_boxes(String::new(), 2).is_none());
        assert!(decode_boxes("0".into(), 2).is_none());
        assert!(decode_boxes("not-a-blob".into(), 2).is_none());
    }

    #[test]
    fn nonsense_in_a_modoption_is_ignored_rather_than_fatal() {
        // A room can carry anything; a bad blob must not blank the minimap.
        let view = from_tags(&tags(&[(OVERRIDE, "not-base64!!"), (SET, "also-not")]), 2);
        assert!(view.is_none());
    }
}
