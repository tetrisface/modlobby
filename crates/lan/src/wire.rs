//! The lines a client sends, read; the lines a server sends, written.
//!
//! `spring-protocol` reads server lines and writes client lines, because it
//! is a client. This is its mirror, for the handful of commands a room needs.
//! Shapes come from teiserver's `spring_in.ex` (what clients send) and
//! `spring_out.ex` (what they expect back), and every emitted line is one the
//! parser in `spring_protocol::event` turns into the event the room wants.

use spring_protocol::RawMessage;

/// One line from a client, as far as a LAN room cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientLine {
	/// `LOGIN <name> <password> <cpu> <ip> <lobby>` — only the name and the
	/// lobby (agent) are kept; there are no accounts.
	Login {
		name: String,
		agent: String,
	},
	Stls,
	Exit,
	Ping,
	/// `JOINBATTLE <id> <password|empty> <script password>`.
	JoinBattle {
		id: u32,
		password: Option<String>,
		script_password: String,
	},
	LeaveBattle,
	/// `MYSTATUS <bits>`.
	MyStatus(u32),
	/// `MYBATTLESTATUS <bits> <colour>`.
	MyBattleStatus {
		bits: u32,
		colour: u32,
	},
	SayBattle(String),
	SayBattleEx(String),
	/// `ADDBOT <name> <bits> <colour> <ai>`.
	AddBot {
		name: String,
		bits: u32,
		colour: u32,
		ai: String,
	},
	/// `UPDATEBOT <name> <bits> <colour>`.
	UpdateBot {
		name: String,
		bits: u32,
		colour: u32,
	},
	RemoveBot(String),
	/// `KICKFROMBATTLE <name>`.
	Kick(String),
	/// Anything else: telemetry, friend lists, channels. Ignored, never
	/// refused — the client sent it in good faith and expects no answer.
	Other(String),
}

impl ClientLine {
	pub fn parse(line: &str) -> Self {
		let raw = RawMessage::parse(line);
		let a = raw.args.as_str();
		let mut words = a.split_whitespace();
		match raw.command.as_str() {
			"LOGIN" => {
				let name = words.next().unwrap_or("").to_owned();
				// name password cpu ip, then the lobby with its own spaces.
				let agent = a
					.splitn(5, ' ')
					.nth(4)
					.map(|rest| rest.split('\t').next().unwrap_or(rest))
					.unwrap_or("")
					.trim()
					.to_owned();
				Self::Login { name, agent }
			}
			"STLS" => Self::Stls,
			"EXIT" => Self::Exit,
			"PING" => Self::Ping,
			"JOINBATTLE" => {
				let id = words.next().and_then(|id| id.parse().ok()).unwrap_or(0);
				let password = words
					.next()
					.filter(|password| *password != "empty" && !password.is_empty())
					.map(str::to_owned);
				let script_password = words.next().unwrap_or("").to_owned();
				Self::JoinBattle {
					id,
					password,
					script_password,
				}
			}
			"LEAVEBATTLE" => Self::LeaveBattle,
			"MYSTATUS" => Self::MyStatus(words.next().and_then(|b| b.parse().ok()).unwrap_or(0)),
			"MYBATTLESTATUS" => Self::MyBattleStatus {
				bits: words.next().and_then(|b| b.parse().ok()).unwrap_or(0),
				colour: words.next().and_then(|c| c.parse().ok()).unwrap_or(0),
			},
			"SAYBATTLE" => Self::SayBattle(a.to_owned()),
			"SAYBATTLEEX" => Self::SayBattleEx(a.to_owned()),
			"ADDBOT" => {
				let mut parts = a.splitn(4, ' ');
				let name = parts.next().unwrap_or("").to_owned();
				let bits = parts.next().and_then(|b| b.parse().ok()).unwrap_or(0);
				let colour = parts.next().and_then(|c| c.parse().ok()).unwrap_or(0);
				let ai = parts.next().unwrap_or("").to_owned();
				Self::AddBot {
					name,
					bits,
					colour,
					ai,
				}
			}
			"UPDATEBOT" => Self::UpdateBot {
				name: words.next().unwrap_or("").to_owned(),
				bits: words.next().and_then(|b| b.parse().ok()).unwrap_or(0),
				colour: words.next().and_then(|c| c.parse().ok()).unwrap_or(0),
			},
			"REMOVEBOT" => Self::RemoveBot(a.trim().to_owned()),
			"KICKFROMBATTLE" => Self::Kick(a.trim().to_owned()),
			_ => Self::Other(raw.command),
		}
	}
}

