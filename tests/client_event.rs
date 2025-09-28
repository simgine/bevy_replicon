use bevy::{ecs::entity::MapEntities, prelude::*, state::app::StatesPlugin, time::TimePlugin};
use bevy_replicon::{
    prelude::*,
    shared::{event::registry::RemoteEventRegistry, server_entity_map::ServerEntityMap},
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn channels() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .add_message::<NonRemoteEvent>()
    .add_client_event::<TestEvent>(Channel::Ordered)
    .finish();

    let event_registry = app.world().resource::<RemoteEventRegistry>();
    assert_eq!(event_registry.client_channel::<NonRemoteEvent>(), None);
    assert_eq!(event_registry.client_channel::<TestEvent>(), Some(2));
}

#[test]
fn regular() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_client_event::<TestEvent>(Channel::Ordered)
            .finish();
    }

    server_app.connect_client(&mut client_app);

    client_app.world_mut().write_message(TestEvent);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let messages = server_app
        .world()
        .resource::<Messages<FromClient<TestEvent>>>();
    assert_eq!(messages.len(), 1);
}

#[test]
fn mapped() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_mapped_client_event::<EntityEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = *client_app
        .world()
        .resource::<ServerEntityMap>()
        .to_client()
        .get(&server_entity)
        .unwrap();

    client_app
        .world_mut()
        .write_message(EntityEvent(client_entity));

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let mapped_entities: Vec<_> = server_app
        .world_mut()
        .resource_mut::<Messages<FromClient<EntityEvent>>>()
        .drain()
        .map(|event| event.0)
        .collect();
    assert_eq!(mapped_entities, [server_entity]);
}

#[test]
fn without_plugins() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    server_app
        .add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .build()
                .disable::<ClientPlugin>()
                .disable::<ClientEventPlugin>(),
        ))
        .add_client_event::<TestEvent>(Channel::Ordered)
        .finish();
    client_app
        .add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .build()
                .disable::<ServerPlugin>()
                .disable::<ServerEventPlugin>(),
        ))
        .add_client_event::<TestEvent>(Channel::Ordered)
        .finish();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().write_message(TestEvent);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let messages = server_app
        .world()
        .resource::<Messages<FromClient<TestEvent>>>();
    assert_eq!(messages.len(), 1);
}

#[test]
fn local_resending() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, StatesPlugin, RepliconPlugins))
        .add_client_event::<TestEvent>(Channel::Ordered)
        .finish();

    app.world_mut().write_message(TestEvent);

    app.update();

    let messages = app.world().resource::<Messages<TestEvent>>();
    assert!(messages.is_empty());

    let client_messages = app.world().resource::<Messages<FromClient<TestEvent>>>();
    assert_eq!(client_messages.len(), 1);
}

#[derive(Message)]
struct NonRemoteEvent;

#[derive(Deserialize, Message, Serialize)]
struct TestEvent;

#[derive(Deserialize, Message, Serialize, Clone, MapEntities)]
struct EntityEvent(#[entities] Entity);
