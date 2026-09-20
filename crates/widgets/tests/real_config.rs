//! Config surgery against the shape BAR actually writes.
//!
//! The committed fixture is synthetic, and deliberately so: a real `BYAR.lua` is
//! a hundred and sixty kilobytes of one player's settings, including the names
//! of everyone they last played with. None of that belongs in a repository.
//! What it does reproduce is the structure that breaks naive parsing — bracketed
//! and bare keys side by side, `data` blocks nested three deep, and braces
//! inside quoted strings.
//!
//! When a real config is present on the machine running the tests it is checked
//! too, read in place rather than copied. That is the case that matters most:
//! a fixture proves the parser handles what its author imagined.

use std::path::PathBuf;

use widgets::config::{ConfigError, WidgetConfig};

/// The synthetic fixture. Always present.
fn real() -> Option<String> {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/BYAR.sample.lua");
	std::fs::read_to_string(path).ok()
}

/// A real config, read where it lives and never copied. Absent on CI.
fn on_this_machine() -> Option<String> {
	let home = std::env::var_os("HOME")?;
	let path = PathBuf::from(home).join(".local/share/modlobby/data/LuaUI/Config/BYAR.lua");
	std::fs::read_to_string(path).ok()
}

/// Both, so the synthetic one is never the only thing exercised.
fn configs() -> Vec<String> {
	real().into_iter().chain(on_this_machine()).collect()
}

#[test]
fn a_real_config_on_this_machine_survives_a_full_round_trip() {
	let Some(text) = on_this_machine() else {
		eprintln!("no real config here; the synthetic fixture covers the rest");
		return;
	};
	let mut config = WidgetConfig::parse(text.clone());
	let states = config.states().expect("a real order table parses");
	assert!(
		states.len() > 200,
		"a real config lists hundreds of widgets, got {}",
		states.len()
	);

	let victim = states
		.iter()
		.find(|state| state.has_settings && state.name != "Widget Selector")
		.expect("something has settings");
	let removed = config.remove(&victim.name).unwrap();
	assert!(removed.order_entry && removed.settings);
	assert_eq!(braces(config.as_str()), 0, "braces still balance");
	assert_eq!(
		WidgetConfig::parse(config.as_str().to_owned())
			.states()
			.unwrap()
			.len(),
		states.len() - 1
	);
}

#[test]
fn every_config_reads_the_same_way() {
	for text in configs() {
		let config = WidgetConfig::parse(text);
		let states = config.states().expect("order table parses");
		assert!(states.iter().any(|state| state.name.contains(' ')));
		assert!(states.iter().any(|state| !state.name.contains(' ')));
	}
}

#[test]
fn a_brace_inside_a_quoted_setting_does_not_end_the_table() {
	// The trap: `format = "hp {cur}/{max} }"`. A naive scan ends `data` there
	// and then reads the rest of the file as though it were top level.
	for text in configs() {
		let config = WidgetConfig::parse(text);
		let states = config.states().unwrap();
		assert!(
			!states.iter().any(|state| state.name == "depth"
				|| state.name == "inner"
				|| state.name == "customScale"),
			"a nested key was read as a top-level widget"
		);
	}
}

#[test]
fn every_widget_in_the_order_table_is_read() {
	let Some(text) = real() else { return };
	let config = WidgetConfig::parse(text);
	let states = config.states().expect("order table parses");
	assert!(
		states.len() > 200,
		"a real config lists hundreds of widgets, got {}",
		states.len()
	);
	assert!(states.iter().any(|state| state.name == "AdvPlayersList"));
	assert!(states.iter().any(|state| state.name.contains(' ')));
}

#[test]
fn disabled_widgets_are_the_ones_at_zero() {
	let Some(text) = real() else { return };
	let config = WidgetConfig::parse(text);
	let states = config.states().unwrap();
	let disabled: Vec<_> = states.iter().filter(|state| !state.enabled()).collect();
	assert!(
		!disabled.is_empty(),
		"a real config has switched-off widgets"
	);
	assert!(disabled.iter().all(|state| state.order == 0));
}

#[test]
fn disabling_changes_one_number_and_nothing_else() {
	let Some(text) = real() else { return };
	let mut config = WidgetConfig::parse(text.clone());
	let target = config
		.states()
		.unwrap()
		.into_iter()
		.find(|state| state.enabled() && state.name != "Widget Selector")
		.expect("something is enabled");

	assert!(config.disable(&target.name).unwrap());
	assert_eq!(config.order_of(&target.name), Some(0));

	// The rest of the file is untouched, which is the point: this is 165 KB of
	// settings a player cannot get back.
	let before: Vec<&str> = text.lines().collect();
	let after: Vec<&str> = config.as_str().lines().collect();
	assert_eq!(before.len(), after.len(), "no lines added or removed");
	let changed: Vec<_> = before
		.iter()
		.zip(&after)
		.filter(|(a, b)| a != b)
		.map(|(a, b)| (*a, *b))
		.collect();
	assert_eq!(changed.len(), 1, "exactly one line changed: {changed:?}");
}

