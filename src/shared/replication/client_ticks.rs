use core::time::Duration;

use bevy::{
    ecs::{change_detection::Tick, entity::hash_map::EntityHashMap},
    platform::collections::HashMap,
    prelude::*,
};
use log::{debug, trace};
use smallvec::SmallVec;

use super::mutate_index::MutateIndex;
use crate::{
    prelude::*,
    shared::replication::registry::{ComponentIndex, component_mask::ComponentMask},
};

/// Alias for cursors associated with components.
///
/// We use a [`SmallVec`] because entities usually don't have more than a few
/// components with diff replication enabled.
pub(crate) type DiffCursors = SmallVec<[(ComponentIndex, DiffIndex); 3]>;

/// Tracks replication ticks for a client.
#[derive(Component, Default)]
pub(crate) struct ClientTicks {
    /// Last acknowledged tick for each visible entity with its components.
    ///
    /// Used to track what the client has already received.
    pub(crate) entities: EntityHashMap<EntityTicks>,

    /// The last tick in which a replicated entity had an insertion, removal, or gained/lost a component from the
    /// perspective of the client.
    ///
    /// It should be included in mutate messages and server events to avoid needless waiting for the next update
    /// message to arrive.
    pub(crate) update_tick: RepliconTick,

    /// Mutate message indices mapped to their info.
    mutations: HashMap<MutateIndex, MutateInfo>,

    /// Index for the next mutate message to be sent to this client.
    ///
    /// See also [`Self::register_mutate_message`].
    mutate_index: MutateIndex,
}

impl ClientTicks {
    /// Allocates a new index for update message.
    ///
    /// The message later needs to be registered via [`Self::register_update_message`].
    #[must_use]
    pub(crate) fn next_mutate_index(&mut self) -> MutateIndex {
        self.mutate_index.advance()
    }

    /// Registers mutate message to later acknowledge updated entities.
    pub(crate) fn register_mutate_message(&mut self, index: MutateIndex, info: MutateInfo) {
        self.mutations.insert(index, info);
    }

    /// Marks mutate message as acknowledged by its index.
    ///
    /// Returns associated entities and their component IDs.
    ///
    /// Updates the tick and components of all entities from this mutation message if the tick is higher.
    pub(crate) fn ack_mutate_message(&mut self, client: Entity, mutate_index: MutateIndex) {
        let Some(mutate_info) = self.mutations.remove(&mutate_index) else {
            debug!("received unknown `{mutate_index:?}` from client `{client}`");
            return;
        };

        for info in mutate_info.entities {
            let Some(entity_ticks) = self.entities.get_mut(&info.entity) else {
                // We ignore missing entities, since they were probably despawned.
                continue;
            };

            // Received tick could be outdated because we bump it
            // if we detect any insertion on the entity in `collect_changes`.
            if entity_ticks.server_tick.is_older(mutate_info.server_tick) {
                entity_ticks.server_tick = mutate_info.server_tick;
                entity_ticks.system_tick = mutate_info.system_tick;
                entity_ticks.components |= &info.components;

                for (component, cursor) in info.diff_cursors {
                    if entity_ticks.components.contains(component) {
                        entity_ticks.set_diff_cursor(component, cursor);
                    }
                }
            }
        }
        trace!(
            "acknowledged mutate message with `{:?}` from client `{client}`",
            mutate_info.server_tick,
        );
    }

    /// Removes all mutate messages older then `min_timestamp`.
    ///
    /// Calls given function for each removed message.
    pub(crate) fn cleanup_older_mutations(&mut self, min_timestamp: Duration) {
        self.mutations
            .retain(|_, mutate_info| mutate_info.timestamp >= min_timestamp);
    }
}

/// Acknowledgment information about an entity.
pub(crate) struct EntityTicks {
    /// The last server tick for which data for this entity was sent.
    ///
    /// This tick serves as the reference point for determining whether components
    /// on the entity have changed and need to be replicated. Component changes
    /// older than this update tick are assumed to have been acknowledged by the client.
    pub(crate) server_tick: RepliconTick,

    /// The corresponding tick for change detection.
    pub(crate) system_tick: Tick,

    /// The list of components that were replicated on this tick.
    pub(crate) components: ComponentMask,

    /// Last acknowledged diff cursor for components.
    ///
    /// This is separate from [`Self::server_tick`]: the server tick controls change
    /// detection for the component as a whole, while the cursor controls the
    /// acknowledged base used to serialize only diffs that the client has not yet
    /// acknowledged.
    ///
    /// Absence means the client has never acknowledged the base value, or
    /// diff replication is not enabled for it.
    ///
    /// Cursors are pruned when the component is removed.
    diff_cursors: DiffCursors,
}

impl EntityTicks {
    pub(crate) fn new(
        server_tick: RepliconTick,
        system_tick: Tick,
        components: ComponentMask,
    ) -> Self {
        Self {
            server_tick,
            system_tick,
            components,
            diff_cursors: Default::default(),
        }
    }

    pub(crate) fn diff_cursor(&self, component: ComponentIndex) -> Option<DiffIndex> {
        self.diff_cursors
            .iter()
            .find_map(|&(index, cursor)| (index == component).then_some(cursor))
    }

    /// Sets the acknowledged diff cursor for a component.
    ///
    /// If ACKs arrive out of order, older ACKs must be filtered out by the caller.
    fn set_diff_cursor(&mut self, component: ComponentIndex, cursor: DiffIndex) {
        if let Some((_, existing)) = self
            .diff_cursors
            .iter_mut()
            .find(|(index, _)| *index == component)
        {
            *existing = cursor;
        } else {
            self.diff_cursors.push((component, cursor));
        }
    }

    pub(crate) fn remove_component(&mut self, component: ComponentIndex) {
        self.components.remove(component);
        // Component removal resets the entity's state for this component, so
        // its diff cursor becomes stale too.
        if let Some(index) = self
            .diff_cursors
            .iter()
            .position(|(index, _)| *index == component)
        {
            self.diff_cursors.remove(index);
        }
    }
}

/// Information about a mutation message.
pub(crate) struct MutateInfo {
    pub(crate) server_tick: RepliconTick,
    pub(crate) system_tick: Tick,
    pub(crate) timestamp: Duration,
    pub(crate) entities: Vec<MutatedEntityInfo>,
}

/// Entity data acknowledged by a mutation message.
pub(crate) struct MutatedEntityInfo {
    pub(crate) entity: Entity,
    pub(crate) components: ComponentMask,
    pub(crate) diff_cursors: DiffCursors,
}
