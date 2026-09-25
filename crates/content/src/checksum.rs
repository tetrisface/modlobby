//! The engine's checksum of a game or a map, computed the way its archive
//! scanner does, so a copy can be proved to be the room's before anything
//! loads it.
//!
//! An archive's checksum is over its files, not its container: every file
//! contributes `sha512(lowercased path) ^ sha512(contents)`, all of them
//! XORed together, leaving out dotfiles and whatever the archive's
//! `springignore.txt` names (`ArchiveScanner.cpp:959-970, 1040-1082`). So a
//! `.sdz`, a `.sd7` and an unpacked `.sdd` of the same files agree. An empty
//! file is the one difference the engine makes: it adds nothing from an
//! archive, and the hash of no bytes from a rapid package (see [`Empty`]).
//!
//! A *complete* checksum XORs in the archives it depends on, each once
//! (`ArchiveScanner.cpp:1702-1718`), and a room announces its first four
//! bytes as a `uint32`: the game's in `JOINBATTLE` (`ArchiveScanner.h:141`,
//! `unitsync.cpp:1170`), the map's in `BATTLEOPENED` and `UPDATEBATTLEINFO`.
//!
//! This matters because the engine does not refuse a copy that differs from
//! the host's: it logs a warning and the game desyncs later
//! (`PreGame.cpp:636-646`).

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use regex::Regex;
use sha2::{Digest as _, Sha512};

/// An engine checksum: SHA-512 sized.
pub type Checksum = [u8; 64];

/// What the engine adds to every game's dependencies
/// (`ArchiveScanner.h:122`, `ArchiveScanner.cpp:803`).
pub const SPRING_CONTENT: &str = "Spring content v1";

/// What the engine adds to every map's dependencies
/// (`ArchiveScanner.h:121`, `ArchiveScanner.cpp:795`).
pub const MAP_HELPER: &str = "Map Helper v1";

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("reading {path}: {reason}")]
	Read { path: String, reason: String },
	#[error("{0} is not an archive the engine loads")]
	NotAnArchive(String),
	#[error("{name}, which {needed_by} depends on, is not here")]
	Missing { name: String, needed_by: String },
}

/// The hash a room announces for its game or map, however its host wrote
/// the 32 bits: unsigned, or the same bits signed. `0` is unitsync's answer
/// when it could not tell (`GetMapChecksum`, `GetPrimaryModChecksum`), so it
/// announces nothing.
pub fn parse_room_hash(text: &str) -> Option<u32> {
	let value: i64 = text.trim().parse().ok()?;
	i32::try_from(value)
		.map(|signed| signed as u32)
		.or_else(|_| u32::try_from(value))
		.ok()
		.filter(|hash| *hash != 0)
}

/// What a room would announce for a game with this complete checksum.
pub fn room_hash(complete: &Checksum) -> u32 {
	u32::from_le_bytes([complete[0], complete[1], complete[2], complete[3]])
}

/// Which files a checksum leaves out: dotfiles, then `springignore.txt`'s
/// rules in order, the last one that matches deciding (`FileFilter.cpp`).
struct Ignore {
	rules: Vec<(Regex, bool)>,
}

impl Ignore {
	fn new(springignore: Option<&str>) -> Self {
		let mut rules = vec![(Regex::new(r"(?i)^\..*$").expect("a valid regex"), false)];
		for line in springignore.unwrap_or_default().lines() {
			let line = line.trim();
			if line.is_empty() || line.starts_with('#') {
				continue;
			}
			let (negate, glob) = match line.strip_prefix('!') {
				Some(rest) => (true, rest),
				None => (false, line),
			};
			if glob.is_empty() {
				continue;
			}
			if let Ok(regex) = Regex::new(&format!("(?i){}", glob_to_regex(glob))) {
				rules.push((regex, negate));
			}
		}
		Self { rules }
	}

	fn leaves_out(&self, name: &str) -> bool {
		self.rules.iter().fold(false, |left_out, (regex, negate)| {
			if regex.is_match(name) {
				!negate
			} else {
				left_out
			}
		})
	}
}

