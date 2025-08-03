use bevy::{
    ecs::{component::ComponentId, entity::Entities},
    prelude::*,
};

use crate::{prelude::*, shared::server_entity_map::ServerEntityMap};

/// Replication context for serialization function.
#[non_exhaustive]
pub struct SerializeCtx<'a> {
    /// ID of the serializing component.
    pub component_id: ComponentId,

    /// Current tick.
    pub server_tick: RepliconTick,

    /// Registry of reflected types.
    pub type_registry: &'a AppTypeRegistry,
}

/// Replication context for writing and deserialization.
#[non_exhaustive]
pub struct WriteCtx<'a> {
    /// Maps server entities to client entities and vice versa.
    pub entity_map: &'a mut ServerEntityMap,

    /// Registry of reflected types.
    pub type_registry: &'a AppTypeRegistry,

    /// ID of the writing component.
    pub component_id: ComponentId,

    /// Tick for the currently processing message.
    pub message_tick: RepliconTick,

    /// World's entities to reserve IDs on new entities inside components.
    pub(crate) entities: &'a Entities,

    /// Disables mapping logic to avoid spawning entities for consume functions.
    pub(crate) ignore_mapping: bool,
}

impl EntityMapper for WriteCtx<'_> {
    fn get_mapped(&mut self, server_entity: Entity) -> Entity {
        if self.ignore_mapping {
            return server_entity;
        }

        self.entity_map
            .server_entry(server_entity)
            .or_insert_with(|| self.entities.reserve_entity())
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
