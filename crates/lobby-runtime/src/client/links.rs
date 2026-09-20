//! The servers the runtime holds a link to: opening one, losing one, coming
//! back to one and letting one go. Everything here is about *which* server
//! and whether it is there; what is said over a link once it is up stays in
//! the parent.

use super::*;

pub(super) struct Connection {
	pub(super) transport: Transport,
	pub(super) inbound: mpsc::Receiver<Inbound>,
	pub(super) session: Session,
}

/// One server: the link to it while there is one, and what it takes to get
/// it back.
#[derive(Default)]
pub(super) struct Server {
	pub(super) link: Option<Connection>,
	/// The connect that is out, by its number; it comes back as
	/// [`Next::Opened`]. A logout clears this, so what comes back finds
	/// nobody waiting and a new login need not wait for it.
	pub(super) connecting: Option<u64>,
	/// How to log in again, kept from the last attempt so a drop can be
	/// recovered from without the user typing anything; gone after a logout.
	pub(super) credentials: Option<(Endpoint, LoginRequest)>,
	pub(super) reconnect: reconnect::Reconnect,
	/// Whoever is waiting to be logged in: the login that opened the
	/// connection, or the code that finishes one the server would not accept.
	pub(super) login_reply: Option<Reply<()>>,
	/// Waiting on `REGISTRATIONDENIED`, or on the agreement that the login
	/// after `REGISTRATIONACCEPTED` is answered with.
	pub(super) register_reply: Option<Reply<Vec<String>>>,
}

/// What a connect was for, carried while it is out.
pub(super) enum Purpose {
	Login(Reply<()>),
	Register {
		email: String,
		password: String,
		reply: Reply<Vec<String>>,
	},
}

impl Purpose {
	pub(super) fn fail(self, err: ClientError) {
		match self {
			Self::Login(reply) => {
				let _ = reply.send(Err(err));
			}
			Self::Register { reply, .. } => {
				let _ = reply.send(Err(err));
			}
		}
	}
}

/// A connect that went out, come back. Connects run beside the actor, so one
/// server taking its time never holds up another.
pub(super) struct Opened {
	pub(super) server: String,
	/// Which connect this was; see [`Server::connecting`].
	pub(super) attempt: u64,
	pub(super) host: String,
	pub(super) request: LoginRequest,
	pub(super) purpose: Purpose,
	pub(super) result: Result<Connected, TransportError>,
}

impl Runtime {
	pub(super) fn try_recv_inbound(&mut self, server: &str) -> Option<Inbound> {
		self.link_mut(server)?.inbound.try_recv().ok()
	}

	pub(super) fn link(&self, server: &str) -> Option<&Connection> {
		self.servers.get(server)?.link.as_ref()
	}

	pub(super) fn link_mut(&mut self, server: &str) -> Option<&mut Connection> {
		self.servers.get_mut(server)?.link.as_mut()
	}

	/// Every server there is a link to.
	pub(super) fn linked(&self) -> Vec<String> {
		self.servers
			.iter()
			.filter(|(_, server)| server.link.is_some())
			.map(|(id, _)| id.clone())
			.collect()
	}

	/// The server whose room we are in. There is one room at most, across
	/// every server: joining one leaves the other.
	pub(super) fn room(&self) -> Option<String> {
		self.servers
			.iter()
			.find(|(_, server)| {
				server
					.link
					.as_ref()
					.is_some_and(|link| link.session.state.my_battle.is_some())
			})
			.map(|(id, _)| id.clone())
	}

	/// Where a room's command goes: the room's server, or with no room any
	/// server, whose session then says there is no room.
	pub(super) fn room_or_any(&self) -> Option<String> {
		self.room().or_else(|| self.linked().into_iter().next())
	}

