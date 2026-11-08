//! The welcome teiserver's Coordinator sends whoever joins a room.
//!
//! On every join the room's consul sends the joiner one bundle of private
//! messages: a line of hashes, the room's welcome message one line per `$$`,
//! its play restrictions, and the line of hashes again (`consul_server.ex`,
//! `handle_info` for `:add_user`). Nothing else it sends is framed that way,
//! so the hashes are what marks the bundle.

/// Who sends it: teiserver's system account, which has this name on every
/// teiserver (`account.ex` `system_user`).
pub const COORDINATOR: &str = "Coordinator";

/// The line either side of the bundle (`consul_server.ex` `@splitter`).
const SPLITTER: &str = "########################################";

/// Where the room's own words end and its play restrictions begin
/// (`lobby_restrictions.ex` `get_lobby_restrictions_welcome_text`). A
/// restriction alone is no welcome: the room's title carries it already.
const RESTRICTIONS: &str = "This lobby has the following play restrictions:";

/// A bundle being read, one private message at a time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reader {
	open: Option<Vec<String>>,
}

/// What one message turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Read {
	/// Not part of a bundle.
	Outside,
	/// Part of a bundle still arriving.
	Inside,
	/// A bundle's closing line, and the room's message out of it: a line per
	/// paragraph, empty when the room has none.
	Done(Vec<String>),
}

impl Reader {
	pub fn read(&mut self, text: &str) -> Read {
		let splitter = text.trim() == SPLITTER;
		match (self.open.take(), splitter) {
			(None, false) => Read::Outside,
			(None, true) => {
				self.open = Some(Vec::new());
				Read::Inside
			}
			(Some(mut lines), false) => {
				lines.push(text.to_owned());
				self.open = Some(lines);
				Read::Inside
			}
			(Some(lines), true) => Read::Done(message(lines)),
		}
	}
}

/// The room's own words: what comes before its restrictions, without the
/// blank lines a `$$$$` leaves.
fn message(lines: Vec<String>) -> Vec<String> {
	lines
		.into_iter()
		.map(|line| line.trim().to_owned())
		.take_while(|line| line != RESTRICTIONS)
		.filter(|line| !line.is_empty())
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn read_all(reader: &mut Reader, lines: &[&str]) -> Vec<Read> {
		lines.iter().map(|line| reader.read(line)).collect()
	}

	#[test]
	fn a_welcome_is_the_lines_between_the_hashes() {
		let mut reader = Reader::default();
		let read = read_all(
			&mut reader,
			&[
				SPLITTER,
				"This lobby was created using https://nuttyb-community.github.io/nuttyb/configurator.",
				SPLITTER,
			],
		);
		assert_eq!(
			read,
			[
				Read::Inside,
				Read::Inside,
				Read::Done(vec![
					"This lobby was created using https://nuttyb-community.github.io/nuttyb/configurator."
						.into()
				]),
			]
		);
	}

	#[test]
	fn the_play_restrictions_are_left_to_the_title() {
		let mut reader = Reader::default();
		let read = read_all(
			&mut reader,
			&[SPLITTER, "Be nice", RESTRICTIONS, "Max chev: 3", SPLITTER],
		);
		assert_eq!(read.last(), Some(&Read::Done(vec!["Be nice".into()])));
	}

	#[test]
	fn restrictions_alone_are_no_welcome() {
		let mut reader = Reader::default();
		let read = read_all(
			&mut reader,
			&[SPLITTER, RESTRICTIONS, "Min chev: 2", SPLITTER],
		);
		assert_eq!(read.last(), Some(&Read::Done(vec![])));
	}

	#[test]
	fn blank_paragraphs_are_dropped() {
		let mut reader = Reader::default();
		let read = read_all(
			&mut reader,
			&[SPLITTER, "Rules:", "", "  no rush  ", SPLITTER],
		);
		assert_eq!(
			read.last(),
			Some(&Read::Done(vec!["Rules:".into(), "no rush".into()]))
		);
	}

	#[test]
	fn a_message_outside_a_bundle_is_not_one() {
		let mut reader = Reader::default();
		assert_eq!(
			reader.read("Setting tweakdefs requires boss privileges"),
			Read::Outside
		);
	}

	#[test]
	fn the_next_bundle_starts_afresh() {
		let mut reader = Reader::default();
		read_all(&mut reader, &[SPLITTER, "first", SPLITTER]);
		assert_eq!(reader.read("a refusal"), Read::Outside);
		let read = read_all(&mut reader, &[SPLITTER, "second", SPLITTER]);
		assert_eq!(read.last(), Some(&Read::Done(vec!["second".into()])));
	}
}
