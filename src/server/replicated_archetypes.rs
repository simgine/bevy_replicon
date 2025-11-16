use core::mem;

use bevy::{
    ecs::{
        archetype::{ArchetypeGeneration, ArchetypeId, Archetypes},
        component::{ComponentId, StorageType},
    },
    platform::collections::HashMap,
    prelude::*,
};
use log::trace;

use crate::{
    prelude::*,
    shared::replication::rules::{ReplicationRules, component::ComponentRule},
};

#[derive(Resource)]
pub(super) struct ReplicatedArchetypes {
    /// ID of [`Replicated`] component.
    marker_id: ComponentId,

    /// Highest processed archetype ID.
    generation: ArchetypeGeneration,

    /// Maps a Bevy archetype ID to an index in [`Self::list`].
    ids_map: HashMap<ArchetypeId, usize>,

    /// Archetypes marked as replicated.
    list: Vec<ReplicatedArchetype>,
}

impl ReplicatedArchetypes {
    pub(super) fn update(&mut self, archetypes: &Archetypes, rules: &ReplicationRules) {
        let old_generation = mem::replace(&mut self.generation, archetypes.generation());

        for archetype in archetypes[old_generation..]
            .iter()
            .filter(|archetype| archetype.contains(self.marker_id))
        {
            trace!("marking `{:?}` as replicated", archetype.id());
            let mut replicated_archetype = ReplicatedArchetype::new(archetype.id());
            for rule in rules.iter().filter(|rule| rule.matches(archetype)) {
                for &component in &rule.components {
                    // Since rules are sorted by priority,
                    // we are inserting only new components that aren't present.
                    if replicated_archetype
                        .components
                        .iter()
                        .any(|(existing, _)| existing.id == component.id)
                    {
                        continue;
                    }

                    // SAFETY: archetype matches the rule, so the component is present.
                    let storage =
                        unsafe { archetype.get_storage_type(component.id).unwrap_unchecked() };
                    replicated_archetype.components.push((component, storage));
                }
            }

            self.ids_map.insert(archetype.id(), self.list.len());
            self.list.push(replicated_archetype);
        }
    }

    pub(super) fn marker_id(&self) -> ComponentId {
        self.marker_id
    }

    pub(super) fn get(&self, id: ArchetypeId) -> Option<&ReplicatedArchetype> {
        let index = *self.ids_map.get(&id)?;
        self.list.get(index)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &ReplicatedArchetype> {
        self.list.iter()
    }
}

impl FromWorld for ReplicatedArchetypes {
    fn from_world(world: &mut World) -> Self {
        Self {
            marker_id: world.register_component::<Replicated>(),
            generation: ArchetypeGeneration::initial(),
            ids_map: Default::default(),
            list: Default::default(),
        }
    }
}

/// An archetype that can be stored in [`ReplicatedArchetypes`].
pub(super) struct ReplicatedArchetype {
    /// Associated archetype ID.
    pub(super) id: ArchetypeId,

    /// Components marked as replicated.
    pub(super) components: Vec<(ComponentRule, StorageType)>,
}

impl ReplicatedArchetype {
    fn new(id: ArchetypeId) -> Self {
        Self {
            id,
            components: Default::default(),
        }
    }

    pub(super) fn find_rule(&self, id: ComponentId) -> Option<&ComponentRule> {
        self.components.iter().map(|(r, _)| r).find(|r| r.id == id)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use test_log::test;

    use super::*;
    use crate::shared::replication::registry::ReplicationRegistry;

    #[test]
    fn empty() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>();

        app.world_mut().spawn_empty();
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert!(archetypes.list.is_empty());
    }

    #[test]
    fn no_components() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>();

        app.world_mut().spawn(Replicated);
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.list.len(), 1);
        let archetype = archetypes.list.first().unwrap();
        assert!(archetype.components.is_empty());
    }

    #[test]
    fn component() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<A>();

        app.world_mut().spawn((Replicated, A));
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.list.len(), 1);
        let archetype = archetypes.list.first().unwrap();
        assert_eq!(archetype.components.len(), 1);
    }

    #[test]
    fn bundle() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .replicate_bundle::<(A, B)>();

        app.world_mut().spawn((Replicated, A, B));
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.list.len(), 1);
        let archetype = archetypes.list.first().unwrap();
        assert_eq!(archetype.components.len(), 2);
    }

    #[test]
    fn part_of_bundle() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .replicate_bundle::<(A, B)>();

        app.world_mut().spawn((Replicated, A));
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.list.len(), 1);
        let archetype = archetypes.list.first().unwrap();
        assert!(archetype.components.is_empty());
    }

    #[test]
    fn bundle_with_subset() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<A>()
            .replicate_bundle::<(A, B)>();

        app.world_mut().spawn((Replicated, A, B));
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.list.len(), 1);
        let archetype = archetypes.list.first().unwrap();
        assert_eq!(archetype.components.len(), 2);
    }

    #[test]
    fn bundle_with_multiple_subsets() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<A>()
            .replicate::<B>()
            .replicate_bundle::<(A, B)>();

        app.world_mut().spawn((Replicated, A, B));
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.list.len(), 1);
        let archetype = archetypes.list.first().unwrap();
        assert_eq!(archetype.components.len(), 2);
    }

    #[test]
    fn bundles_with_overlap() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .replicate_bundle::<(A, B)>()
            .replicate_bundle::<(A, C)>();

        app.world_mut().spawn((Replicated, A, B, C));
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.list.len(), 1);
        let archetype = archetypes.list.first().unwrap();
        assert_eq!(archetype.components.len(), 3);
    }

    fn update_archetypes(app: &mut App) {
        app.world_mut()
            .resource_scope(|world, mut archetypes: Mut<ReplicatedArchetypes>| {
                let rules = world.resource::<ReplicationRules>();
                archetypes.update(world.archetypes(), rules);
            });
    }

    #[derive(Component, Serialize, Deserialize)]
    struct A;

    #[derive(Component, Serialize, Deserialize)]
    struct B;

    #[derive(Component, Serialize, Deserialize)]
    struct C;
}