/// The greeting every connection gets first. `0.38` is what teiserver says
/// and what the client's tests are written against; the UDP port is the
/// engine's.
pub fn tas_server(engine_port: u16) -> String {
	format!("TASSERVER 0.38 * {engine_port} 0")
}

/// `ADDUSER <name> <country> <id> <lobby>`.
pub fn add_user(name: &str, id: u64, agent: &str) -> String {
	format!("ADDUSER {name} ?? {id} {agent}")
}

pub fn client_status(name: &str, bits: u32) -> String {
	format!("CLIENTSTATUS {name} {bits}")
}

/// One battle, announced. Ten space-separated fields, then the engine name and
/// four tab-separated fields after it: the shape `parse_battle_opened` reads.
#[allow(clippy::too_many_arguments)]
pub fn battle_opened(
	id: u32,
	founder: &str,
	ip: &str,
	port: u16,
	max_players: u32,
	passworded: bool,
	engine_version: &str,
	map: &str,
	title: &str,
	game: &str,
) -> String {
	format!(
		"BATTLEOPENED {id} 0 0 {founder} {ip} {port} {max_players} {} 0 0 Recoil\t{engine_version}\t{map}\t{title}\t{game}",
		u8::from(passworded)
	)
}

pub fn update_battle_info(id: u32, spectators: u32, locked: bool, map: &str) -> String {
	format!(
		"UPDATEBATTLEINFO {id} {spectators} {} 0 {map}",
		u8::from(locked)
	)
}

pub fn joined_battle(id: u32, name: &str, script_password: Option<&str>) -> String {
	match script_password {
		Some(password) => format!("JOINEDBATTLE {id} {name} {password}"),
		None => format!("JOINEDBATTLE {id} {name}"),
	}
}

pub fn left_battle(id: u32, name: &str) -> String {
	format!("LEFTBATTLE {id} {name}")
}

pub fn client_battle_status(name: &str, bits: u32, colour: u32) -> String {
	format!("CLIENTBATTLESTATUS {name} {bits} {colour}")
}

pub fn said_battle(name: &str, text: &str) -> String {
	format!("SAIDBATTLE {name} {text}")
}

pub fn said_battle_ex(name: &str, text: &str) -> String {
	format!("SAIDBATTLEEX {name} {text}")
}

/// `SETSCRIPTTAGS k=v\tk=v`, the whole table in one line as teiserver sends it.
pub fn set_script_tags<'a>(tags: impl IntoIterator<Item = (&'a String, &'a String)>) -> String {
	let body: Vec<String> = tags
		.into_iter()
		.map(|(key, value)| format!("{key}={value}"))
		.collect();
	format!("SETSCRIPTTAGS {}", body.join("\t"))
}

pub fn add_bot(id: u32, name: &str, owner: &str, bits: u32, colour: u32, ai: &str) -> String {
	format!("ADDBOT {id} {name} {owner} {bits} {colour} {ai}")
}

pub fn update_bot(id: u32, name: &str, bits: u32, colour: u32) -> String {
	format!("UPDATEBOT {id} {name} {bits} {colour}")
}

pub fn remove_bot(id: u32, name: &str) -> String {
	format!("REMOVEBOT {id} {name}")
}

pub fn add_start_rect(ally: u8, left: u16, top: u16, right: u16, bottom: u16) -> String {
	format!("ADDSTARTRECT {ally} {left} {top} {right} {bottom}")
}

pub fn remove_start_rect(ally: u8) -> String {
	format!("REMOVESTARTRECT {ally}")
}

#[cfg(test)]
mod tests {
	use super::*;
	use spring_protocol::ServerEvent;

	fn parse(line: &str) -> Option<ServerEvent> {
		Some(ServerEvent::from(RawMessage::parse(line)))
	}

	#[test]
	fn a_login_keeps_the_name_and_the_lobby_only() {
		let line = "LOGIN alice * 0 * modlobby:0.1\tabc def\tb sp";
		assert_eq!(
			ClientLine::parse(line),
			ClientLine::Login {
				name: "alice".into(),
				agent: "modlobby:0.1".into()
			}
		);
	}

