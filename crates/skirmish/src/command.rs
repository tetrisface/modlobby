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
//!
//! `!force` is here, and is the reason dragging a LAN guest onto a team works
//! without the room knowing there is such a thing. Online, moving somebody who
//! is not you and is not your AI is a request to the host in battle chat
//! (`views/room/move.ts`); here the console *is* the host, so the same gesture
//! sends the same line and it lands on the same room. A feature that needed no
//! new path through the window is a feature that cannot have broken one.

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
    ("!lan [port]", "open the room to this network, or !lan off"),
    ("!addPlayer <name>", "expect somebody to join over the LAN"),
    ("!removePlayer <name>", "stop expecting them"),
    ("!force <name> team <n>", "move somebody to a team"),
    ("!spec <name>", "a guest watches instead of playing"),
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
        "lan" => lan(room, tail),
        "addplayer" | "addguest" => add_guest(room, tail),
        "removeplayer" | "removeguest" => named(tail, "!removePlayer <name>", |name| {
            if room.remove_guest(name) {
                Outcome::Did(format!("{name} is not expected any more"))
            } else {
                Outcome::Said(format!("nobody here is called {name}"))
            }
        }),
        "force" => force(room, tail),
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
        // Of the host-only commands, this one has a meaning here: a guest with
        // no seat is still in the script and still let in, which is how
        // somebody on this network watches a LAN game rather than plays one.
        "spec" => named(tail, "!spec <name>", |name| {
            if room.seat_guest(name, None) {
                Outcome::Did(format!("{name} is watching"))
            } else if room.guests().iter().any(|guest| guest.name == name) {
                Outcome::Said(format!("{name} is already watching"))
            } else {
                Outcome::Said(format!("nobody here is called {name}"))
            }
        }),
        "balance" | "lock" | "unlock" | "vote" | "kick" | "boss" | "preset" => {
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

/// `!lan`, `!lan <port>`, `!lan off`.
///
/// Bare, it opens on the engine's own default port, which is the number a
/// guest joining from anything but modlobby would have to type the least of.
fn lan(room: &mut Room, tail: &str) -> Outcome {
    let tail = tail.trim();
    let port = match tail.to_ascii_lowercase().as_str() {
        "" | "on" => Some(recoil::script::DEFAULT_PORT),
        "off" | "close" | "private" => None,
        _ => match tail.parse::<u16>() {
            // Port 0 is the engine's "pick one for me", which cannot be
            // announced and cannot be joined.
            Ok(port) if port != 0 => Some(port),
            _ => return Outcome::Said("!lan [port], or !lan off".to_owned()),
        },
    };
    if !room.set_lan(port) {
        return Outcome::Said(match port {
            Some(port) => format!("the room is already open on port {port}"),
            None => "the room is already private".to_owned(),
        });
    }
    Outcome::Did(match port {
        Some(port) => {
            format!("the room is open to this network on port {port}; add the people who will join")
        }
        None => "the room is private again".to_owned(),
    })
}

/// `!addPlayer <name>`, on the next free team.
fn add_guest(room: &mut Room, tail: &str) -> Outcome {
    let Some(name) = tail.split_whitespace().next() else {
        return Outcome::Said("!addPlayer <name>".to_owned());
    };
    let team = room.free_team();
    let ally = room.free_ally();
    let colour = crate::COLOURS[usize::from(ally) % crate::COLOURS.len()];
    if room.add_guest(name, team, ally, colour) {
        Outcome::Did(format!("{name} is expected on team {}", ally + 1))
    } else {
        Outcome::Said(format!("{name} is already here"))
    }
}

/// `!force <name> team <n>` and `!force <name> bonus <n>`, as SPADS spells
/// them and as the room's drag gesture sends them.
///
/// SPADS counts teams from one and prefixes a bot's name with `%` so it is not
/// looked for among the players (`spads.pl:8917-8996`); both are honoured here
/// so one gesture produces one line whichever kind of room it is in.
fn force(room: &mut Room, tail: &str) -> Outcome {
    let mut words = tail.split_whitespace();
    let (Some(who), Some(what), Some(value)) = (words.next(), words.next(), words.next()) else {
        return Outcome::Said("!force <name> team <n>".to_owned());
    };
    let Ok(value) = value.parse::<u32>() else {
        return Outcome::Said(format!("!force <name> {what} <n>"));
    };
    let bot = who.strip_prefix('%');
    let name = bot.unwrap_or(who);

    match what.to_ascii_lowercase().as_str() {
        "team" => {
            let Some(ally) = value.checked_sub(1).filter(|ally| *ally < 256) else {
                return Outcome::Said("!force <name> team <n>, counting from 1".to_owned());
            };
            let ally = ally as u8;
            moved(room, name, bot.is_some(), ally, None)
        }
        "bonus" => {
            let bonus = value.min(100) as u8;
            moved(room, name, bot.is_some(), u8::MAX, Some(bonus))
        }
        _ => Outcome::Said(format!(
            "!force <name> {what} is not something this room does"
        )),
    }
}

/// The move itself, against whichever kind of participant has that name.
///
/// `ally == u8::MAX` means "wherever they already are", which is what a bonus
/// change wants: the message replaces a whole status, so the parts not being
/// changed have to be sent again.
fn moved(room: &mut Room, name: &str, bot: bool, ally: u8, bonus: Option<u8>) -> Outcome {
    if !bot && let Some(guest) = room.guests().iter().find(|guest| guest.name == name) {
        if bonus.is_some() {
            return Outcome::Said("a bonus is something a host gives an AI.".to_owned());
        }
        let (team, colour) = (
            guest.seat.map_or_else(|| room.free_team(), |s| s.team),
            guest.colour,
        );
        room.update_guest(name, team, ally, colour);
        return Outcome::Did(format!("{name} is on team {}", ally + 1));
    }
    let Some(ai) = room.ais().iter().find(|ai| ai.name == name) else {
        return Outcome::Said(format!("nobody here is called {name}"));
    };
    let (team, colour, held) = (ai.seat.team, ai.colour, ai.seat);
    let ally = if ally == u8::MAX {
        held.ally_team
    } else {
        ally
    };
    let handicap = bonus.unwrap_or(held.handicap);
    room.update_ai(name, team, ally, handicap, colour);
    match bonus {
        Some(bonus) => Outcome::Did(format!("{name} has a {bonus}% bonus")),
        None => Outcome::Did(format!("{name} is on team {}", ally + 1)),
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

    /// The console is the host here, so the line the room's drag gesture sends
    /// online lands on the room itself.
    ///
    /// This is the whole reason a LAN guest can be dragged onto a team without
    /// anything in the window having been taught what a guest is: `move.ts`
    /// sends `!force <name> team <n>` for anybody who is not you and not your
    /// own AI, and here that arrives as a command rather than as chat.
    #[test]
    fn force_is_what_makes_the_drag_gesture_work_on_a_guest() {
        let mut room = room();
        room.set_lan(Some(8452));
        room.add_guest("ann", 1, 1, crate::COLOURS[1]);

        // SPADS counts teams from one and takes the one back off, so the
        // number on the wire is one more than the index everything else uses.
        assert_eq!(did(&mut room, "!force ann team 3"), "ann is on team 3");
        assert_eq!(room.guests()[0].seat.unwrap().ally_team, 2);

        // A bot's name arrives prefixed, so it is not looked for among the
        // players -- and the same line moves it.
        room.add_ai("BARb", "BARb", 2, 0, crate::COLOURS[2]);
        assert_eq!(did(&mut room, "!force %BARb team 2"), "BARb is on team 2");
        assert_eq!(room.ais()[0].seat.ally_team, 1);
        assert_eq!(
            did(&mut room, "!force %BARb bonus 40"),
            "BARb has a 40% bonus"
        );
        assert_eq!(room.ais()[0].seat.handicap, 40);
        // The bonus did not move it.
        assert_eq!(room.ais()[0].seat.ally_team, 1);

        assert!(said(&mut room, "!force nobody team 2").contains("nobody here"));
        assert!(said(&mut room, "!force ann team").contains("!force"));
        assert!(said(&mut room, "!force ann team 0").contains("counting from 1"));
        assert!(said(&mut room, "!force ann rank 2").contains("not something"));
    }

    /// Opening a room is a deliberate act with a word for it, and the console
    /// is where the port that is not the default gets said.
    #[test]
    fn lan_opens_the_room_and_closes_it_again() {
        let mut room = room();
        assert!(did(&mut room, "!lan").contains("port 8452"));
        assert_eq!(room.lan(), Some(8452));
        assert!(said(&mut room, "!lan").contains("already open"));

        assert!(did(&mut room, "!lan 9000").contains("port 9000"));
        assert_eq!(room.lan(), Some(9000));

        assert!(did(&mut room, "!lan off").contains("private again"));
        assert_eq!(room.lan(), None);
        assert!(said(&mut room, "!lan off").contains("already private"));

        // The engine's "pick one for me" cannot be announced or joined.
        assert!(said(&mut room, "!lan 0").contains("!lan [port]"));
        assert!(said(&mut room, "!lan soon").contains("!lan [port]"));
        assert_eq!(room.lan(), None);
    }

    #[test]
    fn a_guest_is_added_by_name_and_taken_out_by_name() {
        let mut room = room();
        room.set_lan(Some(8452));
        assert!(did(&mut room, "!addPlayer ann").contains("ann is expected"));
        assert_eq!(room.guests().len(), 1);
        // On a side of their own, which is what makes the first one an
        // opponent rather than a team-mate.
        assert_eq!(room.guests()[0].seat.unwrap().ally_team, 1);

        assert!(said(&mut room, "!addPlayer ann").contains("already here"));
        assert!(said(&mut room, "!addPlayer me").contains("already here"));
        assert!(said(&mut room, "!addPlayer").contains("!addPlayer <name>"));

        // Watching rather than playing is still somebody the engine lets in.
        assert_eq!(did(&mut room, "!spec ann"), "ann is watching");
        assert!(room.guests()[0].seat.is_none());
        assert!(said(&mut room, "!spec ann").contains("already watching"));
        assert!(said(&mut room, "!spec nobody").contains("nobody here"));

        assert!(did(&mut room, "!removePlayer ann").contains("not expected"));
        assert!(room.guests().is_empty());
        assert!(said(&mut room, "!removePlayer ann").contains("nobody here"));
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
