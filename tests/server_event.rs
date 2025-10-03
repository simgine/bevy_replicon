use bevy::{ecs::entity::MapEntities, prelude::*, state::app::StatesPlugin, time::TimePlugin};
use bevy_replicon::{
    client::ServerUpdateTick, prelude::*, shared::server_entity_map::ServerEntityMap,
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

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
        .add_server_event::<Test>(Channel::Ordered)
        .finish();
    }
    client_app.init_resource::<EventReader<Test>>();

    server_app.connect_client(&mut client_app);

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        message: Test,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<EventReader<Test>>();
    assert_eq!(reader.events.len(), 1);
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
        .add_mapped_server_event::<WithEntity>(Channel::Ordered)
        .finish();
    }
    client_app.init_resource::<EventReader<WithEntity>>();

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        message: WithEntity(server_entity),
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = *client_app
        .world()
        .resource::<ServerEntityMap>()
        .to_client()
        .get(&server_entity)
        .unwrap();

    let reader = client_app.world().resource::<EventReader<WithEntity>>();
    let mapped_entities: Vec<_> = reader.events.iter().map(|event| event.0).collect();
    assert_eq!(mapped_entities, [client_entity]);
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
                .set(ServerPlugin::new(PostUpdate))
                .disable::<ClientPlugin>()
                .disable::<ClientMessagePlugin>(),
        ))
        .add_server_event::<Test>(Channel::Ordered)
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
        .add_server_event::<Test>(Channel::Ordered)
        .finish();
    client_app.init_resource::<EventReader<Test>>();

    server_app.connect_client(&mut client_app);

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        message: Test,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<EventReader<Test>>();
    assert_eq!(reader.events.len(), 1);
}

#[test]
fn local_sending() {
    let mut app = App::new();
    app.add_plugins((
        TimePlugin,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .add_server_event::<Test>(Channel::Ordered)
    .finish();
    app.init_resource::<EventReader<Test>>();

    app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        message: Test,
    });

    // Requires 2 updates because local sending runs
    // in `PostUpdate` and triggering runs in `PreUpdate`.
    app.update();
    app.update();

    let reader = app.world().resource::<EventReader<Test>>();
    assert_eq!(reader.events.len(), 1);
}

#[test]
fn independent() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_server_event::<Test>(Channel::Ordered)
        .add_server_event::<Independent>(Channel::Ordered)
        .make_event_independent::<Independent>()
        .finish();
    }
    client_app
        .init_resource::<EventReader<Test>>()
        .init_resource::<EventReader<Independent>>();

    server_app.connect_client(&mut client_app);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially reset the update tick.
    // Normal events would be queued and not triggered yet,
    // but our independent event should be triggered immediately.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = Default::default();

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        message: Test,
    });
    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        message: Independent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<EventReader<Test>>();
    assert!(reader.events.is_empty());

    let independent_reader = client_app.world().resource::<EventReader<Independent>>();
    assert_eq!(independent_reader.events.len(), 1);
}

#[derive(Event, Serialize, Deserialize, Clone)]
struct Test;

#[derive(Event, Serialize, Deserialize, Clone)]
struct Independent;

#[derive(Event, Serialize, Deserialize, MapEntities, Clone)]
struct WithEntity(#[entities] Entity);

#[derive(Resource)]
struct EventReader<E: Event> {
    events: Vec<E>,
}

impl<E: Event + Clone> FromWorld for EventReader<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(|on: On<E>, mut reader: ResMut<Self>| {
            reader.events.push(on.event().clone());
        });

        Self {
            events: Default::default(),
        }
    }
}
