//! What of a widget handler's config crosses between the player's own games
//! and a set of mods' games: which widgets are on, and each widget's
//! settings when they are nothing but numbers, switches and single words --
//! the shape of a layout or an option, never of a conversation or a list of
//! names -- and never the settings of a widget known to keep what it saw.
//!
//! The same rule reads both ways: into the sandbox before a launch, where it
//! keeps the player's own words from mod code, and out of it in a fold-in,
//! where it keeps a mod's words out of the player's files.

use super::lua::{Key, Lua, field, fields_of, key_text, set_field, table_of};

/// Widgets whose data is what they saw and heard: chat lines and input
/// history, the players of the last game, who is ignored, teammates' stats.
pub const KEPT_BACK: &[&str] = &["Chat", "Playernames API", "Ignore List API", "TeamStats"];

/// The longest a single word may be. BAR's unit names run past thirty.
pub const MOST_WORD: usize = 64;

/// One setting a game set to something its launch did not: where it is in
/// the widget's data, and what it is now.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
	pub widget: String,
	pub path: Vec<Key>,
	pub was: Option<Lua>,
	pub now: Lua,
}

/// `config` with `allowUserWidgets` and `order` as they are and, of `data`,
/// each widget's table that [`carries`]; nothing else of it. Fields keep
/// their order.
pub fn carried(config: &Lua) -> Lua {
	let Lua::Table(fields) = config else {
		return Lua::Table(Vec::new());
	};
	let kept = fields
		.iter()
		.filter_map(|(key, value)| match key_text(key).as_str() {
			"allowUserWidgets" | "order" => Some((key.clone(), value.clone())),
			"data" => Some((key.clone(), carried_data(value))),
			_ => None,
		})
		.collect();
	Lua::Table(kept)
}

/// Of `data`, each widget's table that [`carries`].
fn carried_data(data: &Lua) -> Lua {
	let Lua::Table(widgets) = data else {
		return Lua::Table(Vec::new());
	};
	Lua::Table(
		widgets
			.iter()
			.filter(|(key, value)| carries(&key_text(key), value))
			.cloned()
			.collect(),
	)
}

/// Whether `widget`'s settings cross: the widget is none of [`KEPT_BACK`],
/// and every key and value in `data` is a number, a boolean or a word.
pub fn carries(widget: &str, data: &Lua) -> bool {
	!KEPT_BACK.contains(&widget) && matches!(data, Lua::Table(_)) && plain(data)
}

/// Whether `value` is a number, a boolean or a word, or a table of those
/// under number and word keys.
fn plain(value: &Lua) -> bool {
	match value {
		Lua::Bool(_) | Lua::Number(_) => true,
		Lua::Str(text) => is_word(text),
		Lua::Table(fields) => fields.iter().all(|(key, value)| {
			let key_plain = match key {
				Key::Number(_) => true,
				Key::Name(text) | Key::Str(text) => is_word(text),
			};
			key_plain && plain(value)
		}),
	}
}

