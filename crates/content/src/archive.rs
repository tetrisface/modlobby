//! Reading one file back out of an installed game.
//!
//! Rapid does not keep a game as an archive on disk. It keeps a package index
//! at `packages/<md5>.sdp` listing every file with its own hash, and the
//! contents in a shared pool at `pool/<hh>/<rest>.gz`, so two versions sharing
//! a file store it once. Getting `modoptions.lua` out therefore means reading
//! the index, finding the entry, and decompressing one pool object.
//!
//! Both formats are pr-downloader's, which is what wrote them:
//!
//! - The `.sdp` is gzip, holding records of `<u8 name length><name><16 byte
//!   md5><4 byte crc32><4 byte size>` until end of file
//!   (`FileSystem.cpp:parseSdp`).
//! - A pool object is `pool/<first two hex chars>/<remaining thirty>.gz`
//!   (`FileSystem.cpp:getPoolFilename`), gzip again.
//!
//! Reading these directly is what lets the app use the game's own data — the
//! modoption table with its real names and descriptions — without shipping a
//! copy of it. bar-lobby does the same thing through its own archive layer
//! (`game-provider.ts:200`); Chobby asks the engine's Lua VM.

use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;

/// A file's worth of index record, before the name and hash.
const MD5_BYTES: usize = 16;
const RECORD_TAIL: usize = MD5_BYTES + 4 + 4;

/// A ceiling on one file read out of a pool object.
///
/// `modoptions.lua` is about 100 KB. This is generous enough for any Lua in a
/// game archive and small enough that a corrupt length cannot ask for a
/// gigabyte.
const MOST_WE_WILL_READ: u64 = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0} is not a readable rapid package")]
    Malformed(PathBuf),
    #[error("{file} is not in this game")]
    Missing { file: String },
    #[error("the pool is missing the contents of {file}")]
    NotPooled { file: String },
}

/// Every file the package lists, as name and content hash.
///
/// The hash is hex, which is how the pool is keyed.
fn index(sdp: &Path) -> Result<Vec<(String, String)>, Error> {
    let mut bytes = Vec::new();
    GzDecoder::new(std::fs::File::open(sdp)?)
        .take(MOST_WE_WILL_READ)
        .read_to_end(&mut bytes)?;

    let mut files = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let length = usize::from(bytes[at]);
        at += 1;
        let end = at + length + RECORD_TAIL;
        if length == 0 || end > bytes.len() {
            return Err(Error::Malformed(sdp.to_path_buf()));
        }
        let name = String::from_utf8_lossy(&bytes[at..at + length]).into_owned();
        let md5 = &bytes[at + length..at + length + MD5_BYTES];
        files.push((name, md5.iter().map(|byte| format!("{byte:02x}")).collect()));
        at = end;
    }
    Ok(files)
}

/// Where the pool keeps the contents of a file with this hash.
fn pooled(data_dir: &Path, md5: &str) -> PathBuf {
    let (head, rest) = md5.split_at(2.min(md5.len()));
    data_dir.join("pool").join(head).join(format!("{rest}.gz"))
}

/// One file's contents, out of the rapid package with this md5.
pub fn from_package(data_dir: &Path, package_md5: &str, file: &str) -> Result<Vec<u8>, Error> {
    let sdp = data_dir.join("packages").join(format!("{package_md5}.sdp"));
    let wanted = file.to_ascii_lowercase();
    let (_, content_md5) = index(&sdp)?
        .into_iter()
        // Archive names are compared without case: the engine's own virtual
        // file system is case-insensitive, and games are not consistent.
        .find(|(name, _)| name.to_ascii_lowercase() == wanted)
        .ok_or_else(|| Error::Missing {
            file: file.to_owned(),
        })?;

    let object = pooled(data_dir, &content_md5);
    let handle = std::fs::File::open(&object).map_err(|_| Error::NotPooled {
        file: file.to_owned(),
    })?;
    let mut contents = Vec::new();
    GzDecoder::new(handle)
        .take(MOST_WE_WILL_READ)
        .read_to_end(&mut contents)?;
    Ok(contents)
}

