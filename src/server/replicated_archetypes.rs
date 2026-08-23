use core::{
    hash::{BuildHasher, Hash, Hasher},
    mem,
};

use bevy::{
    ecs::{
        archetype::{Archetype, ArchetypeGeneration, ArchetypeId, Archetypes},
        component::{ComponentId, StorageType},
    },
    platform::{
        collections::{HashMap, HashSet},
        hash::{FixedHasher, NoOpHash},
    },
    prelude::*,
};
use log::trace;

use crate::{
    prelude::*,
    shared::replication::{
        receive_markers::ReceiveMarkers,
        rules::{ReplicationRules, component::ComponentRule, filter::FilterRule},
    },
};

#[derive(Resource)]
pub(super) struct ReplicatedArchetypes {
    /// ID of [`Replicated`] component.
    marker_id: ComponentId,

    /// Highest processed archetype ID.
    generation: ArchetypeGeneration,

    /// Components whose presence can affect replication metadata.
    key_components: Option<Box<[ComponentId]>>,

    /// Maps replication-relevant archetype components to an index in [`Self::list`].
    key_map: HashMap<ReplicationArchetypeKey, usize, NoOpHash>,

    /// Maps Bevy archetype IDs to an index in [`Self::list`].
    ids_map: HashMap<ArchetypeId, usize>,

    /// Cached metadata shared by archetypes with the same replication-relevant components.
    list: Vec<ReplicatedArchetype>,
}

impl ReplicatedArchetypes {
    /// Finalizes the components used to distinguish replication archetypes.
    ///
    /// # Arguments
    ///
    /// * `components` - Deduplicated component IDs from all replication rules.
    /// * `rules` - Replication rules whose filter components should also affect the key.
    /// * `receive_markers` - Marker component IDs that can alter receive behavior.
    pub(super) fn finalize(
        &mut self,
        mut components: HashSet<ComponentId>,
        rules: &ReplicationRules,
        receive_markers: &ReceiveMarkers,
    ) {
        assert!(
            self.key_components.is_none(),
            "replicated archetypes should be finalized only once"
        );

        for rule in rules.iter() {
            for filter in &rule.filters {
                filter.push_components(&mut components);
            }
        }
        components.extend(receive_markers.component_ids());

        self.key_components = Some(components.into_iter().collect());
    }

    pub(super) fn update(
        &mut self,
        archetypes: &Archetypes,
        rules: &ReplicationRules,
        receive_markers: &ReceiveMarkers,
    ) {
        let key_components = self
            .key_components
            .as_deref()
            .expect("replicated archetypes should be finalized before updating");
        let old_generation = mem::replace(&mut self.generation, archetypes.generation());

        for archetype in archetypes[old_generation..]
            .iter()
            .filter(|archetype| archetype.contains(self.marker_id))
        {
            trace!("marking `{:?}` as replicated", archetype.id());
            let key = ReplicationArchetypeKey::new(archetype, key_components);
            let index = if let Some(&index) = self.key_map.get(&key) {
                index
            } else {
                let mut replicated_archetype = ReplicatedArchetype::default();
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

                // Incoming receive markers need to be processed before other components so they can
                // affect which receive functions are selected for the rest of the entity update.
                replicated_archetype.components.sort_by_key(|(rule, _)| {
                    receive_markers.marker_index(rule.id).unwrap_or(usize::MAX)
                });

                let index = self.list.len();
                self.key_map.insert(key, index);
                self.list.push(replicated_archetype);
                index
            };

            self.ids_map.insert(archetype.id(), index);
            self.list[index].ids.push(archetype.id());
        }
    }

    pub(super) fn marker_id(&self) -> ComponentId {
        self.marker_id
    }

    pub(super) fn get(&self, id: ArchetypeId) -> Option<&ReplicatedArchetype> {
        let index = *self.ids_map.get(&id)?;
        self.list.get(index)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (ArchetypeId, &ReplicatedArchetype)> {
        self.list.iter().flat_map(|replicated_archetype| {
            replicated_archetype
                .ids
                .iter()
                .map(move |&id| (id, replicated_archetype))
        })
    }
}

impl FromWorld for ReplicatedArchetypes {
    fn from_world(world: &mut World) -> Self {
        Self {
            marker_id: world.register_component::<Replicated>(),
            generation: ArchetypeGeneration::initial(),
            key_components: None,
            key_map: Default::default(),
            ids_map: Default::default(),
            list: Default::default(),
        }
    }
}

/// Hash of all components whose presence affects replication metadata for an archetype.
#[derive(PartialEq, Eq, Hash)]
struct ReplicationArchetypeKey(u64);

impl ReplicationArchetypeKey {
    fn new(archetype: &Archetype, key_components: &[ComponentId]) -> Self {
        let mut hasher = FixedHasher.build_hasher();
        for id in key_components.iter().filter(|&&id| archetype.contains(id)) {
            id.hash(&mut hasher);
        }
        Self(hasher.finish())
    }
}

/// Collects filter components whose presence can affect a replication archetype key.
trait FilterRuleKeyExt {
    /// Appends component IDs referenced by this filter, including nested [`FilterRule::Or`]s.
    fn push_components(&self, components: &mut HashSet<ComponentId>);
}

impl FilterRuleKeyExt for FilterRule {
    fn push_components(&self, components: &mut HashSet<ComponentId>) {
        match self {
            Self::With(id) | Self::Without(id) => {
                components.insert(*id);
            }
            Self::Or(filters) => {
                for filter in filters {
                    filter.push_components(components);
                }
            }
        }
    }
}

/// An archetype that can be stored in [`ReplicatedArchetypes`].
#[derive(Default)]
pub(super) struct ReplicatedArchetype {
    /// IDs of Bevy archetypes that share this replication metadata.
    ids: Vec<ArchetypeId>,

