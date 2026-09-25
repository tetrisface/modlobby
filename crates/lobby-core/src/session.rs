use std::collections::{BTreeMap, VecDeque};
use std::net::Ipv4Addr;
use std::time::Duration;

use spring_protocol::{
	Area, Envelope, LoginRequest, MyBattleStatus, ServerEvent, Sync, battle, chat, friends, login,
	status, telemetry,
};

use crate::hosting::{self, Rtts, SpareRoom};
use crate::spads::{self, Announcement, VoteState};
use crate::state::{Bot, Channel, LobbyState, MyBattle, Phase, SeatOnItsWay, StartRect};
use spring_protocol::policy::PasteBurst;

/// What the application must do in response to an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
	Send(Envelope),
	LoggedIn {
		username: String,
	},
	/// The post-login flood has finished; state is complete.
	Ready,
	LoginDenied {
		reason: String,
	},
	/// The account must confirm the user agreement (email code) before it can log in.
	AgreementRequired {
		text: Vec<String>,
	},
	/// The account was created. It still has to log in and confirm the code
	/// the server has just emailed.
	Registered,
	RegistrationDenied {
		reason: String,
	},
	Redirect {
		host: String,
		port: Option<u16>,
	},
	/// Server-initiated disconnect; `flood` means re-login is blocked for ~10 s.
	Disconnected {
		reason: String,
		flood: bool,
	},
	Notice(String),
	/// A multi-line paste (or one with settings skipped) was handed on:
	/// `lines` queued to send, `skipped` dropped as already in place. The
	/// runtime turns this into progress the front end can show.
	PasteQueued {
		lines: usize,
		skipped: usize,
	},
	/// The host accepted us into the room, as a spectator.
	Joined {
		id: u32,
	},
	JoinFailed {
		reason: String,
	},
	/// We are out of the room: left, kicked, or it closed.
	LeftBattle {
		id: u32,
	},
	BattleChat {
		from: String,
		text: String,
		/// `SAIDBATTLEEX`: host announcements and `/me` lines.
		announcement: bool,
	},
	/// A message in a channel we are in.
	ChannelChat {
		room: String,
		from: String,
		text: String,
		/// `SAIDEX`: an emote, rendered as an action rather than speech.
		emote: bool,
	},
	/// We are now in a channel, with its roster as far as it has arrived.
	ChannelJoined {
		room: String,
	},
	ChannelJoinFailed {
		room: String,
		reason: String,
	},
	ChannelLeft {
		room: String,
	},
	/// The roster or topic of a channel changed.
	ChannelChanged {
		room: String,
	},
	/// The server's channel directory finished arriving.
	ChannelsListed,
	/// The friend list or the pending requests changed.
	FriendsChanged,
	/// Who is bossing our room changed.
	BossChanged,
	/// Something else the room view shows about our own part in it changed:
	/// a pre-ready armed, taken back or spent.
	RoomChanged,
	/// The server broadcast something to everyone.
	ServerSaid {
		text: String,
	},
	/// A line of the message of the day: the same greeting on every connect,
	/// so kept for reading but nothing to be told about.
	Motd {
		text: String,
	},
	/// Someone is summoning us; Chobby alerts on this unconditionally, because
	/// it is a person asking for you rather than the room making noise.
	Rung {
		by: String,
	},
	/// A direct message; the Coordinator uses these for refusals.
	///
	/// `with` is the other person, which is who the conversation is filed
	/// under whichever way the message went; `from` is who wrote it.
	PrivateChat {
		with: String,
		from: String,
		text: String,
	},
	/// The room's modoptions changed; `keys` are unprefixed, e.g. `tweakdefs1`.
	ModOptionsChanged {
		keys: Vec<String>,
	},
	/// The room's vote started, moved or ended.
	VoteChanged,
	/// A cluster manager answered `!privatehost` with a room password.
	PrivateHostOffered {
		manager: String,
		password: String,
	},
	/// The private room asked for has appeared on the list.
	PrivateHostReady {
		id: u32,
		password: String,
	},
	/// We are in the spare autohost we set out to take. `alone` is whether
	/// it was still empty on arrival, which is when it has been claimed.
	Hosting {
		founder: String,
		alone: bool,
	},
	/// The room's game is running; the engine connects with `spring://<me>:<script_password>@<ip>:<port>`.
	GameRunning {
		id: u32,
		ip: String,
		port: u16,
		script_password: String,
		/// Whether the game began just now, rather than having been under way
		/// before we got here.
		///
		/// The two are the same connection but not the same event. Walking
		/// into a room with a game in progress is an invitation to watch;
		/// a game starting around you is the thing you came for. Only the
		/// second is a reason to do anything on somebody's behalf.
		just_started: bool,
	},
	/// Our room's host came back out of its game.
	GameStopped,
	/// SPADS has put us in the running game on a player's ID (`!joinas`), and
	/// these are the others on it: empty until the host has said who they are.
	PlayingWith {
		names: Vec<String>,
	},
	/// How long our room's game had been going when we walked in, from SPADS's
	/// welcome message — the only place this protocol states it.
	GameInProgress {
		id: u32,
		elapsed_secs: u64,
	},
}

/// Whether this connection exists to log in or to create an account.
///
/// The server takes `REGISTER` from an unauthenticated connection and answers
/// it without opening a session, so the two differ only in what is sent when
/// the server says hello.
#[derive(Debug, Clone)]
enum Intent {
	Login,
	Register { email: String, password: String },
}

/// One logical connection: credentials, machine identity and the state they produce.
#[derive(Debug)]
pub struct Session {
	intent: Intent,
	login: LoginRequest,
	hardware: Vec<(String, String)>,
	machine_hash: String,
	/// Whether the server greeted as teiserver, which alone takes the
	/// `c.telemetry.*` properties.
	teiserver: bool,
	agreement: Vec<String>,
	/// Script password of a `JOINBATTLE` the host has not answered yet.
	pending_join: Option<String>,
	/// The seat we hold in our room; `None` is a spectator. The server's word
	/// on it, unless a request of ours it has not answered yet is still on its
	/// way, in which case it is that request (see `in_flight`).
	seat: Option<Seat>,
	/// The statuses we sent that the server has not answered yet, oldest
	/// first.
	///
	/// teiserver answers each one, in order, with the status it kept
	/// (`spring_in.ex` `MYBATTLESTATUS`), and a status about us that arrives
	/// meanwhile is recorded like any other. But our messages are whole
	/// statuses, and one still on its way will be applied after whatever just
	/// arrived; building the next one from that arrival would retract a
	/// request the server has not seen yet. On a join that is the seat
	/// itself: our spectator answer to `REQUESTBATTLESTATUS` echoes back after
	/// the auto-seat's `take_seat`, and the content check's message in
	/// between would otherwise say spectator.
	///
	/// An answer is matched by what it says (`Seat::answers`), against the
	/// newest request it fits: every request before that one was answered
	/// already or replaced before it left (`battle_status` coalesces), and
	/// the server now holds what it asked for. One that fits nothing answers
	/// the oldest, which the server kept its own way -- a seat refused in a
	/// full room, say.
	in_flight: VecDeque<Option<Seat>>,
	/// A Ready pressed while watching: the seat first, and a ready once the
	/// server has answered with it. Sent as two requests because the server
	/// clears a ready that arrives with the seat (`consul_server.ex`
	/// `request_user_change_status`). Dropped with a refused seat, by a
	/// Spectate press, and by an explicit ready press either way: the newest
	/// word from the player wins.
	ready_after_seat: bool,
	/// A `!privatehost` we asked for and the password it came back with.
	private_host: Option<String>,
	/// The spare autohost we are joining to make it ours; claimed on arrival.
	hosting: Option<u32>,
	/// Whether this machine has the room's engine, game and map. Claiming to be
	/// synced when we are not makes the host start a game we cannot join.
	synced: bool,
	/// The two bits `MYSTATUS` carries. Kept together because the command
	/// carries both at once and there is no way to send one alone.
	in_game: bool,
	away: bool,
	/// A friend listing part-way through arriving. Held aside so a listing that
	/// is cut off never half-replaces the one we have.
	collecting_friends: Option<std::collections::BTreeSet<String>>,
	collecting_requests: Option<std::collections::BTreeSet<String>>,
	collecting_ignored: Option<std::collections::BTreeSet<String>>,
	pub state: LobbyState,
}

/// What to do about a friendship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendAction {
	Request,
	Accept,
	Decline,
	Remove,
	Ignore,
	Unignore,
}

#[derive(Debug, thiserror::Error)]
#[error("no such friend action: {0}")]
pub struct UnknownFriendAction(String);

impl std::str::FromStr for FriendAction {
	type Err = UnknownFriendAction;

	fn from_str(text: &str) -> Result<Self, Self::Err> {
		match text {
			"request" => Ok(Self::Request),
			"accept" => Ok(Self::Accept),
			"decline" => Ok(Self::Decline),
			"remove" => Ok(Self::Remove),
			"ignore" => Ok(Self::Ignore),
			"unignore" => Ok(Self::Unignore),
			other => Err(UnknownFriendAction(other.to_owned())),
		}
	}
}

/// Why taking a player slot was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SeatError {
	#[error("not in a battle")]
	NotInARoom,
	/// Ready and faction belong to a player; a spectator has neither.
	#[error("you are spectating")]
	Spectating,
}

/// The seat we hold, and what we have said about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
	pub team: u8,
	pub ally_team: u8,
	pub ready: bool,
	pub side: u8,
	/// The host's to give (`!force <name> bonus`); kept so what we send back
	/// carries its value rather than ours.
	pub handicap: u8,
}

impl Seat {
	/// Our seat as the server has it, or `None` for a spectator.
	fn from_status(status: &spring_protocol::BattleStatus) -> Option<Self> {
		status.player.then_some(Self {
			team: status.team,
			ally_team: status.ally_team,
			ready: status.ready,
			side: status.side,
			handicap: status.handicap,
		})
	}

	/// Whether the server saying `said` answers our asking for `asked`. It
	/// keeps its own team number and bonus (`spring_in.ex` takes only ready,
	/// ally team, player, sync and side from us), so only those are compared.
	fn answers(asked: Option<Self>, said: Option<Self>) -> bool {
		match (asked, said) {
			(None, None) => true,
			(Some(asked), Some(said)) => {
				asked.ally_team == said.ally_team
					&& asked.ready == said.ready
					&& asked.side == said.side
			}
			_ => false,
		}
	}
}

impl Session {
	/// `hardware` are the `hardware:*` telemetry properties uploaded after login (see [`telemetry`]).
	pub fn new(login: LoginRequest, hardware: Vec<(String, String)>, machine_hash: String) -> Self {
		Self {
			intent: Intent::Login,
			login,
			hardware,
			machine_hash,
			teiserver: false,
			agreement: Vec::new(),
			pending_join: None,
			collecting_friends: None,
			collecting_requests: None,
			collecting_ignored: None,
			seat: None,
			in_flight: VecDeque::new(),
			ready_after_seat: false,
			private_host: None,
			hosting: None,
			synced: false,
			in_game: false,
			away: false,
			state: LobbyState {
				phase: Some(Phase::Connecting),
				..LobbyState::default()
			},
		}
	}

	/// Makes this connection a registration rather than a login.
	///
	/// The server takes `REGISTER` before any login, and answers it with
	/// `REGISTRATIONACCEPTED`/`REGISTRATIONDENIED` rather than a session — so
	/// the difference is entirely in what gets sent on `Welcome`, and nothing
	/// downstream needs to know.
	#[must_use]
	pub fn registering(mut self, email: impl Into<String>, password: impl Into<String>) -> Self {
		self.intent = Intent::Register {
			email: email.into(),
			password: password.into(),
		};
		self
	}

	/// Confirms the emailed agreement code for an account that has just been
	/// created; the server then lets it log in.
	pub fn confirm_agreement(&mut self, code: &str) -> Vec<Effect> {
		vec![Effect::Send(Envelope::queue(
			Area::Login,
			login::confirm_agreement(code),
		))]
	}

	/// Asks to join room `id` as a spectator. `script_password` is the secret the
	/// engine later presents to the host; the caller supplies it so this stays pure.
	/// Joins a room, leaving the one we are in first.
	///
	/// teiserver keeps one room per client, so a `JOINBATTLE` sent while still
	/// in another is simply ignored — which looks exactly like nothing
	/// happening.
	pub fn join_battle(
		&mut self,
		id: u32,
		password: Option<&str>,
		script_password: String,
	) -> Vec<Effect> {
		let mut effects = match self.state.my_battle.as_ref() {
			Some(my) if my.id == id => return vec![],
			Some(_) => self.leave_battle(),
			None => Vec::new(),
		};
		let line = battle::join_battle(id, password, &script_password);
		self.pending_join = Some(script_password);
		effects.push(Effect::Send(Envelope::queue(Area::Other, line)));
		effects
	}

	/// Joins a spare autohost to make it ours. Once the host lets us in,
	/// `!boss` and `!preset custom` follow (`battle_list_window.lua:1748`):
	/// the sole occupant's commands run without a vote, and being boss is
	/// what keeps the room ours when the next person walks in.
	pub fn host_public(&mut self, id: u32, script_password: String) -> Vec<Effect> {
		let effects = self.join_battle(id, None, script_password);
		self.hosting = Some(id);
		effects
	}

	/// What arriving in the room we set out to take calls for.
	fn claim_room(&mut self, id: u32) -> Vec<Effect> {
		if self.hosting.take() != Some(id) {
			return vec![];
		}
		let Some(battle) = self.state.battles.get(&id) else {
			return vec![];
		};
		let me = self.state.me.clone().unwrap_or_default();
		let alone = battle
			.members
			.iter()
			.all(|member| *member == battle.founder || *member == me);
		let mut effects = vec![Effect::Hosting {
			founder: battle.founder.clone(),
			alone,
		}];
		if !alone {
			return effects;
		}
		// Seated first: SPADS lets a player call `!boss` and refuses a
		// spectator (`commands_custom.conf` `[boss]`), which is why Chobby
		// joins as a player here (`battle_list_window.lua:1755`). The room is
		// ours by intent, so the public-seat setting does not apply.
		self.seat = Some(Seat {
			team: 0,
			ally_team: 0,
			ready: false,
			side: self.seat.map_or(0, |seat| seat.side),
			handicap: 0,
		});
		effects.push(self.battle_status());
		let lines = [format!("!boss {me}"), "!preset custom".to_owned()];
		effects.extend(
			lines
				.iter()
				.filter_map(|line| battle::say_battle(line).ok())
				.map(Effect::Send),
		);
		effects
	}

	/// Announces whether we are in a game. SPADS admits a mid-game joiner to the
	/// running game only after seeing this bit (`spads.pl` `cbClientStatus`), so it
	/// must go out before the engine connects.
	pub fn set_in_game(&mut self, in_game: bool) -> Vec<Effect> {
		self.in_game = in_game;
		vec![self.status()]
	}

	/// Marks us away, or back. One `MYSTATUS` carries both bits, so each is
	/// remembered here — sending one without the other would silently claim we
	/// had left the game we are in.
	pub fn set_away(&mut self, away: bool) -> Vec<Effect> {
		self.away = away;
		vec![self.status()]
	}

	fn status(&self) -> Effect {
		Effect::Send(Envelope::queue(
			Area::Status,
			status::my_status(self.in_game, self.away),
		))
	}

	/// Takes a player slot.
	///
	/// Refused only when seats in public rooms have been turned off, which is
	/// how a client with nobody at the keyboard says it is watching; a room
	/// that is ours is always ours to sit in. Nothing here does it on its own —
	/// it is always a deliberate action from the user.
	pub fn take_seat(
		&mut self,
		team: u8,
		ally_team: u8,
		ready: bool,
	) -> Result<Vec<Effect>, SeatError> {
		if self
			.state
			.my_battle
			.as_ref()
			.and_then(|my| self.state.battles.get(&my.id))
			.is_none()
		{
			return Err(SeatError::NotInARoom);
		}
		// Ready never survives sitting down from watching: a seat taken is not
		// a game agreed to. A change of side while seated keeps it, as Chobby
		// does: the game agreed to is the same one.
		self.seat = Some(Seat {
			team,
			ally_team,
			ready: self.seat.is_some_and(|seat| seat.ready),
			side: self.seat.map_or(0, |seat| seat.side),
			handicap: self.seat.map_or(0, |seat| seat.handicap),
		});
		// Assigned, not set: the newest press decides.
		self.ready_after_seat = ready;
		let mut effects = vec![self.battle_status()];
		effects.extend(self.wish());
		Ok(effects)
	}

