//! The widget files actually on disk, and which published widget each one is.
//!
//! BAR's config knows widgets by `GetInfo().name` and nothing else, so it cannot
//! tell a player's homebrewed "Dont Stand in Fire" from the one on the usage
//! page — or from a second copy under another file name. The files can. Each is
//! read for the name it declares and hashed the way the game hashes it, which
//! is the same value the replay telemetry reports, so a file either *is* a
//! published revision (the hash matches) or merely shares its name.
//!
//! **Where BAR looks, and nowhere else.** `barwidgets.lua` loads
//! `LuaUI/Widgets/*.lua` and one level of subfolders beneath it, from every data
//! directory the engine is given. A file two folders deep is never loaded, so it
//! is not reported as installed.
//!
//! **Why names collide in practice.** BAR loads user widgets before the game's
//! own, and the first *enabled* file to claim a name wins; any later file with
//! that name fails with "duplicate name". Every file of one name also shares a
//! single `order` entry, so switching one off switches them all off. That is the
//! behaviour a row has to explain when two files, or a file and a published
//! widget, answer to the same name.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::manage::{collapse_crlf, hash_of};

/// Where BAR loads user widgets from, under a data directory.
pub const WIDGET_DIR: &str = "LuaUI/Widgets";

/// Bounds a scan of a directory somebody has filled with an art pack.
pub const MAX_FILES: usize = 4000;
/// Widgets are kilobytes; the largest published one is under 250 KB.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// How far past `GetInfo` to look for its `name`. Real bodies are a dozen lines.
const INFO_WINDOW: usize = 2048;

/// One widget file on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LocalWidget {
	/// What its `GetInfo()` declares — the key BAR's config uses.
	pub name: String,
	/// Relative to its data directory, with forward slashes.
	pub file: String,
	/// The data directory it sits in.
	pub dir: String,
	/// Whether that is modlobby's own write directory.
	pub writable: bool,
	/// `VFS.CalculateHash` of the bytes as they are on disk.
	pub hash: String,
	/// The same with CRLF collapsed, which is what a Windows client reports.
	/// Equal to `hash` for a file with Unix line endings.
	pub hash_text: String,
}

impl LocalWidget {
	/// Whether this file is byte-for-byte a published revision.
	pub fn is_revision(&self, published: &str) -> bool {
		!published.is_empty() && (self.hash == published || self.hash_text == published)
	}
}

/// Every widget file BAR would load from these data directories, in order.
///
/// `dirs` pairs each directory with whether it is modlobby's writable one. Files
/// that declare no widget — includes, configs — are left out: they are not
/// something a player installs or switches off.
pub fn scan(dirs: &[(PathBuf, bool)]) -> Vec<LocalWidget> {
	let mut found = Vec::new();
	for (dir, writable) in dirs {
		let root = dir.join(WIDGET_DIR);
		for path in candidates(&root) {
			if found.len() >= MAX_FILES {
				return found;
			}
			if let Some(widget) = read(dir, &path, *writable) {
				found.push(widget);
			}
		}
	}
	found
}

/// `LuaUI/Widgets/*.lua` and `LuaUI/Widgets/*/*.lua`, sorted, as BAR walks them.
fn candidates(root: &Path) -> Vec<PathBuf> {
	let mut files = Vec::new();
	let Ok(entries) = std::fs::read_dir(root) else {
		return files;
	};
	let mut subdirs = Vec::new();
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			subdirs.push(path);
		} else if is_lua(&path) {
			files.push(path);
		}
	}
	files.sort();
	subdirs.sort();
	for sub in subdirs {
		let Ok(entries) = std::fs::read_dir(&sub) else {
			continue;
		};
		let mut inner: Vec<PathBuf> = entries
			.flatten()
			.map(|entry| entry.path())
			.filter(|path| path.is_file() && is_lua(path))
			.collect();
		inner.sort();
		files.extend(inner);
	}
	files
}

fn is_lua(path: &Path) -> bool {
	path.extension()
		.and_then(|ext| ext.to_str())
		.is_some_and(|ext| ext.eq_ignore_ascii_case("lua"))
}

fn read(dir: &Path, path: &Path, writable: bool) -> Option<LocalWidget> {
	if std::fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
		return None;
	}
	let data = std::fs::read(path).ok()?;
	let name = declared_name(&data)?;
	let file = path
		.strip_prefix(dir)
		.ok()?
		.components()
		.map(|part| part.as_os_str().to_string_lossy())
		.collect::<Vec<_>>()
		.join("/");
	Some(LocalWidget {
		name,
		file,
		dir: dir.display().to_string(),
		writable,
		hash: hash_of(&data),
		hash_text: hash_of(&collapse_crlf(&data)),
	})
}

