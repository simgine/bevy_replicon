use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn event() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_client_event::<TestEvent>(Channel::Ordered)
            .add_server_event::<TestEvent>(Channel::Ordered)
            .finish();
    }

    server_app.connect_client(&mut client_app);

    client_app.world_mut().write_message(TestEvent);
    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    client_app.update();
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.update();

    let messages = server_app
        .world()
        .resource::<Messages<FromClient<TestEvent>>>();
    assert_eq!(
        messages.len(),
        2,
        "server should get 2 messages due to local resending"
    );
    assert_eq!(
        client_app.world().resource::<Messages<TestEvent>>().len(),
        1
    );
}

#[test]
fn trigger() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_client_trigger::<TestEvent>(Channel::Ordered)
        .add_server_trigger::<TestEvent>(Channel::Ordered)
        .finish();
    }
    server_app.init_resource::<TriggerReader<FromClient<TestEvent>>>();
    client_app.init_resource::<TriggerReader<TestEvent>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(TestEvent);
    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    client_app.update();
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.update();

    let server_reader = server_app
        .world()
        .resource::<TriggerReader<FromClient<TestEvent>>>();
    assert_eq!(server_reader.events.len(), 1);

    let client_reader = client_app.world().resource::<TriggerReader<TestEvent>>();
    assert_eq!(client_reader.events.len(), 1);
}

#[derive(Event, Message, Serialize, Deserialize, Clone)]
struct TestEvent;

#[derive(Resource)]
struct TriggerReader<E: Event> {
    events: Vec<E>,
}

impl<E: Event + Clone> FromWorld for TriggerReader<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(|on: On<E>, mut counter: ResMut<Self>| {
            counter.events.push(on.event().clone());
        });

        Self {
            events: Default::default(),
        }
    }
}
