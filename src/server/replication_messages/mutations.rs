use core::{cmp::Ordering, iter, ops::Range, time::Duration};

use bevy::{ecs::component::Tick, prelude::*};
use log::trace;
use postcard::experimental::{max_size::MaxSize, serialized_size};

use super::{change_ranges::ChangeRanges, serialized_data::SerializedData};
use crate::{
    postcard_utils,
    prelude::*,
    server::ClientPools,
    shared::{
        backend::channels::ServerChannel,
        replication::{
            client_ticks::{ClientTicks, MutateInfo},
            mutate_index::MutateIndex,
        },
    },
};

/// Component mutations for the current tick.
///
/// The data is serialized manually and stored in the form of ranges
/// from [`SerializedData`].
///
/// Can be packed into messages using [`Self::send`].
#[derive(Default, Component)]
pub(crate) struct Mutations {
    /// Entities that are related to each other and should be replicated in sync.
    ///
    /// Like [`Self::standalone`], but grouped into arrays based on their relation graph indices.
    /// These entities are guaranteed to be included in a single message.
    related: Vec<Vec<EntityMutations>>,

    /// Component mutations that happened in this tick.
    ///
    /// These mutation are not related to any others and can be replicated independently.
    standalone: Vec<EntityMutations>,

    /// Location of the last written entity since the last call of [`Self::start_entity_mutations`].
    entity_location: Option<EntityLocation>,
}

impl Mutations {
    /// Updates internal state to start writing mutated components for an entity.
    ///
    /// Entities and their data written lazily during the iteration.
    /// See [`Self::add_entity`] and [`Self::add_component`].
    pub(crate) fn start_entity(&mut self) {
        self.entity_location = None;
    }

    /// Returns `true` if [`Self::add_entity`] were called since the last
    /// call of [`Self::start_entity`].
    pub(crate) fn entity_added(&mut self) -> bool {
        self.entity_location.is_some()
    }

    /// Adds an entity chunk.
    pub(crate) fn add_entity(
        &mut self,
        pools: &mut ClientPools,
        entity: Entity,
        graph_index: Option<usize>,
        entity_range: Range<usize>,
    ) {
        let components = pools.ranges.pop().unwrap_or_default();
        let mutations = EntityMutations {
            entity,
            ranges: ChangeRanges {
                entity: entity_range,
                components_len: 0,
                components,
            },
        };

        match graph_index {
            Some(index) => {
                self.related[index].push(mutations);
                self.entity_location = Some(EntityLocation::Related { index });
            }
            None => {
                self.entity_location = Some(EntityLocation::Standalone);
                self.standalone.push(mutations);
            }
        };
    }

    /// Adds a component chunk to the last added entity from [`Self::add_entity`].
    pub(crate) fn add_component(&mut self, component: Range<usize>) {
        let mutations = self
            .entity_location
            .and_then(|location| match location {
                EntityLocation::Related { index } => self.related[index].last_mut(),
                EntityLocation::Standalone => self.standalone.last_mut(),
            })
            .expect("entity should be written before adding components");

        mutations.ranges.add_component(component);
    }

    /// Removes last added entity from [`Self::add_entity`] and returns its components.
    pub(super) fn pop(&mut self) -> Option<ChangeRanges> {
        self.entity_location
            .take()
            .and_then(|location| match location {
                EntityLocation::Related { index } => self.related[index].pop(),
                EntityLocation::Standalone => self.standalone.pop(),
            })
            .map(|mutations| mutations.ranges)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.standalone.is_empty() && self.related.is_empty()
    }

