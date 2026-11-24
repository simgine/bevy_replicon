use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    client::UserdataReceived,
    prelude::*,
    server::ReplicationUserdata,
    shared::backend::channels::{ClientChannel, ServerChannel},
    test_app::ServerTestAppExt,
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
    assert_eq!(**received, Some(USERDATA));
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
    assert_eq!(**received, Some(USERDATA));
}

#[test]
fn deferred_update() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .init_resource::<ReceivedUserdata>()
        .init_resource::<ReadyUserdata>()
        .add_observer(receive_userdata)
        .finish();
    }
    client_app.add_observer(apply_when_ready);

    server_app.connect_client(&mut client_app);

    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.extend_from_slice(&1u32.to_le_bytes());

    server_app.world_mut().spawn((Replicated, TestComponent));
    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.clear();
    userdata.extend_from_slice(&0u32.to_le_bytes());

    server_app.world_mut().spawn((Replicated, TestComponent));
    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    let updates_count = client_app
        .world()
        .resource::<ClientMessages>()
        .received_count(ServerChannel::Updates);
    assert_eq!(updates_count, 2);
    client_app.update();

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(
        remote.iter(client_app.world()).len(),
        0,
        "the ready second update must not pass the deferred first update"
    );
    assert!(client_app.world().resource::<ReceivedUserdata>().is_none());

    **client_app.world_mut().resource_mut::<ReadyUserdata>() = 1;
    client_app.update();

    assert_eq!(remote.iter(client_app.world()).len(), 2);
    assert_eq!(**client_app.world().resource::<ReceivedUserdata>(), Some(0));
}

#[test]
fn deferred_mutation() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .init_resource::<ReceivedUserdata>()
        .init_resource::<ReadyUserdata>()
        .add_observer(receive_userdata)
        .replicate::<ValueComponent>()
        .finish();
    }
    client_app.add_observer(apply_when_ready);

    server_app.connect_client(&mut client_app);

    // Defer the spawn update. It stays buffered and unapplied.
    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.extend_from_slice(&1u32.to_le_bytes());

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, ValueComponent(0)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 0);
    assert!(client_app.world().resource::<ReceivedUserdata>().is_none());

    // The observer accepts this mutation's userdata, but its update is still deferred,
    // so it must stay buffered until the update is applied.
    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.clear();
    userdata.extend_from_slice(&0u32.to_le_bytes());

    let mut component = server_app
        .world_mut()
        .get_mut::<ValueComponent>(server_entity)
        .unwrap();
    **component = 1;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(remote.iter(client_app.world()).len(), 0);
    assert_eq!(**client_app.world().resource::<ReceivedUserdata>(), None);

    let messages = client_app.world().resource::<ClientMessages>();
    assert!(
        messages
            .iter_sent()
            .any(|(c, _)| c == ClientChannel::MutationAcks as usize),
        "a retained mutation should be acknowledged even while waiting for its update"
    );

    **client_app.world_mut().resource_mut::<ReadyUserdata>() = 1;
    client_app.update();

    assert_eq!(
        **client_app.world().resource::<ReceivedUserdata>(),
        Some(0),
        "messages should be applied in wire order once the update is ready"
    );
    assert_eq!(remote.iter(client_app.world()).len(), 1);

    let mut components = client_app.world_mut().query::<&ValueComponent>();
    assert_eq!(**components.single(client_app.world()).unwrap(), 1);
}

const USERDATA: u32 = 42;

#[derive(Component, Deserialize, Serialize)]
struct TestComponent;

#[derive(Component, Deref, DerefMut, Deserialize, Serialize)]
struct ValueComponent(u32);

#[derive(Resource, Deref, DerefMut, Default)]
struct ReceivedUserdata(Option<u32>);

#[derive(Resource, Deref, DerefMut, Default)]
struct ReadyUserdata(u32);

#[test]
fn deferred_mutation_once_per_receive() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<ValueComponent>()
        .finish();
    }
    client_app
        .init_resource::<DeferCount>()
        .add_observer(count_deferred);

    server_app.connect_client(&mut client_app);

    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.extend_from_slice(&0u32.to_le_bytes());

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, ValueComponent(0)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // The observer defers this mutation.
    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.clear();
    userdata.extend_from_slice(&1u32.to_le_bytes());

    let mut component = server_app
        .world_mut()
        .get_mut::<ValueComponent>(server_entity)
        .unwrap();
    **component = 1;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // An update arrives while the mutation is still deferred.
    let mut userdata = server_app.world_mut().resource_mut::<ReplicationUserdata>();
    userdata.clear();
    userdata.extend_from_slice(&0u32.to_le_bytes());
    server_app.world_mut().spawn(Replicated);
    **client_app.world_mut().resource_mut::<DeferCount>() = 0;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(
        **client_app.world().resource::<DeferCount>(),
        1,
        "a deferred mutation should be asked about once per receive"
    );
}

fn receive_userdata(received: On<UserdataReceived>, mut storage: ResMut<ReceivedUserdata>) {
    let bytes = received.bytes.as_ref().try_into().unwrap();
    storage.0 = Some(u32::from_le_bytes(bytes));
}

fn apply_when_ready(mut trigger: On<ShouldApplyReplication>, ready: Res<ReadyUserdata>) {
    let Some(userdata) = &trigger.userdata else {
        return;
    };

    let value = u32::from_le_bytes(userdata.as_ref().try_into().unwrap());
    if value > ready.0 {
        trigger.should_apply = false;
    }
}

#[derive(Resource, Deref, DerefMut, Default)]
struct DeferCount(usize);

/// Defers messages with userdata `1` and counts how often it's asked about them.
fn count_deferred(mut trigger: On<ShouldApplyReplication>, mut count: ResMut<DeferCount>) {
    let Some(userdata) = &trigger.userdata else {
        return;
    };

    if u32::from_le_bytes(userdata.as_ref().try_into().unwrap()) == 1 {
        trigger.should_apply = false;
        **count += 1;
    }
}
