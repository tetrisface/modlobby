//! The command line and environment block `CreateProcessW` takes.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;

/// `program args…` as one line, quoted the way `CommandLineToArgvW` undoes
/// it: an argument with a space, tab or quote in it, or nothing in it, is
/// quoted; a quote inside is escaped, and the backslashes before it doubled.
/// NUL-terminated, for `CreateProcessW`.
pub(crate) fn command_line(program: &OsStr, args: &[&OsStr]) -> Vec<u16> {
	let mut line = String::new();
	push_quoted(&mut line, program);
	for arg in args {
		line.push(' ');
		push_quoted(&mut line, arg);
	}
	line.encode_utf16().chain([0]).collect()
}

fn push_quoted(line: &mut String, arg: &OsStr) {
	let text = arg.to_string_lossy();
	let plain = !text.is_empty() && !text.chars().any(|c| matches!(c, ' ' | '\t' | '"'));
	if plain {
		line.push_str(&text);
		return;
	}
	line.push('"');
	let mut backslashes = 0;
	for c in text.chars() {
		match c {
			'\\' => backslashes += 1,
			'"' => {
				line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
				backslashes = 0;
				line.push('"');
			}
			_ => {
				line.extend(std::iter::repeat_n('\\', backslashes));
				backslashes = 0;
				line.push(c);
			}
		}
	}
	line.extend(std::iter::repeat_n('\\', backslashes * 2));
	line.push('"');
}

/// `NAME=value` entries, each NUL-terminated and the block double-NUL
/// terminated, sorted case-insensitively as the loader expects.
pub(crate) fn environment_block(env: &[(OsString, OsString)]) -> Vec<u16> {
	let mut pairs: Vec<&(OsString, OsString)> = env.iter().collect();
	pairs.sort_by_key(|(key, _)| key.to_string_lossy().to_uppercase());
	let mut block = Vec::new();
	for (key, value) in pairs {
		block.extend(key.encode_wide());
		block.push(u16::from(b'='));
		block.extend(value.encode_wide());
		block.push(0);
	}
	block.push(0);
	block
}

#[cfg(test)]
mod tests {
	use super::*;

	fn text(wide: &[u16]) -> String {
		String::from_utf16_lossy(&wide[..wide.len() - 1])
	}

	#[test]
	fn an_argument_with_a_space_is_quoted() {
		let line = command_line(
			OsStr::new(r"C:\e\spring.exe"),
			&[OsStr::new("--write-dir"), OsStr::new(r"C:\my dir")],
		);
		assert_eq!(text(&line), r#"C:\e\spring.exe --write-dir "C:\my dir""#);
	}

	#[test]
	fn a_backslash_before_a_quote_is_doubled() {
		let line = command_line(OsStr::new("x"), &[OsStr::new(r#"a\"b"#)]);
		assert_eq!(text(&line), r#"x "a\\\"b""#);
	}

	#[test]
	fn the_block_is_sorted_and_double_nul_terminated() {
		let block = environment_block(&[
			(OsString::from("b"), OsString::from("2")),
			(OsString::from("A"), OsString::from("1")),
		]);
		assert_eq!(String::from_utf16_lossy(&block), "A=1\0b=2\0\0");
	}
}
