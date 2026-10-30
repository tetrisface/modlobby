//! What a player changes inside a modded game, kept with the set of mods it
//! was changed with: captured at the engine's exit, seeded into the next
//! game of the same set, shown and folded into their normal settings on
//! request. Nothing a mod wrote ever crosses as it was written: every file
//! goes through its reader and modlobby's writer, or not at all.

pub mod config;
pub mod lua;

use std::collections::{BTreeSet, HashMap};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

/// The player's files a modded game is given and may change, under the
/// mods write directory; beside them, every `LuaUI/Config/*.lua` a widget
/// handler keeps.
pub const FILES: &[&str] = &[CONFIG, HOTKEYS, KEYBINDS, BLUEPRINTS];

const CONFIG: &str = "springsettings.cfg";
const HOTKEYS: &str = "uikeys.txt";
const KEYBINDS: &str = "LuaUI/Config/keybind_profiles.json";
const BLUEPRINTS: &str = "LuaUI/Config/blueprints.json";

/// Where a capture's set and time are written.
const RECORD: &str = "set.json";

/// The largest file a modded game left that is read at all. BAR's own
/// configs are tens of kilobytes; the cap keeps a planted giant from holding
/// the lobby up while it is parsed.
pub const MOST_FILE: u64 = 8 << 20;

/// The commands a hotkeys file may carry over: binding keys to actions,
/// nothing that loads another file or runs anything at load.
const HOTKEY_COMMANDS: &[&str] = &[
	"bind",
	"unbind",
	"unbindall",
	"unbindaction",
	"unbindkeyset",
	"fakemeta",
	"keysym",
];

/// How many added or removed lines of a file are shown.
const MOST_LINES: usize = 100;

/// The mods a lobby loads, as the key its settings are kept under: the
/// unordered set of repositories (a mod with no repository by its name),
/// and the order they were loaded in last time -- shown, never part of the
/// key, since order only matters where two mods overlap in widgets.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModSet {
	pub repos: BTreeSet<String>,
	pub order: Vec<String>,
}

impl ModSet {
	pub fn of(mutators: &[lobby_core::Mutator]) -> Self {
		let label = |mutator: &lobby_core::Mutator| match &mutator.source {
			Some(pin) => pin.repo.clone(),
			None => mutator.name.clone(),
		};
		let key = |mutator: &lobby_core::Mutator| match &mutator.source {
			Some(pin) => pin.repo.to_lowercase(),
			None => format!("name:{}", mutator.name.to_lowercase()),
		};
		Self {
			repos: mutators.iter().map(key).collect(),
			order: mutators.iter().map(label).collect(),
		}
	}

	/// Sixteen hex digits over the sorted repositories: a directory name.
	pub fn id(&self) -> String {
		let joined: Vec<&str> = self.repos.iter().map(String::as_str).collect();
		content::fetch::sha256_hex(joined.join("\n").as_bytes())[..16].to_owned()
	}
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
	#[serde(flatten)]
	set: ModSet,
	/// When the game whose settings `after/` holds ended, RFC 3339; `None`
	/// while only a baseline has been written.
	at: Option<String>,
}

/// One set's directory: `set.json`, `before/` (what the set's first game
/// was given, moved on by a fold-in) and `after/` (what its last game left).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
	pub dir: PathBuf,
}

impl Capture {
	pub fn before(&self) -> PathBuf {
		self.dir.join("before")
	}

	pub fn after(&self) -> PathBuf {
		self.dir.join("after")
	}

	/// The set this holds, and when its game ended.
	pub fn set(&self) -> Option<(ModSet, Option<String>)> {
		let text = std::fs::read_to_string(self.dir.join(RECORD)).ok()?;
		let record: Record = serde_json::from_str(&text).ok()?;
		Some((record.set, record.at))
	}

	fn record(&self, set: &ModSet, at: Option<String>) -> io::Result<()> {
		let at = at.or_else(|| self.set().and_then(|(_, at)| at));
		let record = Record {
			set: set.clone(),
			at,
		};
		let text = serde_json::to_string_pretty(&record).map_err(io::Error::other)?;
		std::fs::create_dir_all(&self.dir)?;
		std::fs::write(self.dir.join(RECORD), text)
	}
}

/// What the first launch of `set` was given, copied from `mods_write` --
/// just seeded -- into `before/`, through the readers. A set that has a
/// baseline keeps it: the list shows what its games changed since then, and
/// only a fold-in or a discard moves it.
pub fn baseline(settings: &Path, set: &ModSet, mods_write: &Path) -> io::Result<Capture> {
	let capture = Capture {
		dir: settings.join(set.id()),
	};
	capture.record(set, None)?;
	if !capture.before().is_dir() {
		copy_cleaned(mods_write, &capture.before())?;
	}
	Ok(capture)
}

/// What the game left in `mods_write`, copied into `after/` through the
/// readers; `at` is when it ended.
pub fn capture(settings: &Path, set: &ModSet, mods_write: &Path, at: &str) -> io::Result<Capture> {
	let capture = Capture {
		dir: settings.join(set.id()),
	};
	capture.record(set, Some(at.to_owned()))?;
	copy_cleaned(mods_write, &capture.after())?;
	Ok(capture)
}