    /// Components marked as replicated.
    pub(super) components: Vec<(ComponentRule, StorageType)>,
}

impl ReplicatedArchetype {
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
            .init_resource::<ReplicationRules>()
            .init_resource::<ReceiveMarkers>();

        app.world_mut().spawn_empty();
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert!(archetypes.list.is_empty());
    }

    #[test]
    fn no_components() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReceiveMarkers>();

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
            .init_resource::<ReceiveMarkers>()
            .replicate::<A>();

        app.world_mut().spawn((Replicated, A));
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.list.len(), 1);
        let archetype = archetypes.list.first().unwrap();
        assert_eq!(archetype.components.len(), 1);
    }

    #[test]
    fn shares_metadata_for_unrelated_components() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReceiveMarkers>()
            .replicate::<A>();

        app.world_mut().spawn((Replicated, A));
        app.world_mut().spawn((Replicated, A, Unrelated));
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.key_map.len(), 1);
        assert_eq!(archetypes.list.len(), 1);
        assert_eq!(archetypes.ids_map.len(), 2);
        assert!(archetypes.ids_map.values().all(|&index| index == 0));
        assert_eq!(archetypes.list[0].ids.len(), 2);
    }

    #[test]
    fn separates_metadata_for_filter_components() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReceiveMarkers>()
            .replicate_filtered::<A, Or<(With<B>, With<C>)>>();

        let first = app.world_mut().spawn((Replicated, A)).archetype().id();
        let second = app.world_mut().spawn((Replicated, A, B)).archetype().id();
        let third = app
            .world_mut()
            .spawn((Replicated, A, Unrelated))
            .archetype()
            .id();
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.key_map.len(), 2);
        assert_eq!(archetypes.list.len(), 2);
        assert_eq!(
            archetypes
                .iter()
                .map(|(archetype_id, _)| archetype_id)
                .collect::<Vec<_>>(),
            [first, third, second]
        );
        assert!(
            archetypes
                .list
                .iter()
                .any(|archetype| archetype.components.is_empty())
        );
        assert!(
            archetypes
                .list
                .iter()
                .any(|archetype| archetype.components.len() == 1)
        );
    }

    #[test]
    fn separates_metadata_for_receive_markers() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReceiveMarkers>()
            .register_marker::<Marker>();

        app.world_mut().spawn(Replicated);
        app.world_mut().spawn((Replicated, Marker));
        update_archetypes(&mut app);

        let archetypes = app.world().resource::<ReplicatedArchetypes>();
        assert_eq!(archetypes.key_map.len(), 2);
        assert_eq!(archetypes.list.len(), 2);
    }

    #[test]
    fn bundle() {
        let mut app = App::new();
        app.init_resource::<ReplicatedArchetypes>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReceiveMarkers>()
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
            .init_resource::<ReceiveMarkers>()
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
            .init_resource::<ReceiveMarkers>()
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
            .init_resource::<ReceiveMarkers>()
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
            .init_resource::<ReceiveMarkers>()
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
                let receive_markers = world.resource::<ReceiveMarkers>();
                if archetypes.key_components.is_none() {
                    let replicated_ids = rules
                        .iter()
                        .flat_map(|rule| &rule.components)
                        .map(|component| component.id)
                        .collect();
                    archetypes.finalize(replicated_ids, rules, receive_markers);
                }
                archetypes.update(world.archetypes(), rules, receive_markers);
            });
    }

    #[derive(Component, Serialize, Deserialize)]
    struct A;

    #[derive(Component, Serialize, Deserialize)]
    struct B;

    #[derive(Component, Serialize, Deserialize)]
    struct C;

    #[derive(Component)]
    struct Unrelated;

    #[derive(Component)]
    struct Marker;
}
