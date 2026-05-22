#![deprecated(note = "renamed into `world_serialization")]

use bevy::prelude::*;

#[deprecated(note = "moved to `world_serialization::replicate_into`")]
pub fn replicate_into(dyn_world: &mut DynamicWorld, world: &World) {
    super::world_serialization::replicate_into(dyn_world, world);
}
