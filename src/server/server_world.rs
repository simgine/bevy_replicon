use core::mem;

use bevy::{
    ecs::{
        archetype::{Archetype, ArchetypeEntity, ArchetypeGeneration, ArchetypeId},
        component::{ComponentId, ComponentTicks, StorageType, Tick},
        query::{FilteredAccess, FilteredAccessSet},
        storage::TableId,
        system::{ReadOnlySystemParam, SystemMeta, SystemParam},
        world::unsafe_world_cell::UnsafeWorldCell,
    },
    prelude::*,
    ptr::Ptr,
};
use log::{debug, trace};

use crate::{
    prelude::*,
    shared::replication::rules::{ReplicationRules, component::ComponentRule},
};

/// A [`SystemParam`] that wraps [`World`], but provides access only for replicated components.
///
/// We don't use [`FilteredEntityRef`](bevy::ecs::world::FilteredEntityRef) to avoid access checks
/// and [`StorageType`] fetch (we cache this information on replicated archetypes).
pub(crate) struct ServerWorld<'w, 's> {
    world: UnsafeWorldCell<'w>,
    state: &'s ReplicationReadState,
}

impl<'w> ServerWorld<'w, '_> {
    /// Extracts a component as [`Ptr`] and its ticks from a table or sparse set, depending on its storage type.
    ///
    /// # Safety
    ///
    /// The component must be present in this archetype, have the specified storage type, and be previously marked for replication.
    pub(super) unsafe fn get_component_unchecked(
        &self,
        entity: &ArchetypeEntity,
        table_id: TableId,
        storage: StorageType,
        component_id: ComponentId,
    ) -> (Ptr<'w>, ComponentTicks) {
        debug_assert!(
            self.state
                .component_access
                .access()
                .has_component_read(component_id)
        );

        let storages = unsafe { self.world.storages() };
        match storage {
            StorageType::Table => unsafe {
                let table = storages.tables.get(table_id).unwrap_unchecked();
                // TODO: re-use column lookup, asked in https://github.com/bevyengine/bevy/issues/16593.
                let component: Ptr<'w> = table
                    .get_component(component_id, entity.table_row())
                    .unwrap_unchecked();
                let ticks = table
                    .get_ticks_unchecked(component_id, entity.table_row())
                    .unwrap_unchecked();

                (component, ticks)
            },
            StorageType::SparseSet => unsafe {
                let sparse_set = storages.sparse_sets.get(component_id).unwrap_unchecked();
                let component = sparse_set.get(entity.id()).unwrap_unchecked();
                let ticks = sparse_set.get_ticks(entity.id()).unwrap_unchecked();

                (component, ticks)
            },
        }
    }

    /// Return iterator over replicated archetypes.
    pub(super) fn iter_archetypes(
        &self,
    ) -> impl Iterator<Item = (&Archetype, &ReplicatedArchetype)> {
        self.state.archetypes.iter().map(|replicated_archetype| {
            // SAFETY: all IDs from replicated archetypes obtained from real archetypes.
            let archetype = unsafe {
                self.world
                    .archetypes()
                    .get(replicated_archetype.id)
                    .unwrap_unchecked()
            };

            (archetype, replicated_archetype)
        })
    }
}

unsafe impl SystemParam for ServerWorld<'_, '_> {
    type State = ReplicationReadState;
    type Item<'w, 's> = ServerWorld<'w, 's>;

    fn init_state(world: &mut World) -> Self::State {
        let mut component_access = FilteredAccess::default();

        let marker_id = world.register_component::<Replicated>();
        component_access.add_component_read(marker_id);

        let rules = world.resource::<ReplicationRules>();
        debug!("initializing with {} replication rules", rules.len());
        for rule in rules.iter() {
            for component in &rule.components {
                component_access.add_component_read(component.id);
            }
        }

        Self::State {
            component_access,
            marker_id,
            archetypes: Default::default(),
            generation: ArchetypeGeneration::initial(),
        }
    }

    fn init_access(
        state: &Self::State,
        system_meta: &mut SystemMeta,
        component_access_set: &mut FilteredAccessSet,
        _world: &mut World,
    ) {
        let conflicts = component_access_set.get_conflicts_single(&state.component_access);
        if !conflicts.is_empty() {
            panic!(
                "replicated components in system `{}` shouldn't be in conflict with other system parameters",
                system_meta.name(),
            );
        }

        component_access_set.add(state.component_access.clone());
    }

    unsafe fn get_param<'world, 'state>(
        state: &'state mut Self::State,
        _system_meta: &SystemMeta,
        world: UnsafeWorldCell<'world>,
        _change_tick: Tick,
    ) -> Self::Item<'world, 'state> {
        let archetypes = world.archetypes();
        let old_generation = mem::replace(&mut state.generation, archetypes.generation());

        // SAFETY: Has access to this resource and the access is unique.
        let rules = unsafe {
            world
                .get_resource::<ReplicationRules>()
                .expect("replication rules should've been initialized in the plugin")
        };
        for archetype in archetypes[old_generation..]
            .iter()
            .filter(|archetype| archetype.contains(state.marker_id))
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

            state.archetypes.push(replicated_archetype);
        }

