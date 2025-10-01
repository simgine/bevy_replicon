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
            .add_client_event::<Test>(Channel::Ordered)
            .finish();
    }
    server_app.init_resource::<EventReader<Test>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(Test);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<EventReader<Test>>();
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
        .add_mapped_client_event::<WithEntity>(Channel::Ordered)
        .finish();
    }
    server_app.init_resource::<EventReader<WithEntity>>();

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
        .client_trigger(WithEntity(client_entity));

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<EventReader<WithEntity>>();
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
                .disable::<ClientMessagePlugin>(),
        ))
        .add_client_event::<Test>(Channel::Ordered)
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
        .add_client_event::<Test>(Channel::Ordered)
        .finish();
    server_app.init_resource::<EventReader<Test>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(Test);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<EventReader<Test>>();
    assert_eq!(reader.events.len(), 1);
}

#[test]
fn local_sending() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, StatesPlugin, RepliconPlugins))
        .add_client_event::<Test>(Channel::Ordered)
        .finish();
    app.init_resource::<EventReader<Test>>();

    app.world_mut().client_trigger(Test);

    // Requires 2 updates because local sending runs
    // in `PostUpdate` and triggering runs in `PreUpdate`.
    app.update();
    app.update();

    let reader = app.world().resource::<EventReader<Test>>();
    assert_eq!(reader.events.len(), 1);
}

#[derive(Deserialize, Event, Serialize, Clone)]
struct Test;

#[derive(Deserialize, Event, Serialize, Clone, MapEntities)]
struct WithEntity(#[entities] Entity);

#[derive(Resource)]
struct EventReader<E: Event> {
    events: Vec<FromClient<E>>,
}

impl<E: Event + Clone> FromWorld for EventReader<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(|on: On<FromClient<E>>, mut reader: ResMut<Self>| {
            reader.events.push(on.event().clone());
        });

        Self {
            events: Default::default(),
        }
    }
}
