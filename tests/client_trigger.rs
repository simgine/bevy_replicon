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
            .add_client_trigger::<TestEvent>(Channel::Ordered)
            .finish();
    }
    server_app.init_resource::<TriggerReader<TestEvent>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(TestEvent);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<TriggerReader<TestEvent>>();
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
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::PostUpdate,
                ..Default::default()
            }),
        ))
        .add_client_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    }
    server_app.init_resource::<TriggerReader<TestEvent>>();

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
        .client_trigger_targets(TestEvent, client_entity);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.entities, [server_entity]);
}

#[test]
fn mapped() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::PostUpdate,
                ..Default::default()
            }),
        ))
        .add_mapped_client_trigger::<EntityEvent>(Channel::Ordered)
        .finish();
    }
    server_app.init_resource::<TriggerReader<EntityEvent>>();

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
        .client_trigger(EntityEvent(client_entity));

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<TriggerReader<EntityEvent>>();
    let mapped_entities: Vec<_> = reader.events.iter().map(|event| event.0).collect();
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
        .add_client_trigger::<TestEvent>(Channel::Ordered)
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
        .add_client_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    server_app.init_resource::<TriggerReader<TestEvent>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(TestEvent);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.events.len(), 1);
}

#[test]
fn local_resending() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, StatesPlugin, RepliconPlugins))
        .add_client_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    app.init_resource::<TriggerReader<TestEvent>>();

    app.world_mut().client_trigger(TestEvent);

    // Requires 2 updates because local resending runs
    // in `PostUpdate` and triggering runs in `PreUpdate`.
    app.update();
    app.update();

    let reader = app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(reader.events.len(), 1);
}

#[derive(Deserialize, Event, Serialize, Clone)]
struct TestEvent;

#[derive(Deserialize, Event, Serialize, Clone, MapEntities)]
struct EntityEvent(#[entities] Entity);

#[derive(Resource)]
struct TriggerReader<E: Event> {
    events: Vec<FromClient<E>>,
    entities: Vec<Entity>,
}

impl<E: Event + Clone> FromWorld for TriggerReader<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(
            |trigger: Trigger<FromClient<E>>, mut counter: ResMut<Self>| {
                counter.events.push(trigger.event().clone());
                counter.entities.push(trigger.target());
            },
        );

        Self {
            events: Default::default(),
            entities: Default::default(),
        }
    }
}
