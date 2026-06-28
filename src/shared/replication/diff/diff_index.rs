use core::{
    cmp::Ordering,
    ops::{Add, AddAssign, Sub},
};

use serde::{Deserialize, Serialize};

/// Monotonic index assigned to a diff.
///
/// All operations on it are wrapping.
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq, Hash, Clone, Copy)]
pub struct DiffIndex(#[serde(with = "postcard::fixint::le")] u16);

impl DiffIndex {
    /// The maximum wrapping distance at which an index is considered newer.
    pub const MAX_NEWER_DISTANCE: u16 = u16::MAX / 2;

    /// Creates a new instance wrapping the given value.
    #[inline]
    pub fn new(value: u16) -> Self {
        Self(value)
    }

    /// Gets the value of this tick.
    #[inline]
    pub fn get(self) -> u16 {
        self.0
    }

    /// Compares indices using wrapping semantics.
    ///
    /// This comparison is only meaningful when the indices are at most
    /// [`Self::MAX_NEWER_DISTANCE`] apart.
    ///
    /// We don't implement [`Ord`] because wrapping ordering is not transitive.
    pub fn wrapping_cmp(self, other: Self) -> Ordering {
        let distance = self.distance_after(other);
        if distance == 0 {
            Ordering::Equal
        } else if distance <= Self::MAX_NEWER_DISTANCE {
            Ordering::Greater
        } else {
            Ordering::Less
        }
    }

    /// Deprecated alias for [`Self::is_newer`].
    #[deprecated = "use `Self::is_newer`"]
    pub fn is_newer_than(self, other: Self) -> bool {
        self.is_newer(other)
    }

    /// Tests if `self` is greater than `other` using [`Self::wrapping_cmp`].
    pub fn is_newer(self, other: Self) -> bool {
        self.wrapping_cmp(other).is_gt()
    }

    /// Returns the wrapping distance from `base` to `self`.
    #[inline]
    pub fn distance_after(self, base: Self) -> u16 {
        self.0.wrapping_sub(base.0)
    }
}

impl Add<u16> for DiffIndex {
    type Output = Self;

    #[inline]
    fn add(self, rhs: u16) -> Self::Output {
        Self(self.0.wrapping_add(rhs))
    }
}

impl AddAssign<u16> for DiffIndex {
    fn add_assign(&mut self, rhs: u16) {
        *self = *self + rhs;
    }
}

impl Sub<u16> for DiffIndex {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: u16) -> Self::Output {
        Self(self.0.wrapping_sub(rhs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison() {
        assert_eq!(DiffIndex::new(0), DiffIndex::new(0));
        assert!(!DiffIndex::new(0).is_newer(DiffIndex::new(0)));
        assert!(DiffIndex::new(1).is_newer(DiffIndex::new(0)));
        assert!(!DiffIndex::new(u16::MAX).is_newer(DiffIndex::new(0)));
    }
}
