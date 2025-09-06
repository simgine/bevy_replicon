use bevy::{
    ecs::{entity::MapEntities, event::Events},
    prelude::*,
    state::app::StatesPlugin,
    time::TimePlugin,
};
use bevy_replicon::{
    client::ServerUpdateTick,
    prelude::*,
    server::server_tick::ServerTick,
    shared::{
        event::remote_event_registry::RemoteEventRegistry, server_entity_map::ServerEntityMap,
    },
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn channels() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin {
            tick_schedule: TickSchedule::PostUpdate,
            ..Default::default()
        }),
    ))
    .add_event::<NonRemoteEvent>()
    .add_server_event::<TestEvent>(Channel::Ordered)
    .finish();

    let event_registry = app.world().resource::<RemoteEventRegistry>();
    assert_eq!(event_registry.server_channel::<NonRemoteEvent>(), None);
    assert_eq!(event_registry.server_channel::<TestEvent>(), Some(3));
}

#[test]
fn regular() {
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
        .add_server_event::<TestEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    for (mode, events_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(ClientId::Server), 0),
        (SendMode::Direct(client.into()), 1),
        (SendMode::BroadcastExcept(ClientId::Server), 1),
        (SendMode::BroadcastExcept(client.into()), 0),
    ] {
        server_app.world_mut().send_event(ToClients {
            mode,
            event: TestEvent,
        });

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let mut events = client_app.world_mut().resource_mut::<Events<TestEvent>>();
        assert_eq!(
            events.drain().count(),
            events_count,
            "event should be emitted {events_count} times for {mode:?}"
        );
    }
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
        .add_mapped_server_event::<EntityEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.world_mut().send_event(ToClients {
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

    let mapped_entities: Vec<_> = client_app
        .world_mut()
        .resource_mut::<Events<EntityEvent>>()
        .drain()
        .map(|event| event.0)
        .collect();
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
                .set(ServerPlugin {
                    tick_schedule: TickSchedule::PostUpdate,
                    ..Default::default()
                })
                .disable::<ClientPlugin>()
                .disable::<ClientEventPlugin>(),
        ))
        .add_server_event::<TestEvent>(Channel::Ordered)
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
        .add_server_event::<TestEvent>(Channel::Ordered)
        .finish();

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    for (mode, events_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(ClientId::Server), 0),
        (SendMode::Direct(client.into()), 1),
        (SendMode::BroadcastExcept(ClientId::Server), 1),
        (SendMode::BroadcastExcept(client.into()), 0),
    ] {
        server_app.world_mut().send_event(ToClients {
            mode,
            event: TestEvent,
        });

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let mut events = client_app.world_mut().resource_mut::<Events<TestEvent>>();
        assert_eq!(
            events.drain().count(),
            events_count,
            "event should be emitted {events_count} times for {mode:?}"
        );
    }
}

#[test]
fn local_resending() {
    let mut app = App::new();
    app.add_plugins((
        TimePlugin,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin {
            tick_schedule: TickSchedule::PostUpdate,
            ..Default::default()
        }),
    ))
    .add_server_event::<TestEvent>(Channel::Ordered)
    .finish();

    const PLACEHOLDER_CLIENT_ID: ClientId = ClientId::Client(Entity::from_raw(1));
    for (mode, events_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(ClientId::Server), 1),
        (SendMode::Direct(PLACEHOLDER_CLIENT_ID), 0),
        (SendMode::BroadcastExcept(ClientId::Server), 0),
        (SendMode::BroadcastExcept(PLACEHOLDER_CLIENT_ID), 1),
    ] {
        app.world_mut().send_event(ToClients {
            mode,
            event: TestEvent,
        });

        app.update();

        let server_events = app.world().resource::<Events<ToClients<TestEvent>>>();
        assert!(server_events.is_empty());

        let mut events = app.world_mut().resource_mut::<Events<TestEvent>>();
        assert_eq!(
            events.drain().count(),
            events_count,
            "event should be emitted {events_count} times for {mode:?}"
        );
    }
}

#[test]
fn server_buffering() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::Custom, // To artificially delay replication after sending.
                ..Default::default()
            }),
        ))
        .add_server_event::<TestEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let events = client_app.world().resource::<Events<TestEvent>>();
    assert!(events.is_empty(), "event should be buffered on server");

    // Trigger replication.
    server_app
        .world_mut()
        .resource_mut::<ServerTick>()
        .increment();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let events = client_app.world().resource::<Events<TestEvent>>();
    assert_eq!(events.len(), 1);
}

#[test]
fn client_queue() {
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
        .add_server_event::<TestEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially reset the update tick to force the next received event to be queued.
    let mut update_tick = client_app.world_mut().resource_mut::<ServerUpdateTick>();
    let previous_tick = *update_tick;
    *update_tick = Default::default();
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let events = client_app.world().resource::<Events<TestEvent>>();
    assert!(events.is_empty());

    // Restore the update tick to receive the event.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = previous_tick;

    client_app.update();

    assert_eq!(client_app.world().resource::<Events<TestEvent>>().len(), 1);
}

#[test]
fn client_queue_and_mapping() {
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
        .add_mapped_server_event::<EntityEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn an entity to trigger world change.
    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially reset the update tick to force the next received event to be queued.
    let mut update_tick = client_app.world_mut().resource_mut::<ServerUpdateTick>();
    let previous_tick = *update_tick;
    *update_tick = Default::default();
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: EntityEvent(server_entity),
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let events = client_app.world().resource::<Events<EntityEvent>>();
    assert!(events.is_empty());

    // Restore the update tick to receive the event.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = previous_tick;

    client_app.update();

    let client_entity = *client_app
        .world()
        .resource::<ServerEntityMap>()
        .to_client()
        .get(&server_entity)
        .unwrap();

    let mapped_entities: Vec<_> = client_app
        .world_mut()
        .resource_mut::<Events<EntityEvent>>()
        .drain()
        .map(|event| event.0)
        .collect();
    assert_eq!(mapped_entities, [client_entity]);
}

