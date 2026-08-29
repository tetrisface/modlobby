//! The alphabet SPADS accepts (`[A-Za-z0-9\-\_]*`, `battlePresets.conf:11-30`)
//! is base64url without padding — `=` is rejected outright.
//!
//! The two kinds decode differently in game, and it matters:
//! `tweakdefs` goes through `string.base64Decode` untouched
//! (`unitdefs_post.lua:266`), while `tweakunits` first turns every `_` into `=`
//! (`common/springUtilities/tableFunctions.lua:8`) — a workaround for the
//! missing padding that silently corrupts any payload whose encoding really
//! contains `_` (alphabet index 63). [`encode`] never produces one.

use base64::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Error, Kind};

/// Something worth telling the user about a decoded payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum Diagnostic {
    /// A `tweakunits` blob contains `_`, which the game reads as `=`: what it
    /// loads is not what is stored here.
    UnderscoreCorruption { count: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Decodes as the game would, tolerating padding and the standard alphabet.
pub fn decode(blob: &str, kind: Kind) -> Result<Decoded, Error> {
    let cleaned: String = blob.chars().filter(|c| !c.is_whitespace()).collect();
    let mut diagnostics = Vec::new();
    let for_game = match kind {
        Kind::Defs => cleaned.clone(),
        Kind::Units => {
            // What `CustomKeyToUsefulTable` does before decoding.
            let count = cleaned.matches('_').count();
            if count > 0 {
                diagnostics.push(Diagnostic::UnderscoreCorruption { count });
            }
            cleaned.replace('_', "=")
        }
    };
    let normalised: String = for_game
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect();
    let bytes = BASE64_URL_SAFE_NO_PAD
        .decode(&normalised)
        .map_err(|err| Error::Base64(err.to_string()))?;
    let text = String::from_utf8(bytes).map_err(|_| Error::Utf8)?;
    Ok(Decoded { text, diagnostics })
}

/// Encodes unpadded base64url. For [`Kind::Units`] the result is guaranteed
/// free of `_`, or the call fails with what to change.
pub fn encode(text: &str, kind: Kind) -> Result<String, Error> {
    let blob = BASE64_URL_SAFE_NO_PAD.encode(text.as_bytes());
    if kind == Kind::Defs || !blob.contains('_') {
        return Ok(blob);
    }
    // Only `?` and DEL survive minification outside a string literal, and only
    // in the leading name comment (`lua::minify` escapes them everywhere else).
    Err(Error::Underscore(
        "a tweakunits payload cannot contain '?' outside a string; remove it from the name comment"
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_what_other_clients_send() {
        let text = "-- name\nreturn {armcom={metalcost=3000}}";
        let blob = encode(text, Kind::Defs).unwrap();
        assert!(!blob.contains('='), "SPADS rejects padding");
        assert_eq!(decode(&blob, Kind::Defs).unwrap().text, text);

        // Padded standard-alphabet input still decodes.
        let padded = BASE64_STANDARD.encode(text.as_bytes());
        assert_eq!(decode(&padded, Kind::Defs).unwrap().text, text);
        // As does a blob split across lines.
        let split = format!("{}\n{}", &blob[..8], &blob[8..]);
        assert_eq!(decode(&split, Kind::Defs).unwrap().text, text);
    }

    /// `?` at a byte offset ≡ 2 (mod 3) is the only ASCII source of `_`.
    #[test]
    fn underscore_is_reported_for_units_and_refused_when_encoding() {
        let text = "ab?";
        let blob = BASE64_URL_SAFE_NO_PAD.encode(text.as_bytes());
        assert!(blob.contains('_'), "fixture must produce the hazard");

        let decoded = decode(&blob, Kind::Units).unwrap();
        assert_eq!(
            decoded.diagnostics,
            [Diagnostic::UnderscoreCorruption { count: 1 }]
        );
        assert_ne!(decoded.text, text, "the game reads something else");
        assert!(decode(&blob, Kind::Defs).unwrap().diagnostics.is_empty());

        assert!(matches!(
            encode(text, Kind::Units),
            Err(Error::Underscore(_))
        ));
        assert!(encode(text, Kind::Defs).is_ok());
    }

    /// Every ASCII payload without `?`/DEL encodes cleanly for tweakunits.
    #[test]
    fn ascii_without_question_marks_never_yields_underscore() {
        let alphabet: Vec<char> = (0x20u8..0x7f)
            .map(char::from)
            .filter(|c| *c != '?')
            .collect();
        for start in 0..alphabet.len() {
            let text: String = alphabet
                .iter()
                .cycle()
                .skip(start)
                .take(alphabet.len())
                .collect();
            let blob = encode(&text, Kind::Units).expect("no hazard");
            assert!(!blob.contains('_'), "{text}");
            assert_eq!(decode(&blob, Kind::Units).unwrap().text, text);
        }
    }
}
