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
        .add_server_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    }
    client_app.init_resource::<TriggerReader<TestEvent>>();

    server_app.connect_client(&mut client_app);

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.entities, [Entity::PLACEHOLDER]);
}

#[test]
fn with_target() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_server_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    }
    client_app.init_resource::<TriggerReader<TestEvent>>();

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.world_mut().server_trigger_targets(
        ToClients {
            mode: SendMode::Broadcast,
            event: TestEvent,
        },
        server_entity,
    );

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = *client_app
        .world()
        .resource::<ServerEntityMap>()
        .to_client()
        .get(&server_entity)
        .unwrap();

    let reader = client_app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.entities, [client_entity]);
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
        .add_mapped_server_trigger::<EntityEvent>(Channel::Ordered)
        .finish();
    }
    client_app.init_resource::<TriggerReader<EntityEvent>>();

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: EntityEvent(server_entity),
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

    let reader = client_app.world().resource::<TriggerReader<EntityEvent>>();
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
                .disable::<ClientEventPlugin>(),
        ))
        .add_server_trigger::<TestEvent>(Channel::Ordered)
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
        .add_server_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    client_app.init_resource::<TriggerReader<TestEvent>>();

    server_app.connect_client(&mut client_app);

    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.events.len(), 1);
}

#[test]
fn local_resending() {
    let mut app = App::new();
    app.add_plugins((
        TimePlugin,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .add_server_trigger::<TestEvent>(Channel::Ordered)
    .finish();
    app.init_resource::<TriggerReader<TestEvent>>();

    app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    // Requires 2 updates because local resending runs
    // in `PostUpdate` and triggering runs in `PreUpdate`.
    app.update();
    app.update();

    let reader = app.world().resource::<TriggerReader<TestEvent>>();
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
        .add_server_trigger::<TestEvent>(Channel::Ordered)
        .add_server_trigger::<IndependentEvent>(Channel::Ordered)
        .make_trigger_independent::<IndependentEvent>()
        .finish();
    }
    client_app
        .init_resource::<TriggerReader<TestEvent>>()
        .init_resource::<TriggerReader<IndependentEvent>>();

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
        event: TestEvent,
    });
    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: IndependentEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let reader = client_app.world().resource::<TriggerReader<TestEvent>>();
    assert!(reader.events.is_empty());

    let independent_reader = client_app
        .world()
        .resource::<TriggerReader<IndependentEvent>>();
    assert_eq!(independent_reader.events.len(), 1);
}

#[derive(Event, Serialize, Deserialize, Clone)]
struct TestEvent;

#[derive(Event, Serialize, Deserialize, Clone)]
struct IndependentEvent;

#[derive(Event, Serialize, Deserialize, MapEntities, Clone)]
struct EntityEvent(#[entities] Entity);

#[derive(Resource)]
struct TriggerReader<E: Event> {
    events: Vec<E>,
    entities: Vec<Entity>,
}

impl<E: Event + Clone> FromWorld for TriggerReader<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(|on: On<E>, mut counter: ResMut<Self>| {
            counter.events.push(on.event().clone());
            // counter.entities.push(on.target());
        });

        Self {
            events: Default::default(),
            entities: Default::default(),
        }
    }
}
