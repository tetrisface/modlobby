//! Formatting with StyLua and a token-level minifier.
//!
//! Minifying matters because the whole `!bSet` line is capped at 16 385
//! characters server-side, and the base64 of the Lua is four thirds of it.
//! The first `--` line is kept: BAR has no field for a tweak's name, so that
//! comment *is* the name (`gui_modoptions_panel.lua:1068-1086`).

use std::path::Path;

use full_moon::LuaVersion;
use full_moon::tokenizer::{Lexer, LexerResult, Token, TokenKind, TokenType};

use crate::{Error, Kind};

/// StyLua's configuration, as a `stylua.toml` provides it.
pub type Config = stylua_lib::Config;

/// StyLua's defaults, except that an indent is two columns wide.
///
/// StyLua indents with tabs; `indent_width` only guides where it wraps a long
/// line. The editor and the room draw a tab two columns wide, so this is what
/// makes the wrapping it chooses match what is on the screen.
pub fn default_config() -> Config {
    Config {
        indent_width: 2,
        ..Config::default()
    }
}

/// Reads the user's `stylua.toml`; [`default_config`] when there is none.
pub fn load_config(path: Option<&Path>) -> Result<Config, Error> {
    let Some(path) = path else {
        return Ok(default_config());
    };
    let text = std::fs::read_to_string(path).map_err(|err| Error::Lua(err.to_string()))?;
    toml::from_str(&text).map_err(|err| Error::Lua(err.to_string()))
}

/// Pretty-prints the payload. `Kind::Units` is a bare table constructor, so it
/// is formatted as the `return <text>` the game evaluates and unwrapped again.
pub fn format(source: &str, kind: Kind, config: &Config) -> Result<String, Error> {
    let wrapped = match kind {
        Kind::Defs => source.to_owned(),
        Kind::Units => format!("return {source}"),
    };
    let formatted = stylua_lib::format_code(
        &wrapped,
        *config,
        None,
        stylua_lib::OutputVerification::None,
    )
    .map_err(|err| Error::Lua(err.to_string()))?;
    Ok(match kind {
        Kind::Defs => formatted,
        Kind::Units => unwrap_return(&formatted),
    })
}

fn unwrap_return(formatted: &str) -> String {
    let mut lines: Vec<&str> = formatted.lines().collect();
    for line in &mut lines {
        if let Some(rest) = line.strip_prefix("return ") {
            *line = rest;
            break;
        }
    }
    lines.join("\n")
}

/// Drops whitespace and comments — except the header block — and keeps exactly
/// the spaces that stop two tokens from fusing. For `Kind::Units`, string
/// literals are also escaped so the blob can never contain `_`
/// (see [`crate::base64url`]).
///
/// The whole leading run of comments is kept, not just the first line. What
/// people actually publish looks like
///
/// ```text
/// --NuttyB v1.52 Cortex Com
/// -- Authors: ChrispyNut, BackBash
/// -- docs.google.com/spreadsheets/d/1QSVsuAAM…
/// ```
///
/// — a name, who wrote it, and where it is documented. Keeping only the first
/// line meant that editing somebody else's tweak here quietly stripped their
/// credit and the link to their notes.
pub fn minify(source: &str, kind: Kind) -> Result<String, Error> {
    const NEWLINE: char = 0x0A as char;
    let tokens = lex(source)?;
    let mut out = String::with_capacity(source.len());

    let mut rest = tokens.as_slice();
    while let Some(first) = rest.first() {
        match first.token_type() {
            TokenType::SingleLineComment { comment } => {
                out.push_str("--");
                out.push_str(comment.trim_end());
                out.push(NEWLINE);
            }
            // The newlines between them, and any indent before the first.
            TokenType::Whitespace { .. } => {}
            _ => break,
        }
        rest = &rest[1..];
    }

    let mut previous: Option<String> = None;
    for token in rest {
        if matches!(
            token.token_kind(),
            TokenKind::Whitespace
                | TokenKind::SingleLineComment
                | TokenKind::MultiLineComment
                | TokenKind::Eof
        ) {
            continue;
        }
        let text = render(token, kind);
        if let Some(previous) = &previous
            && needs_space(previous, &text)
        {
            out.push(' ');
        }
        out.push_str(&text);
        previous = Some(text);
    }
    Ok(out)
}

/// Every token in source order, trivia included. `collect` is the only entry
/// point that yields the whole stream: `Lexer::new` pre-loads two tokens that
/// `process_next` would skip past.
fn lex(source: &str) -> Result<Vec<Token>, Error> {
    match Lexer::new(source, LuaVersion::lua51()).collect() {
        LexerResult::Ok(tokens) => Ok(tokens),
        LexerResult::Recovered(_, errors) | LexerResult::Fatal(errors) => {
            let first = errors
                .first()
                .map_or_else(|| "unreadable".to_owned(), |e| e.to_string());
            Err(Error::Lua(first))
        }
    }
}

