pub mod client_ticks;
pub mod deferred_entity;
pub(crate) mod mutate_index;
pub mod receive_markers;
pub mod registry;
pub mod rules;
pub mod signature;
pub mod track_mutate_messages;
pub mod update_message_flags;

use bevy::prelude::*;

/// Marks an entity for authoritative replication sending.
///
/// Typically inserted on server-owned entities. Received entities are marked
/// with [`Remote`](crate::prelude::Remote) instead.
///
/// See also [`Remote`](crate::prelude::Remote).
#[derive(Component, Default, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct Replicated;
