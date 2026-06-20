use bevy::{
    ecs::{component::ComponentId, entity::EntityAllocator},
    prelude::*,
};

use crate::{prelude::*, shared::server_entity_map::ServerEntityMap};

/// Replication context for serialization function.
#[non_exhaustive]
pub struct SerializeCtx<'a> {
    /// ID of the component entity.
    pub entity: Entity,

    /// ID of the serializing component.
    pub component_id: ComponentId,

    /// Current tick.
    pub server_tick: RepliconTick,

    /// Storage for serialization/deserialization state.
    pub storage: &'a mut ReplicationStorage,

    /// Registry of reflected types.
    pub type_registry: &'a AppTypeRegistry,
}

impl EntityStorageCtx for SerializeCtx<'_> {
    fn entity(&self) -> Entity {
        self.entity
    }

    fn storage(&self) -> &ReplicationStorage {
        self.storage
    }

    fn storage_mut(&mut self) -> &mut ReplicationStorage {
        self.storage
    }
}

/// Replication context for writing and deserialization.
#[non_exhaustive]
pub struct WriteCtx<'a> {
    /// ID of the component entity.
    pub entity: Entity,

    /// ID of the writing component.
    pub component_id: ComponentId,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,

    /// Maps server entities to client entities and vice versa.
    pub entity_map: &'a mut ServerEntityMap,

    /// Storage for serialization/deserialization state.
    pub storage: &'a mut ReplicationStorage,

    /// Registry of reflected types.
    pub type_registry: &'a AppTypeRegistry,

    /// World's entities to reserve IDs on new entities inside components.
    pub(crate) spawner: BufferedSpawner<'a>,

    /// Disables mapping logic to avoid spawning entities for consume functions.
    pub(crate) ignore_mapping: bool,
}

impl EntityStorageCtx for WriteCtx<'_> {
    fn entity(&self) -> Entity {
        self.entity
    }

    fn storage(&self) -> &ReplicationStorage {
        self.storage
    }

    fn storage_mut(&mut self) -> &mut ReplicationStorage {
        self.storage
    }
}

impl EntityMapper for WriteCtx<'_> {
    fn get_mapped(&mut self, server_entity: Entity) -> Entity {
        if self.ignore_mapping {
            return server_entity;
        }

        self.entity_map
            .server_entry(server_entity)
            .or_insert_with(|| self.spawner.spawn_empty())
    }

    fn set_mapped(&mut self, _source: Entity, _target: Entity) {
        unimplemented!()
    }
}

/// Replication context for removal.
#[non_exhaustive]
pub struct RemoveCtx {
    /// ID of the removing component.
    pub component_id: ComponentId,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}

/// Replication context for despawn.
#[non_exhaustive]
pub struct DespawnCtx {
    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,
}

/// Buffers entity spawns.
///
/// Used during entity mapping to avoid borrowing the world mutably.
pub(crate) struct BufferedSpawner<'a> {
    allocator: &'a EntityAllocator,
    spawn_buffer: &'a mut EntityBuffer,
}

impl<'a> BufferedSpawner<'a> {
    /// Creates a spawner with an empty buffer.
    pub(crate) fn new(allocator: &'a EntityAllocator, spawn_buffer: &'a mut EntityBuffer) -> Self {
        debug_assert!(spawn_buffer.is_empty(), "buffer should freed before reuse");
        Self {
            allocator,
            spawn_buffer,
        }
    }

    /// Buffers an empty entity spawn.
    fn spawn_empty(&mut self) -> Entity {
        let entity = self.allocator.alloc();
        self.spawn_buffer.push(entity);
        entity
    }
}

/// Entities allocated by [`BufferedSpawner`] that have not been spawned yet.
#[derive(Default, Deref)]
pub(crate) struct EntityBuffer(Vec<Entity>);

impl EntityBuffer {
    /// Spawns all buffered entities and clears the buffer.
    ///
    /// Should be called before inserting any components that store these
    /// entities, because hooks/observers may reference them during insertion.
    pub(crate) fn spawn(&mut self, world: &mut World) {
        for entity in self.0.drain(..) {
            world
                .spawn_empty_at(entity)
                .expect("all buffered entities must be valid");
        }
    }

    /// Frees all buffered entities without spawning them.
    pub(crate) fn free(&mut self, world: &mut World) {
        // TODO Bevy 0.19: user `free_many`.
        for entity in self.0.drain(..) {
            world.entities_allocator_mut().free(entity);
        }
    }

    pub(crate) fn push(&mut self, entity: Entity) {
        self.0.push(entity);
    }
}