/// `FileFilter.cpp` `glob_to_regex`: a leading separator anchors at the
/// start, anything else matches whole path elements anywhere; `*` and `?`
/// never cross a separator, and every separator matches every other.
fn glob_to_regex(glob: &str) -> String {
	const SEPARATORS: &str = r"/\\:";
	let mut regex = String::new();
	let mut chars = glob.chars().peekable();
	if matches!(chars.peek(), Some('/' | '\\')) {
		regex.push('^');
		chars.next();
	} else {
		regex.push_str(&format!("(^|[{SEPARATORS}])"));
	}
	for c in chars {
		match c {
			'*' => regex.push_str(&format!("[^{SEPARATORS}]*")),
			'?' => regex.push_str(&format!("[^{SEPARATORS}]")),
			'/' | '\\' | ':' => regex.push_str(&format!("[{SEPARATORS}]")),
			c if c.is_ascii_alphanumeric() || c == '_' => regex.push(c),
			c => regex.push_str(&regex::escape(&c.to_string())),
		}
	}
	regex.push_str(&format!("([{SEPARATORS}]|$)"));
	regex
}

/// What an empty file adds, which the engine does not do alike everywhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Empty {
	/// Nothing: `IArchive::CalcHash` hashes no bytes when there are none,
	/// which is how a `.sdz`, `.sd7` or `.sdd` is summed.
	#[default]
	Nothing,
	/// The hash of no bytes: a rapid package's files are hashed as they are
	/// read out of the pool, empty or not (`PoolArchive.cpp:270`).
	Hashed,
}

/// One file's part: the hash of its lowercased path, XORed with the hash of
/// its contents -- for an empty file, as `empty` says -- and whether it was
/// empty.
fn part(name: &str, contents: &mut dyn Read, empty: Empty) -> std::io::Result<(Checksum, bool)> {
	let mut hasher = Sha512::new();
	let mut chunk = [0_u8; 64 * 1024];
	let mut any = false;
	loop {
		let read = contents.read(&mut chunk)?;
		if read == 0 {
			break;
		}
		any = true;
		hasher.update(&chunk[..read]);
	}
	let mut sum: Checksum = Sha512::digest(name.to_lowercase().as_bytes()).into();
	if any || empty == Empty::Hashed {
		let contents: Checksum = hasher.finalize().into();
		xor(&mut sum, &contents);
	}
	Ok((sum, !any))
}

/// A rapid package's parts, its pool objects read on as many threads as the
/// machine has, as the engine does (`for_mt`): a BAR build is thousands of
/// small gzipped files.
fn pooled_parts(files: &[(String, PathBuf)]) -> Result<Parts, String> {
	let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
	let chunk = files.len().div_ceil(threads).max(1);
	let summed: Vec<Result<Vec<FilePart>, String>> = std::thread::scope(|scope| {
		let running: Vec<_> = files
			.chunks(chunk)
			.map(|some| {
				scope.spawn(move || {
					some.iter()
						.map(|(name, object)| {
							let pooled = std::fs::File::open(object)
								.map_err(|err| format!("{name} is not in the pool ({err})"))?;
							let mut contents = flate2::read::GzDecoder::new(pooled);
							part(name, &mut contents, Empty::Hashed)
								.map(|(sum, empty)| (name.clone(), sum, empty))
								.map_err(|err| format!("{name}: {err}"))
						})
						.collect()
				})
			})
			.collect();
		running
			.into_iter()
			.map(|one| {
				one.join()
					.unwrap_or_else(|_| Err("a checksum thread failed".into()))
			})
			.collect()
	});
	let mut parts = Parts::default();
	for some in summed {
		parts.files.extend(some?);
	}
	if let Some((_, object)) = files
		.iter()
		.find(|(name, _)| name.eq_ignore_ascii_case("springignore.txt"))
	{
		let mut text = String::new();
		std::fs::File::open(object)
			.map(flate2::read::GzDecoder::new)
			.and_then(|mut contents| contents.read_to_string(&mut text))
			.map_err(|err| format!("springignore.txt: {err}"))?;
		parts.springignore = Some(text);
	}
	Ok(parts)
}

