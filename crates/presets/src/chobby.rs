//! Reading and writing Chobby's `optionsPresets.json`.
//!
//! It is a single object keyed by preset name, each holding up to five
//! sections. Values in it are whatever Lua's JSON encoder produced at the
//! moment it was saved, so the same modoption can be `0`, `"0"` or `false` in
//! different presets in the same file — this normalises on the way in.
//!
//! On the way out, one distinction has to be put back. Chobby tests
//! `presetMPBattleSettings["locked"]` for truth
//! (`gui_optionpresets_panel.lua:161`), and in Lua the string `"false"` is
//! true. Writing that key as a string would lock every room a preset touched.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::model::{Preset, Stamp, StartBox};

/// Keys Chobby reads as booleans rather than as text.
const BOOLEAN_BATTLE_KEYS: [&str; 1] = ["locked"];

/// One value as it goes on the wire.
///
/// SPADS takes `!bSet <key> <value>` as text, and BAR's modoptions spell their
/// booleans `1` and `0`, so that is what a JSON `true` becomes.
fn wire(value: &Value) -> Option<String> {
    Some(match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "1".into(),
        Value::Bool(false) => "0".into(),
        Value::Number(number) => number.to_string(),
        // A null, array or object is not a setting; dropping it is better than
        // sending the word "null" to a host.
        _ => return None,
    })
}

fn strings(section: Option<&Value>) -> BTreeMap<String, String> {
    section
        .and_then(Value::as_object)
        .map(|table| {
            table
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), wire(value)?)))
                .collect()
        })
        .unwrap_or_default()
}

/// Start boxes, from either shape Lua's encoder produces.
///
/// A Lua table with keys 1..n encodes as a JSON array; one with a gap — a
/// preset that names ally team 2 and no other — encodes as an object with
/// string keys. Both appear in real files, so both are read. Chobby's own
/// apply path assumes the array and silently ignores the object form.
fn start_boxes(section: Option<&Value>) -> BTreeMap<u8, StartBox> {
    let mut boxes = BTreeMap::new();
    let mut put = |ally: u8, value: &Value| {
        let field = |name: &str| -> Option<u16> {
            let held = value.get(name)?;
            held.as_u64()
                .or_else(|| held.as_str()?.trim().parse().ok())
                .map(|number| number.min(u64::from(u16::MAX)) as u16)
        };
        if let (Some(left), Some(top), Some(right), Some(bottom)) =
            (field("left"), field("top"), field("right"), field("bottom"))
        {
            boxes.insert(
                ally,
                StartBox {
                    left,
                    top,
                    right,
                    bottom,
                },
            );
        }
    };

    match section {
        Some(Value::Array(items)) => {
            for (index, value) in items.iter().enumerate() {
                // Chobby writes them 1-based and applies them 0-based
                // (`AddStartRect(index - 1, …)`); ours are the ally team.
                put(index as u8, value);
            }
        }
        Some(Value::Object(table)) => {
            for (key, value) in table {
                if let Ok(ally) = key.trim().parse::<u8>() {
                    put(ally.saturating_sub(1), value);
                }
            }
        }
        _ => {}
    }
    boxes
}

/// Everything in one of Chobby's files, as presets of ours.
///
/// Timestamps it cannot know are all set to `now`: the file records nothing
/// about when anything happened, and inventing older dates would be a lie the
/// sort order would then act on.
pub fn read(text: &str, now: Stamp) -> Result<Vec<Preset>, serde_json::Error> {
    let file: Value = serde_json::from_str(text)?;
    let Some(table) = file.as_object() else {
        return Ok(Vec::new());
    };

    Ok(table
        .iter()
        .map(|(name, entry)| Preset {
            name: name.clone(),
            map: entry
                .get("Map")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|map| !map.is_empty()),
            modoptions: strings(entry.get("Modoptions")),
            battle: strings(entry.get("Multiplayer Battle Settings")),
            start_boxes: start_boxes(entry.get("Start Boxes")),
            bots: entry
                .get("Bots")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            created: now,
            updated: now,
            last_used: None,
        })
        .collect())
}

/// One preset in Chobby's shape.
fn entry(preset: &Preset) -> Value {
    let mut out = Map::new();
    if let Some(map) = &preset.map {
        out.insert("Map".into(), Value::String(map.clone()));
    }
    if !preset.modoptions.is_empty() {
        out.insert(
            "Modoptions".into(),
            Value::Object(
                preset
                    .modoptions
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect(),
            ),
        );
    }
    if !preset.battle.is_empty() {
        out.insert(
            "Multiplayer Battle Settings".into(),
            Value::Object(
                preset
                    .battle
                    .iter()
                    .map(|(key, value)| {
                        let held = if BOOLEAN_BATTLE_KEYS.contains(&key.as_str()) {
                            Value::Bool(matches!(value.as_str(), "1" | "true" | "locked"))
                        } else {
                            Value::String(value.clone())
                        };
                        (key.clone(), held)
                    })
                    .collect(),
            ),
        );
    }
    if !preset.start_boxes.is_empty() {
        // Written the way Chobby's apply path reads them: an array it walks
        // with `ipairs`, one-based, so ally team 0 is the first entry. A gap
        // would end that walk early, so every team up to the highest is
        // present and the ones we do not have are the whole map.
        let highest = preset.start_boxes.keys().copied().max().unwrap_or(0);
        let whole = StartBox {
            left: 0,
            top: 0,
            right: 200,
            bottom: 200,
        };
        let boxes: Vec<Value> = (0..=highest)
            .map(|ally| {
                let held = preset.start_boxes.get(&ally).unwrap_or(&whole);
                serde_json::json!({
                    "left": held.left.to_string(),
                    "top": held.top.to_string(),
                    "right": held.right.to_string(),
                    "bottom": held.bottom.to_string(),
                })
            })
            .collect();
        out.insert("Start Boxes".into(), Value::Array(boxes));
    }
    if !preset.bots.is_empty() {
        out.insert("Bots".into(), Value::Object(preset.bots.clone()));
    }
    Value::Object(out)
}

