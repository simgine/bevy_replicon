pub mod deferred_entity;
pub(crate) mod mutate_index;
pub mod receive;
pub mod receive_markers;
pub mod registry;
pub mod rules;
pub mod send;
pub mod signature;
pub mod track_mutate_messages;
pub mod update_message_flags;

use bevy::prelude::*;

/// Marks an entity for replication on the server.
///
/// See also [`Remote`](crate::prelude::Remote).
#[derive(Component, Default, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct Replicated;