#[test]
fn enabling_puts_it_back_at_the_end_of_the_order() {
	let Some(text) = real() else { return };
	let mut config = WidgetConfig::parse(text);
	let target = config
		.states()
		.unwrap()
		.into_iter()
		.find(|state| !state.enabled())
		.expect("something is disabled");
	let highest = config
		.states()
		.unwrap()
		.iter()
		.map(|state| state.order)
		.max()
		.unwrap();

	assert!(config.enable(&target.name).unwrap());
	assert_eq!(config.order_of(&target.name), Some(highest + 1));
}

#[test]
fn deleting_takes_the_settings_with_it() {
	let Some(text) = real() else { return };
	let mut config = WidgetConfig::parse(text.clone());
	let target = config
		.states()
		.unwrap()
		.into_iter()
		.find(|state| state.has_settings)
		.expect("something has saved settings");

	let removed = config.remove(&target.name).unwrap();
	assert!(removed.order_entry, "removed from order");
	assert!(removed.settings, "removed from data");

	// Both halves gone: leaving `data` behind is what produces the zombie
	// entries already visible in real files.
	let after = WidgetConfig::parse(config.as_str().to_owned());
	let states = after.states().unwrap();
	assert!(!states.iter().any(|state| state.name == target.name));
	assert!(config.as_str().len() < text.len());
}

#[test]
fn the_file_still_parses_as_a_table_after_surgery() {
	let Some(text) = real() else { return };
	let mut config = WidgetConfig::parse(text);
	let states = config.states().unwrap();
	let before = states.len();
	let victim = states
		.iter()
		.find(|state| state.has_settings)
		.unwrap()
		.name
		.clone();
	config.remove(&victim).unwrap();

	let reparsed = WidgetConfig::parse(config.as_str().to_owned());
	assert_eq!(reparsed.states().unwrap().len(), before - 1);
	assert_eq!(braces(config.as_str()), 0, "braces still balance");
}

#[test]
fn the_widget_selector_refuses_to_be_switched_off() {
	let Some(text) = real() else { return };
	let mut config = WidgetConfig::parse(text);
	// BAR forces it back to 1 on load, so accepting the write would report a
	// disable that never happens.
	let outcome = config.disable("Widget Selector");
	assert!(matches!(outcome, Err(ConfigError::ForcedOn(_))));
}

#[test]
fn an_unknown_widget_changes_nothing() {
	let Some(text) = real() else { return };
	let mut config = WidgetConfig::parse(text.clone());
	assert!(!config.disable("No Such Widget At All").unwrap());
	assert_eq!(config.as_str(), text);
}

/// Net brace depth, ignoring quoted strings and comments.
fn braces(text: &str) -> i64 {
	let bytes = text.as_bytes();
	let mut depth = 0i64;
	let mut at = 0;
	while at < bytes.len() {
		match bytes[at] {
			b'-' if bytes.get(at + 1) == Some(&b'-') => {
				while at < bytes.len() && bytes[at] != b'\n' {
					at += 1;
				}
				continue;
			}
			quote @ (b'"' | b'\'') => {
				at += 1;
				while at < bytes.len() && bytes[at] != quote {
					at += if bytes[at] == b'\\' { 2 } else { 1 };
				}
			}
			b'{' => depth += 1,
			b'}' => depth -= 1,
			_ => {}
		}
		at += 1;
	}
	depth
}

#[test]
fn enabling_a_widget_bar_has_never_seen_adds_its_entry() {
	for text in configs() {
		let mut config = WidgetConfig::parse(text);
		let before = config.states().unwrap().len();
		assert!(config.enable("Brand New \"Quoted\" Widget").unwrap());

		let reparsed = WidgetConfig::parse(config.as_str().to_owned());
		let states = reparsed.states().unwrap();
		assert_eq!(states.len(), before + 1);
		let added = states
			.iter()
			.find(|state| state.name == "Brand New \"Quoted\" Widget")
			.expect("the new entry reads back under its own name");
		assert!(added.enabled());
		assert_eq!(braces(config.as_str()), 0);
	}
}

#[test]
fn enabling_twice_does_not_add_a_second_entry() {
	let text = real().expect("fixture");
	let mut config = WidgetConfig::parse(text);
	assert!(config.enable("Brand New Widget").unwrap());
	assert!(!config.enable("Brand New Widget").unwrap());
	assert!(
		config.states().is_ok(),
		"a duplicate entry would be refused here"
	);
}