    /// Packs mutations into messages.
    ///
    /// Contains update tick, current tick, mutate index and component mutations since
    /// the last acknowledged tick for each entity.
    ///
    /// Cannot be applied on the client until the update message matching this message's update tick
    /// has been applied to the client world.
    /// The message will be manually split into packets up to max size, and each packet will be applied
    /// independently on the client.
    /// Message splits only happen per-entity to avoid weird behavior from partial entity mutations.
    ///
    /// Sent over the [`ServerChannel::Mutations`] channel. If the message gets lost, we try to resend it manually,
    /// using the last up-to-date mutations to avoid re-sending old values.
    pub(crate) fn send(
        &mut self,
        messages: &mut ServerMessages,
        client: Entity,
        ticks: &mut ClientTicks,
        split_buffer: &mut Vec<MutationsSplit>,
        pools: &mut ClientPools,
        serialized: &SerializedData,
        track_mutate_messages: bool,
        server_tick_range: Range<usize>,
        server_tick: RepliconTick,
        system_tick: Tick,
        timestamp: Duration,
        max_size: usize,
    ) -> Result<usize> {
        const MAX_COUNT_SIZE: usize = usize::POSTCARD_MAX_SIZE;
        let mut tick_buffer = [0; RepliconTick::POSTCARD_MAX_SIZE];
        let update_tick = postcard::to_slice(&ticks.update_tick(), &mut tick_buffer)?;
        let mut metadata_size = update_tick.len() + server_tick_range.len();
        if track_mutate_messages {
            metadata_size += MAX_COUNT_SIZE;
        }

        let mut mutate_info = MutateInfo {
            system_tick,
            server_tick,
            timestamp,
            entities: pools.entities.pop().unwrap_or_default(),
        };
        let mut mutate_index = ticks.next_mutate_index();
        let chunks = EntityChunks::new(&self.related, &self.standalone);
        let mut header_size = metadata_size + serialized_size(&mutate_index)?;
        let mut body_size = 0;
        let mut chunks_range = Range::<usize>::default();
        for chunk in chunks.iter() {
            let mut mutations_size = 0;
            for mutations in chunk {
                mutations_size += mutations.ranges.size_with_components_size()?;
            }

            // Try to pack back first, then try to pack forward.
            if body_size != 0
                && !can_pack(header_size + body_size, mutations_size, max_size)
                && !can_pack(header_size + mutations_size, body_size, max_size)
            {
                split_buffer.push(MutationsSplit {
                    mutate_index,
                    message_size: body_size + header_size,
                    chunks_range: chunks_range.clone(),
                });

                ticks.register_mutate_message(mutate_index, mutate_info);
                mutate_index = ticks.next_mutate_index();
                mutate_info = MutateInfo {
                    system_tick,
                    server_tick,
                    timestamp,
                    entities: pools.entities.pop().unwrap_or_default(),
                };
                chunks_range.start = chunks_range.end;
                header_size = metadata_size + serialized_size(&mutate_index)?; // Recalculate since the mutate index changed.
                body_size = 0;
            }

            mutate_info
                .entities
                .extend(chunk.iter().map(|mutations| mutations.entity));
            chunks_range.end += 1;
            body_size += mutations_size;
        }
        if !chunks_range.is_empty() || track_mutate_messages {
            // When the loop ends, pack all leftovers into a message.
            // Or create an empty message if tracking mutate messages is enabled.
            split_buffer.push(MutationsSplit {
                mutate_index,
                message_size: body_size + header_size,
                chunks_range,
            });
            ticks.register_mutate_message(mutate_index, mutate_info);
        }

        if split_buffer.len() > 1 {
            trace!(
                "splitting into {} messages for client `{client}`",
                split_buffer.len()
            );
        }

        for split in &*split_buffer {
            let mut message_size = split.message_size;
            if track_mutate_messages {
                // Update message counter size based on actual value.
                message_size -= MAX_COUNT_SIZE - serialized_size(&split_buffer.len())?;
            }
            let mut message = Vec::with_capacity(message_size);

            message.extend_from_slice(update_tick);
            message.extend_from_slice(&serialized[server_tick_range.clone()]);
            if track_mutate_messages {
                postcard_utils::to_extend_mut(&split_buffer.len(), &mut message)?;
            }
            postcard_utils::to_extend_mut(&split.mutate_index, &mut message)?;
            for mutations in chunks.iter_flatten(split.chunks_range.clone()) {
                message.extend_from_slice(&serialized[mutations.ranges.entity.clone()]);
                postcard_utils::to_extend_mut(&mutations.ranges.components_size(), &mut message)?;
                for component in &mutations.ranges.components {
                    message.extend_from_slice(&serialized[component.clone()]);
                }
            }

            debug_assert_eq!(message.len(), message_size);

            messages.send(client, ServerChannel::Mutations, message);
        }

        let len = split_buffer.len();
        split_buffer.clear();
        Ok(len)
    }

    /// Clears all entity mutations.
    ///
    /// Keeps allocated memory for reuse.
    ///
    /// The outer array of [`Self::related`] is not cleared.
    /// It should be resized via [`Self::resize_related`] before
    /// collecting new changes.
    pub(crate) fn clear(&mut self, pools: &mut ClientPools) {
        for entities in self
            .related
            .iter_mut()
            .chain(iter::once(&mut self.standalone))
        {
            let ranges = entities.drain(..).map(|mut mutations| {
                mutations.ranges.components.clear();
                mutations.ranges.components
            });
            pools.ranges.extend(ranges);
        }
    }

