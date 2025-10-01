use core::fmt::{self, Debug, Formatter};

use bevy::prelude::*;

use crate::prelude::*;

/// Received ticks from the server for an entity.
///
/// For efficiency we store only the last received tick and
/// a bitmask indicating whether the most recent 64 ticks were received.
///
/// See also [`EntityReplicated`].
#[derive(Component)]
pub struct ConfirmHistory {
    /// Previously confirmed ticks, including the last tick at position 0.
    mask: u64,

    /// The last received server tick for an entity.
    last_tick: RepliconTick,
}

impl Debug for ConfirmHistory {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ConfirmHistory [{:?} {:b}]", self.last_tick, self.mask)
    }
}

impl ConfirmHistory {
    /// Creates a new instance with a single confirmed tick.
    pub fn new(last_tick: RepliconTick) -> Self {
        Self { mask: 1, last_tick }
    }

    /// Returns the last received tick for an entity.
    pub fn last_tick(&self) -> RepliconTick {
        self.last_tick
    }

    /// Returns a mask that represents the received ticks.
    pub fn mask(&self) -> u64 {
        self.mask
    }

    /// Returns `true` if this tick is confirmed for an entity.
    ///
    /// All ticks older then 64 ticks since [`Self::last_tick`] are considered received.
    pub fn contains(&self, tick: RepliconTick) -> bool {
        if tick > self.last_tick {
            return false;
        }

        let ago = self.last_tick - tick;
        ago >= u64::BITS || ((self.mask >> ago) & 1) == 1
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
        if start_tick <= self.last_tick - u64::BITS {
            return true;
        }

        let end_tick = if end_tick < self.last_tick {
            end_tick
        } else {
            self.last_tick
        };

        let len = end_tick - start_tick + 1; // +1 because the range is inclusive.
        let range = (1 << len) - 1; // Shift 1 to `len` and then decrement to get `len` of 1's.
        let offset = self.last_tick - end_tick;
        let mask = range << offset;

        self.mask & mask != 0
    }

    /// Confirms a tick.
    ///
    /// Useful for unit tests.
    pub fn confirm(&mut self, tick: RepliconTick) {
        if tick > self.last_tick {
            self.set_last_tick(tick);
        } else {
            let ago = self.last_tick - tick;
            if ago < u64::BITS {
                self.set(ago);
            }
        }
    }

    /// Marks previous tick as received.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` are enabled and
    /// `ago` is bigger then [`u64::BITS`].
    pub(super) fn set(&mut self, ago: u32) {
        debug_assert!(ago < u64::BITS);
        self.mask |= 1 << ago;
    }

    /// Sets the last received tick and shifts the mask.
    ///
    /// # Panics
    ///
    /// Panics if `debug_assertions` are enabled and
    /// `tick` is less then the last tick.
    pub(super) fn set_last_tick(&mut self, tick: RepliconTick) {
        debug_assert!(tick >= self.last_tick);
        let diff = tick - self.last_tick;
        self.mask = self.mask.wrapping_shl(diff);
        self.last_tick = tick;
        self.mask |= 1;
    }
}

/// A message that indicates that an entity received update for a tick.
///
/// See also [`ConfirmHistory`].
#[derive(Message, Debug, Clone, Copy)]
pub struct EntityReplicated {
    /// Entity that received an update.
    pub entity: Entity,

    /// Message tick.
    pub tick: RepliconTick,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains() {
        let history = ConfirmHistory::new(RepliconTick::new(1));

        assert!(!history.contains(RepliconTick::new(0)));
        assert!(history.contains(RepliconTick::new(1)));
        assert!(!history.contains(RepliconTick::new(2)));
        assert!(!history.contains(RepliconTick::new(u32::MAX)));
    }

    #[test]
    fn contains_with_wrapping() {
        let history = ConfirmHistory::new(RepliconTick::new(u64::BITS + 1));

        assert!(history.contains(RepliconTick::new(0)));
        assert!(history.contains(RepliconTick::new(1)));
        assert!(!history.contains(RepliconTick::new(2)));

        assert!(!history.contains(RepliconTick::new(u64::BITS)));
        assert!(history.contains(RepliconTick::new(u64::BITS + 1)));
        assert!(!history.contains(RepliconTick::new(u64::BITS + 2)));
    }

