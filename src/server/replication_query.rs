use bevy::{
    ecs::{
        archetype::ArchetypeEntity,
        change_detection::{ComponentTicks, Tick},
        component::{ComponentId, StorageType},
        query::{FilteredAccess, FilteredAccessSet},
        storage::{Column, ComponentSparseSet, TableId},
        system::{ReadOnlySystemParam, SystemMeta, SystemParam, SystemParamValidationError},
        world::unsafe_world_cell::UnsafeWorldCell,
    },
    prelude::*,
    ptr::Ptr,
};
use log::debug;

use crate::{prelude::*, shared::replication::rules::ReplicationRules};

/// Like [`Query`], but provides dynamic access for replicated components and replication metadata.
///
/// We don't use [`FilteredEntityRef`](bevy::ecs::world::FilteredEntityRef) to avoid access checks
/// and [`StorageType`] fetch (we cache this information on replicated archetypes).
pub(crate) struct ReplicationQuery<'w, 's> {
    world: UnsafeWorldCell<'w>,
    state: &'s ReplicationQueryState,
}

impl<'w> ReplicationQuery<'w, '_> {
    /// Returns [`ReplicatePriority`] for an entity if it has one.
    pub(super) fn get_priority(&self, entity: &ArchetypeEntity, table_id: TableId) -> Option<f32> {
        let priority_id = self.state.priority_id;
        debug_assert!(self.state.component_access.access().has_read(priority_id));

        debug_assert_eq!(
            ReplicatePriority::STORAGE_TYPE,
            StorageType::Table,
            "`ReplicatePriority::STORAGE_TYPE` must be `StorageType::Table`",
        );

        // SAFETY: access to `ReplicatePriority` is registered in `component_access`.
        let storages = unsafe { self.world.storages() };
        let table = storages.tables.get(table_id)?;

        // SAFETY: the component has table storage.
        let ptr = unsafe { table.get_component(priority_id, entity.table_row())? };

        // SAFETY: `priority_id` is registered for `ReplicatePriority`.
        let priority = unsafe { ptr.deref::<ReplicatePriority>() };
        Some(**priority)
    }

    /// Resolves where a replicated component of an archetype is stored.
    ///
    /// Looking the column or sparse set up once per archetype makes per-entity
    /// access a row index instead of a map lookup.
    ///
    /// # Safety
    ///
    /// The component must be present in this archetype, have the specified storage type, and be previously marked for replication.
    pub(super) unsafe fn component_storage(
        &self,
        table_id: TableId,
        storage: StorageType,
        component_id: ComponentId,
    ) -> ComponentStorage<'w> {
        debug_assert!(self.state.component_access.access().has_read(component_id));

        // SAFETY: caller ensured the component is replicated.
        let storages = unsafe { self.world.storages() };
        match storage {
            StorageType::Table => unsafe {
                let table = storages.tables.get(table_id).unwrap_unchecked();
                ComponentStorage::Table(table.get_column(component_id).unwrap_unchecked())
            },
            StorageType::SparseSet => unsafe {
                ComponentStorage::SparseSet(
                    storages.sparse_sets.get(component_id).unwrap_unchecked(),
                )
            },
        }
    }
}

/// Storage of a replicated component resolved for a specific archetype.
#[derive(Clone, Copy)]
pub(super) enum ComponentStorage<'w> {
    Table(&'w Column),
    SparseSet(&'w ComponentSparseSet),
}

impl<'w> ComponentStorage<'w> {
    /// Extracts the component as [`Ptr`] with its ticks.
    ///
    /// # Safety
    ///
    /// The entity must belong to the archetype this storage was resolved for.
    pub(super) unsafe fn get(self, entity: &ArchetypeEntity) -> (Ptr<'w>, ComponentTicks) {
        match self {
            ComponentStorage::Table(column) => unsafe {
                let row = entity.table_row();
                (
                    column.get_data_unchecked(row),
                    column.get_ticks_unchecked(row),
                )
            },
            ComponentStorage::SparseSet(sparse_set) => unsafe {
                let component = sparse_set.get(entity.id()).unwrap_unchecked();
                let ticks = sparse_set.get_ticks(entity.id()).unwrap_unchecked();
                (component, ticks)
            },
        }
    }
}

unsafe impl SystemParam for ReplicationQuery<'_, '_> {
    type State = ReplicationQueryState;
    type Item<'w, 's> = ReplicationQuery<'w, 's>;

    fn init_state(world: &mut World) -> Self::State {
        let mut component_access = FilteredAccess::default();

        let marker_id = world.register_component::<Replicated>();
        component_access.add_read(marker_id);

        let priority_id = world.register_component::<ReplicatePriority>();
        component_access.add_read(priority_id);

        let rules = world.resource::<ReplicationRules>();
        debug!("initializing with {} replication rules", rules.len());
        for rule in rules.iter() {
            for component in &rule.components {
                component_access.add_read(component.id);
            }
        }

        Self::State {
            component_access,
            priority_id,
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
    ) -> Result<Self::Item<'world, 'state>, SystemParamValidationError> {
        Ok(ReplicationQuery { world, state })
    }
}

unsafe impl ReadOnlySystemParam for ReplicationQuery<'_, '_> {}

pub(crate) struct ReplicationQueryState {
    /// All replicated components.
    ///
    /// Used only in debug to check component access.
    component_access: FilteredAccess,

    /// ID of [`ReplicatePriority`] component.
    priority_id: ComponentId,
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
            .add_systems(Update, |_: ReplicationQuery, _: Query<&mut Test>| {});

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
            .add_systems(Update, |_: Query<&mut Test>, _: ReplicationQuery| {});

        app.update();
    }

    #[test]
    fn readonly_query() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<Test>()
            .add_systems(Update, |_: ReplicationQuery, _: Query<&Test>| {});

        app.update();
    }

    #[test]
    fn not_replicated() {
        let mut app = App::new();
        app.init_resource::<ReplicationRules>()
            .add_systems(Update, |_: ReplicationQuery, _: Query<&mut Test>| {});

        app.update();
    }

    #[derive(Component, Serialize, Deserialize)]
    struct Test;
}
