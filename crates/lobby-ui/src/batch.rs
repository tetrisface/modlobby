//! Collects deltas between flushes. High-frequency per-entity updates are
//! coalesced last-wins so a burst of `CLIENTSTATUS` lines costs one delta per
//! user; everything else keeps its order. Each delta keeps the server it came
//! from, since two servers can each have a "bob" and a battle 12.

use std::collections::HashMap;

use crate::model::{Delta, UiMessage};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
	UserStatus(String),
	BattleInfo(u32),
	MemberStatus(String),
}

fn key(delta: &Delta) -> Option<Key> {
	match delta {
		Delta::UserStatus { name, .. } => Some(Key::UserStatus(name.clone())),
		Delta::BattleInfo { id, .. } => Some(Key::BattleInfo(*id)),
		Delta::MemberStatus { name, .. } => Some(Key::MemberStatus(name.clone())),
		_ => None,
	}
}

#[derive(Debug, Default)]
pub struct Batcher {
	/// In arrival order, each with the server it came from.
	pending: Vec<(Option<String>, Delta)>,
	keyed: HashMap<(Option<String>, Key), usize>,
}

impl Batcher {
	/// A change to this machine's own state rather than to a session's.
	pub fn push(&mut self, delta: Delta) {
		self.add(None, delta);
	}

	/// A change to `server`'s session.
	pub fn push_for(&mut self, server: &str, delta: Delta) {
		self.add(Some(server.to_owned()), delta);
	}

	fn add(&mut self, server: Option<String>, delta: Delta) {
		let keyed = key(&delta).map(|key| (server.clone(), key));
		if let Some(keyed) = &keyed
			&& let Some(&index) = self.keyed.get(keyed)
		{
			self.pending[index] = (server, delta);
			return;
		}
		if let Some(keyed) = keyed {
			self.keyed.insert(keyed, self.pending.len());
		}
		self.pending.push((server, delta));
	}

	pub fn is_empty(&self) -> bool {
		self.pending.is_empty()
	}

	/// Everything pending, one message per run of deltas from the same
	/// source, in the order they came.
	pub fn take(&mut self) -> Vec<UiMessage> {
		self.keyed.clear();
		let mut messages: Vec<UiMessage> = Vec::new();
		for (server, delta) in std::mem::take(&mut self.pending) {
			match messages.last_mut() {
				Some(UiMessage::Deltas {
					server: last,
					deltas,
				}) if *last == server => deltas.push(delta),
				_ => messages.push(UiMessage::Deltas {
					server,
					deltas: vec![delta],
				}),
			}
		}
		messages
	}
}

#[cfg(test)]
mod tests {
	use spring_protocol::UserStatus;

	use super::*;

	fn status(bits: u32) -> Delta {
		Delta::UserStatus {
			name: "bob".into(),
			status: UserStatus::from_bits(bits).into(),
		}
	}

	fn run(server: Option<&str>, deltas: Vec<Delta>) -> UiMessage {
		UiMessage::Deltas {
			server: server.map(str::to_owned),
			deltas,
		}
	}

	#[test]
	fn coalesces_per_entity_and_keeps_the_rest() {
		let mut b = Batcher::default();
		b.push_for("a", status(0));
		b.push_for(
			"a",
			Delta::UserRemoved {
				name: "alice".into(),
			},
		);
		b.push_for("a", status(1));
		b.push_for("a", status(3));
		let removed = Delta::UserRemoved {
			name: "alice".into(),
		};
		assert_eq!(b.take(), vec![run(Some("a"), vec![status(3), removed])]);
		assert!(b.is_empty());
		b.push_for("a", status(0));
		assert_eq!(b.take().len(), 1);
	}

	#[test]
	fn the_same_name_on_two_servers_is_two_people() {
		let mut b = Batcher::default();
		b.push_for("a", status(0));
		b.push_for("b", status(1));
		b.push_for("a", status(3));
		assert_eq!(
			b.take(),
			vec![
				run(Some("a"), vec![status(3)]),
				run(Some("b"), vec![status(1)]),
			]
		);
	}

	#[test]
	fn runs_keep_the_order_the_changes_came_in() {
		let mut b = Batcher::default();
		let engine = Delta::Engine(crate::EngineStatus::Idle);
		b.push_for("a", Delta::Phase(None));
		b.push(engine.clone());
		b.push_for("a", Delta::RetryIn(Some(3)));
		assert_eq!(
			b.take(),
			vec![
				run(Some("a"), vec![Delta::Phase(None)]),
				run(None, vec![engine]),
				run(Some("a"), vec![Delta::RetryIn(Some(3))]),
			]
		);
	}
}
