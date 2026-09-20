//! The room, with no sockets in it.
//!
//! Everything the server decides is decided here, on lines in and lines out,
//! so a whole session can be replayed in a test with no network. `serve`
//! owns the sockets and does nothing but carry lines between them and this.
//!
//! One battle, opened when the room is, founded by whoever hosts. The founder
//! is a client like any other: it logs in, joins its own battle and takes a
//! seat over the same lines a guest does, and its in-game bit is what tells
//! everyone the game has started. What the founder alone may do is said in
//! chat — the `!` dialect the online room already speaks to SPADS — so the
//! room needs no lines of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use spring_protocol::BattleStatus;

use crate::wire::{self, ClientLine};
use crate::{BATTLE_ID, ENGINE_PORT};

/// One connection, as `serve` numbers them.
pub type Peer = u64;

/// A line to send, and to whom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
	To(Peer, String),
	/// Everyone logged in.
	All(String),
	Close(Peer),
}

/// Who may join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Policy {
	Open,
	Password(String),
	/// Each joiner waits until the founder says `!accept <name>`.
	Approve,
}

/// What the room is opened with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
	pub founder: String,
	pub title: String,
	pub engine_version: String,
	pub game: String,
	pub map: String,
	pub max_players: u32,
	pub policy: Policy,
}

#[derive(Debug, Clone)]
struct User {
	name: String,
	agent: String,
	/// `CLIENTSTATUS` bits: in game, away.
	status: u32,
}

#[derive(Debug, Clone)]
struct Member {
	peer: Peer,
	script_password: String,
	bits: u32,
	colour: u32,
}

#[derive(Debug, Clone)]
struct Bot {
	id: u32,
	name: String,
	owner: String,
	bits: u32,
	colour: u32,
	ai: String,
}

#[derive(Debug, Clone)]
struct Pending {
	peer: Peer,
	name: String,
	script_password: String,
}

/// The room's whole state.
#[derive(Debug, Clone)]
pub struct Room {
	config: Config,
	/// Connections that have said hello, logged in or not.
	greeted: BTreeSet<Peer>,
	users: BTreeMap<Peer, User>,
	/// In the battle, keyed by name; insertion order is player order.
	members: Vec<(String, Member)>,
	bots: Vec<Bot>,
	next_bot: u32,
	/// Ally team to `(left, top, right, bottom)` on the 0-200 grid.
	rects: BTreeMap<u8, (u16, u16, u16, u16)>,
	tags: BTreeMap<String, String>,
	pending: Vec<Pending>,
	refused: BTreeSet<String>,
	kicked: BTreeSet<String>,
	closed: bool,
}

impl Room {
	pub fn new(config: Config) -> Self {
		Self {
			config,
			greeted: BTreeSet::new(),
			users: BTreeMap::new(),
			members: Vec::new(),
			bots: Vec::new(),
			next_bot: 1,
			rects: BTreeMap::new(),
			tags: BTreeMap::new(),
			pending: Vec::new(),
			refused: BTreeSet::new(),
			kicked: BTreeSet::new(),
			closed: false,
		}
	}

	pub fn config(&self) -> &Config {
		&self.config
	}

	/// Whether the founder has left; a closed room takes nobody new.
	pub fn closed(&self) -> bool {
		self.closed
	}

	/// Names waiting for the founder's word, oldest first.
	pub fn pending(&self) -> Vec<String> {
		self.pending.iter().map(|p| p.name.clone()).collect()
	}

	/// Everyone in the battle: name, script password, status, colour.
	pub fn members(&self) -> impl Iterator<Item = (&str, &str, BattleStatus, u32)> {
		self.members.iter().map(|(name, member)| {
			(
				name.as_str(),
				member.script_password.as_str(),
				BattleStatus::from_bits(member.bits),
				member.colour,
			)
		})
	}

	/// The AIs, with who owns each: name, owner, status, colour, kind.
	pub fn bots(&self) -> impl Iterator<Item = (&str, &str, BattleStatus, u32, &str)> {
		self.bots.iter().map(|bot| {
			(
				bot.name.as_str(),
				bot.owner.as_str(),
				BattleStatus::from_bits(bot.bits),
				bot.colour,
				bot.ai.as_str(),
			)
		})
	}

	pub fn tags(&self) -> &BTreeMap<String, String> {
		&self.tags
	}

	/// What to tell the network about this room.
	pub fn announce(&self, id: u32, port: u16) -> crate::discover::Announce {
		let (members, spectators) = self.counts();
		crate::discover::Announce {
			id,
			port,
			title: self.config.title.clone(),
			host: self.config.founder.clone(),
			engine_version: self.config.engine_version.clone(),
			game: self.config.game.clone(),
			map: self.config.map.clone(),
			players: members.saturating_sub(spectators),
			max_players: self.config.max_players,
			passworded: matches!(self.config.policy, Policy::Password(_)),
		}
	}

	pub fn rects(&self) -> &BTreeMap<u8, (u16, u16, u16, u16)> {
		&self.rects
	}

