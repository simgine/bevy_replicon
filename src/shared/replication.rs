pub mod client_ticks;
pub mod deferred_entity;
pub mod diff;
pub mod message_flags;
pub(crate) mod mutate_index;
pub mod receive_markers;
pub mod registry;
pub mod rules;
pub mod signature;
pub mod storage;
pub mod track_mutate_messages;
pub mod visibility;

use bevy::prelude::*;

/// Marks an entity for replication on the server.
///
/// See also [`Remote`](crate::prelude::Remote).
#[derive(Component, Default, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct Replicated;
