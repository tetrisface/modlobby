//! The `!` commands, where they have a meaning in a room with no host.
//!
//! A skirmish has no chat, so the composer under the roster is a console
//! instead. The commands keep SPADS' own names and spelling — `!bSet`, `!map`,
//! `!start` — because that is what people already type, and because the
//! `!bSet <slot> <blob>` the setup pane copies to the clipboard then pastes
//! straight into a room of your own.
//!
//! What is not here is what needs somebody else: `!balance`, `!lock`, `!vote`,
//! `!kick`. Those are answered with a line saying so rather than ignored.

use crate::Room;

/// What a line did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The room changed. The line says how, for the log.
    Did(String),
    /// Start the game.
    Launch,
    /// Nothing changed. The line is the answer — a refusal, or `!help`.
    Said(String),
    /// Nothing changed and nothing worth saying: a control set to the value it
    /// already had. The console never answers this way; a click does.
    Nothing,
}

/// Every command this room answers to, with what it does.
const HELP: &[(&str, &str)] = &[
    (
        "!bSet <key> <value>",
        "set a modoption; an empty value clears it",
    ),
    ("!map <name>", "play a different map"),
    ("!game <version>", "play a different game version"),
    ("!engine <version>", "play on a different engine"),
    ("!rename <title>", "name the room"),
    ("!addBot <name> <ai>", "add an AI on the next free team"),
    ("!removeBot <name>", "take one out"),
    ("!fixColors", "one colour per side"),
    ("!set teamSize <n>", "how many per team"),
    ("!nbTeams <n>", "how many teams"),
    ("!start", "play"),
];

/// Runs one line from the room's composer.
pub fn command(room: &mut Room, line: &str) -> Outcome {
    let line = line.trim();
    let Some(rest) = line.strip_prefix('!') else {
        return Outcome::Said("Nobody else is here. Try !help.".to_owned());
    };
    let mut words = rest.split_whitespace();
    let Some(name) = words.next() else {
        return Outcome::Said("Try !help.".to_owned());
    };
    let tail = rest[name.len()..].trim();

    match name.to_ascii_lowercase().as_str() {
        "help" => Outcome::Said(help()),
        "start" | "forcestart" => Outcome::Launch,
        "bset" => b_set(room, tail),
        "map" => named(tail, "!map <name>", |map| {
            room.set_map(map);
            Outcome::Did(format!("map is {map}"))
        }),
        "game" => named(tail, "!game <version>", |game| {
            room.set_game(game);
            Outcome::Did(format!("game is {game}"))
        }),
        "engine" => named(tail, "!engine <version>", |engine| {
            room.set_engine(engine);
            Outcome::Did(format!("engine is {engine}"))
        }),
        "rename" => named(tail, "!rename <title>", |title| {
            room.set_title(title);
            Outcome::Did(format!("room is called {title}"))
        }),
        "addbot" | "addai" => add_bot(room, tail),
        "removebot" | "removeai" => named(tail, "!removeBot <name>", |name| {
            if room.remove_ai(name) {
                Outcome::Did(format!("{name} is out"))
            } else {
                Outcome::Said(format!("no AI here is called {name}"))
            }
        }),
        "fixcolors" | "fixcolours" => {
            room.fix_colours();
            Outcome::Did("every side has its own colour".to_owned())
        }
        "set" => set(room, tail),
        "nbteams" => count(tail, "!nbTeams <n>", |teams| {
            let size = room.layout_size();
            room.set_layout(teams, size);
            format!("{teams} teams")
        }),
        // Worth saying why rather than answering "unknown": these are the
        // commands whose absence is the point of a room with nobody in it.
        "balance" | "lock" | "unlock" | "vote" | "kick" | "spec" | "boss" | "preset" => {
            Outcome::Said(format!("!{name} needs a host. This room has none."))
        }
        _ => Outcome::Said(format!("!{name} means nothing here. Try !help.")),
    }
}

fn help() -> String {
    let mut out = String::from("Commands:");
    for (form, what) in HELP {
        out.push_str(&format!("\n  {form} — {what}"));
    }
    out
}