#[test]
fn multiple_client_queues() {
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
        .add_server_event::<TestEvent>(Channel::Ordered)
        .add_server_event::<EntityEvent>(Channel::Ordered) // Use as a regular event with a different serialization size.
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially reset the update tick to force the next received event to be queued.
    let mut update_tick = client_app.world_mut().resource_mut::<ServerUpdateTick>();
    let previous_tick = *update_tick;
    *update_tick = Default::default();
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: EntityEvent(Entity::PLACEHOLDER),
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let events = client_app.world().resource::<Events<TestEvent>>();
    assert!(events.is_empty());

    let mapped_events = client_app.world().resource::<Events<EntityEvent>>();
    assert!(mapped_events.is_empty());

    // Restore the update tick to receive the event.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = previous_tick;

    client_app.update();

    assert_eq!(client_app.world().resource::<Events<TestEvent>>().len(), 1);
    assert_eq!(
        client_app.world().resource::<Events<EntityEvent>>().len(),
        1
    );
}

#[test]
fn independent() {
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
        .add_server_event::<TestEvent>(Channel::Ordered)
        .add_server_event::<IndependentEvent>(Channel::Ordered)
        .make_event_independent::<IndependentEvent>()
        .finish();
    }

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

    let client = **client_app.world().resource::<TestClientEntity>();
    for (mode, events_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(ClientId::Server), 0),
        (SendMode::Direct(client.into()), 1),
        (SendMode::BroadcastExcept(ClientId::Server), 1),
        (SendMode::BroadcastExcept(client.into()), 0),
    ] {
        server_app.world_mut().send_event(ToClients {
            mode,
            event: TestEvent,
        });
        server_app.world_mut().send_event(ToClients {
            mode,
            event: IndependentEvent,
        });

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let events = client_app.world().resource::<Events<TestEvent>>();
        assert!(events.is_empty());

        // Event should have already been triggered, even without resetting the tick,
        // since it's independent.
        let mut independent_events = client_app
            .world_mut()
            .resource_mut::<Events<IndependentEvent>>();
        assert_eq!(
            independent_events.drain().count(),
            events_count,
            "event should be emitted {events_count} times for {mode:?}"
        );
    }
}

#[test]
fn before_started_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .set(ServerPlugin {
                    tick_schedule: TickSchedule::PostUpdate,
                    ..Default::default()
                })
                .set(RepliconSharedPlugin {
                    auth_method: AuthMethod::Custom,
                }),
        ))
        .add_server_event::<TestEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    for mode in [
        SendMode::Broadcast,
        SendMode::BroadcastExcept(ClientId::Server),
        SendMode::Direct(client.into()),
    ] {
        server_app.world_mut().send_event(ToClients {
            mode,
            event: TestEvent,
        });
    }

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let events = client_app.world().resource::<Events<TestEvent>>();
    assert!(events.is_empty());
}

#[test]
fn independent_before_started_replication() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .set(ServerPlugin {
                    tick_schedule: TickSchedule::PostUpdate,
                    ..Default::default()
                })
                .set(RepliconSharedPlugin {
                    auth_method: AuthMethod::Custom,
                }),
        ))
        .add_server_event::<TestEvent>(Channel::Ordered)
        .add_server_event::<IndependentEvent>(Channel::Ordered)
        .make_event_independent::<IndependentEvent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });
    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: IndependentEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let events = client_app.world().resource::<Events<TestEvent>>();
    assert!(events.is_empty());

    let independent_events = client_app.world().resource::<Events<IndependentEvent>>();
    assert_eq!(independent_events.len(), 1);
}

#[test]
fn different_ticks() {
    let mut server_app = App::new();
    let mut client_app1 = App::new();
    let mut client_app2 = App::new();
    for app in [&mut server_app, &mut client_app1, &mut client_app2] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: TickSchedule::PostUpdate,
                ..Default::default()
            }),
        ))
        .add_server_event::<TestEvent>(Channel::Ordered)
        .finish();
    }

    // Connect client 1 first.
    server_app.connect_client(&mut client_app1);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    // Update client 1 to initialize their replicon tick.
    server_app.update();
    server_app.exchange_with_client(&mut client_app1);
    client_app1.update();
    server_app.exchange_with_client(&mut client_app1);

    // Connect client 2 later to make it have a higher replicon tick than client 1,
    // since only client 1 will receive a update message here.
    server_app.connect_client(&mut client_app2);

    server_app.world_mut().send_event(ToClients {
        mode: SendMode::Broadcast,
        event: TestEvent,
    });

    // If any client does not have a replicon tick >= the update tick associated with this event,
    // then they will not receive the event until their replicon tick is updated.
    server_app.update();
    server_app.exchange_with_client(&mut client_app1);
    server_app.exchange_with_client(&mut client_app2);
    client_app1.update();
    client_app2.update();

    assert_eq!(client_app1.world().resource::<Events<TestEvent>>().len(), 1);
    assert_eq!(client_app2.world().resource::<Events<TestEvent>>().len(), 1);
}

#[derive(Event)]
struct NonRemoteEvent;

#[derive(Event, Serialize, Deserialize)]
struct TestEvent;

#[derive(Event, Serialize, Deserialize)]
struct IndependentEvent;

#[derive(Event, Serialize, Deserialize, MapEntities)]
struct EntityEvent(#[entities] Entity);