/// Copies what [`FILES`] and the widget handler configs hold from `from`
/// into `into`, each as its reader read it and its writer writes it: a
/// file a reader rejects is left out and said. `into` starts empty.
fn copy_cleaned(from: &Path, into: &Path) -> io::Result<Vec<PathBuf>> {
	match std::fs::remove_dir_all(into) {
		Ok(()) => {}
		Err(err) if err.kind() == io::ErrorKind::NotFound => {}
		Err(err) => return Err(err),
	}
	std::fs::create_dir_all(into)?;
	let mut copied = Vec::new();
	let mut files: Vec<(PathBuf, Kind)> = FILES
		.iter()
		.map(|rel| {
			let kind = match rel.rsplit('.').next() {
				Some("cfg") => Kind::Config,
				Some("json") => Kind::Json,
				_ => Kind::Hotkeys,
			};
			(PathBuf::from(rel), kind)
		})
		.collect();
	files.extend(lua_configs(from).into_iter().map(|rel| (rel, Kind::Lua)));
	for (rel, kind) in files {
		let Some(bytes) = read_plain(from, &rel) else {
			continue;
		};
		let text = String::from_utf8_lossy(&bytes);
		let cleaned = match kind {
			Kind::Config => Some(config::write(&config::read(&text))),
			Kind::Json => json_clean(&text),
			Kind::Hotkeys => Some(hotkeys_clean(&text)),
			Kind::Lua => match lua::read(&text) {
				Ok(value) => Some(lua::write(&value)),
				Err(why) => {
					tracing::warn!(file = %rel.display(), %why, "a modded game's config is not data; not kept");
					None
				}
			},
		};
		let Some(cleaned) = cleaned else {
			continue;
		};
		let target = into.join(&rel);
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::write(&target, cleaned)?;
		copied.push(rel);
	}
	Ok(copied)
}

/// The file at `rel` under `from`, when every step of the way is a plain
/// directory or file of its own -- no link, no junction, which would lead
/// out of the sandbox's directory to the player's files -- and it is no
/// larger than [`MOST_FILE`].
fn read_plain(from: &Path, rel: &Path) -> Option<Vec<u8>> {
	let mut at = from.to_path_buf();
	for part in rel.components() {
		at.push(part);
		let kind = std::fs::symlink_metadata(&at).ok()?.file_type();
		if kind.is_symlink() {
			tracing::warn!(path = %at.display(), "a link in a modded game's directory; not followed");
			return None;
		}
	}
	let meta = std::fs::symlink_metadata(&at).ok()?;
	if !meta.is_file() {
		return None;
	}
	if meta.len() > MOST_FILE {
		tracing::warn!(path = %at.display(), bytes = meta.len(), "a modded game's file past the cap; not read");
		return None;
	}
	let mut bytes = Vec::new();
	std::fs::File::open(&at)
		.ok()?
		.take(MOST_FILE + 1)
		.read_to_end(&mut bytes)
		.ok()?;
	(bytes.len() as u64 <= MOST_FILE).then_some(bytes)
}

/// The binding commands of a hotkeys file, one a line as modlobby writes
/// them: a line that is anything else -- a `keyload` of another file, a
/// comment, a control character -- is dropped.
pub fn hotkeys_clean(text: &str) -> String {
	text.lines()
		.map(str::trim)
		.filter(|line| {
			let command = line.split_whitespace().next().unwrap_or("");
			HOTKEY_COMMANDS.contains(&command.to_ascii_lowercase().as_str())
				&& line
					.chars()
					.all(|c| c == ' ' || c == '\t' || c.is_ascii_graphic())
		})
		.map(|line| format!("{line}\n"))
		.collect()
}

/// What [`seed_from`] put into the mods write directory, and the settings
/// text the launch pins its config over.
#[derive(Debug, Default)]
pub struct Seeded {
	pub files: Vec<PathBuf>,
	pub config: Option<String>,
}

/// Seeds the next game of a set from what its last game left: every file
/// in `after/` but the settings file goes into `mods_write` through the
/// readers once more, over the open seed; the settings file's text is
/// handed back for the launch to pin its keys over.
pub fn seed_from(capture: &Capture, mods_write: &Path) -> io::Result<Seeded> {
	let after = capture.after();
	let staged = capture.dir.join("seed.tmp");
	let files = copy_cleaned(&after, &staged)?;
	let mut seeded = Seeded::default();
	for rel in files {
		let source = staged.join(&rel);
		if rel == Path::new("springsettings.cfg") {
			seeded.config = std::fs::read_to_string(&source).ok();
			continue;
		}
		let target = mods_write.join(&rel);
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::copy(&source, &target)?;
		seeded.files.push(rel);
	}
	let _ = std::fs::remove_dir_all(&staged);
	Ok(seeded)
}

#[derive(Clone, Copy)]
enum Kind {
	Config,
	Json,
	Hotkeys,
	Lua,
}

/// The capture to seed `set` from: the set itself when it has one, else the
/// largest captured strict subset of its repositories (the newest among
/// equals), else none.
pub fn find(settings: &Path, set: &ModSet) -> Option<Capture> {
	let held = captures(settings);
	if let Some((_, capture, _)) = held.iter().find(|(kept, _, _)| kept.repos == set.repos) {
		return Some(capture.clone());
	}
	held.into_iter()
		.filter(|(kept, _, _)| !kept.repos.is_empty() && kept.repos.is_subset(&set.repos))
		.max_by(|(a, _, at_a), (b, _, at_b)| a.repos.len().cmp(&b.repos.len()).then(at_a.cmp(at_b)))
		.map(|(_, capture, _)| capture)
}

/// Every set with a game's settings captured, newest first.
pub fn captures(settings: &Path) -> Vec<(ModSet, Capture, String)> {
	let Ok(entries) = std::fs::read_dir(settings) else {
		return Vec::new();
	};
	let mut held: Vec<(ModSet, Capture, String)> = entries
		.filter_map(Result::ok)
		.filter_map(|entry| {
			let capture = Capture { dir: entry.path() };
			let (set, at) = capture.set()?;
			let at = at?;
			capture.after().is_dir().then_some((set, capture, at))
		})
		.collect();
	held.sort_by(|(_, _, a), (_, _, b)| b.cmp(a));
	held
}

