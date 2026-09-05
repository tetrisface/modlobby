//! Channels: joining them, leaving them, and talking in them.
//!
//! Private messages live here too, because from the protocol's side a person
//! is just another place to say something.

use crate::battle::TooLong;
use crate::paste;
use crate::policy::{Area, Envelope};

/// teiserver truncates a channel message with `String.slice(0..256)`
/// (`spring_in.ex:767`). That range is inclusive, so 257 characters survive and
/// the 258th is the first one lost.
pub const SAY_MAX_LEN: usize = 257;

/// A channel name as teiserver will read it: its `JOIN` and `SAY` handlers both
/// match `\w+`, so anything else is silently a different room, or no room.
pub fn valid_channel(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadChannel;

impl std::fmt::Display for BadChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a channel name is letters, digits and underscores")
    }
}

impl std::error::Error for BadChannel {}

pub fn join(room: &str, key: Option<&str>) -> Result<Envelope, BadChannel> {
    if !valid_channel(room) {
        return Err(BadChannel);
    }
    let line = match key {
        // `JOIN <room>\t<key>` (`spring_in.ex:719`).
        Some(key) if !key.is_empty() => format!("JOIN {room}\t{key}"),
        _ => format!("JOIN {room}"),
    };
    Ok(Envelope::queue(Area::ChannelChat, line))
}

pub fn leave(room: &str) -> Result<Envelope, BadChannel> {
    if !valid_channel(room) {
        return Err(BadChannel);
    }
    Ok(Envelope::queue(Area::ChannelChat, format!("LEAVE {room}")))
}

/// The server's channel listing, answered as `CHANNEL` lines then `ENDOFCHANNELS`.
pub fn list() -> Envelope {
    Envelope::queue(Area::ChannelChat, "CHANNELS")
}

fn capped(text: &str) -> Result<(), TooLong> {
    let len = text.chars().count();
    if len > SAY_MAX_LEN {
        return Err(TooLong {
            len,
            max: SAY_MAX_LEN,
        });
    }
    Ok(())
}

pub fn say(room: &str, text: &str) -> Result<Envelope, SayError> {
    if !valid_channel(room) {
        return Err(SayError::Channel);
    }
    capped(text)?;
    Ok(Envelope::queue(
        Area::ChannelChat,
        format!("SAY {room} {text}"),
    ))
}

/// `SAYEX`: an emote, which the server relays as `SAIDEX`.
pub fn say_ex(room: &str, text: &str) -> Result<Envelope, SayError> {
    if !valid_channel(room) {
        return Err(SayError::Channel);
    }
    capped(text)?;
    Ok(Envelope::queue(
        Area::ChannelChat,
        format!("SAYEX {room} {text}"),
    ))
}

/// A pasted channel message, one `SAY` per line, long lines wrapped. A line
/// starting `/me ` is an emote, which is how every lobby client has spelled
/// it since the protocol was written.
pub fn say_lines(room: &str, text: &str) -> Result<Vec<Envelope>, SayError> {
    if !valid_channel(room) {
        return Err(SayError::Channel);
    }
    let mut envelopes = Vec::new();
    for line in paste::lines(text) {
        let action = line.strip_prefix("/me ");
        for piece in paste::wrap(action.unwrap_or(line), SAY_MAX_LEN) {
            envelopes.push(match action {
                Some(_) => say_ex(room, &piece)?,
                None => say(room, &piece)?,
            });
        }
    }
    Ok(envelopes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SayError {
    Channel,
    TooLong(TooLong),
}

impl From<TooLong> for SayError {
    fn from(err: TooLong) -> Self {
        Self::TooLong(err)
    }
}

impl std::fmt::Display for SayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Channel => BadChannel.fmt(f),
            Self::TooLong(long) => write!(f, "{long}"),
        }
    }
}

impl std::error::Error for SayError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(envelope: Envelope) -> String {
        envelope.line
    }

    #[test]
    fn joining_and_leaving_name_the_room() {
        assert_eq!(line(join("main", None).unwrap()), "JOIN main");
        assert_eq!(line(join("main", Some("")).unwrap()), "JOIN main");
        assert_eq!(line(join("secret", Some("pw")).unwrap()), "JOIN secret\tpw");
        assert_eq!(line(leave("main").unwrap()), "LEAVE main");
    }

    #[test]
    fn a_name_the_server_would_not_match_is_refused_here() {
        // Both handlers match `\w+`, so `#main` joins nothing and reports nothing.
        assert_eq!(join("#main", None), Err(BadChannel));
        assert_eq!(join("two words", None), Err(BadChannel));
        assert_eq!(join("", None), Err(BadChannel));
        assert!(valid_channel("bar_moderators"));
    }

    #[test]
    fn the_cap_is_inclusive_like_the_slice_that_enforces_it() {
        let fits = "x".repeat(SAY_MAX_LEN);
        let over = "x".repeat(SAY_MAX_LEN + 1);
        assert!(say("main", &fits).is_ok());
        assert_eq!(
            say("main", &over),
            Err(SayError::TooLong(TooLong { len: 258, max: 257 }))
        );
    }

    #[test]
    fn a_paste_is_one_say_per_line_and_a_long_line_is_wrapped() {
        let text = format!("hello\n/me waves\n{} tail", "w".repeat(257));
        let sent: Vec<String> = say_lines("main", &text)
            .unwrap()
            .into_iter()
            .map(line)
            .collect();
        assert_eq!(
            sent,
            [
                "SAY main hello".to_string(),
                "SAYEX main waves".to_string(),
                format!("SAY main {}", "w".repeat(257)),
                "SAY main tail".to_string(),
            ]
        );
        assert_eq!(say_lines("#main", "hi"), Err(SayError::Channel));
    }

    #[test]
    fn an_emote_is_its_own_command() {
        assert_eq!(line(say("main", "hello").unwrap()), "SAY main hello");
        assert_eq!(line(say_ex("main", "waves").unwrap()), "SAYEX main waves");
    }
}
