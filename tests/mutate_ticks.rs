use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    client::server_mutate_ticks::{MutateTickReceived, ServerMutateTicks},
    prelude::*,
    server::server_tick::ServerTick,
    shared::replication::track_mutate_messages::TrackAppExt,
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn without_changes() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .track_mutate_messages()
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let events_count = client_app
        .world_mut()
        .resource_mut::<Events<MutateTickReceived>>()
        .drain()
        .count();
    assert_eq!(
        events_count, 2,
        "should receive one event for connection and one for the exchange"
    );
}

#[test]
fn one_message() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .track_mutate_messages()
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut tick_events = client_app
        .world_mut()
        .resource_mut::<Events<MutateTickReceived>>();
    assert_eq!(
        tick_events.drain().count(),
        2,
        "should receive one event for connection and one for spawn"
    );

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    // Clear previous events.
    client_app
        .world_mut()
        .resource_mut::<Events<MutateTickReceived>>()
        .clear();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();

    let mut tick_events = client_app
        .world_mut()
        .resource_mut::<Events<MutateTickReceived>>();
    let [event] = tick_events.drain().collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(event.tick, tick);

    let mutate_ticks = client_app.world().resource::<ServerMutateTicks>();
    assert!(mutate_ticks.contains(tick));
}

#[test]
fn multiple_messages() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .track_mutate_messages()
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn many entities to cover message splitting.
    const ENTITIES_COUNT: usize = 300;
    server_app
        .world_mut()
        .spawn_batch([(Replicated, BoolComponent(false)); ENTITIES_COUNT]);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).len(), ENTITIES_COUNT);

    let mut tick_events = client_app
        .world_mut()
        .resource_mut::<Events<MutateTickReceived>>();
    assert_eq!(
        tick_events.drain().count(),
        2,
        "should receive one event for connection and one for spawns"
    );

    for mut component in server_app
        .world_mut()
        .query::<&mut BoolComponent>()
        .iter_mut(server_app.world_mut())
    {
        component.0 = true;
    }

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();

    let mut tick_events = client_app
        .world_mut()
        .resource_mut::<Events<MutateTickReceived>>();
    let [event] = tick_events.drain().collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(event.tick, tick);

    let mutate_ticks = client_app.world().resource::<ServerMutateTicks>();
    assert!(mutate_ticks.contains(tick));
}

#[derive(Clone, Component, Copy, Deserialize, Serialize)]
struct BoolComponent(bool);
