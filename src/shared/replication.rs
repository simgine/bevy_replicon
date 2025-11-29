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

/// Marks entity for replication.
#[derive(Component, Default, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct Replicated;
