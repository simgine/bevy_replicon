use core::any::TypeId;

use bevy::{
    ecs::entity::{EntityHashMap, EntityHashSet},
    platform::{
        collections::{HashSet, hash_map::Entry},
        hash::NoOpHash,
    },
    prelude::*,
};
use log::trace;

use crate::prelude::*;

/// Cached visibility information for a client.
///
/// Automatically updated by observers from [`AppVisibilityExt`].
#[derive(Component, Default)]
pub struct ClientVisibility {
    /// List of hidden entities.
    hidden: EntityHashMap<HashSet<TypeId, NoOpHash>>,

    /// All entities that lost visibility in this tick.
    lost: EntityHashSet,

    /// IDs removed from [`Self::hidden`].
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    ids_buffer: Vec<HashSet<TypeId, NoOpHash>>,
}

impl ClientVisibility {
    /// Clears all lost entities during this tick, returning them as an iterator.
    pub(crate) fn drain_lost(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.lost.drain()
    }

    /// Removes a despawned entity tracked by this client.
    ///
    /// Since observers can't be ordered, we can't distinguish between
    /// a despawn and removal of a visibility filter. As a workaround, we record
    /// all changes and remove all despawned entities when processing despawns
    /// during replication.
    pub(crate) fn remove_despawned(&mut self, entity: Entity) {
        self.hidden.remove(&entity);
        self.lost.remove(&entity);
    }

    /// Sets visibility of an entity for a filter.
    pub(super) fn set_visibility<F: VisibilityFilter>(&mut self, entity: Entity, visible: bool) {
        trace!(
            "setting `{visible}` from filter `{}` for `{entity}`",
            ShortName::of::<F>()
        );
        if visible {
            if let Entry::Occupied(mut components) = self.hidden.entry(entity) {
                components.get_mut().remove(&TypeId::of::<F>());
                if components.get().is_empty() {
                    self.lost.remove(&entity);
                    self.ids_buffer.push(components.remove());
                }
            }
        } else {
            let components = self
                .hidden
                .entry(entity)
                .or_insert_with(|| self.ids_buffer.pop().unwrap_or_default());

            if components.is_empty() {
                self.lost.insert(entity);
            }
            components.insert(TypeId::of::<F>());
        }
    }

    /// Returns `true` if an entity is visible for this client.
    pub fn is_visible(&self, entity: Entity) -> bool {
        !self.hidden.contains_key(&entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single() {
        let mut visibility = ClientVisibility::default();

        visibility.set_visibility::<A>(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_some());

        visibility.set_visibility::<A>(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_none());
    }

    #[test]
    fn multiple() {
        let mut visibility = ClientVisibility::default();

        visibility.set_visibility::<A>(Entity::PLACEHOLDER, false);
        visibility.set_visibility::<B>(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_some());

        visibility.set_visibility::<A>(Entity::PLACEHOLDER, true);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_some());

        visibility.set_visibility::<B>(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_none());
    }

    #[test]
    fn already_visible() {
        let mut visibility = ClientVisibility::default();
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        visibility.set_visibility::<A>(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_none());
    }

    #[derive(Component)]
    #[component(immutable)]
    struct A;

    impl VisibilityFilter for A {
        fn is_visible(&self, _entity_filter: &Self) -> bool {
            unimplemented!()
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct B;

    impl VisibilityFilter for B {
        fn is_visible(&self, _entity_filter: &Self) -> bool {
            unimplemented!()
        }
    }
}
