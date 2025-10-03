use alloc::collections::VecDeque;

use bevy::prelude::*;
use log::trace;

use crate::prelude::*;

/// Received ticks for mutate message from server.
///
/// For efficiency we store only the last received tick and
/// an array indicating whether all mutate messages for the most
/// recent 64 ticks were received.
///
/// Inserted to the world in [`ClientPlugin::finish`](super::ClientPlugin::finish) if
/// [`TrackAppExt::track_mutate_messages`](crate::shared::replication::track_mutate_messages::TrackAppExt::track_mutate_messages)
/// were called.
///
/// See also [`MutateTickReceived`] and [`ServerUpdateTick`](super::ServerUpdateTick).
#[derive(Debug, Resource)]
pub struct ServerMutateTicks {
    ticks: VecDeque<TickMessages>,

    /// The last received server tick with mutation.
    last_tick: RepliconTick,
}

impl ServerMutateTicks {
    /// Returns the last received tick.
    pub fn last_tick(&self) -> RepliconTick {
        self.last_tick
    }

    /// Returns a mask that represents the received ticks.
    pub fn mask(&self) -> u64 {
        let mut bitmask = 0;

        for (i, tick) in self.ticks.iter().enumerate() {
            if tick.all_received() {
                bitmask |= 1 << i;
            }
        }

        bitmask
    }

    /// Returns `true` if this tick is confirmed for an entity.
    ///
    /// All ticks older then 64 ticks since [`Self::last_tick`] are considered received.
    pub fn contains(&self, tick: RepliconTick) -> bool {
        if tick > self.last_tick {
            return false;
        }

        let ago = self.last_tick - tick;
        if let Some(tick) = self.ticks.get(ago as usize) {
            tick.all_received()
        } else {
            true
        }
    }

    /// Returns `true` if any tick in the given range was confirmed for the entity with
    /// this component.
    ///
    /// All ticks older then 64 ticks since [`Self::last_tick`] are considered received.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` are enabled and
    /// `start_tick` is greater then `end_tick`.
    pub fn contains_any(&self, start_tick: RepliconTick, end_tick: RepliconTick) -> bool {
        debug_assert!(start_tick <= end_tick);

        if start_tick > self.last_tick {
            return false;
        }
        if start_tick <= self.last_tick - self.ticks.len() as u32 {
            return true;
        }

        let end_tick = if end_tick < self.last_tick {
            end_tick
        } else {
            self.last_tick
        };

        // Start and end are reversed because ticks in the
        // array are stored in decreasing order.
        let end = (self.last_tick - start_tick) as usize;
        let start = (self.last_tick - end_tick) as usize;
        self.ticks
            .range(start..=end)
            .any(|tick| tick.all_received())
    }

    /// Confirms a message was received for a tick and initializes the number of sent
    /// messages for it.
    ///
    /// Return `true` if the number of received messages matches `messages_count`.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` are enabled and `messages_count` is different
    /// from the last call or if the number of calls for the same tick exceeds `messages_count`.
    pub fn confirm(&mut self, tick: RepliconTick, messages_count: usize) -> bool {
        let len = self.ticks.len();
        debug_assert_eq!(len, u64::BITS as usize);

        if tick > self.last_tick {
            let delta = (tick - self.last_tick) as usize;
            trace!("confirming `{tick:?}` which is {delta} ticks since last");
            if delta >= len {
                // If the difference exceeds the size, clear all ticks.
                self.ticks.clear();
                self.ticks.resize(u64::BITS as usize, Default::default());
            } else {
                for _ in 0..delta {
                    self.ticks.pop_back();
                    self.ticks.push_front(Default::default());
                }
            }

            self.last_tick = tick;
            self.ticks[0].confirm(messages_count)
        } else {
            let delta = (self.last_tick - tick) as usize;
            trace!("confirming `{tick:?}` which is {delta} ticks ago");
            if delta < len {
                self.ticks[delta].confirm(messages_count)
            } else {
                false
            }
        }
    }

    pub(super) fn clear(&mut self) {
        for tick in &mut self.ticks {
            *tick = Default::default();
        }
        self.last_tick = Default::default();
    }
}