	/// Says we are ready, or not. Only a player can be either.
	pub fn set_ready(&mut self, ready: bool) -> Result<Vec<Effect>, SeatError> {
		// An explicit word on ready, either way, supersedes one that was to
		// follow the seat.
		self.ready_after_seat = false;
		let seat = self.seat.ok_or(SeatError::Spectating)?;
		// The seat itself is still on its way and lands as a sit-down. The
		// server clears a ready that arrives with one, and the flood window
		// would merge this into it; so it follows the seat instead.
		if ready && self.sitting_down() {
			self.ready_after_seat = true;
			return Ok(self.wish().into_iter().collect());
		}
		if seat.ready == ready {
			return Ok(vec![]);
		}
		self.seat = Some(Seat { ready, ..seat });
		let mut effects = vec![self.battle_status()];
		effects.extend(self.wish());
		Ok(effects)
	}

	/// Whether the seat we hold has yet to land as a sit-down: the server has
	/// not seen us seated, or a stand-up of ours is still ahead of it.
	fn sitting_down(&self) -> bool {
		let me = self.state.me.as_deref().unwrap_or_default();
		let seated = self
			.state
			.users
			.get(me)
			.and_then(|user| user.battle_status)
			.is_some_and(|status| status.player);
		!seated || self.in_flight.iter().any(Option::is_none)
	}

	/// Picks a faction: 0 Armada, 1 Cortex, 2 Random, 3 Legion.
	pub fn set_side(&mut self, side: u8) -> Result<Vec<Effect>, SeatError> {
		let seat = self.seat.as_mut().ok_or(SeatError::Spectating)?;
		if seat.side == side {
			return Ok(vec![]);
		}
		seat.side = side;
		Ok(vec![self.battle_status()])
	}

	/// Goes back to spectating; always allowed. A pre-ready goes with the
	/// seat: whoever stands up has stopped planning to play.
	pub fn release_seat(&mut self) -> Vec<Effect> {
		self.seat = None;
		self.ready_after_seat = false;
		let Some(my) = self.state.my_battle.as_mut() else {
			return vec![];
		};
		let disarmed = std::mem::take(&mut my.pre_ready);
		let mut effects = vec![self.battle_status()];
		effects.extend(self.wish());
		if disarmed && !effects.contains(&Effect::RoomChanged) {
			effects.push(Effect::RoomChanged);
		}
		effects
	}

	/// Arms a ready given in advance, or takes it back.
	///
	/// The next time the server unreadies us of its own accord -- seating us
	/// from the join queue, or ending a game -- it is answered with one ready,
	/// and then it is spent. The server's unready is taken either way; this
	/// only answers it. Never remembered past the room.
	pub fn set_pre_ready(&mut self, on: bool) -> Result<Vec<Effect>, SeatError> {
		let my = self.state.my_battle.as_mut().ok_or(SeatError::NotInARoom)?;
		if my.pre_ready == on {
			return Ok(vec![]);
		}
		my.pre_ready = on;
		Ok(vec![Effect::RoomChanged])
	}

	/// The server's word on our own status, taken as it stands.
	///
	/// It answers a request of ours still in flight, when there is one (see
	/// `in_flight`). While a newer one is still on its way, that request stays
	/// what we build from, since the server will apply it after this. Once
	/// none is, this is our seat.
	fn our_status(&mut self, status: &spring_protocol::BattleStatus) -> Vec<Effect> {
		let said = Seat::from_status(status);
		let answer = !self.in_flight.is_empty();
		if answer {
			let upto = self
				.in_flight
				.iter()
				.rposition(|&asked| Seat::answers(asked, said))
				.unwrap_or(0);
			self.in_flight.drain(..=upto);
		}
		let mut effects: Vec<Effect> = self.wish().into_iter().collect();
		if !self.in_flight.is_empty() {
			return effects;
		}
		self.seat = said;
		if answer {
			// Our own request answered is nothing to answer back -- unless a
			// Ready pressed while watching is still owed its second half.
			if std::mem::take(&mut self.ready_after_seat)
				&& let Some(seat) = self.seat.filter(|seat| !seat.ready)
			{
				self.seat = Some(Seat {
					ready: true,
					..seat
				});
				effects.push(self.battle_status());
				effects.extend(self.wish());
			}
			return effects;
		}
		let Some(seat) = self.seat.filter(|seat| !seat.ready) else {
			return effects;
		};
		let Some(my) = self.state.my_battle.as_mut().filter(|my| my.pre_ready) else {
			return effects;
		};
		my.pre_ready = false;
		self.seat = Some(Seat {
			ready: true,
			..seat
		});
		effects.push(self.battle_status());
		effects.extend(self.wish());
		if !effects.contains(&Effect::RoomChanged) {
			effects.push(Effect::RoomChanged);
		}
		effects
	}

	/// Keeps the room view's `ready_on_its_way` and `seat_on_its_way` in
	/// step: what our newest request asks for, while the server still shows
	/// otherwise. The page draws it at once, as on its way; the server's word
	/// stays what is true.
	fn wish(&mut self) -> Option<Effect> {
		let newest = self.in_flight.back().copied();
		let asked_ready = newest
			.flatten()
			.map(|seat| seat.ready || self.ready_after_seat);
		let asked_seat = newest.map(|seat| SeatOnItsWay {
			player: seat.is_some(),
			ally_team: seat.map_or(0, |seat| seat.ally_team),
		});
		let me = self.state.me.as_deref().unwrap_or_default();
		let shown = self.state.users.get(me).and_then(|user| user.battle_status);
		let shown_ready = shown.is_some_and(|status| status.player && status.ready);
		let shown_seat = SeatOnItsWay {
			player: shown.is_some_and(|status| status.player),
			ally_team: shown
				.filter(|status| status.player)
				.map_or(0, |status| status.ally_team),
		};
		let ready = asked_ready.filter(|&ready| ready != shown_ready);
		let seat = asked_seat.filter(|&seat| seat != shown_seat);
		let my = self.state.my_battle.as_mut()?;
		if my.ready_on_its_way == ready && my.seat_on_its_way == seat {
			return None;
		}
		my.ready_on_its_way = ready;
		my.seat_on_its_way = seat;
		Some(Effect::RoomChanged)
	}

	/// The runtime's word that the status we last asked for waits for the
	/// flood window, and until when, in Unix milliseconds. The first word
	/// stands: the window opens at one moment however often it is asked.
	pub fn status_held(&mut self, until_ms: u64) -> Option<Effect> {
		let my = self.state.my_battle.as_mut()?;
		if my.held_until_ms.is_some() {
			return None;
		}
		my.held_until_ms = Some(until_ms);
		Some(Effect::RoomChanged)
	}

	/// The runtime's word that a status of ours left.
	pub fn status_sent(&mut self) -> Option<Effect> {
		let my = self.state.my_battle.as_mut()?;
		my.held_until_ms.take().map(|_| Effect::RoomChanged)
	}

	pub fn seat(&self) -> Option<Seat> {
		self.seat
	}

	/// Asks a cluster manager for a room of our own (`!privatehost`); it
	/// answers privately with a password (`battle_list_window.lua:1632-1645`).
	pub fn request_private_host(&mut self, manager: &str) -> Result<Vec<Effect>, battle::TooLong> {
		self.private_host = None;
		Ok(vec![Effect::Send(battle::say_private(
			manager,
			"!privatehost",
		)?)])
	}

	/// Joins a channel. The server answers with `JOIN` or `JOINFAILED`, so
	/// nothing is added to state here.
	pub fn join_channel(
		&mut self,
		room: &str,
		key: Option<&str>,
	) -> Result<Vec<Effect>, chat::BadChannel> {
		Ok(vec![Effect::Send(chat::join(room, key)?)])
	}

	/// Leaves a channel. teiserver reports our own departure as `LEFT`, the
	/// same as anyone else's, which is where the channel is dropped from state.
	pub fn leave_channel(&mut self, room: &str) -> Result<Vec<Effect>, chat::BadChannel> {
		Ok(vec![Effect::Send(chat::leave(room)?)])
	}

	/// Says something in a channel: one line per message, a leading `/me `
	/// an emote, a line past the cap wrapped rather than cut.
	pub fn say_channel(&mut self, room: &str, text: &str) -> Result<Vec<Effect>, chat::SayError> {
		Ok(sends(chat::say_lines(room, text)?))
	}

	/// Says something in the room, a paste as one message per line.
	///
	/// Two things happen to a paste first. A setting the room already has,
	/// as `!bSet key value` or SPADS's shortcut `!key value`, is dropped: a
	/// pasted preset is mostly settings already in place, and SPADS would
	/// only echo each one back, a vote or a line at a time. Then, if more
	/// than one line is left and `burst` allows it, the lines take the burst
	/// lane ([`Area::BattlePaste`]) and go out in a few large writes rather
	/// than paced one by one; see the policy module for why that is only
	/// safe for a sender SPADS does not count.
	pub fn say_battle(
		&mut self,
		text: &str,
		burst: PasteBurst,
	) -> Result<Vec<Effect>, battle::TooLong> {
		let envelopes = battle::say_battle_lines(text)?;
		let Some(my) = self.state.my_battle.as_ref() else {
			return Ok(sends(envelopes));
		};
		// In order: once a line wipes the room's settings, what the room has
		// now says nothing about what the lines after it will find.
		let mut wiped = false;
		let (in_place, mut wanted): (Vec<_>, Vec<_>) =
			envelopes.into_iter().partition(|envelope| {
				wiped = wiped || resets_settings(&envelope.line);
				!wiped && already_set(my, &envelope.line)
			});
		let burst_now = wanted.len() > 1
			&& match burst {
				PasteBurst::Always => true,
				PasteBurst::Never => false,
				PasteBurst::Boss => self.is_boss(),
			};
		if burst_now {
			for envelope in &mut wanted {
				envelope.area = Area::BattlePaste;
			}
		}
		let lines = wanted.len();
		let mut effects = sends(wanted);
		if lines > 1 || !in_place.is_empty() {
			effects.push(Effect::PasteQueued {
				lines,
				skipped: in_place.len(),
			});
		}
		Ok(effects)
	}

	/// Whether SPADS has told us we boss the room we are in. A room can have
	/// several: BarManager sends them joined with commas
	/// (`barmanager.py`, `','.join(spads.getBosses())`).
	pub fn is_boss(&self) -> bool {
		let Some(me) = self.state.me.as_deref() else {
			return false;
		};
		self.state
			.my_battle
			.as_ref()
			.and_then(|my| my.boss.as_deref())
			.is_some_and(|bosses| bosses.split(',').any(|boss| boss.trim() == me))
	}

	/// Sends a direct message. Nothing appears until the server echoes it back.
	pub fn say_private(&mut self, user: &str, text: &str) -> Result<Vec<Effect>, battle::TooLong> {
		Ok(sends(battle::say_private_lines(user, text)?))
	}

	/// Asks a host how long its game has been going.
	///
	/// The only way to learn that about a room you are not in: SPADS answers a
	/// JSON-RPC request carried on a private message. Ask sparingly — it is a
	/// message to a real account, and hovering a list would otherwise send one
	/// per row.
	pub fn request_game_status(&mut self, founder: &str) -> Result<Vec<Effect>, battle::TooLong> {
		Ok(vec![Effect::Send(battle::say_private(
			founder,
			spads::GAME_STATUS_REQUEST,
		)?)])
	}

	/// The battle a host runs, if we can see one.
	fn battle_hosted_by(&self, founder: &str) -> Option<u32> {
		self.state
			.battles
			.values()
			.find(|battle| battle.founder == founder)
			.map(|battle| battle.id)
	}

	/// Rings someone: the lobby's way of saying the game is waiting on you.
	pub fn ring(&mut self, user: &str) -> Vec<Effect> {
		vec![Effect::Send(battle::ring(user))]
	}

	/// Adds an AI to the room, seated and ready.
	///
	/// The AI runs on this machine when the game starts, so the caller names
	/// one that is installed here. Whether the room lets us is the server's
	/// call — SPADS answers a refusal in chat, where it will be seen.
	pub fn add_bot(
		&mut self,
		name: &str,
		ai: &str,
		team: u8,
		ally_team: u8,
		colour: u32,
	) -> Vec<Effect> {
		let status =
			battle::MyBattleStatus::player(battle::Sync::Synced, team, ally_team).ready(true);
		vec![Effect::Send(battle::add_bot(name, ai, status, colour))]
	}

	/// Moves an AI, or changes its bonus, colour or faction.
	///
	/// Only the AI's owner, the room's founder and moderators may: teiserver
	/// drops anyone else's `UPDATEBOT` without a word (`lobby.ex:722`), so a
	/// caller that is none of those should be asking the host by chat instead.
	/// The whole status goes out each time, since the message replaces it.
	pub fn update_bot(
		&mut self,
		name: &str,
		team: u8,
		ally_team: u8,
		handicap: u8,
		colour: u32,
	) -> Vec<Effect> {
		let status = battle::MyBattleStatus::player(battle::Sync::Synced, team, ally_team)
			.ready(true)
			.handicap(handicap);
		vec![Effect::Send(battle::update_bot(name, status, colour))]
	}

	/// Removes an AI by name; the server decides whether we may.
	pub fn remove_bot(&mut self, name: &str) -> Vec<Effect> {
		vec![Effect::Send(battle::remove_bot(name))]
	}

	/// Asks for both friend listings; each replaces what it had.
	pub fn refresh_friends(&mut self) -> Vec<Effect> {
		vec![
			Effect::Send(friends::list()),
			Effect::Send(friends::list_requests()),
			Effect::Send(friends::list_ignored()),
		]
	}

	/// Acts on a friendship, then asks for the listings again — the server
	/// sends nothing of its own accord when one changes.
	pub fn friend_action(&mut self, action: FriendAction, user: &str) -> Vec<Effect> {
		let envelope = match action {
			FriendAction::Request => friends::request(user),
			FriendAction::Accept => friends::accept(user),
			FriendAction::Decline => friends::decline(user),
			FriendAction::Remove => friends::remove(user),
			FriendAction::Ignore => friends::ignore(user),
			FriendAction::Unignore => friends::unignore(user),
		};
		let mut effects = vec![Effect::Send(envelope)];
		effects.extend(self.refresh_friends());
		effects
	}

	/// Asks for the server's channel directory, which replaces the last one.
	pub fn list_channels(&mut self) -> Vec<Effect> {
		self.state.directory.clear();
		vec![Effect::Send(chat::list())]
	}

	/// Reports whether the room's content is installed. Only a caller that has
	/// actually looked at the disk should say `true`.
	pub fn set_synced(&mut self, synced: bool) -> Vec<Effect> {
		if self.synced == synced {
			return vec![];
		}
		self.synced = synced;
		if self.state.my_battle.is_none() {
			return vec![];
		}
		vec![self.battle_status()]
	}

	pub fn is_synced(&self) -> bool {
		self.synced
	}

	/// The runtime's word that the newest status replaced one still waiting
	/// for the flood window (`PolicyEvent::Coalesced`). The replaced request
	/// never leaves, so no answer to it comes; left waiting, it would take
	/// the answer meant for the request that replaced it.
	pub fn status_replaced(&mut self) {
		let n = self.in_flight.len();
		if n >= 2 {
			self.in_flight.remove(n - 2);
		}
	}

	fn battle_status(&mut self) -> Effect {
		self.in_flight.push_back(self.seat);
		let sync = if self.synced {
			Sync::Synced
		} else {
			Sync::Unsynced
		};
		let status = match self.seat {
			Some(seat) => MyBattleStatus::player(sync, seat.team, seat.ally_team)
				.ready(seat.ready)
				.side(seat.side)
				.handicap(seat.handicap),
			None => MyBattleStatus::spectator(sync),
		};
		// Coalesced: a status still waiting for the flood window
		// (`policy.rs`, five in any eight seconds, under SPADS's status-flood
		// kick) is replaced by the newest, so rapid changes send only the last.
		Effect::Send(Envelope::coalesce(Area::BattleStatus, "me", status.line()))
	}

