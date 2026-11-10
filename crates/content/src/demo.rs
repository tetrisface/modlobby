//! Reading the start script, and the game's length, back out of a replay.
//!
//! A replay opens with a fixed header and the start script that made the game
//! — every modoption, the map, the teams and their start boxes. That is a
//! whole room setup, saved by the engine on every game anyone ever played, and
//! it is the only place to get one for a game you were not in.
//!
//! The layout is `rts/System/LoadSave/demofile.h`, which the engine documents
//! as stable for third parties. Only the fields before the script are read,
//! and only the first part of the file is decompressed: a replay is tens of
//! megabytes of demo stream after a start script that is a few kilobytes.

use std::io::Read;
use std::path::Path;

/// `DEMOFILE_MAGIC`, padded to 16 bytes.
const MAGIC: &[u8] = b"spring demofile";

/// Where each field we need sits, from the top of the header.
const VERSION_AT: usize = 16;
const HEADER_SIZE_AT: usize = 20;
/// `gameID`: sixteen bytes, all zero when the engine had none to write.
const GAME_ID_AT: usize = 280;
/// `unixTime`: when the game started, in seconds, as a `u64`.
const UNIX_TIME_AT: usize = 296;
const SCRIPT_SIZE_AT: usize = 304;
/// Everything up to and including `scriptSize`.
const NEEDED: usize = SCRIPT_SIZE_AT + 4;
/// `gameTime` and `wallclockTime`, in seconds. The engine fills them in at
/// game over (`CGame::GameEnd`); a game left before then keeps zeros.
const GAME_TIME_AT: usize = 312;
const WALLCLOCK_TIME_AT: usize = 316;
const LENGTH_NEEDED: usize = WALLCLOCK_TIME_AT + 4;

/// The engine writes version 5; anything else is a format we have not read.
const KNOWN_VERSION: i32 = 5;

/// A ceiling on what we will decompress looking for the script.
///
/// Start scripts are a few kilobytes even with twenty tweak slots of Lua in
/// them. This exists so a corrupt header claiming a gigabyte script cannot ask
/// us to allocate one.
const MOST_WE_WILL_READ: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("{0}")]
	Io(#[from] std::io::Error),
	#[error("not a replay: the file does not begin with a demo header")]
	NotADemo,
	#[error("replay format version {0}, which this does not read")]
	Version(i32),
	#[error("the header claims a start script of {0} bytes")]
	Impossible(i64),
	#[error("the replay ends before its start script does")]
	Truncated,
}

fn i32_at(bytes: &[u8], at: usize) -> i32 {
	i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Refuses what is not a header we read, or is too short to hold `needed`.
fn check(head: &[u8], needed: usize) -> Result<(), Error> {
	if head.len() < needed || !head.starts_with(MAGIC) {
		return Err(Error::NotADemo);
	}
	let version = i32_at(head, VERSION_AT);
	if version != KNOWN_VERSION {
		return Err(Error::Version(version));
	}
	Ok(())
}

/// How long a game ran, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Length {
	/// Time in the game: frames over 30. It stands still while paused, and
	/// covers the whole game even for someone who joined late and caught up.
	pub game: u32,
	/// Time on the clock, from the game starting to its game over.
	pub wall: u32,
}

impl Length {
	/// This length, if the game reached its game over: any game over takes
	/// some seconds on the clock, and a game nobody saw end keeps zeros.
	pub fn ended(self) -> Option<Self> {
		(self.wall > 0).then_some(self)
	}
}

/// The game's length from a replay's bytes.
pub fn length_from_bytes(head: &[u8]) -> Result<Length, Error> {
	check(head, LENGTH_NEEDED)?;
	let seconds = |at| u32::try_from(i32_at(head, at)).unwrap_or(0);
	Ok(Length {
		game: seconds(GAME_TIME_AT),
		wall: seconds(WALLCLOCK_TIME_AT),
	})
}

/// The game's length from a replay on disk. Only the header is decompressed,
/// which is what makes it cheap enough to ask of every replay in a list.
pub fn length(path: impl AsRef<Path>) -> Result<Length, Error> {
	let mut head = Vec::with_capacity(LENGTH_NEEDED);
	open(path.as_ref())?
		.take(LENGTH_NEEDED as u64)
		.read_to_end(&mut head)?;
	length_from_bytes(&head)
}

/// Where the start script sits: from `headerSize`, for `scriptSize` bytes.
fn script_span(head: &[u8]) -> Result<std::ops::Range<usize>, Error> {
	check(head, NEEDED)?;

	// `headerSize` is the header's own length and doubles as its minor
	// version, so the script starts there rather than at a fixed offset — that
	// is how the format was designed to grow.
	let header_size = i32_at(head, HEADER_SIZE_AT);
	let script_size = i32_at(head, SCRIPT_SIZE_AT);
	if header_size <= 0 || script_size <= 0 {
		return Err(Error::Impossible(i64::from(script_size)));
	}
	let (start, size) = (header_size as usize, script_size as usize);
	if size > MOST_WE_WILL_READ {
		return Err(Error::Impossible(i64::from(script_size)));
	}
	let end = start.checked_add(size).ok_or(Error::Truncated)?;
	Ok(start..end)
}

