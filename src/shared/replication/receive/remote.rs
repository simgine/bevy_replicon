use bevy::prelude::*;

/// Marker for entities spawned by replication.
///
/// Automatically inserted for each newly received entity.
///
/// See also [`Replicated`](crate::shared::replication::Replicated).
#[derive(Component, Default, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct Remote;