	pub fn leave_battle(&mut self) -> Vec<Effect> {
		self.pending_join = None;
		self.hosting = None;
		self.seat = None;
		self.in_flight.clear();
		let Some(my) = self.state.my_battle.take() else {
			return vec![];
		};
		self.state.forget_room_details(my.id);
		vec![
			Effect::Send(Envelope::queue(Area::Other, battle::LEAVE_BATTLE)),
			Effect::LeftBattle { id: my.id },
		]
	}

	pub fn handle(&mut self, event: ServerEvent) -> Vec<Effect> {
		use ServerEvent as E;
		let state = &mut self.state;
		match event {
			E::Welcome { server_version, .. } => {
				tracing::info!(server_version, "connected");
				state.phase = Some(Phase::AwaitingLogin);
				self.teiserver = server_version == login::TEISERVER_VERSION;
				let line = match &self.intent {
					Intent::Login => self.login.line_for(&server_version),
					Intent::Register { email, password } => {
						login::register(&self.login.username, password, email)
					}
				};
				vec![Effect::Send(Envelope::queue(Area::Login, line))]
			}
			E::Accepted { username } => {
				state.phase = Some(Phase::Loading);
				state.me = Some(username.clone());
				vec![Effect::LoggedIn { username }]
			}
			E::Denied { reason } => vec![Effect::LoginDenied { reason }],
			E::Queued => vec![Effect::Notice("login queued by server, waiting".into())],
			E::Agreement { line } => {
				self.agreement.push(line);
				vec![]
			}
			E::AgreementEnd => vec![Effect::AgreementRequired {
				text: std::mem::take(&mut self.agreement),
			}],
			E::RegistrationAccepted => vec![Effect::Registered],
			E::RegistrationDenied { reason } => vec![Effect::RegistrationDenied { reason }],
			E::Motd { line } => {
				state.motd.push(line);
				vec![]
			}
			E::CompFlags { flags } => {
				state.comp_flags = flags;
				vec![]
			}
			E::ServerMsg { text } => match machine_marker(&text) {
				// teiserver rides machine data on SERVERMSG behind an
				// `@MARKER@` prefix (`spring_out.ex:82`). It is protocol, not
				// prose, and a person should never be shown it.
				Some(marker) => {
					tracing::debug!(marker, "server extension message");
					vec![]
				}
				// Kept as well as flashed: a broadcast you were away for is
				// still worth being able to scroll back to.
				None => vec![
					Effect::ServerSaid { text: text.clone() },
					Effect::Notice(text),
				],
			},
			E::LoginInfoEnd => {
				state.phase = Some(Phase::Ready);
				// `c.telemetry.*` is teiserver's; uberserver answers each
				// property with "Unknown command".
				let hardware = if self.teiserver {
					self.hardware.as_slice()
				} else {
					&[]
				};
				let mut effects: Vec<Effect> = hardware
					.iter()
					.map(|(name, value)| {
						Effect::Send(Envelope::queue(
							Area::Other,
							telemetry::update_client_property(name, value, &self.machine_hash),
						))
					})
					.collect();
				// The message of the day arrives before the login is complete,
				// so it is replayed here — in order, once there is somewhere
				// for it to go.
				effects.extend(
					state
						.motd
						.iter()
						.map(|line| Effect::Motd { text: line.clone() }),
				);
				effects.push(Effect::Ready);
				effects
			}
			E::AddUser {
				name,
				country,
				user_id,
				lobby_client,
			} => {
				state.add_user(name, country, user_id, lobby_client);
				vec![]
			}
			E::RemoveUser { name } => {
				state.remove_user(&name);
				vec![]
			}
			E::ClientStatus { name, status } => {
				let was_in_game = state.users.get(&name).is_some_and(|u| u.status.in_game);
				state.set_status(&name, status);
				if !self.hosts_my_battle(&name) || status.in_game == was_in_game {
					return vec![];
				}
				if status.in_game {
					// The host's bit went up while we were standing here. A new
					// game, so nobody has put us in it yet.
					if let Some(my) = self.state.my_battle.as_mut() {
						my.joined_id = None;
					}
					return self.game_running(true).into_iter().collect();
				}
				// The bit going the other way is the only sign a game ended.
				// Without this the room goes on offering to connect you to one
				// that finished, for as long as you stay in it.
				vec![Effect::GameStopped]
			}
			E::BattleOpened(opened) => {
				// The private room a cluster manager spun up for us carries our
				// name in its title (`battle_list_window.lua:1664`).
				let mine = self.private_host.as_ref().and_then(|password| {
					let me = state.me.as_deref()?;
					opened
						.title
						.starts_with(me)
						.then(|| (opened.id, password.clone()))
				});
				state.open_battle(*opened);
				match mine {
					Some((id, password)) => {
						self.private_host = None;
						vec![Effect::PrivateHostReady { id, password }]
					}
					None => vec![],
				}
			}
			E::BattleClosed { id } => {
				state.close_battle(id);
				if state.my_battle.as_ref().is_some_and(|my| my.id == id) {
					state.my_battle = None;
					return vec![Effect::LeftBattle { id }];
				}
				vec![]
			}
			E::UpdateBattleInfo {
				id,
				spectator_count,
				locked,
				map_hash,
				map_name,
			} => {
				if let Some(battle) = state.battles.get_mut(&id) {
					battle.spectator_count = spectator_count;
					battle.locked = locked;
					battle.map_hash = map_hash;
					battle.map_name = map_name;
				}
				vec![]
			}
			E::JoinedBattle { id, name, .. } => {
				state.join_battle(id, name);
				vec![]
			}
			E::LeftBattle { id, name } => {
				state.leave_battle(id, &name);
				let me = state.me.as_deref() == Some(name.as_str());
				if me && state.my_battle.as_ref().is_some_and(|my| my.id == id) {
					state.my_battle = None;
					state.forget_room_details(id);
					self.seat = None;
					self.in_flight.clear();
					return vec![Effect::LeftBattle { id }];
				}
				vec![]
			}
			E::ClientBattleStatus { name, status, .. } => {
				if let Some(user) = state.users.get_mut(&name) {
					user.battle_status = Some(status);
				}
				let ours = state.my_battle.is_some() && state.me.as_deref() == Some(name.as_str());
				if !ours {
					return vec![];
				}
				self.our_status(&status)
			}
			E::JoinBattle { id, game_hash } => {
				// The room's word on its game, which a copy is held to.
				tracing::info!(id, game_hash, "joined a room");
				let script_password = self.pending_join.take().unwrap_or_default();
				state.my_battle = Some(MyBattle::new(id, game_hash, script_password));
				self.in_flight.clear();
				self.ready_after_seat = false;
				let mut effects = vec![Effect::Joined { id }];
				effects.extend(self.claim_room(id));
				// Already under way before we arrived: worth saying, not worth
				// acting on.
				effects.extend(self.game_running(false));
				effects
			}
			E::JoinBattleFailed { reason } => {
				self.pending_join = None;
				self.hosting = None;
				vec![Effect::JoinFailed { reason }]
			}
			E::RequestBattleStatus => {
				// The last line of the join replay: from here on a tag that
				// changes is history, not the room introducing itself.
				if let Some(my) = state.my_battle.as_mut() {
					my.settled = true;
				}
				vec![self.battle_status()]
			}
			E::SentPrivate { name, text } => {
				// The server echoing us back is the only confirmation the
				// message left, so it is what puts our own words on screen.
				let me = state.me.clone().unwrap_or_default();
				vec![Effect::PrivateChat {
					with: name,
					from: me,
					text,
				}]
			}
			E::JoinedChannel { room } => {
				self.state
					.channels
					.entry(room.clone())
					.or_insert_with(|| Channel {
						name: room.clone(),
						..Channel::default()
					});
				vec![Effect::ChannelJoined { room }]
			}
			E::JoinChannelFailed { room, reason } => {
				vec![Effect::ChannelJoinFailed { room, reason }]
			}
			E::Clients { room, names } => {
				// Sent in batches, so this adds to the roster rather than
				// replacing it; a second batch must not erase the first.
				let Some(channel) = self.state.channels.get_mut(&room) else {
					return vec![];
				};
				channel.members.extend(names);
				vec![Effect::ChannelChanged { room }]
			}
			E::JoinedRoom { room, name } => {
				let Some(channel) = self.state.channels.get_mut(&room) else {
					return vec![];
				};
				channel.members.insert(name);
				vec![Effect::ChannelChanged { room }]
			}
			E::LeftRoom { room, name } => {
				let Some(channel) = self.state.channels.get_mut(&room) else {
					return vec![];
				};
				// The server tells us about our own departure the same way it
				// tells us about anyone else's.
				if Some(&name) == self.state.me.as_ref() {
					self.state.channels.remove(&room);
					return vec![Effect::ChannelLeft { room }];
				}
				channel.members.remove(&name);
				vec![Effect::ChannelChanged { room }]
			}
			E::ChannelTopic { room, author } => {
				let Some(channel) = self.state.channels.get_mut(&room) else {
					return vec![];
				};
				channel.topic_author = Some(author);
				vec![Effect::ChannelChanged { room }]
			}
			E::Said { room, name, text } => vec![Effect::ChannelChat {
				room,
				from: name,
				text,
				emote: false,
			}],
			E::SaidEx { room, name, text } => vec![Effect::ChannelChat {
				room,
				from: name,
				text,
				emote: true,
			}],
			E::ChannelListed { name, members } => {
				self.state
					.directory
					.push(crate::state::ChannelSummary { name, members });
				vec![]
			}
			E::EndOfChannels => vec![Effect::ChannelsListed],
			// Both listings arrive whole, so each one is collected into a
			// fresh set and swapped in at the end: a friend removed elsewhere
			// has to disappear here too.
			E::Ring { name } => vec![Effect::Rung { by: name }],
			E::ForceQuitBattle => {
				// The server has already dropped us; keeping the room on screen
				// would leave every action in it silently doing nothing.
				let Some(my) = state.my_battle.take() else {
					return vec![];
				};
				state.forget_room_details(my.id);
				self.seat = None;
				self.in_flight.clear();
				vec![
					Effect::Notice("you were removed from the room".into()),
					Effect::LeftBattle { id: my.id },
				]
			}
			E::FriendListBegin => {
				self.collecting_friends = Some(std::collections::BTreeSet::new());
				vec![]
			}
			E::Friend { name } => {
				if let Some(collecting) = self.collecting_friends.as_mut() {
					collecting.insert(name);
				}
				vec![]
			}
			E::FriendListEnd => {
				let Some(collected) = self.collecting_friends.take() else {
					return vec![];
				};
				self.state.friends = collected;
				vec![Effect::FriendsChanged]
			}
			E::IgnoreListBegin => {
				self.collecting_ignored = Some(std::collections::BTreeSet::new());
				vec![]
			}
			E::Ignored { name } => {
				if let Some(collecting) = self.collecting_ignored.as_mut() {
					collecting.insert(name);
				}
				vec![]
			}
			E::IgnoreListEnd => {
				let Some(collected) = self.collecting_ignored.take() else {
					return vec![];
				};
				self.state.ignored = collected;
				vec![Effect::FriendsChanged]
			}
			E::FriendRequestListBegin => {
				self.collecting_requests = Some(std::collections::BTreeSet::new());
				vec![]
			}
			E::FriendRequest { name } => {
				if let Some(collecting) = self.collecting_requests.as_mut() {
					collecting.insert(name);
				}
				vec![]
			}
			E::FriendRequestListEnd => {
				let Some(collected) = self.collecting_requests.take() else {
					return vec![];
				};
				self.state.friend_requests = collected;
				vec![Effect::FriendsChanged]
			}
			E::SaidBattle { name, text } => vec![Effect::BattleChat {
				from: name,
				text,
				announcement: false,
			}],
			E::SaidBattleEx { name, text } => {
				let mut effects = self.host_announcement(&name, &text);
				effects.push(Effect::BattleChat {
					from: name,
					text,
					announcement: true,
				});
				effects
			}
			E::SaidPrivate { name, text } => {
				let mut effects = Vec::new();
				// A host answering what we asked about its game, on the side
				// channel SPADS carries over private messages.
				if let Some(spads::RpcStatus::Game { seconds, .. }) = spads::parse_rpc(&text)
					&& let Some(id) = self.battle_hosted_by(&name)
				{
					effects.push(Effect::GameInProgress {
						id,
						elapsed_secs: seconds,
					});
				}
				// Who shares the ID our room's host put us on, which is what
				// it was asked when it did.
				if self.hosts_my_battle(&name)
					&& let Some(id) = self.state.my_battle.as_ref().and_then(|my| my.joined_id)
					&& let Some(players) = spads::players_on(&text, id)
				{
					let me = self.state.me.as_deref();
					let names = players
						.into_iter()
						.filter(|player| Some(player.as_str()) != me)
						.collect();
					effects.push(Effect::PlayingWith { names });
				}
				if let Some(password) = private_host_password(&text) {
					self.private_host = Some(password.clone());
					effects.push(Effect::PrivateHostOffered {
						manager: name.clone(),
						password,
					});
				}
				effects.push(Effect::PrivateChat {
					with: name.clone(),
					from: name,
					text,
				});
				effects
			}
			E::SetScriptTags { tags } => {
				let Some(my) = state.my_battle.as_mut() else {
					return vec![];
				};
				let keys = my.set_script_tags(tags);
				if keys.is_empty() {
					return vec![];
				}
				vec![Effect::ModOptionsChanged { keys }]
			}
			E::RemoveScriptTags { keys } => {
				if let Some(my) = state.my_battle.as_mut() {
					for key in &keys {
						my.script_tags.remove(key);
					}
				}
				vec![]
			}
			E::AddBot {
				id,
				name,
				owner,
				status,
				team_colour,
				ai,
			} => {
				if let Some(battle) = state.battles.get_mut(&id) {
					let bot = Bot {
						name: name.clone(),
						owner,
						status,
						team_colour,
						ai,
					};
					battle.bots.insert(name, bot);
				}
				vec![]
			}
			E::UpdateBot {
				id,
				name,
				status,
				team_colour,
			} => {
				let bot = state
					.battles
					.get_mut(&id)
					.and_then(|battle| battle.bots.get_mut(&name));
				if let Some(bot) = bot {
					bot.status = status;
					bot.team_colour = team_colour;
				}
				vec![]
			}
			E::RemoveBot { id, name } => {
				if let Some(battle) = state.battles.get_mut(&id) {
					battle.bots.remove(&name);
				}
				vec![]
			}
			E::AddStartRect {
				ally_team,
				left,
				top,
				right,
				bottom,
			} => {
				if let Some(battle) = state.my_room_mut() {
					let rect = StartRect {
						left,
						top,
						right,
						bottom,
					};
					battle.start_rects.insert(ally_team, rect);
				}
				vec![]
			}
			E::RemoveStartRect { ally_team } => {
				if let Some(battle) = state.my_room_mut() {
					battle.start_rects.remove(&ally_team);
				}
				vec![]
			}
			E::BattleTitle { id, title } => {
				if let Some(battle) = state.battles.get_mut(&id) {
					battle.title = title;
				}
				vec![]
			}
			E::BattleTeams { layouts } => {
				for (id, layout) in layouts {
					if let Some(battle) = state.battles.get_mut(&id) {
						battle.layout = Some(layout);
					}
				}
				vec![]
			}
			E::BattleQueue { id, names } => {
				// No longer queued and still without a seat: we left the queue,
				// by button or by `$leaveq` typed, and a ready given for it goes
				// with it. The status seating us arrives before this update
				// (`consul_server.ex` `player_count_changed`), so a seating has
				// already found it.
				let me = state.me.as_deref().unwrap_or_default();
				let left = self.seat.is_none() && !names.iter().any(|name| name == me);
				if let Some(battle) = state.battles.get_mut(&id) {
					battle.queue = names;
				}
				match state.my_battle.as_mut() {
					Some(my) if my.id == id && left && my.pre_ready => {
						my.pre_ready = false;
						vec![Effect::RoomChanged]
					}
					_ => vec![],
				}
			}
			E::Redirect { host, port } => vec![Effect::Redirect { host, port }],
			E::Disconnect { reason } => {
				let flood = reason.contains("Flood");
				vec![Effect::Disconnected { reason, flood }]
			}
			E::Shutdown => vec![Effect::Disconnected {
				reason: "server shutdown".into(),
				flood: false,
			}],
			E::Pong => vec![],
			E::Unknown(raw) => {
				tracing::debug!(command = raw.command, "unhandled command");
				vec![]
			}
			E::Malformed(raw) => {
				tracing::warn!(command = raw.command, args = raw.args, "malformed command");
				vec![]
			}
		}
	}

