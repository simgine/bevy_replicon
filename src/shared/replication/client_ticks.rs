use core::{mem, time::Duration};

use bevy::{
    ecs::{component::Tick, entity::hash_map::EntityHashMap},
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
    /// Sets the client's update tick.
    pub(crate) fn set_update_tick(&mut self, tick: RepliconTick) {
        self.update_tick = tick;
    }

    /// Returns the last tick in which a replicated entity had an insertion, removal, or gained/lost a component from the
    /// perspective of the client.
    pub fn update_tick(&self) -> RepliconTick {
        self.update_tick
    }

    /// Registers mutate message at specified `tick` and `timestamp` and returns its index with entities to fill.
    ///
    /// Used later to acknowledge updated entities.
    #[must_use]
    pub(crate) fn register_mutate_message(
        &mut self,
        entity_buffer: &mut EntityBuffer,
        system_tick: Tick,
        server_tick: RepliconTick,
        timestamp: Duration,
    ) -> (MutateIndex, &mut Vec<Entity>) {
        let mutate_index = self.mutate_index.advance();

        let mut entities = entity_buffer.pop().unwrap_or_default();
        entities.clear();
        let mutate_info = MutateInfo {
            system_tick,
            server_tick,
            timestamp,
            entities,
        };
        let mutate_info = self
            .mutations
            .entry(mutate_index)
            .insert(mutate_info)
            .into_mut();

        (mutate_index, &mut mutate_info.entities)
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

    /// Marks mutate message as acknowledged by its index.
    ///
    /// Mutation tick for all entities from this mutate message will be set to the message tick if it's higher.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(crate) fn ack_mutate_message(
        &mut self,
        client: Entity,
        this_run: Tick,
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
            if !system_tick.is_newer_than(mutate_info.system_tick, this_run) {
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
    pub(crate) fn remove_entity(&mut self, entity: Entity) {
        self.mutation_ticks.remove(&entity);
        // We don't clean up `self.mutations` for efficiency reasons.
        // `Self::acknowledge` will properly ignore despawned entities.
    }

    /// Removes all mutate messages older then `min_timestamp`.
    ///
    /// Keeps allocated memory in the buffers for reuse.
    pub(crate) fn cleanup_older_mutations(
        &mut self,
        entity_buffer: &mut EntityBuffer,
        min_timestamp: Duration,
    ) {
        self.mutations.retain(|_, mutate_info| {
            if mutate_info.timestamp < min_timestamp {
                entity_buffer.push(mem::take(&mut mutate_info.entities));
                false
            } else {
                true
            }
        });
    }
}

/// Reusable buffer for [`ClientTicks`].
///
/// Stores [`Vec`]'s from acknowledged [`MutateInfo`]'s.
/// to reuse allocated capacity.
#[derive(Default, Resource, Deref, DerefMut)]
pub(crate) struct EntityBuffer(Vec<Vec<Entity>>);

struct MutateInfo {
    system_tick: Tick,
    server_tick: RepliconTick,
    timestamp: Duration,
    entities: Vec<Entity>,
}
