use bevy::prelude::*;
use bytes::Bytes;
use log::trace;

/// Sent and received messages for exchange between Replicon and the messaging backend.
///
/// The messaging backend is responsible for updating this resource:
/// - Received messages should be forwarded to Replicon via [`Self::insert_received`] in
///   [`ServerSystems::ReceivePackets`](crate::prelude::ServerSystems::ReceivePackets).
/// - Replicon messages needs to be forwarded to the backend via [`Self::drain_sent`] in
///   [`ServerSystems::SendPackets`](crate::prelude::ServerSystems::SendPackets).
///
/// Inserted as resource by [`ServerPlugin`](crate::server::ServerPlugin).
#[derive(Resource, Default)]
pub struct ServerMessages {
    /// List of received messages for each channel.
    ///
    /// Top index is channel ID.
    /// Inner [`Vec`] stores received messages since the last tick.
    received_messages: Vec<Vec<(Entity, Bytes)>>,

    /// List of sent messages for each channel since the last tick.
    sent_messages: Vec<(Entity, usize, Bytes)>,
}

impl ServerMessages {
    /// Changes the size of the receive messages storage according to the number of client channels.
    pub(crate) fn setup_client_channels(&mut self, channels_count: usize) {
        self.received_messages.resize(channels_count, Vec::new());
    }

    /// Removes a disconnected client.
    pub(crate) fn remove_client(&mut self, client: Entity) {
        for receive_channel in &mut self.received_messages {
            receive_channel.retain(|&(entity, _)| entity != client);
        }
        self.sent_messages.retain(|&(entity, ..)| entity != client);
    }

    /// Receives all available messages from clients over a channel.
    ///
    /// All messages will be drained.
    pub(crate) fn receive<I: Into<usize>>(
        &mut self,
        channel_id: I,
    ) -> impl Iterator<Item = (Entity, Bytes)> + '_ {
        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get_mut(channel_id)
            .unwrap_or_else(|| panic!("server should have a receive channel with id {channel_id}"));

        if !channel_messages.is_empty() {
            trace!(
                "received {} message(s) totaling {} bytes from channel {channel_id}",
                channel_messages.len(),
                channel_messages
                    .iter()
                    .map(|(_, bytes)| bytes.len())
                    .sum::<usize>()
            );
        }

        channel_messages.drain(..)
    }

    /// Sends a message to a client over a channel.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn send<I: Into<usize>, B: Into<Bytes>>(
        &mut self,
        client: Entity,
        channel_id: I,
        message: B,
    ) {
        let channel_id = channel_id.into();
        let message: Bytes = message.into();

        trace!("sending {} bytes over channel {channel_id}", message.len());

        self.sent_messages.push((client, channel_id, message));
    }

    /// Retains only the messages specified by the predicate.
    ///
    /// Used for testing.
    pub(crate) fn retain_sent<F>(&mut self, f: F)
    where
        F: FnMut(&(Entity, usize, Bytes)) -> bool,
    {
        self.sent_messages.retain(f)
    }

    /// Removes all sent messages, returning them as an iterator with client entity and channel.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn drain_sent(&mut self) -> impl Iterator<Item = (Entity, usize, Bytes)> + '_ {
        self.sent_messages.drain(..)
    }

    /// Adds a message from a client to the list of received messages.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn insert_received<I: Into<usize>, B: Into<Bytes>>(
        &mut self,
        client: Entity,
        channel_id: I,
        message: B,
    ) {
        let channel_id = channel_id.into();
        let receive_channel = self
            .received_messages
            .get_mut(channel_id)
            .unwrap_or_else(|| panic!("server should have a receive channel with id {channel_id}"));

        receive_channel.push((client, message.into()));
    }

    pub(crate) fn clear(&mut self) {
        for receive_channel in &mut self.received_messages {
            receive_channel.clear();
        }
        self.sent_messages.clear();
    }
}

#[doc(hidden)]
#[deprecated = "Use `ServerMessages` instead"]
pub type RepliconServer = ServerMessages;