	/// How many are in the battle, and how many watch. The client counts the
	/// founder as a member from `BATTLEOPENED` on, so an absent founder is a
	/// spectator here too, or the room would show a player who is not there.
	pub fn counts(&self) -> (u32, u32) {
		let founder_absent = !self.is_member(&self.config.founder);
		let spectators = self
			.members
			.iter()
			.filter(|(_, member)| !BattleStatus::from_bits(member.bits).player)
			.count() as u32
			+ u32::from(founder_absent);
		(self.members.len() as u32, spectators)
	}

	// --- connections ------------------------------------------------------

	/// A socket was accepted. `reached_at` is our address as that peer sees it.
	pub fn connect(&mut self, peer: Peer, _reached_at: IpAddr) -> Vec<Out> {
		self.greeted.insert(peer);
		vec![Out::To(peer, wire::tas_server(ENGINE_PORT))]
	}

	/// A socket closed, for whatever reason.
	pub fn disconnect(&mut self, peer: Peer) -> Vec<Out> {
		self.greeted.remove(&peer);
		self.pending.retain(|p| p.peer != peer);
		let Some(user) = self.users.remove(&peer) else {
			return Vec::new();
		};
		let mut out = self.leave(&user.name);
		out.push(Out::All(format!("REMOVEUSER {}", user.name)));
		out
	}

	/// One line from a peer.
	pub fn apply(&mut self, peer: Peer, line: &str, reached_at: IpAddr) -> Vec<Out> {
		match ClientLine::parse(line) {
			ClientLine::Login { name, agent } => self.login(peer, name, agent, reached_at),
			ClientLine::Stls => vec![
				Out::To(peer, "SERVERMSG no TLS on the LAN".into()),
				Out::Close(peer),
			],
			ClientLine::Exit => vec![Out::Close(peer)],
			ClientLine::Ping => vec![Out::To(peer, "PONG".into())],
			ClientLine::Other(_) => Vec::new(),
			other => {
				let Some(name) = self.users.get(&peer).map(|u| u.name.clone()) else {
					return Vec::new();
				};
				match other {
					ClientLine::JoinBattle {
						id,
						password,
						script_password,
					} => self.join(peer, &name, id, password.as_deref(), script_password),
					ClientLine::LeaveBattle => self.leave(&name),
					ClientLine::MyStatus(bits) => self.my_status(&name, bits),
					ClientLine::MyBattleStatus { bits, colour } => {
						self.my_battle_status(&name, bits, colour)
					}
					ClientLine::SayBattle(text) => self.say(&name, &text),
					ClientLine::SayBattleEx(text) => {
						if self.is_member(&name) {
							vec![Out::All(wire::said_battle_ex(&name, &text))]
						} else {
							Vec::new()
						}
					}
					ClientLine::AddBot {
						name: bot,
						bits,
						colour,
						ai,
					} => self.add_bot(&name, bot, bits, colour, ai),
					ClientLine::UpdateBot {
						name: bot,
						bits,
						colour,
					} => self.update_bot(&name, &bot, bits, colour),
					ClientLine::RemoveBot(bot) => self.remove_bot(&name, &bot),
					ClientLine::Kick(who) => self.kick(&name, &who),
					_ => Vec::new(),
				}
			}
		}
	}

	// --- login -------------------------------------------------------------

	fn login(&mut self, peer: Peer, name: String, agent: String, reached_at: IpAddr) -> Vec<Out> {
		let deny = |reason: &str| vec![Out::To(peer, format!("DENIED {reason}")), Out::Close(peer)];
		if name.is_empty() || name.contains(char::is_whitespace) {
			return deny("a name with no spaces is needed");
		}
		if self.kicked.contains(&name) {
			return deny("kicked from this room");
		}
		if self.users.values().any(|user| user.name == name) {
			let mut n = 2;
			while self.users.values().any(|u| u.name == format!("{name}_{n}")) {
				n += 1;
			}
			return deny(&format!("{name} is already here; try {name}_{n}"));
		}
		if self.users.contains_key(&peer) {
			return deny("already logged in");
		}
		let user = User {
			name: name.clone(),
			agent,
			status: 0,
		};
		let mut out = vec![Out::To(peer, format!("ACCEPTED {name}"))];
		// The newcomer first, so that `users[me]` exists before anything
		// refers to it; then everyone already here.
		out.push(Out::To(peer, wire::add_user(&name, peer, &user.agent)));
		for (id, other) in &self.users {
			out.push(Out::To(
				peer,
				wire::add_user(&other.name, *id, &other.agent),
			));
			if other.status != 0 {
				out.push(Out::To(
					peer,
					wire::client_status(&other.name, other.status),
				));
			}
		}
		if !self.closed {
			out.push(Out::To(peer, self.opened_line(reached_at)));
			let (_, spectators) = self.counts();
			out.push(Out::To(
				peer,
				wire::update_battle_info(BATTLE_ID, spectators, false, &self.config.map),
			));
			for (member, _) in &self.members {
				if *member != self.config.founder {
					out.push(Out::To(peer, wire::joined_battle(BATTLE_ID, member, None)));
				}
			}
		}
		out.push(Out::To(peer, "LOGININFOEND".into()));
		let announce = wire::add_user(&name, peer, &user.agent);
		self.users.insert(peer, user);
		for id in self.users.keys().filter(|id| **id != peer) {
			out.push(Out::To(*id, announce.clone()));
		}
		out
	}