/// A string a setting may be: up to [`MOST_WORD`] ASCII letters, digits,
/// `_`, `.` and `-`, or nothing at all. No space, so never a sentence; none
/// of `|`, `;`, `/` or brackets, so never a packed list, a path or a name
/// with a clan tag.
pub fn is_word(text: &str) -> bool {
	text.len() <= MOST_WORD
		&& text
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// The settings `now`'s data sets to something `was`'s did not, by widget
/// and place, over the widgets whose data `now` [`carries`]; a setting gone
/// from `now` is no change.
pub fn changed(was: &Lua, now: &Lua) -> Vec<Setting> {
	let before = fields_of(was, "data");
	let mut out = Vec::new();
	for (key, data) in fields_of(now, "data") {
		let widget = key_text(&key);
		if !carries(&widget, &data) {
			continue;
		}
		walk(
			&widget,
			&mut Vec::new(),
			field(&before, &widget),
			&data,
			&mut out,
		);
	}
	out
}

/// Every scalar under `now` that `was` does not hold the same at the same
/// place, as a setting at `path`.
fn walk(widget: &str, path: &mut Vec<Key>, was: Option<&Lua>, now: &Lua, out: &mut Vec<Setting>) {
	let Lua::Table(fields) = now else {
		if was != Some(now) {
			out.push(Setting {
				widget: widget.to_owned(),
				path: path.clone(),
				was: was.cloned(),
				now: now.clone(),
			});
		}
		return;
	};
	for (key, value) in fields {
		let held = match was {
			Some(Lua::Table(theirs)) => field(theirs, &key_text(key)),
			_ => None,
		};
		path.push(key.clone());
		walk(widget, path, held, value, out);
		path.pop();
	}
}

/// `own` with each of `settings` at its place in `data`, tables made on the
/// way, and the rest as it was.
pub fn with_settings(own: Lua, settings: &[Setting]) -> Lua {
	let mut fields = match own {
		Lua::Table(fields) => fields,
		_ => Vec::new(),
	};
	let mut data = table_of(&fields, "data");
	for setting in settings {
		let mut widget = table_of(&data, &setting.widget);
		put(&mut widget, &setting.path, &setting.now);
		set_field(&mut data, &setting.widget, Lua::Table(widget));
	}
	set_field(&mut fields, "data", Lua::Table(data));
	Lua::Table(fields)
}

/// Sets `value` at `path` among `fields`, tables made on the way, each key
/// kept as it is: a list's entry stays a number, never the text of one.
fn put(fields: &mut Vec<(Key, Lua)>, path: &[Key], value: &Lua) {
	let [key, rest @ ..] = path else {
		return;
	};
	let name = key_text(key);
	let held = if rest.is_empty() {
		value.clone()
	} else {
		let mut inner = table_of(fields, &name);
		put(&mut inner, rest, value);
		Lua::Table(inner)
	};
	match fields.iter_mut().find(|(there, _)| key_text(there) == name) {
		Some((_, slot)) => *slot = held,
		None => fields.push((key.clone(), held)),
	}
}

/// `path` as text: its keys joined by `.`.
pub fn path_text(path: &[Key]) -> String {
	path.iter().map(key_text).collect::<Vec<_>>().join(".")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::modsettings::lua::read;

	fn table(text: &str) -> Lua {
		read(&format!("return {text}")).unwrap()
	}

	#[test]
	fn a_word_is_short_and_plain() {
		assert!(is_word("raptor_allterrain_swarmer_emp_t2_v1"));
		assert!(is_word(""));
		assert!(is_word("ff0000"));
		assert!(is_word(&"x".repeat(MOST_WORD)));
		assert!(!is_word(&"x".repeat(MOST_WORD + 1)));
		assert!(!is_word("late game lag"));
		assert!(!is_word("187128|tetrisface;224860|someone"));
		assert!(!is_word("[BFB]Someone"));
		assert!(!is_word("music/original/loading/track.ogg"));
		assert!(!is_word("fW8ElZ0eTmsQG/AdQlSRLw=="));
		assert!(!is_word("ünïcode"));
	}

	#[test]
	fn a_widgets_settings_carry_when_they_are_numbers_switches_and_words() {
		assert!(carries("Top Bar", &table("{ autoHideButtons = true }")));
		assert!(carries(
			"Auto Group",
			&table("{ [1] = { \"raptor_allterrain_swarmer_emp_t2_v1\", 3 }, scale = -1.5 }")
		));
		assert!(carries("Scavenger Info", &table("{ numGames = 982 }")));
		assert!(carries("Empty", &table("{}")));
		assert!(
			!carries(
				"Widget Selector",
				&table("{ pickedSet = \"late game lag\", a = true }")
			),
			"one sentence drops the widget's settings whole"
		);
		assert!(
			!carries("Notes", &table("{ [\"hello everyone\"] = true }")),
			"a key is a string too"
		);
		assert!(!carries(
			"Deep",
			&table("{ a = { b = { c = \"187128|tetrisface\" } } }")
		));
		assert!(
			!carries("Scalar", &Lua::Number(1.0)),
			"settings are a table"
		);
	}

	#[test]
	fn the_widgets_that_keep_what_they_saw_never_carry() {
		for widget in KEPT_BACK {
			assert!(!carries(widget, &table("{ size = 1 }")), "{widget}");
		}
		assert!(
			carries("Chat Colours", &table("{ size = 1 }")),
			"by name, whole"
		);
	}

	#[test]
	fn carried_keeps_the_selection_and_the_plain_settings() {
		let config = table(
			"{\n\tallowUserWidgets = true,\n\tdata = {\n\t\tChat = {\n\t\t\tsize = 1,\n\t\t\tinputHistory = { \"hello everyone\" },\n\t\t},\n\t\t[\"Commander Name Tags\"] = {\n\t\t\tnameScaling = true,\n\t\t},\n\t\t[\"Widget Selector\"] = {\n\t\t\tpickedSet = \"late game lag\",\n\t\t},\n\t},\n\torder = {\n\t\tChat = 7,\n\t\t[\"API LOS Combiner GL4\"] = 0,\n\t},\n\treplaced = {\n\t\tChat = \"Chat 2\",\n\t},\n}",
		);
		let kept = carried(&config);
		let Lua::Table(fields) = &kept else {
			panic!("{kept:?}");
		};
		let names: Vec<String> = fields.iter().map(|(key, _)| key_text(key)).collect();
		assert_eq!(names, ["allowUserWidgets", "data", "order"]);
		assert_eq!(field(fields, "allowUserWidgets"), Some(&Lua::Bool(true)));
		let data = match field(fields, "data") {
			Some(Lua::Table(data)) => data,
			other => panic!("{other:?}"),
		};
		let widgets: Vec<String> = data.iter().map(|(key, _)| key_text(key)).collect();
		assert_eq!(widgets, ["Commander Name Tags"]);
		assert_eq!(
			field(fields, "order"),
			Some(&table(
				"{\n\tChat = 7,\n\t[\"API LOS Combiner GL4\"] = 0,\n}"
			))
		);
	}

	#[test]
	fn carried_of_a_config_without_data_is_just_its_selection() {
		let kept = carried(&table("{ order = { Chat = 1 } }"));
		assert_eq!(kept, table("{ order = { Chat = 1 } }"));
		assert_eq!(carried(&Lua::Number(1.0)), Lua::Table(Vec::new()));
	}

	#[test]
	fn changed_settings_are_the_scalars_now_sets_that_was_did_not() {
		let was = table(
			"{ data = { [\"Top Bar\"] = { scale = 1, a = { b = 1 } }, Chat = { size = 1 }, Gone = { x = 1 } } }",
		);
		let now = table(
			"{ data = { [\"Top Bar\"] = { scale = 2, a = { b = 1, c = \"x\" }, list = { 5 } }, Chat = { size = 2 }, New = { on = true }, Named = { who = \"[BFB]Someone\", n = 1 } } }",
		);
		let setting = |widget: &str, path: &[Key], was: Option<Lua>, now: Lua| Setting {
			widget: widget.into(),
			path: path.to_vec(),
			was,
			now,
		};
		let name = |text: &str| Key::Name(text.into());
		assert_eq!(
			changed(&was, &now),
			[
				setting(
					"Top Bar",
					&[name("scale")],
					Some(Lua::Number(1.0)),
					Lua::Number(2.0)
				),
				setting(
					"Top Bar",
					&[name("a"), name("c")],
					None,
					Lua::Str("x".into())
				),
				setting(
					"Top Bar",
					&[name("list"), Key::Number(1.0)],
					None,
					Lua::Number(5.0)
				),
				setting("New", &[name("on")], None, Lua::Bool(true)),
			],
			"Chat is kept back, Named holds a name, Gone is no change, a.b is the same"
		);
		assert!(changed(&now, &now).is_empty());
		assert!(changed(&Lua::Table(Vec::new()), &Lua::Table(Vec::new())).is_empty());
	}

	#[test]
	fn with_settings_puts_each_at_its_place_and_leaves_the_rest() {
		let own =
			table("{ data = { [\"Top Bar\"] = { scale = 1, other = 3 } }, order = { Chat = 1 } }");
		let name = |text: &str| Key::Name(text.into());
		let settings = [
			Setting {
				widget: "Top Bar".into(),
				path: vec![name("scale")],
				was: Some(Lua::Number(1.0)),
				now: Lua::Number(2.0),
			},
			Setting {
				widget: "Top Bar".into(),
				path: vec![name("a"), name("c")],
				was: None,
				now: Lua::Str("x".into()),
			},
			Setting {
				widget: "New".into(),
				path: vec![name("on")],
				was: None,
				now: Lua::Bool(true),
			},
		];
		assert_eq!(
			with_settings(own, &settings),
			table(
				"{ data = { [\"Top Bar\"] = { scale = 2, other = 3, a = { c = \"x\" } }, New = { on = true } }, order = { Chat = 1 } }"
			)
		);
		assert_eq!(
			with_settings(Lua::Table(Vec::new()), &settings[2..]),
			table("{ data = { New = { on = true } } }"),
			"a config with no data yet gets one"
		);
	}

	#[test]
	fn a_path_reads_as_its_keys_joined() {
		assert_eq!(
			path_text(&[
				Key::Name("a".into()),
				Key::Number(1.0),
				Key::Str("b c".into())
			]),
			"a.1.b c"
		);
	}
}
