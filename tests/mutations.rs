use core::time::Duration;
use test_log::test;

use bevy::prelude::*;
use bevy_replicon::{
    client::{
        ServerUpdateTick,
        confirm_history::{ConfirmHistory, EntityReplicated},
    },
    prelude::*,
    server::server_tick::ServerTick,
    shared::{
        replication::{
            command_markers::MarkerConfig,
            deferred_entity::DeferredEntity,
            registry::{command_fns, ctx::WriteCtx},
        },
        server_entity_map::ServerEntityMap,
    },
    test_app::{ServerTestAppExt, TestClientEntity},
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[test]
fn small_component() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
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

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world_mut()
        .query::<&BoolComponent>()
        .single(client_app.world())
        .unwrap();
    assert!(component.0, "mutated value should be updated on client");
}

#[test]
fn package_size_component() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<VecComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, VecComponent::default()))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // To exceed packed size.
    const BIG_DATA: &[u8] = &[0; 1200];
    let mut component = server_app
        .world_mut()
        .get_mut::<VecComponent>(server_entity)
        .unwrap();
    component.0 = BIG_DATA.to_vec();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world_mut()
        .query::<&VecComponent>()
        .single(client_app.world())
        .unwrap();
    assert_eq!(component.0, BIG_DATA);
}

#[test]
fn many_components() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<BoolComponent>()
        .replicate::<VecComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false), VecComponent::default()))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut server_entity = server_app.world_mut().entity_mut(server_entity);

    let mut bool_component = server_entity.get_mut::<BoolComponent>().unwrap();
    bool_component.0 = true;

    const VEC_VALUE: &[u8] = &[1; 10];
    let mut vec_component = server_entity.get_mut::<VecComponent>().unwrap();
    vec_component.0 = VEC_VALUE.to_vec();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let (bool_component, vec_component) = client_app
        .world_mut()
        .query::<(&BoolComponent, &VecComponent)>()
        .single(client_app.world())
        .unwrap();
    assert!(bool_component.0);
    assert_eq!(vec_component.0, VEC_VALUE);
}

#[test]
fn once() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate_once::<BoolComponent>()
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

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world_mut()
        .query::<&BoolComponent>()
        .single(client_app.world())
        .unwrap();
    assert!(!component.0, "only initial value should be replicated");
}

#[test]
fn filtered() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate_filtered::<BoolComponent, Without<TestComponent>>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let with_match = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false)))
        .id();
    let without_match = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false), TestComponent))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change values.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(with_match)
        .unwrap();
    component.0 = true;
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(without_match)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world_mut()
        .query::<&BoolComponent>()
        .single(client_app.world())
        .unwrap();
    assert!(
        component.0,
        "only entity that matches the filter should be replicated"
    );
}

#[test]
fn related() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .sync_related_entities::<ChildOf>()
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn((
        Replicated,
        BoolComponent(false),
        children![(Replicated, BoolComponent(false))],
    ));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change values.
    let mut components = server_app.world_mut().query::<&mut BoolComponent>();
    for mut component in components.iter_mut(server_app.world_mut()) {
        component.0 = true;
    }

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&BoolComponent>();
    let mut components = components.iter(client_app.world());
    assert_eq!(components.len(), 2);
    assert!(components.all(|c| c.0));
}

#[test]
fn command_fns() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<OriginalComponent>()
        .set_command_fns(replace, command_fns::default_remove::<ReplacedComponent>)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, OriginalComponent(false)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app
        .world_mut()
        .query_filtered::<&ReplacedComponent, Without<OriginalComponent>>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<OriginalComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = components.single(client_app.world()).unwrap();
    assert!(component.0);
}

#[test]
fn marker() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .register_marker::<ReplaceMarker>()
        .replicate::<OriginalComponent>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        )
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, OriginalComponent(false)))
        .id();

    let client_entity = client_app.world_mut().spawn(ReplaceMarker).id();

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    let mut entity_map = server_app
        .world_mut()
        .get_mut::<ClientEntityMap>(test_client_entity)
        .unwrap();
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<OriginalComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    assert!(!client_entity.contains::<OriginalComponent>());

    let component = client_entity.get::<ReplacedComponent>().unwrap();
    assert!(component.0);
}

#[test]
fn marker_with_history() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .register_marker_with::<HistoryMarker>(MarkerConfig {
            need_history: true,
            ..Default::default()
        })
        .set_marker_fns::<HistoryMarker, BoolComponent>(
            write_history,
            command_fns::default_remove::<BoolComponent>,
        )
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false)))
        .id();

    let client_entity = client_app.world_mut().spawn(HistoryMarker).id();

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    let mut entity_map = server_app
        .world_mut()
        .get_mut::<ClientEntityMap>(test_client_entity)
        .unwrap();
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value, but don't process it on client.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value again to trigger another message.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = false;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    let history = client_entity.get::<BoolHistory>().unwrap();
    assert_eq!(
        history.0,
        [false, false, true],
        "the initial value should come first, then the latest mutation, \
        and after that the older mutation because recent mutations processed first"
    );
}

