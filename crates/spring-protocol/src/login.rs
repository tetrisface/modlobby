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
//! you are in, so every other room's roster stays empty. `modlobby` has been a
//! `:partial` entry since teiserver PR #1514 (merged 2026-09-02, live on
//! server4 the day after); before that the default borrowed Chobby's name.

use std::fmt;

use crate::hash;

/// Our own `@optimisation_level` key, mapped to `:partial` like Chobby's.
pub const MODLOBBY_CLIENT: &str = "modlobby";
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
            client_name: MODLOBBY_CLIENT.into(),
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
        let req = LoginRequest::new("alice", "password", "0.1", "abc def");
        assert_eq!(
            req.line(),
            "LOGIN alice X03MO1qnZdYdgyfeuILPmQ== 0 * modlobby:0.1\tabc def\tb sp"
        );
    }

    #[test]
    fn client_name_replaces_the_default() {
        let req = LoginRequest::new("alice", "password", "0.1.0", "abc def")
            .client_name("LuaLobby Chobby");
        assert_eq!(
            req.line(),
            "LOGIN alice X03MO1qnZdYdgyfeuILPmQ== 0 * LuaLobby Chobby:0.1.0\tabc def\tb sp"
        );
        assert_eq!(stored_client_name(&req.line()), "LuaLobby Chobby");
    }

    /// The separator, not the name, decides what teiserver ends up storing.
    #[test]
    fn the_colon_terminates_the_stored_client_name() {
        let default = LoginRequest::new("alice", "password", "0.1.0", "h");
        assert_eq!(stored_client_name(&default.line()), MODLOBBY_CLIENT);

        // The name survives a version that itself contains a space.
        let spaced_version = LoginRequest::new("alice", "password", "modlobby 0.1.0", "h");
        assert_eq!(stored_client_name(&spaced_version.line()), MODLOBBY_CLIENT);

        // What a space separator would cost: a name that matches no map key.
        let spaced = LoginRequest::new("alice", "password", "0.1.0", "h").client_name("modlobby ");
        assert_ne!(stored_client_name(&spaced.line()), MODLOBBY_CLIENT);
    }

    #[test]
    fn debug_output_redacts_the_hash() {
        let req = LoginRequest::new("alice", "password", "v", "h");
        assert!(!format!("{req:?}").contains("X03MO1qnZdYdgyfeuILPmQ=="));
    }
}

/// `REGISTER <username> <base64(md5(password))> <email>`.
///
/// teiserver's regex is `(\S+) (\S+) (\S+)` (`spring_in.ex:336`), so none of
/// the three may contain a space — which is already true of a valid name and
/// an email address. The password is hashed exactly as `LOGIN` hashes it.
pub fn register(username: &str, password: &str, email: &str) -> String {
    format!("REGISTER {username} {} {email}", hash::md5_base64(password))
}

/// `CONFIRMAGREEMENT <code>`, the code being the one the server emailed.
///
/// A fresh account is `unverified` until this arrives, and the server answers
/// a login from one with the agreement text rather than with a session.
pub fn confirm_agreement(code: &str) -> String {
    format!("CONFIRMAGREEMENT {code}")
}

/// Why a name would be refused, checked here so the answer is immediate.
///
/// Mirrors `CacheUser.valid_name?` (`cache_user.ex:350-364`) for the two rules
/// that are purely mechanical. The rest — reserved words, an acceptable-name
/// check, whether it is taken — only the server can answer, and it does.
pub fn name_problem(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("a username is required".into());
    }
    if name.len() > MAX_USERNAME {
        return Some(format!("at most {MAX_USERNAME} characters"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '[' | ']' | '_'))
    {
        return Some("only a-z, A-Z, 0-9, [, ] and _ are allowed".into());
    }
    None
}

/// teiserver's `teiserver.Username max length` default.
const MAX_USERNAME: usize = 20;

#[cfg(test)]
mod register_tests {
    use super::*;

    #[test]
    fn register_hashes_the_password_the_way_login_does() {
        // The same password and hash the `LOGIN` test uses, so that the two
        // being identical is visible rather than asserted.
        assert_eq!(
            register("alice", "password", "a@b.c"),
            "REGISTER alice X03MO1qnZdYdgyfeuILPmQ== a@b.c"
        );
    }

    #[test]
    fn confirming_carries_only_the_code() {
        assert_eq!(confirm_agreement("A1B2C3"), "CONFIRMAGREEMENT A1B2C3");
    }

    #[test]
    fn a_name_the_server_would_refuse_is_refused_here_first() {
        assert!(name_problem("alice").is_none());
        assert!(name_problem("Al_ice[1]").is_none());

        assert!(name_problem("").is_some());
        assert!(name_problem("has space").is_some());
        assert!(name_problem("çedilla").is_some());
        assert!(name_problem(&"a".repeat(21)).is_some());
        assert!(name_problem(&"a".repeat(20)).is_none());
    }
}
