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

        match section.as_str() {
            "game" => {
                script.game.insert(key, value);
            }
            "modoptions" => {
                script.modoptions.insert(key, value);
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
            _ => {}
        }
    }
    script
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

    #[test]
    fn nothing_recognisable_yields_nothing_rather_than_failing() {
        let script = parse("this is not a start script at all");
        assert!(script.game.is_empty());
        assert!(script.modoptions.is_empty());
    }
}
