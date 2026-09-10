//! The start-box override as an editor document.
//!
//! `mapmetadata_startbox_override` is JSON, zlib-compressed inside the
//! base64url rather than plain, and the `startbox` crate owns that wire
//! format. What is here is the editing: pretty for the screen, minified for
//! the wire, and a shape check against what the game's reader would skip.
//! Everything the `Boxes` kind does differently from the two Lua kinds is in
//! this module.

use crate::Error;
use crate::check::{Check, Problem};

/// The JSON inside a stored value, exactly as it was sent.
pub fn decode(blob: &str) -> Result<String, Error> {
    Ok(startbox::decode_text(blob)?)
}

/// Minified JSON into a value the modoption can carry.
pub fn encode(minified: &str) -> Result<String, Error> {
    Ok(startbox::encode_text(minified)?)
}

/// One key per line, indented with tabs like the Lua the editor draws beside it.
pub fn format(text: &str) -> Result<String, Error> {
    let value = parse(text)?;
    let mut out = Vec::new();
    let mut writer = serde_json::Serializer::with_formatter(
        &mut out,
        serde_json::ser::PrettyFormatter::with_indent(b"\t"),
    );
    serde::Serialize::serialize(&value, &mut writer).map_err(|err| Error::Json(err.to_string()))?;
    String::from_utf8(out).map_err(|_| Error::Utf8)
}

/// The JSON with its whitespace gone, once it is an arrangement the game
/// could read. Keys the reader ignores are kept as typed: the gauge is what
/// says they cost bytes.
pub fn minify(text: &str) -> Result<String, Error> {
    let value = parse(text)?;
    let arrangement: startbox::Arrangement =
        serde_json::from_value(value.clone()).map_err(|err| Error::Json(err.to_string()))?;
    startbox::check(&arrangement)?;
    serde_json::to_string(&value).map_err(|err| Error::Json(err.to_string()))
}

/// Where the JSON stops making sense. Nothing to outline: an arrangement is
/// a handful of boxes, not a hundred units.
pub fn check(text: &str) -> Check {
    let problems = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(_) => Vec::new(),
        Err(err) => {
            // serde reports column 0 at the end of the input; a marker needs a column.
            let line = u32::try_from(err.line()).unwrap_or(u32::MAX).max(1);
            let column = u32::try_from(err.column()).unwrap_or(u32::MAX).max(1);
            vec![Problem {
                line,
                column,
                end_line: line,
                end_column: column + 1,
                message: err.to_string(),
            }]
        }
    };
    Check {
        problems,
        outline: Vec::new(),
    }
}

fn parse(text: &str) -> Result<serde_json::Value, Error> {
    serde_json::from_str(text).map_err(|err| Error::Json(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_BOX: &str = r#"{"startboxes":[{"poly":[{"x":0,"y":0},{"x":50,"y":200}]}]}"#;

    #[test]
    fn what_is_typed_goes_out_minified_and_comes_back_pretty() {
        let typed = "{ \"startboxes\": [ { \"poly\": [ {\"x\": 0, \"y\": 0}, {\"x\": 50, \"y\": 200} ] } ] }";
        let minified = minify(typed).unwrap();
        assert_eq!(minified, ONE_BOX);
        let blob = encode(&minified).unwrap();
        assert_eq!(decode(&blob).unwrap(), ONE_BOX);
        // And the game reads the same arrangement out of it.
        assert_eq!(
            startbox::decode_override(&blob).unwrap().startboxes.len(),
            1
        );

        let pretty = format(ONE_BOX).unwrap();
        assert!(
            pretty.starts_with("{\n\t\"startboxes\": [\n\t\t{\n"),
            "{pretty}"
        );
        assert_eq!(
            minify(&pretty).unwrap(),
            minified,
            "formatting is display only"
        );
    }

    #[test]
    fn a_shape_the_game_would_skip_is_refused_and_says_why() {
        let off_map = r#"{"startboxes":[{"poly":[{"x":-1,"y":0},{"x":50,"y":200}]}]}"#;
        assert!(matches!(minify(off_map), Err(Error::Boxes(_))));
        assert_eq!(
            minify(off_map).unwrap_err().to_string(),
            "start boxes: a point is on the map: 0-200 on both axes"
        );
        // Well-formed JSON that is not an arrangement at all.
        assert!(matches!(minify(r#"{"boxes":[]}"#), Err(Error::Json(_))));
        // Keys the game ignores ride along; they are the user's to drop.
        let extra = r#"{"maxPlayersPerStartbox":8,"startboxes":[{"poly":[{"x":0,"y":0},{"x":50,"y":200}]}]}"#;
        assert_eq!(minify(extra).unwrap(), extra);
    }

    #[test]
    fn a_syntax_error_is_placed_on_its_line() {
        let out = check("{\n\t\"startboxes\": [\n\t\t{ \"poly\": }\n\t]\n}");
        let problem = out.problems.first().expect("a problem");
        assert_eq!(problem.line, 3, "{problem:?}");
        assert!(problem.column > 1);
        assert!(out.outline.is_empty());
        assert!(check(ONE_BOX).problems.is_empty());
        // The end of the input is column 0 to serde and column 1 to an editor.
        let cut = check("{\"startboxes\":");
        assert_eq!(cut.problems[0].line, 1);
        assert!(cut.problems[0].column >= 1);
    }
}