        ServerWorld { world, state }
    }
}

unsafe impl ReadOnlySystemParam for ServerWorld<'_, '_> {}

pub(crate) struct ReplicationReadState {
    /// All replicated components.
    ///
    /// Used only in debug to check component access.
    component_access: FilteredAccess,

    /// ID of [`Replicated`] component.
    marker_id: ComponentId,

    /// Highest processed archetype ID.
    generation: ArchetypeGeneration,

    /// Archetypes marked as replicated.
    archetypes: Vec<ReplicatedArchetype>,
}

/// An archetype that can be stored in [`ReplicatedArchetypes`].
pub(crate) struct ReplicatedArchetype {
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
}

#[cfg(test)]
mod tests {
    use bevy::transform::components::Transform;
    use serde::{Deserialize, Serialize};
    use test_log::test;

    use super::*;
    use crate::shared::replication::registry::ReplicationRegistry;

    #[test]
    #[should_panic]
    fn query_after() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<Transform>()
            .add_systems(Update, |_: ServerWorld, _: Query<&mut Transform>| {});

        app.update();
    }

    #[test]
    #[should_panic]
    fn query_before() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<Transform>()
            .add_systems(Update, |_: Query<&mut Transform>, _: ServerWorld| {});

        app.update();
    }

    #[test]
    fn readonly_query() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<Transform>()
            .add_systems(Update, |_: ServerWorld, _: Query<&Transform>| {});

        app.update();
    }

    #[test]
    fn empty() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .add_systems(Update, |world: ServerWorld| {
                assert!(world.state.archetypes.is_empty());
            });

        app.world_mut().spawn_empty();
        app.update();
    }

    #[test]
    fn no_components() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .add_systems(Update, |world: ServerWorld| {
                assert_eq!(world.state.archetypes.len(), 1);
                let archetype = world.state.archetypes.first().unwrap();
                assert!(archetype.components.is_empty());
            });

        app.world_mut().spawn(Replicated);
        app.update();
    }

    #[test]
    fn not_replicated() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<A>()
            .add_systems(Update, |world: ServerWorld| {
                assert_eq!(world.state.archetypes.len(), 1);
                let archetype = world.state.archetypes.first().unwrap();
                assert!(archetype.components.is_empty());
            });

        app.world_mut().spawn((Replicated, B));
        app.update();
    }

    #[test]
    fn component() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<A>()
            .add_systems(Update, |world: ServerWorld| {
                assert_eq!(world.state.archetypes.len(), 1);
                let archetype = world.state.archetypes.first().unwrap();
                assert_eq!(archetype.components.len(), 1);
            });

        app.world_mut().spawn((Replicated, A));
        app.update();
    }

    #[test]
    fn bundle() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate_bundle::<(A, B)>()
            .add_systems(Update, |world: ServerWorld| {
                assert_eq!(world.state.archetypes.len(), 1);
                let archetype = world.state.archetypes.first().unwrap();
                assert_eq!(archetype.components.len(), 2);
            });

        app.world_mut().spawn((Replicated, A, B));
        app.update();
    }

    #[test]
    fn part_of_bundle() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate_bundle::<(A, B)>()
            .add_systems(Update, |world: ServerWorld| {
                assert_eq!(world.state.archetypes.len(), 1);
                let archetype = world.state.archetypes.first().unwrap();
                assert!(archetype.components.is_empty());
            });

        app.world_mut().spawn((Replicated, A));
        app.update();
    }

    #[test]
    fn bundle_with_subset() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<A>()
            .replicate_bundle::<(A, B)>()
            .add_systems(Update, |world: ServerWorld| {
                assert_eq!(world.state.archetypes.len(), 1);
                let archetype = world.state.archetypes.first().unwrap();
                assert_eq!(archetype.components.len(), 2);
            });

        app.world_mut().spawn((Replicated, A, B));
        app.update();
    }

    #[test]
    fn bundle_with_multiple_subsets() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<A>()
            .replicate::<B>()
            .replicate_bundle::<(A, B)>()
            .add_systems(Update, |world: ServerWorld| {
                assert_eq!(world.state.archetypes.len(), 1);
                let archetype = world.state.archetypes.first().unwrap();
                assert_eq!(archetype.components.len(), 2);
            });

        app.world_mut().spawn((Replicated, A, B));
        app.update();
    }

    #[test]
    fn bundle_with_overlap() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate_bundle::<(A, C)>()
            .replicate_bundle::<(A, B)>()
            .add_systems(Update, |world: ServerWorld| {
                assert_eq!(world.state.archetypes.len(), 1);
                let archetype = world.state.archetypes.first().unwrap();
                assert_eq!(archetype.components.len(), 3);
            });

        app.world_mut().spawn((Replicated, A, B, C));
        app.update();
    }

    #[derive(Serialize, Deserialize, Component)]
    struct A;

    #[derive(Serialize, Deserialize, Component)]
    struct B;

    #[derive(Serialize, Deserialize, Component)]
    struct C;
}
