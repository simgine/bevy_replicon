//! API for messaging backends.
//!
//! We don't provide any traits to avoid Rust's orphan rule. Instead, backends are expected to:
//!
//! - Create channels defined in the [`RepliconChannels`](channels::RepliconChannels) resource.
//!   This can be done via an extension trait that provides a conversion which the user needs to call manually to get channels for the backend.
//! - Manage the [`ClientState`] and [`ServerState`] states.
//! - Update the [`RepliconServer`](replicon_server::RepliconServer) and [`RepliconClient`](replicon_client::RepliconClient) resources.
//! - Spawn and despawn entities with [`ConnectedClient`](connected_client::ConnectedClient) component.
//! - React on [`DisconnectRequest`] event.
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
pub mod connected_client;
pub mod replicon_client;
pub mod replicon_server;

use bevy::prelude::*;

/// Connection state of the client.
///
/// <div class="warning">
///
/// Should only be changed from the messaging backend when the client changes its state
/// in [`ClientSet::ReceivePackets`](crate::client::ClientSet::ReceivePackets).
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

/// Statistic associated with [`RepliconClient`](replicon_client::RepliconClient) or
/// [`ConnectedClient`](connected_client::ConnectedClient).
///
/// All values can be zero if not provided by the backend.
///
/// <div class="warning">
///
/// Should only be modified from the messaging backend.
///
/// </div>
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
pub struct NetworkStats {
    /// Round-time trip in seconds for the connection.
    pub rtt: f64,

    /// Packet loss % for the connection.
    pub packet_loss: f64,

    /// Bytes sent per second for the connection.
    pub sent_bps: f64,

    /// Bytes received per second for the connection.
    pub received_bps: f64,
}

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
        let mut client = RepliconClient::default();
        client.setup_server_channels(channels.server_channels().len());

        const MESSAGES: &[&[u8]] = &[&[0], &[1]];
        for &message in MESSAGES {
            client.send(ClientChannel::MutationAcks, message);
        }

        let mut server = RepliconServer::default();
        server.setup_client_channels(channels.client_channels().len());

        for (channel_id, message) in client.drain_sent() {
            server.insert_received(Entity::PLACEHOLDER, channel_id, message);
        }

        let messages: Vec<_> = server
            .receive(ClientChannel::MutationAcks)
            .map(|(_, message)| message)
            .collect();
        assert_eq!(messages, MESSAGES);
    }

    #[test]
    fn server_to_client() {
        let channels = RepliconChannels::default();
        let mut server = RepliconServer::default();
        server.setup_client_channels(channels.client_channels().len());

        const MESSAGES: &[&[u8]] = &[&[0], &[1]];
        for &message in MESSAGES {
            server.send(Entity::PLACEHOLDER, ServerChannel::Mutations, message);
        }

        let mut client = RepliconClient::default();
        client.setup_server_channels(channels.server_channels().len());

        for (_, channel_id, message) in server.drain_sent() {
            client.insert_received(channel_id, message);
        }

        let messages: Vec<_> = client.receive(ServerChannel::Mutations).collect();
        assert_eq!(messages, MESSAGES);
    }
}
