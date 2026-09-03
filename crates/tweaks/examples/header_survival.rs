//! Runs the minifier over every tweak in a real `optionsPresets.json` and
//! reports whether each one's header block came through whole.
//!
//! Fixtures we write ourselves only prove what we already believed. This reads
//! what people have actually saved — 486 tweaks with headers, in one file here
//! — and is how the header-block rule was settled: they carry a name, their
//! authors, a documentation link, and sometimes splice markers like
//! `EDITP_CLONES_BEGIN` that a tool goes looking for later.
//!
//! ```sh
//! cargo run -p tweaks --example header_survival -- <path to optionsPresets.json>
//! ```

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(launcher_presets)
        .expect("pass the path to optionsPresets.json");
    let raw = std::fs::read_to_string(&path).expect("presets");
    let presets: serde_json::Value = serde_json::from_str(&raw).expect("json");
    let (mut checked, mut kept, mut lost, mut failed) = (0, 0, 0, 0);

    for (name, preset) in presets.as_object().expect("object") {
        let Some(mods) = preset.get("Modoptions").and_then(|m| m.as_object()) else {
            continue;
        };
        for (key, value) in mods {
            if !key.starts_with("tweak") {
                continue;
            }
            let Some(blob) = value.as_str() else { continue };
            if blob.len() < 40 {
                continue;
            }
            let kind = if key.starts_with("tweakunits") {
                tweaks::Kind::Units
            } else {
                tweaks::Kind::Defs
            };
            let decoded = match tweaks::base64url::decode(blob, kind) {
                Ok(decoded) => decoded.text,
                Err(err) => {
                    failed += 1;
                    println!("DECODE {name}/{key}: {err}");
                    continue;
                }
            };
            let heads = decoded
                .lines()
                .take_while(|l| l.trim_start().starts_with("--"))
                .count();
            if heads == 0 {
                continue;
            }
            checked += 1;
            match tweaks::lua::minify(&decoded, kind) {
                Ok(min) => {
                    let after = min
                        .lines()
                        .take_while(|l| l.trim_start().starts_with("--"))
                        .count();
                    if after == heads {
                        kept += 1
                    } else {
                        lost += 1;
                        println!("LOST {name} / {key}: {heads} -> {after}");
                    }
                }
                Err(err) => {
                    failed += 1;
                    println!("PARSE {name}/{key}: {err}");
                }
            }
        }
    }
    println!(
        "checked {checked} tweaks with headers: {kept} kept whole, {lost} lost lines, {failed} unparsed"
    );
}

/// The launcher's presets on Windows; anywhere else the path is an argument.
fn launcher_presets() -> Option<std::path::PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        std::path::PathBuf::from(local).join("Programs/Beyond-All-Reason/data/optionsPresets.json"),
    )
}
