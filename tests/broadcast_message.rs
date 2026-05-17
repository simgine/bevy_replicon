use bevy::{ecs::entity::MapEntities, prelude::*, state::app::StatesPlugin, time::TimePlugin};
use bevy_replicon::{
    prelude::*,
    shared::server_entity_map::ServerEntityMap,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn regular() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_shared_message::<Test>(Channel::Ordered)
            .finish();
    }

    server_app.connect_client(&mut client_app);
    let client_entity = **client_app.world().resource::<TestClientEntity>();

    client_app.world_mut().write_message(Test);

    client_app.update();

    let local_messages: Vec<_> = client_app
        .world_mut()
        .resource_mut::<Messages<LocalOrRemote<Test>>>()
        .drain()
        .collect();
    assert_eq!(local_messages.len(), 1);
    assert_eq!(local_messages[0].sender, Sender::Local);

    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let remote_messages: Vec<_> = server_app
        .world_mut()
        .resource_mut::<Messages<LocalOrRemote<Test>>>()
        .drain()
        .collect();
    assert_eq!(remote_messages.len(), 1);
    assert_eq!(
        remote_messages[0].sender,
        Sender::Remote(ClientId::Client(client_entity))
    );
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
        .add_mapped_shared_message::<WithEntity>(Channel::Ordered)
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
        .write_message(WithEntity(client_entity));

    client_app.update();

    let local_entities: Vec<_> = client_app
        .world_mut()
        .resource_mut::<Messages<LocalOrRemote<WithEntity>>>()
        .drain()
        .map(|m| m.0)
        .collect();
    assert_eq!(local_entities, [client_entity]);

    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let mapped_entities: Vec<_> = server_app
        .world_mut()
        .resource_mut::<Messages<LocalOrRemote<WithEntity>>>()
        .drain()
        .map(|m| m.0)
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
                .disable::<ClientMessagePlugin>(),
        ))
        .add_shared_message::<Test>(Channel::Ordered)
        .finish();
    client_app
        .add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .build()
                .disable::<ServerPlugin>()
                .disable::<ServerMessagePlugin>(),
        ))
        .add_shared_message::<Test>(Channel::Ordered)
        .finish();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().write_message(Test);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let messages: Vec<_> = server_app
        .world_mut()
        .resource_mut::<Messages<LocalOrRemote<Test>>>()
        .drain()
        .collect();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].sender.is_remote());
}

#[test]
fn local_sending() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, StatesPlugin, RepliconPlugins))
        .add_shared_message::<Test>(Channel::Ordered)
        .finish();

    app.world_mut().write_message(Test);

    app.update();

    let messages = app.world().resource::<Messages<Test>>();
    assert!(messages.is_empty());

    let shared_messages: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<LocalOrRemote<Test>>>()
        .drain()
        .collect();
    assert_eq!(shared_messages.len(), 1);
    assert_eq!(shared_messages[0].sender, Sender::Local);
}

#[test]
fn with_disconnect() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_shared_message::<Test>(Channel::Ordered)
            .finish();
    }

    server_app.connect_client(&mut client_app);

    client_app.world_mut().write_message(Test);

    server_app.disconnect_client(&mut client_app);

    let messages = client_app.world().resource::<Messages<Test>>();
    assert!(messages.is_empty());

    let shared_messages = client_app
        .world()
        .resource::<Messages<LocalOrRemote<Test>>>();
    assert!(
        shared_messages.is_empty(),
        "client shouldn't resend shared messages locally after disconnect"
    );
}

#[derive(Deserialize, Message, Serialize)]
struct Test;

#[derive(Deserialize, Message, Serialize, Clone, MapEntities)]
struct WithEntity(#[entities] Entity);
