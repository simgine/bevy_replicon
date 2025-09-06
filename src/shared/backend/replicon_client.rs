use bevy::prelude::*;
use bytes::Bytes;
use log::trace;

/// Stores information about a client independent from the messaging backend.
///
/// The messaging backend is responsible for updating this resource:
/// - For receiving messages, [`Self::insert_received`] should be to used.
///   A system to forward backend messages to Replicon should run in
///   [`ClientSet::ReceivePackets`](crate::prelude::ClientSet::ReceivePackets).
/// - For sending messages, [`Self::drain_sent`] should be used to drain all sent messages.
///   A system to forward Replicon messages to the backend should run in
///   [`ClientSet::SendPackets`](crate::prelude::ClientSet::SendPackets).
///
/// Inserted as resource by [`ClientPlugin`](crate::prelude::ClientPlugin).
#[derive(Resource, Default)]
pub struct RepliconClient {
    /// List of received messages for each channel.
    ///
    /// Top index is channel ID.
    /// Inner [`Vec`] stores received messages since the last tick.
    received_messages: Vec<Vec<Bytes>>,

    /// List of sent messages and their channels since the last tick.
    sent_messages: Vec<(usize, Bytes)>,
}

impl RepliconClient {
    /// Changes the size of the receive messages storage according to the number of server channels.
    pub(crate) fn setup_server_channels(&mut self, channels_count: usize) {
        self.received_messages.resize(channels_count, Vec::new());
    }

    /// Returns number of received messages for a channel.
    ///
    /// See also [`Self::receive`].
    pub(crate) fn received_count<I: Into<usize>>(&self, channel_id: I) -> usize {
        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get(channel_id)
            .unwrap_or_else(|| panic!("client should have a receive channel with id {channel_id}"));

        channel_messages.len()
    }

    /// Receives all available messages from the server over a channel.
    ///
    /// All messages will be drained.
    pub(crate) fn receive<I: Into<usize>>(
        &mut self,
        channel_id: I,
    ) -> impl Iterator<Item = Bytes> + '_ {
        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get_mut(channel_id)
            .unwrap_or_else(|| panic!("client should have a receive channel with id {channel_id}"));

        if !channel_messages.is_empty() {
            trace!(
                "received {} message(s) totaling {} bytes from channel {channel_id}",
                channel_messages.len(),
                channel_messages
                    .iter()
                    .map(|bytes| bytes.len())
                    .sum::<usize>()
            );
        }

        channel_messages.drain(..)
    }

    /// Sends a message to the server over a channel.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn send<I: Into<usize>, B: Into<Bytes>>(&mut self, channel_id: I, message: B) {
        let channel_id = channel_id.into();
        let message: Bytes = message.into();

        trace!("sending {} bytes over channel {channel_id}", message.len());

        self.sent_messages.push((channel_id, message));
    }

    pub(crate) fn clear(&mut self) {
        for channel_messages in &mut self.received_messages {
            channel_messages.clear();
        }
        self.sent_messages.clear();
    }

    /// Removes all sent messages, returning them as an iterator with channel.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn drain_sent(&mut self) -> impl Iterator<Item = (usize, Bytes)> + '_ {
        self.sent_messages.drain(..)
    }

    /// Adds a message from the server to the list of received messages.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn insert_received<I: Into<usize>, B: Into<Bytes>>(&mut self, channel_id: I, message: B) {
        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get_mut(channel_id)
            .unwrap_or_else(|| panic!("client should have a channel with id {channel_id}"));

        channel_messages.push(message.into());
    }
}
