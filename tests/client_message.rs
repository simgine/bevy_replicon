use bevy::{ecs::entity::MapEntities, prelude::*, state::app::StatesPlugin, time::TimePlugin};
use bevy_replicon::{
    prelude::*, shared::server_entity_map::ServerEntityMap, test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn regular() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_client_message::<Test>(Channel::Ordered)
            .finish();
    }

    server_app.connect_client(&mut client_app);

    client_app.world_mut().write_message(Test);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let messages = server_app.world().resource::<Messages<FromClient<Test>>>();
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
        .add_mapped_client_message::<WithEntity>(Channel::Ordered)
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
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let mapped_entities: Vec<_> = server_app
        .world_mut()
        .resource_mut::<Messages<FromClient<WithEntity>>>()
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
                .disable::<ClientMessagePlugin>(),
        ))
        .add_client_message::<Test>(Channel::Ordered)
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
        .add_client_message::<Test>(Channel::Ordered)
        .finish();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().write_message(Test);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let messages = server_app.world().resource::<Messages<FromClient<Test>>>();
    assert_eq!(messages.len(), 1);
}

#[test]
fn local_sending() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, StatesPlugin, RepliconPlugins))
        .add_client_message::<Test>(Channel::Ordered)
        .finish();

    app.world_mut().write_message(Test);

    app.update();

    let messages = app.world().resource::<Messages<Test>>();
    assert!(messages.is_empty());

    let client_messages = app.world().resource::<Messages<FromClient<Test>>>();
    assert_eq!(client_messages.len(), 1);
}

#[derive(Deserialize, Message, Serialize)]
struct Test;

#[derive(Deserialize, Message, Serialize, Clone, MapEntities)]
struct WithEntity(#[entities] Entity);
