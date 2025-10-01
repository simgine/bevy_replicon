use bevy::{ecs::entity::MapEntities, prelude::*, state::app::StatesPlugin, time::TimePlugin};
use bevy_replicon::{
    client::ServerUpdateTick,
    prelude::*,
    server::server_tick::ServerTick,
    shared::{message::registry::RemoteMessageRegistry, server_entity_map::ServerEntityMap},
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
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .add_message::<NonRemoteEvent>()
    .add_server_message::<TestEvent>(Channel::Ordered)
    .finish();

    let registry = app.world().resource::<RemoteMessageRegistry>();
    assert_eq!(registry.server_channel::<NonRemoteEvent>(), None);
    assert_eq!(registry.server_channel::<TestEvent>(), Some(3));
}

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
        .add_server_message::<TestEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    for (mode, messages_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(ClientId::Server), 0),
        (SendMode::Direct(client.into()), 1),
        (SendMode::BroadcastExcept(ClientId::Server), 1),
        (SendMode::BroadcastExcept(client.into()), 0),
    ] {
        server_app.world_mut().write_message(ToClients {
            mode,
            message: TestEvent,
        });

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let mut messages = client_app.world_mut().resource_mut::<Messages<TestEvent>>();
        assert_eq!(
            messages.drain().count(),
            messages_count,
            "message should be received {messages_count} times for {mode:?}"
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
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_mapped_server_message::<EntityEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: EntityEvent(server_entity),
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
        .resource_mut::<Messages<EntityEvent>>()
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
                .set(ServerPlugin::new(PostUpdate))
                .disable::<ClientPlugin>()
                .disable::<ClientMessagePlugin>(),
        ))
        .add_server_message::<TestEvent>(Channel::Ordered)
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
        .add_server_message::<TestEvent>(Channel::Ordered)
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
        server_app.world_mut().write_message(ToClients {
            mode,
            message: TestEvent,
        });

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let mut messages = client_app.world_mut().resource_mut::<Messages<TestEvent>>();
        assert_eq!(
            messages.drain().count(),
            events_count,
            "message should be received {events_count} times for {mode:?}"
        );
    }
}

#[test]
fn local_resending() {
    let mut app = App::new();
    app.add_plugins((
        TimePlugin,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .add_server_message::<TestEvent>(Channel::Ordered)
    .finish();

    const CLIENT_ENTITY: Entity = Entity::from_raw_u32(1).unwrap();
    const PLACEHOLDER_CLIENT_ID: ClientId = ClientId::Client(CLIENT_ENTITY);
    for (mode, messages_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(ClientId::Server), 1),
        (SendMode::Direct(PLACEHOLDER_CLIENT_ID), 0),
        (SendMode::BroadcastExcept(ClientId::Server), 0),
        (SendMode::BroadcastExcept(PLACEHOLDER_CLIENT_ID), 1),
    ] {
        app.world_mut().write_message(ToClients {
            mode,
            message: TestEvent,
        });

        app.update();

        let server_messages = app.world().resource::<Messages<ToClients<TestEvent>>>();
        assert!(server_messages.is_empty());

        let mut messages = app.world_mut().resource_mut::<Messages<TestEvent>>();
        assert_eq!(
            messages.drain().count(),
            messages_count,
            "message should be received {messages_count} times for {mode:?}"
        );
    }
}

#[test]
fn server_buffering() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_server_message::<TestEvent>(Channel::Ordered)
            .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: TestEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert!(messages.is_empty(), "message should be buffered on server");

    // Trigger replication.
    server_app
        .world_mut()
        .resource_mut::<ServerTick>()
        .increment();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert_eq!(messages.len(), 1);
}

#[test]
fn client_queue() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_server_message::<TestEvent>(Channel::Ordered)
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
    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: TestEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert!(messages.is_empty());

    // Restore the update tick to receive the event.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = previous_tick;

    client_app.update();

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert_eq!(messages.len(), 1);
}

