use bevy::prelude::*;

use crate::prelude::*;

pub trait TrackAppExt {
    /// Enables mutate messages tracking.
    ///
    /// Server will start sending mutate messages each tick even if they empty
    /// and include the amount of the messages for each header.
    ///
    /// Client will track the received messages using
    /// [`ServerMutateTicks`](crate::client::server_mutate_ticks::ServerMutateTicks).
    ///
    /// Needs to be called by rollback crates to assume that the entity value didn't change
    /// on a tick if all updates were received and
    /// [`ConfirmHistory`](crate::client::confirm_history::ConfirmHistory)
    /// don't have this tick confirmed.
    fn track_mutate_messages(&mut self) -> &mut Self;
}

impl TrackAppExt for App {
    fn track_mutate_messages(&mut self) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .track_mutate_messages();

        self.world_mut().resource_mut::<TrackMutateMessages>().0 = true;
        self
    }
}

#[derive(Resource, Deref, Default, Debug, Clone, Copy)]
pub(crate) struct TrackMutateMessages(bool);
