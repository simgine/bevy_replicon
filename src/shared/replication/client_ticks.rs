use core::time::Duration;

use bevy::{
    ecs::{component::Tick, entity::hash_map::EntityHashMap},
    platform::collections::HashMap,
    prelude::*,
};
use log::{debug, trace};

use super::mutate_index::MutateIndex;
use crate::{prelude::*, shared::replication::registry::component_mask::ComponentMask};

/// Tracks replication ticks for a client.
#[derive(Component, Default)]
pub struct ClientTicks {
    /// Acknowledged ticks and components for each visible entity.
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
    pub(crate) fn ack_mutate_message(
        &mut self,
        client: Entity,
        mutate_index: MutateIndex,
    ) -> Option<Vec<(Entity, ComponentMask)>> {
        let Some(mutate_info) = self.mutations.remove(&mutate_index) else {
            debug!("received unknown `{mutate_index:?}` from client `{client}`");
            return None;
        };

        for (entity, components) in &mutate_info.entities {
            let Some(entity_ticks) = self.entities.get_mut(entity) else {
                // We ignore missing entities, since they were probably despawned.
                continue;
            };

            // Received tick could be outdated because we bump it
            // if we detect any insertion on the entity in `collect_changes`.
            if entity_ticks.server_tick < mutate_info.server_tick {
                entity_ticks.server_tick = mutate_info.server_tick;
                entity_ticks.system_tick = mutate_info.system_tick;
                entity_ticks.components |= components;
            }
        }
        trace!(
            "acknowledged mutate message with `{:?}` from client `{client}`",
            mutate_info.server_tick,
        );

        Some(mutate_info.entities)
    }

    /// Removes all mutate messages older then `min_timestamp`.
    ///
    /// Calls given function for each removed message.
    pub(crate) fn cleanup_older_mutations(
        &mut self,
        min_timestamp: Duration,
        mut f: impl FnMut(&mut MutateInfo),
    ) {
        self.mutations.retain(|_, mutate_info| {
            if mutate_info.timestamp < min_timestamp {
                (f)(mutate_info);
                false
            } else {
                true
            }
        });
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
}

/// Information about a mutation message.
pub(crate) struct MutateInfo {
    pub(crate) server_tick: RepliconTick,
    pub(crate) system_tick: Tick,
    pub(crate) timestamp: Duration,
    pub(crate) entities: Vec<(Entity, ComponentMask)>,
}
