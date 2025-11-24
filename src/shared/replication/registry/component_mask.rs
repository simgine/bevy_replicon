use core::ops::BitOrAssign;

use smallbitvec::SmallBitVec;

use super::ComponentIndex;

/// Wraps a bitvec to provide a dynamically growing bitmask for compactly storing component IDs.
#[derive(Default, Debug, Clone)]
pub struct ComponentMask {
    /// Each bit corresponds to a [`ComponentIndex`].
    bits: SmallBitVec,
}

impl ComponentMask {
    pub(crate) fn contains(&self, index: ComponentIndex) -> bool {
        self.bits.get(index.0).unwrap_or(false)
    }

    pub(crate) fn insert(&mut self, index: ComponentIndex) {
        if index.0 >= self.bits.len() {
            self.bits.resize(index.0 + 1, false);
        }
        self.bits.set(index.0, true);
    }

    pub(crate) fn remove(&mut self, index: ComponentIndex) {
        self.bits.set(index.0, false);
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

    pub(crate) fn iter(&self) -> impl Iterator<Item = ComponentIndex> {
        self.bits
            .iter()
            .enumerate()
            .filter_map(|(index, value)| value.then_some(ComponentIndex(index)))
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
    fn insert_remove() {
        let mut mask = ComponentMask {
            bits: sbvec![false; 3],
        };

        mask.insert(ComponentIndex(0));
        mask.insert(ComponentIndex(2));
        mask.insert(ComponentIndex(10));

        assert!(mask.contains(ComponentIndex(0)));
        assert!(!mask.contains(ComponentIndex(1)));
        assert!(mask.contains(ComponentIndex(2)));
        assert!(mask.contains(ComponentIndex(10)));
        assert!(!mask.contains(ComponentIndex(100)));

        mask.remove(ComponentIndex(2));
        assert!(!mask.contains(ComponentIndex(2)));
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
