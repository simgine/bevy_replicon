use bevy::{
    ecs::{
        archetype::Archetype,
        component::ComponentId,
        entity::hash_map::EntityHashMap,
        lifecycle::{RemovedComponentEntity, RemovedComponentMessages},
        message::MessageCursor,
        system::SystemParam,
    },
    platform::collections::{HashMap, HashSet},
    prelude::*,
};

use crate::{
    prelude::*,
    shared::replication::{registry::FnsId, rules::ReplicationRules},
};

/// Reader for removed components.
///
/// Like [`RemovedComponentMessages`], but reads them in per-entity format.
#[derive(SystemParam)]
pub(super) struct RemovalReader<'w, 's> {
    /// Cached components list from [`ReplicationRules`].
    components: Local<'s, ReplicatedComponents>,

    /// Individual readers for each component.
    readers: Local<'s, HashMap<ComponentId, MessageCursor<RemovedComponentEntity>>>,

    /// Component removals grouped by entity.
    removals: Local<'s, EntityHashMap<HashSet<ComponentId>>>,

    /// [`HashSet`]'s from removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    ids_buffer: Local<'s, Vec<HashSet<ComponentId>>>,

    /// Component removals grouped by [`ComponentId`].
    remove_messages: &'w RemovedComponentMessages,

    /// Filter for replicated and valid entities.
    replicated: Query<'w, 's, (), With<Replicated>>,
}

impl RemovalReader<'_, '_> {
    /// Returns iterator over all components removed since the last call.
    ///
    /// Only replicated entities taken into account.
    pub(super) fn read(&mut self) -> impl Iterator<Item = (&Entity, &HashSet<ComponentId>)> {
        self.clear();

        for (&component_id, component_messages) in self
            .remove_messages
            .iter()
            .filter(|(component_id, _)| self.components.contains(*component_id))
        {
            // Removed components are grouped by type, not by entity, so we need an intermediate container.
            let reader = self.readers.entry(component_id).or_default();
            for entity in reader
                .read(component_messages)
                .cloned()
                .map(Into::into)
                .filter(|&entity| self.replicated.get(entity).is_ok())
            {
                self.removals
                    .entry(entity)
                    .or_insert_with(|| self.ids_buffer.pop().unwrap_or_default())
                    .insert(component_id);
            }
        }

        self.removals.iter()
    }

    /// Clears all removals.
    ///
    /// Keeps the allocated memory for reuse.
    fn clear(&mut self) {
        self.ids_buffer
            .extend(self.removals.drain().map(|(_, mut components)| {
                components.clear();
                components
            }));
    }
}

#[derive(Deref)]
struct ReplicatedComponents(HashSet<ComponentId>);

impl FromWorld for ReplicatedComponents {
    fn from_world(world: &mut World) -> Self {
        let rules = world.resource::<ReplicationRules>();
        let component_ids = rules
            .iter()
            .flat_map(|rule| &rule.components)
            .map(|component| component.id)
            .collect();

        Self(component_ids)
    }
}

/// Buffer with removed components.
///
/// Used to avoid missing messages.
#[derive(Default, Resource, Deref)]
pub(super) struct RemovalBuffer {
    /// Component removals grouped by entity.
    #[deref]
    removals: EntityHashMap<Vec<(ComponentId, FnsId)>>,

    /// [`Vec`]s from removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    ids_buffer: Vec<Vec<(ComponentId, FnsId)>>,
}

impl RemovalBuffer {
    /// Registers component removals that match replication rules for an entity.
    pub(super) fn update(
        &mut self,
        rules: &ReplicationRules,
        archetype: &Archetype,
        entity: Entity,
        removed_components: &HashSet<ComponentId>,
    ) {
        let mut removed_ids = self.ids_buffer.pop().unwrap_or_default();
        for rule in rules
            .iter()
            .filter(|rule| rule.matches_removals(archetype, removed_components))
        {
            for component in &rule.components {
                // Since rules are sorted by priority,
                // we are inserting only new components that aren't present.
                if removed_ids.iter().all(|&(id, _)| id != component.id)
                    && removed_components.contains(&component.id)
                {
                    removed_ids.push((component.id, component.fns_id));
                }
            }
        }

        if removed_ids.is_empty() {
            self.ids_buffer.push(removed_ids);
        } else {
            self.removals.insert(entity, removed_ids);
        }
    }

    /// Clears all removals.
    ///
    /// Keeps the allocated memory for reuse.
    pub(super) fn clear(&mut self) {
        self.ids_buffer
            .extend(self.removals.drain().map(|(_, mut components)| {
                components.clear();
                components
            }));
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{server, shared::replication::registry::ReplicationRegistry};

    #[test]
    fn not_replicated() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<RemovalBuffer>()
            .add_systems(PostUpdate, server::buffer_removals);

        app.update();

        app.world_mut().spawn((Replicated, A)).remove::<A>();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert!(removal_buffer.removals.is_empty());
    }

    #[test]
    fn component() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<RemovalBuffer>()
            .add_systems(PostUpdate, server::buffer_removals)
            .replicate::<A>();

        app.update();

        let entity = app.world_mut().spawn((Replicated, A)).remove::<A>().id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removal_ids = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removal_ids.len(), 1);
    }

    #[test]
    fn bundle() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<RemovalBuffer>()
            .add_systems(PostUpdate, server::buffer_removals)
            .replicate_bundle::<(A, B)>();

        app.update();

        let entity = app
            .world_mut()
            .spawn((Replicated, A, B))
            .remove::<(A, B)>()
            .id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removal_ids = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removal_ids.len(), 2);
    }

    #[test]
    fn part_of_bundle() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<RemovalBuffer>()
            .add_systems(PostUpdate, server::buffer_removals)
            .replicate_bundle::<(A, B)>();

        app.update();

        let entity = app.world_mut().spawn((Replicated, A, B)).remove::<A>().id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removal_ids = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removal_ids.len(), 1);
    }

    #[test]
    fn bundle_with_subset() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<RemovalBuffer>()
            .add_systems(PostUpdate, server::buffer_removals)
            .replicate::<A>()
            .replicate_bundle::<(A, B)>();

        app.update();

        let entity = app
            .world_mut()
            .spawn((Replicated, A, B))
            .remove::<(A, B)>()
            .id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removal_ids = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removal_ids.len(), 2);
    }

    #[test]
    fn part_of_bundle_with_subset() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<RemovalBuffer>()
            .add_systems(PostUpdate, server::buffer_removals)
            .replicate::<A>()
            .replicate_bundle::<(A, B)>();

        app.update();

        let entity = app.world_mut().spawn((Replicated, A, B)).remove::<A>().id();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.removals.len(), 1);

        let removal_ids = removal_buffer.removals.get(&entity).unwrap();
        assert_eq!(removal_ids.len(), 1);
    }

    #[test]
    fn despawn() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<RemovalBuffer>()
            .add_systems(PostUpdate, server::buffer_removals)
            .replicate::<A>();

        app.update();

        app.world_mut().spawn((Replicated, A)).despawn();

        app.update();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert!(
            removal_buffer.removals.is_empty(),
            "despawns shouldn't be counted as removals"
        );
    }

    #[derive(Serialize, Deserialize, Component)]
    struct A;

    #[derive(Serialize, Deserialize, Component)]
    struct B;
}