	fn opened_line(&self, reached_at: IpAddr) -> String {
		wire::battle_opened(
			BATTLE_ID,
			&self.config.founder,
			&reached_at.to_string(),
			ENGINE_PORT,
			self.config.max_players,
			matches!(self.config.policy, Policy::Password(_)),
			&self.config.engine_version,
			&self.config.map,
			&self.config.title,
			&self.config.game,
		)
	}

	// --- the battle --------------------------------------------------------

	fn is_member(&self, name: &str) -> bool {
		self.members.iter().any(|(held, _)| held == name)
	}

	fn member(&mut self, name: &str) -> Option<&mut Member> {
		self.members
			.iter_mut()
			.find(|(held, _)| held == name)
			.map(|(_, member)| member)
	}

	fn founder_peer(&self) -> Option<Peer> {
		self.users
			.iter()
			.find(|(_, user)| user.name == self.config.founder)
			.map(|(peer, _)| *peer)
	}

	fn join(
		&mut self,
		peer: Peer,
		name: &str,
		id: u32,
		password: Option<&str>,
		script_password: String,
	) -> Vec<Out> {
		let fail = |reason: &str| vec![Out::To(peer, format!("JOINBATTLEFAILED {reason}"))];
		if self.closed || id != BATTLE_ID {
			return fail("no such battle");
		}
		if self.is_member(name) {
			return fail("already in the battle");
		}
		if self.refused.contains(name) {
			return fail("the host said no");
		}
		let founder = name == self.config.founder;
		if !founder {
			match &self.config.policy {
				Policy::Open => {}
				Policy::Password(wanted) => {
					if password != Some(wanted.as_str()) {
						return fail("wrong password");
					}
				}
				Policy::Approve => {
					if self.pending.iter().any(|p| p.name == name) {
						return fail("still waiting for the host");
					}
					self.pending.push(Pending {
						peer,
						name: name.to_owned(),
						script_password,
					});
					let mut out = vec![Out::To(
						peer,
						format!(
							"SERVERMSG waiting for {} to let you in",
							self.config.founder
						),
					)];
					if let Some(host) = self.founder_peer() {
						out.push(Out::To(
							host,
							wire::said_battle_ex(
								&self.config.founder,
								&format!("* {name} asks to join: !accept {name} or !deny {name}"),
							),
						));
					}
					return out;
				}
			}
			if self.members.len() as u32
				>= self.config.max_players + u32::from(!self.is_member(&self.config.founder))
			{
				return fail("the room is full");
			}
		}
		self.admit(peer, name, script_password)
	}

	/// The joiner is in: the replay, in the order the client reads it, then
	/// the news to everyone else.
	fn admit(&mut self, peer: Peer, name: &str, script_password: String) -> Vec<Out> {
		let mut out = vec![Out::To(peer, format!("JOINBATTLE {BATTLE_ID} 0"))];
		if !self.tags.is_empty() {
			out.push(Out::To(peer, wire::set_script_tags(self.tags.iter())));
		}
		for (member, held) in &self.members {
			if *member != self.config.founder {
				out.push(Out::To(peer, wire::joined_battle(BATTLE_ID, member, None)));
			}
			out.push(Out::To(
				peer,
				wire::client_battle_status(member, held.bits, held.colour),
			));
		}
		for bot in &self.bots {
			out.push(Out::To(
				peer,
				wire::add_bot(bot.id, &bot.name, &bot.owner, bot.bits, bot.colour, &bot.ai),
			));
		}
		for (ally, (l, t, r, b)) in &self.rects {
			out.push(Out::To(peer, wire::add_start_rect(*ally, *l, *t, *r, *b)));
		}
		out.push(Out::To(peer, "REQUESTBATTLESTATUS".into()));

		self.members.push((
			name.to_owned(),
			Member {
				peer,
				script_password: script_password.clone(),
				bits: READY,
				colour: 0,
			},
		));
		// Everyone, the joiner included, hears it; the founder alone hears
		// the script password, which is what its engine admits the joiner by.
		let founder = self.founder_peer();
		if name != self.config.founder {
			for id in self.users.keys() {
				let password = (Some(*id) == founder).then_some(script_password.as_str());
				out.push(Out::To(*id, wire::joined_battle(BATTLE_ID, name, password)));
			}
		}
		out.push(self.info_line());
		out
	}

	fn info_line(&self) -> Out {
		let (_, spectators) = self.counts();
		Out::All(wire::update_battle_info(
			BATTLE_ID,
			spectators,
			false,
			&self.config.map,
		))
	}

	fn leave(&mut self, name: &str) -> Vec<Out> {
		let Some(at) = self.members.iter().position(|(held, _)| held == name) else {
			return Vec::new();
		};
		self.members.remove(at);
		let mut out = Vec::new();
		// An AI goes with the player whose machine would have run it.
		let (gone, kept): (Vec<Bot>, Vec<Bot>) =
			self.bots.drain(..).partition(|bot| bot.owner == name);
		self.bots = kept;
		for bot in gone {
			out.push(Out::All(wire::remove_bot(bot.id, &bot.name)));
		}
		if name == self.config.founder {
			// The founder's engine is the game; without it there is no room.
			self.closed = true;
			out.push(Out::All(format!("BATTLECLOSED {BATTLE_ID}")));
			return out;
		}
		out.push(Out::All(wire::left_battle(BATTLE_ID, name)));
		out.push(self.info_line());
		out
	}