#[test]
fn client_queue_and_mapping() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_mapped_server_message::<EntityEvent>(Channel::Ordered)
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
    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: EntityEvent(server_entity),
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let events = client_app.world().resource::<Messages<EntityEvent>>();
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
        .resource_mut::<Messages<EntityEvent>>()
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
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_server_message::<TestEvent>(Channel::Ordered)
        .add_server_message::<EntityEvent>(Channel::Ordered) // Use as a regular event with a different serialization size.
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
    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: TestEvent,
    });
    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: EntityEvent(Entity::PLACEHOLDER),
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert!(messages.is_empty());

    let mapped_messages = client_app.world().resource::<Messages<EntityEvent>>();
    assert!(mapped_messages.is_empty());

    // Restore the update tick to receive the event.
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = previous_tick;

    client_app.update();

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert_eq!(messages.len(), 1);

    let mapped_messages = client_app.world().resource::<Messages<EntityEvent>>();
    assert_eq!(mapped_messages.len(), 1);
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
        .add_server_message::<TestEvent>(Channel::Ordered)
        .add_server_message::<IndependentEvent>(Channel::Ordered)
        .make_message_independent::<IndependentEvent>()
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
    for (mode, messages_count) in [
        (SendMode::Broadcast, 1),
        (SendMode::Direct(ClientId::Server), 0),
        (SendMode::Direct(client.into()), 1),
        (SendMode::BroadcastExcept(ClientId::Server), 1),
        (SendMode::BroadcastExcept(client.into()), 0),
    ] {
        server_app.world_mut().write_message(ToClients {
            mode,
            message: TestEvent,
        });
        server_app.world_mut().write_message(ToClients {
            mode,
            message: IndependentEvent,
        });

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let messages = client_app.world().resource::<Messages<TestEvent>>();
        assert!(messages.is_empty());

        // Message should have already been triggered, even without resetting the tick,
        // since it's independent.
        let mut independent_messages = client_app
            .world_mut()
            .resource_mut::<Messages<IndependentEvent>>();
        assert_eq!(
            independent_messages.drain().count(),
            messages_count,
            "message should be received {messages_count} times for {mode:?}"
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
                .set(ServerPlugin::new(PostUpdate))
                .set(RepliconSharedPlugin {
                    auth_method: AuthMethod::Custom,
                }),
        ))
        .add_server_message::<TestEvent>(Channel::Ordered)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    for mode in [
        SendMode::Broadcast,
        SendMode::BroadcastExcept(ClientId::Server),
        SendMode::Direct(client.into()),
    ] {
        server_app.world_mut().write_message(ToClients {
            mode,
            message: TestEvent,
        });
    }

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert!(messages.is_empty());
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
                .set(ServerPlugin::new(PostUpdate))
                .set(RepliconSharedPlugin {
                    auth_method: AuthMethod::Custom,
                }),
        ))
        .add_server_message::<TestEvent>(Channel::Ordered)
        .add_server_message::<IndependentEvent>(Channel::Ordered)
        .make_message_independent::<IndependentEvent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Spawn entity to trigger world change.
    server_app.world_mut().spawn(Replicated);

    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: TestEvent,
    });
    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: IndependentEvent,
    });

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let messages = client_app.world().resource::<Messages<TestEvent>>();
    assert!(messages.is_empty());

    let independent_messages = client_app.world().resource::<Messages<IndependentEvent>>();
    assert_eq!(independent_messages.len(), 1);
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
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_server_message::<TestEvent>(Channel::Ordered)
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

    server_app.world_mut().write_message(ToClients {
        mode: SendMode::Broadcast,
        message: TestEvent,
    });

    // If any client does not have a replicon tick >= the update tick associated with this event,
    // then they will not receive the event until their replicon tick is updated.
    server_app.update();
    server_app.exchange_with_client(&mut client_app1);
    server_app.exchange_with_client(&mut client_app2);
    client_app1.update();
    client_app2.update();

    let messages1 = client_app1.world().resource::<Messages<TestEvent>>();
    assert_eq!(messages1.len(), 1);

    let messages2 = client_app2.world().resource::<Messages<TestEvent>>();
    assert_eq!(messages2.len(), 1);
}

#[derive(Message)]
struct NonRemoteEvent;

#[derive(Message, Serialize, Deserialize)]
struct TestEvent;

#[derive(Message, Serialize, Deserialize)]
struct IndependentEvent;

#[derive(Message, Serialize, Deserialize, MapEntities)]
struct EntityEvent(#[entities] Entity);
