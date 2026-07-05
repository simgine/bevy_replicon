use core::{
    cmp::Ordering,
    ops::{Add, AddAssign, Sub, SubAssign},
};

use bevy::prelude::*;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Like [`Tick`](bevy::ecs::change_detection::Tick), but for replication.
///
/// All operations on it are wrapping.
///
/// See also [`ServerUpdateTick`](crate::client::ServerUpdateTick) and
/// [`ServerTick`](crate::server::server_tick::ServerTick).
#[derive(
    Reflect, Debug, Default, Serialize, Deserialize, Eq, Hash, PartialEq, MaxSize, Clone, Copy,
)]
pub struct RepliconTick(u32);

impl RepliconTick {
    /// The maximum wrapping distance at which a tick is considered newer.
    pub const MAX_NEWER_DISTANCE: u32 = u32::MAX / 2;

    /// Creates a new instance wrapping the given value.
    #[inline]
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    /// Gets the value of this tick.
    #[inline]
    pub fn get(self) -> u32 {
        self.0
    }

    /// Compares ticks using wrapping semantics.
    ///
    /// This comparison is only meaningful when the ticks are at most
    /// [`Self::MAX_NEWER_DISTANCE`] apart.
    ///
    /// We don't implement [`Ord`] because wrapping ordering is not transitive.
    pub fn wrapping_cmp(self, other: Self) -> Ordering {
        let distance = self.0.wrapping_sub(other.0);
        if distance == 0 {
            Ordering::Equal
        } else if distance <= Self::MAX_NEWER_DISTANCE {
            Ordering::Greater
        } else {
            Ordering::Less
        }
    }

    /// Tests if `self` is greater or equal than `other` using [`Self::wrapping_cmp`].
    pub fn is_newer_or_eq(self, other: Self) -> bool {
        self.wrapping_cmp(other).is_ge()
    }

    /// Tests if `self` is greater than `other` using [`Self::wrapping_cmp`].
    pub fn is_newer(self, other: Self) -> bool {
        self.wrapping_cmp(other).is_gt()
    }

    /// Tests if `self` is less than `other` using [`Self::wrapping_cmp`].
    pub fn is_older(self, other: Self) -> bool {
        self.wrapping_cmp(other).is_lt()
    }

    /// Tests if `self` is less or equal than `other` using [`Self::wrapping_cmp`].
    pub fn is_older_or_eq(self, other: Self) -> bool {
        self.wrapping_cmp(other).is_le()
    }
}

impl Add<u32> for RepliconTick {
    type Output = Self;

    fn add(self, rhs: u32) -> Self::Output {
        Self(self.0.wrapping_add(rhs))
    }
}

impl AddAssign<u32> for RepliconTick {
    fn add_assign(&mut self, rhs: u32) {
        self.0 = self.0.wrapping_add(rhs)
    }
}

impl Sub for RepliconTick {
    type Output = u32;

    fn sub(self, rhs: Self) -> Self::Output {
        self.0.wrapping_sub(rhs.0)
    }
}

impl Sub<u32> for RepliconTick {
    type Output = Self;

    fn sub(self, rhs: u32) -> Self::Output {
        Self(self.0.wrapping_sub(rhs))
    }
}

impl SubAssign<u32> for RepliconTick {
    fn sub_assign(&mut self, rhs: u32) {
        self.0 = self.0.wrapping_sub(rhs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_comparison() {
        assert_eq!(RepliconTick::new(0), RepliconTick::new(0));
        assert!(RepliconTick::new(0).is_newer_or_eq(RepliconTick::new(0)));
        assert!(RepliconTick::new(0).is_older_or_eq(RepliconTick::new(0)));
        assert!(RepliconTick::new(1).is_newer(RepliconTick::new(0)));
        assert!(RepliconTick::new(1).is_newer_or_eq(RepliconTick::new(0)));
        assert!(RepliconTick::new(u32::MAX).is_older(RepliconTick::new(0)));
        assert!(RepliconTick::new(u32::MAX).is_older_or_eq(RepliconTick::new(0)));
    }
}