/// Merges presets into an existing file's text, keeping everything already in
/// it that we are not replacing.
///
/// Never a wholesale rewrite: that file is Chobby's, it holds presets this
/// window has never seen, and a preset is somebody's evening of tuning. A name
/// we also have wins, because that is what exporting means; every other entry
/// is left exactly as it was found, byte-identical for anything untouched.
pub fn merge_into(existing: &str, presets: &[Preset]) -> Result<String, serde_json::Error> {
    let mut file: Value = if existing.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(existing)?
    };
    let table = match file.as_object_mut() {
        Some(table) => table,
        None => {
            file = Value::Object(Map::new());
            file.as_object_mut().expect("just made an object")
        }
    };

    for preset in presets {
        table.insert(preset.name.clone(), entry(preset));
    }
    serde_json::to_string_pretty(&file)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "nuttyb timer coms": {
        "Map": "Full Metal Plate 1.7",
        "Modoptions": {"scav_spawncountmult": 1, "allowuserwidgets": "1", "disable_fogofwar": false},
        "Multiplayer Battle Settings": {"teamSize": 12, "locked": true, "preset": "coop"},
        "Start Boxes": {"2": {"top": "82", "right": "115", "left": "80", "bottom": "117"}},
        "Bots": {}
      }
    }"#;

    #[test]
    fn a_value_of_any_json_type_becomes_what_goes_on_the_wire() {
        let presets = read(SAMPLE, 100).unwrap();
        let one = &presets[0];
        // The same file spells the same kind of setting three ways.
        assert_eq!(one.modoptions["scav_spawncountmult"], "1");
        assert_eq!(one.modoptions["allowuserwidgets"], "1");
        assert_eq!(one.modoptions["disable_fogofwar"], "0");
        assert_eq!(one.battle["teamSize"], "12");
        assert_eq!(one.battle["locked"], "1");
    }

    #[test]
    fn start_boxes_are_read_from_the_shape_lua_happened_to_produce() {
        let presets = read(SAMPLE, 100).unwrap();
        // An object keyed "2" is Chobby's one-based ally team 2, ours is 1.
        let held = presets[0].start_boxes.get(&1).expect("ally team 1");
        assert_eq!(
            *held,
            StartBox {
                left: 80,
                top: 82,
                right: 115,
                bottom: 117
            }
        );

        let array = r#"{"p": {"Start Boxes": [{"left":0,"top":0,"right":50,"bottom":200}]}}"#;
        let presets = read(array, 100).unwrap();
        assert!(presets[0].start_boxes.contains_key(&0));
    }

    #[test]
    fn timestamps_it_cannot_know_are_all_now() {
        let presets = read(SAMPLE, 4242).unwrap();
        assert_eq!(presets[0].created, 4242);
        assert_eq!(presets[0].updated, 4242);
        assert_eq!(presets[0].last_used, None);
    }

    #[test]
    fn exporting_keeps_every_preset_it_was_not_asked_about() {
        let mut ours = read(SAMPLE, 100).unwrap();
        ours[0].map = Some("Supreme Isthmus v2.1".into());
        let existing = r#"{"someone else's": {"Map": "Comet Catcher Remake 1.8"}}"#;

        let merged = merge_into(existing, &ours).unwrap();
        let back: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            back["someone else's"]["Map"],
            Value::String("Comet Catcher Remake 1.8".into())
        );
        assert_eq!(
            back["nuttyb timer coms"]["Map"],
            Value::String("Supreme Isthmus v2.1".into())
        );
    }

    #[test]
    fn locked_goes_back_as_a_boolean_because_lua_reads_it_as_one() {
        let presets = read(SAMPLE, 100).unwrap();
        let merged = merge_into("{}", &presets).unwrap();
        let back: Value = serde_json::from_str(&merged).unwrap();
        let battle = &back["nuttyb timer coms"]["Multiplayer Battle Settings"];
        // The string "false" is true in Lua, which would lock every room this
        // preset touched.
        assert_eq!(battle["locked"], Value::Bool(true));
        assert_eq!(battle["teamSize"], Value::String("12".into()));
    }

    #[test]
    fn start_boxes_go_back_as_the_array_chobby_walks() {
        let presets = read(SAMPLE, 100).unwrap();
        let merged = merge_into("{}", &presets).unwrap();
        let back: Value = serde_json::from_str(&merged).unwrap();
        let boxes = back["nuttyb timer coms"]["Start Boxes"]
            .as_array()
            .expect("an array, which is what ipairs can walk");
        // Ally 1 was the only one named, so ally 0 is filled in whole-map
        // rather than left as a hole that would end the walk at nothing.
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0]["right"], Value::String("200".into()));
        assert_eq!(boxes[1]["left"], Value::String("80".into()));
    }

    #[test]
    fn a_file_that_is_not_an_object_yields_nothing_rather_than_failing() {
        assert!(read("[]", 1).unwrap().is_empty());
        assert!(read("null", 1).unwrap().is_empty());
    }
}