/// What `after/` holds that `before/` did not: engine keys, widgets turned
/// on or off, and the lines of the keybind profiles, hotkeys and blueprints
/// that differ. A key or widget gone from `after/` is no change, since a
/// fold-in adds and never removes. `None` for a capture with no game ended
/// yet.
pub fn changes(capture: &Capture) -> Option<lobby_ui::ModSetView> {
	let (set, at) = capture.set()?;
	let at = at?;
	let before = capture.before();
	let after = capture.after();
	Some(lobby_ui::ModSetView {
		id: set.id(),
		repos: set.repos.iter().cloned().collect(),
		order: set.order.clone(),
		at,
		engine: engine_changes(&text_of(&before, CONFIG), &text_of(&after, CONFIG)),
		widgets: widget_changes(&before, &after),
		keybinds: text_change(&before, &after, KEYBINDS),
		hotkeys: text_change(&before, &after, HOTKEYS),
		blueprints: text_change(&before, &after, BLUEPRINTS),
	})
}

/// The keys `after` sets to something `before` did not.
fn engine_changes(before: &str, after: &str) -> Vec<lobby_ui::KeyChange> {
	let before = config::read(before);
	config::read(after)
		.into_iter()
		.filter_map(|(key, value)| {
			let was = before
				.iter()
				.find(|(held, _)| held.eq_ignore_ascii_case(&key))
				.map(|(_, held)| held.clone());
			(was.as_deref() != Some(value.as_str())).then(|| lobby_ui::KeyChange {
				plain: config::is_plain(&value),
				key,
				before: was,
				after: value,
			})
		})
		.collect()
}

/// Every widget handler config in `after`, each against its own in `before`.
fn widget_changes(before: &Path, after: &Path) -> Vec<lobby_ui::WidgetChange> {
	let mut out = Vec::new();
	for rel in lua_configs(after) {
		let Some(now) = read_lua(after, &rel) else {
			continue;
		};
		let was = read_lua(before, &rel).unwrap_or(lua::Lua::Table(Vec::new()));
		out.extend(toggles(&was, &now));
	}
	out.sort_by(|a, b| a.widget.cmp(&b.widget));
	out
}

/// The widgets `now`'s `order` turns on or off against `was`'s; a widget
/// only moved within the order is no change, and one `now` lists as replaced
/// was switched off by the widget handler, not by the player.
fn toggles(was: &lua::Lua, now: &lua::Lua) -> Vec<lobby_ui::WidgetChange> {
	let on = |value: &lua::Lua| matches!(value, lua::Lua::Number(n) if *n > 0.0);
	let order_was = fields_of(was, "order");
	let replaced = fields_of(now, "replaced");
	fields_of(now, "order")
		.iter()
		.filter_map(|(key, value)| {
			let widget = key_text(key);
			if field(&replaced, &widget).is_some() {
				return None;
			}
			let was_on = field(&order_was, &widget).is_some_and(on);
			(was_on != on(value)).then(|| lobby_ui::WidgetChange {
				widget,
				on: on(value),
			})
		})
		.collect()
}

/// The lines of `rel` that `after` added and removed against `before`,
/// counted as many times as they occur, blank lines aside; `None` when there
/// are none, or `after` has no such file.
fn text_change(before: &Path, after: &Path, rel: &str) -> Option<lobby_ui::TextChange> {
	let now = std::fs::read_to_string(after.join(rel)).ok()?;
	let was = std::fs::read_to_string(before.join(rel)).unwrap_or_default();
	let (added, removed) = lines_between(&was, &now);
	if added.is_empty() && removed.is_empty() {
		return None;
	}
	let more = added.len().saturating_sub(MOST_LINES) + removed.len().saturating_sub(MOST_LINES);
	Some(lobby_ui::TextChange {
		added: added.into_iter().take(MOST_LINES).collect(),
		removed: removed.into_iter().take(MOST_LINES).collect(),
		more: u32::try_from(more).unwrap_or(u32::MAX),
	})
}

/// The lines in `now` that `was` does not hold, and those in `was` that
/// `now` does not, each trimmed, as many times as they occur.
fn lines_between(was: &str, now: &str) -> (Vec<String>, Vec<String>) {
	let lines = |text: &str| -> Vec<String> {
		text.lines()
			.map(str::trim)
			.filter(|line| !line.is_empty())
			.map(str::to_owned)
			.collect()
	};
	let (was, now) = (lines(was), lines(now));
	let only_in = |these: &[String], those: &[String]| -> Vec<String> {
		let mut left: HashMap<&str, usize> = HashMap::new();
		for line in those {
			*left.entry(line.as_str()).or_default() += 1;
		}
		these
			.iter()
			.filter(|line| match left.get_mut(line.as_str()) {
				Some(count) if *count > 0 => {
					*count -= 1;
					false
				}
				_ => true,
			})
			.cloned()
			.collect()
	};
	(only_in(&now, &was), only_in(&was, &now))
}

/// The fields of `table`'s field `name` when that is a table, else none.
fn fields_of(table: &lua::Lua, name: &str) -> Vec<(lua::Key, lua::Lua)> {
	match table {
		lua::Lua::Table(fields) => table_of(fields, name),
		_ => Vec::new(),
	}
}

/// The fields of the field `name` among `fields` when it is a table, else
/// none.
fn table_of(fields: &[(lua::Key, lua::Lua)], name: &str) -> Vec<(lua::Key, lua::Lua)> {
	match field(fields, name) {
		Some(lua::Lua::Table(inner)) => inner.clone(),
		_ => Vec::new(),
	}
}

fn field<'a>(fields: &'a [(lua::Key, lua::Lua)], name: &str) -> Option<&'a lua::Lua> {
	fields
		.iter()
		.find(|(key, _)| key_text(key) == name)
		.map(|(_, value)| value)
}

