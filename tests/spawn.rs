use bevy::{ecs::schedule::ScheduleLabel, prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    client::confirm_history::ConfirmHistory,
    prelude::*,
    shared::server_entity_map::ServerEntityMap,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn empty() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, With<Replicated>>()
        .single(client_app.world())
        .unwrap();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    assert_eq!(
        entity_map.to_client().get(&server_entity),
        Some(&client_entity),
        "server entity should be mapped to a replicated entity on client"
    );
    assert_eq!(
        entity_map.to_server().get(&client_entity),
        Some(&server_entity),
        "replicated entity on client should be mapped to a server entity"
    );
}

#[test]
fn with_component() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, A));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Replicated, &A)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn with_multiple_components() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .replicate::<B>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let before_archetypes = client_app.world().archetypes().len();

    server_app.world_mut().spawn((Replicated, A, B));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Replicated, &A, &B)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
    assert_eq!(
        client_app.world().archetypes().len() - before_archetypes,
        1,
        "should cause only a single archetype move"
    );
}

#[test]
fn with_old_component() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn an entity with replicated component, but without a marker.
    let server_entity = server_app.world_mut().spawn(A).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).len(), 0);

    // Enable replication for previously spawned entity
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Replicated, &A)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn before_connection() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    // Spawn an entity before client connected.
    server_app.world_mut().spawn((Replicated, A));

    server_app.update();

    server_app.connect_client(&mut client_app);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Replicated, &A)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn pre_spawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client_entity = client_app.world_mut().spawn_empty().id();
    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    let client = **client_app.world().resource::<TestClientEntity>();
    let mut entity_map = server_app
        .world_mut()
        .get_mut::<ClientEntityMap>(client)
        .unwrap();
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    assert_eq!(
        entity_map.to_client().get(&server_entity),
        Some(&client_entity),
        "server entity should be mapped to a replicated entity on client"
    );
    assert_eq!(
        entity_map.to_server().get(&client_entity),
        Some(&server_entity),
        "replicated entity on client should be mapped to a server entity"
    );

    let client_entity = client_app.world().entity(client_entity);
    assert!(
        client_entity.contains::<Replicated>(),
        "entity should start receive replication"
    );
    assert!(
        client_entity.contains::<ConfirmHistory>(),
        "server should confirm replication of client entity"
    );
    assert!(
        client_entity.contains::<A>(),
        "component from server should be replicated"
    );

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(
        replicated.iter(client_app.world()).count(),
        1,
        "new entity shouldn't be spawned on client"
    );
}

#[test]
fn after_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Remove and insert `Replicated` to trigger despawn and spawn for client at the same time.
    server_app
        .world_mut()
        .spawn((Replicated, A))
        .remove::<Replicated>()
        .insert(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Replicated, &A)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[derive(Component, Deserialize, Serialize)]
struct A;

#[derive(Component, Deserialize, Serialize)]
struct B;
