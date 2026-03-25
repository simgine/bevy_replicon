use core::iter;

use bevy::prelude::*;

use super::registry::{FilterRegistry, VisibilityScope};
use crate::shared::replication::registry::{ComponentIndex, component_mask::ComponentMask};

/// Bitset of visibility filters for an entity for a client.
///
/// If the bit is set, it means that this filter hides its associated data.
#[derive(Default, Reflect, Debug, PartialEq, Clone, Copy)]
pub(crate) struct FiltersMask(u32);

impl FiltersMask {
    pub(super) fn insert(&mut self, bit: FilterBit) {
        self.0 |= 1 << *bit;
    }

    pub(super) fn remove(&mut self, bit: FilterBit) {
        self.0 &= !(1 << *bit);
    }

    pub(super) fn contains(&self, bit: FilterBit) -> bool {
        (self.0 & (1 << *bit)) != 0
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// Returns an iterator over all set bits, in ascending bit order.
    fn iter(self) -> impl Iterator<Item = FilterBit> {
        let mut mask = self.0;
        iter::from_fn(move || {
            if mask == 0 {
                return None;
            }

            let bit = mask.trailing_zeros() as u8;
            mask &= mask - 1; // Clear the lowest set bit.
            Some(FilterBit::new(bit))
        })
    }

    /// Returns `true` if the entity is hidden by any of the filters.
    pub(crate) fn is_hidden(&self, registry: &FilterRegistry) -> bool {
        self.iter()
            .any(|bit| matches!(registry.scope(bit), VisibilityScope::Entity))
    }

    /// Returns `true` if the given component is hidden by any of the filters.
    ///
    /// Entity filters are treated as hiding all components.
    pub(crate) fn is_component_hidden(
        &self,
        registry: &FilterRegistry,
        index: ComponentIndex,
    ) -> bool {
        self.iter().any(|bit| match registry.scope(bit) {
            VisibilityScope::Entity => true,
            VisibilityScope::Components(component_mask) => component_mask.contains(index),
        })
    }

    /// Returns an iterator over hidden components for an entity.
    pub(crate) fn hidden_components(
        self,
        registry: &FilterRegistry,
    ) -> impl Iterator<Item = &ComponentMask> {
        self.iter().map(|bit| match registry.scope(bit) {
            VisibilityScope::Entity => {
                panic!("if the entity is hidden, iteration over hidden components shouldn't happen")
            }
            VisibilityScope::Components(component_mask) => component_mask,
        })
    }
}

/// Bit that represents a visibility filter from [`FilterRegistry`].
#[derive(Deref, Default, Debug, PartialEq, Clone, Copy)]
pub struct FilterBit(u8);

impl FilterBit {
    /// Creates a new instance for the given bit index.
    ///
    /// Valid values are in the range `[0, 32)` that map directly to bits in [`FiltersMask`].
    pub(super) fn new(value: u8) -> Self {
        debug_assert!(value < 32, "filter bit must be less than {}", u32::BITS);
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn insert_remove() {
        let mut mask = FiltersMask::default();
        assert!(mask.is_empty());

        let bit = FilterBit::new(2);
        mask.insert(bit);
        assert_eq!(mask, FiltersMask(0b100));
        assert!(mask.contains(bit));
        assert!(!mask.is_empty());

        mask.remove(bit);
        assert!(!mask.contains(bit));
        assert!(mask.is_empty());
    }

    #[test]
    fn iter() {
        let mask = FiltersMask(0b10101);
        let indices: Vec<_> = mask.iter().map(|bit| bit.0).collect();
        assert_eq!(indices, [0, 2, 4]);
    }
}
