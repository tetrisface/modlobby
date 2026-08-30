//! `LOGIN` as teiserver parses it (`spring_in.ex` `do_handle("LOGIN", …)`):
//!
//! ```text
//! LOGIN <user> <base64(md5(pw))> 0 * <client>:<version>\t<lobby_hash>\t<flags>
//! ```
//!
//! The literal `0` and `*` satisfy the server regex's cpu / local-ip groups.
//!
//! Of the lobby field teiserver keeps only the leading `^[a-zA-Z ]+` as the
//! client name (`cache_user.ex` `do_login/4`), then looks that up in
//! `spring_in.ex`'s `@optimisation_level` to decide how much of the event stream
//! the connection receives. **Space is inside that character class**, so the
//! version has to be held off by something else: `modlobby:0.1.0` stores
//! `modlobby`, while `modlobby 0.1.0` would store `modlobby ` — trailing space,
//! matching no key.
//!
//! An unlisted name falls through to `:full`, which despite the name is the
//! *most* filtered tier: `JOINEDBATTLE` / `LEFTBATTLE` arrive only for the room
//! you are in, so every other room's roster stays empty. That is why the default
//! is still [`CHOBBY_CLIENT`]; it retires once `modlobby` has its own `:partial`
//! entry upstream.

use std::fmt;

use crate::hash;

/// Chobby's name, which teiserver maps to `:partial`. modlobby's default until
/// it is listed in `@optimisation_level` itself.
pub const CHOBBY_CLIENT: &str = "LuaLobby Chobby";
/// Chobby's compatibility flags; teiserver ignores the field.
pub const DEFAULT_FLAGS: &str = "b sp";

#[derive(Clone)]
pub struct LoginRequest {
    pub username: String,
    password_hash: String,
    /// Announced identity, and what teiserver stores after its truncation.
    pub client_name: String,
    /// Free text after `<client>:`; truncated away server-side, so informational.
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
            client_name: CHOBBY_CLIENT.into(),
            lobby_version: lobby_version.into(),
            lobby_hash: lobby_hash.into(),
            flags: DEFAULT_FLAGS.into(),
        }
    }

    /// Announce a different client. The name must be letters and spaces only, or
    /// teiserver stores just the leading run of them — see the module docs.
    #[must_use]
    pub fn client_name(mut self, name: impl Into<String>) -> Self {
        self.client_name = name.into();
        self
    }

    pub fn line(&self) -> String {
        format!(
            "LOGIN {} {} 0 * {}:{}\t{}\t{}",
            self.username,
            self.password_hash,
            self.client_name,
            self.lobby_version,
            self.lobby_hash,
            self.flags
        )
    }
}

impl fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginRequest")
            .field("username", &self.username)
            .field("client_name", &self.client_name)
            .field("lobby_version", &self.lobby_version)
            .field("lobby_hash", &self.lobby_hash)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `cache_user.ex` `do_login/4` keeps of the lobby field:
    /// `Regex.run(~r/^[a-zA-Z\ ]+/, lobby_client)`.
    fn stored_client_name(line: &str) -> String {
        line.split(" 0 * ")
            .nth(1)
            .and_then(|rest| rest.split('\t').next())
            .expect("login line carries a lobby field")
            .chars()
            .take_while(|c| c.is_ascii_alphabetic() || *c == ' ')
            .collect()
    }

    #[test]
    fn login_line_matches_teiserver_regex_shape() {
        let req = LoginRequest::new("alice", "password", "modlobby 0.1", "abc def");
        assert_eq!(
            req.line(),
            "LOGIN alice X03MO1qnZdYdgyfeuILPmQ== 0 * LuaLobby Chobby:modlobby 0.1\tabc def\tb sp"
        );
    }

    #[test]
    fn client_name_replaces_the_chobby_default() {
        let req =
            LoginRequest::new("alice", "password", "0.1.0", "abc def").client_name("modlobby");
        assert_eq!(
            req.line(),
            "LOGIN alice X03MO1qnZdYdgyfeuILPmQ== 0 * modlobby:0.1.0\tabc def\tb sp"
        );
    }

    /// The separator, not the name, decides what teiserver ends up storing.
    #[test]
    fn the_colon_terminates_the_stored_client_name() {
        let named = LoginRequest::new("alice", "password", "0.1.0", "h").client_name("modlobby");
        assert_eq!(stored_client_name(&named.line()), "modlobby");

        // The default survives a version that itself contains a space.
        let default = LoginRequest::new("alice", "password", "modlobby 0.1.0", "h");
        assert_eq!(stored_client_name(&default.line()), CHOBBY_CLIENT);

        // What a space separator would cost: a name that matches no map key.
        let spaced = LoginRequest::new("alice", "password", "0.1.0", "h").client_name("modlobby ");
        assert_ne!(stored_client_name(&spaced.line()), "modlobby");
    }

    #[test]
    fn debug_output_redacts_the_hash() {
        let req = LoginRequest::new("alice", "password", "v", "h");
        assert!(!format!("{req:?}").contains("X03MO1qnZdYdgyfeuILPmQ=="));
    }
}