	fn my_status(&mut self, name: &str, bits: u32) -> Vec<Out> {
		// Only the two bits a client owns; rank and the rest are ours to say.
		let bits = bits & 0b11;
		if let Some(user) = self.users.values_mut().find(|user| user.name == name) {
			user.status = bits;
		}
		vec![Out::All(wire::client_status(name, bits))]
	}

	fn my_battle_status(&mut self, name: &str, bits: u32, colour: u32) -> Vec<Out> {
		let Some(member) = self.member(name) else {
			return Vec::new();
		};
		// `force` reads these back and writes them out again, so setting it
		// here is also what keeps a moved or bonused seat ready.
		let bits = bits | READY;
		member.bits = bits;
		member.colour = colour;
		vec![
			Out::All(wire::client_battle_status(name, bits, colour)),
			self.info_line(),
		]
	}

	// --- chat and the founder's dialect ----------------------------------

	fn say(&mut self, name: &str, text: &str) -> Vec<Out> {
		if !self.is_member(name) {
			return Vec::new();
		}
		let echo = Out::All(wire::said_battle(name, text));
		if name != self.config.founder || !text.starts_with('!') {
			return vec![echo];
		}
		let mut out = vec![echo];
		out.extend(self.command(text));
		out
	}

	/// What the founder may say and be obeyed: the corner of SPADS's language
	/// the online room speaks. Anything else is answered, not ignored, so a
	/// person learns the room did not hear a command rather than waiting.
	fn command(&mut self, text: &str) -> Vec<Out> {
		let mut words = text.split_whitespace();
		let verb = words.next().unwrap_or("");
		let rest: Vec<&str> = words.collect();
		let founder = self.config.founder.clone();
		let tell = |what: String| Out::All(wire::said_battle_ex(&founder, &what));
		match (verb, rest.as_slice()) {
			("!bSet" | "!bset", [key, value @ ..]) => {
				let key = format!("game/modoptions/{}", key.to_ascii_lowercase());
				let value = value.join(" ");
				let mut out = Vec::new();
				if key == "game/modoptions/mapmetadata_startbox_override" {
					out.extend(self.rects_from(&value));
				}
				if value.is_empty() {
					self.tags.remove(&key);
					out.push(Out::All(format!("REMOVESCRIPTTAGS {key}")));
				} else {
					self.tags.insert(key.clone(), value.clone());
					out.push(Out::All(wire::set_script_tags([(&key, &value)])));
				}
				out
			}
			("!map", [name @ ..]) if !name.is_empty() => {
				self.config.map = name.join(" ");
				vec![self.info_line()]
			}
			("!set", ["startPosType" | "startpostype", value]) => {
				let key = "game/startpostype".to_owned();
				let value = (*value).to_owned();
				self.tags.insert(key.clone(), value.clone());
				vec![Out::All(wire::set_script_tags([(&key, &value)]))]
			}
			("!force", [who, "team", n]) => {
				let ally = n.parse::<u8>().unwrap_or(1).saturating_sub(1);
				self.force(who, |status| status.ally_team = ally)
					.unwrap_or_else(|| vec![tell(format!("* nobody called {who}"))])
			}
			("!force", [who, "bonus", n]) => {
				let bonus = n.parse::<u8>().unwrap_or(0).min(100);
				self.force(who, |status| status.handicap = bonus)
					.unwrap_or_else(|| vec![tell(format!("* nobody called {who}"))])
			}
			("!spec", [who]) => self
				.force(who, |status| status.player = false)
				.unwrap_or_else(|| vec![tell(format!("* nobody called {who}"))]),
			("!accept", [who]) => self.answer(who, true),
			("!deny", [who]) => self.answer(who, false),
			("!kick", [who]) => self.kick(&founder, who),
			("!start", []) => vec![tell(
				"* press Start; the game begins when your engine does".into(),
			)],
			_ => vec![tell(format!("* {verb} is not something a LAN room does"))],
		}
	}

	/// Rewrites one member's or one AI's status. `%name` is an AI, as SPADS
	/// spells it.
	fn force(&mut self, who: &str, change: impl Fn(&mut BattleStatus)) -> Option<Vec<Out>> {
		if let Some(bot_name) = who.strip_prefix('%') {
			let bot = self.bots.iter_mut().find(|bot| bot.name == bot_name)?;
			let mut status = BattleStatus::from_bits(bot.bits);
			change(&mut status);
			bot.bits = bits_of(status);
			return Some(vec![Out::All(wire::update_bot(
				bot.id, &bot.name, bot.bits, bot.colour,
			))]);
		}
		let member = self.member(who)?;
		let mut status = BattleStatus::from_bits(member.bits);
		change(&mut status);
		member.bits = bits_of(status);
		let (bits, colour) = (member.bits, member.colour);
		Some(vec![
			Out::All(wire::client_battle_status(who, bits, colour)),
			self.info_line(),
		])
	}