/// The `name` a widget's `GetInfo` returns, or `None` for a file that is not one.
///
/// Read from a window after the declaration, so an unrelated `name =` elsewhere
/// in the file is not mistaken for the widget's own. The same rule the pipeline
/// uses to read published widgets, so both ends agree on what a name is.
pub fn declared_name(data: &[u8]) -> Option<String> {
	let at = find(data, b"GetInfo")?;
	let window = &data[at..data.len().min(at + INFO_WINDOW)];
	let mut from = 0;
	while let Some(offset) = find(&window[from..], b"name") {
		let start = from + offset;
		from = start + 4;
		let before = start.checked_sub(1).map(|i| window[i]);
		if before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
			continue;
		}
		let mut i = start + 4;
		while window.get(i).is_some_and(|b| *b == b' ' || *b == b'\t') {
			i += 1;
		}
		if window.get(i) != Some(&b'=') {
			continue;
		}
		i += 1;
		while window.get(i).is_some_and(|b| *b == b' ' || *b == b'\t') {
			i += 1;
		}
		let quote = *window.get(i)?;
		if quote != b'"' && quote != b'\'' {
			continue;
		}
		let rest = &window[i + 1..];
		let end = rest.iter().take(121).position(|b| *b == quote)?;
		let name = String::from_utf8_lossy(&rest[..end]).trim().to_owned();
		return (!name.is_empty()).then_some(name);
	}
	None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	haystack
		.windows(needle.len())
		.position(|window| window == needle)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_declared_name_is_read_from_get_info() {
		let source = b"-- name = \"not this\"\nfunction widget:GetInfo()\n  return { desc = 'x', name = \"Dont Stand in Fire\", author = 'a' }\nend\n";
		assert_eq!(declared_name(source).as_deref(), Some("Dont Stand in Fire"));
	}

	#[test]
	fn a_longer_key_ending_in_name_is_not_the_name() {
		let source = b"function widget:GetInfo() return { basename = 'x', name = 'Real' } end";
		assert_eq!(declared_name(source).as_deref(), Some("Real"));
	}

	#[test]
	fn a_file_without_get_info_is_not_a_widget() {
		assert_eq!(
			declared_name(b"local M = { name = 'helper' } return M"),
			None
		);
	}

	#[test]
	fn single_quotes_are_names_too() {
		let source = b"function widget:GetInfo() return { name='Flea Transport' } end";
		assert_eq!(declared_name(source).as_deref(), Some("Flea Transport"));
	}

	#[test]
	fn files_are_found_where_bar_looks_and_nowhere_deeper() {
		let dir = tempfile::tempdir().unwrap();
		let widgets = dir.path().join(WIDGET_DIR);
		std::fs::create_dir_all(widgets.join("folder/deeper")).unwrap();
		let body =
			|name: &str| format!("function widget:GetInfo() return {{ name = '{name}' }} end\n");
		std::fs::write(widgets.join("top.lua"), body("Top")).unwrap();
		std::fs::write(widgets.join("folder/inner.lua"), body("Inner")).unwrap();
		std::fs::write(widgets.join("folder/deeper/lost.lua"), body("Lost")).unwrap();
		std::fs::write(widgets.join("folder/helper.lua"), "return {}").unwrap();

		let found = scan(&[(dir.path().to_owned(), true)]);
		let names: Vec<_> = found.iter().map(|w| w.name.as_str()).collect();
		assert_eq!(names, ["Top", "Inner"]);
		assert_eq!(found[1].file, "LuaUI/Widgets/folder/inner.lua");
		assert!(found.iter().all(|w| w.writable));
	}

	#[test]
	fn a_windows_line_ending_file_matches_its_text_mode_hash() {
		let dir = tempfile::tempdir().unwrap();
		let widgets = dir.path().join(WIDGET_DIR);
		std::fs::create_dir_all(&widgets).unwrap();
		let crlf = b"function widget:GetInfo()\r\n return { name = 'W' }\r\nend\r\n";
		std::fs::write(widgets.join("w.lua"), crlf).unwrap();
		let found = &scan(&[(dir.path().to_owned(), false)])[0];
		let unix: Vec<u8> = crlf.iter().copied().filter(|b| *b != b'\r').collect();
		assert!(found.is_revision(&hash_of(&unix)));
		assert!(found.is_revision(&hash_of(crlf)));
		assert!(!found.is_revision(&hash_of(b"something else")));
	}

	#[test]
	fn two_files_with_one_name_are_both_reported() {
		// BAR loads one and rejects the other as a duplicate; the page has to
		// be able to say so, which starts with seeing both.
		let dir = tempfile::tempdir().unwrap();
		let widgets = dir.path().join(WIDGET_DIR);
		std::fs::create_dir_all(&widgets).unwrap();
		let body = "function widget:GetInfo() return { name = 'Dont Stand in Fire' } end\n";
		std::fs::write(widgets.join("a.lua"), body).unwrap();
		std::fs::write(widgets.join("b.lua"), format!("{body}-- tweaked\n")).unwrap();
		let found = scan(&[(dir.path().to_owned(), true)]);
		assert_eq!(found.len(), 2);
		assert_ne!(found[0].hash, found[1].hash);
	}
}
