use core::ops::{Add, Sub};

use serde::{Deserialize, Serialize};

/// Monotonic index assigned to a sent diff batch.
///
/// All operations on it are wrapping.
#[derive(Debug, Default, Serialize, Deserialize, Eq, PartialEq, Hash, Clone, Copy)]
pub struct PatchIndex(#[serde(with = "postcard::fixint::le")] u16);

impl PatchIndex {
    /// The maximum wrapping distance at which an index is considered newer.
    pub const MAX_NEWER_DISTANCE: u16 = u16::MAX / 2;

    #[inline]
    pub fn new(value: u16) -> Self {
        Self(value)
    }

    #[inline]
    pub fn get(self) -> u16 {
        self.0
    }

    /// Returns `true` if `self` is newer than `other`.
    ///
    /// The value is considered newer if it is ahead of the other value
    /// by less than [`PatchIndex::MAX_NEWER_DISTANCE`].
    pub fn is_newer_than(self, other: Self) -> bool {
        let distance = self.distance_after(other);
        distance != 0 && distance <= Self::MAX_NEWER_DISTANCE
    }

    /// Returns the wrapping distance from `base` to `self`.
    #[inline]
    pub fn distance_after(self, base: Self) -> u16 {
        self.0.wrapping_sub(base.0)
    }
}

impl Add<u16> for PatchIndex {
    type Output = Self;

    #[inline]
    fn add(self, rhs: u16) -> Self::Output {
        Self(self.0.wrapping_add(rhs))
    }
}

impl Sub<u16> for PatchIndex {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: u16) -> Self::Output {
        Self(self.0.wrapping_sub(rhs))
    }
}

impl Sub for PatchIndex {
    type Output = u16;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        self.0.wrapping_sub(rhs.0)
    }
}