	fn answer(&mut self, who: &str, allow: bool) -> Vec<Out> {
		let Some(at) = self.pending.iter().position(|p| p.name == who) else {
			return vec![Out::All(wire::said_battle_ex(
				&self.config.founder,
				&format!("* {who} is not waiting"),
			))];
		};
		let pending = self.pending.remove(at);
		if allow {
			return self.admit(pending.peer, &pending.name, pending.script_password);
		}
		self.refused.insert(pending.name);
		vec![Out::To(
			pending.peer,
			"JOINBATTLEFAILED the host said no".into(),
		)]
	}

	fn kick(&mut self, by: &str, who: &str) -> Vec<Out> {
		if by != self.config.founder || who == self.config.founder {
			return Vec::new();
		}
		let Some(peer) = self.member(who).map(|member| member.peer) else {
			return Vec::new();
		};
		self.kicked.insert(who.to_owned());
		let mut out = vec![Out::To(peer, "FORCEQUITBATTLE".into())];
		out.extend(self.leave(who));
		out
	}

	// --- AIs -----------------------------------------------------------------

	fn add_bot(
		&mut self,
		owner: &str,
		name: String,
		bits: u32,
		colour: u32,
		ai: String,
	) -> Vec<Out> {
		if !self.is_member(owner) || name.is_empty() || self.bots.iter().any(|bot| bot.name == name)
		{
			return Vec::new();
		}
		let bot = Bot {
			id: self.next_bot,
			name,
			owner: owner.to_owned(),
			bits,
			colour,
			ai,
		};
		self.next_bot += 1;
		let line = wire::add_bot(bot.id, &bot.name, &bot.owner, bot.bits, bot.colour, &bot.ai);
		self.bots.push(bot);
		vec![Out::All(line)]
	}

	fn update_bot(&mut self, by: &str, name: &str, bits: u32, colour: u32) -> Vec<Out> {
		let founder = self.config.founder.clone();
		let Some(bot) = self.bots.iter_mut().find(|bot| bot.name == name) else {
			return Vec::new();
		};
		if bot.owner != by && by != founder {
			return Vec::new();
		}
		bot.bits = bits;
		bot.colour = colour;
		vec![Out::All(wire::update_bot(bot.id, &bot.name, bits, colour))]
	}

	fn remove_bot(&mut self, by: &str, name: &str) -> Vec<Out> {
		let Some(at) = self.bots.iter().position(|bot| bot.name == name) else {
			return Vec::new();
		};
		if self.bots[at].owner != by && by != self.config.founder {
			return Vec::new();
		}
		let bot = self.bots.remove(at);
		vec![Out::All(wire::remove_bot(bot.id, &bot.name))]
	}

	// --- start boxes -------------------------------------------------------

	/// The engine's own rectangles, from the arrangement the room drew: one
	/// per box, as a bounding rectangle, so a client without the decoder still
	/// sees where the sides start.
	fn rects_from(&mut self, encoded: &str) -> Vec<Out> {
		let mut out: Vec<Out> = self
			.rects
			.keys()
			.map(|ally| Out::All(wire::remove_start_rect(*ally)))
			.collect();
		self.rects.clear();
		let Ok(arrangement) = startbox::decode_override(encoded) else {
			return out;
		};
		for (ally, held) in arrangement.startboxes.iter().enumerate() {
			let (left, top, right, bottom) = held.bounds();
			let rect = (
				left.round() as u16,
				top.round() as u16,
				right.round() as u16,
				bottom.round() as u16,
			);
			self.rects.insert(ally as u8, rect);
			out.push(Out::All(wire::add_start_rect(
				ally as u8, rect.0, rect.1, rect.2, rect.3,
			)));
		}
		out
	}
}

/// The ready bit, which a room on the LAN keeps set for everyone.
///
/// There is nothing here to be ready *for*: the founder starts the game when
/// they choose, and no host, vote or countdown is reading the flag. Left to a
/// client that never sets it, the mark sits red beside every name all evening
/// and means nothing. So the room says what is true of it instead, and it
/// says it once, for every client that joins rather than only for ours.
const READY: u32 = 1 << 1;

/// The bits of a decoded status, the mirror of `BattleStatus::from_bits`.
pub(crate) fn bits_of(status: BattleStatus) -> u32 {
	let team = u32::from(status.team);
	let ally = u32::from(status.ally_team);
	(u32::from(status.ready) << 1)
		| ((team & 0xF) << 2)
		| ((ally & 0xF) << 6)
		| (u32::from(status.player) << 10)
		| ((u32::from(status.handicap) & 0x7F) << 11)
		| (((team >> 4) & 0xF) << 18)
		| ((status.sync as u32) << 22)
		| ((u32::from(status.side) & 0xF) << 24)
		| (((ally >> 4) & 0xF) << 28)
}

#[cfg(test)]
mod tests {
	use super::*;
	use spring_protocol::MyBattleStatus;
	use spring_protocol::Sync;

