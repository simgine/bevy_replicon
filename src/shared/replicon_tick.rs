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
}

impl PartialOrd for RepliconTick {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RepliconTick {
    fn cmp(&self, other: &Self) -> Ordering {
        let difference = self.0.wrapping_sub(other.0);
        if difference == 0 {
            Ordering::Equal
        } else if difference > u32::MAX / 2 {
            Ordering::Less
        } else {
            Ordering::Greater
        }
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
        assert!(RepliconTick::new(0) < RepliconTick::new(1));
        assert!(RepliconTick::new(0) > RepliconTick::new(u32::MAX));
    }
}
