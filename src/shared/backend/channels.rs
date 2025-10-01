use bevy::prelude::*;
use log::debug;

/// A resource with all channels used by Replicon.
///
/// Initialized in [`ClientPlugin::finish`](crate::client::ClientPlugin) and
/// [`ServerPlugin::finish`](crate::server::ServerPlugin).
///
/// The channels are divided into client and server channels.
/// On a client:
/// - [`Self::client_channels`] are used for sending.
/// - [`Self::server_channels`] are used for receiving.
///
/// On a server:
/// - [`Self::server_channels`] are used for sending.
/// - [`Self::client_channels`] are used for receiving.
///
/// If the backend does not distinguish between sending and receiving channels,
/// create channels for both client and server by chaining them.
///
/// Channel IDs are represented by [`usize`], but backends may limit the number of channels.
/// See [`ServerChannel`] and [`ClientChannel`] for channels that are always reserved.
/// Other channels are used for events, with one channel per event. For more details, see
/// [`RemoteEventRegistry`](crate::shared::event::registry::RemoteEventRegistry).
///
/// The backend needs to provide an API for creating its own channels. This can be done
/// by writing an extension trait for this struct. Created channels should have the defined
/// delivery guarantee or stronger.
#[derive(Clone, Resource)]
pub struct RepliconChannels {
    /// Stores settings for each server channel.
    server: Vec<Channel>,

    /// Same as [`Self::server`], but for client.
    client: Vec<Channel>,
}

/// Only stores the replication channel by default.
impl Default for RepliconChannels {
    fn default() -> Self {
        Self {
            server: vec![
                ServerChannel::Updates.into(),
                ServerChannel::Mutations.into(),
            ],
            client: vec![ClientChannel::MutationAcks.into()],
        }
    }
}

impl RepliconChannels {
    /// Creates a new server channel and returns its ID.
    pub(crate) fn create_server_channel(&mut self, channel: Channel) -> usize {
        let id = self.server.len();
        debug!("creating a server channel with ID {id}");
        self.server.push(channel);

        id
    }

    /// Creates a new client channel and returns its ID.
    pub(crate) fn create_client_channel(&mut self, channel: Channel) -> usize {
        let id = self.client.len();
        debug!("creating a client channel with ID {id}");
        self.client.push(channel);

        id
    }

    /// Returns the list of registered server channels, which are used for sending data from server to client.
    ///
    /// For example, if you register a client event, it won't be reflected here.
    pub fn server_channels(&self) -> &[Channel] {
        &self.server
    }

    /// Returns the list of registered client channels,
    /// which are used for sending data from client to server.
    ///
    /// For example, if you register a server event, it won't be reflected here.
    pub fn client_channels(&self) -> &[Channel] {
        &self.client
    }
}

/// Constant ID of a channel for sending data from server to client.
///
/// These channels are always reserved, though additional channels may be required
/// for remote server events. Use [`RepliconChannels`] to retrieve all channels required for Replicon.
///
/// See also [`ClientChannel`].
///
/// # Replication channels
///
/// To synchronize the state, we send only changes using Bevy's change detection.
///
/// We can't use only a reliable channel because of how reliability is implemented. Messages are split into packets
/// based on the MTU and are considered received only if all their packets are received. If any packet is dropped,
/// it gets resent with the same data. However, on the client, we care only about the latest data. For example:
///
/// - Tick 1, position X - received.
/// - Tick 2, position Y - missed.
/// - Tick 3, position Z - received.
///
/// By tick 3, we no longer care about the missing position from tick 2, but it will still be resent.
///
/// We also can't use only an unreliable channel. We could implement a custom acknowledgment system on top of it
/// and resend the latest data if a message is lost. However, partial updates would break the game logic.
/// For example, if a component that references an entity is lost, we can't resend it with the new entity
/// because the client might not have received the entity yet.
///
/// This is why we use a dual-channel approach to send data.
///
/// For everything except mutations, we use a reliable channel. This data can't be outdated and is sent in
/// a single update message for each tick to ensure atomic updates.
///
/// For mutations, we use an unreliable channel. This data can be outdated, so we always send the latest values
/// since the last acknowledgment. We also include a minimum required tick - the tick on which the last update
/// message was sent. Messages will be buffered until an update message for this tick is received. Mutations
/// are split into packet-size messages to allow applying them partially without waiting for all parts of the message.
/// We guarantee that all mutations for a single entity arrive won't be split across messages, even if they are larger
/// than the packet size. You can also ensure that mutations for specific entities arrive in sync by using
/// [`SyncRelatedAppExt::sync_related_entities`](crate::server::related_entities::SyncRelatedAppExt::sync_related_entities).
///
/// Server events also have minimum required tick. For details, see the documentation on
/// [`ServerMessageAppExt::make_message_independent`](crate::shared::event::server_event::ServerMessageAppExt::make_message_independent).
///
/// See also [`RepliconChannels`], [`Channel`] and [corresponding section](../index.html#eventual-consistency)
/// from the quick start guide.
pub enum ServerChannel {
    /// For sending messages with entity mappings, inserts, removals and despawns.
    ///
    /// This is an ordered reliable channel.
    Updates,
    /// For sending messages with component mutations.
    ///
    /// This is an unreliable channel.
    ///
    /// See also [`ClientChannel::MutationAcks`].
    Mutations,
}

impl From<ServerChannel> for Channel {
    fn from(value: ServerChannel) -> Self {
        match value {
            ServerChannel::Updates => Channel::Ordered,
            ServerChannel::Mutations => Channel::Unreliable,
        }
    }
}

impl From<ServerChannel> for usize {
    fn from(value: ServerChannel) -> Self {
        value as usize
    }
}

/// Constant ID of a channel for sending data from client to server.
///
/// These channels are always reserved, though additional channels may be required
/// for remote client events. Use [`RepliconChannels`] to retrieve all channels required for Replicon.
///
/// See also [`ServerChannel`].
pub enum ClientChannel {
    /// For sending acks to acknowledge mutation messages received from [`ServerChannel::Mutations`].
    ///
    /// This is an ordered reliable channel.
    MutationAcks,
}

impl From<ClientChannel> for Channel {
    fn from(value: ClientChannel) -> Self {
        match value {
            ClientChannel::MutationAcks => Channel::Ordered,
        }
    }
}

impl From<ClientChannel> for usize {
    fn from(value: ClientChannel) -> Self {
        value as usize
    }
}

/// Channel delivery guarantee.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    /// Unreliable and unordered.
    Unreliable,
    /// Reliable and unordered.
    Unordered,
    /// Reliable and ordered.
    Ordered,
}
