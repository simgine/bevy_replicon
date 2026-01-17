pub mod client_ticks;
pub mod command_markers;
pub mod deferred_entity;
pub(crate) mod mutate_index;
pub mod registry;
pub mod rules;
pub mod signature;
pub mod track_mutate_messages;
pub mod update_message_flags;

use bevy::prelude::*;

/// Marks an entity for replication on the server.
///
/// After replication, client entities will also have this component,
/// so it can be used to run shared logic for networked entities.
///
/// See also [`Remote`](crate::prelude::Remote).
#[derive(Component, Default, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct Replicated;
