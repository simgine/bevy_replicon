use bevy::{prelude::*, state::app::StatesPlugin};
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
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
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
        .query_filtered::<Entity, With<Remote>>()
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
fn component() {
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

    server_app.world_mut().spawn((Replicated, A));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Remote, &A)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn multiple_components() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
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

    let mut components = client_app.world_mut().query::<(&Remote, &A, &B)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
    assert_eq!(
        client_app.world().archetypes().len() - before_archetypes,
        1,
        "should cause only a single archetype move"
    );
}

#[test]
fn old_component() {
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

    // Spawn an entity with replicated component, but without a marker.
    let server_entity = server_app.world_mut().spawn(A).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 0);

    // Enable replication for previously spawned entity
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Remote, &A)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn empty_before_connection() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .finish();
    }

    // Spawn an entity before client connected.
    server_app.world_mut().spawn(Replicated);

    server_app.update();

    server_app.connect_client(&mut client_app);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).count(), 1);
}

#[test]
fn before_connection() {
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

    // Spawn an entity before client connected.
    server_app.world_mut().spawn((Replicated, A));

    server_app.update();

    server_app.connect_client(&mut client_app);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Remote, &A)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn signature() {
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

    let client_entity = client_app.world_mut().spawn(Signature::from(0)).id();
    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, A, Signature::from(0)))
        .id();

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
        client_entity.contains::<Remote>(),
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

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(
        remote.iter(client_app.world()).count(),
        1,
        "new entity shouldn't be spawned on client"
    );
}

#[test]
fn signature_before_connection() {
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

    let client_entity = client_app.world_mut().spawn(Signature::from(0)).id();
    server_app
        .world_mut()
        .spawn((Replicated, A, Signature::from(0)));

    server_app.update();

    server_app.connect_client(&mut client_app);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert!(client_app.world().get::<A>(client_entity).is_some());
}

#[test]
fn signature_before_replication() {
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

    let client_entity = client_app.world_mut().spawn(Signature::from(0)).id();
    let server_entity = server_app.world_mut().spawn(Signature::from(0)).id();

    server_app.update();

    let mut server_messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(server_messages.drain_sent().count(), 0);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert!(
        client_app
            .world()
            .get::<Replicated>(client_entity)
            .is_some()
    );
}

#[test]
fn signature_for_client() {
    let mut server_app = App::new();
    let mut client_app1 = App::new();
    let mut client_app2 = App::new();
    for app in [&mut server_app, &mut client_app1, &mut client_app2] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app1);
    server_app.connect_client(&mut client_app2);

    let client1 = **client_app1.world().resource::<TestClientEntity>();

    let client_entity1 = client_app1.world_mut().spawn(Signature::from(0)).id();
    let client_entity2 = client_app2.world_mut().spawn(Signature::from(0)).id();
    server_app
        .world_mut()
        .spawn((Replicated, A, Signature::from(0).for_client(client1)));

    server_app.update();
    server_app.exchange_with_client(&mut client_app1);
    client_app1.update();
    server_app.exchange_with_client(&mut client_app2);
    client_app2.update();

    assert!(client_app1.world().get::<A>(client_entity1).is_some());
    assert!(client_app2.world().get::<A>(client_entity2).is_none());
}

#[test]
fn before_started_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .set(RepliconSharedPlugin {
                    auth_method: AuthMethod::Custom,
                })
                .set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, A));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(
        components.iter(client_app.world()).count(),
        0,
        "no entities should have been sent to the client"
    );

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert(AuthorizedClient);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn after_started_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .set(RepliconSharedPlugin {
                    auth_method: AuthMethod::Custom,
                })
                .set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert(AuthorizedClient);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, A));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn hidden_entity() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_visibility_filter::<EntityVisibility>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, EntityVisibility));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 0);
}

#[test]
fn visibility_gain() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .add_visibility_filter::<EntityVisibility>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app
        .world_mut()
        .spawn((Replicated, EntityVisibility, A));

    server_app.update();

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).len(), 0);

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert(EntityVisibility);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 1);
}

#[test]
fn visibility_gain_with_signature() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .add_visibility_filter::<EntityVisibility>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client_entity = client_app.world_mut().spawn(Signature::from(0)).id();
    server_app
        .world_mut()
        .spawn((Replicated, EntityVisibility, Signature::from(0), A));

    server_app.update();

    let mut server_messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(server_messages.drain_sent().count(), 0);

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert(EntityVisibility);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert!(client_app.world().get::<A>(client_entity).is_some());
}

#[derive(Component, Deserialize, Serialize)]
struct A;

#[derive(Component, Deserialize, Serialize)]
struct B;

#[derive(Component)]
#[component(immutable)]
struct EntityVisibility;

impl VisibilityFilter for EntityVisibility {
    type Scope = Entity;

    fn is_visible(&self, _entity_filter: &Self) -> bool {
        true
    }
}