	/// SPADS announces votes and setting changes as chat. Only the room's
	/// founder is believed — Chobby gates on the same thing
	/// (`gui_battle_room_window.lua:4848`).
	fn host_announcement(&mut self, from: &str, text: &str) -> Vec<Effect> {
		if !self.hosts_my_battle(from) {
			return vec![];
		}
		let Some(announcement) = spads::parse(text) else {
			return vec![];
		};
		let Some(my) = self.state.my_battle.as_mut() else {
			return vec![];
		};
		match announcement {
			Announcement::VoteCalled { by, command } => {
				my.vote = Some(VoteState::called(by, command));
				vec![Effect::VoteChanged]
			}
			Announcement::VoteProgress {
				command,
				yes,
				yes_needed,
				no,
				no_needed,
				remaining_secs,
			} => {
				let vote = my
					.vote
					.get_or_insert_with(|| VoteState::called(String::new(), command));
				vote.yes = yes;
				vote.yes_needed = yes_needed;
				vote.no = no;
				vote.no_needed = no_needed;
				vote.remaining_secs = remaining_secs;
				vec![Effect::VoteChanged]
			}
			Announcement::VoteEnded { .. } | Announcement::VoteCancelled => {
				my.vote = None;
				vec![Effect::VoteChanged]
			}
			// The one statement of a game's age this protocol carries, and it
			// is addressed to us alone as we walk in.
			Announcement::GameInProgress { elapsed_secs } => {
				vec![Effect::GameInProgress {
					id: my.id,
					elapsed_secs,
				}]
			}
			// The line names the ID and not whose it is, so the host is asked.
			Announcement::PlayerAdded { name, id } => {
				if self.state.me.as_deref() != Some(name.as_str()) {
					return vec![];
				}
				my.joined_id = Some(id);
				let mut effects = vec![Effect::PlayingWith { names: vec![] }];
				effects.extend(
					battle::say_private(from, spads::GAME_STATUS_REQUEST)
						.ok()
						.map(Effect::Send),
				);
				effects
			}
			Announcement::SettingChanged { by, key, value } => {
				if my.setting_changed(key.clone(), value, by) {
					return vec![Effect::ModOptionsChanged { keys: vec![key] }];
				}
				vec![]
			}
			// The BAR plugin's JSON duplicates what the text already told us.
			Announcement::BarManager { json } => {
				// The room's own statement of who is in charge of it, and of
				// whether it arranges its own teams.
				let boss = spads::boss(&json);
				let auto_balance = spads::auto_balance(&json);
				let preset = spads::preset(&json);
				let Some(my) = self.state.my_battle.as_mut() else {
					return vec![];
				};
				let settled =
					my.boss == boss && my.auto_balance == auto_balance && my.preset == preset;
				if settled {
					return vec![];
				}
				my.boss = boss;
				my.auto_balance = auto_balance;
				my.preset = preset;
				vec![Effect::BossChanged]
			}
		}
	}

	/// Cluster managers, the bots that spin up rooms: `Host[EU1]`, `Host[AU2]`.
	pub fn cluster_managers(&self) -> Vec<&str> {
		let mut names: Vec<&str> = self
			.state
			.users
			.values()
			.filter(|user| user.status.bot && hosting::is_manager(&user.name))
			.map(|user| user.name.as_str())
			.collect();
		names.sort_unstable();
		names
	}

	/// How many rooms a cluster is already running.
	///
	/// A manager `Host[EU1]` runs instances named `Host[EU1][012]`, and each
	/// instance is one room, so counting the instances counts the load.
	fn cluster_load(&self, manager: &str) -> u32 {
		self.state
			.users
			.values()
			.filter(|user| user.status.bot && hosting::cluster_of(&user.name) == Some(manager))
			.count() as u32
	}

	/// The machines the clusters run on, by manager: a manager shares its
	/// address with every room it runs, and the rooms are what carry one.
	/// A manager with no room on the list has no address to give.
	pub fn cluster_ips(&self) -> BTreeMap<&str, Ipv4Addr> {
		self.state
			.battles
			.values()
			.filter_map(|battle| {
				let cluster = hosting::cluster_of(&battle.founder)?;
				Some((cluster, battle.ip.parse().ok()?))
			})
			.collect()
	}

	/// Picks a cluster manager, favouring the near and then the emptiest.
	///
	/// Chobby weights by `(1 - current/limit) * (limit - current)` and draws
	/// against the total (`battle_list_window.lua:1453`), which spreads rooms
	/// across clusters instead of piling every request onto whichever name
	/// happens to sort first. `roll` is a fresh number in `0.0..1.0`.
	pub fn pick_cluster_manager(&self, rtts: &Rtts, roll: f64) -> Option<&str> {
		/// Chobby's default when a cluster reports no capacity of its own.
		const LIMIT: f64 = 80.0;

		let ips = self.cluster_ips();
		let measured: Vec<(&str, Option<Duration>)> = self
			.cluster_managers()
			.into_iter()
			.map(|manager| {
				let rtt = ips.get(manager).and_then(|ip| rtts.get(ip)).copied();
				(manager, rtt)
			})
			.collect();
		let weighted: Vec<(&str, f64)> = hosting::near(measured)
			.into_iter()
			.map(|manager| {
				let current = f64::from(self.cluster_load(manager)).min(LIMIT);
				let weight = (1.0 - current / LIMIT) * (LIMIT - current);
				(manager, weight.max(0.0))
			})
			.collect();

		let total: f64 = weighted.iter().map(|(_, weight)| weight).sum();
		if total <= 0.0 {
			// Every cluster is full, or none reported capacity; the caller
			// still deserves an answer rather than silence.
			return weighted.first().map(|(manager, _)| *manager);
		}

		let mut drawn = roll.clamp(0.0, 1.0) * total;
		for (manager, weight) in &weighted {
			drawn -= weight;
			if drawn <= 0.0 {
				return Some(manager);
			}
		}
		weighted.last().map(|(manager, _)| *manager)
	}

	/// Autohost rooms nobody is in, on the engine everyone is on.
	///
	/// Joining one is how Chobby's Host button makes a *public* room: the
	/// autohost is already listed, and the first person in it becomes its
	/// boss. Chobby's tests (`battle_list_window.lua:1719-1727`), with one
	/// stand-in: it wants the engine it runs on, which has no meaning for a
	/// lobby that fetches engines, so the engine most rooms are on is taken
	/// to be the current one. That keeps the `ENGINE TESTING` hosts out.
	pub fn spare_rooms(&self) -> Vec<SpareRoom> {
		let Some(engine) = self.common_engine() else {
			return vec![];
		};
		let host_in_game = |founder: &str| {
			self.state
				.users
				.get(founder)
				.is_some_and(|user| user.status.in_game)
		};
		let mut rooms: Vec<SpareRoom> =
			self.state
				.battles
				.values()
				.filter(|battle| {
					!battle.passworded
						&& !battle.locked && battle.spectator_count <= 1
						&& battle.members.len() == 1
						&& battle.engine_version == engine
						&& !host_in_game(&battle.founder)
				})
				.filter_map(|battle| {
					Some(SpareRoom {
						id: battle.id,
						founder: battle.founder.clone(),
						cluster: hosting::cluster_of(&battle.founder)?.to_owned(),
						ip: battle.ip.parse().ok(),
					})
				})
				.collect();
		rooms.sort_by_key(|room| room.id);
		rooms
	}

	/// The machines with a spare room on them, each once. These are the only
	/// ones worth measuring: a public room is one of these spares, and a
	/// cluster with no spare is either starting one or at its limit, so it
	/// is not where to ask for a private one either.
	pub fn spare_machines(&self) -> Vec<Ipv4Addr> {
		let machines: std::collections::BTreeSet<Ipv4Addr> = self
			.spare_rooms()
			.into_iter()
			.filter_map(|room| room.ip)
			.collect();
		machines.into_iter().collect()
	}

	/// The engine most listed rooms run; the alphabetically last on a tie,
	/// which during a rollout is the newer one.
	fn common_engine(&self) -> Option<&str> {
		let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
		for battle in self.state.battles.values() {
			*counts.entry(&battle.engine_version).or_default() += 1;
		}
		counts
			.into_iter()
			.max_by_key(|(engine, count)| (*count, *engine))
			.map(|(engine, _)| engine)
	}

	fn hosts_my_battle(&self, name: &str) -> bool {
		self.state
			.my_battle
			.as_ref()
			.and_then(|my| self.state.battles.get(&my.id))
			.is_some_and(|battle| battle.founder == name)
	}

	/// [`Effect::GameRunning`] when our room's host is in game right now.
	fn game_running(&self, just_started: bool) -> Option<Effect> {
		let my = self.state.my_battle.as_ref()?;
		let battle = self.state.battles.get(&my.id)?;
		let host = self.state.users.get(&battle.founder)?;
		host.status.in_game.then(|| Effect::GameRunning {
			id: my.id,
			ip: battle.ip.clone(),
			port: battle.port,
			script_password: my.script_password.clone(),
			just_started,
		})
	}
}

/// `… Starting a new private instance in …, password=XXXX` — the reply Chobby
/// scrapes at `battle_list_window.lua:1634-1637`.
/// Whether `line` sets a modoption to the value the room has now.
///
/// Two spellings set one: `!bSet key value`, and `!key value`, which SPADS
/// rewrites to the former when `key` is a modoption rather than a command
/// (`spads.pl`, `allowSettingsShortcut`, near line 3044). The second only
/// matches when the room has that key at that value, so `!preset custom`
/// stays a command here too. Only an exact match counts: SPADS may
/// normalise what it stores, and a guess about that would skip a change
/// someone meant.
fn already_set(my: &MyBattle, line: &str) -> bool {
	let Some(command) = line.strip_prefix("SAYBATTLE !") else {
		return false;
	};
	let (key, value) = match spads::Proposal::parse(command) {
		spads::Proposal::SetOption { key, value } => (key, value),
		spads::Proposal::Other => {
			let mut words = command.split_whitespace();
			let (Some(word), Some(value)) = (words.next(), words.next()) else {
				return false;
			};
			(word.to_ascii_lowercase(), value.to_owned())
		}
	};
	!value.is_empty() && my.modoption(&key) == value
}

/// Whether a line wipes the room's settings, so nothing after it in the same
/// paste can be compared with what the room has now. A preset applies its
/// battle preset, which on BAR always carries `resetoptions:1`
/// (`battlePresets.conf`), and that deletes every modoption before setting
/// its own (`SpadsConf.pm`, `applyBPreset`, near line 2619). `reloadConf`
/// re-applies the presets; `rck` keeps settings but is not worth a special
/// case. `gametype` is BAR's alias for `preset` (`CustomAliases.conf`). A map
/// change is not one of these: BAR has no per-map presets.
fn resets_settings(line: &str) -> bool {
	let Some(command) = line.strip_prefix("SAYBATTLE !") else {
		return false;
	};
	let word = command
		.split_whitespace()
		.next()
		.unwrap_or_default()
		.to_ascii_lowercase();
	matches!(
		word.as_str(),
		"preset" | "gametype" | "bpreset" | "reloadconf" | "rc" | "rck"
	)
}

/// One `Send` per line the protocol layer produced.
fn sends(envelopes: Vec<spring_protocol::Envelope>) -> Vec<Effect> {
	envelopes.into_iter().map(Effect::Send).collect()
}

fn private_host_password(text: &str) -> Option<String> {
	let (_, rest) = text.split_once("password=")?;
	let password: String = rest
		.chars()
		.take_while(char::is_ascii_alphanumeric)
		.collect();
	(!password.is_empty()).then_some(password)
}

/// `@NAME@ …`: teiserver's convention for data addressed to the client rather
/// than to the person using it.
fn machine_marker(text: &str) -> Option<&str> {
	let (name, _) = text.strip_prefix('@')?.split_once('@')?;
	let machine = !name.is_empty()
		&& name
			.chars()
			.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
	machine.then_some(name)
}

#[cfg(test)]
mod tests {
	#[test]
	fn a_server_extension_line_is_not_shown_to_anyone() {
		use super::machine_marker;
		// What teiserver actually sends on connect (`spring_out.ex:82`).
		assert_eq!(
			machine_marker("@PROTOCOL_EXTENSIONS@ {\"ring:originator\":1}"),
			Some("PROTOCOL_EXTENSIONS")
		);
		// Prose keeps its notice, including prose that merely mentions an `@`.
		assert_eq!(machine_marker("Welcome to teiserver"), None);
		assert_eq!(machine_marker("ask @someone about it"), None);
		assert_eq!(machine_marker("@@"), None);
	}
	use spring_protocol::{BattleStatus, RawMessage};

	use super::*;

	fn session() -> Session {
		Session::new(
			LoginRequest::new("me", "pw", "test", "h h"),
			vec![("hardware:cpuinfo".into(), "cpu".into())],
			"MH".into(),
		)
	}

	#[test]
	fn registering_sends_register_instead_of_login() {
		let mut session = session().registering("a@b.c", "pw");
		let effects = feed(&mut session, &["TASSERVER 0.38 * 8201 0"]);
		let [Effect::Send(envelope)] = &effects[..] else {
			panic!("expected one line, got {effects:?}");
		};
		// Same bucket as a login, since the server rate-limits both, and the
		// password is hashed the same way.
		assert_eq!(envelope.area, Area::Login);

		// The name comes from the login request, not the email, and the
		// password is hashed exactly as a login would hash it — which is the
		// contract that matters, so it is asserted against a login rather than
		// against a hash copied into the test.
		let hash = LoginRequest::new("me", "pw", "test", "h h")
			.line()
			.split_whitespace()
			.nth(2)
			.expect("the hash")
			.to_owned();
		assert_eq!(envelope.line, format!("REGISTER me {hash} a@b.c"));
	}

	#[test]
	fn the_server_accepting_or_refusing_is_reported_once() {
		let mut accepted = session().registering("a@b.c", "pw");
		assert_eq!(
			feed(&mut accepted, &["REGISTRATIONACCEPTED"]),
			vec![Effect::Registered]
		);

		let mut denied = session().registering("a@b.c", "pw");
		assert_eq!(
			feed(&mut denied, &["REGISTRATIONDENIED Username already taken"]),
			vec![Effect::RegistrationDenied {
				reason: "Username already taken".into()
			}]
		);
	}

	#[test]
	fn confirming_the_agreement_carries_the_emailed_code() {
		let mut session = session();
		let effects = session.confirm_agreement("A1B2C3");
		let [Effect::Send(envelope)] = &effects[..] else {
			panic!("expected one line");
		};
		assert_eq!(envelope.line, "CONFIRMAGREEMENT A1B2C3");
	}

	fn feed(session: &mut Session, lines: &[&str]) -> Vec<Effect> {
		lines
			.iter()
			.flat_map(|l| session.handle(RawMessage::parse(l).into()))
			.collect()
	}

	/// A session that believes it is logged in as `me`, without the flood.
	fn joined_session() -> Session {
		let mut s = session();
		s.state.me = Some("me".into());
		s
	}

	#[test]
	fn a_channel_roster_is_built_from_batches_rather_than_replaced() {
		let mut s = joined_session();
		feed(&mut s, &["JOIN main"]);
		// teiserver sends CLIENTS in batches; a later one must not erase the
		// earlier one (`spring_out.ex:442`).
		feed(&mut s, &["CLIENTS main alice bob", "CLIENTS main carol"]);

		let channel = &s.state.channels["main"];
		assert_eq!(
			channel.members.iter().cloned().collect::<Vec<_>>(),
			vec!["alice", "bob", "carol"]
		);
	}

	#[test]
	fn people_arriving_and_leaving_move_the_roster() {
		let mut s = joined_session();
		feed(&mut s, &["JOIN main", "CLIENTS main alice bob"]);
		feed(&mut s, &["JOINED main carol", "LEFT main alice"]);

		let members = &s.state.channels["main"].members;
		assert!(members.contains("carol"));
		assert!(!members.contains("alice"));
	}

