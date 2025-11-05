use core::ops::BitOrAssign;

use smallbitvec::SmallBitVec;

use crate::shared::replication::registry::ComponentIndex;

/// Wraps a bitvec to provide a dynamically growing bitmask.
///
/// Each bit corresponds to a [`ComponentIndex`](crate::shared::replication::registry::ComponentIndex).
#[derive(Default, Debug)]
pub(crate) struct ComponentMask {
    bits: SmallBitVec,
}

impl ComponentMask {
    pub(crate) fn get(&self, index: ComponentIndex) -> bool {
        self.bits.get(index.0).unwrap_or(false)
    }

    pub(crate) fn set(&mut self, index: ComponentIndex, value: bool) {
        if index.0 >= self.bits.len() {
            self.bits.resize(index.0 + 1, false);
        }
        self.bits.set(index.0, value);
    }

    pub(crate) fn is_heap(&self) -> bool {
        self.bits.heap_ptr().is_some()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bits.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.bits.clear();
    }
}

impl BitOrAssign<&ComponentMask> for ComponentMask {
    #[inline]
    fn bitor_assign(&mut self, rhs: &ComponentMask) {
        if self.bits.len() < rhs.bits.len() {
            self.bits.resize(rhs.bits.len(), false);
        }

        for index in 0..self.bits.len().min(rhs.bits.len()) {
            // SAFETY: index is correct.
            unsafe {
                let value = self.bits.get_unchecked(index) | rhs.bits.get_unchecked(index);
                self.bits.set_unchecked(index, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use smallbitvec::sbvec;
    use test_log::test;

    use super::*;

    #[test]
    fn set_get() {
        let mut mask = ComponentMask {
            bits: sbvec![false; 3],
        };

        mask.set(ComponentIndex(0), true);
        mask.set(ComponentIndex(2), true);
        mask.set(ComponentIndex(10), true);

        assert!(mask.get(ComponentIndex(0)));
        assert!(!mask.get(ComponentIndex(1)));
        assert!(mask.get(ComponentIndex(2)));
        assert!(mask.get(ComponentIndex(10)));
        assert!(!mask.get(ComponentIndex(100)));

        mask.set(ComponentIndex(2), false);
        assert!(!mask.get(ComponentIndex(2)));
    }

    #[test]
    fn bitor_assign() {
        let mut a = ComponentMask {
            bits: sbvec![true, false, true],
        };
        let b = ComponentMask {
            bits: sbvec![false, true, false, true],
        };

        a |= &b;

        assert_eq!(a.bits, sbvec![true; 4]);
    }
}
