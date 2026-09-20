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

/// What an archive says about itself, in the two places a map ever says it.
pub struct Says {
	/// `mapinfo.lua`, where a map made this century writes its name.
	pub mapinfo: Option<String>,
	/// The base name of its `.smf`, which is the name of a map made before
	/// `mapinfo.lua` existed -- every classic still on springfiles.
	pub smf: Option<String>,
}

/// The map's own name, as the engine would arrive at it, or `None` when the
/// archive says neither.
///
/// `mapinfo.lua` first (`ArchiveScanner.cpp:192`), then the `.smf` file's
/// base name, which is what the scanner falls back to for an archive with no
/// `mapinfo.lua` at all (`ArchiveScanner.cpp:788`). Case included: a name
/// that differs from the real one only in case is as unusable as one that
/// differs entirely.
pub fn of_archive(path: &Path) -> Option<String> {
	let says = read(path)?;
	says.mapinfo
		.as_deref()
		.and_then(compose)
		.or_else(|| says.smf.clone())
}

/// What the map says it needs loaded beside it, as `mapinfo.lua` lists them.
///
/// Worth looking at because the engine takes the list literally: an entry of
/// `""`, which a common template ships and which at least one custom map is
/// still carrying, becomes a dependency on an archive with no name
/// (`ArchiveScanner.cpp:169`). The game then stops with `Dependent archive ""
/// (resolved to "") not found`, which names nothing and blames nobody.
pub fn dependencies(path: &Path) -> Option<Vec<String>> {
	let says = read(path)?;
	// A map old enough to have no `mapinfo.lua` declares no dependencies,
	// which is not the same as saying nothing about them.
	Some(says.mapinfo.as_deref().map(depends).unwrap_or_default())
}

/// The strings of a `depend = { … }` table, in the order they are written.
///
/// Read from the text rather than by running the Lua: every real one is a
/// list of literals on one line or a few, and a map that writes its
/// dependencies some cleverer way is one this says nothing about rather than
/// one it guesses at.
pub fn depends(mapinfo: &str) -> Vec<String> {
	let Some(at) = find_key(mapinfo, "depend") else {
		return Vec::new();
	};
	let rest = &mapinfo[at..];
	let Some(open) = rest.find('{') else {
		return Vec::new();
	};
	let Some(close) = rest[open..].find('}') else {
		return Vec::new();
	};
	let body = &rest[open + 1..open + close];
	body.split(',')
		.filter_map(|item| {
			let item = item.trim();
			let quote = item.chars().next().filter(|c| *c == '"' || *c == '\'')?;
			let inner = &item[1..];
			Some(inner[..inner.find(quote)?].to_owned())
		})
		.collect()
}

/// Where a `<key> =` sits at the head of a line, ignoring what a comment says.
fn find_key(mapinfo: &str, key: &str) -> Option<usize> {
	let mut at = 0;
	for line in mapinfo.lines() {
		let code = line.split("--").next().unwrap_or(line);
		let trimmed = code.trim_start();
		if let Some(rest) = trimmed.strip_prefix(key)
			&& rest.trim_start().starts_with('=')
		{
			return Some(at + (line.len() - trimmed.len()));
		}
		at += line.len() + 1;
	}
	None
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
/// One pass over a `.sd7` (7z), `.sdz` (zip) or unpacked `.sdd`.
pub fn read(path: &Path) -> Option<Says> {
	match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
		"sd7" => from_sd7(path),
		"sdz" => from_sdz(path),
		"sdd" => from_directory(path),
		_ => None,
	}
}

/// Whether an entry is the `mapinfo.lua`, wherever in the archive it sits.
fn is_mapinfo(name: &str) -> bool {
	base(name).eq_ignore_ascii_case("mapinfo.lua")
}

/// The `.smf`'s name without its directory or extension, which is the map's.
fn smf_name(name: &str) -> Option<String> {
	let file = base(name);
	let stem = file.get(..file.len().checked_sub(4)?)?;
	let is_smf = file[file.len() - 4..].eq_ignore_ascii_case(".smf");
	(is_smf && !stem.is_empty()).then(|| stem.to_owned())
}