impl Default for ServerMutateTicks {
    fn default() -> Self {
        Self {
            ticks: VecDeque::from([Default::default(); u64::BITS as usize]),
            last_tick: Default::default(),
        }
    }
}

/// Tracker for mutable messages received for a tick.
#[derive(Clone, Copy, Debug, Default)]
struct TickMessages {
    /// Number of sent messages.
    ///
    /// If zero, we consider the tick as completely non-received.
    messages_count: usize,

    /// Number of received messages.
    received: usize,
}

impl TickMessages {
    fn confirm(&mut self, messages_count: usize) -> bool {
        debug_assert_ne!(messages_count, 0);
        debug_assert!(
            self.messages_count == 0 || self.messages_count == messages_count,
            "messages count shouldn't change, expected {}, but got {messages_count}",
            self.messages_count
        );

        self.messages_count = messages_count;
        self.received += 1;

        debug_assert!(
            self.received <= self.messages_count,
            "expected at most {} messages, but confirmed {}",
            self.messages_count,
            self.received,
        );

        self.all_received()
    }

    fn all_received(&self) -> bool {
        self.messages_count != 0 && self.messages_count == self.received
    }
}

/// A message that indicates that all mutate messages are received for a tick.
///
/// See also [`ServerMutateTicks`].
#[derive(Message, Debug, Clone, Copy)]
pub struct MutateTickReceived {
    /// Message(s) tick.
    pub tick: RepliconTick,
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn contains() {
        let mut ticks = ServerMutateTicks::default();
        ticks.confirm(RepliconTick::new(1), 1);

        assert!(!ticks.contains(RepliconTick::new(0)));
        assert!(ticks.contains(RepliconTick::new(1)));
        assert!(!ticks.contains(RepliconTick::new(2)));
        assert!(!ticks.contains(RepliconTick::new(u32::MAX)));
    }

    #[test]
    fn contains_with_wrapping() {
        let mut ticks = ServerMutateTicks::default();
        ticks.confirm(RepliconTick::new(u64::BITS + 1), 1);

        assert!(ticks.contains(RepliconTick::new(0)));
        assert!(ticks.contains(RepliconTick::new(1)));
        assert!(!ticks.contains(RepliconTick::new(2)));

        assert!(!ticks.contains(RepliconTick::new(u64::BITS)));
        assert!(ticks.contains(RepliconTick::new(u64::BITS + 1)));
        assert!(!ticks.contains(RepliconTick::new(u64::BITS + 2)));
    }

