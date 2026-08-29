//! The two hash forms the legacy protocol uses.

use base64::prelude::*;
use md5::{Digest, Md5};

/// `base64(md5(input))` — passwords (`VFS.CalculateHash(pw, 0)` in Chobby) and telemetry machine hashes.
pub fn md5_base64(input: &str) -> String {
    BASE64_STANDARD.encode(Md5::digest(input.as_bytes()))
}

/// Lowercase hex `md5(input)` — the form Chobby uses for `sysInfoHash`.
pub fn md5_hex(input: &str) -> String {
    Md5::digest(input.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(md5_base64("password"), "X03MO1qnZdYdgyfeuILPmQ==");
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
    }
}