/// `!bSet <key> <value>`; no value clears the option, which is what falls back
/// to the game's own default rather than setting it to an empty string.
fn b_set(room: &mut Room, tail: &str) -> Outcome {
    let mut parts = tail.splitn(2, char::is_whitespace);
    let Some(key) = parts.next().filter(|key| !key.is_empty()) else {
        return Outcome::Said("!bSet <key> <value>".to_owned());
    };
    let value = parts.next().unwrap_or("").trim();
    if value.is_empty() {
        return if room.clear_option(key) {
            Outcome::Did(format!("{key} cleared"))
        } else {
            Outcome::Said(format!("{key} was not set"))
        };
    }
    if room.set_option(key, value) {
        Outcome::Did(format!("{key} = {value}"))
    } else {
        Outcome::Said(format!("{key} is already {value}"))
    }
}

/// `!addBot <name> <ai>`, and `!addBot <ai>` when one name will do for both.
fn add_bot(room: &mut Room, tail: &str) -> Outcome {
    let mut words = tail.split_whitespace();
    let Some(first) = words.next() else {
        return Outcome::Said("!addBot <name> <ai>".to_owned());
    };
    let ai = words.next().unwrap_or(first);
    let name = if words.clone().next().is_some() || ai == first {
        room.unused_name(ai)
    } else {
        first.to_owned()
    };
    let team = room.free_team();
    let ally = room.free_ally();
    let colour = crate::COLOURS[usize::from(ally) % crate::COLOURS.len()];
    if room.add_ai(&name, ai, team, ally, colour) {
        Outcome::Did(format!("{name} ({ai}) on team {}", ally + 1))
    } else {
        Outcome::Said(format!("{name} is already here"))
    }
}

/// `!set <key> <value>`, of which only the room's shape means anything here.
fn set(room: &mut Room, tail: &str) -> Outcome {
    let mut words = tail.split_whitespace();
    let Some(key) = words.next() else {
        return Outcome::Said("!set teamSize <n>".to_owned());
    };
    let value = words.next().unwrap_or("");
    match key.to_ascii_lowercase().as_str() {
        "teamsize" => count(value, "!set teamSize <n>", |size| {
            let teams = room.layout_teams();
            room.set_layout(teams, size);
            format!("{size} per team")
        }),
        "nbteams" => count(value, "!set nbTeams <n>", |teams| {
            let size = room.layout_size();
            room.set_layout(teams, size);
            format!("{teams} teams")
        }),
        "startpostype" => count(value, "!set startPosType <0-2>", |pos| {
            let pos = (pos - 1) as u8;
            room.set_start_pos(pos);
            format!("start positions: {}", crate::start_pos_name(pos))
        }),
        _ => Outcome::Said(format!("!set {key} is a host's setting; try !bSet.")),
    }
}

/// A command whose argument is the whole rest of the line. The body says for
/// itself whether anything changed -- asking for an AI that is not here runs
/// fine and changes nothing.
fn named(tail: &str, form: &str, run: impl FnOnce(&str) -> Outcome) -> Outcome {
    if tail.is_empty() {
        return Outcome::Said(form.to_owned());
    }
    run(tail)
}