fn key_text(key: &lua::Key) -> String {
	match key {
		lua::Key::Name(name) | lua::Key::Str(name) => name.clone(),
		lua::Key::Number(number) => number.to_string(),
	}
}

/// Folds what `after/` changed, of `kinds`, into the player's own files
/// under `open_write`, each through its writer: the plain engine keys over
/// `springsettings.cfg`, the widgets turned on or off into the open widget
/// config's order, the hotkeys' added and removed lines into `uikeys.txt`,
/// the keybind profiles and blueprints whole. What was folded is then the
/// baseline too, so it is not shown again; a text-valued engine key stays
/// shown, for the player to set by hand. What was written, by path relative
/// to `open_write`.
pub fn fold_in(
	capture: &Capture,
	open_write: &Path,
	kinds: &lobby_ui::FoldKinds,
) -> io::Result<Vec<PathBuf>> {
	let (before, after) = (capture.before(), capture.after());
	let mut written = Vec::new();
	if kinds.engine {
		let changed = engine_changes(&text_of(&before, CONFIG), &text_of(&after, CONFIG));
		let plain: Vec<(&str, &str)> = changed
			.iter()
			.filter(|change| change.plain)
			.map(|change| (change.key.as_str(), change.after.as_str()))
			.collect();
		if !plain.is_empty() {
			let theirs = text_of(open_write, CONFIG);
			let text = recoil::window_mode::set_keys(&theirs, &plain);
			write_under(open_write, Path::new(CONFIG), text.as_bytes())?;
			written.push(PathBuf::from(CONFIG));
			let settled = recoil::window_mode::set_keys(&text_of(&before, CONFIG), &plain);
			write_under(
				&before,
				Path::new(CONFIG),
				config::write(&config::read(&settled)).as_bytes(),
			)?;
		}
	}
	if kinds.widgets {
		for rel in lua_configs(&after) {
			let Some(now) = read_lua(&after, &rel) else {
				continue;
			};
			let was = read_lua(&before, &rel).unwrap_or(lua::Lua::Table(Vec::new()));
			let changed = toggles(&was, &now);
			if changed.is_empty() {
				continue;
			}
			let own = match std::fs::read_to_string(open_write.join(&rel)) {
				Ok(text) => lua::read(&text)
					.map_err(|why| io::Error::other(format!("{}: {why}", rel.display())))?,
				Err(_) => lua::Lua::Table(Vec::new()),
			};
			write_under(
				open_write,
				&rel,
				lua::write(&with_order(own, &now, &changed)).as_bytes(),
			)?;
			written.push(rel.clone());
			write_under(
				&before,
				&rel,
				lua::write(&with_order(was, &now, &changed)).as_bytes(),
			)?;
		}
	}
	if kinds.hotkeys
		&& let Some(change) = text_change(&before, &after, HOTKEYS)
	{
		let own = text_of(open_write, HOTKEYS);
		write_under(
			open_write,
			Path::new(HOTKEYS),
			with_lines(&own, &change).as_bytes(),
		)?;
		written.push(PathBuf::from(HOTKEYS));
		settle(&after, &before, Path::new(HOTKEYS))?;
	}
	for (asked, rel) in [(kinds.keybinds, KEYBINDS), (kinds.blueprints, BLUEPRINTS)] {
		if !asked || text_change(&before, &after, rel).is_none() {
			continue;
		}
		let bytes = std::fs::read(after.join(rel))?;
		write_under(open_write, Path::new(rel), &bytes)?;
		written.push(PathBuf::from(rel));
		settle(&after, &before, Path::new(rel))?;
	}
	Ok(written)
}

/// Forgets the set: nothing is left to show, and its next game starts from
/// the player's own settings.
pub fn discard(capture: &Capture) -> io::Result<()> {
	std::fs::remove_dir_all(&capture.dir)
}

/// `own` with each of `changed` in its order as `now` has it, and the rest
/// as it was.
fn with_order(own: lua::Lua, now: &lua::Lua, changed: &[lobby_ui::WidgetChange]) -> lua::Lua {
	let mut fields = match own {
		lua::Lua::Table(fields) => fields,
		_ => Vec::new(),
	};
	let theirs = fields_of(now, "order");
	let mut order = table_of(&fields, "order");
	for change in changed {
		if let Some(value) = field(&theirs, &change.widget) {
			set_field(&mut order, &change.widget, value.clone());
		}
	}
	set_field(&mut fields, "order", lua::Lua::Table(order));
	lua::Lua::Table(fields)
}

/// `own` less the lines `change` removed, with the lines it added after
/// them; everything else of the player's file stays as it was.
fn with_lines(own: &str, change: &lobby_ui::TextChange) -> String {
	let mut gone: HashMap<&str, usize> = HashMap::new();
	for line in &change.removed {
		*gone.entry(line.as_str()).or_default() += 1;
	}
	let mut out: String = own
		.lines()
		.filter(|line| match gone.get_mut(line.trim()) {
			Some(count) if *count > 0 => {
				*count -= 1;
				false
			}
			_ => true,
		})
		.map(|line| format!("{line}\n"))
		.collect();
	for line in &change.added {
		out.push_str(line);
		out.push('\n');
	}
	out
}

fn set_field(fields: &mut Vec<(lua::Key, lua::Lua)>, name: &str, value: lua::Lua) {
	match fields.iter_mut().find(|(key, _)| key_text(key) == name) {
		Some((_, held)) => *held = value,
		None => fields.push((lua::Key::Name(name.to_owned()), value)),
	}
}

/// `rel` in `into` as it is in `from`: gone from `into` when `from` has
/// none.
fn settle(from: &Path, into: &Path, rel: &Path) -> io::Result<()> {
	match std::fs::read(from.join(rel)) {
		Ok(bytes) => write_under(into, rel, &bytes),
		Err(err) if err.kind() == io::ErrorKind::NotFound => {
			match std::fs::remove_file(into.join(rel)) {
				Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
				_ => Ok(()),
			}
		}
		Err(err) => Err(err),
	}
}

