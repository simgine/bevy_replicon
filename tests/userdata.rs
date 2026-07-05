use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    client::UserdataReceived, prelude::*, server::ReplicationUserdata,
    shared::backend::channels::ServerChannel, test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn update_message() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .init_resource::<ReceivedUserdata>()
        .add_observer(receive_userdata)
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.extend_from_slice(&USERDATA.to_le_bytes());

    server_app.world_mut().spawn((Replicated, TestComponent));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    let messages = client_app.world().resource::<ClientMessages>();
    assert_eq!(messages.received_count(ServerChannel::Updates), 1);
    assert_eq!(messages.received_count(ServerChannel::Mutations), 0);

    client_app.update();

    let received = client_app.world().resource::<ReceivedUserdata>();
    assert_eq!(received.0, USERDATA);
}

#[test]
fn mutate_message() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .init_resource::<ReceivedUserdata>()
        .add_observer(receive_userdata)
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, TestComponent))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut component = server_app
        .world_mut()
        .get_mut::<TestComponent>(server_entity)
        .unwrap();
    component.set_changed();

    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.extend_from_slice(&USERDATA.to_le_bytes());

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    let messages = client_app.world().resource::<ClientMessages>();
    assert_eq!(messages.received_count(ServerChannel::Updates), 0);
    assert_eq!(messages.received_count(ServerChannel::Mutations), 1);

    client_app.update();

    let received = client_app.world().resource::<ReceivedUserdata>();
    assert_eq!(received.0, USERDATA);
}

const USERDATA: u32 = 42;

#[derive(Component, Deserialize, Serialize)]
struct TestComponent;

#[derive(Resource, Default)]
struct ReceivedUserdata(u32);

fn receive_userdata(received: On<UserdataReceived>, mut storage: ResMut<ReceivedUserdata>) {
    let bytes = received.bytes.as_ref().try_into().unwrap();
    storage.0 = u32::from_le_bytes(bytes);
}