/// A file's name, its part of the checksum, and whether it was empty.
type FilePart = (String, Checksum, bool);

fn xor(into: &mut Checksum, other: &Checksum) {
	for (a, b) in into.iter_mut().zip(other) {
		*a ^= b;
	}
}

/// Every file of an archive, with its part and, for `springignore.txt`, its
/// text; the ignore rules can only be applied once that has been read, and
/// a `.sd7` can only be read once through.
#[derive(Default)]
struct Parts {
	files: Vec<FilePart>,
	springignore: Option<String>,
}

/// An archive's own checksum, and how many of the files in it are empty:
/// the one thing the engine sums differently between an archive and a rapid
/// package ([`Empty`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Summed {
	sum: Checksum,
	empties: usize,
}

impl Parts {
	fn add(&mut self, name: &str, contents: &mut dyn Read) -> std::io::Result<()> {
		if name.eq_ignore_ascii_case("springignore.txt") {
			let mut text = Vec::new();
			contents.read_to_end(&mut text)?;
			self.springignore = Some(String::from_utf8_lossy(&text).into_owned());
			let (part, empty) = part(name, &mut text.as_slice(), Empty::Nothing)?;
			self.files.push((name.to_owned(), part, empty));
			return Ok(());
		}
		let (part, empty) = part(name, contents, Empty::Nothing)?;
		self.files.push((name.to_owned(), part, empty));
		Ok(())
	}

	fn checksum(self) -> Summed {
		let ignore = Ignore::new(self.springignore.as_deref());
		let mut sum = [0; 64];
		let mut empties = 0;
		for (name, part, empty) in &self.files {
			if !ignore.leaves_out(name) {
				xor(&mut sum, part);
				empties += usize::from(*empty);
			}
		}
		Summed { sum, empties }
	}
}

/// One archive's own checksum, worked out once per file as it is: a base
/// archive or a large game is read once however many games or rooms ask.
/// Keyed by path, size and modification time, so a replaced file is read
/// again.
pub fn single(path: &Path) -> Result<Checksum, Error> {
	summed(path).map(|summed| summed.sum)
}

fn summed(path: &Path) -> Result<Summed, Error> {
	use std::collections::HashMap;
	use std::sync::{Mutex, OnceLock};
	use std::time::SystemTime;
	type Key = (PathBuf, u64, Option<SystemTime>);
	static SUMMED: OnceLock<Mutex<HashMap<Key, Summed>>> = OnceLock::new();

	let meta = std::fs::metadata(path).map_err(|err| Error::Read {
		path: path.display().to_string(),
		reason: err.to_string(),
	})?;
	let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
	let summed = SUMMED.get_or_init(Mutex::default);
	if let Some(sum) = summed.lock().ok().and_then(|held| held.get(&key).copied()) {
		return Ok(sum);
	}
	let sum = read_single(path)?;
	if let Ok(mut held) = summed.lock() {
		held.insert(key, sum);
	}
	Ok(sum)
}

