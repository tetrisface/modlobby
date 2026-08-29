//! Line-level syntax: `[#<id> ]<COMMAND>[ <args>]\n`.
//!
//! The server echoes `#<id>` on every reply line it produces while handling a
//! request tagged with that id (`spring_out.ex` `_send/3`). Arguments are
//! command-specific; the `s.`/`c.` extensions separate them with tabs.

/// teiserver drops a partial line once its buffer exceeds 64 KiB
/// (`teiserver.Spring max message buffer size`).
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// A protocol line split into its syntactic parts, before command-specific parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMessage {
    /// Present on replies to a tagged request.
    pub id: Option<u32>,
    pub command: String,
    /// Everything after the command name; empty when the command takes no arguments.
    pub args: String,
}

impl RawMessage {
    pub fn parse(line: &str) -> Self {
        let line = line.trim_end_matches(['\r', '\n']);
        let (id, rest) = match line.strip_prefix('#') {
            Some(tagged) => {
                let (id, rest) = tagged.split_once(' ').unwrap_or((tagged, ""));
                (id.parse().ok(), rest)
            }
            None => (None, line),
        };
        let (command, args) = rest.split_once(' ').unwrap_or((rest, ""));
        Self {
            id,
            command: command.to_owned(),
            args: args.to_owned(),
        }
    }
}

/// Encodes an outbound line, tagging it with a message id when one is given.
pub fn encode(id: Option<u32>, line: &str) -> String {
    match id {
        Some(id) => format!("#{id} {line}\n"),
        None => format!("{line}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tagged_line_with_spaces_in_args() {
        let raw = RawMessage::parse("#12 JOINBATTLEFAILED Battle is locked\r\n");
        assert_eq!(raw.id, Some(12));
        assert_eq!(raw.command, "JOINBATTLEFAILED");
        assert_eq!(raw.args, "Battle is locked");
    }

    #[test]
    fn parses_bare_command() {
        let raw = RawMessage::parse("LOGININFOEND");
        assert_eq!(
            raw,
            RawMessage {
                id: None,
                command: "LOGININFOEND".into(),
                args: String::new()
            }
        );
    }

    #[test]
    fn parses_extension_command_with_tabs() {
        let raw = RawMessage::parse("s.battle.queue_status 123\tAlice\tBob");
        assert_eq!(raw.command, "s.battle.queue_status");
        assert_eq!(raw.args, "123\tAlice\tBob");
    }

    #[test]
    fn encodes_with_and_without_id() {
        assert_eq!(encode(None, "PING"), "PING\n");
        assert_eq!(encode(Some(7), "JOINBATTLE 1 pw"), "#7 JOINBATTLE 1 pw\n");
    }
}