/// The last segment of a path as an archive spells it, either separator.
fn base(name: &str) -> &str {
	name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn from_sd7(path: &Path) -> Option<Says> {
	let mut says = Says {
		mapinfo: None,
		smf: None,
	};
	sevenz_rust2::decompress_file_with_extract_fn(path, "", |entry, reader, _| {
		let name = entry.name().to_owned();
		if says.smf.is_none() {
			says.smf = smf_name(&name);
		}
		if is_mapinfo(&name) {
			let mut held = Vec::new();
			reader.take(MOST as u64).read_to_end(&mut held)?;
			says.mapinfo = String::from_utf8(held).ok();
		}
		// Keep walking: the `.smf` may come after the `mapinfo.lua`, or
		// instead of it on a map old enough to have none.
		Ok(true)
	})
	.ok()?;
	Some(says)
}

fn from_sdz(path: &Path) -> Option<Says> {
	let file = std::fs::File::open(path).ok()?;
	let mut zip = zip::ZipArchive::new(file).ok()?;
	let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
	let smf = names.iter().find_map(|name| smf_name(name));
	let mapinfo = names.iter().find(|name| is_mapinfo(name)).and_then(|name| {
		let mut held = Vec::new();
		zip.by_name(name)
			.ok()?
			.take(MOST as u64)
			.read_to_end(&mut held)
			.ok()?;
		String::from_utf8(held).ok()
	});
	Some(Says { mapinfo, smf })
}

/// An unpacked map is a directory, and its files are just files.
fn from_directory(path: &Path) -> Option<Says> {
	let mapinfo = std::fs::read_to_string(path.join("mapinfo.lua")).ok();
	let smf = std::fs::read_dir(path.join("maps"))
		.into_iter()
		.flatten()
		.filter_map(Result::ok)
		.find_map(|entry| smf_name(entry.file_name().to_str()?));
	Some(Says { mapinfo, smf })
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

	/// The one that cost an evening: a table holding an empty string is a
	/// dependency on an archive with no name, and the engine says so in
	/// words that name neither the map nor the mistake.
	#[test]
	fn an_empty_dependency_is_read_as_the_empty_name_it_is() {
		assert_eq!(depends(r#"depend = {""},"#), [""]);
		assert_eq!(depends("depend = {},"), [] as [String; 0]);
		assert_eq!(depends(r#"depend = {"Map Helper v1"},"#), ["Map Helper v1"]);
		assert_eq!(
			depends(r#"	depend      = {"Map Helper v1", "Other v2"},"#),
			["Map Helper v1", "Other v2"]
		);
		// A map that says nothing depends on nothing.
		assert_eq!(depends("name = \"Bare\""), [] as [String; 0]);
		// And the commented-out line in every template is not a declaration.
		assert_eq!(depends(r#"--depend = {"Ghost"},"#), [] as [String; 0]);
	}

	/// Every classic on springfiles predates `mapinfo.lua` and is named
	/// after its `.smf` instead -- `SpeedMetal`, not `speedmetal`, which is
	/// what the file is called and what a guess would have produced.
	#[test]
	fn a_map_older_than_mapinfo_is_named_by_its_smf() {
		assert_eq!(
			smf_name("maps/SpeedMetal.smf").as_deref(),
			Some("SpeedMetal")
		);
		assert_eq!(
			smf_name(concat!("maps", "\\", "TitanDuel.SMF")).as_deref(),
			Some("TitanDuel"),
			"either separator, either spelling of the extension"
		);
		// Everything else in such an archive is not its name.
		assert_eq!(smf_name("maps/SpeedMetal.smd"), None);
		assert_eq!(smf_name("maps/SpeedMetal.smt"), None);
		assert_eq!(smf_name("maps/mini.bmp"), None);
		assert_eq!(smf_name(".smf"), None, "no name at all is not a name");
		assert!(is_mapinfo("mapinfo.lua") && !is_mapinfo("notmapinfo.lua"));
	}

	#[test]
	fn single_quotes_are_lua_too() {
		assert_eq!(
			compose("name = 'Quoted'\nversion = 'v2'").as_deref(),
			Some("Quoted v2")
		);
	}
}