	/// Leaves the room on every server but `server`, which has just let us
	/// into one: one room at a time. After the join rather than before it, as
	/// a server does it for its own rooms — a join that is refused costs
	/// nothing, and two asked for at once still end in one room. Sent
	/// straight out rather than through `apply_effects`, which the join is
	/// already inside.
	pub(super) async fn leave_rooms_except(&mut self, server: &str) {
		for other in self.linked() {
			if other == server {
				continue;
			}
			let Some(link) = self.link_mut(&other) else {
				continue;
			};
			// Only a room we are in: `leave_battle` also drops a join still
			// being answered, and that one settles itself the same way.
			if link.session.state.my_battle.is_none() {
				continue;
			}
			let leaving = link.session.leave_battle();
			self.project_effects(&other, &leaving);
			for effect in leaving {
				if let Effect::Send(envelope) = effect {
					let _ = self.send_line(&other, envelope).await;
				}
			}
			self.game = None;
			self.auto_launch = None;
		}
	}

	/// Lets `server` go and stops coming back to it: asked for, by a logout
	/// or by the idle limit.
	pub(super) async fn log_out(&mut self, server: &str) {
		let Some(slot) = self.servers.get_mut(server) else {
			return;
		};
		slot.credentials = None;
		slot.reconnect.stop();
		// A connect still out comes back unwanted and is dropped then.
		if slot.connecting.take().is_some() {
			self.batcher.push_for(server, Delta::Phase(None));
		}
		self.disconnect(server).await;
		self.announce_retry(server);
	}

	/// Starts a connect to the endpoint's server, for a login or a
	/// registration. It runs beside the actor and comes back as
	/// [`Next::Opened`], trying the way in that worked last on its own first.
	pub(super) fn connect(
		&mut self,
		mut endpoint: Endpoint,
		request: LoginRequest,
		purpose: Purpose,
	) {
		let server = server_id(&endpoint.host);
		let slot = self.servers.entry(server.clone()).or_default();
		if slot.link.is_some() || slot.connecting.is_some() {
			purpose.fail(ClientError::AlreadyConnected);
			return;
		}
		self.attempts += 1;
		let attempt = self.attempts;
		slot.connecting = Some(attempt);
		slot.credentials = Some((endpoint.clone(), request.clone()));
		self.batcher
			.push_for(&server, Delta::Phase(Some(Phase::Connecting)));
		self.flush();
		endpoint.preferred = self.ways.get(&endpoint.host);
		let connecting = (self.connector)(endpoint.clone(), self.policy.clone());
		let opened = self.opened_tx.clone();
		tokio::spawn(async move {
			let result = connecting.await;
			let _ = opened
				.send(Opened {
					server,
					attempt,
					host: endpoint.host,
					request,
					purpose,
					result,
				})
				.await;
		});
	}

	/// A connect came back: the link is up — unless nobody wants it any
	/// more, having logged out meanwhile — or it failed.
	///
	/// A registration's link is the one an account is created on and then
	/// lives on: what it sends on `Welcome` is a `REGISTER` rather than a
	/// `LOGIN`, and everything after that is a login. The account exists but
	/// is unverified, so the connection stays open for the code that verifies
	/// it, and the credentials are kept exactly as a login keeps them.
	pub(super) async fn on_opened(&mut self, opened: Opened) {
		let Opened {
			server,
			attempt,
			host,
			request,
			purpose,
			result,
		} = opened;
		let slot = self.servers.entry(server.clone()).or_default();
		// Logged out of while it was out. The phase is not this connect's to
		// touch any more: the logout said it, and a newer login may be on.
		if slot.connecting != Some(attempt) {
			if let Ok((transport, ..)) = result {
				transport.shutdown().await;
			}
			purpose.fail(ClientError::Refused("logged out while connecting".into()));
			return;
		}
		slot.connecting = None;
		let (transport, inbound, way) = match result {
			Ok(connected) => connected,
			Err(err) => {
				self.batcher.push_for(&server, Delta::Phase(None));
				purpose.fail(err.into());
				return;
			}
		};
		tracing::info!(host, %way, "connected");
		self.ways.remember(&host, way);
		self.ways_changed();
		let session = Session::new(
			request,
			self.hardware.properties.clone(),
			self.hardware.machine_hash.clone(),
		);
		let slot = self.servers.entry(server.clone()).or_default();
		let session = match purpose {
			Purpose::Login(reply) => {
				slot.login_reply = Some(reply);
				session
			}
			Purpose::Register {
				email,
				password,
				reply,
			} => {
				slot.register_reply = Some(reply);
				session.registering(email, password)
			}
		};
		slot.link = Some(Connection {
			transport,
			inbound,
			session,
		});
	}