    /// Updates size of [`Self::related`] to split related entities by graph index.
    ///
    /// Keeps allocated memory for reuse.
    pub(crate) fn resize_related(&mut self, pools: &mut ClientPools, graphs_count: usize) {
        match self.related.len().cmp(&graphs_count) {
            Ordering::Less => self
                .related
                .resize_with(graphs_count, || pools.mutations.pop().unwrap_or_default()),
            Ordering::Greater => {
                let mutations = self.related.drain(graphs_count..).map(|mut mutations| {
                    mutations.clear();
                    mutations
                });
                pools.mutations.extend(mutations);
            }
            Ordering::Equal => (),
        }
    }
}

/// Mutations data for [`Mutations::related`] and [`Mutations::standalone`].
pub(crate) struct EntityMutations {
    /// Associated entity.
    ///
    /// Used to associate entities with the mutate message index that the client
    /// needs to acknowledge to consider entity mutations received.
    entity: Entity,

    /// Component mutations that happened in this tick.
    ///
    /// Serialized as a list of pairs of entity chunk and multiple chunks with mutated components.
    /// Components are stored in multiple chunks because some clients may acknowledge mutations,
    /// while others may not.
    ///
    /// Unlike with [`Updates`](super::updates::Updates), we serialize the number
    /// of chunk bytes instead of the number of components. This is because, during deserialization,
    /// some entities may be skipped if they have already been updated (as mutations are sent until
    /// the client acknowledges them).
    ranges: ChangeRanges,
}

#[derive(Clone, Copy)]
enum EntityLocation {
    Related { index: usize },
    Standalone,
}

/// Treats related and standalone entity mutations as a single continuous buffer,
/// with related entities first, followed by standalone ones.
struct EntityChunks<'a> {
    related: &'a [Vec<EntityMutations>],
    standalone: &'a [EntityMutations],
}

impl<'a> EntityChunks<'a> {
    fn new(related: &'a [Vec<EntityMutations>], standalone: &'a [EntityMutations]) -> Self {
        Self {
            related,
            standalone,
        }
    }

    /// Returns an iterator over slices of related entities.
    ///
    /// Standalone entities are represented as single-element slices.
    fn iter(&self) -> impl Iterator<Item = &[EntityMutations]> {
        self.related
            .iter()
            .map(Vec::as_slice)
            .chain(self.standalone.chunks(1))
    }

    /// Returns an iterator over flattened slices of entity mutations within the specified range.
    ///
    /// The range indexes chunk numbers (not individual elements).
    fn iter_flatten(&self, range: Range<usize>) -> impl Iterator<Item = &EntityMutations> {
        let total_len = self.related.len() + self.standalone.len();
        debug_assert!(range.start <= total_len);
        debug_assert!(range.end <= total_len);

        let split_point = self.related.len();

        let related_start = range.start.min(split_point);
        let related_end = range.end.min(split_point);
        let standalone_start = range.start.saturating_sub(split_point);
        let standalone_end = range.end.saturating_sub(split_point);

        let related_range = related_start..related_end;
        let standalone_range = standalone_start..standalone_end;

        self.related[related_range]
            .iter()
            .flatten()
            .chain(&self.standalone[standalone_range])
    }
}

/// Information about mutations that are split into a message.
///
/// We split mutations into messages first in order to know their count in advance.
pub(crate) struct MutationsSplit {
    mutate_index: MutateIndex,
    message_size: usize,
    /// Indices in [`EntityChunks`].
    chunks_range: Range<usize>,
}