/// A command whose argument is a count between 1 and 16.
fn count(tail: &str, form: &str, run: impl FnOnce(u32) -> String) -> Outcome {
    match tail.trim().parse::<u32>() {
        Ok(n) if (1..=16).contains(&n) => Outcome::Did(run(n)),
        _ => Outcome::Said(format!("{form}, 1 to 16")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> Room {
        Room::new("me", "BAR test-1", "Comet Catcher", "2026.07.04")
    }

    fn did(room: &mut Room, line: &str) -> String {
        match command(room, line) {
            Outcome::Did(said) => said,
            other => panic!("expected a change from {line:?}, got {other:?}"),
        }
    }

    fn said(room: &mut Room, line: &str) -> String {
        match command(room, line) {
            Outcome::Said(said) => said,
            other => panic!("expected an answer to {line:?}, got {other:?}"),
        }
    }

    #[test]
    fn bset_takes_the_command_the_setup_pane_copies() {
        let mut room = room();
        // Exactly what `SlotActions` puts on the clipboard, pasted here.
        did(&mut room, "!bSet tweakdefs1 LS1OdXR0eUIgdjEuNTI=");
        assert_eq!(room.modoption("tweakdefs1"), "LS1OdXR0eUIgdjEuNTI=");
    }

    #[test]
    fn bset_is_spelled_however_it_is_typed() {
        let mut room = room();
        did(&mut room, "!bset ranked_game 0");
        assert_eq!(room.modoption("ranked_game"), "0");
        assert!(said(&mut room, "!BSET ranked_game 0").contains("already"));
    }

    #[test]
    fn bset_with_no_value_clears_rather_than_empties() {
        let mut room = room();
        did(&mut room, "!bSet tweakdefs1 abc");
        did(&mut room, "!bSet tweakdefs1");
        assert_eq!(room.modoption("tweakdefs1"), "");
        assert!(room.view(content()).my.script_tags.is_empty());
    }

    fn content() -> lobby_ui::ContentView {
        lobby_ui::ContentView {
            engine: true,
            game: true,
            map: true,
        }
    }

    #[test]
    fn start_is_the_one_that_leaves_the_room() {
        assert_eq!(command(&mut room(), "!start"), Outcome::Launch);
        assert_eq!(command(&mut room(), "!forceStart"), Outcome::Launch);
    }

    #[test]
    fn a_bot_lands_on_a_side_of_its_own() {
        let mut room = room();
        let said = did(&mut room, "!addBot BARb");
        assert!(said.contains("team 2"), "{said}");
        assert_eq!(room.ais().len(), 1);
        assert_eq!(room.ais()[0].ai, "BARb");
        // A second one goes opposite again, with a name it does not share.
        did(&mut room, "!addBot BARb");
        assert_eq!(room.ais()[1].name, "BARb2");
    }

    #[test]
    fn a_bot_can_be_named_as_well_as_chosen() {
        let mut room = room();
        did(&mut room, "!addBot Left BARb");
        assert_eq!(room.ais()[0].name, "Left");
        assert_eq!(room.ais()[0].ai, "BARb");
        assert!(said(&mut room, "!removeBot Nobody").contains("no AI"));
        assert!(did(&mut room, "!removeBot Left").contains("out"));
        assert!(room.ais().is_empty());
    }

    #[test]
    fn the_map_and_the_name_are_taken_whole() {
        let mut room = room();
        did(&mut room, "!map Supreme Isthmus v2.1");
        assert_eq!(room.map, "Supreme Isthmus v2.1");
        did(&mut room, "!rename Tuesday night raptors");
        assert_eq!(room.title, "Tuesday night raptors");
        did(&mut room, "!game Beyond All Reason test-31200");
        assert_eq!(room.game, "Beyond All Reason test-31200");
        did(&mut room, "!engine 2026.08.01");
        assert_eq!(room.engine, "2026.08.01");
        assert_eq!(said(&mut room, "!map"), "!map <name>");
    }

    #[test]
    fn the_rooms_shape_is_a_count_within_reason() {
        let mut room = room();
        did(&mut room, "!set teamSize 8");
        did(&mut room, "!nbTeams 2");
        let layout = room.view(content()).battle.layout.unwrap();
        assert_eq!((layout.teams, layout.team_size), (2, 8));
        assert!(said(&mut room, "!set teamSize 40").contains("1 to 16"));
        assert!(said(&mut room, "!nbTeams x").contains("1 to 16"));
    }

    #[test]
    fn what_needs_a_host_says_so_rather_than_failing_silently() {
        let mut room = room();
        for line in ["!balance", "!lock", "!vote y", "!kick someone"] {
            assert!(said(&mut room, line).contains("host"), "{line}");
        }
        assert!(said(&mut room, "!wat").contains("means nothing here"));
        assert!(said(&mut room, "hello?").contains("Nobody else is here"));
        assert!(said(&mut room, "!help").contains("!bSet"));
    }
}
