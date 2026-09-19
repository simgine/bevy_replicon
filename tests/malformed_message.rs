//! Replication messages whose announced sizes do not match the payload.

use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    bytes::Bytes,
    prelude::*,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn truncated_removals() {
    exchange_removal_message(1);
}

#[test]
fn intact_removals() {
    exchange_removal_message(0);
}

/// Replicates a removal for an entity the client never learned about, cutting
/// `cut` bytes off the end of every message the server sends.
fn exchange_removal_message(cut: usize) {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();
    server_app.update();

    // Drop the spawn message, so the entity stays unknown to the client.
    let client_entity = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .retain_sent(|(entity, _, _)| *entity != client_entity);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();
    server_app.update();

    exchange_truncated(&mut server_app, &mut client_app, cut);
    client_app.update();
}

/// Like [`ServerTestAppExt::exchange_with_client`], but cuts `cut` bytes off the
/// end of each message.
fn exchange_truncated(server_app: &mut App, client_app: &mut App, cut: usize) {
    let client_entity = **client_app.world().resource::<TestClientEntity>();

    let mut captured: Vec<(usize, Bytes)> = Vec::new();
    server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .retain_sent(|(entity, channel_id, message)| {
            if *entity == client_entity {
                let keep = message.len().saturating_sub(cut);
                captured.push((*channel_id, message.slice(..keep)));
                false
            } else {
                true
            }
        });

    let mut client_messages = client_app.world_mut().resource_mut::<ClientMessages>();
    for (channel_id, message) in captured {
        client_messages.insert_received(channel_id, message);
    }
}

#[derive(Component, Serialize, Deserialize)]
struct A;
