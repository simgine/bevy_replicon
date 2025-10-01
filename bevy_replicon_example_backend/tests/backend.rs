use std::{io, net::Ipv4Addr};

use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::prelude::*;
use bevy_replicon_example_backend::{ExampleClient, ExampleServer, RepliconExampleBackendPlugins};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn connect_disconnect() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
            RepliconExampleBackendPlugins,
        ))
        .finish();
    }

    setup(&mut server_app, &mut client_app).unwrap();

    let server_state = server_app.world().resource::<State<ServerState>>();
    assert_eq!(*server_state, ServerState::Running);

    let mut clients = server_app
        .world_mut()
        .query::<(&ConnectedClient, &AuthorizedClient)>();
    assert_eq!(clients.iter(server_app.world()).len(), 1);

    let client_state = client_app.world().resource::<State<ClientState>>();
    assert_eq!(*client_state, ClientState::Connected);

    let renet_client = client_app.world().resource::<ExampleClient>();
    assert!(renet_client.is_connected());

    client_app.world_mut().remove_resource::<ExampleClient>();

    client_app.update();
    server_app.update();

    assert_eq!(clients.iter(server_app.world()).len(), 0);

    let client_state = client_app.world().resource::<State<ClientState>>();
    assert_eq!(*client_state, ClientState::Disconnected);
}

#[test]
fn disconnect_request() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
            RepliconExampleBackendPlugins,
        ))
        .add_server_message::<TestEvent>(Channel::Ordered)
        .finish();
    }

    setup(&mut server_app, &mut client_app).unwrap();

    server_app.world_mut().spawn(Replicated);
    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: TestEvent,
    });

    let mut clients = server_app
        .world_mut()
        .query_filtered::<Entity, With<ConnectedClient>>();
    let client = clients.single(server_app.world()).unwrap();
    server_app
        .world_mut()
        .write_message(DisconnectRequest { client });

    server_app.update();
    client_app.update();

    assert_eq!(clients.iter(server_app.world()).len(), 0);

    let client_state = client_app.world().resource::<State<ClientState>>();
    assert_eq!(*client_state, ClientState::Disconnected);

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert_eq!(messages.len(), 1, "last message should be received");

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(
        replicated.iter(client_app.world()).len(),
        1,
        "last replication should be received"
    );
}

#[test]
fn server_stop() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
            RepliconExampleBackendPlugins,
        ))
        .add_server_message::<TestEvent>(Channel::Ordered)
        .finish();
    }

    setup(&mut server_app, &mut client_app).unwrap();

    server_app.world_mut().remove_resource::<ExampleServer>();
    server_app.world_mut().spawn(Replicated);
    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: TestEvent,
    });

    server_app.update();
    client_app.update();

    let mut clients = server_app.world_mut().query::<&ConnectedClient>();
    assert_eq!(clients.iter(server_app.world()).len(), 0);

    let server_state = server_app.world().resource::<State<ServerState>>();
    assert_eq!(*server_state, ServerState::Stopped);

    let client_state = client_app.world().resource::<State<ClientState>>();
    assert_eq!(*client_state, ClientState::Disconnected);

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert!(
        messages.is_empty(),
        "message shouldn't be received after stop"
    );

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(
        replicated.iter(client_app.world()).len(),
        0,
        "replication after stop shouldn't be received"
    );
}

#[test]
fn replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
            RepliconExampleBackendPlugins,
        ))
        .finish();
    }

    setup(&mut server_app, &mut client_app).unwrap();

    server_app.world_mut().spawn(Replicated);

    server_app.update();
    client_app.update();

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).len(), 1);
}

#[test]
fn server_event() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
            RepliconExampleBackendPlugins,
        ))
        .add_server_message::<TestEvent>(Channel::Ordered)
        .finish();
    }

    setup(&mut server_app, &mut client_app).unwrap();

    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: TestEvent,
    });

    server_app.update();
    client_app.update();

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert_eq!(messages.len(), 1);
}

#[test]
fn client_event() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
            RepliconExampleBackendPlugins,
        ))
        .add_client_message::<TestEvent>(Channel::Ordered)
        .finish();
    }

    setup(&mut server_app, &mut client_app).unwrap();

    client_app.world_mut().write_message(TestEvent);

    client_app.update();
    server_app.update();

    let messages = server_app
        .world()
        .resource::<Messages<FromClient<TestEvent>>>();
    assert_eq!(messages.len(), 1);
}

fn setup(server_app: &mut App, client_app: &mut App) -> io::Result<()> {
    let server_socket = ExampleServer::new(0)?;
    let server_addr = server_socket.local_addr()?;
    let client_socket = ExampleClient::new((Ipv4Addr::LOCALHOST, server_addr.port()))?;

    server_app.insert_resource(server_socket);
    client_app.insert_resource(client_socket);

    server_app.update();
    client_app.update();
    server_app.update();
    client_app.update();

    Ok(())
}

#[derive(Message, Serialize, Deserialize)]
struct TestEvent;
