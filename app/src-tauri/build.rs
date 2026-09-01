fn main() {
    println!("cargo::rustc-env=MODLOBBY_COMMIT={}", commit());
    println!("cargo::rerun-if-env-changed=MODLOBBY_COMMIT");
    // A new commit is a new stamp. `HEAD` names a branch, and the branch file
    // is what moves on commit, so both are watched.
    println!("cargo::rerun-if-changed=../../.git/HEAD");
    if let Ok(head) = std::fs::read_to_string("../../.git/HEAD")
        && let Some(reference) = head.trim().strip_prefix("ref: ")
    {
        println!("cargo::rerun-if-changed=../../.git/{reference}");
    }
    tauri_build::build()
}

/// The short hash of the commit being built.
///
/// A commit cannot contain its own hash, so this can never be a field in a
/// committed file: CI states it outright, a working tree asks git, and a
/// source tree with neither says so rather than guessing.
fn commit() -> String {
    if let Ok(given) = std::env::var("MODLOBBY_COMMIT") {
        return given;
    }
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_else(|| "dev".into())
}
