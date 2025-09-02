use bevy::prelude::*;
use bytes::Bytes;
use log::{debug, trace, warn};

use crate::prelude::*;

/// Stores information about a client independent from the messaging backend.
///
/// The messaging backend is responsible for updating this resource:
/// - When the messaging client changes its status, [`Self::set_status`] should be used to
///   reflect this change:
///   - [`RepliconClientStatus::Connecting`] and [`RepliconClientStatus::Connected`] should
///     be set in [`ClientSet::ReceivePackets`] to allow the client to process received
///     messages immediately.
///   - [`RepliconClientStatus::Disconnected`] should be set before [`ClientSet::Send`]
///     to allow the client to process
///     all received messages before disconnecting.
/// - For receiving messages, [`Self::insert_received`] should be to used.
///   A system to forward backend messages to Replicon should run in [`ClientSet::ReceivePackets`].
/// - For sending messages, [`Self::drain_sent`] should be used to drain all sent messages.
///   A system to forward Replicon messages to the backend should run in [`ClientSet::SendPackets`].
/// - Optionally update statistic using [`Self::stats_mut`].
///
/// Inserted as resource by [`ClientPlugin`].
#[derive(Resource, Default)]
pub struct RepliconClient {
    /// Client connection status.
    status: RepliconClientStatus,

    /// List of received messages for each channel.
    ///
    /// Top index is channel ID.
    /// Inner [`Vec`] stores received messages since the last tick.
    received_messages: Vec<Vec<Bytes>>,

    /// List of sent messages and their channels since the last tick.
    sent_messages: Vec<(usize, Bytes)>,

    stats: NetworkStats,
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
        if !self.is_connected() {
            // We can't return here because we need to return an empty iterator.
            warn!("trying to receive a message when the client is not connected");
        }

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
        if !self.is_connected() {
            warn!("trying to send a message when the client is not connected");
            return;
        }

        let channel_id = channel_id.into();
        let message: Bytes = message.into();

        trace!("sending {} bytes over channel {channel_id}", message.len());

        self.sent_messages.push((channel_id, message));
    }

    /// Sets the client connection status.
    ///
    /// Discards all messages if the state changes from [`RepliconClientStatus::Connected`].
    /// See also [`Self::status`].
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend when the client status changes.
    ///
    /// </div>
    pub fn set_status(&mut self, status: RepliconClientStatus) {
        debug!("changing status to `{status:?}`");

        if self.is_connected() && status != RepliconClientStatus::Connected {
            for channel_messages in &mut self.received_messages {
                channel_messages.clear();
            }
            self.sent_messages.clear();

            self.stats = Default::default();
        }

        self.status = status;
    }

    /// Returns the current client status.
    ///
    /// See also [`Self::set_status`].
    #[inline]
    pub fn status(&self) -> RepliconClientStatus {
        self.status
    }

    /// Returns `true` if the client is disconnected.
    ///
    /// See also [`Self::status`].
    #[inline]
    pub fn is_disconnected(&self) -> bool {
        self.status == RepliconClientStatus::Disconnected
    }

    /// Returns `true` if the client is connecting.
    ///
    /// See also [`Self::status`].
    #[inline]
    pub fn is_connecting(&self) -> bool {
        self.status == RepliconClientStatus::Connecting
    }

    /// Returns `true` if the client is connected.
    ///
    /// See also [`Self::status`].
    #[inline]
    pub fn is_connected(&self) -> bool {
        self.status == RepliconClientStatus::Connected
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
        if !self.is_connected() {
            warn!("trying to insert a received message when the client is not connected");
            return;
        }

        let channel_id = channel_id.into();
        let channel_messages = self
            .received_messages
            .get_mut(channel_id)
            .unwrap_or_else(|| panic!("client should have a channel with id {channel_id}"));

        channel_messages.push(message.into());
    }

    /// Returns network statistic.
    pub fn stats(&self) -> &NetworkStats {
        &self.stats
    }

    /// Returns a mutable reference to set network statistic.
    ///
    /// <div class="warning">
    ///
    /// Should only be called from the messaging backend.
    ///
    /// </div>
    pub fn stats_mut(&mut self) -> &mut NetworkStats {
        &mut self.stats
    }
}

/// Connection status of the [`RepliconClient`].
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum RepliconClientStatus {
    /// Not connected or trying to connect.
    #[default]
    Disconnected,
    /// Trying to connect to the server.
    Connecting,
    /// Connected to the server.
    Connected,
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;
    use crate::shared::backend::channels::{ClientChannel, ServerChannel};

    #[test]
    fn cleanup_on_disconnect() {
        let channels = RepliconChannels::default();
        let mut client = RepliconClient::default();
        client.setup_server_channels(channels.server_channels().len());
        client.set_status(RepliconClientStatus::Connected);

        client.send(ClientChannel::MutationAcks, Vec::new());
        client.insert_received(ServerChannel::Mutations, Vec::new());

        client.set_status(RepliconClientStatus::Disconnected);

        assert_eq!(client.drain_sent().count(), 0);
        assert_eq!(client.receive(ServerChannel::Mutations).count(), 0);
    }

    #[test]
    fn disconnected() {
        let channels = RepliconChannels::default();
        let mut client = RepliconClient::default();
        client.setup_server_channels(channels.server_channels().len());

        client.send(ClientChannel::MutationAcks, Vec::new());
        client.insert_received(ServerChannel::Mutations, Vec::new());

        assert_eq!(client.drain_sent().count(), 0);
        assert_eq!(client.receive(ServerChannel::Mutations).count(), 0);
    }
}