	#[test]
	fn our_own_departure_drops_the_channel_entirely() {
		let mut s = joined_session();
		feed(&mut s, &["JOIN main", "CLIENTS main me alice"]);

		// The server reports us leaving exactly as it reports anyone else.
		let effects = feed(&mut s, &["LEFT main me"]);
		assert!(matches!(
			effects.as_slice(),
			[Effect::ChannelLeft { room }] if room == "main"
		));
		assert!(!s.state.channels.contains_key("main"));
	}

	#[test]
	fn traffic_for_a_channel_we_are_not_in_is_ignored() {
		let mut s = joined_session();
		assert!(feed(&mut s, &["CLIENTS other alice"]).is_empty());
		assert!(feed(&mut s, &["JOINED other alice"]).is_empty());
		assert!(s.state.channels.is_empty());
	}

	#[test]
	fn a_message_and_an_emote_are_told_apart() {
		let mut s = joined_session();
		let effects = feed(
			&mut s,
			&["SAID main alice hello", "SAIDEX main alice waves"],
		);
		assert_eq!(
			effects,
			vec![
				Effect::ChannelChat {
					room: "main".into(),
					from: "alice".into(),
					text: "hello".into(),
					emote: false,
				},
				Effect::ChannelChat {
					room: "main".into(),
					from: "alice".into(),
					text: "waves".into(),
					emote: true,
				},
			]
		);
	}

	#[test]
	fn a_private_conversation_is_filed_under_the_other_person_either_way() {
		let mut s = joined_session();
		let effects = feed(
			&mut s,
			&["SAIDPRIVATE bob you there?", "SAYPRIVATE bob on my way"],
		);
		assert_eq!(
			effects,
			vec![
				Effect::PrivateChat {
					with: "bob".into(),
					from: "bob".into(),
					text: "you there?".into(),
				},
				// Our own message comes back from the server; both sides of the
				// conversation file under `bob`.
				Effect::PrivateChat {
					with: "bob".into(),
					from: "me".into(),
					text: "on my way".into(),
				},
			]
		);
	}

	#[test]
	fn slash_me_becomes_an_emote_and_the_cap_is_enforced() {
		let mut s = joined_session();
		assert_eq!(
			sent_lines(&s.say_channel("main", "hello").unwrap()),
			vec!["SAY main hello"]
		);
		assert_eq!(
			sent_lines(&s.say_channel("main", "/me waves").unwrap()),
			vec!["SAYEX main waves"]
		);
		// Past the cap it is wrapped, not refused: the server would cut it.
		assert_eq!(
			sent_lines(
				&s.say_channel("main", &format!("{} y", "x".repeat(257)))
					.unwrap()
			),
			vec![
				format!("SAY main {}", "x".repeat(257)),
				"SAY main y".to_string()
			]
		);
		assert_eq!(
			sent_lines(&s.say_channel("main", "one\ntwo").unwrap()),
			vec!["SAY main one", "SAY main two"]
		);
		assert!(s.join_channel("#main", None).is_err());
	}

	#[test]
	fn the_directory_replaces_itself_rather_than_growing() {
		let mut s = joined_session();
		s.list_channels();
		feed(
			&mut s,
			&["CHANNEL main 412", "CHANNEL bar 30", "ENDOFCHANNELS"],
		);
		assert_eq!(s.state.directory.len(), 2);

		// Asking again starts over, so a shrinking server list shrinks here.
		s.list_channels();
		feed(&mut s, &["CHANNEL main 400", "ENDOFCHANNELS"]);
		assert_eq!(s.state.directory.len(), 1);
		assert_eq!(s.state.directory[0].members, 400);
	}

	#[test]
	fn a_friend_listing_replaces_the_last_one_whole() {
		let mut s = joined_session();
		feed(
			&mut s,
			&[
				"FRIENDLISTBEGIN",
				"FRIENDLIST userName=alice",
				"FRIENDLIST userName=bob",
				"FRIENDLISTEND",
			],
		);
		assert_eq!(
			s.state.friends.iter().cloned().collect::<Vec<_>>(),
			vec!["alice", "bob"]
		);

		// Someone unfriended elsewhere has to disappear here too, which only
		// works because the listing replaces rather than merges.
		feed(
			&mut s,
			&[
				"FRIENDLISTBEGIN",
				"FRIENDLIST userName=alice",
				"FRIENDLISTEND",
			],
		);
		assert_eq!(
			s.state.friends.iter().cloned().collect::<Vec<_>>(),
			vec!["alice"]
		);
	}

	#[test]
	fn a_listing_cut_off_part_way_leaves_the_old_one_standing() {
		let mut s = joined_session();
		feed(
			&mut s,
			&[
				"FRIENDLISTBEGIN",
				"FRIENDLIST userName=alice",
				"FRIENDLISTEND",
			],
		);
		// Begin, one name, and then nothing: without the end marker the
		// half-built set must not be swapped in.
		feed(&mut s, &["FRIENDLISTBEGIN", "FRIENDLIST userName=zoe"]);
		assert_eq!(
			s.state.friends.iter().cloned().collect::<Vec<_>>(),
			vec!["alice"]
		);
	}

	#[test]
	fn requests_are_kept_apart_from_friends() {
		let mut s = joined_session();
		feed(
			&mut s,
			&[
				"FRIENDREQUESTLISTBEGIN",
				"FRIENDREQUESTLIST userName=carol",
				"FRIENDREQUESTLISTEND",
			],
		);
		assert!(s.state.friends.is_empty());
		assert!(s.state.friend_requests.contains("carol"));
	}

	#[test]
	fn acting_on_a_friendship_asks_for_the_listings_again() {
		let mut s = joined_session();
		// The server announces nothing when a friendship changes, so the only
		// way to see the result is to ask.
		assert_eq!(
			sent_lines(&s.friend_action(FriendAction::Accept, "carol")),
			vec![
				"ACCEPTFRIENDREQUEST userName=carol",
				"FRIENDLIST",
				"FRIENDREQUESTLIST",
				"IGNORELIST"
			]
		);
	}

	#[test]
	fn a_kick_drops_the_room_rather_than_leaving_it_on_screen() {
		let mut s = joined_session();
		feed(
			&mut s,
			&[
				"BATTLEOPENED 7 0 0 host 1.2.3.4 8452 16 0 0 -1 Recoil	2026.07.04	Supreme Isthmus v2.1	a room	BAR test",
				"JOINBATTLE 7 hash",
			],
		);
		assert!(s.state.my_battle.is_some());

		// The server has already forgotten us by the time this arrives, so
		// there is nothing to send back — only state to correct.
		let effects = feed(&mut s, &["FORCEQUITBATTLE"]);
		assert!(s.state.my_battle.is_none());
		assert!(sent_lines(&effects).is_empty(), "a kick is not a request");
		assert!(matches!(
			effects.as_slice(),
			[Effect::Notice(_), Effect::LeftBattle { id: 7 }]
		));
	}

	#[test]
	fn a_kick_when_we_are_in_no_room_changes_nothing() {
		let mut s = joined_session();
		assert!(feed(&mut s, &["FORCEQUITBATTLE"]).is_empty());
	}

	/// The `MYBATTLESTATUS` a set of effects carries, decoded.
	#[test]
	fn the_emptiest_cluster_is_favoured_over_the_first_by_name() {
		let mut s = joined_session();
		// Two managers: EU1 already running three rooms, EU2 running none.
		for name in [
			"Host[EU1]",
			"Host[EU1][001]",
			"Host[EU1][002]",
			"Host[EU1][003]",
			"Host[EU2]",
		] {
			feed(&mut s, &[&format!("ADDUSER {name} EU 0 modlobby")]);
			feed(&mut s, &[&format!("CLIENTSTATUS {name} 64")]);
		}

		// Sorting by name alone would always answer EU1; weighting by how
		// loaded each one is puts most of the draw on EU2.
		let unmeasured = Rtts::new();
		assert_eq!(s.pick_cluster_manager(&unmeasured, 0.99), Some("Host[EU2]"));
		assert_eq!(s.pick_cluster_manager(&unmeasured, 0.0), Some("Host[EU1]"));
	}

	#[test]
	fn a_far_cluster_is_not_asked_for_a_private_room() {
		let mut s = joined_session();
		feed(
			&mut s,
			&[
				"ADDUSER Host[EU1] DE 9 SPADS",
				"ADDUSER Host[US1] US 10 SPADS",
				"CLIENTSTATUS Host[EU1] 64",
				"CLIENTSTATUS Host[US1] 64",
				// The rooms are what carry each cluster's address.
				"BATTLEOPENED 1 0 0 Host[EU1][001] 10.0.0.1 8452 16 0 0 -1 R\tv\tm\tt\tg",
				"BATTLEOPENED 2 0 0 Host[US1][001] 10.0.0.2 8452 16 0 0 -1 R\tv\tm\tt\tg",
			],
		);
		let rtts: Rtts = [
			("10.0.0.1".parse().unwrap(), Duration::from_millis(140)),
			("10.0.0.2".parse().unwrap(), Duration::from_millis(25)),
		]
		.into();
		// Emptiness alone would split the draw; distance settles it.
		for roll in [0.0, 0.5, 0.99] {
			assert_eq!(s.pick_cluster_manager(&rtts, roll), Some("Host[US1]"));
		}
	}

	#[test]
	fn with_no_managers_there_is_nothing_to_pick() {
		let s = joined_session();
		assert_eq!(s.pick_cluster_manager(&Rtts::new(), 0.5), None);
	}

	#[test]
	fn a_spare_autohost_is_one_with_nobody_but_its_host_in_it() {
		let mut s = joined_session();
		feed(
			&mut s,
			&[
				"ADDUSER Host[EU1][006] DE 9 SPADS",
				"CLIENTSTATUS Host[EU1][006] 65",
				// Busy: someone is already in it.
				"BATTLEOPENED 1 0 0 Host[EU1][001] 1.2.3.4 8452 16 0 0 -1 R\tv\tm\tt\tg",
				"JOINEDBATTLE 1 alice",
				// Passworded: not a public room.
				"BATTLEOPENED 2 0 0 Host[EU1][002] 1.2.3.4 8452 16 1 0 -1 R\tv\tm\tt\tg",
				// Empty and open.
				"BATTLEOPENED 3 0 0 Host[EU1][003] 1.2.3.4 8452 16 0 0 -1 R\tv\tm\tt\tg",
				// Somebody parked in it as a spectator: nobody plays, but it
				// is not empty, and bossing it would need their vote.
				"BATTLEOPENED 4 0 0 Host[EU1][004] 1.2.3.4 8452 16 0 0 -1 R\tv\tm\tt\tg",
				"JOINEDBATTLE 4 bob",
				"UPDATEBATTLEINFO 4 2 0 -1 m",
				// Another engine than the rest: a testing host.
				"BATTLEOPENED 5 0 0 Host[EU1][005] 1.2.3.4 8452 16 0 0 -1 R\tv2\tm\tt\tg",
				// Its host is in a game, which after a server restart is what
				// an empty room whose game is still running looks like.
				"BATTLEOPENED 6 0 0 Host[EU1][006] 1.2.3.4 8452 16 0 0 -1 R\tv\tm\tt\tg",
				// Not a cluster's room at all.
				"BATTLEOPENED 7 0 0 [teh]host 1.2.3.4 8452 16 0 0 -1 R\tv\tm\tt\tg",
				// Another region is as good as any.
				"BATTLEOPENED 8 0 0 Host[US1][001] 5.6.7.8 8452 16 0 0 -1 R\tv\tm\tt\tg",
				// A second spare on the first machine.
				"BATTLEOPENED 9 0 0 Host[EU1][009] 1.2.3.4 8452 16 0 0 -1 R\tv\tm\tt\tg",
			],
		);

		let spares = s.spare_rooms();
		assert_eq!(
			spares.iter().map(|room| room.id).collect::<Vec<_>>(),
			[3, 8, 9]
		);
		assert_eq!(spares[0].cluster, "Host[EU1]");
		assert_eq!(spares[1].ip, "5.6.7.8".parse().ok());
		// Each machine once, however many spares it runs.
		assert_eq!(
			s.spare_machines(),
			[Ipv4Addr::new(1, 2, 3, 4), Ipv4Addr::new(5, 6, 7, 8)]
		);
	}

	#[test]
	fn a_spare_room_is_claimed_on_arrival() {
		let mut s = joined_session();
		feed(
			&mut s,
			&["BATTLEOPENED 3 0 0 Host[EU1][003] 1.2.3.4 8452 16 0 0 -1 R\tv\tm\tt\tg"],
		);
		let asked = s.host_public(3, "pw".into());
		assert_eq!(sent_lines(&asked), ["JOINBATTLE 3 empty pw"]);

		let arrived = feed(&mut s, &["JOINBATTLE 3 h", "JOINEDBATTLE 3 me"]);
		assert!(arrived.contains(&Effect::Hosting {
			founder: "Host[EU1][003]".into(),
			alone: true
		}));
		// A seat before the commands: SPADS takes `!boss` from a player only.
		let lines = sent_lines(&arrived);
		assert!(lines[0].starts_with("MYBATTLESTATUS "), "{lines:?}");
		assert_eq!(
			&lines[1..],
			["SAYBATTLE !boss me", "SAYBATTLE !preset custom"]
		);
		assert!(s.seat().is_some());
		// Claimed once; the next join is an ordinary one.
		s.leave_battle();
		s.join_battle(3, None, "pw".into());
		assert!(sent_lines(&feed(&mut s, &["JOINBATTLE 3 h"])).is_empty());
	}

	#[test]
	fn a_room_someone_reached_first_is_not_bossed() {
		let mut s = joined_session();
		feed(
			&mut s,
			&["BATTLEOPENED 3 0 0 Host[EU1][003] 1.2.3.4 8452 16 0 0 -1 R\tv\tm\tt\tg"],
		);
		s.host_public(3, "pw".into());
		// They got in between our look at the list and the host's answer.
		let arrived = feed(&mut s, &["JOINEDBATTLE 3 bob", "JOINBATTLE 3 h"]);
		assert!(arrived.contains(&Effect::Hosting {
			founder: "Host[EU1][003]".into(),
			alone: false
		}));
		assert!(sent_lines(&arrived).is_empty(), "{arrived:?}");
	}

	fn sent_status(effects: &[Effect]) -> spring_protocol::BattleStatus {
		let sends: Vec<_> = effects
			.iter()
			.filter_map(|effect| match effect {
				Effect::Send(env) => Some(env),
				_ => None,
			})
			.collect();
		let [env] = sends[..] else {
			panic!("expected one status, got {effects:?}")
		};
		let rest = env
			.line
			.strip_prefix("MYBATTLESTATUS ")
			.expect("a status line");
		let bits: u32 = rest.split(' ').next().unwrap().parse().unwrap();
		spring_protocol::BattleStatus::from_bits(bits)
	}

	/// A session sitting in a public room, ready to try for a seat. Listed as
	/// a user, as the server lists everyone, ourselves included: our own
	/// entry is where the server's word on our seat is read from.
	fn in_a_public_room() -> Session {
		let mut s = joined_session();
		feed(
			&mut s,
			&[
				"ADDUSER me EU 1 modlobby",
				"ADDUSER host EU 0 SPADS",
				"BATTLEOPENED 3 0 0 host 1.2.3.4 8452 16 0 0 -1 R	v	m	t	g",
				"JOINBATTLE 3 hash",
			],
		);
		s
	}

	#[test]
	fn joining_a_second_room_leaves_the_first() {
		let mut s = in_a_public_room();
		feed(
			&mut s,
			&["BATTLEOPENED 4 0 0 other 1.2.3.4 8452 16 0 0 -1 R	v	m	t	g"],
		);

		// Without the LEAVEBATTLE the server ignores the join outright, and
		// the app sits in the old room believing it moved.
		let effects = s.join_battle(4, None, "pw".into());
		let lines = sent_lines(&effects);
		assert_eq!(lines.len(), 2, "leave then join, got {lines:?}");
		assert_eq!(lines[0], "LEAVEBATTLE");
		assert!(lines[1].starts_with("JOINBATTLE 4"));
	}

	#[test]
	fn joining_the_room_we_are_already_in_does_nothing() {
		let mut s = in_a_public_room();
		assert!(s.join_battle(3, None, "pw".into()).is_empty());
	}

	#[test]
	fn a_seat_may_be_taken_while_the_current_game_runs() {
		let mut s = in_a_public_room();

		// Sitting down during a game is how you join the *next* one: SPADS has
		// the lineup ready when this one ends. Refusing it would make the
		// commonest thing anyone does in a busy room impossible.
		feed(&mut s, &["CLIENTSTATUS host 1"]);
		assert!(s.take_seat(0, 0, false).is_ok());
	}