    #[test]
    fn contains_any() {
        let history = ConfirmHistory::new(RepliconTick::new(1));

        assert!(history.contains_any(RepliconTick::new(0), RepliconTick::new(1)));
        assert!(history.contains_any(RepliconTick::new(1), RepliconTick::new(1)));
        assert!(history.contains_any(RepliconTick::new(1), RepliconTick::new(2)));
        assert!(!history.contains_any(RepliconTick::new(2), RepliconTick::new(3)));
        assert!(!history.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(0)));
        assert!(history.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(1)));
        assert!(history.contains_any(RepliconTick::new(0), RepliconTick::new(2)));
        assert!(history.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(3)));
    }

    #[test]
    fn contains_any_with_wrapping() {
        let history = ConfirmHistory::new(RepliconTick::new(u64::BITS + 1));

        assert!(history.contains_any(RepliconTick::new(0), RepliconTick::new(1)));
        assert!(history.contains_any(RepliconTick::new(1), RepliconTick::new(2)));
        assert!(!history.contains_any(RepliconTick::new(2), RepliconTick::new(3)));
        assert!(history.contains_any(RepliconTick::new(0), RepliconTick::new(3)));

        assert!(history.contains_any(
            RepliconTick::new(u64::BITS),
            RepliconTick::new(u64::BITS + 1)
        ));
        assert!(history.contains_any(
            RepliconTick::new(u64::BITS + 1),
            RepliconTick::new(u64::BITS + 2)
        ));
        assert!(!history.contains_any(
            RepliconTick::new(u64::BITS + 2),
            RepliconTick::new(u64::BITS + 3)
        ));
        assert!(history.contains_any(
            RepliconTick::new(u64::BITS),
            RepliconTick::new(u64::BITS + 3)
        ));
    }

    #[test]
    fn contains_any_with_older() {
        let mut history = ConfirmHistory::new(RepliconTick::new(1));
        assert_eq!(history.mask(), 0b1);

        history.confirm(RepliconTick::new(u32::MAX));
        assert_eq!(history.mask(), 0b101);

        assert!(history.contains_any(RepliconTick::new(0), RepliconTick::new(1)));
        assert!(history.contains_any(RepliconTick::new(1), RepliconTick::new(2)));
        assert!(!history.contains_any(RepliconTick::new(2), RepliconTick::new(3)));
        assert!(history.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(0)));
        assert!(history.contains_any(RepliconTick::new(0), RepliconTick::new(2)));
        assert!(history.contains_any(RepliconTick::new(u32::MAX), RepliconTick::new(3)));
        assert!(history.contains_any(RepliconTick::new(u32::MAX - 1), RepliconTick::new(u32::MAX)));
        assert!(!history.contains_any(
            RepliconTick::new(u32::MAX - 2),
            RepliconTick::new(u32::MAX - 1)
        ));
    }

    #[test]
    fn confirm_newer() {
        let mut history = ConfirmHistory::new(RepliconTick::new(1));
        history.confirm(RepliconTick::new(2));
        assert_eq!(history.mask(), 0b11);

        assert!(!history.contains(RepliconTick::new(0)));
        assert!(history.contains(RepliconTick::new(1)));
        assert!(history.contains(RepliconTick::new(2)));
    }

    #[test]
    fn confirm_older() {
        let mut history = ConfirmHistory::new(RepliconTick::new(1));
        history.confirm(RepliconTick::new(0));
        assert_eq!(history.mask(), 0b11);

        assert!(history.contains(RepliconTick::new(0)));
        assert!(history.contains(RepliconTick::new(1)));
        assert!(!history.contains(RepliconTick::new(2)));
    }

    #[test]
    fn confirm_same() {
        let mut history = ConfirmHistory::new(RepliconTick::new(1));
        history.confirm(RepliconTick::new(1));
        assert_eq!(history.mask(), 0b1);

        assert!(!history.contains(RepliconTick::new(0)));
        assert!(history.contains(RepliconTick::new(1)));
        assert!(!history.contains(RepliconTick::new(2)));
    }

    #[test]
    fn confirm_with_wrapping() {
        let mut history = ConfirmHistory::new(RepliconTick::new(1));
        history.confirm(RepliconTick::new(u64::BITS + 1));
        assert_eq!(history.mask(), 0b1);

        assert!(history.contains(RepliconTick::new(0)));
        assert!(history.contains(RepliconTick::new(1)));
        assert!(!history.contains(RepliconTick::new(2)));
        assert!(!history.contains(RepliconTick::new(u64::BITS)));
        assert!(history.contains(RepliconTick::new(u64::BITS + 1)));
        assert!(!history.contains(RepliconTick::new(u64::BITS + 2)));
    }

    #[test]
    fn confirm_with_overflow() {
        let mut history = ConfirmHistory::new(RepliconTick::new(u32::MAX));
        history.confirm(RepliconTick::new(1));
        assert_eq!(history.mask(), 0b101);

        assert!(!history.contains(RepliconTick::new(0)));
        assert!(history.contains(RepliconTick::new(1)));
        assert!(!history.contains(RepliconTick::new(3)));
        assert!(history.contains(RepliconTick::new(u32::MAX)));
    }
}
