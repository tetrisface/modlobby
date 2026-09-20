//! What a map archive says about itself, before a room is opened on it.
//!
//! `cargo run -p content --example mapname -- <archive>...`
//!
//! Two things, both of which have stopped a game from starting:
//!
//! - the name. A room names its map the way the engine does, and that name is
//!   written in the archive and nowhere else. Handed the file name instead,
//!   the engine refuses: `Dependent archive "frostycove_v1.13" (resolved to
//!   "FrostyCove_v1.13") not found`.
//! - what the map says it depends on. An entry of `""` -- which a common
//!   mapinfo template ships -- is taken literally by
//!   `ArchiveScanner.cpp:169`, and the engine dies looking for an archive
//!   with no name: `Dependent archive "" (resolved to "") not found`. The map
//!   is at fault and only the map can fix it, so it is worth knowing before
//!   everybody has downloaded it.

fn main() {
	let paths: Vec<String> = std::env::args().skip(1).collect();
	if paths.is_empty() {
		eprintln!("usage: mapname <archive>...");
		return;
	}
	for path in paths {
		let at = std::path::PathBuf::from(&path);
		println!("{path}");
		match content::map_name::of_archive(&at) {
			Some(name) => println!("  name    {name:?}"),
			None => {
				println!("  name    (the archive does not say; the file name would be guessed)")
			}
		}
		match content::map_name::dependencies(&at) {
			None => println!("  depends (the archive does not say)"),
			Some(deps) if deps.is_empty() => println!("  depends nothing"),
			Some(deps) => {
				println!("  depends {deps:?}");
				if deps.iter().any(|dep| dep.trim().is_empty()) {
					println!(
						"  BROKEN  one dependency has no name, so the engine will refuse to \
						 start this map: mapinfo.lua should say `depend = {{}}`, not `depend = {{\"\"}}`"
					);
				}
			}
		}
	}
}
