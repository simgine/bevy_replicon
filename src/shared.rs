pub mod backend;
pub mod client_id;
pub mod message;
pub mod protocol;
pub mod replication;
pub mod replicon_tick;
pub mod server_entity_map;

use bevy::prelude::*;

use crate::prelude::*;
use backend::connected_client::NetworkIdMap;
use message::registry::RemoteMessageRegistry;
use replication::signature::SignatureMap;
use replication::{
    command_markers::CommandMarkers, registry::ReplicationRegistry, rules::ReplicationRules,
    track_mutate_messages::TrackMutateMessages,
};

/// Initializes types, resources and events needed for both client and server.
#[derive(Default)]
pub struct RepliconSharedPlugin {
    /**
    Configures the authorization process.

    # Examples

    Custom authorization to set a player name before starting replication.
    We re-use [`ProtocolMismatch`], which is registered only with
    [`AuthMethod::ProtocolCheck`], but it could be any event.

    ```
    use bevy::{prelude::*, state::app::StatesPlugin};
    use bevy_replicon::prelude::*;
    use serde::{Deserialize, Serialize};

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(RepliconSharedPlugin {
            auth_method: AuthMethod::Custom,
        }),
    ))
    .add_client_event::<ClientInfo>(Channel::Ordered)
    .add_server_event::<ProtocolMismatch>(Channel::Unreliable)
    .make_event_independent::<ProtocolMismatch>() // Let the client receive it without replication.
    .add_observer(start_game)
    .add_systems(OnEnter(ClientState::Connected), send_info);

    fn send_info(
        mut commands: Commands,
        protocol: Res<ProtocolHash>,
    ) {
        commands.client_trigger(ClientInfo {
            protocol: *protocol,
            player_name: "Shatur".to_string(), // Could be read from console or UI.
        });
    }

    fn start_game(
        client_info: On<FromClient<ClientInfo>>,
        mut commands: Commands,
        mut disconnects: MessageWriter<DisconnectRequest>,
        protocol: Res<ProtocolHash>,
    ) {
        let client = client_info
            .client_id
            .entity()
            .expect("protocol hash sent only from clients");

        // Since we are using custom authorization,
        // we need to verify the protocol manually.
        if client_info.protocol != *protocol {
            // Notify the client about the problem. No delivery
            // guarantee, since we disconnect after sending.
            commands.server_trigger(ToClients {
                mode: SendMode::Direct(client_info.client_id),
                message: ProtocolMismatch,
            });
            disconnects.write(DisconnectRequest { client });
        }

        // Validate player name, run the necessary game logic...

        // Manually mark the client as authorized.
        commands.entity(client).insert(AuthorizedClient);
    }

    /// A client event for custom authorization.
    #[derive(Event, Serialize, Deserialize)]
    struct ClientInfo {
        protocol: ProtocolHash,
        player_name: String,
    }
    ```
    **/
    pub auth_method: AuthMethod,
}

impl Plugin for RepliconSharedPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Replicated>()
            .register_type::<ConnectedClient>()
            .register_type::<NetworkIdMap>()
            .register_type::<ClientStats>()
            .init_state::<ClientState>()
            .init_state::<ServerState>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<NetworkIdMap>()
            .init_resource::<TrackMutateMessages>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .init_resource::<SignatureMap>()
            .init_resource::<CommandMarkers>()
            .init_resource::<RemoteMessageRegistry>()
            .insert_resource(self.auth_method)
            .add_message::<DisconnectRequest>();

        if self.auth_method == AuthMethod::ProtocolCheck {
            app.add_client_event::<ProtocolHash>(Channel::Ordered)
                .add_server_event::<ProtocolMismatch>(Channel::Unreliable)
                .make_event_independent::<ProtocolMismatch>();
        }
    }

    fn finish(&self, app: &mut App) {
        let protocol_hasher = app
            .world_mut()
            .remove_resource::<ProtocolHasher>()
            .expect("protocol hasher should be initialized at the plugin build");

        app.world_mut().insert_resource(protocol_hasher.finish());
    }
}

/// Configures the insertion of [`AuthorizedClient`].
///
/// Can be set via [`RepliconSharedPlugin::auth_method`].
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// Wait for receiving [`ProtocolHash`] event from the client.
    ///
    /// - If the hash differs from the server's, the client will be notified with
    ///   a [`ProtocolMismatch`] event and disconnected.
    /// - If the hash matches, the [`AuthorizedClient`] component will be inserted.
    #[default]
    ProtocolCheck,

    /// Consider all connected clients immediately authorized.
    ///
    /// [`AuthorizedClient`] will be configured as a required component for [`ConnectedClient`].
    ///
    /// Use with caution.
    None,

    /// Disable automatic insertion.
    ///
    /// The user is responsible for manually inserting [`AuthorizedClient`] on the server.
    Custom,
}
