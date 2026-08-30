//! Reads the start script out of every replay in a directory and reports what
//! came back.
//!
//! Synthetic headers only prove the arithmetic. This reads what the engine
//! actually wrote, which is where the awkward cases live.
//!
//! ```sh
//! cargo run -p content --example read_replays -- <demos dir> [how many]
//! ```

fn main() {
    let dir = std::env::args().nth(1).expect("a demos directory");
    let limit: usize = std::env::args()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or(25);

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("readable")
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".sdfz") || name.ends_with(".sdf")
        })
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));

    let (mut ok, mut failed, mut with_boxes, mut with_tweaks) = (0, 0, 0, 0);
    for entry in entries.iter().take(limit) {
        match content::demo::script(entry.path()) {
            Ok(text) => {
                let script = recoil::script_read::parse(&text);
                let tweaks = script
                    .modoptions
                    .keys()
                    .filter(|key| key.starts_with("tweak"))
                    .count();
                ok += 1;
                with_boxes += usize::from(!script.start_boxes.is_empty());
                with_tweaks += usize::from(tweaks > 0);
                println!(
                    "{:>4} opts, {:>2} tweaks, {} boxes  {}",
                    script.modoptions.len(),
                    tweaks,
                    script.start_boxes.len(),
                    script.map().unwrap_or("(no map)")
                );
            }
            Err(err) => {
                failed += 1;
                println!("FAILED {}: {err}", entry.file_name().to_string_lossy());
            }
        }
    }
    println!("{ok} read, {failed} failed; {with_boxes} had start boxes, {with_tweaks} had tweaks");
}
