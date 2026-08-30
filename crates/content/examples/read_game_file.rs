//! Pulls one file out of the installed game and reports what came back.
//!
//! ```sh
//! cargo run -p content --example read_game_file -- <data dir> "<game name>" [file]
//! ```

fn main() {
    let dir = std::env::args().nth(1).expect("a BAR data directory");
    let game = std::env::args().nth(2).expect("a game version name");
    let file = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "modoptions.lua".to_owned());

    let library = content::Library::new(&dir);
    match library.game_file(&game, &file) {
        Some(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            println!("{file}: {} bytes", bytes.len());
            match modoptions::parse(&text) {
                Ok(options) => {
                    let described = options.iter().filter(|o| !o.desc.is_empty()).count();
                    println!(
                        "parsed {} options, {described} with descriptions",
                        options.len()
                    );
                    for option in options.iter().take(3) {
                        println!("  {} — {}", option.key, option.name);
                    }
                }
                Err(err) => println!("parse failed: {err}"),
            }
        }
        None => println!("{game}: not installed, or {file} is not in it"),
    }
}