	#[test]
	fn a_seat_in_a_public_room_is_simply_taken() {
		let mut s = in_a_public_room();

		// There was a licence for this once -- a setting, and a session that
		// started watching-only -- from when this client was first pointed at
		// a live server. A lobby you cannot sit down in is not a lobby.
		assert!(s.take_seat(0, 0, false).is_ok());
		assert_eq!(
			s.seat().map(|seat| (seat.team, seat.ally_team)),
			Some((0, 0))
		);
	}

	#[test]
	fn ready_and_faction_ride_on_the_battle_status() {
		let mut s = in_a_public_room();
		s.take_seat(3, 1, false).unwrap();
		feed(&mut s, &[&told(MyBattleStatus::player(Sync::Synced, 3, 1))]);

		let ready = sent_status(&s.set_ready(true).unwrap());
		assert!(ready.ready);
		assert_eq!((ready.team, ready.ally_team), (3, 1));

		let side = sent_status(&s.set_side(2).unwrap());
		assert_eq!(side.side, 2);
		// Setting ready did not lose the faction, nor the other way round.
		assert!(side.ready);
	}

	#[test]
	fn a_setting_the_room_already_has_is_not_sent_again() {
		let mut s = in_a_public_room();
		feed(
			&mut s,
			&["SETSCRIPTTAGS game/modoptions/startmetal=1000	game/modoptions/tweakdefs1=QUJD"],
		);
		let effects = s
            .say_battle(
                "!bSet startmetal 1000\n!bset StartMetal 2000\n!bSet tweakdefs1 QUJD\n!startmetal 1000\n!preset custom\nhi",
                PasteBurst::Never,
            )
            .unwrap();
		assert_eq!(
			sent_lines(&effects),
			[
				"SAYBATTLE !bset StartMetal 2000",
				"SAYBATTLE !preset custom",
				"SAYBATTLE hi"
			]
		);
		assert_eq!(
			effects.last(),
			Some(&Effect::PasteQueued {
				lines: 3,
				skipped: 3
			})
		);
		// Nothing to compare against outside a room, and nothing to skip.
		let mut out = joined_session();
		assert_eq!(
			sent_lines(
				&out.say_battle("!bSet startmetal 1000", PasteBurst::Boss)
					.unwrap()
			),
			["SAYBATTLE !bSet startmetal 1000"]
		);
	}

	#[test]
	fn a_preset_line_wipes_the_room_so_what_follows_is_not_skipped() {
		let mut s = in_a_public_room();
		feed(&mut s, &["SETSCRIPTTAGS game/modoptions/startmetal=1000"]);
		let effects = s
			.say_battle(
				"!bSet startmetal 1000
!preset custom
!bSet startmetal 1000
!startmetal 1000",
				PasteBurst::Never,
			)
			.unwrap();
		assert_eq!(
			sent_lines(&effects),
			[
				"SAYBATTLE !preset custom",
				"SAYBATTLE !bSet startmetal 1000",
				"SAYBATTLE !startmetal 1000",
			]
		);
		assert_eq!(
			effects.last(),
			Some(&Effect::PasteQueued {
				lines: 3,
				skipped: 1
			})
		);
	}

