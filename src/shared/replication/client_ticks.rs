use core::time::Duration;

use bevy::{
    ecs::{
        component::{CheckChangeTicks, Tick},
        entity::hash_map::EntityHashMap,
    },
    platform::collections::HashMap,
    prelude::*,
};
use log::{debug, trace};

use super::mutate_index::MutateIndex;
use crate::prelude::*;

/// Tracks replication ticks for a client.
#[derive(Component, Default)]
pub struct ClientTicks {
    /// Lowest tick for use in change detection for each entity.
    mutation_ticks: EntityHashMap<(Tick, RepliconTick)>,

    /// The last tick in which a replicated entity had an insertion, removal, or gained/lost a component from the
    /// perspective of the client.
    ///
    /// It should be included in mutate messages and server events to avoid needless waiting for the next update
    /// message to arrive.
    update_tick: RepliconTick,

    /// Mutate message indices mapped to their info.
    mutations: HashMap<MutateIndex, MutateInfo>,

    /// Index for the next mutate message to be sent to this client.
    ///
    /// See also [`Self::register_mutate_message`].
    mutate_index: MutateIndex,
}

impl ClientTicks {
    pub(crate) fn check_mutation_ticks(&mut self, check: CheckChangeTicks) {
        for (tick, _) in &mut self.mutation_ticks.values_mut() {
            tick.check_tick(check);
        }
    }

    /// Sets the client's update tick.
    pub(crate) fn set_update_tick(&mut self, tick: RepliconTick) {
        self.update_tick = tick;
    }

    /// Returns the last tick in which a replicated entity had an insertion, removal, or gained/lost a component from the
    /// perspective of the client.
    pub fn update_tick(&self) -> RepliconTick {
        self.update_tick
    }

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

    /// Sets the mutation tick for an entity that is replicated to this client.
    ///
    /// The mutation tick is the reference point for determining if components on an entity have mutated and
    /// need to be replicated. Component mutations older than the update tick are assumed to be acked by the client.
    pub(crate) fn set_mutation_tick(
        &mut self,
        entity: Entity,
        system_tick: Tick,
        server_tick: RepliconTick,
    ) {
        self.mutation_ticks
            .insert(entity, (system_tick, server_tick));
    }

    /// Gets the mutation tick for an entity that is replicated to this client.
    pub(crate) fn mutation_tick(&self, entity: Entity) -> Option<(Tick, RepliconTick)> {
        self.mutation_ticks.get(&entity).copied()
    }

    /// Returns whether this entity is new for the client.
    ///
    /// This can be a new entity spawned on the server or an entity that has just become visible to the client.
    /// This occurs when its mutation tick is not set. It resets when the entity loses visibility or stops being replicated.
    pub(crate) fn is_new_for_client(&self, entity: Entity) -> bool {
        self.mutation_tick(entity).is_none()
    }

    /// Marks mutate message as acknowledged by its index.
    ///
    /// Mutation tick for all entities from this mutate message will be set to the message tick if it's higher.
    pub(crate) fn ack_mutate_message(
        &mut self,
        client: Entity,
        mutate_index: MutateIndex,
    ) -> Option<Vec<Entity>> {
        let Some(mutate_info) = self.mutations.remove(&mutate_index) else {
            debug!("received unknown `{mutate_index:?}` from client `{client}`");
            return None;
        };

        for entity in &mutate_info.entities {
            let Some((system_tick, server_tick)) = self.mutation_ticks.get_mut(entity) else {
                // We ignore missing entities, since they were probably despawned.
                continue;
            };

            // Received tick could be outdated because we bump it
            // if we detect any insertion on the entity in `collect_changes`.
            if *server_tick < mutate_info.server_tick {
                *system_tick = mutate_info.system_tick;
                *server_tick = mutate_info.server_tick;
            }
        }
        trace!(
            "acknowledged mutate message with `{:?}` from client `{client}`",
            mutate_info.server_tick,
        );

        Some(mutate_info.entities)
    }

    /// Removes a despawned or hidden entity from tracking by this client.
    ///
    /// Returns `true` if the entity has a tick.
    pub(crate) fn remove_entity(&mut self, entity: Entity) -> bool {
        self.mutation_ticks.remove(&entity).is_some()
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

pub(crate) struct MutateInfo {
    pub(crate) system_tick: Tick,
    pub(crate) server_tick: RepliconTick,
    pub(crate) timestamp: Duration,
    pub(crate) entities: Vec<Entity>,
}