/// One archive's own checksum, read: a `.sdz`, `.sd7`, unpacked `.sdd`, or
/// a rapid package (`.sdp`, its files in the pool beside it).
fn read_single(path: &Path) -> Result<Summed, Error> {
	let read = |reason: String| Error::Read {
		path: path.display().to_string(),
		reason,
	};
	let mut parts = Parts::default();
	let kind = path
		.extension()
		.and_then(|ext| ext.to_str())
		.map(str::to_ascii_lowercase);
	match kind.as_deref() {
		Some("sdz") => {
			let file = std::fs::File::open(path).map_err(|err| read(err.to_string()))?;
			let mut zip = zip::ZipArchive::new(file).map_err(|err| read(err.to_string()))?;
			for at in 0..zip.len() {
				let mut entry = zip.by_index(at).map_err(|err| read(err.to_string()))?;
				if entry.is_dir() {
					continue;
				}
				let name = entry.name().to_owned();
				parts
					.add(&name, &mut entry)
					.map_err(|err| read(err.to_string()))?;
			}
		}
		Some("sd7") => {
			let mut failed = None;
			sevenz_rust2::decompress_file_with_extract_fn(path, "", |entry, reader, _| {
				if entry.is_directory() {
					return Ok(true);
				}
				if let Err(err) = parts.add(entry.name(), reader) {
					failed = Some(err.to_string());
					return Ok(false);
				}
				Ok(true)
			})
			.map_err(|err| read(err.to_string()))?;
			if let Some(reason) = failed {
				return Err(read(reason));
			}
		}
		Some("sdd") => {
			let mut pending = vec![path.to_path_buf()];
			while let Some(dir) = pending.pop() {
				let entries = std::fs::read_dir(&dir).map_err(|err| read(err.to_string()))?;
				for entry in entries.filter_map(Result::ok) {
					let full = entry.path();
					if full.is_dir() {
						pending.push(full);
						continue;
					}
					let Ok(relative) = full.strip_prefix(path) else {
						continue;
					};
					let name = relative.to_string_lossy().replace('\\', "/");
					let mut file =
						std::fs::File::open(&full).map_err(|err| read(err.to_string()))?;
					parts
						.add(&name, &mut file)
						.map_err(|err| read(err.to_string()))?;
				}
			}
		}
		Some("sdp") => {
			let files = crate::archive::pooled_files(path).map_err(|err| read(err.to_string()))?;
			parts = pooled_parts(&files).map_err(read)?;
		}
		_ => return Err(Error::NotAnArchive(path.display().to_string())),
	}
	Ok(parts.checksum())
}

/// A game's or a map's complete checksum: its own, XORed with every archive
/// it uses ([`needs`]) and theirs, each once, found by name through `find`.
pub fn complete(game: &Path, find: &dyn Fn(&str) -> Option<PathBuf>) -> Result<Checksum, Error> {
	let mut sum = [0; 64];
	let mut seen = HashSet::new();
	let mut pending = vec![(game.to_path_buf(), game.display().to_string())];
	while let Some((path, name)) = pending.pop() {
		xor(&mut sum, &single(&path)?);
		for needed in needs(&path) {
			if !seen.insert(needed.to_lowercase()) {
				continue;
			}
			let found = find(&needed).ok_or_else(|| Error::Missing {
				name: needed.clone(),
				needed_by: name.clone(),
			})?;
			pending.push((found, needed));
		}
	}
	Ok(sum)
}

/// Whether an archive here is the one the room plays, by the hash the room
/// announced for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
	Matches,
	/// Ours and the room's, as the room would announce them.
	Differs {
		ours: u32,
		room: u32,
	},
	/// Could not be worked out, and why: a dependency that is not here, an
	/// archive that would not read.
	Unchecked(String),
}

/// `game` against the hash its room announced, its dependencies found
/// through `find`.
///
/// The same files match however the room's host keeps them: a game with an
/// odd number of empty files sums to one thing as an archive and to another,
/// by the hash of no bytes, as a rapid package ([`Empty`]), and a host on
/// rapid with a copy from a release is still playing the same game.
pub fn against_room(game: &Path, room: u32, find: &dyn Fn(&str) -> Option<PathBuf>) -> Verdict {
	let packaged_the_other_way = |sum: &Checksum| {
		summed(game).is_ok_and(|summed| room_hash(&other_packaging(sum, summed.empties)) == room)
	};
	match complete(game, find) {
		Ok(sum) if room_hash(&sum) == room || packaged_the_other_way(&sum) => Verdict::Matches,
		Ok(sum) => Verdict::Differs {
			ours: room_hash(&sum),
			room,
		},
		Err(err) => Verdict::Unchecked(err.to_string()),
	}
}

