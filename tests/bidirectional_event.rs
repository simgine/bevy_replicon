use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn message() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_client_message::<Test>(Channel::Ordered)
            .add_server_message::<Test>(Channel::Ordered)
            .finish();
    }

    server_app.connect_client(&mut client_app);

    client_app.world_mut().write_message(Test);
    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: Test,
    });

    client_app.update();
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.update();

    let messages = server_app.world().resource::<Messages<FromClient<Test>>>();
    assert_eq!(
        messages.len(),
        2,
        "server should get 2 messages due to local resending"
    );
    assert_eq!(client_app.world().resource::<Messages<Test>>().len(), 1);
}

#[test]
fn event() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_client_event::<Test>(Channel::Ordered)
        .add_server_event::<Test>(Channel::Ordered)
        .finish();
    }
    server_app.init_resource::<EventReader<FromClient<Test>>>();
    client_app.init_resource::<EventReader<Test>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(Test);
    server_app.world_mut().server_trigger(ToClients {
        mode: SendMode::Broadcast,
        message: Test,
    });

    client_app.update();
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.update();

    let server_reader = server_app
        .world()
        .resource::<EventReader<FromClient<Test>>>();
    assert_eq!(server_reader.events.len(), 1);

    let client_reader = client_app.world().resource::<EventReader<Test>>();
    assert_eq!(client_reader.events.len(), 1);
}

#[derive(Message, Event, Serialize, Deserialize, Clone)]
struct Test;

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
