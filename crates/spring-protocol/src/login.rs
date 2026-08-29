//! `LOGIN` as teiserver parses it (`spring_in.ex` `do_handle("LOGIN", …)`):
//!
//! ```text
//! LOGIN <user> <base64(md5(pw))> 0 * <lobby>\t<lobby_hash>\t<flags>
//! ```
//!
//! The literal `0` and `*` satisfy the server regex's cpu / local-ip groups.
//! teiserver keeps the leading `[a-zA-Z ]+` of `<lobby>` as the client name
//! (`cache_user.ex` `do_login/4`); only `LuaLobby Chobby` and `skylobby`
//! receive the unfiltered `:partial` feed that carries other lobbies' joins.

use std::fmt;

use crate::hash;

/// Client name that puts the connection in teiserver's `:partial` bucket.
pub const CHOBBY_CLIENT: &str = "LuaLobby Chobby";
/// Chobby's compatibility flags; teiserver ignores the field.
pub const DEFAULT_FLAGS: &str = "b sp";

#[derive(Clone)]
pub struct LoginRequest {
    pub username: String,
    password_hash: String,
    /// Free text after `LuaLobby Chobby:`; truncated server-side, so purely informational.
    pub lobby_version: String,
    /// Chobby's `agent`: `"<macAddrHash> <sysInfoHash[..16]>"`; stored, not enforced.
    pub lobby_hash: String,
    pub flags: String,
}

impl LoginRequest {
    pub fn new(
        username: impl Into<String>,
        password: &str,
        lobby_version: impl Into<String>,
        lobby_hash: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            password_hash: hash::md5_base64(password),
            lobby_version: lobby_version.into(),
            lobby_hash: lobby_hash.into(),
            flags: DEFAULT_FLAGS.into(),
        }
    }

    pub fn line(&self) -> String {
        format!(
            "LOGIN {} {} 0 * {CHOBBY_CLIENT}:{}\t{}\t{}",
            self.username, self.password_hash, self.lobby_version, self.lobby_hash, self.flags
        )
    }
}

impl fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginRequest")
            .field("username", &self.username)
            .field("lobby_version", &self.lobby_version)
            .field("lobby_hash", &self.lobby_hash)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_line_matches_teiserver_regex_shape() {
        let req = LoginRequest::new("alice", "password", "modlobby 0.1", "abc def");
        assert_eq!(
            req.line(),
            "LOGIN alice X03MO1qnZdYdgyfeuILPmQ== 0 * LuaLobby Chobby:modlobby 0.1\tabc def\tb sp"
        );
    }

    #[test]
    fn debug_output_redacts_the_hash() {
        let req = LoginRequest::new("alice", "password", "v", "h");
        assert!(!format!("{req:?}").contains("X03MO1qnZdYdgyfeuILPmQ=="));
    }
}
