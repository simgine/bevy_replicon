use bevy::{ecs::entity::EntityHashMap, platform::collections::hash_map::Entry, prelude::*};

use super::filters_mask::{FilterBit, FiltersMask};

/// Cached visibility as masks for a client.
///
/// Each bit in the [`FiltersMask`] corresponds to a visibility filter
/// registered in the [`FilterRegistry`](super::registry::FilterRegistry).
/// This allows us to avoid storing the filter data for every client,
/// which would be expensive in memory-wise.
///
/// Stores only entities that have some hidden data.
///
/// Automatically updated by observers from [`AppVisibilityExt`](super::AppVisibilityExt).
#[derive(Component, Default)]
pub(crate) struct ClientVisibility {
    /// Entities with hidden data.
    hidden: EntityHashMap<FiltersMask>,

    /// Entities and bits that became set during this tick.
    ///
    /// Stored redundantly to quickly iterate only over entities
    /// with newly hidden data.
    lost: EntityHashMap<FiltersMask>,
}

impl ClientVisibility {
    /// Returns iterator over all entities that lose any visibility during this tick.
    pub(crate) fn iter_lost(&self) -> impl Iterator<Item = (Entity, FiltersMask)> {
        self.lost.iter().map(|(&e, &m)| (e, m))
    }

    /// Clears all entities that lost any visibility during this tick, returning them as an iterator.
    pub(crate) fn drain_lost(&mut self) -> impl Iterator<Item = (Entity, FiltersMask)> {
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

    /// Sets visibility of an entity for the given filter bit.
    pub(super) fn set(&mut self, entity: Entity, bit: FilterBit, visible: bool) {
        if visible {
            if let Entry::Occupied(mut mask) = self.hidden.entry(entity) {
                mask.get_mut().remove(bit);
                if mask.get().is_empty() {
                    mask.remove();
                }

                if let Entry::Occupied(mut lost_mask) = self.lost.entry(entity) {
                    lost_mask.get_mut().remove(bit);
                    if lost_mask.get().is_empty() {
                        lost_mask.remove();
                    }
                }
            }
        } else {
            let mask = self.hidden.entry(entity).or_default();
            if !mask.contains(bit) {
                mask.insert(bit);
                self.lost.entry(entity).or_default().insert(bit);
            }
        }
    }

    /// Returns bits for all filters that affect visibility of the given entity.
    pub(crate) fn get(&self, entity: Entity) -> FiltersMask {
        self.hidden.get(&entity).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single() {
        let mut visibility = ClientVisibility::default();

        visibility.set(Entity::PLACEHOLDER, FilterBit::new(0), false);
        assert!(!visibility.get(Entity::PLACEHOLDER).is_empty());
        assert!(visibility.lost.contains_key(&Entity::PLACEHOLDER));

        visibility.set(Entity::PLACEHOLDER, FilterBit::new(0), true);
        assert!(visibility.get(Entity::PLACEHOLDER).is_empty());
        assert!(!visibility.lost.contains_key(&Entity::PLACEHOLDER));
    }

    #[test]
    fn multiple() {
        let mut visibility = ClientVisibility::default();

        visibility.set(Entity::PLACEHOLDER, FilterBit::new(0), false);
        visibility.set(Entity::PLACEHOLDER, FilterBit::new(1), false);
        assert!(!visibility.get(Entity::PLACEHOLDER).is_empty());
        assert!(visibility.lost.contains_key(&Entity::PLACEHOLDER));

        visibility.set(Entity::PLACEHOLDER, FilterBit::new(0), true);
        assert!(!visibility.get(Entity::PLACEHOLDER).is_empty());
        assert!(visibility.lost.contains_key(&Entity::PLACEHOLDER));

        visibility.set(Entity::PLACEHOLDER, FilterBit::new(1), true);
        assert!(visibility.get(Entity::PLACEHOLDER).is_empty());
        assert!(!visibility.lost.contains_key(&Entity::PLACEHOLDER));
    }

    #[test]
    fn already_visible() {
        let mut visibility = ClientVisibility::default();
        assert!(visibility.get(Entity::PLACEHOLDER).is_empty());

        visibility.set(Entity::PLACEHOLDER, FilterBit::new(0), true);
        assert!(visibility.get(Entity::PLACEHOLDER).is_empty());
        assert!(!visibility.lost.contains_key(&Entity::PLACEHOLDER));
    }
}