/// A token as source text; for `tweakunits`, string literals get escapes that
/// keep `?` and non-ASCII bytes out of the encoded blob.
fn render(token: &Token, kind: Kind) -> String {
    let TokenType::StringLiteral {
        literal,
        multi_line_depth,
        quote_type,
    } = token.token_type()
    else {
        return token.to_string();
    };
    if kind == Kind::Defs || *multi_line_depth > 0 {
        return token.to_string();
    }
    let mut escaped = String::with_capacity(literal.len());
    for ch in literal.chars() {
        match ch {
            // 0x3F and 0x7F are the only ASCII bytes that can encode to `_`.
            '?' => escaped.push_str("\\063"),
            '\u{7f}' => escaped.push_str("\\127"),
            ch if ch.is_ascii() => escaped.push(ch),
            ch => {
                let mut buffer = [0u8; 4];
                for byte in ch.encode_utf8(&mut buffer).as_bytes() {
                    escaped.push_str(&format!("\\{byte:03}"));
                }
            }
        }
    }
    format!("{quote_type}{escaped}{quote_type}")
}

/// Whether `a` and `b` written together would lex as something else.
fn needs_space(a: &str, b: &str) -> bool {
    let (Some(last), Some(first)) = (a.chars().last(), b.chars().next()) else {
        return false;
    };
    if word_like(last) && word_like(first) {
        return true;
    }
    // `--`, `==`, `..`, `::`, `<=`, `>=`, `~=`, `//`
    const FUSING: &str = "-=<>~/.:";
    if FUSING.contains(last) && FUSING.contains(first) {
        return true;
    }
    // A number swallowing a following `.`, and `[[`/`[=[` long brackets.
    (last.is_ascii_digit() && first == '.') || (last == '[' && (first == '[' || first == '='))
}

fn word_like(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The token stream, ignoring trivia — what must survive minification.
    fn significant(source: &str) -> Vec<String> {
        lex(source)
            .unwrap()
            .iter()
            .filter(|t| {
                !matches!(
                    t.token_kind(),
                    TokenKind::Whitespace
                        | TokenKind::SingleLineComment
                        | TokenKind::MultiLineComment
                        | TokenKind::Eof
                )
            })
            .map(|t| t.to_string())
            .collect()
    }

    #[test]
    fn minify_keeps_the_header_and_the_meaning() {
        // Three header lines, which is what a published tweak carries: a
        // name, its authors, and a link to where it is documented.
        let source = "--NuttyB v1.52 Cortex Com
-- Authors: ChrispyNut, BackBash
-- docs.example/1QSV
local a = 1
for i = 1, 10 do
  a = a - -i -- inline
end
return a .. 'x'
";
        let minified = minify(source, Kind::Defs).unwrap();
        assert!(minified.starts_with(
            "--NuttyB v1.52 Cortex Com
-- Authors: ChrispyNut, BackBash
-- docs.example/1QSV
"
        ));
        assert!(
            minified.contains("ChrispyNut"),
            "the header is somebody's credit, not noise"
        );
        assert!(!minified.contains("inline"));
        assert!(minified.contains("a- -i"), "{minified}");
        assert_eq!(significant(source), significant(&minified));
        assert!(minified.len() < source.len());
    }

    #[test]
    fn default_config_indents_with_two_column_tabs() {
        let config = default_config();
        assert_eq!(config.indent_type, stylua_lib::IndentType::Tabs);
        assert_eq!(config.indent_width, 2);
        let formatted = format(
            "local function f()
return 1
end",
            Kind::Defs,
            &config,
        )
        .unwrap();
        assert!(formatted.contains("	return 1"), "{formatted}");
    }

    #[test]
    fn minify_is_idempotent_and_formats_back() {
        let source = "local x = { a = 1, b = 2 }\nreturn x\n";
        let once = minify(source, Kind::Defs).unwrap();
        assert_eq!(minify(&once, Kind::Defs).unwrap(), once);
        let config = Config::default();
        assert_eq!(
            format(&once, Kind::Defs, &config).unwrap(),
            format(source, Kind::Defs, &config).unwrap()
        );
    }

    #[test]
    fn units_escape_what_would_corrupt_the_payload() {
        let source = "{ armcom = { name = 'why?', tip = 'caf\u{e9}' } }";
        let minified = minify(source, Kind::Units).unwrap();
        assert!(minified.contains("\\063"), "{minified}");
        assert!(!minified.contains('?'));
        assert!(minified.contains("\\195\\169"), "{minified}");
        let blob = crate::base64url::encode(&minified, Kind::Units).unwrap();
        assert!(!blob.contains('_'));
        // The escapes are Lua, so the game reads the same table back.
        let decoded = crate::base64url::decode(&blob, Kind::Units).unwrap();
        assert!(full_moon::parse(&format!("return {}", decoded.text)).is_ok());
    }

    #[test]
    fn units_round_trip_through_format() {
        let source = "{armcom={metalcost=3000,weapondefs={gun={damage={default=50}}}}}";
        let formatted = format(source, Kind::Units, &Config::default()).unwrap();
        assert!(formatted.starts_with('{'), "{formatted}");
        assert!(formatted.contains("metalcost = 3000"));
        assert_eq!(significant(source), significant(&formatted));
    }

    #[test]
    fn broken_lua_is_reported_not_swallowed() {
        assert!(matches!(
            minify("local x = 'unterminated", Kind::Defs),
            Err(Error::Lua(_))
        ));
        assert!(matches!(
            format("if then", Kind::Defs, &Config::default()),
            Err(Error::Lua(_))
        ));
    }
}