	/// The server said no (before or after login): answer whoever waits, then drop the link.
	/// A refusal we can do nothing about, except when it is the server telling
	/// us to wait — which is the one refusal worth answering by waiting.
	pub(super) async fn refuse(&mut self, server: &str, reason: String) {
		let was_room = self.room().as_deref() == Some(server);
		let transient = reason.to_ascii_lowercase().contains("flood protection");
		let slot = self.servers.entry(server.to_owned()).or_default();
		if transient && slot.credentials.is_some() {
			slot.reconnect
				.flooded(Instant::now(), rand::random::<f64>());
			tracing::info!(
				server,
				reason,
				"login refused as flooding; will wait and retry"
			);
		}
		let link = slot.link.take();
		self.reply_login(server, Err(ClientError::Refused(reason.clone())));
		self.reply_join(server, Err(ClientError::Refused(reason)));
		if let Some(conn) = link {
			conn.transport.shutdown().await;
		}
		if was_room {
			self.game = None;
			self.auto_launch = None;
		}
		self.batcher.push_for(server, Delta::Phase(None));
		self.announce_retry(server);
	}

	pub(super) fn connection_lost(&mut self, server: &str, reason: String) {
		tracing::warn!(server, reason, "connection lost");
		let was_room = self.room().as_deref() == Some(server);
		// A drop while a game is running gets the longer wait: it was probably
		// the server, and everyone in that game is about to try at once.
		let in_game = self.game.is_some() || self.engine.is_some();
		let slot = self.servers.entry(server.to_owned()).or_default();
		if slot.credentials.is_some() {
			slot.reconnect
				.disconnected(Instant::now(), in_game, rand::random::<f64>());
		}
		slot.link = None;
		let armed = slot.reconnect.is_armed();
		self.reply_login(server, Err(ClientError::Refused(reason.clone())));
		self.reply_join(server, Err(ClientError::Refused(reason.clone())));
		if was_room {
			self.game = None;
			self.auto_launch = None;
			// The scheduler went with the connection; what it held is not coming.
			if self.paste.take().is_some() {
				self.batcher.push(Delta::Paste(PasteStatus::Idle));
			}
		}
		let text = if armed {
			format!("connection lost: {reason} — trying again shortly")
		} else {
			format!("connection lost: {reason}")
		};
		// A warning: the network and the server are not the app's to fix, and
		// `Error` makes the next look for a fix come sooner. A server restart
		// would otherwise put every client on that ladder at once.
		self.batcher.push_for(
			server,
			Delta::Notice {
				level: lobby_ui::NoticeLevel::Warning,
				text,
			},
		);
		self.batcher.push_for(server, Delta::Phase(None));
		self.announce_retry(server);
	}

	/// Lets the server go and stops coming back. The credentials go with it:
	/// a window nobody is at should not log in again on its own, and the
	/// login screen has the remembered password anyway.
	pub(super) async fn idle_disconnect(&mut self) {
		tracing::info!("idle past the limit; letting the servers go");
		for server in self.servers.keys().cloned().collect::<Vec<_>>() {
			self.log_out(&server).await;
		}
		self.batcher.push(Delta::Notice {
			level: lobby_ui::NoticeLevel::Info,
			text: "disconnected: nobody has touched the lobby for a while".into(),
		});
	}

	/// Tries the last known credentials again.
	pub(super) async fn try_reconnect(&mut self) {
		let now = Instant::now();
		let due: Vec<String> = self
			.servers
			.iter()
			.filter(|(_, slot)| slot.reconnect.due(now))
			.map(|(id, _)| id.clone())
			.collect();
		// A drop is not worth recovering from for nobody.
		if !due.is_empty() && self.idle.due(now) {
			self.idle_disconnect().await;
			return;
		}
		for server in due {
			let slot = self.servers.entry(server.clone()).or_default();
			let Some((endpoint, request)) = slot.credentials.clone() else {
				slot.reconnect.stop();
				continue;
			};
			slot.reconnect.attempted(now);
			self.announce_retry(&server);
			tracing::info!(server, "reconnecting");
			let (tx, _rx) = oneshot::channel();
			self.connect(endpoint, request, Purpose::Login(tx));
		}
	}

