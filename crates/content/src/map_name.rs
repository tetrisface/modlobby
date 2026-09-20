//! The name a map calls itself, read out of its own archive.
//!
//! A room names its map the way the engine does -- `FrostyCove v1.13` -- and
//! nothing about the file on the disk says what that is. For BAR's own maps
//! the published index answers it. For a map nobody publishes, which is every
//! custom map on a LAN, the only place it is written down is the `mapinfo.lua`
//! inside the archive.
//!
//! Getting this wrong is not a cosmetic matter. Handed a name that is not a
//! map's own, the engine looks for an archive under it, does not find one, and
//! refuses to start the game: `Dependent archive "frostycove_v1.13" (resolved
//! to "FrostyCove_v1.13") not found` (`ArchiveScanner.cpp:1581`).

use std::io::Read;
use std::path::Path;

/// How much of a `mapinfo.lua` is read. The name is in the first few lines;
/// past this it is something else wearing the name.
const MOST: usize = 256 * 1024;

/// The map's own name, as the engine would compose it, or `None` when the
/// archive does not say.
pub fn of_archive(path: &Path) -> Option<String> {
	let lua = read_mapinfo(path)?;
	compose(&lua)
}

/// `name` and `version` put together the way `ArchiveScanner.cpp:192` does:
/// a version that is not already part of the name is appended after a space.
pub fn compose(mapinfo: &str) -> Option<String> {
	let name = field(mapinfo, "name")?;
	if name.is_empty() {
		return None;
	}
	let version = field(mapinfo, "version").unwrap_or_default();
	if version.is_empty() || name.contains(&version) {
		return Some(name);
	}
	Some(format!("{name} {version}"))
}

/// The first `<key> = "<value>"` at the head of a line.
///
/// By line rather than by pattern over the whole file, so `shortname` is not
/// read as `name` and a mention further down does not win over the
/// declaration. Lua's own comment marker ends a line here as it does there.
fn field(mapinfo: &str, key: &str) -> Option<String> {
	mapinfo.lines().find_map(|line| {
		let line = line.split("--").next().unwrap_or(line).trim();
		let rest = line.strip_prefix(key)?.trim_start();
		let value = rest.strip_prefix('=')?.trim_start();
		let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
		let body = &value[1..];
		Some(body[..body.find(quote)?].to_owned())
	})
}

/// `mapinfo.lua` out of a `.sd7` (7z) or `.sdz` (zip), whichever this is.
fn read_mapinfo(path: &Path) -> Option<String> {
	let extension = path.extension()?.to_str()?.to_ascii_lowercase();
	let raw = match extension.as_str() {
		"sd7" => from_sd7(path),
		"sdz" => from_sdz(path),
		// An unpacked map is a directory, and its mapinfo.lua is just a file.
		"sdd" => std::fs::read(path.join("mapinfo.lua")).ok(),
		_ => None,
	}?;
	String::from_utf8(raw).ok()
}

fn from_sd7(path: &Path) -> Option<Vec<u8>> {
	let mut found = None;
	sevenz_rust2::decompress_file_with_extract_fn(path, "", |entry, reader, _| {
		if !entry.name().eq_ignore_ascii_case("mapinfo.lua") {
			return Ok(true);
		}
		let mut held = Vec::new();
		reader.take(MOST as u64).read_to_end(&mut held)?;
		found = Some(held);
		// Nothing else in the archive is wanted, and a map is a large thing
		// to keep reading past the one file that was.
		Ok(false)
	})
	.ok()?;
	found
}

fn from_sdz(path: &Path) -> Option<Vec<u8>> {
	let file = std::fs::File::open(path).ok()?;
	let mut zip = zip::ZipArchive::new(file).ok()?;
	let at = (0..zip.len()).find(|index| {
		zip.by_index(*index)
			.is_ok_and(|entry| entry.name().eq_ignore_ascii_case("mapinfo.lua"))
	})?;
	let mut held = Vec::new();
	zip.by_index(at)
		.ok()?
		.take(MOST as u64)
		.read_to_end(&mut held)
		.ok()?;
	Some(held)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The head of a real one, which is where the trouble was.
	const FROSTYCOVE: &str = r#"
local mapinfo = {
	name        = "FrostyCove",
	shortname   = "Frosty",
	description = "Frozen Wonderland",
	author      = "PetTurtle",
	version     = "v1.13",
	--mutator   = "deployment";
	--mapfile   = "", --// location of smf/sm3 file (optional)
	modtype     = 3,
}
"#;

	#[test]
	fn the_name_is_the_one_the_engine_composes() {
		assert_eq!(compose(FROSTYCOVE).as_deref(), Some("FrostyCove v1.13"));
	}

	/// `shortname` and `mapfile` both contain a shorter key; neither may be
	/// read as it, and a commented-out line is not a declaration at all.
	#[test]
	fn a_longer_key_and_a_comment_are_not_the_field() {
		assert_eq!(field(FROSTYCOVE, "name").as_deref(), Some("FrostyCove"));
		assert_eq!(field(FROSTYCOVE, "version").as_deref(), Some("v1.13"));
		assert_eq!(field(FROSTYCOVE, "mapfile"), None, "that line is a comment");
		assert_eq!(field(FROSTYCOVE, "author").as_deref(), Some("PetTurtle"));
	}

	#[test]
	fn a_version_already_in_the_name_is_not_said_twice() {
		let held = r#"name = "Comet Catcher Remake 1.8"
version = "1.8""#;
		assert_eq!(
			compose(held).as_deref(),
			Some("Comet Catcher Remake 1.8"),
			"the engine warns and leaves it alone"
		);
	}

	#[test]
	fn a_map_that_says_nothing_is_not_guessed_at() {
		assert_eq!(compose("modtype = 3"), None);
		assert_eq!(compose(r#"name = """#), None);
		// No version is a name on its own, which is legal.
		assert_eq!(compose(r#"name = "Bare""#).as_deref(), Some("Bare"));
	}

	#[test]
	fn single_quotes_are_lua_too() {
		assert_eq!(
			compose("name = 'Quoted'\nversion = 'v2'").as_deref(),
			Some("Quoted v2")
		);
	}
}
