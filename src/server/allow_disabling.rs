use bevy::{
    ecs::{
        archetype::Archetype,
        change_detection::Tick,
        component::{ComponentId, Components},
        entity_disabling::DefaultQueryFilters,
        query::{FilteredAccess, FilteredAccessSet, QueryFilter, WorldQuery},
        storage::{Table, TableRow},
        world::unsafe_world_cell::UnsafeWorldCell,
    },
    prelude::*,
};
use smallvec::SmallVec;

/// Like [`Allow`], but for every disabling component registered in [`DefaultQueryFilters`].
///
/// Replication doesn't skip disabled entities, so their bookkeeping can't skip them either.
///
/// Captures the disabling components when the query is initialized, like
/// [`DefaultQueryFilters`] itself.
pub(super) struct AllowDisabling;

// SAFETY: like `Allow`, adds only archetypal access, which doesn't affect the query,
// and the fetch doesn't access any components.
unsafe impl WorldQuery for AllowDisabling {
    type Fetch<'w> = ();
    type State = SmallVec<[ComponentId; 4]>;

    fn shrink_fetch<'wlong: 'wshort, 'wshort>(_: Self::Fetch<'wlong>) -> Self::Fetch<'wshort> {}

    #[inline]
    unsafe fn init_fetch(_: UnsafeWorldCell, _: &Self::State, _: Tick, _: Tick) {}

    const IS_DENSE: bool = true;

    #[inline]
    unsafe fn set_archetype(_: &mut (), _: &Self::State, _: &Archetype, _: &Table) {}

    #[inline]
    unsafe fn set_table(_: &mut (), _: &Self::State, _: &Table) {}

    fn update_component_access(state: &Self::State, access: &mut FilteredAccess) {
        for &id in state {
            access.access_mut().add_archetypal(id);
        }
    }

    fn init_nested_access(
        _state: &Self::State,
        _system_name: Option<&str>,
        _component_access_set: &mut FilteredAccessSet,
        _world: UnsafeWorldCell,
    ) {
    }

    fn init_state(world: &mut World) -> Self::State {
        world
            .get_resource::<DefaultQueryFilters>()
            .map(|filters| filters.disabling_ids().collect())
            .unwrap_or_default()
    }

    fn get_state(_components: &Components) -> Option<Self::State> {
        // Disabling components are stored in a resource.
        None
    }

    fn matches_component_set(_: &Self::State, _: &impl Fn(ComponentId) -> bool) -> bool {
        true
    }

    fn update_archetypes(_state: &mut Self::State, _world: UnsafeWorldCell) {}
}

// SAFETY: the `WorldQuery` impl performs no access.
unsafe impl QueryFilter for AllowDisabling {
    const IS_ARCHETYPAL: bool = true;

    #[inline(always)]
    unsafe fn filter_fetch(
        _: &Self::State,
        _: &mut Self::Fetch<'_>,
        _: Entity,
        _: TableRow,
    ) -> bool {
        true
    }
}
