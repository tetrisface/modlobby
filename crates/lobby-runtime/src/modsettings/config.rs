//! The engine's `springsettings.cfg` as data: `key = value` lines, read as
//! the engine reads them, without the keys the sandbox pins or that point
//! the engine at files, and written back as modlobby's own lines.

/// Keys never captured, whatever a game wrote: the socket gate the sandbox
/// pins, and the keys that point the engine at directories and archives.
pub const NEVER: &[&str] = &[
	"LuaSocketEnabled",
	"TCPAllowConnect",
	"TCPAllowListen",
	"UDPAllowListen",
	"SpringData",
	"SpringDataRoot",
	"MenuArchive",
];

/// The settings in `text` as the engine reads them: the first definition
/// of a key wins, comments and blanks are nothing, and [`NEVER`] is left
/// out.
pub fn read(text: &str) -> Vec<(String, String)> {
	let mut seen: Vec<String> = Vec::new();
	let mut out = Vec::new();
	for line in text.lines() {
		let Some((key, value)) = recoil::window_mode::setting(line) else {
			continue;
		};
		let folded = key.to_ascii_lowercase();
		if seen.contains(&folded) {
			continue;
		}
		seen.push(folded);
		if NEVER.iter().any(|never| never.eq_ignore_ascii_case(key)) {
			continue;
		}
		out.push((key.to_owned(), value.to_owned()));
	}
	out
}

/// `keys` as a settings file: one `key = value` a line.
pub fn write(keys: &[(String, String)]) -> String {
	keys.iter()
		.map(|(key, value)| format!("{key} = {value}\n"))
		.collect()
}

/// Whether `value` is a number or a boolean: what may fold into the open
/// settings file. A string could be a path or an address.
pub fn is_plain(value: &str) -> bool {
	let value = value.trim();
	matches!(value, "true" | "false")
		|| (!value.is_empty()
			&& value
				.chars()
				.all(|c| c.is_ascii_digit() || matches!(c, '-' | '.'))
			&& value.parse::<f64>().is_ok())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn pinned_and_path_keys_are_never_captured() {
		let text = "LuaSocketEnabled = 1\nTCPAllowConnect = *\nSpringData = C:\\x\nFullscreen = 0\nMenuArchive = evil\n";
		assert_eq!(read(text), [("Fullscreen".to_owned(), "0".to_owned())]);
	}

	#[test]
	fn the_first_definition_wins_and_comments_are_dropped() {
		let text = "# a comment\n// another\nFullscreen = 0\n\nfullscreen = 1\nVSync=1\n";
		assert_eq!(
			read(text),
			[
				("Fullscreen".to_owned(), "0".to_owned()),
				("VSync".to_owned(), "1".to_owned())
			]
		);
		assert_eq!(write(&read(text)), "Fullscreen = 0\nVSync = 1\n");
	}

	#[test]
	fn only_numbers_and_booleans_are_plain() {
		for plain in ["0", "1", "2.5", "-3", "true", "false"] {
			assert!(is_plain(plain), "{plain}");
		}
		for text in ["C:\\x", "", "1; rm", "yes", "0x10"] {
			assert!(!is_plain(text), "{text}");
		}
	}
}
