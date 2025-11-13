use bevy::{
    ecs::{
        archetype::ArchetypeEntity,
        component::{ComponentId, ComponentTicks, StorageType, Tick},
        query::{FilteredAccess, FilteredAccessSet},
        storage::TableId,
        system::{ReadOnlySystemParam, SystemMeta, SystemParam},
        world::unsafe_world_cell::UnsafeWorldCell,
    },
    prelude::*,
    ptr::Ptr,
};
use log::debug;

use crate::{prelude::*, shared::replication::rules::ReplicationRules};

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

        // SAFETY: caller ensured the component is replicated.
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

        Self::State { component_access }
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
        ServerWorld { world, state }
    }
}

unsafe impl ReadOnlySystemParam for ServerWorld<'_, '_> {}

pub(crate) struct ReplicationReadState {
    /// All replicated components.
    ///
    /// Used only in debug to check component access.
    component_access: FilteredAccess,
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use test_log::test;

    use super::*;
    use crate::shared::replication::registry::ReplicationRegistry;

    #[test]
    #[should_panic]
    fn query_after() {
        let mut app = App::new();
        app.init_resource::<ReplicationRegistry>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .replicate::<Test>()
            .add_systems(Update, |_: ServerWorld, _: Query<&mut Test>| {});

        app.update();
    }

    #[test]
    #[should_panic]
    fn query_before() {
        let mut app = App::new();
        app.init_resource::<ReplicationRegistry>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .replicate::<Test>()
            .add_systems(Update, |_: Query<&mut Test>, _: ServerWorld| {});

        app.update();
    }

    #[test]
    fn readonly_query() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<Test>()
            .add_systems(Update, |_: ServerWorld, _: Query<&Test>| {});

        app.update();
    }

    #[test]
    fn not_replicated() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .add_systems(Update, |_: ServerWorld, _: Query<&mut Test>| {});

        app.update();
    }

    #[derive(Component, Serialize, Deserialize)]
    struct Test;
}
