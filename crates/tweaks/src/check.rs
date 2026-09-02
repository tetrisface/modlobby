//! Whether a payload will load, said with positions -- and what is in it.
//!
//! One parse serves both. `full_moon` parses what it can and keeps going, so
//! a payload with a typo on line 40 still yields the forty unit keys above it
//! for the outline, and the error lands on the line rather than as a
//! sentence in a notice.

use full_moon::LuaVersion;
use full_moon::tokenizer::Position;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Kind, outline};

/// One place the Lua stops making sense. Lines and columns are 1-based, and
/// the end is exclusive, which is how an editor draws a marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Problem {
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub message: String,
}

/// Something named at the top level: a unit key in `tweakunits`, a local,
/// an assignment or a function in `tweakdefs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Symbol {
    pub name: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Check {
    pub problems: Vec<Problem>,
    pub outline: Vec<Symbol>,
}

/// What `tweakunits` is evaluated as; see [`crate::lua::format`].
const WRAP: &str = "return ";

pub fn check(source: &str, kind: Kind) -> Check {
    let (wrapped, shift) = match kind {
        Kind::Defs => (source.to_owned(), 0),
        Kind::Units => (format!("{WRAP}{source}"), WRAP.len()),
    };
    let result = full_moon::parse_fallible(&wrapped, LuaVersion::lua51());
    let problems = result
        .errors()
        .iter()
        .map(|error| {
            let (start, end) = error.range();
            let line = to_u32(start.line());
            let column = unwrapped(start, shift);
            let end_line = to_u32(end.line());
            let mut end_column = unwrapped(end, shift);
            // A point has no width; a marker needs one.
            if end_line == line && end_column <= column {
                end_column = column + 1;
            }
            Problem {
                line,
                column,
                end_line,
                end_column,
                message: error.error_message().into_owned(),
            }
        })
        .collect();
    Check {
        problems,
        outline: outline::symbols(result.ast(), kind),
    }
}

/// A column in the source, not in the `return `-prefixed text that was parsed.
fn unwrapped(at: Position, shift: usize) -> u32 {
    let column = at.character();
    if at.line() == 1 {
        return to_u32(column.saturating_sub(shift).max(1));
    }
    to_u32(column)
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_lua_has_no_problems_and_an_outline() {
        let out = check(
            "-- Nutty\nlocal walls = { armwall = 1 }\nfunction scale(ud) end\nUnitDefs.armcom = nil\nlocal function helper() end\n",
            Kind::Defs,
        );
        assert!(out.problems.is_empty(), "{:?}", out.problems);
        let names: Vec<(&str, u32)> = out
            .outline
            .iter()
            .map(|s| (s.name.as_str(), s.line))
            .collect();
        assert_eq!(
            names,
            vec![
                ("walls", 2),
                ("scale", 3),
                ("UnitDefs.armcom", 4),
                ("helper", 5)
            ]
        );
    }

    #[test]
    fn units_are_outlined_by_unit_key_including_string_keys() {
        let out = check(
            "{\n\tarmcom = { metalcost = 1 },\n\t[\"cor_golt4\"] = {},\n\tcorak = {},\n}",
            Kind::Units,
        );
        assert!(out.problems.is_empty(), "{:?}", out.problems);
        let names: Vec<(&str, u32)> = out
            .outline
            .iter()
            .map(|s| (s.name.as_str(), s.line))
            .collect();
        assert_eq!(names, vec![("armcom", 2), ("cor_golt4", 3), ("corak", 4)]);
    }

    #[test]
    fn a_units_problem_on_the_first_line_is_placed_in_the_source_not_the_wrapper() {
        // `{ armcom = }`: the missing expression is reported at the `=`,
        // column 10 of the source and column 17 of what was parsed.
        let out = check("{ armcom = }", Kind::Units);
        let problem = out.problems.first().expect("a problem");
        assert_eq!(problem.line, 1);
        assert_eq!(problem.column, 10, "{problem:?}");
        assert!(problem.end_column > problem.column);
    }

    #[test]
    fn a_problem_on_a_later_line_keeps_its_column() {
        let out = check("{\n\tarmcom = {\n\t\tmetalcost = ,\n\t},\n}", Kind::Units);
        let problem = out.problems.first().expect("a problem");
        assert_eq!(problem.line, 3, "{problem:?}");
        // Two tabs, `metalcost`, a space: the `=` is column 13, unshifted.
        assert_eq!(problem.column, 13, "{problem:?}");
        // What parsed before the problem is still in the outline.
        assert_eq!(out.outline.first().map(|s| s.name.as_str()), Some("armcom"));
    }

    #[test]
    fn a_tokenizer_error_is_a_problem_too() {
        let out = check("local s = 'unterminated", Kind::Defs);
        assert!(!out.problems.is_empty());
        assert_eq!(out.problems[0].line, 1);
    }
}
