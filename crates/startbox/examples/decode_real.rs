//! Decodes the startbox modoptions out of a real `optionsPresets.json`.
//!
//! ```sh
//! cargo run -p startbox --example decode_real -- <path to optionsPresets.json>
//! ```

fn main() {
    let path = std::env::args().nth(1).expect("a path");
    let raw = std::fs::read_to_string(&path).expect("readable");
    let file: serde_json::Value = serde_json::from_str(&raw).expect("json");

    for (name, preset) in file.as_object().expect("object") {
        let Some(mods) = preset.get("Modoptions").and_then(|m| m.as_object()) else {
            continue;
        };
        let get = |key: &str| {
            mods.get(key)
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
        };

        if let Some(raw) = get("mapmetadata_startbox_override") {
            match startbox::decode_override(raw) {
                Ok(one) => println!(
                    "{name}: override with {} boxes, first has {} points",
                    one.startboxes.len(),
                    one.startboxes.first().map_or(0, |b| b.poly.len())
                ),
                Err(err) => println!("{name}: override FAILED: {err}"),
            }
        }
        if let Some(raw) = get("mapmetadata_startboxes_set") {
            match startbox::decode_set(raw) {
                Ok(set) => {
                    let counts: Vec<String> = set
                        .iter()
                        .map(|(teams, one)| format!("{teams}→{}", one.startboxes.len()))
                        .collect();
                    println!("{name}: set for team counts {}", counts.join(", "));
                    // What the game would pick for a few room sizes.
                    for teams in [2, 3, 4, 8] {
                        match startbox::resolve(None, &set, teams) {
                            Some((held, source)) => println!(
                                "    {teams} teams -> {} boxes ({source:?})",
                                held.startboxes.len()
                            ),
                            None => println!("    {teams} teams -> engine start rects"),
                        }
                    }
                }
                Err(err) => println!("{name}: set FAILED: {err}"),
            }
        }
    }
}
