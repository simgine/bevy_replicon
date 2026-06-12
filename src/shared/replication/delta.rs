pub mod diffable;
pub mod rules;

use bevy::prelude::*;

use crate::shared::replicon_tick::RepliconTick;

#[derive(Component)]
pub struct Cached<T: Component> {
    /// Cached value of the last full snapshot.
    pub cached: T,
    /// Last tick `cached` was cached.
    pub last_tick: RepliconTick,
}
