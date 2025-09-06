use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    prelude::*,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn empty_blacklist() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::PostUpdate,
                visibility_policy: VisibilityPolicy::Blacklist,
                ..Default::default()
            }),
        ))
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, TestComponent));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app
        .world_mut()
        .query::<(&Replicated, &TestComponent)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn blacklist() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::PostUpdate,
                visibility_policy: VisibilityPolicy::Blacklist,
                ..Default::default()
            }),
        ))
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, TestComponent))
        .id();

    let client = **client_app.world().resource::<TestClientEntity>();
    let mut visibility = server_app
        .world_mut()
        .get_mut::<ClientVisibility>(client)
        .unwrap();
    visibility.set_visibility(server_entity, false);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).len(), 0);

    // Reverse visibility back.
    let mut visibility = server_app
        .world_mut()
        .get_mut::<ClientVisibility>(client)
        .unwrap();
    visibility.set_visibility(server_entity, true);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app
        .world_mut()
        .query::<(&Replicated, &TestComponent)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn blacklist_with_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::PostUpdate,
                visibility_policy: VisibilityPolicy::Blacklist,
                ..Default::default()
            }),
        ))
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    let client = **client_app.world().resource::<TestClientEntity>();
    let mut visibility = server_app
        .world_mut()
        .get_mut::<ClientVisibility>(client)
        .unwrap();
    visibility.set_visibility(server_entity, false);
    server_app.world_mut().despawn(server_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).len(), 0);

    let visibility = server_app.world().get::<ClientVisibility>(client).unwrap();
    assert!(visibility.is_visible(server_entity)); // The missing entity must be removed from the list, so this should return `true`.
}

#[test]
fn empty_whitelist() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::PostUpdate,
                visibility_policy: VisibilityPolicy::Whitelist,
                ..Default::default()
            }),
        ))
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, TestComponent));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(
        replicated.iter(client_app.world()).len(),
        0,
        "no entities should be replicated without adding to whitelist"
    );
}

#[test]
fn whitelist() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::PostUpdate,
                visibility_policy: VisibilityPolicy::Whitelist,
                ..Default::default()
            }),
        ))
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, TestComponent))
        .id();

    let client = **client_app.world().resource::<TestClientEntity>();
    let mut visibility = server_app
        .world_mut()
        .get_mut::<ClientVisibility>(client)
        .unwrap();
    visibility.set_visibility(server_entity, true);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app
        .world_mut()
        .query::<(&Replicated, &TestComponent)>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    // Reverse visibility.
    let mut visibility = server_app
        .world_mut()
        .get_mut::<ClientVisibility>(client)
        .unwrap();
    visibility.set_visibility(server_entity, false);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(
        components.iter(client_app.world()).len(),
        0,
        "entity should be despawned after removing from whitelist"
    );
}

#[test]
fn whitelist_with_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::PostUpdate,
                visibility_policy: VisibilityPolicy::Whitelist,
                ..Default::default()
            }),
        ))
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    let client = **client_app.world().resource::<TestClientEntity>();
    let mut visibility = server_app
        .world_mut()
        .get_mut::<ClientVisibility>(client)
        .unwrap();
    visibility.set_visibility(server_entity, true);
    server_app.world_mut().despawn(server_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).len(), 0);

    let visibility = server_app.world().get::<ClientVisibility>(client).unwrap();
    assert!(!visibility.is_visible(server_entity));
}

#[derive(Component, Deserialize, Serialize)]
struct TestComponent;