/// The start script from a replay's bytes.
pub fn script_from_bytes(head: &[u8]) -> Result<String, Error> {
	let span = script_span(head)?;
	let Some(script) = head.get(span) else {
		return Err(Error::Truncated);
	};

	// Trailing NULs: the engine writes the script with its terminator.
	let script = script.split(|byte| *byte == 0).next().unwrap_or(script);
	Ok(String::from_utf8_lossy(script).into_owned())
}

/// The start script from a replay on disk, compressed (`.sdfz`) or not.
pub fn script(path: impl AsRef<Path>) -> Result<String, Error> {
	head(path).map(|head| head.script)
}

/// What a replay says about its game before the demo stream begins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
	/// The engine's `GameID`; `None` when the header holds only zeros.
	pub game_id: Option<[u8; 16]>,
	/// When the game started, in seconds since the Unix epoch.
	pub started: u64,
	/// `None` for a game left before its game over, whose header keeps zeros.
	pub length: Option<Length>,
	pub script: String,
}

/// The head of a replay on disk, compressed (`.sdfz`) or not.
pub fn head(path: impl AsRef<Path>) -> Result<Head, Error> {
	head_from_reader(open(path.as_ref())?)
}

/// The head from a replay's bytes, reading exactly the header and the script
/// and none of the demo stream after them, which is what makes a replay large.
pub fn head_from_reader(mut reader: impl Read) -> Result<Head, Error> {
	let mut bytes = Vec::with_capacity(LENGTH_NEEDED);
	(&mut reader)
		.take(LENGTH_NEEDED as u64)
		.read_to_end(&mut bytes)?;
	check(&bytes, LENGTH_NEEDED)?;

	let end = script_span(&bytes)?.end;
	let rest = end.saturating_sub(bytes.len());
	reader.take(rest as u64).read_to_end(&mut bytes)?;

	let id: [u8; 16] = bytes[GAME_ID_AT..GAME_ID_AT + 16]
		.try_into()
		.expect("sixteen bytes");
	let started: [u8; 8] = bytes[UNIX_TIME_AT..UNIX_TIME_AT + 8]
		.try_into()
		.expect("eight bytes");
	Ok(Head {
		game_id: id.iter().any(|byte| *byte != 0).then_some(id),
		started: u64::from_le_bytes(started),
		length: length_from_bytes(&bytes)?.ended(),
		script: script_from_bytes(&bytes)?,
	})
}

