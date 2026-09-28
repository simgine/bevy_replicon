pub mod backend;
pub mod client_id;
pub mod message;
pub mod protocol;
pub mod replication;
pub mod replicon_tick;
pub mod server_entity_map;

use bevy::{
    ecs::{
        change_detection::MutUntyped,
        component::ComponentId,
        resource::ResourceEntities,
        world::{FilteredEntityMut, FilteredEntityRef},
    },
    prelude::*,
    ptr::Ptr,
};

use crate::prelude::*;
use backend::connected_client::NetworkIdMap;
use message::registry::RemoteMessageRegistry;
use replication::{
    receive_markers::ReceiveMarkers, registry::ReplicationRegistry, rules::ReplicationRules,
    signature::SignatureMap,
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
                targets: SendTargets::Single(client_info.client_id),
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
        app.init_state::<ClientState>()
            .init_state::<ServerState>()
            .init_resource::<ProtocolHasher>()
            .init_resource::<NetworkIdMap>()
            .init_resource::<RepliconChannels>()
            .init_resource::<ReplicationRegistry>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationStorage>()
            .init_resource::<SignatureMap>()
            .init_resource::<ReceiveMarkers>()
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
#[derive(Resource, Default, Debug, PartialEq, Eq, Clone, Copy)]
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

/// Fetches the resource with `resource_id` from `query`.
///
/// Returns [`None`] if the resource doesn't exist.
pub(crate) fn get_resource_by_id<'q>(
    resource_id: ComponentId,
    query: &'q Query<'_, '_, FilteredEntityRef<'_, '_>>,
    resource_entities: &ResourceEntities,
) -> Option<Ptr<'q>> {
    resource_entities
        .get(resource_id)
        .and_then(|entity| query.get(entity).ok())
        .and_then(|entity| entity.get_by_id(resource_id))
}

/// Fetches the resource entity for `resource_id` from `query`.
///
/// Returns [`None`] if the resource doesn't exist.
// Because of lifetime issues, we need to store the `FilteredEntityMut` returned by the queries.
// TODO: Replace these two functions with one that uses `FilteredEntityMut::into_mut_by_id` when
// it's upstream.
pub(crate) fn get_resource_entity_mut<'q, 's>(
    resource_id: ComponentId,
    query: &'q mut Query<'_, 's, FilteredEntityMut>,
    resource_entities: &ResourceEntities,
) -> Option<FilteredEntityMut<'q, 's>> {
    resource_entities
        .get(resource_id)
        .and_then(|entity| query.get_mut(entity).ok())
}

/// Fetches the resource with `resource_id` from `entity_mut`.
///
/// Returns [`None`] if the entity doesn't hold the resource, or the query didn't have mutable
/// access to the resource.
// Because of lifetime issues, we need to store the `FilteredEntityMut` returned by the queries.
// TODO: Replace these two functions with one that uses `FilteredEntityMut::into_mut_by_id` when
// it's upstream.
pub(crate) fn get_resource_mut_by_id<'e>(
    resource_id: ComponentId,
    entity_mut: &'e mut FilteredEntityMut<'_, '_>,
) -> Option<MutUntyped<'e>> {
    entity_mut.get_mut_by_id(resource_id)
}
