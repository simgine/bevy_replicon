use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::prelude::*;

/// Stores current [`RepliconTick`].
///
/// Used only on the server. Can represent your simulation step, and is made
/// available to the client in the custom deserialization, despawn, and component
/// removal functions.
///
/// The server sends replication data in [`ServerSet::Send`] any time this resource changes.
/// You can configure when the tick is incremented via [`ServerPlugin::tick_schedule`].
///
/// Note that component mutations are replicated over the unreliable channel.
/// If a component mutation message is lost, the mutation will not be resent
/// until the server's replication system runs again.
///
/// See [`ServerUpdateTick`](crate::client::ServerUpdateTick) for tracking the last received
/// tick on clients.
#[derive(Clone, Copy, Deref, Debug, Default, Deserialize, Resource, Serialize)]
pub struct ServerTick(RepliconTick);

impl ServerTick {
    /// Increments current tick by the specified `value` and takes wrapping into account.
    #[inline]
    pub fn increment_by(&mut self, value: u32) {
        self.0 += value;
    }

    /// Same as [`Self::increment_by`], but increments only by 1.
    #[inline]
    pub fn increment(&mut self) {
        self.increment_by(1)
    }
}
