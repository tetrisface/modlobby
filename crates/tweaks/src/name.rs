//! How BAR identifies a tweak. There is no name field: Chobby shows
//! `<length>:<hash>` plus the first 25 characters of a leading `--` comment
//! (`gui_modoptions_panel.lua:1068-1086`), so that comment is the name by
//! convention. Both are reproduced exactly so our labels match Chobby's.

use base64::prelude::*;
use sha2::{Digest, Sha512};

/// The leading comment, trimmed the way Chobby trims it, or `None` when the
/// payload does not start with one or it holds unprintable characters.
pub fn name(lua: &str) -> Option<String> {
    let line = lua.lines().next()?;
    let comment = line.strip_prefix("--")?;
    // Chobby's `line:sub(3, 27)`: 25 characters after the dashes.
    let cut: String = comment.chars().take(25).collect();
    let printable = cut
        .chars()
        .all(|c| c == ' ' || c.is_ascii_graphic() || c.is_alphanumeric());
    printable
        .then(|| cut.trim().to_owned())
        .filter(|n| !n.is_empty())
}

/// `<length>:<first 4 of base64(sha512-hex)>` — Chobby's tweak summary, over
/// the stored blob. `VFS.CalculateHash(value, 1)` is SHA-512 as a hex string
/// (`RecoilEngine/rts/Lua/LuaVFS.cpp:927-948`), which is then base64'd.
pub fn summary(blob: &str) -> String {
    let digest = Sha512::digest(blob.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let encoded = BASE64_URL_SAFE.encode(hex.as_bytes());
    format!("{}:{}", blob.len(), &encoded[..4])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_the_leading_comment_chobby_would_show() {
        assert_eq!(
            name("-- Sphere spawner\nlocal x = 1"),
            Some("Sphere spawner".into())
        );
        assert_eq!(
            name("--this name is far longer than twenty-five characters\n"),
            Some("this name is far longer t".into())
        );
        assert_eq!(name("local x = 1"), None);
        assert_eq!(name("--\nlocal x = 1"), None, "an empty comment is no name");
    }

    #[test]
    fn summary_matches_the_documented_shape() {
        let summary = summary("QUJD");
        let (len, hash) = summary.split_once(':').unwrap();
        assert_eq!(len, "4");
        assert_eq!(hash.len(), 4);
        // Stable for a given blob, and length-prefixed by the blob, not the Lua.
        assert_eq!(summary, super::summary("QUJD"));
        assert_ne!(summary, super::summary("QUJE"));
    }
}
