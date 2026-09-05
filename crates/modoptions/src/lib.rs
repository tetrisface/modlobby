//! BAR's modoption schema, read out of the game's own `modoptions.lua`.
//!
//! Chobby reads this table from the game archive with the engine's Lua VM
//! (`gui_modoptions_panel.lua:1215-1255`). We have no VM, so this parses the
//! Lua itself, over bytes `content::Library::game_file` reads out of the
//! installed game. The table therefore matches the version the room is running,
//! and none of BAR's own writing is redistributed.
//!
//! Only the literal `local options = { … }` table is read. The two `for i = 1,
//! 9` loops at the end of the file generate the numbered tweak slots, which we
//! already model exactly in the `tweaks` crate — reading them twice would let
//! the two drift apart.

use full_moon::ast::{BinOp, Expression, Field, Stmt, TableConstructor, UnOp};
use full_moon::tokenizer::{StringLiteralQuoteType, Symbol, TokenType};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("modoptions.lua does not parse as Lua: {0}")]
    Lua(String),
    #[error("modoptions.lua has no `local options = {{ … }}` table")]
    NoTable,
}

/// What a lobby is meant to render for one entry.
///
/// `Section` declares a tab; `Subheader` and `Separator` are layout marks
/// inside one. Anything BAR adds later arrives as `Other` rather than failing
/// the parse, so a new control type costs us a plain row, not a broken app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
// `Kind` is already the tweak-slot kind in the shared bindings directory.
#[ts(export, rename = "OptionType")]
pub enum Kind {
    Bool,
    Number,
    String,
    List,
    Section,
    Subheader,
    Separator,
    Link,
    Other(String),
}

