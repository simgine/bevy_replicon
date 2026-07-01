use core::marker::PhantomData;

use alloc::collections::VecDeque;
use bevy::prelude::*;
use bytes::Bytes;
use smallvec::{SmallVec, smallvec};

use crate::prelude::*;

/// Stores all received messages from server that arrived earlier then replication message with their tick.
///
/// Stores data sorted by ticks and maintains order of arrival.
/// Needed to ensure that when an message is triggered, all the data that it affects or references already exists.
#[derive(Resource)]
pub(super) struct MessageQueue<M> {
    entries: VecDeque<(RepliconTick, SmallVec<[Bytes; 4]>)>,
    marker: PhantomData<M>,
}

impl<M> MessageQueue<M> {
    pub(super) fn insert(&mut self, tick: RepliconTick, message: Bytes) {
        let index = self.entries.partition_point(|&(t, _)| tick.is_newer(t));
        if let Some((entry_tick, messages)) = self.entries.get_mut(index)
            && *entry_tick == tick
        {
            messages.push(message);
        } else {
            self.entries.insert(index, (tick, smallvec![message]));
        }
    }

    /// Pops the next message that is at least as old as the specified replicon tick.
    pub(super) fn pop_if_le(
        &mut self,
        update_tick: RepliconTick,
    ) -> Option<(RepliconTick, SmallVec<[Bytes; 4]>)> {
        let (tick, _) = self.entries.front()?;
        if tick.is_newer(update_tick) {
            return None;
        }

        self.entries.pop_front()
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}

impl<M> Default for MessageQueue<M> {
    fn default() -> Self {
        Self {
            entries: Default::default(),
            marker: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_tick() {
        let mut queue = MessageQueue::<Test>::default();
        queue.insert(RepliconTick::new(1), Default::default());

        assert_eq!(queue.len(), 1);
        assert!(queue.pop_if_le(RepliconTick::new(0)).is_none());
    }

    #[test]
    fn bigger_tick() {
        let mut queue = MessageQueue::<Test>::default();
        queue.insert(RepliconTick::new(1), Default::default());

        assert!(queue.pop_if_le(RepliconTick::new(2)).is_some());
        assert!(queue.is_empty());
    }

    #[test]
    fn ticks_ordering() {
        let mut queue = MessageQueue::<Test>::default();
        queue.insert(RepliconTick::new(0), Default::default());
        queue.insert(RepliconTick::new(1), Default::default());
        queue.insert(RepliconTick::new(2), Default::default());

        let (tick, _) = queue.pop_if_le(RepliconTick::new(1)).unwrap();
        assert_eq!(tick, RepliconTick::new(0));

        let (tick, _) = queue.pop_if_le(RepliconTick::new(1)).unwrap();
        assert_eq!(tick, RepliconTick::new(1));

        assert!(queue.pop_if_le(RepliconTick::new(1)).is_none());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn messages_ordering() {
        let mut queue = MessageQueue::<Test>::default();
        queue.insert(RepliconTick::new(0), Bytes::from_static(&[0]));
        queue.insert(RepliconTick::new(0), Bytes::from_static(&[1]));

        let (_, messages) = queue.pop_if_le(RepliconTick::new(0)).unwrap();
        let bytes: Vec<_> = messages.into_iter().flatten().collect();
        assert_eq!(bytes, [0, 1]);
        assert!(queue.is_empty());
    }

    struct Test;
}
