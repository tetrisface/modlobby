//! What the widget may say, and how we decide it was really the widget.
//!
//! One line per request: `<token> <verb>`. Small on purpose — this crosses a
//! loopback socket any local process can open, so the only sane surface is one
//! a glance can audit.

/// What the in-game widget is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Escape was pressed with nothing selected: raise the lobby.
    Raise,
    /// Stop the game.
    QuitGame,
}

/// Reads one line, or `None` if it was not the widget or not a verb we know.
///
/// The token is compared in constant time over its whole length, so a wrong
/// guess cannot be narrowed down by how long the answer took.
pub fn parse(line: &str, token: &str) -> Option<Command> {
    let (given, verb) = line.trim().split_once(' ')?;
    if !constant_time_eq(given.as_bytes(), token.as_bytes()) {
        return None;
    }
    match verb.trim() {
        "raise" => Some(Command::Raise),
        "quit" => Some(Command::QuitGame),
        _ => None,
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // `fold`, not `all`: short-circuiting is exactly what leaks the position
    // of the first wrong byte.
    a.iter()
        .zip(b)
        .fold(0u8, |differs, (x, y)| differs | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "8f3c1d9e";

    #[test]
    fn the_two_things_the_widget_can_ask_for() {
        assert_eq!(parse("8f3c1d9e raise", TOKEN), Some(Command::Raise));
        assert_eq!(parse("8f3c1d9e quit", TOKEN), Some(Command::QuitGame));
        // Lua writes a newline; the reader may or may not have stripped it.
        assert_eq!(parse("8f3c1d9e raise\r\n", TOKEN), Some(Command::Raise));
    }

    #[test]
    fn a_wrong_token_asks_for_nothing() {
        assert_eq!(parse("deadbeef raise", TOKEN), None);
        assert_eq!(parse("8f3c1d9 raise", TOKEN), None, "a prefix is not it");
        assert_eq!(parse("8f3c1d9ee raise", TOKEN), None, "nor is a superset");
        assert_eq!(parse(" raise", TOKEN), None);
    }

    #[test]
    fn a_verb_we_do_not_know_is_not_guessed_at() {
        assert_eq!(parse("8f3c1d9e launch", TOKEN), None);
        assert_eq!(parse("8f3c1d9e", TOKEN), None);
        assert_eq!(parse("", TOKEN), None);
    }

    #[test]
    fn comparing_a_token_does_not_stop_at_the_first_wrong_byte() {
        assert!(constant_time_eq(b"abcd", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abce"));
        assert!(!constant_time_eq(b"abcd", b"zbcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
    }
}