/// The same file from an unpacked game directory, for a game built locally.
///
/// `games/*.sdd` is how a developer runs a checkout of the game itself, and
/// those never appear in rapid. The first directory holding the file wins;
/// there is normally only one.
pub fn from_unpacked(data_dir: &Path, file: &str) -> Option<Vec<u8>> {
    let games = std::fs::read_dir(data_dir.join("games")).ok()?;
    for entry in games.flatten() {
        let candidate = entry.path().join(file);
        if let Ok(bytes) = std::fs::read(&candidate) {
            return Some(bytes);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Writes a package and its pool objects, laid out as rapid does.
    fn install(dir: &Path, files: &[(&str, &[u8])]) -> String {
        std::fs::create_dir_all(dir.join("packages")).unwrap();
        let mut index = Vec::new();
        for (name, contents) in files {
            // The pool is keyed by the content's hash; any stable stand-in
            // does for a test, since nothing here verifies it.
            let md5: [u8; 16] = std::array::from_fn(|i| {
                name.as_bytes()
                    .get(i)
                    .copied()
                    .unwrap_or(contents.len() as u8)
            });
            let hex: String = md5.iter().map(|byte| format!("{byte:02x}")).collect();

            index.push(name.len() as u8);
            index.extend_from_slice(name.as_bytes());
            index.extend_from_slice(&md5);
            index.extend_from_slice(&[0, 0, 0, 0]);
            index.extend_from_slice(&(contents.len() as u32).to_be_bytes());

            let object = pooled(dir, &hex);
            std::fs::create_dir_all(object.parent().unwrap()).unwrap();
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gz.write_all(contents).unwrap();
            std::fs::write(object, gz.finish().unwrap()).unwrap();
        }

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&index).unwrap();
        let package = "abc123";
        std::fs::write(
            dir.join("packages").join(format!("{package}.sdp")),
            gz.finish().unwrap(),
        )
        .unwrap();
        package.to_owned()
    }

    #[test]
    fn a_file_comes_back_out_of_the_package_and_the_pool() {
        let dir = tempfile::tempdir().unwrap();
        let package = install(
            dir.path(),
            &[
                ("modoptions.lua", b"return { { key = 'x' } }"),
                ("luaai.lua", b"return {}"),
            ],
        );
        let got = from_package(dir.path(), &package, "modoptions.lua").unwrap();
        assert_eq!(got, b"return { { key = 'x' } }");
    }

    #[test]
    fn the_name_is_matched_without_case_as_the_engine_does() {
        let dir = tempfile::tempdir().unwrap();
        let package = install(dir.path(), &[("ModOptions.lua", b"upper")]);
        assert_eq!(
            from_package(dir.path(), &package, "modoptions.lua").unwrap(),
            b"upper"
        );
    }

    #[test]
    fn a_file_the_game_does_not_have_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let package = install(dir.path(), &[("modinfo.lua", b"x")]);
        assert!(matches!(
            from_package(dir.path(), &package, "modoptions.lua"),
            Err(Error::Missing { .. })
        ));
    }

    #[test]
    fn contents_missing_from_the_pool_are_told_apart_from_a_missing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let package = install(dir.path(), &[("modoptions.lua", b"x")]);
        std::fs::remove_dir_all(dir.path().join("pool")).unwrap();
        assert!(matches!(
            from_package(dir.path(), &package, "modoptions.lua"),
            Err(Error::NotPooled { .. })
        ));
    }

    #[test]
    fn a_truncated_index_is_refused_rather_than_read_past() {
        let dir = tempfile::tempdir().unwrap();
        let package = install(dir.path(), &[("modoptions.lua", b"x")]);
        // A record that claims a longer name than the file holds.
        let path = dir.path().join("packages").join(format!("{package}.sdp"));
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&[200, b'a', b'b']).unwrap();
        std::fs::write(&path, gz.finish().unwrap()).unwrap();
        assert!(matches!(
            from_package(dir.path(), &package, "modoptions.lua"),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn an_unpacked_game_is_read_straight_off_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("games").join("Beyond-All-Reason.sdd");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("modoptions.lua"), b"local x").unwrap();
        assert_eq!(
            from_unpacked(dir.path(), "modoptions.lua").unwrap(),
            b"local x"
        );
    }
}