	const HOST: Peer = 1;
	const GUEST: Peer = 2;
	const IP: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 5));

	fn config(policy: Policy) -> Config {
		Config {
			founder: "ann".into(),
			title: "Ann's game".into(),
			engine_version: "2026.07.04".into(),
			game: "Beyond All Reason test-31357-b06bb1a".into(),
			map: "Supreme Isthmus v2.1".into(),
			max_players: 8,
			policy,
		}
	}

	/// Lines that reached `peer`, `All` included.
	fn to(out: &[Out], peer: Peer) -> Vec<String> {
		out.iter()
			.filter_map(|o| match o {
				Out::To(p, line) if *p == peer => Some(line.clone()),
				Out::All(line) => Some(line.clone()),
				_ => None,
			})
			.collect()
	}

	fn closed(out: &[Out], peer: Peer) -> bool {
		out.iter().any(|o| *o == Out::Close(peer))
	}

	fn login(room: &mut Room, peer: Peer, name: &str) -> Vec<Out> {
		room.connect(peer, IP);
		room.apply(
			peer,
			&format!("LOGIN {name} * 0 * modlobby:0.1\tx y\tb sp"),
			IP,
		)
	}

	fn seated(room: &mut Room, peer: Peer, name: &str, sp: &str) -> Vec<Out> {
		login(room, peer, name);
		room.apply(peer, &format!("JOINBATTLE 1 empty {sp}"), IP)
	}

	#[test]
	fn a_login_is_the_flood_the_client_expects_ending_where_it_ends() {
		let mut room = Room::new(config(Policy::Open));
		let out = login(&mut room, HOST, "ann");
		let lines = to(&out, HOST);
		assert_eq!(lines[0], "ACCEPTED ann");
		assert_eq!(lines[1], "ADDUSER ann ?? 1 modlobby:0.1");
		assert!(lines[2].starts_with("BATTLEOPENED 1 0 0 ann 192.168.1.5 8452 8 0 0 0 Recoil\t2026.07.04\tSupreme Isthmus v2.1\tAnn's game\tBeyond"));
		// The founder is not in yet, so the client's implied member watches.
		assert_eq!(lines[3], "UPDATEBATTLEINFO 1 1 0 0 Supreme Isthmus v2.1");
		assert_eq!(lines.last().unwrap(), "LOGININFOEND");

		let out = login(&mut room, GUEST, "bob");
		let lines = to(&out, GUEST);
		assert!(lines.contains(&"ADDUSER ann ?? 1 modlobby:0.1".to_owned()));
		assert!(to(&out, HOST).contains(&"ADDUSER bob ?? 2 modlobby:0.1".to_owned()));
	}

	/// Nothing on the LAN waits for a ready flag, so the room keeps it set:
	/// the mark beside a name says "here", not "still deciding".
	#[test]
	fn every_seat_is_ready_whatever_the_client_sent() {
		let mut room = Room::new(config(Policy::Open));
		seated(&mut room, HOST, "ann", "1111");
		seated(&mut room, GUEST, "bob", "4242");
		let bits = MyBattleStatus::player(Sync::Synced, 1, 1).bits();
		assert!(!BattleStatus::from_bits(bits).ready, "the client says no");
		let out = room.apply(GUEST, &format!("MYBATTLESTATUS {bits} 255"), IP);
		let said = to(&out, GUEST)
			.into_iter()
			.find(|line| line.starts_with("CLIENTBATTLESTATUS"))
			.expect("the room says so");
		let sent: u32 = said.split(' ').nth(2).unwrap().parse().unwrap();
		let status = BattleStatus::from_bits(sent);
		assert!(status.ready, "the room says yes");
		// And nothing else about the seat was touched on the way through.
		assert_eq!((status.team, status.ally_team, status.player), (1, 1, true));
	}

	#[test]
	fn a_name_already_here_is_refused_with_a_free_one() {
		let mut room = Room::new(config(Policy::Open));
		login(&mut room, HOST, "ann");
		let out = login(&mut room, GUEST, "ann");
		assert_eq!(to(&out, GUEST), ["DENIED ann is already here; try ann_2"]);
		assert!(closed(&out, GUEST));
	}

	#[test]
	fn a_password_gate_and_an_open_door() {
		let mut room = Room::new(config(Policy::Password("hunter2".into())));
		login(&mut room, HOST, "ann");
		let out = login(&mut room, GUEST, "bob");
		assert!(
			to(&out, GUEST)
				.iter()
				.any(|l| l.contains(" 8 1 0 0 Recoil\t")),
			"passworded"
		);
		let out = room.apply(GUEST, "JOINBATTLE 1 empty 4242", IP);
		assert_eq!(to(&out, GUEST), ["JOINBATTLEFAILED wrong password"]);
		let out = room.apply(GUEST, "JOINBATTLE 1 hunter2 4242", IP);
		assert_eq!(to(&out, GUEST)[0], "JOINBATTLE 1 0");
	}

	#[test]
	fn the_join_replay_is_in_the_order_the_client_reads_it() {
		let mut room = Room::new(config(Policy::Open));
		seated(&mut room, HOST, "ann", "1111");
		// 4195328 is a synced player on team 0; the room adds ready (2) to it.
		room.apply(HOST, "MYBATTLESTATUS 4195328 255", IP);
		room.apply(HOST, "SAYBATTLE !bSet startmetal 1000", IP);
		room.apply(HOST, "ADDBOT Bot1 4194304 16 BARb", IP);
		let out = seated(&mut room, GUEST, "bob", "4242");
		let lines = to(&out, GUEST);
		assert_eq!(
			lines,
			[
				"JOINBATTLE 1 0",
				"SETSCRIPTTAGS game/modoptions/startmetal=1000",
				"CLIENTBATTLESTATUS ann 4195330 255",
				"ADDBOT 1 Bot1 ann 4194304 16 BARb",
				"REQUESTBATTLESTATUS",
				"JOINEDBATTLE 1 bob",
				"UPDATEBATTLEINFO 1 1 0 0 Supreme Isthmus v2.1",
			]
		);
		// The founder alone learns the script password.
		assert!(to(&out, HOST).contains(&"JOINEDBATTLE 1 bob 4242".to_owned()));
	}

	#[test]
	fn approval_waits_silently_on_the_wire_and_the_founder_decides() {
		let mut room = Room::new(config(Policy::Approve));
		seated(&mut room, HOST, "ann", "1111");
		login(&mut room, GUEST, "bob");
		let out = room.apply(GUEST, "JOINBATTLE 1 empty 4242", IP);
		assert_eq!(to(&out, GUEST), ["SERVERMSG waiting for ann to let you in"]);
		assert_eq!(
			to(&out, HOST),
			["SAIDBATTLEEX ann * bob asks to join: !accept bob or !deny bob"]
		);
		assert_eq!(room.pending(), ["bob"]);

		let out = room.apply(HOST, "SAYBATTLE !deny bob", IP);
		assert!(to(&out, GUEST).contains(&"JOINBATTLEFAILED the host said no".to_owned()));
		let out = room.apply(GUEST, "JOINBATTLE 1 empty 4242", IP);
		assert_eq!(to(&out, GUEST), ["JOINBATTLEFAILED the host said no"]);

		let mut room = Room::new(config(Policy::Approve));
		seated(&mut room, HOST, "ann", "1111");
		login(&mut room, GUEST, "bob");
		room.apply(GUEST, "JOINBATTLE 1 empty 4242", IP);
		let out = room.apply(HOST, "SAYBATTLE !accept bob", IP);
		assert_eq!(to(&out, GUEST)[0], "SAIDBATTLE ann !accept bob");
		assert_eq!(to(&out, GUEST)[1], "JOINBATTLE 1 0");
		assert!(room.pending().is_empty());
	}

	#[test]
	fn the_founders_in_game_bit_is_the_start_and_only_its_two_bits_travel() {
		let mut room = Room::new(config(Policy::Open));
		seated(&mut room, HOST, "ann", "1111");
		seated(&mut room, GUEST, "bob", "4242");
		let out = room.apply(HOST, "MYSTATUS 1", IP);
		assert_eq!(to(&out, GUEST), ["CLIENTSTATUS ann 1"]);
		let out = room.apply(HOST, "MYSTATUS 65", IP);
		assert_eq!(
			to(&out, GUEST),
			["CLIENTSTATUS ann 1"],
			"no promoting yourself to a bot"
		);
	}

	#[test]
	fn a_status_is_echoed_and_the_watcher_count_follows_it() {
		let mut room = Room::new(config(Policy::Open));
		seated(&mut room, HOST, "ann", "1111");
		seated(&mut room, GUEST, "bob", "4242");
		let bits = MyBattleStatus::player(Sync::Synced, 0, 0).bits();
		// What comes back is what was sent plus the ready bit the room keeps
		// set; `every_seat_is_ready_whatever_the_client_sent` is about that.
		let echoed = bits | READY;
		let out = room.apply(GUEST, &format!("MYBATTLESTATUS {bits} 255"), IP);
		assert_eq!(
			to(&out, HOST),
			[
				format!("CLIENTBATTLESTATUS bob {echoed} 255"),
				"UPDATEBATTLEINFO 1 1 0 0 Supreme Isthmus v2.1".to_owned()
			]
		);
		assert_eq!(room.counts(), (2, 1));
	}

	#[test]
	fn the_founder_forces_by_the_online_rooms_words() {
		let mut room = Room::new(config(Policy::Open));
		seated(&mut room, HOST, "ann", "1111");
		seated(&mut room, GUEST, "bob", "4242");
		let bits = MyBattleStatus::player(Sync::Synced, 1, 0).bits();
		room.apply(GUEST, &format!("MYBATTLESTATUS {bits} 255"), IP);
		let out = room.apply(HOST, "SAYBATTLE !force bob team 2", IP);
		let moved = to(&out, GUEST)
			.into_iter()
			.find(|l| l.starts_with("CLIENTBATTLESTATUS bob"))
			.unwrap();
		let bits: u32 = moved.split(' ').nth(2).unwrap().parse().unwrap();
		let status = BattleStatus::from_bits(bits);
		assert_eq!(status.ally_team, 1, "SPADS counts from one");
		assert_eq!(status.team, 1, "the rest is kept");
		assert!(status.player);

		room.apply(HOST, "SAYBATTLE !force bob bonus 30", IP);
		let (_, _, status, _) = room.members().find(|(n, ..)| *n == "bob").unwrap();
		assert_eq!(status.handicap, 30);

		room.apply(GUEST, "ADDBOT Bot1 4194304 16 BARb", IP);
		let out = room.apply(HOST, "SAYBATTLE !force %Bot1 team 3", IP);
		assert!(
			to(&out, GUEST)
				.iter()
				.any(|l| l.starts_with("UPDATEBOT 1 Bot1 "))
		);

		let out = room.apply(HOST, "SAYBATTLE !dance", IP);
		assert!(
			to(&out, GUEST)
				.contains(&"SAIDBATTLEEX ann * !dance is not something a LAN room does".to_owned())
		);
		// A guest's `!` line is only chat.
		let out = room.apply(GUEST, "SAYBATTLE !kick ann", IP);
		assert_eq!(to(&out, HOST), ["SAIDBATTLE bob !kick ann"]);
	}

	#[test]
	fn a_map_change_and_a_modoption_reach_everyone() {
		let mut room = Room::new(config(Policy::Open));
		seated(&mut room, HOST, "ann", "1111");
		seated(&mut room, GUEST, "bob", "4242");
		let out = room.apply(HOST, "SAYBATTLE !map Comet Catcher Remake 1.8", IP);
		// Both sit as spectators still, so both are counted as watching.
		assert!(
			to(&out, GUEST)
				.contains(&"UPDATEBATTLEINFO 1 2 0 0 Comet Catcher Remake 1.8".to_owned())
		);
		assert_eq!(room.config().map, "Comet Catcher Remake 1.8");
		let out = room.apply(HOST, "SAYBATTLE !set startPosType 2", IP);
		assert!(to(&out, GUEST).contains(&"SETSCRIPTTAGS game/startpostype=2".to_owned()));
		let out = room.apply(HOST, "SAYBATTLE !bSet StartMetal 1500", IP);
		assert!(
			to(&out, GUEST).contains(&"SETSCRIPTTAGS game/modoptions/startmetal=1500".to_owned())
		);
		let out = room.apply(HOST, "SAYBATTLE !bSet startmetal", IP);
		assert!(
			to(&out, GUEST).contains(&"REMOVESCRIPTTAGS game/modoptions/startmetal".to_owned())
		);
	}

	#[test]
	fn leaving_and_dropping_take_your_ais_with_you_and_the_founder_takes_the_room() {
		let mut room = Room::new(config(Policy::Open));
		seated(&mut room, HOST, "ann", "1111");
		seated(&mut room, GUEST, "bob", "4242");
		room.apply(GUEST, "ADDBOT Bot1 4194304 16 BARb", IP);
		let out = room.disconnect(GUEST);
		assert_eq!(
			to(&out, HOST),
			[
				"REMOVEBOT 1 Bot1",
				"LEFTBATTLE 1 bob",
				"UPDATEBATTLEINFO 1 1 0 0 Supreme Isthmus v2.1",
				"REMOVEUSER bob"
			]
		);
		let out = room.apply(HOST, "LEAVEBATTLE", IP);
		assert_eq!(to(&out, HOST), ["BATTLECLOSED 1"]);
		assert!(room.closed());
		let out = login(&mut room, 3, "cat");
		assert!(!to(&out, 3).iter().any(|l| l.starts_with("BATTLEOPENED")));
	}

	#[test]
	fn a_kick_is_final_for_the_room() {
		let mut room = Room::new(config(Policy::Open));
		seated(&mut room, HOST, "ann", "1111");
		seated(&mut room, GUEST, "bob", "4242");
		let out = room.apply(HOST, "SAYBATTLE !kick bob", IP);
		assert!(to(&out, GUEST).contains(&"FORCEQUITBATTLE".to_owned()));
		assert!(to(&out, HOST).contains(&"LEFTBATTLE 1 bob".to_owned()));
		room.disconnect(GUEST);
		let out = login(&mut room, 3, "bob");
		assert_eq!(to(&out, 3), ["DENIED kicked from this room"]);
	}

	#[test]
	fn a_full_room_and_a_stray_join_are_refused_and_the_handshake_lines_answered() {
		let mut room = Room::new(config(Policy::Open));
		room.config.max_players = 1;
		seated(&mut room, HOST, "ann", "1111");
		login(&mut room, GUEST, "bob");
		let out = room.apply(GUEST, "JOINBATTLE 2 empty 4242", IP);
		assert_eq!(to(&out, GUEST), ["JOINBATTLEFAILED no such battle"]);
		let out = room.apply(GUEST, "JOINBATTLE 1 empty 4242", IP);
		assert_eq!(to(&out, GUEST), ["JOINBATTLEFAILED the room is full"]);
		let out = room.apply(GUEST, "PING", IP);
		assert_eq!(to(&out, GUEST), ["PONG"]);
		let out = room.apply(3, "STLS", IP);
		assert!(closed(&out, 3));
		assert!(room.apply(GUEST, "FRIENDLIST", IP).is_empty());
	}

	#[test]
	fn status_bits_round_trip() {
		for bits in [0, 4195328, 0x7FFF_FFFF, 1 << 10 | 3 << 22 | 5 << 24] {
			let status = BattleStatus::from_bits(bits);
			assert_eq!(BattleStatus::from_bits(bits_of(status)), status);
		}
	}
}
