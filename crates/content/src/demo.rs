//! Reading the start script back out of a replay.
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
const SCRIPT_SIZE_AT: usize = 304;
/// Everything up to and including `scriptSize`.
const NEEDED: usize = SCRIPT_SIZE_AT + 4;

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

/// The start script from a replay's bytes.
pub fn script_from_bytes(head: &[u8]) -> Result<String, Error> {
    if head.len() < NEEDED || !head.starts_with(MAGIC) {
        return Err(Error::NotADemo);
    }
    let version = i32_at(head, VERSION_AT);
    if version != KNOWN_VERSION {
        return Err(Error::Version(version));
    }

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
    if head.len() < end {
        return Err(Error::Truncated);
    }

    // Trailing NULs: the engine writes the script with its terminator.
    let script = &head[start..end];
    let script = script.split(|byte| *byte == 0).next().unwrap_or(script);
    Ok(String::from_utf8_lossy(script).into_owned())
}

/// The start script from a replay on disk, compressed (`.sdfz`) or not.
pub fn script(path: impl AsRef<Path>) -> Result<String, Error> {
    let path = path.as_ref();
    let file = std::fs::File::open(path)?;
    let compressed = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("sdfz"));

    // Only the front of the file: everything after the script is the demo
    // stream, which is the part that makes these files large.
    let mut head = Vec::new();
    let mut reader: Box<dyn Read> = if compressed {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    reader
        .by_ref()
        .take(MOST_WE_WILL_READ as u64)
        .read_to_end(&mut head)?;
    script_from_bytes(&head)
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
    fn a_script_written_with_its_terminator_does_not_keep_it() {
        let bytes = demo("[GAME]{}\0", KNOWN_VERSION);
        assert_eq!(script_from_bytes(&bytes).unwrap(), "[GAME]{}");
    }
}
