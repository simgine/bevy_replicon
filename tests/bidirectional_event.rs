use bevy::{ecs::event::Events, prelude::*, state::app::StatesPlugin};
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

    client_app.world_mut().send_event(TestEvent);
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    client_app.update();
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.update();

    let client_events = server_app
        .world()
        .resource::<Events<FromClient<TestEvent>>>();
    assert_eq!(
        client_events.len(),
        2,
        "server should get 2 events due to local resending"
    );
    assert_eq!(client_app.world().resource::<Events<TestEvent>>().len(), 1);
}

#[test]
fn trigger() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
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

#[derive(Event, Serialize, Deserialize, Clone)]
struct TestEvent;

#[derive(Resource)]
struct TriggerReader<E: Event> {
    events: Vec<E>,
}

impl<E: Event + Clone> FromWorld for TriggerReader<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(|trigger: Trigger<E>, mut counter: ResMut<Self>| {
            counter.events.push(trigger.event().clone());
        });

        Self {
            events: Default::default(),
        }
    }
}