/// A mutator here against the checksum its room announced for it, as 128
/// hex digits. The engine hashes a room's game and map but loads its
/// mutators unchecked (`PreGame.cpp:264-269`), so this is the only check a
/// mutator gets. Its own files only: what it depends on is the game, which
/// the room's game hash already holds.
pub fn against_announced(mutator: &Path, announced: &str) -> Verdict {
	let Some(room) = announced_hash(announced) else {
		return Verdict::Unchecked(format!("the room announced {announced:?}, not a checksum"));
	};
	let ours = match summed(mutator) {
		Ok(ours) => ours,
		Err(err) => return Verdict::Unchecked(err.to_string()),
	};
	let same = |sum: &Checksum| crate::fetch::hex(sum) == announced;
	if same(&ours.sum) || same(&other_packaging(&ours.sum, ours.empties)) {
		return Verdict::Matches;
	}
	Verdict::Differs {
		ours: room_hash(&ours.sum),
		room,
	}
}

/// An announced checksum's first four bytes, as a room writes a hash; `None`
/// for anything but 128 lowercase hex digits.
pub fn announced_hash(hex: &str) -> Option<u32> {
	let valid = hex.len() == 128 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
	if !valid {
		return None;
	}
	let byte = |at: usize| u8::from_str_radix(&hex[at * 2..at * 2 + 2], 16).ok();
	Some(u32::from_le_bytes([byte(0)?, byte(1)?, byte(2)?, byte(3)?]))
}

/// `sum` as the same files packaged the other way would give it: an empty
/// file adds the hash of no bytes to a rapid package and nothing to an
/// archive ([`Empty`]), so only an odd number of them tells the two apart.
fn other_packaging(sum: &Checksum, empties: usize) -> Checksum {
	let mut other = *sum;
	if empties % 2 == 1 {
		xor(&mut other, &Sha512::digest(b"").into());
	}
	other
}

/// What an archive depends on, as the scanner reads it
/// (`ArchiveScanner.cpp:752-806`): a map its `mapinfo.lua`'s `depend` and
/// [`MAP_HELPER`]; a game or a menu its `modinfo.lua`'s and
/// [`SPRING_CONTENT`]; anything else with a `modinfo.lua` what that says; and
/// a map older than `mapinfo.lua`, known by its `.smf`, [`MAP_HELPER`] alone.
fn needs(path: &Path) -> Vec<String> {
	if let Some(mapinfo) = crate::map_name::mapinfo(path) {
		return with(crate::map_name::depends(&mapinfo), MAP_HELPER);
	}
	if let Some(modinfo) = crate::map_name::modinfo(path) {
		let needs = crate::map_name::depends(&modinfo);
		return match modtype(&modinfo) {
			0 | 1 | 5 => with(needs, SPRING_CONTENT),
			_ => needs,
		};
	}
	if crate::map_name::read(path).is_some_and(|says| says.smf.is_some()) {
		return vec![MAP_HELPER.to_owned()];
	}
	Vec::new()
}

/// `needs` with `implied` added, unless it already names it.
fn with(mut needs: Vec<String>, implied: &str) -> Vec<String> {
	if !needs.iter().any(|name| name.eq_ignore_ascii_case(implied)) {
		needs.push(implied.to_owned());
	}
	needs
}

