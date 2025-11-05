use core::ops::Range;

use bevy::prelude::*;

use super::replication_messages::mutations::EntityMutations;
use crate::shared::replication::registry::component_mask::ComponentMask;

/// Pools for various client components to reuse allocated capacity.
///
/// All data is cleared before the insertion.
#[derive(Resource, Default)]
pub(super) struct ClientPools {
    /// Entities with bitvecs for components from
    /// [`MutateInfo`](crate::shared::replication::client_ticks::MutateInfo).
    entities: Vec<Vec<(Entity, ComponentMask)>>,
    /// Bitvecs for components from [`Updates`], [`Mutations`] and
    /// [`MutateInfo`](crate::shared::replication::client_ticks::MutateInfo).
    ///
    /// Only heap-allocated instances are stored.
    components: Vec<ComponentMask>,
    /// Ranges from [`Updates`] and [`Mutations`].
    ranges: Vec<Vec<Range<usize>>>,
    /// Entities from [`Mutations`].
    mutations: Vec<Vec<EntityMutations>>,
}

impl ClientPools {
    pub(super) fn recycle_entities(&mut self, mut entities: Vec<(Entity, ComponentMask)>) {
        for (_, mut components) in entities.drain(..) {
            if components.is_heap() {
                components.clear();
                self.components.push(components);
            }
        }
        self.entities.push(entities);
    }

    pub(super) fn recycle_components(&mut self, mut components: ComponentMask) {
        if components.is_heap() {
            components.clear();
            self.components.push(components);
        }
    }

    pub(super) fn recycle_ranges(&mut self, ranges: impl Iterator<Item = Vec<Range<usize>>>) {
        self.ranges.extend(ranges.map(|mut ranges| {
            ranges.clear();
            ranges
        }));
    }

    pub(super) fn recycle_mutations(
        &mut self,
        mutations: impl Iterator<Item = Vec<EntityMutations>>,
    ) {
        self.mutations.extend(mutations.map(|mut mutations| {
            mutations.clear();
            mutations
        }));
    }

    pub(super) fn take_entities(&mut self) -> Vec<(Entity, ComponentMask)> {
        self.entities.pop().unwrap_or_default()
    }

    pub(super) fn take_components(&mut self) -> ComponentMask {
        self.components.pop().unwrap_or_default()
    }

    pub(super) fn take_ranges(&mut self) -> Vec<Range<usize>> {
        self.ranges.pop().unwrap_or_default()
    }

    pub(super) fn take_mutations(&mut self) -> Vec<EntityMutations> {
        self.mutations.pop().unwrap_or_default()
    }
}