impl Kind {
    fn parse(text: &str) -> Self {
        match text {
            "bool" => Self::Bool,
            "number" => Self::Number,
            "string" => Self::String,
            "list" => Self::List,
            "section" => Self::Section,
            "subheader" => Self::Subheader,
            "separator" => Self::Separator,
            "link" => Self::Link,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// A default, in the one of three shapes Lua wrote it in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export, rename = "OptionValue")]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "OptionItem")]
pub struct Item {
    pub key: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub desc: String,
}

/// One row of BAR's table, in the order the file declares it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModOption {
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub desc: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    /// The tab this belongs to. Absent on `Kind::Section` itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub def: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// Tab order, descending. Only sections carry one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
    /// Declared `hidden = true`: Chobby draws neither the option nor the tab.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
}

fn is_false(hidden: &bool) -> bool {
    !*hidden
}

pub fn parse(lua: &str) -> Result<Vec<ModOption>, Error> {
    let ast = full_moon::parse(lua).map_err(|errors| {
        Error::Lua(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;

    let table = ast
        .nodes()
        .stmts()
        .find_map(options_table)
        .ok_or(Error::NoTable)?;

    Ok(table.fields().iter().filter_map(read_option).collect())
}

/// The `local options = { … }` statement, ignoring every other binding.
fn options_table(stmt: &Stmt) -> Option<&TableConstructor> {
    let Stmt::LocalAssignment(assignment) = stmt else {
        return None;
    };
    let named_options = assignment
        .names()
        .iter()
        .any(|name| identifier(name.token().token_type()) == Some("options"));
    if !named_options {
        return None;
    }
    assignment.expressions().iter().find_map(table)
}

fn read_option(field: &Field) -> Option<ModOption> {
    let Field::NoKey(expression) = field else {
        return None;
    };
    let entry = table(expression)?;

    let mut option = ModOption {
        key: String::new(),
        name: String::new(),
        desc: String::new(),
        kind: Kind::Other(String::new()),
        section: None,
        def: None,
        min: None,
        max: None,
        step: None,
        weight: None,
        items: Vec::new(),
        hidden: false,
    };

    for field in entry.fields() {
        let Field::NameKey { key, value, .. } = field else {
            continue;
        };
        match identifier(key.token().token_type())? {
            "key" => option.key = string(value)?,
            "name" => option.name = string(value).unwrap_or_default(),
            "desc" => option.desc = string(value).unwrap_or_default(),
            "type" => option.kind = Kind::parse(&string(value)?),
            "section" => option.section = string(value),
            "def" => option.def = value_of(value),
            "min" => option.min = number(value),
            "max" => option.max = number(value),
            "step" => option.step = number(value),
            "weight" => option.weight = number(value),
            "hidden" => option.hidden = boolean(value).unwrap_or(false),
            "items" => option.items = table(value).map(read_items).unwrap_or_default(),
            _ => {}
        }
    }

    (!option.key.is_empty()).then_some(option)
}

fn read_items(items: &TableConstructor) -> Vec<Item> {
    items
        .fields()
        .iter()
        .filter_map(|field| {
            let Field::NoKey(expression) = field else {
                return None;
            };
            let entry = table(expression)?;

            let mut item = Item {
                key: String::new(),
                name: String::new(),
                desc: String::new(),
            };
            for field in entry.fields() {
                let Field::NameKey { key, value, .. } = field else {
                    continue;
                };
                match identifier(key.token().token_type()) {
                    Some("key") => item.key = string(value).unwrap_or_default(),
                    Some("name") => item.name = string(value).unwrap_or_default(),
                    Some("desc") => item.desc = string(value).unwrap_or_default(),
                    _ => {}
                }
            }
            (!item.key.is_empty()).then_some(item)
        })
        .collect()
}

fn identifier(token: &TokenType) -> Option<&str> {
    match token {
        TokenType::Identifier { identifier } => Some(identifier.as_str()),
        _ => None,
    }
}

fn table(expression: &Expression) -> Option<&TableConstructor> {
    match expression {
        Expression::TableConstructor(table) => Some(table),
        _ => None,
    }
}

/// A string expression: one literal, or literals joined with `..`. BAR writes
/// most descriptions and a few names that way, some of them to slip a colour
/// code in between.
fn string(expression: &Expression) -> Option<String> {
    match expression {
        Expression::String(token) => match token.token().token_type() {
            TokenType::StringLiteral {
                literal,
                quote_type: StringLiteralQuoteType::Brackets,
                ..
            } => Some(literal.as_str().to_owned()),
            TokenType::StringLiteral { literal, .. } => Some(unescape(literal.as_str())),
            _ => None,
        },
        Expression::BinaryOperator {
            lhs,
            binop: BinOp::TwoDots(_),
            rhs,
        } => Some(string(lhs)? + &string(rhs)?),
        Expression::Parentheses { expression, .. } => string(expression),
        _ => None,
    }
}

/// The byte Spring reads as "a colour follows": `\255` and three colour bytes
/// tint the rest of a Chobby text box. Never valid UTF-8, so unambiguous.
const COLOUR: u8 = 255;

/// A quoted literal with Lua's escapes decoded and Spring's inline colour
/// codes dropped, since they mean nothing outside a Chobby text box.
///
/// Lua 5.1's escapes are the C ones and `\ddd`; the `\x` and `\u{}` forms of
/// later Luas do not appear in the file.
fn unescape(literal: &str) -> String {
    let mut bytes = Vec::with_capacity(literal.len());
    let mut chars = literal.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\\' {
            bytes.extend(c.encode_utf8(&mut [0; 4]).as_bytes());
            continue;
        }
        match chars.next() {
            Some('n' | '\n') => bytes.push(b'\n'),
            Some('t') => bytes.push(b'\t'),
            Some('r') => bytes.push(b'\r'),
            Some('a') => bytes.push(0x07),
            Some('b') => bytes.push(0x08),
            Some('f') => bytes.push(0x0c),
            Some('v') => bytes.push(0x0b),
            Some(digit) if digit.is_ascii_digit() => {
                let mut text = String::from(digit);
                while text.len() < 3 && chars.peek().is_some_and(|next| next.is_ascii_digit()) {
                    text.extend(chars.next());
                }
                bytes.extend(text.parse::<u8>().ok());
            }
            Some(other) => bytes.extend(other.encode_utf8(&mut [0; 4]).as_bytes()),
            None => {}
        }
    }

    String::from_utf8_lossy(&without_colour(&bytes)).into_owned()
}

fn without_colour(bytes: &[u8]) -> Vec<u8> {
    let mut kept = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((&first, tail)) = rest.split_first() {
        if first == COLOUR {
            rest = tail.get(3..).unwrap_or_default();
            continue;
        }
        kept.push(first);
        rest = tail;
    }
    kept
}

fn number(expression: &Expression) -> Option<f64> {
    match expression {
        Expression::Number(token) => match token.token().token_type() {
            TokenType::Number { text } => text.as_str().parse().ok(),
            _ => None,
        },
        // `min = -1` is a unary minus applied to a literal, not a negative literal.
        Expression::UnaryOperator {
            unop: UnOp::Minus(_),
            expression,
        } => number(expression).map(|value| -value),
        _ => None,
    }
}

fn boolean(expression: &Expression) -> Option<bool> {
    match expression {
        Expression::Symbol(token) => match token.token().token_type() {
            TokenType::Symbol {
                symbol: Symbol::True,
            } => Some(true),
            TokenType::Symbol {
                symbol: Symbol::False,
            } => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn value_of(expression: &Expression) -> Option<Value> {
    boolean(expression)
        .map(Value::Bool)
        .or_else(|| number(expression).map(Value::Number))
        .or_else(|| string(expression).map(Value::Text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real table, so the assertions below are about BAR, not a fixture.
    fn schema() -> Vec<ModOption> {
        let lua = include_str!("../../../external/Beyond-All-Reason/modoptions.lua");
        parse(lua).expect("BAR's modoptions.lua should parse")
    }

    fn find<'a>(options: &'a [ModOption], key: &str) -> &'a ModOption {
        options
            .iter()
            .find(|option| option.key == key)
            .unwrap_or_else(|| panic!("no option {key}"))
    }

    #[test]
    fn every_section_is_read_with_its_weight() {
        let options = schema();
        let sections: Vec<_> = options
            .iter()
            .filter(|option| option.kind == Kind::Section)
            .map(|option| (option.key.as_str(), option.name.as_str(), option.weight))
            .collect();

        // Chobby sorts tabs by weight, descending; unweighted ones fall to the
        // end. `dev` it drops outright, and hidden sections never draw.
        assert_eq!(
            sections,
            vec![
                ("options_main", "Main", Some(7.0)),
                ("options", "Other", None),
                ("raptor_defense_options", "Raptors", Some(4.0)),
                ("scav_defense_options", "Scavengers", Some(3.0)),
                ("options_extra", "Extras", Some(2.0)),
                ("options_experimental", "Experimental", Some(1.0)),
                ("modes", "GameModes", None),
                ("dev", "_DEV", None),
                ("mapmetadata", "MapMetadata", None),
                ("options_cheats", "Cheats", Some(-1.0)),
            ]
        );
    }

    #[test]
    fn the_modding_six_are_where_we_expect_to_find_them() {
        let options = schema();
        // These are the options we regroup into a Modding tab. If BAR moves one
        // itself, this fails and the regrouping gets revisited rather than
        // silently disagreeing with the game.
        for (key, section) in [
            ("tweakdefs", "options_cheats"),
            ("tweakunits", "options_cheats"),
            ("forceallunits", "options_cheats"),
            ("experimentallegionfaction", "options_experimental"),
            ("experimentalextraunits", "options_extra"),
            ("scavunitsforplayers", "options_extra"),
        ] {
            assert_eq!(
                find(&options, key).section.as_deref(),
                Some(section),
                "{key} moved"
            );
        }
    }

    #[test]
    fn defaults_keep_the_shape_lua_wrote_them_in() {
        let options = schema();
        assert_eq!(find(&options, "ranked_game").def, Some(Value::Bool(true)));
        assert_eq!(
            find(&options, "startmetal").def,
            Some(Value::Number(1000.0))
        );
        assert_eq!(
            find(&options, "tweakdefs").def,
            Some(Value::Text(String::new()))
        );
        assert_eq!(
            find(&options, "experimentalshields").def,
            Some(Value::Text("unchanged".into()))
        );
    }

    #[test]
    fn a_list_option_carries_its_items() {
        let options = schema();
        let shields = find(&options, "experimentalshields");
        assert_eq!(shields.kind, Kind::List);
        let keys: Vec<_> = shields.items.iter().map(|item| item.key.as_str()).collect();
        assert!(keys.contains(&"unchanged"), "got {keys:?}");
        assert!(keys.len() >= 4, "got {keys:?}");
    }

    #[test]
    fn the_generated_tweak_slots_are_left_to_the_tweaks_crate() {
        let options = schema();
        // `tweakdefs1..9` come from a `for` loop we deliberately do not read.
        assert!(options.iter().all(|option| option.key != "tweakdefs1"));
    }

    #[test]
    fn a_name_joined_with_two_dots_is_read_whole_without_its_colour_code() {
        let options = schema();
        // `"No Rush Time" .. "\255\128\128\128" .. " [minutes]"` in the file.
        assert_eq!(find(&options, "norushtimer").name, "No Rush Time [minutes]");
    }

    #[test]
    fn a_description_is_decoded_the_way_lua_would_read_it() {
        let options = schema();
        let desc = &find(&options, "norushtimer").desc;
        assert!(desc.contains(".\nPLEASE NOTE"), "{desc:?}");
        assert!(!desc.contains('\\'), "{desc:?}");
        assert!(desc.contains("Raptors.\nWARNING"), "{desc:?}");
    }

    #[test]
    fn escapes_and_colour_codes() {
        assert_eq!(unescape(r"a\nb\tc\\d\65"), "a\nb\tc\\dA");
        assert_eq!(unescape(r"red\255\255\0\0 text"), "red text");
        assert_eq!(unescape(r#"it\'s \"q\""#), "it's \"q\"");
        // A colour code cut short at the end of a literal takes nothing else.
        assert_eq!(unescape(r"end\255\1"), "end");
    }

    #[test]
    fn hidden_options_are_marked_rather_than_dropped() {
        let options = schema();
        assert!(find(&options, "holiday_events").hidden);
        assert!(!find(&options, "startmetal").hidden);
    }
}