	#[test]
	fn a_join_reads_empty_as_no_password() {
		assert_eq!(
			ClientLine::parse("JOINBATTLE 1 empty 4242"),
			ClientLine::JoinBattle {
				id: 1,
				password: None,
				script_password: "4242".into()
			}
		);
		assert_eq!(
			ClientLine::parse("JOINBATTLE 1 hunter2 4242"),
			ClientLine::JoinBattle {
				id: 1,
				password: Some("hunter2".into()),
				script_password: "4242".into()
			}
		);
	}

	#[test]
	fn an_ai_name_takes_no_spaces_but_the_ai_may() {
		assert_eq!(
			ClientLine::parse("ADDBOT Bot1 4194304 255 Simple AI"),
			ClientLine::AddBot {
				name: "Bot1".into(),
				bits: 4194304,
				colour: 255,
				ai: "Simple AI".into()
			}
		);
	}

	#[test]
	fn what_the_room_does_not_speak_is_named_not_refused() {
		assert_eq!(
			ClientLine::parse("c.telemetry.update_client_property hardware:x y z"),
			ClientLine::Other("c.telemetry.update_client_property".into())
		);
	}

	/// Every line the room emits must come back out of the client's parser
	/// as the event the room meant, or the client draws nothing.
	#[test]
	fn emitted_lines_are_what_the_client_reads() {
		use spring_protocol::ServerEvent as E;
		let opened = battle_opened(
			1,
			"ann",
			"192.168.1.5",
			8452,
			8,
			true,
			"2026.07.04",
			"Supreme Isthmus v2.1",
			"Ann's LAN game",
			"Beyond All Reason test-31357-b06bb1a",
		);
		match parse(&opened) {
			Some(E::BattleOpened(b)) => {
				assert_eq!(b.founder, "ann");
				assert_eq!(b.ip, "192.168.1.5");
				assert_eq!(b.port, 8452);
				assert!(b.passworded);
				assert_eq!(b.engine_version, "2026.07.04");
				assert_eq!(b.map_name, "Supreme Isthmus v2.1");
				assert_eq!(b.title, "Ann's LAN game");
				assert_eq!(b.game_name, "Beyond All Reason test-31357-b06bb1a");
			}
			other => panic!("{other:?}"),
		}
		assert!(matches!(
			parse(&tas_server(8452)),
			Some(E::Welcome { udp_port: 8452, .. })
		));
		assert!(matches!(
			parse(&add_user("ann", 3, "modlobby:0.1")),
			Some(E::AddUser { ref name, ref lobby_client, .. }) if name == "ann" && lobby_client == "modlobby:0.1"
		));
		assert!(matches!(
			parse(&joined_battle(1, "bob", Some("4242"))),
			Some(E::JoinedBattle { id: 1, ref name, script_password: Some(ref sp) }) if name == "bob" && sp == "4242"
		));
		assert!(matches!(
			parse(&update_battle_info(1, 2, false, "Comet Catcher Remake 1.8")),
			Some(E::UpdateBattleInfo { spectator_count: 2, ref map_name, .. }) if map_name == "Comet Catcher Remake 1.8"
		));
		let tags = [("game/modoptions/startmetal".to_owned(), "1000".to_owned())];
		assert!(matches!(
			parse(&set_script_tags(tags.iter().map(|(k, v)| (k, v)))),
			Some(E::SetScriptTags { ref tags }) if tags.iter().any(|(k, v)| k == "game/modoptions/startmetal" && v == "1000")
		));
		assert!(matches!(
			parse(&add_bot(1, "Bot1", "ann", 4194304, 255, "Simple AI")),
			Some(E::AddBot { id: 1, ref ai, .. }) if ai == "Simple AI"
		));
		assert!(matches!(
			parse(&add_start_rect(0, 0, 0, 200, 50)),
			Some(E::AddStartRect {
				ally_team: 0,
				bottom: 50,
				..
			})
		));
		assert!(matches!(
			parse(&said_battle("bob", "hello there")),
			Some(E::SaidBattle { ref text, .. }) if text == "hello there"
		));
	}
}