#[test]
fn marker_with_history_consume() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .register_marker_with::<HistoryMarker>(MarkerConfig {
            need_history: true,
            ..Default::default()
        })
        .set_marker_fns::<HistoryMarker, BoolComponent>(
            write_history,
            command_fns::default_remove::<BoolComponent>,
        )
        .replicate::<BoolComponent>()
        .replicate::<MappedComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let entities_before = client_app.world().entities().len();

    let server_map_entity = server_app.world_mut().spawn_empty().id();
    let server_entity = server_app
        .world_mut()
        .spawn((
            Replicated,
            BoolComponent(false),
            MappedComponent(server_map_entity),
        ))
        .id();

    let client_entity = client_app.world_mut().spawn(HistoryMarker).id();

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    let mut entity_map = server_app
        .world_mut()
        .get_mut::<ClientEntityMap>(test_client_entity)
        .unwrap();
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value, but don't process it on client.
    let new_entity1 = server_app.world_mut().spawn_empty().id();
    let mut component = server_app
        .world_mut()
        .get_mut::<MappedComponent>(server_entity)
        .unwrap();
    component.0 = new_entity1;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change value again to trigger another message.
    let new_entity2 = server_app.world_mut().spawn_empty().id();
    let mut component = server_app
        .world_mut()
        .get_mut::<MappedComponent>(server_entity)
        .unwrap();
    component.0 = new_entity2;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    assert!(entity_map.to_client().contains_key(&new_entity2));
    assert!(
        !entity_map.to_client().contains_key(&new_entity1),
        "client should consume older mutations for other components with marker that requested history"
    );

    assert_eq!(
        client_app.world().entities().len() - entities_before,
        3,
        "client should have 2 initial entities and 1 from mutate message"
    );
}

#[test]
fn marker_with_history_old_update() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .register_marker_with::<HistoryMarker>(MarkerConfig {
            need_history: true,
            ..Default::default()
        })
        .set_marker_fns::<HistoryMarker, BoolComponent>(
            write_history,
            command_fns::default_remove::<BoolComponent>,
        )
        .replicate::<BoolComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false)))
        .id();

    let client_entity = client_app.world_mut().spawn(HistoryMarker).id();

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    let mut entity_map = server_app
        .world_mut()
        .get_mut::<ClientEntityMap>(test_client_entity)
        .unwrap();
    entity_map.insert(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Artificially make the last confirmed tick too large
    // so that the next mutation for this entity is discarded.
    let mut tick = **server_app.world().resource::<ServerTick>();
    tick += u64::BITS + 1;
    let mut history = client_app
        .world_mut()
        .get_mut::<ConfirmHistory>(client_entity)
        .unwrap();
    history.confirm(tick);

    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let history = client_app
        .world()
        .get::<BoolHistory>(client_entity)
        .unwrap();

    assert_eq!(
        history.0,
        [false],
        "mutation should be considered too old and discarded"
    );
}

#[test]
fn many_entities() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
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
    assert_eq!(replicated.iter(client_app.world()).count(), ENTITIES_COUNT);

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

    for component in client_app
        .world_mut()
        .query::<&BoolComponent>()
        .iter(client_app.world())
    {
        assert!(component.0);
    }
}

#[test]
fn with_insertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<BoolComponent>()
        .replicate::<TestComponent>()
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

    let mut server_entity = server_app.world_mut().entity_mut(server_entity);
    server_entity.get_mut::<BoolComponent>().unwrap().0 = true;
    server_entity.insert(TestComponent);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world_mut()
        .query_filtered::<&BoolComponent, With<TestComponent>>()
        .single(client_app.world())
        .unwrap();
    assert!(component.0);
}

#[test]
fn with_removal() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<BoolComponent>()
        .replicate::<TestComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, BoolComponent(false), TestComponent))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut server_entity = server_app.world_mut().entity_mut(server_entity);
    server_entity.get_mut::<BoolComponent>().unwrap().0 = true;
    server_entity.remove::<TestComponent>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world_mut()
        .query_filtered::<&BoolComponent, Without<TestComponent>>()
        .single(client_app.world())
        .unwrap();
    assert!(component.0);
}

#[test]
fn with_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
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

    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    // Update without client to send mutate message.
    server_app.update();

    server_app.world_mut().despawn(server_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update(); // Let server receive an update to trigger acknowledgment.

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).len(), 0);
}

