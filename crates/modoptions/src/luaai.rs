//! The AIs a game implements in Lua, read out of its `luaai.lua`.
//!
//! The engine only knows the skirmish AIs it ships itself, one directory each
//! under `AI/Skirmish/`. An AI written in the game's own Lua — BAR's Scavengers
//! and Raptors among them — is declared instead by the game, in a `luaai.lua`
//! that returns a table of `{ name, desc }` rows. Chobby reads it through the
//! engine's VFS; this parses the file itself, over bytes
//! `content::Library::game_file` reads out of the installed game, so the list
//! matches the game version the room is running. Only the names are kept: the
//! description is the game's to show, not ours.

use full_moon::ast::{Field, LastStmt};

use crate::{identifier, lua_errors, string, table};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("luaai.lua does not parse as Lua: {0}")]
    Lua(String),
    #[error("luaai.lua does not return a table")]
    NoTable,
}

/// One AI the game implements. `name` is what `ADDBOT` and the start script
/// call it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaAi {
    pub name: String,
}

/// The AIs `luaai.lua` declares, in the order the file lists them.
pub fn parse(lua: &str) -> Result<Vec<LuaAi>, Error> {
    let ast = full_moon::parse(lua).map_err(|errors| Error::Lua(lua_errors(&errors)))?;

    let Some(LastStmt::Return(returned)) = ast.nodes().last_stmt() else {
        return Err(Error::NoTable);
    };
    let rows = returned
        .returns()
        .iter()
        .find_map(table)
        .ok_or(Error::NoTable)?;

    Ok(rows.fields().iter().filter_map(read_ai).collect())
}

fn read_ai(field: &Field) -> Option<LuaAi> {
    let Field::NoKey(expression) = field else {
        return None;
    };
    let entry = table(expression)?;

    entry.fields().iter().find_map(|field| {
        let Field::NameKey { key, value, .. } = field else {
            return None;
        };
        if identifier(key.token().token_type()) != Some("name") {
            return None;
        }
        let name = string(value)?;
        (!name.is_empty()).then_some(LuaAi { name })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ais: &[LuaAi]) -> Vec<&str> {
        ais.iter().map(|ai| ai.name.as_str()).collect()
    }

    /// The real file, so the assertion is about BAR rather than a fixture.
    #[test]
    fn bar_declares_scavengers_and_raptors() {
        let lua = include_str!("../../../external/Beyond-All-Reason/luaai.lua");
        let ais = parse(lua).expect("BAR's luaai.lua should parse");

        assert!(names(&ais).contains(&"ScavengersAI"));
        assert!(names(&ais).contains(&"RaptorsAI"));
    }

    #[test]
    fn rows_keep_the_order_of_the_file_and_only_the_name_matters() {
        let ais = parse(
            r#"
            -- a comment first
            return {
                { name = "First", desc = "one" },
                { name = "Second" },
                { desc = "no name, so not an AI" },
                { name = "Th" .. "ird" },
            }
            "#,
        )
        .unwrap();

        assert_eq!(names(&ais), ["First", "Second", "Third"]);
    }

    #[test]
    fn a_file_that_returns_nothing_is_an_error_rather_than_an_empty_list() {
        assert!(matches!(parse("local x = 1"), Err(Error::NoTable)));
        assert!(matches!(parse("return 42"), Err(Error::NoTable)));
    }

    #[test]
    fn broken_lua_says_so() {
        assert!(matches!(parse("return { name = "), Err(Error::Lua(_))));
    }
}