/// Returns `true` if the additional data fits within the remaining space
/// of the current packet tail.
///
/// When the message already exceeds the MTU, more data can be packed
/// as long as it fits within the last partial packet without causing
/// an additional packet to be created.
fn can_pack(message_size: usize, add: usize, mtu: usize) -> bool {
    let dangling = message_size % mtu;
    (dangling > 0) && ((dangling + add) <= mtu)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_SIZE: usize = 1200;

    #[test]
    fn packing() {
        assert!(can_pack(10, 5, MAX_SIZE));
        assert!(can_pack(10, 1190, MAX_SIZE));
        assert!(!can_pack(10, 1191, MAX_SIZE));
        assert!(!can_pack(10, 3000, MAX_SIZE));

        assert!(can_pack(1500, 500, MAX_SIZE));
        assert!(can_pack(1500, 900, MAX_SIZE));
        assert!(!can_pack(1500, 1000, MAX_SIZE));

        assert!(can_pack(1199, 1, MAX_SIZE));
        assert!(!can_pack(1200, 0, MAX_SIZE));
        assert!(!can_pack(1200, 1, MAX_SIZE));
        assert!(!can_pack(1200, 3000, MAX_SIZE));
    }

    #[test]
    fn splitting() {
        assert_eq!(send([], [], false), 0);
        assert_eq!(send([], [10], false), 1);
        assert_eq!(send([], [1300], false), 1);
        assert_eq!(send([], [20, 20], false), 1);
        assert_eq!(send([], [700, 700], false), 2);
        assert_eq!(send([], [1300, 700], false), 1);
        assert_eq!(send([], [1300, 1300], false), 2);

        assert_eq!(send([&[10]], [], false), 1);
        assert_eq!(send([&[1300]], [], false), 1);
        assert_eq!(send([&[20, 20]], [], false), 1);
        assert_eq!(send([&[700, 700]], [], false), 1);
        assert_eq!(send([&[1300, 1300]], [], false), 1);
        assert_eq!(send([&[20], &[20]], [], false), 1);
        assert_eq!(send([&[700], &[700]], [], false), 2);
        assert_eq!(send([&[1300], &[1300]], [], false), 2);

        assert_eq!(send([&[10]], [10], false), 1);
        assert_eq!(send([&[1300]], [1300], false), 2);
        assert_eq!(send([&[20, 20]], [20, 20], false), 1);
        assert_eq!(send([&[700, 700]], [700, 700], false), 2);
        assert_eq!(send([&[1300, 1300]], [1300, 1300], false), 3);
        assert_eq!(send([&[20], &[20]], [20], false), 1);
        assert_eq!(send([&[700], &[700]], [700], false), 3);
        assert_eq!(send([&[1300], &[1300]], [1300], false), 3);

        assert_eq!(send([], [], true), 1);
        assert_eq!(send([], [10], true), 1);
        assert_eq!(send([&[10]], [], true), 1);
        assert_eq!(send([&[10]], [10], true), 1);
        assert_eq!(send([], [1194], true), 1);
    }

    /// Mocks message sending with specified data sizes.
    ///
    /// `related` and `standalone` specify sizes for entities and their mutations.
    /// See also [`write_entity`].
    fn send<const N: usize, const M: usize>(
        related: [&[usize]; N],
        standalone: [usize; M],
        track_mutate_messages: bool,
    ) -> usize {
        let mut serialized = SerializedData::default();
        let mut messages = ServerMessages::default();
        let mut mutations = Mutations::default();
        let mut pools = ClientPools::default();

        mutations.resize_related(&mut pools, related.len());

        for (index, &entities) in related.iter().enumerate() {
            for &mutations_size in entities {
                write_entity(
                    &mut mutations,
                    &mut serialized,
                    &mut pools,
                    Some(index),
                    mutations_size,
                );
            }
        }

        for &mutations_size in &standalone {
            write_entity(
                &mut mutations,
                &mut serialized,
                &mut pools,
                None,
                mutations_size,
            );
        }

        mutations
            .send(
                &mut messages,
                Entity::PLACEHOLDER,
                &mut Default::default(),
                &mut Default::default(),
                &mut Default::default(),
                &serialized,
                track_mutate_messages,
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                MAX_SIZE,
            )
            .unwrap()
    }

    /// Mocks writing an entity with a single mutated component of specified size.
    ///
    /// 4 bytes will be used for the entity, with the remaining space used by the component.
    /// All written data will be zeros.
    fn write_entity(
        mutations: &mut Mutations,
        serialized: &mut SerializedData,
        pools: &mut ClientPools,
        graph_index: Option<usize>,
        mutations_size: usize,
    ) {
        assert!(mutations_size > 4);
        let start = serialized.len();
        serialized.resize(start + mutations_size, 0);

        let entity_size = start + 4;
        mutations.start_entity();
        mutations.add_entity(pools, Entity::PLACEHOLDER, graph_index, start..entity_size);
        mutations.add_component(entity_size..serialized.len());
    }
}