#[test]
fn buffering() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
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

    // Artificially reset the update tick to force the next received mutation to be buffered.
    let mut update_tick = client_app.world_mut().resource_mut::<ServerUpdateTick>();
    let previous_tick = *update_tick;
    *update_tick = Default::default();
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&BoolComponent>();
    let component = components.single(client_app.world()).unwrap();
    assert!(!component.0, "client should buffer the mutation");

    // Restore the update tick to let the buffered mutation apply
    *client_app.world_mut().resource_mut::<ServerUpdateTick>() = previous_tick;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = components.single(client_app.world()).unwrap();
    assert!(component.0, "buffered mutation should be applied");
}

#[test]
fn old_ignored() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
        .replicate::<MappedComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let entities_before = client_app.world().entities().len();

    let server_map_entity = server_app.world_mut().spawn_empty().id();
    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, MappedComponent(server_map_entity)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change the value, but don't process it on client.
    let new_entity1 = server_app.world_mut().spawn_empty().id();
    let mut component = server_app
        .world_mut()
        .get_mut::<MappedComponent>(server_entity)
        .unwrap();
    component.0 = new_entity1;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    // Change the value again to trigger another message.
    let new_entity2 = server_app.world_mut().spawn_empty().id();
    let mut component = server_app
        .world_mut()
        .get_mut::<MappedComponent>(server_entity)
        .unwrap();
    component.0 = new_entity2;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    assert!(
        !entity_map.to_client().contains_key(&new_entity1),
        "client should ignore older mutation"
    );

    assert_eq!(
        client_app.world().entities().len() - entities_before,
        3,
        "client should have 2 initial entities and 1 from mutation"
    );
}

#[test]
fn acknowledgment() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                mutations_timeout: Duration::ZERO, // Will cause dropping updates after each frame.
                ..Default::default()
            }),
        ))
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

    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<Ref<BoolComponent>>();
    let component = components.single(client_app.world()).unwrap();
    let tick1 = component.last_changed();

    // Take and drop ack message.
    let mut client = client_app.world_mut().resource_mut::<RepliconClient>();
    assert_eq!(client.drain_sent().count(), 1);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let component = components.single(client_app.world()).unwrap();
    let tick2 = component.last_changed();

    assert!(
        tick1.get() < tick2.get(),
        "client should receive the same mutation twice because server missed the ack"
    );

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world_mut()
        .query::<Ref<BoolComponent>>()
        .single(client_app.world())
        .unwrap();
    let tick3 = component.last_changed();

    assert_eq!(
        tick2.get(),
        tick3.get(),
        "client shouldn't receive acked mutation"
    );
}

#[test]
fn confirm_history() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
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

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    // Clear previous events.
    client_app
        .world_mut()
        .resource_mut::<Events<EntityReplicated>>()
        .clear();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();

    let (client_entity, confirm_history) = client_app
        .world_mut()
        .query::<(Entity, &ConfirmHistory)>()
        .single(client_app.world())
        .unwrap();
    assert!(confirm_history.contains(tick));

    let mut replicated_events = client_app
        .world_mut()
        .resource_mut::<Events<EntityReplicated>>();
    let [event] = replicated_events
        .drain()
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert_eq!(event.entity, client_entity);
    assert_eq!(event.tick, tick);
}

#[test]
fn after_disconnect() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            RepliconPlugins.set(ServerPlugin {
                tick_policy: TickPolicy::EveryFrame,
                ..Default::default()
            }),
        ))
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

    // Change value.
    let mut component = server_app
        .world_mut()
        .get_mut::<BoolComponent>(server_entity)
        .unwrap();
    component.0 = true;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let test_client_entity = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(test_client_entity)
        .despawn();
    server_app.update();
}

#[derive(Component, Deserialize, Serialize)]
struct TestComponent;

#[derive(Clone, Component, Copy, Deserialize, Serialize)]
struct BoolComponent(bool);

#[derive(Component, Default, Deserialize, Serialize)]
struct VecComponent(Vec<u8>);

#[derive(Component, Deserialize, Serialize)]
struct MappedComponent(#[entities] Entity);

#[derive(Component)]
struct ReplaceMarker;

#[derive(Component, Deserialize, Serialize)]
struct OriginalComponent(bool);

#[derive(Component, Deserialize, Serialize)]
struct ReplacedComponent(bool);

#[derive(Component)]
struct HistoryMarker;

#[derive(Component, Deref, DerefMut)]
struct BoolHistory(Vec<bool>);

/// Deserializes [`OriginalComponent`], but inserts it as [`ReplacedComponent`].
fn replace(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<OriginalComponent>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    let component = rule_fns.deserialize(ctx, message)?;

    entity.insert(ReplacedComponent(component.0));

    Ok(())
}

/// Instead of writing into [`BoolComponent`], it writes data into [`BoolHistory`].
fn write_history(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<BoolComponent>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    let component = rule_fns.deserialize(ctx, message)?;
    if let Some(mut history) = entity.get_mut::<BoolHistory>() {
        history.push(component.0);
    } else {
        entity.insert(BoolHistory(vec![component.0]));
    }

    Ok(())
}
