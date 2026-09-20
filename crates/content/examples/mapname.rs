//! `cargo run -p content --example mapname -- <archive>`: the name a map
//! archive calls itself, which is the name a room has to use.
fn main() {
	for path in std::env::args().skip(1) {
		let at = std::path::PathBuf::from(&path);
		match content::map_name::of_archive(&at) {
			Some(name) => println!("{path}\n  -> {name:?}"),
			None => println!("{path}\n  -> (the archive does not say)"),
		}
	}
}
