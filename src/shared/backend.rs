//! API for messaging backends.
//!
//! We don't provide any traits to avoid Rust's orphan rule. Instead, backends are expected to:
//!
//! - Create channels defined in the [`RepliconChannels`](channels::RepliconChannels) resource.
//!   This can be done via an extension trait that provides a conversion which the user needs to call manually to get channels for the backend.
//! - Manage the [`ClientState`] and [`ServerState`] states.
//! - Update the [`ServerMessages`](server_messages::ServerMessages) and [`ClientMessages`](client_messages::ClientMessages) resources.
//! - Spawn and despawn entities with [`ConnectedClient`](connected_client::ConnectedClient) component.
//! - React on [`DisconnectRequest`] event.
//! - Optionally update statistic in [`ClientStats`] resource and components.
//!
//! This way, integrations can be provided as separate crates without requiring us or crate authors to maintain them under a feature.
//! See the documentation on types in this module for details.
//!
//! It's also recommended to split the crate into client and server plugins, along with `server` and `client` features.
//! This way, plugins can be conveniently disabled at compile time, which is useful for dedicated server or client
//! configurations.
//!
//! You can also use
//! [bevy_replicon_example_backend](https://github.com/simgine/bevy_replicon/tree/master/bevy_replicon_example_backend)
//! as a reference. For a real backend integration, see [bevy_replicon_renet](https://github.com/simgine/bevy_replicon_renet),
//! which we maintain.

pub mod channels;
pub mod client_messages;
pub mod connected_client;
pub mod server_messages;

use bevy::prelude::*;

/// Connection state of the client.
///
/// <div class="warning">
///
/// Should only be changed from the messaging backend when the client changes its state
/// in [`ClientSystems::ReceivePackets`](crate::client::ClientSystems::ReceivePackets).
///
/// </div>
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, States)]
pub enum ClientState {
    /// Not connected or trying to connect.
    #[default]
    Disconnected,
    /// Trying to connect to the server.
    Connecting,
    /// Connected to the server.
    Connected,
}

#[doc(hidden)]
#[deprecated = "Use `ClientState` instead"]
pub type RepliconClientStatus = ClientState;

/// Connection state of the server.
///
/// <div class="warning">
///
/// Should only be changed from the messaging backend when the server changes its state
/// in [`ServerSet::ReceivePackets`](crate::server::ServerSet::ReceivePackets).
///
/// </div>
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, States)]
pub enum ServerState {
    /// Inactive.
    #[default]
    Stopped,
    /// Accepting and handling client connections.
    Running,
}

#[doc(hidden)]
#[deprecated = "Use `ServerState` instead"]
pub type RepliconServerStatus = ServerState;

/// An event for the messaging backend to queue a disconnection
/// for a specific client on the server.
///
/// The disconnection should occur **after** all pending messages
/// for this client have been sent. The actual delivery of these
/// messages is not guaranteed.
#[derive(Event, Clone, Copy, Debug)]
pub struct DisconnectRequest {
    pub client: Entity,
}

/// Statistic for the current client when used as a resource,
/// or for a connected client when used as a component
/// on connected entities on the server.
///
/// All values can be zero if not provided by the backend.
///
/// <div class="warning">
///
/// Should only be modified from the messaging backend.
///
/// </div>
#[derive(Resource, Component, Debug, Clone, Copy, Default, Reflect)]
pub struct ClientStats {
    /// Round-time trip in seconds for the connection.
    pub rtt: f64,

    /// Packet loss % for the connection.
    pub packet_loss: f64,

    /// Bytes sent per second for the connection.
    pub sent_bps: f64,

    /// Bytes received per second for the connection.
    pub received_bps: f64,
}

#[doc(hidden)]
#[deprecated = "Use `ClientStats` instead"]
pub type NetworkStats = ClientStats;

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;
    use crate::{
        prelude::*,
        shared::backend::channels::{ClientChannel, ServerChannel},
    };

    #[test]
    fn client_to_server() {
        let channels = RepliconChannels::default();
        let mut client_messages = ClientMessages::default();
        client_messages.setup_server_channels(channels.server_channels().len());

        const MESSAGES: &[&[u8]] = &[&[0], &[1]];
        for &message in MESSAGES {
            client_messages.send(ClientChannel::MutationAcks, message);
        }

        let mut server_messages = ServerMessages::default();
        server_messages.setup_client_channels(channels.client_channels().len());

        for (channel_id, message) in client_messages.drain_sent() {
            server_messages.insert_received(Entity::PLACEHOLDER, channel_id, message);
        }

        let messages: Vec<_> = server_messages
            .receive(ClientChannel::MutationAcks)
            .map(|(_, message)| message)
            .collect();
        assert_eq!(messages, MESSAGES);
    }

    #[test]
    fn server_to_client() {
        let channels = RepliconChannels::default();
        let mut server_messages = ServerMessages::default();
        server_messages.setup_client_channels(channels.client_channels().len());

        const MESSAGES: &[&[u8]] = &[&[0], &[1]];
        for &message in MESSAGES {
            server_messages.send(Entity::PLACEHOLDER, ServerChannel::Mutations, message);
        }

        let mut client_messages = ClientMessages::default();
        client_messages.setup_server_channels(channels.server_channels().len());

        for (_, channel_id, message) in server_messages.drain_sent() {
            client_messages.insert_received(channel_id, message);
        }

        let messages: Vec<_> = client_messages.receive(ServerChannel::Mutations).collect();
        assert_eq!(messages, MESSAGES);
    }
}