    #[test]
    fn contains_any() {
        let mut ticks = ServerMutateTicks::default();
        ticks.confirm(RepliconTick::new(1), 1);

        assert!(ticks.contains_any(RepliconTick::new(0), RepliconTick::new(1)));
        assert!(ticks.contains_any(RepliconTick::new(1), RepliconTick::new(1)));
        assert!(ticks.contains_any(RepliconTick::new(1), RepliconTick::new(2)));
        assert!(!ticks.contains_any(RepliconTick::new(2), RepliconTick::new(3)));
        assert!(!ticks.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(0)));
        assert!(ticks.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(1)));
        assert!(ticks.contains_any(RepliconTick::new(0), RepliconTick::new(2)));
        assert!(ticks.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(3)));
    }

    #[test]
    fn contains_any_with_wrapping() {
        let mut ticks = ServerMutateTicks::default();
        ticks.confirm(RepliconTick::new(u64::BITS + 1), 1);

        assert!(ticks.contains_any(RepliconTick::new(0), RepliconTick::new(1)));
        assert!(ticks.contains_any(RepliconTick::new(1), RepliconTick::new(2)));
        assert!(!ticks.contains_any(RepliconTick::new(2), RepliconTick::new(3)));
        assert!(ticks.contains_any(RepliconTick::new(0), RepliconTick::new(3)));

        assert!(ticks.contains_any(
            RepliconTick::new(u64::BITS),
            RepliconTick::new(u64::BITS + 1)
        ));
        assert!(ticks.contains_any(
            RepliconTick::new(u64::BITS + 1),
            RepliconTick::new(u64::BITS + 2)
        ));
        assert!(!ticks.contains_any(
            RepliconTick::new(u64::BITS + 2),
            RepliconTick::new(u64::BITS + 3)
        ));
        assert!(ticks.contains_any(
            RepliconTick::new(u64::BITS),
            RepliconTick::new(u64::BITS + 3)
        ));
    }

    #[test]
    fn contains_any_with_older() {
        let mut ticks = ServerMutateTicks::default();
        ticks.confirm(RepliconTick::new(1), 1);
        assert_eq!(ticks.mask(), 0b1);

        ticks.confirm(RepliconTick::new(u32::MAX), 1);
        assert_eq!(ticks.mask(), 0b101);

        assert!(ticks.contains_any(RepliconTick::new(0), RepliconTick::new(1)));
        assert!(ticks.contains_any(RepliconTick::new(1), RepliconTick::new(2)));
        assert!(!ticks.contains_any(RepliconTick::new(2), RepliconTick::new(3)));
        assert!(ticks.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(0)));
        assert!(ticks.contains_any(RepliconTick::new(0), RepliconTick::new(2)));
        assert!(ticks.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(3)));
        assert!(ticks.contains_any(RepliconTick::new(u32::MAX - 1), RepliconTick::new(u32::MAX)));
        assert!(!ticks.contains_any(
            RepliconTick::new(u32::MAX - 2),
            RepliconTick::new(u32::MAX - 1)
        ));
    }

    #[test]
    fn confirm_newer() {
        let mut ticks = ServerMutateTicks::default();
        ticks.confirm(RepliconTick::new(1), 1);
        ticks.confirm(RepliconTick::new(2), 1);
        assert_eq!(ticks.mask(), 0b11);

        assert!(!ticks.contains(RepliconTick::new(0)));
        assert!(ticks.contains(RepliconTick::new(1)));
        assert!(ticks.contains(RepliconTick::new(2)));
    }

    #[test]
    fn confirm_older() {
        let mut ticks = ServerMutateTicks::default();
        ticks.confirm(RepliconTick::new(1), 1);
        ticks.confirm(RepliconTick::new(0), 1);
        assert_eq!(ticks.mask(), 0b11);

        assert!(ticks.contains(RepliconTick::new(0)));
        assert!(ticks.contains(RepliconTick::new(1)));
        assert!(!ticks.contains(RepliconTick::new(2)));
    }

    #[test]
    fn confirm_same() {
        let mut ticks = ServerMutateTicks::default();
        assert!(!ticks.confirm(RepliconTick::new(1), 2));
        assert_eq!(ticks.mask(), 0b0);
        assert!(ticks.confirm(RepliconTick::new(1), 2));
        assert_eq!(ticks.mask(), 0b1);

        assert!(!ticks.contains(RepliconTick::new(0)));
        assert!(ticks.contains(RepliconTick::new(1)));
        assert!(!ticks.contains(RepliconTick::new(2)));
    }

    #[test]
    fn confirm_with_wrapping() {
        let mut ticks = ServerMutateTicks::default();
        ticks.confirm(RepliconTick::new(1), 1);
        ticks.confirm(RepliconTick::new(u64::BITS + 1), 1);
        assert_eq!(ticks.mask(), 0b1);

        assert!(ticks.contains(RepliconTick::new(0)));
        assert!(ticks.contains(RepliconTick::new(1)));
        assert!(!ticks.contains(RepliconTick::new(2)));
        assert!(!ticks.contains(RepliconTick::new(u64::BITS)));
        assert!(ticks.contains(RepliconTick::new(u64::BITS + 1)));
        assert!(!ticks.contains(RepliconTick::new(u64::BITS + 2)));
    }

    #[test]
    fn confirm_with_overflow() {
        let mut ticks = ServerMutateTicks::default();
        ticks.confirm(RepliconTick::new(u32::MAX), 1);
        ticks.confirm(RepliconTick::new(1), 1);
        assert_eq!(ticks.mask(), 0b101);

        assert!(!ticks.contains(RepliconTick::new(0)));
        assert!(ticks.contains(RepliconTick::new(1)));
        assert!(!ticks.contains(RepliconTick::new(3)));
        assert!(ticks.contains(RepliconTick::new(u32::MAX)));
    }
}
