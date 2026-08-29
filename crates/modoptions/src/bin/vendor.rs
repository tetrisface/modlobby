//! Rewrites `app/src/data/modoptions.json` from the game submodule.
//!
//! Run it when `external/Beyond-All-Reason` moves:
//!
//! ```text
//! cargo run -p modoptions --bin vendor-modoptions
//! ```
//!
//! The output is committed, so the change BAR made shows up as a reviewable
//! diff instead of arriving silently at runtime.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const SOURCE: &str = "external/Beyond-All-Reason/modoptions.lua";
const TARGET: &str = "app/src/data/modoptions.json";

fn main() -> ExitCode {
    match vendor() {
        Ok(count) => {
            println!("wrote {count} options to {TARGET}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("vendor-modoptions: {message}");
            ExitCode::FAILURE
        }
    }
}

fn vendor() -> Result<usize, String> {
    let root = repo_root()?;
    let source = root.join(SOURCE);
    let lua = std::fs::read_to_string(&source)
        .map_err(|err| format!("reading {}: {err}", source.display()))?;

    let options = modoptions::parse(&lua).map_err(|err| err.to_string())?;
    let mut json = serde_json::to_string_pretty(&options).map_err(|err| err.to_string())?;
    json.push('\n');

    let target = root.join(TARGET);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    std::fs::write(&target, json).map_err(|err| format!("writing {}: {err}", target.display()))?;

    Ok(options.len())
}

/// This binary is run from anywhere in the tree, so walk up to the workspace.
fn repo_root() -> Result<PathBuf, String> {
    let start = Path::new(env!("CARGO_MANIFEST_DIR"));
    start
        .ancestors()
        .find(|dir| dir.join(SOURCE).exists())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!("cannot find {SOURCE}; is the Beyond-All-Reason submodule checked out?")
        })
}
