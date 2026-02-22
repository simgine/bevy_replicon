use bevy::{
    ecs::{component::ComponentId, entity::hash_map::EntityHashMap},
    prelude::*,
};
use log::trace;

use crate::{
    server::replicated_archetypes::ReplicatedArchetype,
    shared::replication::registry::{ComponentIndex, FnsId, ReplicationRegistry},
};

/// Buffer with removed components for the current tick.
#[derive(Resource, Deref, Default)]
pub(super) struct RemovalBuffer {
    /// Component removals grouped by entity.
    #[deref]
    removals: EntityHashMap<Vec<(ComponentIndex, FnsId)>>,

    /// [`Vec`]s from removals.
    ///
    /// All data is cleared before the insertion.
    /// Stored to reuse allocated capacity.
    pool: Vec<Vec<(ComponentIndex, FnsId)>>,
}

impl RemovalBuffer {
    pub(super) fn insert(
        &mut self,
        entity: Entity,
        components: &[ComponentId],
        archetype: &ReplicatedArchetype,
        registry: &ReplicationRegistry,
    ) {
        let entity_removals = self
            .removals
            .entry(entity)
            .or_insert_with(|| self.pool.pop().unwrap_or_default());

        for &id in components {
            let Some(rule) = archetype.find_rule(id) else {
                trace!("skipping non-replicated `{id:?}` removal for `{entity}`");
                continue;
            };

            let (component_index, ..) = registry.get(rule.fns_id);
            trace!("buffering `{:?}` removal for `{entity}`", rule.fns_id);
            entity_removals.push((component_index, rule.fns_id));
        }
    }

    /// Clears all removals.
    ///
    /// Keeps the allocated memory for reuse.
    pub(super) fn clear(&mut self) {
        self.pool
            .extend(self.removals.drain().map(|(_, mut components)| {
                components.clear();
                components
            }));
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{prelude::*, shared::replication::rules::ReplicationRules};

    #[test]
    fn not_replicated() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .finish();

        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);
        app.update();

        app.world_mut().spawn((Replicated, A)).remove::<A>();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert!(removal_buffer.is_empty());
    }

    #[test]
    fn component() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .replicate::<A>()
            .finish();

        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);
        app.update();

        let entity = app.world_mut().spawn((Replicated, A)).remove::<A>().id();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.len(), 1);

        let removal_ids = removal_buffer.get(&entity).unwrap();
        assert_eq!(removal_ids.len(), 1);
    }

    #[test]
    fn bundle() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .replicate_bundle::<(A, B)>()
            .finish();

        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);
        app.update();

        let entity = app
            .world_mut()
            .spawn((Replicated, A, B))
            .remove::<(A, B)>()
            .id();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.len(), 1);

        let removal_ids = removal_buffer.get(&entity).unwrap();
        assert_eq!(removal_ids.len(), 2);
    }

    #[test]
    fn part_of_bundle() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .replicate_bundle::<(A, B)>()
            .finish();

        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);
        app.update();

        let entity = app.world_mut().spawn((Replicated, A, B)).remove::<A>().id();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.len(), 1);

        let removal_ids = removal_buffer.get(&entity).unwrap();
        assert_eq!(removal_ids.len(), 1);
    }

    #[test]
    fn bundle_with_subset() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .replicate::<A>()
            .replicate_bundle::<(A, B)>()
            .finish();

        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);
        app.update();

        let entity = app
            .world_mut()
            .spawn((Replicated, A, B))
            .remove::<(A, B)>()
            .id();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.len(), 1);

        let removal_ids = removal_buffer.get(&entity).unwrap();
        assert_eq!(removal_ids.len(), 2);
    }

    #[test]
    fn part_of_bundle_with_subset() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .replicate::<A>()
            .replicate_bundle::<(A, B)>()
            .finish();

        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);
        app.update();

        let entity = app.world_mut().spawn((Replicated, A, B)).remove::<A>().id();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert_eq!(removal_buffer.len(), 1);

        let removal_ids = removal_buffer.get(&entity).unwrap();
        let [(_, fns_id)] = removal_ids.as_slice().try_into().unwrap();

        let rules = app.world().resource::<ReplicationRules>();
        let bundle_rule = rules.iter().find(|r| r.components.len() == 2).unwrap();
        assert!(
            bundle_rule.components.iter().any(|r| r.fns_id == fns_id),
            "removal should be long to the bundle"
        );
    }

    #[test]
    fn despawn() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .replicate::<A>()
            .finish();

        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);
        app.update();

        app.world_mut().spawn((Replicated, A)).despawn();

        let removal_buffer = app.world().resource::<RemovalBuffer>();
        assert!(
            removal_buffer.is_empty(),
            "despawns shouldn't be counted as removals"
        );
    }

    #[derive(Component, Serialize, Deserialize)]
    struct A;

    #[derive(Component, Serialize, Deserialize)]
    struct B;
}