/// A replay's bytes from the top, decompressed when it is an `.sdfz`.
fn open(path: &Path) -> Result<Box<dyn Read>, Error> {
	let file = std::fs::File::open(path)?;
	let compressed = path
		.extension()
		.is_some_and(|extension| extension.eq_ignore_ascii_case("sdfz"));
	Ok(if compressed {
		Box::new(flate2::read::GzDecoder::new(file))
	} else {
		Box::new(file)
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A header with a script glued on, laid out as the engine writes it.
	fn demo(script: &str, version: i32) -> Vec<u8> {
		let header_size = 352_i32;
		let mut bytes = vec![0_u8; header_size as usize];
		bytes[..MAGIC.len()].copy_from_slice(MAGIC);
		bytes[VERSION_AT..VERSION_AT + 4].copy_from_slice(&version.to_le_bytes());
		bytes[HEADER_SIZE_AT..HEADER_SIZE_AT + 4].copy_from_slice(&header_size.to_le_bytes());
		let size = script.len() as i32;
		bytes[SCRIPT_SIZE_AT..SCRIPT_SIZE_AT + 4].copy_from_slice(&size.to_le_bytes());
		bytes.extend_from_slice(script.as_bytes());
		bytes
	}

	#[test]
	fn the_script_is_read_from_where_the_header_says_it_starts() {
		let bytes = demo("[GAME]\n{\n\tMapName=Comet Catcher;\n}\n", KNOWN_VERSION);
		let script = script_from_bytes(&bytes).unwrap();
		assert!(script.contains("MapName=Comet Catcher"));
	}

	#[test]
	fn anything_that_is_not_a_replay_is_refused_rather_than_guessed_at() {
		assert!(matches!(
			script_from_bytes(b"PK\x03\x04 this is a zip"),
			Err(Error::NotADemo)
		));
		assert!(matches!(script_from_bytes(b"short"), Err(Error::NotADemo)));
	}

	#[test]
	fn a_format_we_have_not_read_says_so_instead_of_returning_nonsense() {
		let bytes = demo("[GAME]{}", 9);
		assert!(matches!(script_from_bytes(&bytes), Err(Error::Version(9))));
	}

	#[test]
	fn a_header_claiming_more_script_than_there_is_does_not_read_past_the_end() {
		let mut bytes = demo("[GAME]{}", KNOWN_VERSION);
		let huge = 1_000_000_i32.to_le_bytes();
		bytes[SCRIPT_SIZE_AT..SCRIPT_SIZE_AT + 4].copy_from_slice(&huge);
		assert!(matches!(script_from_bytes(&bytes), Err(Error::Truncated)));
	}

	#[test]
	fn a_header_claiming_an_absurd_script_is_refused_before_allocating_it() {
		let mut bytes = demo("[GAME]{}", KNOWN_VERSION);
		let absurd = i32::MAX.to_le_bytes();
		bytes[SCRIPT_SIZE_AT..SCRIPT_SIZE_AT + 4].copy_from_slice(&absurd);
		assert!(matches!(
			script_from_bytes(&bytes),
			Err(Error::Impossible(_))
		));
	}

	#[test]
	fn the_length_is_read_from_the_header() {
		let mut bytes = demo("[GAME]{}", KNOWN_VERSION);
		bytes[GAME_TIME_AT..GAME_TIME_AT + 4].copy_from_slice(&2_280_i32.to_le_bytes());
		bytes[WALLCLOCK_TIME_AT..WALLCLOCK_TIME_AT + 4].copy_from_slice(&2_400_i32.to_le_bytes());
		assert_eq!(
			length_from_bytes(&bytes).unwrap(),
			Length {
				game: 2_280,
				wall: 2_400
			}
		);
		assert!(matches!(length_from_bytes(b"short"), Err(Error::NotADemo)));
	}

	#[test]
	fn a_script_written_with_its_terminator_does_not_keep_it() {
		let bytes = demo("[GAME]{}\0", KNOWN_VERSION);
		assert_eq!(script_from_bytes(&bytes).unwrap(), "[GAME]{}");
	}

	/// Counts what is taken from the reader it wraps.
	struct Counting<R> {
		inner: R,
		taken: usize,
	}

	impl<R: Read> Read for Counting<R> {
		fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
			let read = self.inner.read(buf)?;
			self.taken += read;
			Ok(read)
		}
	}

	#[test]
	fn the_head_stops_reading_where_the_script_ends() {
		let script = "[GAME]\n{\n\tMapName=Comet Catcher;\n}\n";
		let mut bytes = demo(script, KNOWN_VERSION);
		let end = bytes.len();
		bytes.extend(std::iter::repeat_n(0xA5_u8, 10 * 1024 * 1024));
		let mut reader = Counting {
			inner: std::io::Cursor::new(bytes),
			taken: 0,
		};

		let head = head_from_reader(&mut reader).unwrap();

		assert_eq!(head.script, script);
		assert_eq!(reader.taken, end, "not a byte of the demo stream");
	}

	#[test]
	fn the_head_carries_the_game_id_and_when_it_started() {
		let mut bytes = demo("[GAME]{}", KNOWN_VERSION);
		let id: [u8; 16] = std::array::from_fn(|at| at as u8 + 1);
		bytes[GAME_ID_AT..GAME_ID_AT + 16].copy_from_slice(&id);
		bytes[UNIX_TIME_AT..UNIX_TIME_AT + 8].copy_from_slice(&1_762_000_000_u64.to_le_bytes());

		let head = head_from_reader(bytes.as_slice()).unwrap();

		assert_eq!(head.game_id, Some(id));
		assert_eq!(head.started, 1_762_000_000);
	}

	#[test]
	fn a_zero_game_id_is_no_id() {
		let head = head_from_reader(demo("[GAME]{}", KNOWN_VERSION).as_slice()).unwrap();
		assert_eq!(head.game_id, None);
	}

	#[test]
	fn the_head_has_a_length_only_for_a_game_that_reached_its_end() {
		let mut bytes = demo("[GAME]{}", KNOWN_VERSION);
		assert_eq!(head_from_reader(bytes.as_slice()).unwrap().length, None);

		bytes[GAME_TIME_AT..GAME_TIME_AT + 4].copy_from_slice(&2_280_i32.to_le_bytes());
		bytes[WALLCLOCK_TIME_AT..WALLCLOCK_TIME_AT + 4].copy_from_slice(&2_400_i32.to_le_bytes());
		assert_eq!(
			head_from_reader(bytes.as_slice()).unwrap().length,
			Some(Length {
				game: 2_280,
				wall: 2_400
			})
		);
	}

	#[test]
	fn a_head_cut_off_inside_its_script_is_truncated() {
		let mut bytes = demo("[GAME]{ MapName=X; }", KNOWN_VERSION);
		bytes.truncate(bytes.len() - 4);
		assert!(matches!(
			head_from_reader(bytes.as_slice()),
			Err(Error::Truncated)
		));
	}
}
