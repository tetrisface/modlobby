//! Reads a real `optionsPresets.json` and reports what came across.
//!
//! ```sh
//! cargo run -p presets --example import_real -- <path>
//! ```

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("a path to optionsPresets.json");
    let text = std::fs::read_to_string(&path).expect("readable");
    let presets = presets::chobby::read(&text, 1_000).expect("parsed");

    let mut with_map = 0;
    let mut with_boxes = 0;
    let mut with_bots = 0;
    let mut options = 0;
    let mut tweaks = 0;
    for preset in &presets {
        with_map += usize::from(preset.map.is_some());
        with_boxes += usize::from(!preset.start_boxes.is_empty());
        with_bots += usize::from(!preset.bots.is_empty());
        options += preset.option_count();
        tweaks += preset.tweak_count();
    }
    println!(
        "{} presets: {with_map} with a map, {with_boxes} with start boxes, {with_bots} with bots",
        presets.len()
    );
    println!("{options} modoptions in total, {tweaks} filled tweak slots");

    // Everything must survive a round trip through our own file and back.
    let there = presets::chobby::merge_into("{}", &presets).expect("exported");
    let back = presets::chobby::read(&there, 1_000).expect("re-read");
    let mut differ = 0;
    for (before, after) in presets.iter().zip(&back) {
        if before.modoptions != after.modoptions || before.map != after.map {
            differ += 1;
            println!("DIFFERS {}", before.name);
        }
    }
    println!("{differ} presets changed across an export and re-import");
}
