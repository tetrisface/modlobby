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

/// What teiserver greets with, always (`spring_out.ex:92`): how a server that
/// takes Chobby's fingerprint is told from one that refuses it.
pub const TEISERVER_VERSION: &str = "0.38-33-ga5f3b28";

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

impl LoginRequest {
	/// The line for a server that greeted with `server_version`.
	///
	/// teiserver keeps the lobby hash as the machine fingerprint its moderation
	/// goes by, so it gets Chobby's. uberserver refuses that one outright
	/// (`_validLoginSentence`: the middle field is SpringLobby's `<uint32 user
	/// id> <hex mac hash>`, and a base64 hash is neither), so every other
	/// server gets that shape, which teiserver would take as well.
	pub fn line_for(&self, server_version: &str) -> String {
		if server_version == TEISERVER_VERSION {
			return self.line();
		}
		format!(
			"LOGIN {} {} 0 * {}:{}\t{}\t{}",
			self.username,
			self.password_hash,
			self.client_name,
			self.lobby_version,
			spring_ids(&self.lobby_hash),
			self.flags
		)
	}
}

/// SpringLobby's pair, made from Chobby's fingerprint: the same machine
/// gives the same pair, and nothing else goes into it.
fn spring_ids(lobby_hash: &str) -> String {
	let digest = hash::md5_hex(lobby_hash);
	let user_id = u32::from_str_radix(&digest[..8], 16).unwrap_or_default();
	format!("{user_id} {}", &digest[8..24])
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

	/// uberserver's `_validLoginSentence`, which refused ours with "Invalid
	/// sentence format" on lobby.recoilengine.org (2026-09-22).
	fn uberserver_takes(line: &str) -> bool {
		let sentence = line.splitn(6, ' ').nth(5).expect("a sentence");
		let fields: Vec<&str> = sentence.split('\t').collect();
		let [lobby, ids, flags] = fields[..] else {
			return false;
		};
		let (user_id, mac) = ids.split_once(' ').unwrap_or((ids, "0"));
		lobby.len() <= 64
			&& ids.len() <= 40
			&& mac.len() <= 16
			&& u64::from_str_radix(mac, 16).is_ok()
			&& user_id.parse::<u32>().is_ok()
			&& flags.chars().all(|c| c.is_ascii_lowercase() || c == ' ')
	}

	#[test]
	fn teiserver_gets_chobbys_fingerprint_and_every_other_server_springlobbys() {
		let req = LoginRequest::new(
			"alice",
			"password",
			"0.1.22",
			"wPM0PNyjjxe/qpsPngTLmg== 6e3b2c9d0a1f4e57",
		);
		assert_eq!(req.line_for(TEISERVER_VERSION), req.line());
		assert!(!uberserver_takes(&req.line()), "the refusal, as it was");

		let spring = req.line_for("unknown");
		assert!(uberserver_takes(&spring), "{spring}");
		assert_eq!(
			spring,
			req.line_for("0.38-86-gb142493"),
			"the same machine, the same ids"
		);
		assert_eq!(stored_client_name(&spring), MODLOBBY_CLIENT);
	}

	#[test]
	fn a_taken_name_reads_the_same_on_either_server() {
		assert!(name_taken("Username already taken"));
		assert!(name_taken("Username is already in use."));
		// Not whose name it is: somebody's address.
		assert!(!name_taken("Email already in use"));
		assert!(!name_taken("Email address is already in use."));
		assert!(!name_taken(
			"too many recent registration attempts, please try again later"
		));
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

/// Whether a `REGISTRATIONDENIED` says the name is an account already:
/// teiserver's "Username already taken" (`cache_user.ex:369`), uberserver's
/// "Username is already in use." (`SQLUsers.py` `check_register_user`). Not
/// the email address either one refuses, which says nothing about whose
/// name it is.
pub fn name_taken(reason: &str) -> bool {
	let reason = reason.trim().to_ascii_lowercase();
	reason.starts_with("username") && (reason.contains("taken") || reason.contains("in use"))
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
