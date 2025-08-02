pub mod backend;
pub mod common_conditions;
pub mod entity_serde;
pub mod event;
pub mod protocol;
pub mod replication;
pub mod replicon_tick;
pub mod server_entity_map;

use bevy::prelude::*;

use crate::prelude::*;
use backend::connected_client::NetworkIdMap;
use event::remote_event_registry::RemoteEventRegistry;
use replication::{
    command_markers::CommandMarkers, replication_registry::ReplicationRegistry,
    replication_rules::ReplicationRules, track_mutate_messages::TrackMutateMessages,
};

/// Initializes types, resources and events needed for both client and server.
#[derive(Default)]
pub struct RepliconSharedPlugin {
    /**
    Configures the authorization process.

    # Examples

    Custom authorization to send chess square entities. The board is deterministically spawned
    on both client and server, and we wire their IDs to receive replication without sending the
    entire board data through the network. We re-use [`ProtocolMismatch`] that is registered
    only with [`AuthMethod::ProtocolCheck`], but it could be any event.

    ```
    use bevy::prelude::*;
    use bevy_replicon::prelude::*;
    use serde::{Deserialize, Serialize};

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        RepliconPlugins.set(RepliconSharedPlugin {
            auth_method: AuthMethod::Custom,
        }),
    ))
    .add_client_trigger::<ClientInfo>(Channel::Ordered)
    .add_server_trigger::<ProtocolMismatch>(Channel::Unreliable)
    .make_trigger_independent::<ProtocolMismatch>() // Let client receive it without replication.
    .add_observer(start_game)
    .add_systems(Update, send_info.run_if(client_just_connected));

    fn send_info(
        mut commands: Commands,
        protocol: Res<ProtocolHash>,
        squares: Query<(Entity, &Square)>,
    ) {
        // Sort deterministically to enable matching them on the server.
        let mut squares: Vec<_> = squares.iter().collect();
        squares.sort_by(|(_, a), (_, b)| (a.x, a.y).cmp(&(b.x, b.y)));

        let info = ClientInfo {
            protocol: *protocol,
            squares: squares.into_iter().map(|(entity, _)| entity).collect(),
        };
        commands.client_trigger(info);
    }

    fn start_game(
        trigger: Trigger<FromClient<ClientInfo>>,
        mut commands: Commands,
        mut events: EventWriter<DisconnectRequest>,
        protocol: Res<ProtocolHash>,
        squares: Query<(Entity, &Square)>,
    ) {
        // Since we using custom authorization,
        // we need to verify the protocol manually.
        if trigger.protocol != *protocol {
            // Notify client about the problem. No delivery
            // guarantee since we disconnect after sending.
            commands.server_trigger(ToClients {
                mode: SendMode::Direct(trigger.client),
                event: ProtocolMismatch,
            });
            events.write(DisconnectRequest {
                client: trigger.client,
            });
        }

        // Sort local square entities to match them with the received.
        let mut squares: Vec<_> = squares.iter().collect();
        squares.sort_by(|(_, a), (_, b)| (a.x, a.y).cmp(&(b.x, b.y)));

        // This map is a required component for `AuthorizedClient`.
        // By default it's empty, but we can initialize it with the
        // received entities.
        let mut entity_map = ClientEntityMap::default();
        for (&server_entity, &client_entity) in squares
            .iter()
            .map(|(entity, _)| entity)
            .zip(&trigger.squares)
        {
            entity_map.insert(server_entity, client_entity);
        }

        // Manually mark client as authorized and insert mappings.
        commands
            .entity(trigger.client)
            .insert((AuthorizedClient, entity_map));

        // Run other commands to start the game...
    }

    /// A client trigger with protocol information and client's chess board entities.
    #[derive(Event, Serialize, Deserialize)]
    struct ClientInfo {
        protocol: ProtocolHash,
        squares: Vec<Entity>,
    }

    /// A chessboard square.
    #[derive(Component, Serialize, Deserialize)]
    struct Square {
        x: u8,
        y: u8,
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
            .register_type::<NetworkStats>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<NetworkIdMap>()
            .init_resource::<TrackMutateMessages>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .init_resource::<CommandMarkers>()
            .init_resource::<RemoteEventRegistry>()
            .insert_resource(self.auth_method)
            .add_event::<DisconnectRequest>();

        if self.auth_method == AuthMethod::ProtocolCheck {
            app.add_client_trigger::<ProtocolHash>(Channel::Ordered)
                .add_server_trigger::<ProtocolMismatch>(Channel::Unreliable)
                .make_trigger_independent::<ProtocolMismatch>();
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

/// A placeholder entity for a connected client that refers to the listen server (when the server is also a client).
///
/// Equal to [`Entity::PLACEHOLDER`].
///
/// See also [`ToClients`] and [`FromClient`] events.
pub const SERVER: Entity = Entity::PLACEHOLDER;

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
