//! Collects deltas between flushes. High-frequency per-entity updates are
//! coalesced last-wins so a burst of `CLIENTSTATUS` lines costs one delta per
//! user; everything else keeps its order.

use std::collections::HashMap;

use crate::model::Delta;

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
    pending: Vec<Delta>,
    keyed: HashMap<Key, usize>,
}

impl Batcher {
    pub fn push(&mut self, delta: Delta) {
        if let Some(key) = key(&delta)
            && let Some(&index) = self.keyed.get(&key)
        {
            self.pending[index] = delta;
            return;
        }
        if let Some(key) = key(&delta) {
            self.keyed.insert(key, self.pending.len());
        }
        self.pending.push(delta);
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn take(&mut self) -> Vec<Delta> {
        self.keyed.clear();
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use spring_protocol::UserStatus;

    use super::*;

    #[test]
    fn coalesces_per_entity_and_keeps_the_rest() {
        let mut b = Batcher::default();
        let status = |bits| Delta::UserStatus {
            name: "bob".into(),
            status: UserStatus::from_bits(bits).into(),
        };
        b.push(status(0));
        b.push(Delta::UserRemoved {
            name: "alice".into(),
        });
        b.push(status(1));
        b.push(status(3));
        let taken = b.take();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0], status(3));
        assert!(b.is_empty());
        b.push(status(0));
        assert_eq!(b.take().len(), 1);
    }
}
