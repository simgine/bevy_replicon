use bevy::{
    ecs::{component::HookContext, world::DeferredWorld},
    platform::collections::HashMap,
    prelude::*,
};
use log::error;
use serde::{Deserialize, Serialize};

use crate::prelude::*;

/// Marker for a connected client.
///
/// Backends should spawn and despawn entities with this component on connect and disconnect
/// and optionally update the [`ClientStats`] component.
///
/// If the MTU of the connected client is dynamic, it's required for the backend to update
/// [`Self::max_size`] to ensure message splitting works properly.
///
/// `Entity` is used an identifier to refer to a client.
///
/// Needs to be inserted with [`NetworkId`] if the backend provides support for it.
///
/// <div class="warning">
///
/// Entities with this components should be spawned and despawned only from the messaging backend.
///
/// </div>
///
/// See also [`AuthorizedClient`].
#[derive(Component, Reflect)]
#[require(Name::new("Connected client"), ClientStats)]
pub struct ConnectedClient {
    /// Maximum size of a message that can be transferred over unreliable channel without
    /// splitting into multiple packets.
    ///
    /// Used to manually split mutations over packet-size messages to allow applying them partially.
    /// For more details on replication see [`ServerChannel`](super::channels::ServerChannel).
    ///
    /// <div class="warning">
    ///
    /// Should only be modified from the messaging backend.
    ///
    /// </div>
    pub max_size: usize,
}

/// Maps [`NetworkId`] to its associated entity.
///
/// Automatically updated on client entity spawns and despawns.
#[derive(Resource, Reflect, Default, Deref)]
pub struct NetworkIdMap(HashMap<NetworkId, Entity>);

/// A unique and persistent client ID provided by a messaging backend.
///
/// Used to identify the same client across reconnects if the backend supports
/// persistent identifiers.
///
/// This component needs to be optionally inserted alongside [`ConnectedClient`].
///
/// See also [`NetworkIdMap`].
///
/// <div class="warning">
///
/// This component should only be inserted by the messaging backend
/// and never removed until the entity is despawned.
///
/// </div>
#[derive(
    Component,
    Debug,
    Clone,
    Copy,
    Hash,
    PartialEq,
    Eq,
    Ord,
    PartialOrd,
    Reflect,
    Serialize,
    Deserialize,
)]
#[component(on_add = on_id_add, on_remove = on_id_remove)]
pub struct NetworkId(u64);

impl NetworkId {
    /// Creates a new ID wrapping the given value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Gets the value of this ID.
    pub fn get(&self) -> u64 {
        self.0
    }
}

fn on_id_add(mut world: DeferredWorld, ctx: HookContext) {
    let network_id = *world.get::<NetworkId>(ctx.entity).unwrap();
    let mut network_map = world.resource_mut::<NetworkIdMap>();
    if let Some(old_client) = network_map.0.insert(network_id, ctx.entity) {
        error!("backend-provided `{network_id:?}` was already mapped to client `{old_client}`");
    }
}

fn on_id_remove(mut world: DeferredWorld, ctx: HookContext) {
    let network_id = *world.get::<NetworkId>(ctx.entity).unwrap();
    let mut network_map = world.resource_mut::<NetworkIdMap>();
    network_map.0.remove(&network_id);
}
