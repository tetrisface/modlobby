//! Indexes a real BAR data directory and says what it found and how long it
//! took. Run it twice for a cold and a warm look:
//!
//! ```text
//! cargo run --release -p game-history --example index_real -- <data dir> [<history dir> [<view.json> [<my name>]]]
//! ```
//!
//! The history goes to `<history dir>`, by default a directory under the
//! system's temporary one, never the app's own. `<view.json>` gets the view
//! the webview would, as seen by `<my name>`.

use std::path::PathBuf;
use std::time::Instant;

fn main() {
	let mut args = std::env::args().skip(1);
	let Some(data) = args.next().map(PathBuf::from) else {
		eprintln!("usage: index_real <data dir> [<history dir> [<view.json> [<my name>]]]");
		std::process::exit(2);
	};
	let history = args
		.next()
		.map(PathBuf::from)
		.unwrap_or_else(|| std::env::temp_dir().join("modlobby-history-example"));
	let dump = args.next().map(PathBuf::from);
	let me = game_history::Identity {
		names: args.next().into_iter().collect(),
		ids: Vec::new(),
	};

	let library = content::Library::new(data);
	let store = game_history::Store::new(&history);

	let started = Instant::now();
	let before = store.load().expect("the history could not be read");
	let loaded = started.elapsed();

	let started = Instant::now();
	let mut read = 0;
	let index = game_history::refresh(&before, &store, &library, |step| read = step.done)
		.expect("the history could not be saved");
	let scanned = started.elapsed();

	let started = Instant::now();
	let view = game_history::view(&index, &me);
	let json = serde_json::to_vec(&view).expect("the view serialises");
	let viewed = started.elapsed();

	let unreadable = index
		.files
		.values()
		.filter(|seen| seen.game.is_none())
		.count();
	println!("history in {}", history.display());
	println!("loaded the index in {loaded:?}");
	println!(
		"read {read} replays in {scanned:?}: {} files, {unreadable} unreadable",
		index.files.len()
	);
	println!(
		"{} games, {} settings; the view is {} KB, made in {viewed:?}",
		index.games.len(),
		index.settings.len(),
		json.len() / 1024
	);
	if let Some(path) = dump {
		std::fs::write(&path, &json).expect("the view could not be written");
		println!("view written to {}", path.display());
	}
}
