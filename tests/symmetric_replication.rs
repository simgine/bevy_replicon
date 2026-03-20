use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    prelude::*,
    shared::backend::{
        client_messages::ClientMessages, connected_client::ConnectedClient,
        server_messages::ServerMessages,
    },
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn exchanges_entities_between_peer_apps() {
    let (mut app_a, mut app_b) = create_peer_apps();
    let peers = connect_peers(&mut app_a, &mut app_b);

    app_a.world_mut().spawn((Replicated, AComponent));
    app_b.world_mut().spawn((Replicated, BComponent));

    propagate_initial_replication(&mut app_a, &mut app_b, &peers);

    let mut a_received = app_a
        .world_mut()
        .query_filtered::<Entity, (With<Remote>, With<BComponent>, Without<AComponent>)>();
    assert_eq!(a_received.iter(app_a.world()).count(), 1);
    let a_received = a_received.single(app_a.world()).unwrap();
    assert!(app_a.world().get::<Replicated>(a_received).is_none());

    let mut b_received = app_b
        .world_mut()
        .query_filtered::<Entity, (With<Remote>, With<AComponent>, Without<BComponent>)>();
    assert_eq!(b_received.iter(app_b.world()).count(), 1);
    let b_received = b_received.single(app_b.world()).unwrap();
    assert!(app_b.world().get::<Replicated>(b_received).is_none());
}

#[test]
fn received_entities_do_not_echo_back_on_next_flush() {
    let (mut app_a, mut app_b) = create_peer_apps();
    let peers = connect_peers(&mut app_a, &mut app_b);

    app_a.world_mut().spawn((Replicated, AComponent));
    app_b.world_mut().spawn((Replicated, BComponent));

    propagate_initial_replication(&mut app_a, &mut app_b, &peers);

    // Flushing the next round of packets should not create an echoed remote copy
    // of each app's own authoritative entity.
    exchange_peer_messages(&mut app_a, &mut app_b, &peers);
    app_a.update();
    app_b.update();

    let mut a_remotes = app_a.world_mut().query_filtered::<Entity, With<Remote>>();
    assert_eq!(a_remotes.iter(app_a.world()).count(), 1);
    let mut a_echoed = app_a
        .world_mut()
        .query_filtered::<Entity, (With<Remote>, With<AComponent>)>();
    assert_eq!(a_echoed.iter(app_a.world()).count(), 0);

    let mut b_remotes = app_b.world_mut().query_filtered::<Entity, With<Remote>>();
    assert_eq!(b_remotes.iter(app_b.world()).count(), 1);
    let mut b_echoed = app_b
        .world_mut()
        .query_filtered::<Entity, (With<Remote>, With<BComponent>)>();
    assert_eq!(b_echoed.iter(app_b.world()).count(), 0);
}

fn create_peer_apps() -> (App, App) {
    let mut app_a = App::new();
    let mut app_b = App::new();
    for app in [&mut app_a, &mut app_b] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .set(RepliconSharedPlugin {
                    auth_method: AuthMethod::None,
                })
                .set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<AComponent>()
        .replicate::<BComponent>()
        .finish();
    }

    (app_a, app_b)
}

fn connect_peers(app_a: &mut App, app_b: &mut App) -> PeerConnection {
    app_a
        .world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);
    app_a
        .world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::Connected);
    app_b
        .world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);
    app_b
        .world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::Connected);

    let b_on_a = app_a
        .world_mut()
        .spawn(ConnectedClient { max_size: 1200 })
        .id();
    let a_on_b = app_b
        .world_mut()
        .spawn(ConnectedClient { max_size: 1200 })
        .id();

    app_a.update();
    app_b.update();

    PeerConnection { a_on_b, b_on_a }
}

fn propagate_initial_replication(app_a: &mut App, app_b: &mut App, peers: &PeerConnection) {
    app_a.update();
    app_b.update();
    exchange_peer_messages(app_a, app_b, peers);
    app_a.update();
    app_b.update();
}

fn exchange_peer_messages(app_a: &mut App, app_b: &mut App, peers: &PeerConnection) {
    let a_client_messages: Vec<_> = app_a
        .world_mut()
        .resource_mut::<ClientMessages>()
        .drain_sent()
        .collect();
    let a_server_messages: Vec<_> = app_a
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .collect();
    let b_client_messages: Vec<_> = app_b
        .world_mut()
        .resource_mut::<ClientMessages>()
        .drain_sent()
        .collect();
    let b_server_messages: Vec<_> = app_b
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .collect();

    {
        let mut server_messages = app_a.world_mut().resource_mut::<ServerMessages>();
        for (channel_id, message) in b_client_messages {
            server_messages.insert_received(peers.b_on_a, channel_id, message);
        }
    }
    {
        let mut client_messages = app_a.world_mut().resource_mut::<ClientMessages>();
        for (client, channel_id, message) in b_server_messages {
            assert_eq!(client, peers.a_on_b);
            client_messages.insert_received(channel_id, message);
        }
    }
    {
        let mut server_messages = app_b.world_mut().resource_mut::<ServerMessages>();
        for (channel_id, message) in a_client_messages {
            server_messages.insert_received(peers.a_on_b, channel_id, message);
        }
    }
    {
        let mut client_messages = app_b.world_mut().resource_mut::<ClientMessages>();
        for (client, channel_id, message) in a_server_messages {
            assert_eq!(client, peers.b_on_a);
            client_messages.insert_received(channel_id, message);
        }
    }
}

struct PeerConnection {
    a_on_b: Entity,
    b_on_a: Entity,
}

#[derive(Component, Deserialize, Serialize)]
struct AComponent;

#[derive(Component, Deserialize, Serialize)]
struct BComponent;
