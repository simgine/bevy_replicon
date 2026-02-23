use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    prelude::*,
    shared::server_entity_map::ServerEntityMap,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn single() {
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
    server_app.exchange_with_client(&mut client_app);

    let client_entity = *client_app
        .world()
        .resource::<ServerEntityMap>()
        .to_client()
        .get(&server_entity)
        .unwrap();

    server_app.world_mut().despawn(server_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert!(client_app.world().get_entity(client_entity).is_err());

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    assert!(entity_map.to_client().is_empty());
    assert!(entity_map.to_server().is_empty());
}

#[test]
fn resource() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_resource::<TestResource>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().insert_resource(TestResource);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    assert!(client_app.world().contains_resource::<TestResource>());

    server_app.world_mut().remove_resource::<TestResource>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert!(!client_app.world().contains_resource::<TestResource>());
}

#[test]
fn with_relations() {
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

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, children![Replicated]))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 2);

    server_app.world_mut().despawn(server_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(remote.iter(client_app.world()).len(), 0);
}

#[test]
fn after_spawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Insert and remove `Replicated` to trigger spawn and despawn for client at the same time.
    server_app
        .world_mut()
        .spawn((Replicated, TestComponent))
        .remove::<Replicated>();

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(
        messages.drain_sent().count(),
        0,
        "client shouldn't receive anything for a despawned entity"
    );
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
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Signature::from(0)))
        .id();
    let client_entity = client_app
        .world_mut()
        .spawn((Replicated, Signature::from(0)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app.world_mut().despawn(server_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert!(client_app.world().get_entity(client_entity).is_err());
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

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, EntityVisibility))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app.world_mut().entity_mut(server_entity).despawn();

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(
        messages.drain_sent().count(),
        0,
        "client shouldn't receive anything for a hidden entity"
    );
}

#[test]
fn visibility_lose() {
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

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert(EntityVisibility);

    server_app.world_mut().spawn((Replicated, EntityVisibility));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 1);

    server_app
        .world_mut()
        .entity_mut(client)
        .remove::<EntityVisibility>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(remote.iter(client_app.world()).len(), 0);
}

#[test]
fn visibility_lose_with_component_scope() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<TestComponent>()
        .add_visibility_filter::<EntityVisibility>()
        .add_visibility_filter::<ComponentVisibility>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert((EntityVisibility, ComponentVisibility));

    server_app.world_mut().spawn((
        Replicated,
        TestComponent,
        EntityVisibility,
        ComponentVisibility,
    ));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&TestComponent>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    server_app
        .world_mut()
        .entity_mut(client)
        .remove::<(EntityVisibility, ComponentVisibility)>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
}

#[test]
fn with_visibility_gain_and_signature() {
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

    // Remove visibility filter to make the entity visible and unreplicate.
    server_app
        .world_mut()
        .spawn((Replicated, EntityVisibility, Signature::from(0)))
        .remove::<(Replicated, EntityVisibility)>();

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(messages.drain_sent().count(), 0);
}

#[derive(Component, Deserialize, Serialize)]
struct TestComponent;

#[derive(Resource, Deserialize, Serialize)]
struct TestResource;

#[derive(Component)]
#[component(immutable)]
struct EntityVisibility;

impl VisibilityFilter for EntityVisibility {
    type ClientComponent = Self;
    type Scope = Entity;

    fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
        component.is_some()
    }
}

#[derive(Component)]
#[component(immutable)]
struct ComponentVisibility;

impl VisibilityFilter for ComponentVisibility {
    type ClientComponent = Self;
    type Scope = SingleComponent<TestComponent>;

    fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
        component.is_some()
    }
}
