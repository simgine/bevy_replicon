use bevy::prelude::*;

use crate::prelude::*;

/**
Extension for [`App`] to communicate with other instances like it's a server.

# Example

```
use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};

let mut server_app = App::new();
let mut client_app = App::new();
for app in [&mut server_app, &mut client_app] {
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        // No messaging library plugin required.
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)), // Tick each app update.
    ))
    .finish(); // Don't forget to call `finish`.
}

// Simulate connection between two apps:
// - server app will register a connected client,
// - client app will be in connected state.
server_app.connect_client(&mut client_app);

server_app.world_mut().spawn(Replicated);

// Run tick for each app and trigger message exchange.
server_app.update();
server_app.exchange_with_client(&mut client_app);
client_app.update();

let mut relicated = client_app.world_mut().query::<&Replicated>();
assert_eq!(
    relicated.iter(client_app.world()).len(),
    1,
    "client should replicate spawned entity"
);

// You can optionally simulate a disconnect.
server_app.disconnect_client(&mut client_app);
```
**/
pub trait ServerTestAppExt {
    /// Starts server in [`self`] and connects a client app.
    ///
    /// Can be called multiple times on different client apps.
    /// Internally updates both apps one time.
    ///
    /// Inserts [`TestClientEntity`] for tracking.
    ///
    /// # Panics
    ///
    /// Panics if a client app has been connected before.
    fn connect_client(&mut self, client_app: &mut App);

    /// Disconnects a client app from [`self`].
    ///
    /// Can be called multiple times on different client apps.
    /// Internally updates both apps once.
    ///
    /// # Panics
    ///
    /// Panics if a client app hasn't been connected before.
    fn disconnect_client(&mut self, client_app: &mut App);

    /// Exchanges messages between client and server.
    ///
    /// Internally updates [`self`] before sending and updates the client app after receiving.
    ///
    /// # Panics
    ///
    /// Panics if a client app hasn't been connected before.
    fn exchange_with_client(&mut self, client_app: &mut App);
}

impl ServerTestAppExt for App {
    fn connect_client(&mut self, client_app: &mut App) {
        self.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .as_mut()
            .set_if_neq(ServerState::Running);

        let client_entity = self
            .world_mut()
            .spawn(ConnectedClient { max_size: 1200 })
            .id();

        assert_eq!(
            *client_app.world().resource::<State<ClientState>>(),
            ClientState::Disconnected,
            "client can't be connected multiple times"
        );

        client_app
            .world_mut()
            .resource_mut::<NextState<ClientState>>()
            .as_mut()
            .set_if_neq(ClientState::Connected);

        client_app
            .world_mut()
            .insert_resource(TestClientEntity(client_entity));

        // Exchange messages to perform handshake.
        client_app.update();
        self.exchange_with_client(client_app);
        self.update();
        self.exchange_with_client(client_app);
        client_app.update();
    }

    fn disconnect_client(&mut self, client_app: &mut App) {
        client_app
            .world_mut()
            .resource_mut::<NextState<ClientState>>()
            .as_mut()
            .set_if_neq(ClientState::Disconnected);

        let client_entity = *client_app
            .world_mut()
            .remove_resource::<TestClientEntity>()
            .expect("client should have an assigned ID for disconnect");

        self.world_mut().despawn(client_entity);

        self.update();
        client_app.update();
    }

    fn exchange_with_client(&mut self, client_app: &mut App) {
        let client_entity = **client_app.world().resource::<TestClientEntity>();
        let mut client_messages = client_app.world_mut().resource_mut::<ClientMessages>();

        let mut server_messages = self.world_mut().resource_mut::<ServerMessages>();
        for (channel_id, message) in client_messages.drain_sent() {
            server_messages.insert_received(client_entity, channel_id, message)
        }

        server_messages.retain_sent(|(entity, channel_id, message)| {
            if *entity == client_entity {
                client_messages.insert_received(*channel_id, message.clone());
                false
            } else {
                true
            }
        })
    }
}

/// Stores connected client entity from server on client.
///
/// Inserted in [`ServerTestAppExt::connect_client`] and removed by [`ServerTestAppExt::disconnect_client`].
/// Used to track which client corresponds to which connection.
#[derive(Resource, Deref, Debug, Clone, Copy)]
pub struct TestClientEntity(Entity);
