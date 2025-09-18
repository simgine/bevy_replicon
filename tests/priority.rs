use test_log::test;

use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    prelude::*,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};

#[test]
fn regular() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false)))
        .id();

    let client = **client_app.world().resource::<TestClientEntity>();
    let mut priority = server_app
        .world_mut()
        .get_mut::<PriorityMap>(client)
        .unwrap();
    priority.insert(server_entity, 0.5);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&BoolComponent>();
    let component = components.single(client_app.world()).unwrap();
    assert!(!component.0, "mutation should be deprioritized");

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = components.single(client_app.world()).unwrap();
    assert!(component.0);
}

#[test]
fn with_miss() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false)))
        .id();

    let client = **client_app.world().resource::<TestClientEntity>();
    let mut priority = server_app
        .world_mut()
        .get_mut::<PriorityMap>(client)
        .unwrap();
    priority.insert(server_entity, 0.5);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&BoolComponent>();
    let component = components.single(client_app.world()).unwrap();
    assert!(!component.0, "mutation should be deprioritized");

    server_app.update();

    // Take and drop the mutation message.
    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(messages.drain_sent().count(), 1);

    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&BoolComponent>();
    let component = components.single(client_app.world()).unwrap();
    assert!(!component.0);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = components.single(client_app.world()).unwrap();
    assert!(component.0, "change should be resent");
}

#[derive(Clone, Component, Copy, Deserialize, Serialize)]
struct BoolComponent(bool);
