use bevy::{
    ecs::entity::{EntityHashMap, EntityHashSet},
    platform::collections::hash_map::Entry,
    prelude::*,
};

/// Cached visibility information for a client.
///
/// Automatically updated by observers from [`AppVisibilityExt`](super::AppVisibilityExt).
#[derive(Component, Default)]
pub struct ClientVisibility {
    /// List of hidden entities and the filters that block their visibility.
    ///
    /// Dilters are stored as a bitmask, with bit indices assigned by the
    /// [`FilterRegistry`](super::registry::FilterRegistry).
    hidden: EntityHashMap<u32>,

    /// All entities that lost visibility in this tick.
    lost: EntityHashSet,
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
    pub(super) fn set_visibility(&mut self, entity: Entity, filter_bit: u8, visible: bool) {
        if visible {
            if let Entry::Occupied(mut filters) = self.hidden.entry(entity) {
                *filters.get_mut() &= !(1 << filter_bit);
                if *filters.get() == 0 {
                    filters.remove();
                    self.lost.remove(&entity);
                }
            }
        } else {
            let filters = self.hidden.entry(entity).or_default();

            if *filters == 0 {
                self.lost.insert(entity);
            }
            *filters |= 1 << filter_bit;
        }
    }

    /// Returns `true` if the entity is hidden from this client.
    pub fn is_hidden(&self, entity: Entity) -> bool {
        self.hidden.contains_key(&entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single() {
        let mut visibility = ClientVisibility::default();

        visibility.set_visibility(Entity::PLACEHOLDER, 0, false);
        assert!(visibility.is_hidden(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_some());

        visibility.set_visibility(Entity::PLACEHOLDER, 0, true);
        assert!(!visibility.is_hidden(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_none());
    }

    #[test]
    fn multiple() {
        let mut visibility = ClientVisibility::default();

        visibility.set_visibility(Entity::PLACEHOLDER, 0, false);
        visibility.set_visibility(Entity::PLACEHOLDER, 1, false);
        assert!(visibility.is_hidden(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_some());

        visibility.set_visibility(Entity::PLACEHOLDER, 0, true);
        assert!(visibility.is_hidden(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_some());

        visibility.set_visibility(Entity::PLACEHOLDER, 1, true);
        assert!(!visibility.is_hidden(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_none());
    }

    #[test]
    fn already_visible() {
        let mut visibility = ClientVisibility::default();
        assert!(!visibility.is_hidden(Entity::PLACEHOLDER));

        visibility.set_visibility(Entity::PLACEHOLDER, 0, true);
        assert!(!visibility.is_hidden(Entity::PLACEHOLDER));
        assert!(visibility.lost.get(&Entity::PLACEHOLDER).is_none());
    }
}