fn write_under(dir: &Path, rel: &Path, bytes: &[u8]) -> io::Result<()> {
	let path = dir.join(rel);
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)?;
	}
	std::fs::write(path, bytes)
}

/// `rel` under `dir` as text; empty when it is not there.
fn text_of(dir: &Path, rel: &str) -> String {
	std::fs::read_to_string(dir.join(rel)).unwrap_or_default()
}

fn read_lua(dir: &Path, rel: &Path) -> Option<lua::Lua> {
	let text = std::fs::read_to_string(dir.join(rel)).ok()?;
	lua::read(&text).ok()
}

/// Every `LuaUI/Config/*.lua` under `dir`, by path relative to it.
fn lua_configs(dir: &Path) -> Vec<PathBuf> {
	let Ok(configs) = std::fs::read_dir(dir.join("LuaUI").join("Config")) else {
		return Vec::new();
	};
	configs
		.filter_map(Result::ok)
		.filter(|entry| {
			let path = entry.path();
			path.extension().is_some_and(|ext| ext == "lua") && path.is_file()
		})
		.map(|entry| Path::new("LuaUI").join("Config").join(entry.file_name()))
		.collect()
}

/// `text` as JSON, written again by modlobby: the same data, none of the
/// bytes. `None` for what is not JSON.
pub fn json_clean(text: &str) -> Option<String> {
	let value: serde_json::Value = serde_json::from_str(text).ok()?;
	serde_json::to_string_pretty(&value).ok()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn json_is_read_and_written_as_data() {
		assert_eq!(
			json_clean("{\"active\": \"mine\", \"n\": 1}").as_deref(),
			Some("{\n  \"active\": \"mine\",\n  \"n\": 1\n}")
		);
		assert_eq!(json_clean("{not json"), None);
	}

	fn pinned(repo: &str) -> lobby_core::Mutator {
		lobby_core::Mutator {
			name: format!("github-{}-000000000000.sdd", repo.replace('/', "-")),
			checksum: None,
			source: Some(lobby_core::GitPin {
				repo: repo.into(),
				commit: "0".repeat(40),
			}),
			date: None,
			track: None,
		}
	}

	fn named(name: &str) -> lobby_core::Mutator {
		lobby_core::Mutator {
			name: name.into(),
			checksum: None,
			source: None,
			date: None,
			track: None,
		}
	}

	fn plant(dir: &Path, rel: &str, text: &str) {
		let path = dir.join(rel);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, text).unwrap();
	}

	#[test]
	fn a_sets_id_ignores_order_and_case() {
		let one = ModSet::of(&[pinned("dev/B"), pinned("Dev/A"), named("tiny maps v1")]);
		let other = ModSet::of(&[named("Tiny Maps v1"), pinned("dev/a"), pinned("dev/b")]);
		assert_eq!(one.id(), other.id());
		assert_eq!(one.order, ["dev/B", "Dev/A", "tiny maps v1"]);
		assert_eq!(
			one.repos.iter().cloned().collect::<Vec<_>>(),
			["dev/a", "dev/b", "name:tiny maps v1"]
		);
		assert_ne!(one.id(), ModSet::of(&[pinned("dev/a")]).id());
		assert_eq!(one.id().len(), 16);
	}

	#[test]
	fn a_capture_holds_only_what_its_parsers_admit() {
		let dir = tempfile::tempdir().unwrap();
		let settings = dir.path().join("mods-settings");
		let write = dir.path().join("mods-data");
		plant(
			&write,
			"springsettings.cfg",
			"Fullscreen = 0
LuaSocketEnabled = 0
",
		);
		plant(
			&write,
			"uikeys.txt",
			"bind esc quitmenu
",
		);
		plant(
			&write,
			"LuaUI/Config/BYAR.lua",
			"return {
	order = {
		Chat = 7,
	},
}
",
		);
		plant(
			&write,
			"LuaUI/Config/BYAR_bad.lua",
			"return { x = print(\"y\") }
",
		);
		plant(
			&write,
			"LuaUI/Config/evil.lua",
			"print(\"x\")
",
		);
		plant(
			&write,
			"LuaUI/Config/keybind_profiles.json",
			"{\"active\": \"mine\"}",
		);
		plant(&write, "LuaUI/Config/notes.txt", "not a settings file");
		let set = ModSet::of(&[pinned("dev/sphere")]);
		let capture = capture(&settings, &set, &write, "2026-10-22T10:00:00Z").unwrap();
		let after = |rel: &str| std::fs::read_to_string(capture.dir.join("after").join(rel)).ok();
		assert_eq!(
			after("springsettings.cfg").as_deref(),
			Some(
				"Fullscreen = 0
"
			)
		);
		assert_eq!(
			after("uikeys.txt").as_deref(),
			Some(
				"bind esc quitmenu
"
			)
		);
		assert_eq!(
			after("LuaUI/Config/BYAR.lua").as_deref(),
			Some(
				"return {
	order = {
		Chat = 7,
	},
}
"
			)
		);
		assert_eq!(
			after("LuaUI/Config/keybind_profiles.json").as_deref(),
			Some(
				"{
  \"active\": \"mine\"
}"
			)
		);
		assert!(
			after("LuaUI/Config/BYAR_bad.lua").is_none(),
			"a call got through"
		);
		assert!(after("LuaUI/Config/evil.lua").is_none());
		assert!(after("LuaUI/Config/notes.txt").is_none());
		let (recorded, at) = capture.set().unwrap();
		assert_eq!(recorded, set);
		assert_eq!(at.as_deref(), Some("2026-10-22T10:00:00Z"));
	}

	#[test]
	fn the_largest_captured_subset_seeds_a_new_set() {
		let dir = tempfile::tempdir().unwrap();
		let settings = dir.path().join("mods-settings");
		let write = dir.path().join("mods-data");
		std::fs::create_dir_all(&write).unwrap();
		let ab = ModSet::of(&[pinned("dev/a"), pinned("dev/b")]);
		let a = ModSet::of(&[pinned("dev/a")]);
		capture(&settings, &ab, &write, "2026-10-20T00:00:00Z").unwrap();
		capture(&settings, &a, &write, "2026-10-21T00:00:00Z").unwrap();
		let found = |set: &ModSet| find(&settings, set).map(|c| c.set().unwrap().0);
		assert_eq!(
			found(&ModSet::of(&[
				pinned("dev/a"),
				pinned("dev/b"),
				pinned("dev/c")
			])),
			Some(ab.clone())
		);
		assert_eq!(found(&a), Some(a.clone()));
		assert_eq!(found(&ab), Some(ab));
		assert_eq!(found(&ModSet::of(&[pinned("dev/c")])), None);
		assert_eq!(
			found(&ModSet::of(&[pinned("dev/b")])),
			None,
			"a superset is not a subset"
		);
		let listed: Vec<String> = captures(&settings)
			.into_iter()
			.map(|(set, _, at)| format!("{} {at}", set.order.join("+")))
			.collect();
		assert_eq!(
			listed,
			[
				"dev/a 2026-10-21T00:00:00Z",
				"dev/a+dev/b 2026-10-20T00:00:00Z"
			]
		);
	}

	// ------------------------------------------------- what changed, as a diff

	/// A capture with `before/` and `after/` planted as given, already in
	/// the readers' shape.
	fn planted(dir: &Path, before: &[(&str, &str)], after: &[(&str, &str)]) -> (ModSet, Capture) {
		let set = ModSet::of(&[pinned("dev/a"), pinned("dev/b")]);
		let capture = Capture {
			dir: dir.join("mods-settings").join(set.id()),
		};
		capture
			.record(&set, Some("2026-10-22T10:00:00Z".into()))
			.unwrap();
		for (rel, text) in before {
			plant(&capture.before(), rel, text);
		}
		for (rel, text) in after {
			plant(&capture.after(), rel, text);
		}
		(set, capture)
	}

	const WIDGETS_BEFORE: &str = "return {\n\tdata = {\n\t\tChat = {\n\t\t\tname = \"spring\",\n\t\t\tsize = 1,\n\t\t},\n\t\tIdle = {\n\t\t\tx = 1,\n\t\t},\n\t},\n\torder = {\n\t\tChat = 0,\n\t\tIdle = 3,\n\t},\n}\n";
	const WIDGETS_AFTER: &str = "return {\n\tdata = {\n\t\tChat = {\n\t\t\tname = \"spring\",\n\t\t\tsize = 2,\n\t\t},\n\t\tIdle = {\n\t\t\tx = 1,\n\t\t},\n\t},\n\torder = {\n\t\tChat = 1,\n\t\tIdle = 4,\n\t},\n}\n";

	#[test]
	fn engine_keys_widgets_and_keybinds_diff_against_the_baseline() {
		let dir = tempfile::tempdir().unwrap();
		let (set, capture) = planted(
			dir.path(),
			&[
				("springsettings.cfg", "Fullscreen = 1\nVSync = 1\n"),
				("LuaUI/Config/BYAR.lua", WIDGETS_BEFORE),
				(
					"LuaUI/Config/keybind_profiles.json",
					"{\n  \"active\": \"a\"\n}",
				),
				("uikeys.txt", "bind esc quitmenu\n"),
			],
			&[
				(
					"springsettings.cfg",
					"Fullscreen = 0\nVSync = 1\nLogFile = x\n",
				),
				("LuaUI/Config/BYAR.lua", WIDGETS_AFTER),
				(
					"LuaUI/Config/keybind_profiles.json",
					"{\n  \"active\": \"b\"\n}",
				),
				("uikeys.txt", "bind esc quitmenu\n"),
			],
		);
		let view = changes(&capture).unwrap();
		assert_eq!(view.id, set.id());
		assert_eq!(view.repos, ["dev/a", "dev/b"]);
		assert_eq!(view.order, ["dev/a", "dev/b"]);
		assert_eq!(view.at, "2026-10-22T10:00:00Z");
		assert_eq!(
			view.engine,
			[
				lobby_ui::KeyChange {
					key: "Fullscreen".into(),
					before: Some("1".into()),
					after: "0".into(),
					plain: true,
				},
				lobby_ui::KeyChange {
					key: "LogFile".into(),
					before: None,
					after: "x".into(),
					plain: false,
				},
			]
		);
		assert_eq!(
			view.widgets,
			[lobby_ui::WidgetChange {
				widget: "Chat".into(),
				on: true,
			}],
			"Idle only moved in the order, and widget data is never a change: \
			 a modded game starts every widget at its defaults"
		);
		assert_eq!(
			view.keybinds,
			Some(lobby_ui::TextChange {
				added: vec!["\"active\": \"b\"".into()],
				removed: vec!["\"active\": \"a\"".into()],
				more: 0,
			})
		);
		assert_eq!(view.hotkeys, None);
		assert_eq!(view.blueprints, None);
	}

	#[test]
	fn an_unchanged_set_has_no_changes() {
		let dir = tempfile::tempdir().unwrap();
		let same = [
			("springsettings.cfg", "Fullscreen = 1\n"),
			(
				"LuaUI/Config/BYAR.lua",
				"return {\n\torder = {\n\t\tChat = 1,\n\t},\n}\n",
			),
		];
		let (_, capture) = planted(dir.path(), &same, &same);
		let view = changes(&capture).unwrap();
		assert!(view.engine.is_empty(), "{view:?}");
		assert!(view.widgets.is_empty(), "{view:?}");
		assert!(
			view.keybinds.is_none() && view.hotkeys.is_none() && view.blueprints.is_none(),
			"{view:?}"
		);
		// A file the game left that the launch never gave it is all new.
		plant(&capture.after(), "uikeys.txt", "bind esc quitmenu\n");
		assert_eq!(
			changes(&capture).unwrap().hotkeys,
			Some(lobby_ui::TextChange {
				added: vec!["bind esc quitmenu".into()],
				removed: Vec::new(),
				more: 0,
			})
		);
	}

	// ------------------------------------------------- fold in and discard

	fn kinds(engine: bool, widgets: bool) -> lobby_ui::FoldKinds {
		lobby_ui::FoldKinds {
			engine,
			widgets,
			keybinds: false,
			hotkeys: false,
			blueprints: false,
		}
	}

	/// The value at `path` in `value`, by field names.
	fn at<'a>(value: &'a lua::Lua, path: &[&str]) -> Option<&'a lua::Lua> {
		path.iter().try_fold(value, |value, name| match value {
			lua::Lua::Table(fields) => field(fields, name),
			_ => None,
		})
	}

	#[test]
	fn fold_in_never_writes_a_string_key() {
		let dir = tempfile::tempdir().unwrap();
		let (_, capture) = planted(
			dir.path(),
			&[("springsettings.cfg", "Fullscreen = 1\n")],
			&[("springsettings.cfg", "Fullscreen = 0\nLogFile = x\n")],
		);
		let open = dir.path().join("open");
		plant(&open, "springsettings.cfg", "Fullscreen = 1\nVSync = 1\n");
		let written = fold_in(&capture, &open, &kinds(true, false)).unwrap();
		assert_eq!(written, [PathBuf::from("springsettings.cfg")]);
		assert_eq!(
			std::fs::read_to_string(open.join("springsettings.cfg")).unwrap(),
			"Fullscreen = 0\nVSync = 1\n"
		);
		// The plain key is folded and no longer shown; the text one was not
		// written, so it stays in the list for the player to set by hand.
		let left = changes(&capture).unwrap().engine;
		assert_eq!(left.len(), 1, "{left:?}");
		assert_eq!(left[0].key, "LogFile");
	}

	/// BAR switches off a widget another takes the place of (GetInfo `replaces`)
	/// and lists it under `replaced`, and hands it back when the other goes. A
	/// mod's replacement is not there in a normal game to hand anything back.
	#[test]
	fn a_widget_a_mod_took_the_place_of_stays_on_in_a_fold_in() {
		let dir = tempfile::tempdir().unwrap();
		let (_, capture) = planted(
			dir.path(),
			&[(
				"LuaUI/Config/BYAR.lua",
				"return {\n\torder = {\n\t\tStock = 3,\n\t\tOther = 2,\n\t},\n}\n",
			)],
			&[(
				"LuaUI/Config/BYAR.lua",
				"return {\n\torder = {\n\t\tStock = 0,\n\t\tOther = 0,\n\t},\n\treplaced = {\n\t\tStock = \"Swap\",\n\t},\n}\n",
			)],
		);
		let open = dir.path().join("open");
		plant(
			&open,
			"LuaUI/Config/BYAR.lua",
			"return {\n\torder = {\n\t\tStock = 3,\n\t\tOther = 2,\n\t},\n}\n",
		);
		fold_in(&capture, &open, &kinds(false, true)).unwrap();
		let text = std::fs::read_to_string(open.join("LuaUI/Config/BYAR.lua")).unwrap();
		let merged = lua::read(&text).unwrap();
		assert_eq!(
			at(&merged, &["order", "Stock"]),
			Some(&lua::Lua::Number(3.0)),
			"what the mod replaced is not what the player turned off"
		);
		assert_eq!(
			at(&merged, &["order", "Other"]),
			Some(&lua::Lua::Number(0.0)),
			"what the player did turn off still folds in"
		);
	}

	#[test]
	fn fold_in_turns_widgets_on_and_off_and_leaves_their_settings() {
		let dir = tempfile::tempdir().unwrap();
		let (_, capture) = planted(
			dir.path(),
			&[
				("springsettings.cfg", "Fullscreen = 1\n"),
				("LuaUI/Config/BYAR.lua", WIDGETS_BEFORE),
			],
			&[
				("springsettings.cfg", "Fullscreen = 0\n"),
				("LuaUI/Config/BYAR.lua", WIDGETS_AFTER),
			],
		);
		let open = dir.path().join("open");
		plant(&open, "springsettings.cfg", "Fullscreen = 1\n");
		plant(
			&open,
			"LuaUI/Config/BYAR.lua",
			"return {\n\tallowUserWidgets = true,\n\tdata = {\n\t\tChat = {\n\t\t\tsize = 5,\n\t\t},\n\t\tOther = {\n\t\t\tx = 1,\n\t\t},\n\t},\n\torder = {\n\t\tChat = 0,\n\t\tOther = 2,\n\t},\n}\n",
		);
		let written = fold_in(&capture, &open, &kinds(false, true)).unwrap();
		assert_eq!(written, [Path::new("LuaUI/Config/BYAR.lua").to_path_buf()]);
		let text = std::fs::read_to_string(open.join("LuaUI/Config/BYAR.lua")).unwrap();
		let merged = lua::read(&text).unwrap();
		assert_eq!(
			at(&merged, &["order", "Chat"]),
			Some(&lua::Lua::Number(1.0))
		);
		assert_eq!(
			at(&merged, &["order", "Other"]),
			Some(&lua::Lua::Number(2.0)),
			"the player's other widgets stay"
		);
		assert_eq!(
			at(&merged, &["data", "Chat", "size"]),
			Some(&lua::Lua::Number(5.0)),
			"a widget's settings stay the player's own"
		);
		assert_eq!(
			at(&merged, &["data", "Other", "x"]),
			Some(&lua::Lua::Number(1.0))
		);
		assert_eq!(
			at(&merged, &["allowUserWidgets"]),
			Some(&lua::Lua::Bool(true))
		);
		assert_eq!(
			at(&merged, &["order", "Idle"]),
			None,
			"a widget only moved in the order is not the player's business"
		);
		// The engine keys were not asked for: still theirs, still shown.
		assert_eq!(
			std::fs::read_to_string(open.join("springsettings.cfg")).unwrap(),
			"Fullscreen = 1\n"
		);
		let view = changes(&capture).unwrap();
		assert_eq!(view.engine.len(), 1, "{view:?}");
		assert!(view.widgets.is_empty(), "{view:?}");
	}

	/// Discarding forgets the set: nothing is left to show, and its next
	/// game starts from the player's own settings, never from a baseline
	/// that may hold what the sandbox is not given.
	#[test]
	fn discard_forgets_the_set() {
		let dir = tempfile::tempdir().unwrap();
		let (_, capture) = planted(
			dir.path(),
			&[("springsettings.cfg", "Fullscreen = 1\n")],
			&[
				("springsettings.cfg", "Fullscreen = 0\n"),
				("uikeys.txt", "bind esc quitmenu\n"),
			],
		);
		discard(&capture).unwrap();
		assert!(!capture.dir.exists());
		assert!(captures(&dir.path().join("mods-settings")).is_empty());
	}

	// ------------------------------------------- fix pass: the review's cases

	/// The baseline is what the set's first game was given, and only a
	/// fold-in or a discard moves it: the list shows what changed since then,
	/// however many games of the set were played in between.
	#[test]
	fn a_baseline_is_kept_until_folded_or_forgotten() {
		let dir = tempfile::tempdir().unwrap();
		let settings = dir.path().join("mods-settings");
		let write = dir.path().join("mods-data");
		let set = ModSet::of(&[pinned("dev/a")]);
		plant(&write, "springsettings.cfg", "Fullscreen = 1\n");
		baseline(&settings, &set, &write).unwrap();
		plant(&write, "springsettings.cfg", "Fullscreen = 0\n");
		let capture = baseline(&settings, &set, &write).unwrap();
		assert_eq!(
			std::fs::read_to_string(capture.before().join("springsettings.cfg")).unwrap(),
			"Fullscreen = 1\n"
		);
	}

	/// `uikeys.txt` is a list of commands the engine runs: only the binding
	/// commands are kept, one a line, as modlobby writes them; a `keyload`
	/// of another file, a control character, anything else, is dropped.
	#[test]
	fn a_hotkeys_file_keeps_only_binding_commands() {
		let dir = tempfile::tempdir().unwrap();
		let settings = dir.path().join("mods-settings");
		let write = dir.path().join("mods-data");
		plant(
			&write,
			"uikeys.txt",
			"bind esc quitmenu\r\n  unbindall\nkeyload evil.txt\n// a comment\nbind Any+x say \u{7}hi\nfakemeta space\nrm -rf\n",
		);
		let capture = capture(
			&settings,
			&ModSet::of(&[pinned("dev/a")]),
			&write,
			"2026-10-23T10:00:00Z",
		)
		.unwrap();
		assert_eq!(
			std::fs::read_to_string(capture.after().join("uikeys.txt")).unwrap(),
			"bind esc quitmenu\nunbindall\nfakemeta space\n"
		);
	}

	/// A file past the cap is not read at all: nothing a mod writes may hold
	/// the lobby up while it is parsed.
	#[test]
	fn a_file_past_the_size_cap_is_not_kept() {
		let dir = tempfile::tempdir().unwrap();
		let settings = dir.path().join("mods-settings");
		let write = dir.path().join("mods-data");
		let huge = "VSync = 1\n".repeat((MOST_FILE as usize / 10) + 10);
		plant(&write, "springsettings.cfg", &huge);
		plant(&write, "uikeys.txt", "bind esc quitmenu\n");
		let capture = capture(
			&settings,
			&ModSet::of(&[pinned("dev/a")]),
			&write,
			"2026-10-23T10:00:00Z",
		)
		.unwrap();
		assert!(!capture.after().join("springsettings.cfg").exists());
		assert!(capture.after().join("uikeys.txt").is_file());
	}

	/// A link the game made in its write directory is never followed: what
	/// it points at is the player's, outside the sandbox, and copying it in
	/// would hand it to the next game of the set.
	#[test]
	fn a_linked_directory_is_not_followed() {
		let dir = tempfile::tempdir().unwrap();
		let settings = dir.path().join("mods-settings");
		let write = dir.path().join("mods-data");
		let open = dir.path().join("open");
		plant(
			&open,
			"LuaUI/Config/BYAR.lua",
			"return {\n\torder = {\n\t\tChat = 1,\n\t},\n}\n",
		);
		std::fs::create_dir_all(write.join("LuaUI")).unwrap();
		#[cfg(windows)]
		junction::create(
			open.join("LuaUI").join("Config"),
			write.join("LuaUI").join("Config"),
		)
		.unwrap();
		#[cfg(unix)]
		std::os::unix::fs::symlink(
			open.join("LuaUI").join("Config"),
			write.join("LuaUI").join("Config"),
		)
		.unwrap();
		assert!(
			write.join("LuaUI/Config/BYAR.lua").is_file(),
			"the link is in place"
		);
		let capture = capture(
			&settings,
			&ModSet::of(&[pinned("dev/a")]),
			&write,
			"2026-10-23T10:00:00Z",
		)
		.unwrap();
		assert!(!capture.after().join("LuaUI/Config/BYAR.lua").exists());
	}
}