	pub(super) async fn disconnect(&mut self, server: &str) {
		let Some(mut conn) = self
			.servers
			.get_mut(server)
			.and_then(|slot| slot.link.take())
		else {
			return;
		};
		let leaving = conn.session.leave_battle();
		let in_room = !leaving.is_empty();
		if in_room {
			for effect in leaving {
				if let Effect::Send(envelope) = effect {
					let _ = conn.transport.send(envelope).await;
				}
			}
			// Let the writer flush LEAVEBATTLE before the socket goes away.
			tokio::time::sleep(Duration::from_millis(300)).await;
		}
		conn.transport.shutdown().await;
		if in_room {
			self.game = None;
			self.auto_launch = None;
			self.batcher.push_for(server, Delta::MyBattle(None));
			self.batcher.push_for(server, Delta::GameRunning(None));
		}
		self.batcher.push_for(server, Delta::Phase(None));
	}

	pub(super) fn reply_login(&mut self, server: &str, result: Result<(), ClientError>) {
		if let Some(reply) = self
			.servers
			.get_mut(server)
			.and_then(|slot| slot.login_reply.take())
		{
			let _ = reply.send(result);
		}
	}

	/// Seconds until the policy's next attempt, for the corner to count down.
	pub(super) fn retry_in(&self, server: &str) -> Option<u64> {
		self.servers
			.get(server)?
			.reconnect
			.until_due(Instant::now())
			.map(|wait| wait.as_secs())
	}

	/// Tells the front end when `server` is next tried again, if at all,
	/// after anything that moved it: armed, attempted, or called off.
	pub(super) fn announce_retry(&mut self, server: &str) {
		let delta = Delta::RetryIn(self.retry_in(server));
		self.batcher.push_for(server, delta);
	}

	/// One server as a front end is to hold it, or `None` for a server there
	/// is nothing to say about: logged out of, and staying that way.
	pub(super) fn session_snapshot(&self, id: &str) -> Option<ServerSnapshot> {
		let slot = self.servers.get(id)?;
		let session = match &slot.link {
			Some(conn) => ServerSnapshot::from_state(
				id,
				&conn.session.state,
				self.game
					.as_ref()
					.filter(|_| self.room().as_deref() == Some(id))
					.map(|game| game.view.clone()),
			),
			// On its way: a snapshot must not make this one look like nobody
			// is trying.
			None if slot.connecting.is_some() => ServerSnapshot {
				phase: Some(Phase::Connecting),
				..ServerSnapshot::disconnected(id)
			},
			// Gone, but coming back: the front end counts down to it.
			None if slot.reconnect.is_armed() => ServerSnapshot::disconnected(id),
			None => return None,
		};
		Some(ServerSnapshot {
			retry_in: self.retry_in(id),
			..session
		})
	}
}

/// The next thing any server's link delivers, and whose; never, with no
/// links. `turn` is who is asked first, so that a server with plenty to say
/// does not keep the others waiting: the caller moves it on each time.
pub(super) async fn recv_any(
	servers: &mut BTreeMap<String, Server>,
	turn: usize,
) -> (String, Inbound) {
	std::future::poll_fn(|cx| {
		let mut links: Vec<_> = servers.iter_mut().collect();
		let first = turn % links.len().max(1);
		links.rotate_left(first);
		for (id, server) in links {
			let Some(link) = server.link.as_mut() else {
				continue;
			};
			if let std::task::Poll::Ready(message) = link.inbound.poll_recv(cx) {
				let inbound = message.unwrap_or(Inbound::Closed {
					reason: "transport task ended".into(),
				});
				return std::task::Poll::Ready((id.clone(), inbound));
			}
		}
		std::task::Poll::Pending
	})
	.await
}