/// `modtype = <n>` at the head of a line; 0, "hidden", when there is none.
fn modtype(modinfo: &str) -> u32 {
	modinfo
		.lines()
		.find_map(|line| {
			let line = line.split("--").next().unwrap_or(line).trim();
			let value = line
				.strip_prefix("modtype")?
				.trim_start()
				.strip_prefix('=')?;
			let digits: String = value
				.trim_start()
				.chars()
				.take_while(char::is_ascii_digit)
				.collect();
			digits.parse().ok()
		})
		.unwrap_or(0)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The engine's own base archive, from the 2026.09.01 engine.
	const MAPHELPER: &[u8] = include_bytes!("../testdata/maphelper.sdz");

	fn hex(sum: &Checksum) -> String {
		crate::fetch::hex(sum)
	}

	#[test]
	fn an_archive_sums_to_what_the_engine_recorded_for_it() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("maphelper.sdz");
		std::fs::write(&path, MAPHELPER).unwrap();
		// The engine's ArchiveCache22.lua, 2026-09-22.
		assert_eq!(
			hex(&single(&path).unwrap()),
			"6ecfd8e47b2c0bde27517e92bd171d1d036e0482cbfc929a1925b5239e4d5947ed335df18ba4775b16843a9affe42b5fcd2aad9957bacb63a48ae005c799524e"
		);
	}

	#[test]
	fn a_mutator_is_held_to_the_checksum_its_room_announced() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("maphelper.sdz");
		std::fs::write(&path, MAPHELPER).unwrap();
		let announced = hex(&single(&path).unwrap());
		assert!(announced.starts_with("6ecfd8e4"));
		assert_eq!(against_announced(&path, &announced), Verdict::Matches);

		let other = format!("00{}", &announced[2..]);
		assert_eq!(
			against_announced(&path, &other),
			Verdict::Differs {
				ours: u32::from_le_bytes([0x6e, 0xcf, 0xd8, 0xe4]),
				room: u32::from_le_bytes([0x00, 0xcf, 0xd8, 0xe4]),
			}
		);
		for not_one in [
			"",
			"6ecf",
			&announced.to_uppercase(),
			&format!("{announced}0"),
		] {
			assert!(
				matches!(against_announced(&path, not_one), Verdict::Unchecked(_)),
				"{not_one:?} is no checksum"
			);
		}
	}

	#[test]
	fn the_same_files_sum_alike_in_a_zip_and_in_a_folder() {
		let dir = tempfile::tempdir().unwrap();
		let sdd = dir.path().join("Same.sdd");
		std::fs::create_dir_all(sdd.join("units")).unwrap();
		std::fs::write(sdd.join("modinfo.lua"), "name = 'Same'\n").unwrap();
		std::fs::write(sdd.join("units").join("Tank.lua"), "return {}\n").unwrap();
		std::fs::write(sdd.join("empty.txt"), "").unwrap();

		let sdz = dir.path().join("Same.sdz");
		let mut zip = zip::ZipWriter::new(std::fs::File::create(&sdz).unwrap());
		let stored = zip::write::SimpleFileOptions::default();
		for (name, body) in [
			("modinfo.lua", "name = 'Same'\n"),
			("units/Tank.lua", "return {}\n"),
			("empty.txt", ""),
		] {
			zip.start_file(name, stored).unwrap();
			std::io::Write::write_all(&mut zip, body.as_bytes()).unwrap();
		}
		zip.add_directory("units/", stored).unwrap();
		zip.finish().unwrap();

		assert_eq!(single(&sdd).unwrap(), single(&sdz).unwrap());
	}

	#[test]
	fn a_rapid_package_sums_its_files_and_hashes_an_empty_one_as_the_engine_does() {
		use std::io::Write;
		let dir = tempfile::tempdir().unwrap();
		let files: [(&str, &[u8]); 3] = [
			("modinfo.lua", b"name = 'Rapid'\n"),
			("units/a.lua", b"return 1\n"),
			("units/empty.lua", b""),
		];

		// The package index: `<len><name><md5><crc32><size>`, gzipped; the
		// pool object for each is its contents, gzipped, under its md5.
		let mut index = Vec::new();
		for (name, body) in files {
			let md5 = <md5::Md5 as md5::Digest>::digest(body);
			index.push(name.len() as u8);
			index.extend_from_slice(name.as_bytes());
			index.extend_from_slice(&md5);
			index.extend_from_slice(&[0; 8]);
			let hex = crate::fetch::hex(&md5);
			let pool = dir.path().join("pool").join(&hex[..2]);
			std::fs::create_dir_all(&pool).unwrap();
			let mut gz = flate2::write::GzEncoder::new(
				std::fs::File::create(pool.join(format!("{}.gz", &hex[2..]))).unwrap(),
				flate2::Compression::default(),
			);
			gz.write_all(body).unwrap();
			gz.finish().unwrap();
		}
		std::fs::create_dir_all(dir.path().join("packages")).unwrap();
		let sdp = dir.path().join("packages").join("abc.sdp");
		let mut gz = flate2::write::GzEncoder::new(
			std::fs::File::create(&sdp).unwrap(),
			flate2::Compression::default(),
		);
		gz.write_all(&index).unwrap();
		gz.finish().unwrap();

		let sdd = dir.path().join("Rapid.sdd");
		for (name, body) in files {
			let path = sdd.join(name);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, body).unwrap();
		}
		// The same files, but the package hashes its empty one where a folder
		// adds nothing for it (`PoolArchive.cpp:270` against `IArchive.cpp:44`).
		let mut expected = single(&sdd).unwrap();
		xor(&mut expected, &Sha512::digest(b"").into());
		assert_eq!(single(&sdp).unwrap(), expected);
	}

	#[test]
	fn dotfiles_and_what_springignore_names_are_left_out_the_way_the_engine_does() {
		let ignore = Ignore::new(Some("# comment\n.git\n/.svn\n*.tmp\n!keep.tmp\n/docs/*\n"));
		for left_out in [
			".hidden",
			".git/config",
			"sub/.git/HEAD",
			".svn/x",
			"a/b.tmp",
			"docs/readme.md",
			// `*` stops at a separator, but a rule may end at any separator.
			"docs/deeper/readme.md",
		] {
			assert!(ignore.leaves_out(left_out), "{left_out}");
		}
		for kept in [
			"sub/.svn/x",
			"keep.tmp",
			"units/tank.lua",
			"sub/docs/readme.md",
			"a.tmpx",
		] {
			assert!(!ignore.leaves_out(kept), "{kept}");
		}
	}

	#[test]
	fn a_game_sums_with_what_it_depends_on_each_once() {
		let dir = tempfile::tempdir().unwrap();
		let write = |name: &str, modinfo: &str| {
			let path = dir.path().join(name);
			std::fs::create_dir_all(&path).unwrap();
			std::fs::write(path.join("modinfo.lua"), modinfo).unwrap();
			path
		};
		let game = write(
			"Game.sdd",
			"name = 'Game'\nversion = '1'\nmodtype = 1\ndepend = { 'Lib v1', 'Spring content v1' }\n",
		);
		let lib = write("Lib.sdd", "name = 'Lib'\nversion = 'v1'\nmodtype = 0\n");
		let content = write(
			"Content.sdd",
			"name = 'Spring content'\nversion = 'v1'\nmodtype = 4\ndepend = { 'Spring Bitmaps' }\n",
		);
		let bitmaps = write("Bitmaps.sdd", "name = 'Spring Bitmaps'\nmodtype = 4\n");
		let by_name = |name: &str| match name {
			"Lib v1" => Some(lib.clone()),
			"Spring content v1" => Some(content.clone()),
			"Spring Bitmaps" => Some(bitmaps.clone()),
			_ => None,
		};

		let mut expected = [0; 64];
		for path in [&game, &lib, &content, &bitmaps] {
			xor(&mut expected, &single(path).unwrap());
		}
		assert_eq!(complete(&game, &by_name).unwrap(), expected);

		let orphan = write("Orphan.sdd", "name = 'Orphan'\ndepend = { 'Nowhere v2' }\n");
		assert!(matches!(
			complete(&orphan, &by_name),
			Err(Error::Missing { name, .. }) if name == "Nowhere v2"
		));
	}

	#[test]
	fn a_game_is_the_rooms_copy_only_when_its_hash_is_the_rooms() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path().join("Game.sdd");
		std::fs::create_dir(&game).unwrap();
		std::fs::write(
			game.join("modinfo.lua"),
			"name = 'Game'
modtype = 1
",
		)
		.unwrap();
		let content = dir.path().join("Content.sdd");
		std::fs::create_dir(&content).unwrap();
		std::fs::write(
			content.join("modinfo.lua"),
			"name = 'Spring content'
version = 'v1'
modtype = 4
",
		)
		.unwrap();
		let find = |name: &str| (name == SPRING_CONTENT).then(|| content.clone());

		let ours = room_hash(&complete(&game, &find).unwrap());
		assert_eq!(against_room(&game, ours, &find), Verdict::Matches);
		assert_eq!(
			against_room(&game, ours ^ 1, &find),
			Verdict::Differs {
				ours,
				room: ours ^ 1
			}
		);
		assert!(matches!(
			against_room(&game, ours, &|_| None),
			Verdict::Unchecked(_)
		));
	}

	/// Checked live: Carrot Mountains v2.0's own checksum XOR maphelper.sdz's
	/// is the `6a344023…` the engine logged as the host's (2026-09-22).
	#[test]
	fn a_map_needs_the_map_helper_and_what_its_mapinfo_names() {
		let dir = tempfile::tempdir().unwrap();
		let write = |name: &str, file: &str, text: &str| {
			let path = dir.path().join(name);
			std::fs::create_dir_all(path.join("maps")).unwrap();
			std::fs::write(path.join(file), text).unwrap();
			path
		};
		let map = write(
			"Map.sdd",
			"mapinfo.lua",
			"name = 'Map'\ndepend = { 'Lib v1' }\n",
		);
		let old = write("Old.sdd", "maps/Old.smf", "not really a map");
		let lib = write(
			"Lib.sdd",
			"modinfo.lua",
			"name = 'Lib'\nversion = 'v1'\nmodtype = 4\n",
		);
		let helper = write(
			"Helper.sdd",
			"modinfo.lua",
			"name = 'Map Helper'\nversion = 'v1'\nmodtype = 4\n",
		);
		let by_name = |name: &str| match name {
			"Lib v1" => Some(lib.clone()),
			MAP_HELPER => Some(helper.clone()),
			_ => None,
		};
		let summed = |paths: &[&PathBuf]| {
			let mut sum = [0; 64];
			for path in paths {
				xor(&mut sum, &single(path).unwrap());
			}
			sum
		};

		assert_eq!(
			complete(&map, &by_name).unwrap(),
			summed(&[&map, &lib, &helper])
		);
		assert_eq!(complete(&old, &by_name).unwrap(), summed(&[&old, &helper]));
	}

	#[test]
	fn the_same_files_are_the_rooms_copy_whether_its_host_has_them_packed_or_on_rapid() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path().join("Game.sdd");
		std::fs::create_dir(&game).unwrap();
		std::fs::write(game.join("modinfo.lua"), "name = 'Game'\nmodtype = 4\n").unwrap();
		std::fs::write(game.join("empty.lua"), "").unwrap();
		let find = |_: &str| None;

		// A host whose copy is a rapid package hashes the empty file.
		let mut on_rapid = complete(&game, &find).unwrap();
		xor(&mut on_rapid, &Sha512::digest(b"").into());
		assert_eq!(
			against_room(&game, room_hash(&on_rapid), &find),
			Verdict::Matches
		);
	}

	#[test]
	fn a_room_hash_is_the_first_four_bytes_whichever_way_it_was_written() {
		// SplinterFaction 0.1.86's complete checksum, as the engine logged
		// it joining a room on lobby.recoilengine.org (2026-09-22).
		let mut logged = [0; 64];
		logged[..4].copy_from_slice(&[0x71, 0xf7, 0xab, 0x5a]);
		assert_eq!(room_hash(&logged), 1_521_219_441);

		assert_eq!(parse_room_hash("1521219441"), Some(1_521_219_441));
		assert_eq!(parse_room_hash(" -1 "), Some(u32::MAX));
		assert_eq!(parse_room_hash("4294967295"), Some(u32::MAX));
		assert_eq!(parse_room_hash("4294967296"), None);
		assert_eq!(parse_room_hash("abc"), None);
		assert_eq!(parse_room_hash("0"), None);
	}
}