	#[test]
	fn a_paste_takes_the_burst_lane_only_as_boss_and_only_when_it_is_a_paste() {
		let areas = |effects: &[Effect]| -> Vec<Area> {
			effects
				.iter()
				.filter_map(|e| match e {
					Effect::Send(env) => Some(env.area),
					_ => None,
				})
				.collect()
		};
		let mut s = in_a_public_room();
		let paste = "!preset custom\n!bSet a 1\nthanks";
		assert_eq!(
			areas(&s.say_battle(paste, PasteBurst::Boss).unwrap()),
			[Area::BattleCommand, Area::BattleCommand, Area::BattleChat],
			"not boss: paced"
		);
		feed(
			&mut s,
			&[r#"SAIDBATTLEEX host * BarManager|{"BattleStateChanged": {"boss": "alice,me"}}"#],
		);
		assert!(s.is_boss(), "the second of two bosses is still one");
		assert_eq!(
			areas(&s.say_battle(paste, PasteBurst::Boss).unwrap()),
			[Area::BattlePaste; 3]
		);
		assert_eq!(
			areas(&s.say_battle(paste, PasteBurst::Never).unwrap()),
			[Area::BattleCommand, Area::BattleCommand, Area::BattleChat]
		);
		// One line is not a paste, whoever sends it.
		assert_eq!(
			areas(&s.say_battle("!vote y", PasteBurst::Always).unwrap()),
			[Area::BattleCommand]
		);
		let mut nobody = in_a_public_room();
		assert_eq!(
			areas(&nobody.say_battle(paste, PasteBurst::Always).unwrap()),
			[Area::BattlePaste; 3]
		);
	}

	#[test]
	fn saying_the_same_thing_twice_sends_nothing() {
		let mut s = in_a_public_room();
		s.take_seat(0, 0, false).unwrap();
		s.set_ready(true).unwrap();
		assert!(
			s.set_ready(true).unwrap().is_empty(),
			"the room already knows"
		);
		assert!(s.set_side(0).unwrap().is_empty());
	}

	#[test]
	fn a_spectator_is_neither_ready_nor_a_faction() {
		let mut s = in_a_public_room();
		assert!(matches!(s.set_ready(true), Err(SeatError::Spectating)));
		assert!(matches!(s.set_side(1), Err(SeatError::Spectating)));
	}

	#[test]
	fn changing_side_keeps_ready_and_faction_but_sitting_down_does_not() {
		let mut s = in_a_public_room();
		s.take_seat(0, 0, false).unwrap();
		feed(&mut s, &[&told(seated(0, false))]);
		s.set_side(3).unwrap();
		s.set_ready(true).unwrap();

		// Moving to another team keeps both: the game agreed to is the same
		// one, and the faction is a preference.
		let moved = sent_status(&s.take_seat(1, 1, false).unwrap());
		assert!(moved.ready);
		assert_eq!(moved.side, 3);

		// Standing up and sitting down again is a new seat, not a game agreed to.
		s.release_seat();
		let sat = sent_status(&s.take_seat(0, 0, false).unwrap());
		assert!(!sat.ready);
	}

	fn sent_lines(effects: &[Effect]) -> Vec<&str> {
		effects
			.iter()
			.filter_map(|e| match e {
				Effect::Send(env) => Some(env.line.as_str()),
				_ => None,
			})
			.collect()
	}

	/// Logged in, with one SPADS-hosted room (id 5, host `host`) on the list.
	fn ready_with_room() -> Session {
		let mut s = session();
		feed(
			&mut s,
			&[
				"TASSERVER 0.38 * 8201 0",
				"ACCEPTED me",
				"ADDUSER me SE 1 LuaLobby Chobby",
				"ADDUSER host GB 2 SPADS",
				"BATTLEOPENED 5 0 0 host 1.2.3.4 8452 16 0 0 h R\tv\tm\tt\tg",
				"LOGININFOEND",
			],
		);
		s
	}

	#[test]
	fn greeting_triggers_login() {
		let mut s = session();
		let effects = feed(&mut s, &["TASSERVER 0.38 * 8201 0"]);
		assert!(
			matches!(&effects[..], [Effect::Send(env)] if env.area == Area::Login && env.line.starts_with("LOGIN me "))
		);
		assert_eq!(s.state.phase, Some(Phase::AwaitingLogin));
	}

	#[test]
	fn an_uberserver_is_sent_no_telemetry_it_would_call_unknown() {
		let mut s = session();
		let effects = feed(
			&mut s,
			&["TASSERVER unknown * 8201 0", "ACCEPTED me", "LOGININFOEND"],
		);
		assert!(
			!sent_lines(&effects)
				.iter()
				.any(|line| line.starts_with("c.telemetry.")),
			"{effects:?}"
		);
		assert_eq!(effects.last(), Some(&Effect::Ready));
	}

	#[test]
	fn login_flood_builds_state_and_ready_uploads_telemetry() {
		let mut s = session();
		let effects = feed(
			&mut s,
			&[
				"TASSERVER 0.38-33-ga5f3b28 * 8201 0",
				"ACCEPTED me",
				"MOTD hi",
				"ADDUSER me SE 1 LuaLobby Chobby",
				"ADDUSER bot GB 2 SPADS",
				"ADDUSER alice DE 3 LuaLobby Chobby",
				"BATTLEOPENED 5 0 0 bot 1.2.3.4 8452 16 0 0 h Recoil\t2026.07.04\tMap One\tTitle\tBeyond All Reason test-1",
				"UPDATEBATTLEINFO 5 1 0 h Map One",
				"JOINEDBATTLE 5 alice",
				"CLIENTSTATUS bot 64",
				"LOGININFOEND",
			],
		);
		assert_eq!(s.state.phase, Some(Phase::Ready));
		assert_eq!(s.state.users.len(), 3);
		assert!(s.state.users["bot"].status.bot);
		let battle = &s.state.battles[&5];
		assert_eq!(battle.members.len(), 2);
		assert_eq!(battle.spectator_count, 1);
		assert_eq!(battle.player_count(), 1);
		assert_eq!(s.state.user_battle["alice"], 5);
		assert_eq!(s.state.user_battle["bot"], 5);
		assert!(sent_lines(&effects).iter().any(|line| {
			line.starts_with("c.telemetry.update_client_property hardware:cpuinfo ")
		}));
		assert!(effects.contains(&Effect::Motd {
			text: "hi".to_owned()
		}));
		assert_eq!(effects.last(), Some(&Effect::Ready));
	}

	#[test]
	fn leaving_and_closing_clean_up_membership() {
		let mut s = session();
		feed(
			&mut s,
			&[
				"BATTLEOPENED 5 0 0 bot 1.2.3.4 8452 16 0 0 h R\tv\tm\tt\tg",
				"ADDUSER alice DE 3 x",
				"JOINEDBATTLE 5 alice",
				"LEFTBATTLE 5 alice",
				"JOINEDBATTLE 5 alice",
				"BATTLECLOSED 5",
			],
		);
		assert!(s.state.battles.is_empty());
		assert!(s.state.user_battle.is_empty());
	}

	#[test]
	fn founder_counts_without_a_joinedbattle_line() {
		// Captured 2026-08-29: a full 8v8 with three human spectators. The host
		// never gets a JOINEDBATTLE but is counted in UPDATEBATTLEINFO's spectators.
		let mut s = session();
		let mut lines = vec![
            "BATTLEOPENED 57 0 0 Host[US4][000] 144.126.147.151 53200 16 0 0 -590370561 spring\t2026.07.04\tSupreme Isthmus v2.1\tSuPrEmE MuFF | 8v8\tBeyond All Reason test-31115-21dbf79".to_owned(),
            "UPDATEBATTLEINFO 57 4 0 -590370561 Supreme Isthmus v2.1".to_owned(),
        ];
		lines.extend((0..19).map(|i| format!("JOINEDBATTLE 57 player{i}")));
		for line in &lines {
			s.handle(RawMessage::parse(line).into());
		}
		let battle = &s.state.battles[&57];
		assert_eq!(battle.members.len(), 20);
		assert_eq!(battle.player_count(), 16);
		assert_eq!(s.state.user_battle["Host[US4][000]"], 57);
	}

	#[test]
	fn battle_title_and_teams_update_the_room() {
		let mut s = session();
		feed(
			&mut s,
			&[
				"BATTLEOPENED 22 0 0 bot 1.2.3.4 8452 16 0 0 h R\tv\tm\told title\tg",
				"s.battle.update_lobby_title 22\tBeginner Players | 4v4",
				"s.battle.teams eyIyMiI6eyJuYlRlYW1zIjoyLCJ0ZWFtU2l6ZSI6OH19",
			],
		);
		let battle = &s.state.battles[&22];
		assert_eq!(battle.title, "Beginner Players | 4v4");
		assert_eq!(battle.layout.map(|l| (l.teams, l.team_size)), Some((2, 8)));
	}

	#[test]
	fn the_join_queue_is_replaced_whole_and_forgotten_on_leaving() {
		let mut s = session();
		feed(
			&mut s,
			&[
				"BATTLEOPENED 22 0 0 bot 1.2.3.4 8452 16 0 0 h R\tv\tm\ttitle\tg",
				"s.battle.queue_status 22\tAlice\tBob",
			],
		);
		assert_eq!(s.state.battles[&22].queue, ["Alice", "Bob"]);
		feed(&mut s, &["s.battle.queue_status 22\tBob"]);
		assert_eq!(s.state.battles[&22].queue, ["Bob"]);
		feed(&mut s, &["s.battle.queue_status 22"]);
		assert!(s.state.battles[&22].queue.is_empty());

		feed(&mut s, &["s.battle.queue_status 22\tAlice"]);
		s.state.forget_room_details(22);
		assert!(s.state.battles[&22].queue.is_empty());
	}

	#[test]
	fn join_flow_answers_the_status_request_as_a_spectator() {
		let mut s = ready_with_room();
		let effects = s.join_battle(5, None, "4242".into());
		assert_eq!(sent_lines(&effects), ["JOINBATTLE 5 empty 4242"]);

		// What teiserver sends once SPADS accepts (spring_out.ex do_join_battle).
		let effects = feed(
			&mut s,
			&[
				"JOINBATTLE 5 -1",
				"JOINEDBATTLE 5 me 4242",
				"REQUESTBATTLESTATUS",
			],
		);
		assert!(effects.contains(&Effect::Joined { id: 5 }));
		let [status_line] = sent_lines(&effects)[..] else {
			panic!("expected exactly one MYBATTLESTATUS, got {effects:?}")
		};
		let bits: u32 = status_line
			.strip_prefix("MYBATTLESTATUS ")
			.and_then(|rest| rest.split(' ').next())
			.and_then(|bits| bits.parse().ok())
			.expect("MYBATTLESTATUS <bits> <colour>");
		let status = BattleStatus::from_bits(bits);
		assert!(!status.player, "must never take a player slot");
		assert!(!status.ready);
		assert_eq!(
			s.state
				.my_battle
				.as_ref()
				.map(|m| (m.id, m.script_password.as_str())),
			Some((5, "4242"))
		);
		assert_eq!(s.state.user_battle["me"], 5);
	}

	#[test]
	fn the_host_leaving_its_game_is_what_says_the_game_ended() {
		let mut s = ready_with_room();
		s.join_battle(5, None, "4242".into());
		feed(&mut s, &["JOINBATTLE 5 -1", "CLIENTSTATUS host 65"]);

		// The same bit going back down. Nothing else on the wire says a game
		// finished, and a room that never hears it goes on offering to connect
		// you to one that is over.
		let effects = feed(&mut s, &["CLIENTSTATUS host 64"]);
		assert!(effects.contains(&Effect::GameStopped));

		// Said once: a status line that changes something else is not news.
		assert!(!feed(&mut s, &["CLIENTSTATUS host 64"]).contains(&Effect::GameStopped));
	}

	#[test]
	fn a_joinas_asks_the_host_who_shares_the_id() {
		let playing_with = |effects: &[Effect]| {
			effects.iter().find_map(|effect| match effect {
				Effect::PlayingWith { names } => Some(names.clone()),
				_ => None,
			})
		};
		let answer = r#"SAIDPRIVATE host !#JSONRPC {"jsonrpc":"2.0","result":{"game":{"clients":[{"Name":"alice","Id":3},{"Name":"+ me","Id":3},{"Name":"eve","Id":0}]}},"id":1}"#;
		let mut s = ready_with_room();
		s.join_battle(5, None, "4242".into());
		feed(
			&mut s,
			&[
				"JOINBATTLE 5 -1",
				"CLIENTSTATUS host 64",
				"CLIENTSTATUS host 65",
			],
		);

		// Only the host is believed, and only about us.
		assert_eq!(
			playing_with(&feed(
				&mut s,
				&[
					"SAIDBATTLEEX alice * Adding player me in ID 3",
					"SAIDBATTLEEX host * Adding player bob in ID 3",
					answer,
				]
			)),
			None
		);

		let effects = feed(&mut s, &["SAIDBATTLEEX host * Adding player me in ID 3"]);
		assert_eq!(playing_with(&effects), Some(vec![]));
		assert_eq!(
			sent_lines(&effects),
			[format!("SAYPRIVATE host {}", spads::GAME_STATUS_REQUEST)]
		);
		assert_eq!(
			playing_with(&feed(&mut s, &[answer])),
			Some(vec!["alice".into()])
		);

		// The next game starts with nobody having put us in it.
		feed(&mut s, &["CLIENTSTATUS host 64", "CLIENTSTATUS host 65"]);
		assert_eq!(playing_with(&feed(&mut s, &[answer])), None);
	}

	#[test]
	fn walking_into_a_running_game_is_told_apart_from_one_starting() {
		// The distinction the whole auto-launch behaviour rests on. Both are
		// the same running game and the same connection details; only one is
		// a reason to start an engine for somebody.
		let started = |effects: &[Effect]| {
			effects.iter().find_map(|effect| match effect {
				Effect::GameRunning { just_started, .. } => Some(*just_started),
				_ => None,
			})
		};

		// Joining a room whose host is already in game.
		let mut s = ready_with_room();
		feed(&mut s, &["CLIENTSTATUS host 65"]);
		s.join_battle(5, None, "4242".into());
		assert_eq!(started(&feed(&mut s, &["JOINBATTLE 5 -1"])), Some(false));

		// The same room, the game starting while we stand in it.
		let mut s = ready_with_room();
		s.join_battle(5, None, "4242".into());
		feed(&mut s, &["JOINBATTLE 5 -1", "CLIENTSTATUS host 64"]);
		assert_eq!(
			started(&feed(&mut s, &["CLIENTSTATUS host 65"])),
			Some(true)
		);
	}

	#[test]
	fn host_in_game_reports_game_running_on_join_and_on_change() {
		// Already running when we join: bot (64) + in-game (1).
		let mut s = ready_with_room();
		feed(&mut s, &["CLIENTSTATUS host 65"]);
		s.join_battle(5, None, "4242".into());
		let effects = feed(&mut s, &["JOINBATTLE 5 -1"]);
		// Reported, but marked as something that was already under way: it is
		// an invitation to watch, not a game starting around us, and nothing
		// should be launched on somebody's behalf for it.
		assert!(effects.contains(&Effect::GameRunning {
			id: 5,
			ip: "1.2.3.4".into(),
			port: 8452,
			script_password: "4242".into(),
			just_started: false,
		}));

		// Starts after we joined; repeated in-game statuses do not repeat the effect.
		let mut s = ready_with_room();
		s.join_battle(5, None, "4242".into());
		let effects = feed(
			&mut s,
			&[
				"JOINBATTLE 5 -1",
				"CLIENTSTATUS host 64",
				"CLIENTSTATUS host 65",
				"CLIENTSTATUS host 65",
			],
		);
		let running = effects
			.iter()
			.filter(|e| matches!(e, Effect::GameRunning { .. }))
			.count();
		assert_eq!(running, 1);
	}

	#[test]
	fn join_failure_and_leaving_clear_the_room() {
		let mut s = ready_with_room();
		s.join_battle(5, None, "1".into());
		assert_eq!(
			feed(&mut s, &["JOINBATTLEFAILED Battle locked"]),
			vec![Effect::JoinFailed {
				reason: "Battle locked".into()
			}]
		);
		assert!(s.state.my_battle.is_none());

		s.join_battle(5, None, "1".into());
		feed(&mut s, &["JOINBATTLE 5 -1", "JOINEDBATTLE 5 me 1"]);
		assert_eq!(
			feed(&mut s, &["LEFTBATTLE 5 me"]),
			vec![Effect::LeftBattle { id: 5 }]
		);
		assert!(s.state.my_battle.is_none());

		s.join_battle(5, None, "1".into());
		feed(&mut s, &["JOINBATTLE 5 -1"]);
		let effects = s.leave_battle();
		assert_eq!(sent_lines(&effects), ["LEAVEBATTLE"]);
		assert!(effects.contains(&Effect::LeftBattle { id: 5 }));
		assert!(s.leave_battle().is_empty());
	}

	#[test]
	fn in_game_status_is_a_mystatus_line() {
		let mut s = ready_with_room();
		assert_eq!(sent_lines(&s.set_in_game(true)), ["MYSTATUS 1"]);
		assert_eq!(sent_lines(&s.set_in_game(false)), ["MYSTATUS 0"]);
	}

	#[test]
	fn away_and_in_game_are_sent_together_because_the_command_carries_both() {
		let mut s = session();
		assert_eq!(sent_lines(&s.set_away(true)), ["MYSTATUS 2"]);
		// Going into a game must not quietly say we came back.
		assert_eq!(sent_lines(&s.set_in_game(true)), ["MYSTATUS 3"]);
		assert_eq!(sent_lines(&s.set_away(false)), ["MYSTATUS 1"]);
	}

	#[test]
	fn battle_chat_becomes_effects() {
		let mut s = ready_with_room();
		assert_eq!(
			feed(
				&mut s,
				&[
					"SAIDBATTLE host hello there",
					"SAIDBATTLEEX host * welcomes me"
				]
			),
			vec![
				Effect::BattleChat {
					from: "host".into(),
					text: "hello there".into(),
					announcement: false
				},
				Effect::BattleChat {
					from: "host".into(),
					text: "* welcomes me".into(),
					announcement: true
				},
			]
		);
	}

	#[test]
	fn room_details_follow_script_tags_bots_and_start_boxes() {
		let mut s = ready_with_room();
		s.join_battle(5, None, "1".into());
		feed(
			&mut s,
			&[
				"JOINBATTLE 5 -1",
				"JOINEDBATTLE 5 me 1",
				"SETSCRIPTTAGS game/modoptions/tweakdefs=abc\tgame/hosttype=SPADS",
				"ADDBOT 5 RaptorsAI host 4195330 16777215 BARb",
				"ADDSTARTRECT 1 0 0 200 40",
			],
		);
		let my = s.state.my_battle.as_ref().unwrap();
		assert_eq!(my.modoptions().collect::<Vec<_>>(), [("tweakdefs", "abc")]);
		assert_eq!(my.script_tags["game/hosttype"], "SPADS");
		let room = &s.state.battles[&5];
		assert_eq!(room.bots["RaptorsAI"].ai, "BARb");
		assert!(room.bots["RaptorsAI"].status.player);
		assert_eq!(room.start_rects[&1].bottom, 40);

		feed(
			&mut s,
			&[
				"SETSCRIPTTAGS game/modoptions/tweakdefs=def",
				"UPDATEBOT 5 RaptorsAI 0 16777215",
				"REMOVESTARTRECT 1",
			],
		);
		assert_eq!(
			s.state.my_battle.as_ref().unwrap().script_tags["game/modoptions/tweakdefs"],
			"def"
		);
		assert!(!s.state.battles[&5].bots["RaptorsAI"].status.player);
		assert!(s.state.battles[&5].start_rects.is_empty());

		feed(&mut s, &["REMOVESCRIPTTAGS game/modoptions/tweakdefs"]);
		assert_eq!(s.state.my_battle.as_ref().unwrap().modoptions().count(), 0);

		// Leaving drops what the server stops telling us about.
		feed(&mut s, &["LEFTBATTLE 5 me"]);
		assert!(s.state.battles[&5].bots.is_empty());
	}

	/// A tweak vote as the room actually delivers it: the host announces, the
	/// tally moves, it passes, and only then does the value land.
	#[test]
	fn a_tweak_vote_is_followed_from_call_to_setting() {
		let mut s = ready_with_room();
		s.join_battle(5, None, "1".into());
		feed(
			&mut s,
			&[
				"JOINBATTLE 5 -1",
				"JOINEDBATTLE 5 me 1",
				"REQUESTBATTLESTATUS",
			],
		);

		let effects = feed(
			&mut s,
			&[
				"SAIDBATTLEEX host * Bob called a vote for command \"bSet tweakdefs1 QUJD\" [!vote y, !vote n, !vote b]",
			],
		);
		assert!(effects.contains(&Effect::VoteChanged));
		let vote = s.state.my_battle.as_ref().unwrap().vote.clone().unwrap();
		assert_eq!(vote.by.as_deref(), Some("Bob"));
		assert_eq!(
			vote.proposal,
			crate::Proposal::SetOption {
				key: "tweakdefs1".into(),
				value: "QUJD".into()
			}
		);

		feed(
			&mut s,
			&[
				"SAIDBATTLEEX host * Vote in progress: \"bSet tweakdefs1 QUJD\" [y:2/3, n:1/4(5)] (17s remaining)",
			],
		);
		let vote = s.state.my_battle.as_ref().unwrap().vote.clone().unwrap();
		assert_eq!((vote.yes, vote.yes_needed, vote.no), (2, 3, 1));
		assert_eq!(vote.remaining_secs, 17);

		// Someone else's chat must not move the room's state.
		feed(&mut s, &["SAIDBATTLEEX alice * Vote cancelled by alice"]);
		assert!(s.state.my_battle.as_ref().unwrap().vote.is_some());

		let effects = feed(
			&mut s,
			&[
				"SAIDBATTLEEX host * Vote for command \"bSet tweakdefs1 QUJD\" passed.",
				"SETSCRIPTTAGS game/modoptions/tweakdefs1=QUJD",
				"SAIDBATTLEEX host * Battle setting changed by Bob (tweakdefs1=QUJD)",
			],
		);
		assert!(s.state.my_battle.as_ref().unwrap().vote.is_none());
		assert!(effects.contains(&Effect::ModOptionsChanged {
			keys: vec!["tweakdefs1".into()]
		}));

		let my = s.state.my_battle.as_ref().unwrap();
		assert_eq!(my.modoption("tweakdefs1"), "QUJD");
		// One change, attributed once the host said who did it.
		assert_eq!(my.history.len(), 1);
		assert_eq!(my.history[0].by.as_deref(), Some("Bob"));
		assert_eq!(
			(my.history[0].from.as_str(), my.history[0].to.as_str()),
			("", "QUJD")
		);
		// Stamped when seen, which is what the editor says "5m ago" from.
		assert!(my.history[0].at > 0);
	}

	/// Clearing a slot never reaches SETSCRIPTTAGS (`spads.pl:2625-2628`), so
	/// the announcement has to carry it.
	#[test]
	fn a_cleared_slot_is_recorded_from_the_announcement_alone() {
		let mut s = ready_with_room();
		s.join_battle(5, None, "1".into());
		feed(
			&mut s,
			&[
				"JOINBATTLE 5 -1",
				"SETSCRIPTTAGS game/modoptions/tweakdefs1=QUJD",
				"REQUESTBATTLESTATUS",
				"SAIDBATTLEEX host * Battle setting changed by Bob (tweakdefs1=)",
			],
		);
		let my = s.state.my_battle.as_ref().unwrap();
		assert_eq!(my.modoption("tweakdefs1"), "");
		assert_eq!(my.history.len(), 1);
		assert_eq!(my.history[0].from, "QUJD");
		assert!(my.history[0].to.is_empty());
	}

	/// teiserver replays the room's whole modoption map after `JOINBATTLE`
	/// (`spring_out.ex` `do_join_battle`). That is the room as found, not a
	/// change, and the history starts at the `REQUESTBATTLESTATUS` that ends
	/// the replay. The values still land, so the frontend sees them.
	#[test]
	fn the_join_replay_is_not_history() {
		let mut s = ready_with_room();
		s.join_battle(5, None, "1".into());
		let effects = feed(
			&mut s,
			&[
				"JOINBATTLE 5 -1",
				"SETSCRIPTTAGS game/modoptions/startmetal=1000\tgame/modoptions/mapmetadata_startbox_override=QUJD",
			],
		);
		assert!(effects.contains(&Effect::ModOptionsChanged {
			keys: vec!["startmetal".into(), "mapmetadata_startbox_override".into()]
		}));
		let my = s.state.my_battle.as_ref().unwrap();
		assert_eq!(my.modoption("startmetal"), "1000");
		assert!(my.history.is_empty(), "the replay is not a change");

		feed(&mut s, &["REQUESTBATTLESTATUS"]);
		feed(
			&mut s,
			&["SETSCRIPTTAGS game/modoptions/mapmetadata_startbox_override=REVG"],
		);
		let my = s.state.my_battle.as_ref().unwrap();
		assert_eq!(my.history.len(), 1);
		assert_eq!(my.history[0].seq, 1);
		assert_eq!(
			(my.history[0].from.as_str(), my.history[0].to.as_str()),
			("QUJD", "REVG")
		);
	}

	/// The safety property this project runs on: a seat is never taken outside
	/// a room, and never without being asked for.
	#[test]
	fn a_seat_is_refused_outside_a_room_and_never_taken_unasked() {
		let mut s = ready_with_room();
		assert_eq!(s.take_seat(0, 0, false), Err(SeatError::NotInARoom));

		s.join_battle(5, None, "1".into());
		feed(&mut s, &["JOINBATTLE 5 -1", "JOINEDBATTLE 5 me 1"]);
		// Joining is not sitting down: arriving in a room leaves us watching
		// until something asks for a seat.
		assert_eq!(s.seat(), None);

		// What the room answers while we are a spectator.
		let status = |effects: &[Effect]| sent_status(effects);
		assert!(!status(&feed(&mut s, &["REQUESTBATTLESTATUS"])).player);

		// A passworded room is one a cluster manager gave us.
		feed(
			&mut s,
			&["BATTLEOPENED 9 0 0 host2 1.2.3.4 8452 16 1 0 h R\tv\tm\tt\tg"],
		);
		s.leave_battle();
		s.join_battle(9, Some("pw"), "1".into());
		feed(&mut s, &["JOINBATTLE 9 -1", "JOINEDBATTLE 9 me 1"]);

		let taken = s.take_seat(2, 1, false).expect("a passworded room is ours");
		assert_eq!(
			s.seat().map(|seat| (seat.team, seat.ally_team)),
			Some((2, 1))
		);
		let decoded = status(&taken);
		assert!(decoded.player);
		assert_eq!((decoded.team, decoded.ally_team), (2, 1));
		// And the room's later question gets the same answer.
		assert!(status(&feed(&mut s, &["REQUESTBATTLESTATUS"])).player);

		assert!(!status(&s.release_seat()).player);
		assert_eq!(s.seat(), None);
		// Leaving forgets the seat, so the next room starts as a spectator.
		s.take_seat(2, 1, false).unwrap();
		s.leave_battle();
		assert_eq!(s.seat(), None);
	}

	#[test]
	fn a_private_host_is_asked_for_and_recognised_when_it_appears() {
		let mut s = ready_with_room();
		feed(
			&mut s,
			&[
				"ADDUSER Host[EU1] DE 9 SPADS",
				"ADDUSER Host[EU2] DE 10 SPADS",
				"ADDUSER Host[EU1][03] DE 11 SPADS",
				"ADDUSER Host[AU1] AU 12 SPADS",
				"CLIENTSTATUS Host[EU1] 64",
				"CLIENTSTATUS Host[EU2] 64",
				"CLIENTSTATUS Host[EU1][03] 64",
				"CLIENTSTATUS Host[AU1] 64",
			],
		);
		assert_eq!(
			s.cluster_managers(),
			["Host[AU1]", "Host[EU1]", "Host[EU2]"]
		);

		let asked = s.request_private_host("Host[EU1]").unwrap();
		assert!(
			matches!(&asked[..], [Effect::Send(env)] if env.line == "SAYPRIVATE Host[EU1] !privatehost")
		);

		let offered = feed(
			&mut s,
			&[
				"SAIDPRIVATE Host[EU1] Starting a new private instance in EU, password=ab12 - please wait",
			],
		);
		assert!(offered.contains(&Effect::PrivateHostOffered {
			manager: "Host[EU1]".into(),
			password: "ab12".into()
		}));

		// Someone else's room is not ours; ours carries our name in the title.
		assert!(
			feed(
				&mut s,
				&[
					"BATTLEOPENED 20 0 0 Host[EU1][04] 1.2.3.4 8452 16 1 0 h R\tv\tm\tsomeone else\tg"
				],
			)
			.is_empty()
		);
		let ready = feed(
			&mut s,
			&[
				"BATTLEOPENED 21 0 0 Host[EU1][05] 1.2.3.4 8452 16 1 0 h R\tv\tm\tme's private room\tg",
			],
		);
		assert!(ready.contains(&Effect::PrivateHostReady {
			id: 21,
			password: "ab12".into()
		}));
	}

	#[test]
	fn private_messages_become_effects() {
		let mut s = ready_with_room();
		assert_eq!(
			feed(
				&mut s,
				&["SAIDPRIVATE Coordinator Setting tweakdefs requires boss privileges"]
			),
			vec![Effect::PrivateChat {
				with: "Coordinator".into(),
				from: "Coordinator".into(),
				text: "Setting tweakdefs requires boss privileges".into()
			}]
		);
	}

	#[test]
	fn flood_disconnect_is_flagged() {
		let mut s = session();
		let effects = feed(&mut s, &["s.system.disconnect Flood protection"]);
		assert_eq!(
			effects,
			vec![Effect::Disconnected {
				reason: "Flood protection".into(),
				flood: true
			}]
		);
	}

	/// teiserver telling the room, us included, what our status now is.
	fn told(status: MyBattleStatus) -> String {
		format!("CLIENTBATTLESTATUS me {} 0", status.bits())
	}

	fn seated(ally_team: u8, ready: bool) -> MyBattleStatus {
		MyBattleStatus::player(Sync::Synced, 0, ally_team).ready(ready)
	}

	fn watching() -> MyBattleStatus {
		MyBattleStatus::spectator(Sync::Synced)
	}

	/// Every status we sent among `effects`, in order.
	fn statuses(effects: &[Effect]) -> Vec<BattleStatus> {
		effects
			.iter()
			.filter_map(|effect| match effect {
				Effect::Send(env) => env.line.strip_prefix("MYBATTLESTATUS "),
				_ => None,
			})
			.map(|rest| BattleStatus::from_bits(rest.split(' ').next().unwrap().parse().unwrap()))
			.collect()
	}

	fn pre_ready(s: &Session) -> bool {
		s.state.my_battle.as_ref().is_some_and(|my| my.pre_ready)
	}

	#[test]
	fn a_stale_answer_does_not_retract_a_seat_still_on_its_way() {
		let mut s = in_a_public_room();
		// The join's answer to the status request, then the auto-seat.
		feed(&mut s, &["REQUESTBATTLESTATUS"]);
		s.take_seat(0, 1, false).unwrap();

		// The first answer lands: us watching, from before the seat. Taken,
		// and the content check reporting in meanwhile still asks for the seat.
		feed(&mut s, &[&told(watching())]);
		let sent = sent_status(&s.set_synced(true));
		assert!(sent.player);
		assert_eq!(sent.ally_team, 1);

		// Once everything is answered, the seat is the server's.
		feed(&mut s, &[&told(seated(1, false)), &told(seated(1, false))]);
		assert_eq!(s.seat().map(|seat| seat.ally_team), Some(1));
	}

	#[test]
	fn a_refused_seat_leaves_us_watching() {
		let mut s = in_a_public_room();
		s.take_seat(0, 0, false).unwrap();
		// A full room keeps us a spectator, and queues us instead.
		feed(&mut s, &[&told(watching())]);
		assert_eq!(s.seat(), None);
		assert!(matches!(s.set_ready(true), Err(SeatError::Spectating)));
	}

	#[test]
	fn a_seat_the_server_gives_is_ours_to_ready() {
		let mut s = in_a_public_room();
		feed(&mut s, &[&told(seated(2, false))]);
		let sent = sent_status(&s.set_ready(true).unwrap());
		assert!(sent.player && sent.ready);
		assert_eq!(sent.ally_team, 2);
	}

	#[test]
	fn the_server_unreadying_us_is_taken() {
		let mut s = in_a_public_room();
		s.take_seat(0, 0, false).unwrap();
		s.set_ready(true).unwrap();
		feed(&mut s, &[&told(seated(0, false)), &told(seated(0, true))]);

		// The game ends and the server unreadies everyone. Readying again is a
		// change, so it goes out.
		feed(&mut s, &[&told(seated(0, false))]);
		assert!(sent_status(&s.set_ready(true).unwrap()).ready);
	}

	#[test]
	fn a_bonus_the_host_gave_goes_back_out_with_us() {
		let mut s = in_a_public_room();
		feed(&mut s, &[&told(seated(0, false).handicap(20))]);
		assert_eq!(sent_status(&s.set_ready(true).unwrap()).handicap, 20);
	}

	#[test]
	fn a_ready_given_in_the_queue_answers_the_seat_once() {
		let mut s = in_a_public_room();
		feed(&mut s, &["s.battle.queue_status 3\tme"]);
		assert_eq!(s.set_pre_ready(true).unwrap(), [Effect::RoomChanged]);

		// The consul seats us, unready. That is taken, and answered once.
		let effects = feed(&mut s, &[&told(seated(1, false))]);
		let sent = statuses(&effects);
		assert_eq!(sent.len(), 1, "{effects:?}");
		assert!(sent[0].player && sent[0].ready);
		assert_eq!(sent[0].ally_team, 1);
		assert!(effects.contains(&Effect::RoomChanged));
		assert!(!pre_ready(&s));

		// Its answer, then the queue update that follows: nothing more to send.
		assert!(
			statuses(&feed(
				&mut s,
				&[&told(seated(1, true)), "s.battle.queue_status 3"]
			))
			.is_empty()
		);
	}

	#[test]
	fn seated_from_the_queue_without_a_ready_given_is_only_seated() {
		let mut s = in_a_public_room();
		feed(&mut s, &["s.battle.queue_status 3\tme"]);
		assert!(statuses(&feed(&mut s, &[&told(seated(1, false))])).is_empty());
		assert_eq!(s.seat().map(|seat| seat.ready), Some(false));
	}

	#[test]
	fn a_ready_for_the_next_game_answers_its_end_once() {
		let mut s = in_a_public_room();
		// Seated and ready while the game runs.
		feed(&mut s, &[&told(seated(0, true))]);
		s.set_pre_ready(true).unwrap();

		let reset = statuses(&feed(&mut s, &[&told(seated(0, false))]));
		assert!(reset.first().is_some_and(|status| status.ready));

		// Its answer, then the next game's end: spent, so taken as it stands.
		feed(&mut s, &[&told(seated(0, true))]);
		assert!(statuses(&feed(&mut s, &[&told(seated(0, false))])).is_empty());
	}

	#[test]
	fn leaving_the_queue_or_the_seat_takes_a_ready_given_back() {
		let mut s = in_a_public_room();
		feed(&mut s, &["s.battle.queue_status 3\tme"]);
		s.set_pre_ready(true).unwrap();
		assert_eq!(
			feed(&mut s, &["s.battle.queue_status 3"]),
			[Effect::RoomChanged]
		);
		assert!(!pre_ready(&s));

		feed(&mut s, &[&told(seated(0, false))]);
		s.set_pre_ready(true).unwrap();
		assert!(s.release_seat().contains(&Effect::RoomChanged));
		assert!(!pre_ready(&s));
	}

	#[test]
	fn our_own_seat_answered_is_no_reason_to_ready() {
		let mut s = in_a_public_room();
		feed(&mut s, &["s.battle.queue_status 3\tme"]);
		s.set_pre_ready(true).unwrap();
		s.take_seat(0, 0, false).unwrap();
		assert!(statuses(&feed(&mut s, &[&told(seated(0, false))])).is_empty());
		assert!(pre_ready(&s));
	}

	#[test]
	fn our_status_waits_for_the_flood_window_as_one_line() {
		let mut s = in_a_public_room();
		let effects = s.take_seat(0, 0, false).unwrap();
		let Some(Effect::Send(env)) = effects.first() else {
			panic!("{effects:?}")
		};
		// Coalesced: a newer status replaces one still waiting to leave.
		assert_eq!(
			env.mode,
			spring_protocol::policy::Mode::Coalesce("me".into())
		);
	}

	#[test]
	fn requests_replaced_before_they_left_do_not_hold_the_seat() {
		let mut s = in_a_public_room();
		feed(&mut s, &[&told(seated(0, false))]);
		// Clicked fast: two go out, the three after them wait and replace
		// each other, so only the last of those leaves.
		for ready in [true, false, true, false, true] {
			s.set_ready(ready).unwrap();
		}
		feed(
			&mut s,
			&[
				&told(seated(0, true)),
				&told(seated(0, false)),
				&told(seated(0, true)),
			],
		);

		// The game ends and the server unreadies us: taken, and readying
		// again goes out rather than being mistaken for a request in flight.
		feed(&mut s, &[&told(seated(0, false))]);
		assert!(sent_status(&s.set_ready(true).unwrap()).ready);
	}

	#[test]
	fn a_ready_on_its_way_is_in_the_room_view_until_answered() {
		let mut s = in_a_public_room();
		// Listed, as we always are, so what the server shows for us is known.
		feed(
			&mut s,
			&["ADDUSER me EU 1 modlobby", &told(seated(0, false))],
		);
		let on_its_way = |s: &Session| {
			s.state
				.my_battle
				.as_ref()
				.and_then(|my| my.ready_on_its_way)
		};

		assert!(s.set_ready(true).unwrap().contains(&Effect::RoomChanged));
		assert_eq!(on_its_way(&s), Some(true));
		// Changing the faction meanwhile carries the same wish, and says nothing new.
		assert!(!s.set_side(1).unwrap().contains(&Effect::RoomChanged));

		// Shown ready once the server answers, so no longer on its way.
		assert!(feed(&mut s, &[&told(seated(0, true))]).contains(&Effect::RoomChanged));
		assert_eq!(on_its_way(&s), None);
	}

	#[test]
	fn a_ready_pressed_from_watching_follows_the_seat() {
		let mut s = in_a_public_room();
		s.take_seat(0, 1, true).unwrap();
		// The seat, answered as the server keeps it: unready. The ready follows.
		let sent = statuses(&feed(&mut s, &[&told(seated(1, false))]));
		assert_eq!(sent.len(), 1);
		assert!(sent[0].player && sent[0].ready);
		assert_eq!(sent[0].ally_team, 1);
		assert!(statuses(&feed(&mut s, &[&told(seated(1, true))])).is_empty());
		assert_eq!(s.seat().map(|seat| seat.ready), Some(true));
	}

	#[test]
	fn a_ready_pressed_from_watching_is_dropped_with_a_refused_seat() {
		let mut s = in_a_public_room();
		s.take_seat(0, 1, true).unwrap();
		// A full room keeps us watching and queues us.
		assert!(statuses(&feed(&mut s, &[&told(watching())])).is_empty());
		feed(&mut s, &["s.battle.queue_status 3\tme"]);
		// The queue seats us later: nothing was armed for it.
		assert!(statuses(&feed(&mut s, &[&told(seated(1, false))])).is_empty());
		assert!(!pre_ready(&s));
	}

	#[test]
	fn the_joins_stale_echo_does_not_spend_the_ready_that_follows_the_seat() {
		let mut s = in_a_public_room();
		feed(&mut s, &["REQUESTBATTLESTATUS"]);
		s.take_seat(0, 1, true).unwrap();
		assert!(statuses(&feed(&mut s, &[&told(watching())])).is_empty());
		let sent = statuses(&feed(&mut s, &[&told(seated(1, false))]));
		assert_eq!(sent.len(), 1);
		assert!(sent[0].ready);
	}

	#[test]
	fn standing_up_before_the_seat_is_answered_drops_the_ready() {
		let mut s = in_a_public_room();
		s.take_seat(0, 1, true).unwrap();
		s.release_seat();
		// A locked room refuses the release and echoes the seat instead.
		let effects = feed(&mut s, &[&told(seated(1, false)), &told(seated(1, false))]);
		assert!(statuses(&effects).is_empty(), "{effects:?}");
	}

	#[test]
	fn a_side_change_before_the_seat_is_answered_readies_once() {
		let mut s = in_a_public_room();
		s.take_seat(0, 1, true).unwrap();
		s.set_side(3).unwrap();
		let first = statuses(&feed(&mut s, &[&told(seated(1, false))]));
		let second = statuses(&feed(&mut s, &[&told(seated(1, false).side(3))]));
		let sent: Vec<_> = first.into_iter().chain(second).collect();
		assert_eq!(sent.len(), 1, "{sent:?}");
		assert!(sent[0].ready);
		assert_eq!(sent[0].side, 3);
	}

	#[test]
	fn an_explicit_ready_press_supersedes_the_one_that_follows_the_seat() {
		let mut s = in_a_public_room();
		s.take_seat(0, 0, true).unwrap();
		s.set_ready(true).unwrap();
		s.set_ready(false).unwrap();
		assert!(statuses(&feed(&mut s, &[&told(seated(0, false))])).is_empty());
	}

	#[test]
	fn a_request_replaced_before_it_left_is_not_waited_on() {
		let mut s = in_a_public_room();
		feed(&mut s, &[&told(seated(0, false))]);
		// Two side changes inside a full flood window: the second replaces the
		// first before it leaves, and the runtime says so.
		s.set_side(1).unwrap();
		s.set_side(3).unwrap();
		s.status_replaced();
		// The host moves us as it answers. That fits neither request: it is
		// the answer to the one line that left, and the seat is the server's.
		feed(&mut s, &[&told(seated(4, false).side(3))]);
		assert_eq!(s.seat().map(|seat| seat.ally_team), Some(4));
		// Nothing is left waiting, so the server's next word is simply taken.
		feed(&mut s, &[&told(seated(4, true).side(3))]);
		assert_eq!(s.seat().map(|seat| seat.ready), Some(true));
	}

	#[test]
	fn a_ready_pressed_behind_a_stand_up_follows_the_seat() {
		let mut s = in_a_public_room();
		feed(&mut s, &[&told(seated(0, false))]);
		// Spectate, Play, Ready, faster than the server answers. The seat lands
		// as a sit-down, which clears a ready sent with it; so none is.
		s.release_seat();
		s.take_seat(0, 0, false).unwrap();
		assert!(statuses(&s.set_ready(true).unwrap()).is_empty());
		assert!(statuses(&feed(&mut s, &[&told(watching())])).is_empty());
		let sent = statuses(&feed(&mut s, &[&told(seated(0, false))]));
		assert_eq!(sent.len(), 1);
		assert!(sent[0].ready);
	}

	#[test]
	fn a_ready_pressed_before_the_server_has_seated_us_follows_the_seat() {
		let mut s = in_a_public_room();
		s.take_seat(0, 0, false).unwrap();
		let effects = s.set_ready(true).unwrap();
		assert!(statuses(&effects).is_empty());
		assert!(
			effects.contains(&Effect::RoomChanged),
			"shown on its way at once"
		);
		let sent = statuses(&feed(&mut s, &[&told(seated(0, false))]));
		assert_eq!(sent.len(), 1);
		assert!(sent[0].ready);
	}

	#[test]
	fn a_seat_on_its_way_is_in_the_room_view_until_answered() {
		let mut s = in_a_public_room();
		feed(&mut s, &[&told(watching())]);
		let on_its_way = |s: &Session| s.state.my_battle.as_ref().and_then(|my| my.seat_on_its_way);

		assert!(
			s.take_seat(0, 1, false)
				.unwrap()
				.contains(&Effect::RoomChanged)
		);
		assert_eq!(
			on_its_way(&s),
			Some(SeatOnItsWay {
				player: true,
				ally_team: 1
			})
		);
		// Shown seated once the server answers, so no longer on its way.
		assert!(feed(&mut s, &[&told(seated(1, false))]).contains(&Effect::RoomChanged));
		assert_eq!(on_its_way(&s), None);

		assert!(s.release_seat().contains(&Effect::RoomChanged));
		assert_eq!(
			on_its_way(&s),
			Some(SeatOnItsWay {
				player: false,
				ally_team: 0
			})
		);
		feed(&mut s, &[&told(watching())]);
		assert_eq!(on_its_way(&s), None);
	}

	#[test]
	fn a_status_held_by_the_flood_window_is_in_the_room_view_until_it_leaves() {
		let mut s = in_a_public_room();
		let held = |s: &Session| s.state.my_battle.as_ref().and_then(|my| my.held_until_ms);

		assert_eq!(s.status_held(1_000), Some(Effect::RoomChanged));
		// Said again as the runtime tries again: the same moment, nothing new.
		assert_eq!(s.status_held(1_002), None);
		assert_eq!(held(&s), Some(1_000));
		assert_eq!(s.status_sent(), Some(Effect::RoomChanged));
		assert_eq!(held(&s), None);
		assert_eq!(s.status_sent(), None);
	}
}
