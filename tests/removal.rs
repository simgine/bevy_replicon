use bevy::{ecs::schedule::ScheduleLabel, prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    client::confirm_history::{ConfirmHistory, EntityReplicated},
    prelude::*,
    server::server_tick::ServerTick,
    shared::replication::{
        deferred_entity::DeferredEntity,
        registry::{command_fns, ctx::WriteCtx},
    },
    test_app::ServerTestAppExt,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn single() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
}

#[test]
fn multiple() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .replicate::<B>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A, B)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<(&A, &B)>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    let before_archetypes = client_app.world().archetypes().len();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<(A, B)>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
    assert_eq!(
        client_app.world().archetypes().len() - before_archetypes,
        1,
        "should cause only a single archetype move"
    );
}

#[test]
fn command_fns() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<Original>()
        .set_command_fns(replace, command_fns::default_remove::<Replaced>)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, Original)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&Replaced>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<Original>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
}

#[test]
fn marker() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .register_marker::<ReplaceMarker>()
        .replicate::<Original>()
        .set_marker_fns::<ReplaceMarker, _>(replace, command_fns::default_remove::<Replaced>)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Original, Signature::from(0)))
        .id();

    let client_entity = client_app
        .world_mut()
        .spawn((ReplaceMarker, Signature::from(0)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<Original>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    assert!(!client_entity.contains::<Replaced>());
}

#[test]
fn group() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate_bundle::<(A, B)>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, (A, B))).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, (With<A>, With<B>)>()
        .single(client_app.world())
        .unwrap();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<(A, B)>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    assert!(!client_entity.contains::<A>());
    assert!(!client_entity.contains::<B>());
}

#[test]
fn not_replicated() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, NotReplicated))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, (With<Replicated>, Without<NotReplicated>)>()
        .single(client_app.world())
        .unwrap();

    client_app
        .world_mut()
        .entity_mut(client_entity)
        .insert(NotReplicated);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<NotReplicated>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    assert!(client_entity.contains::<NotReplicated>());
}

#[test]
fn after_insertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    // Insert and remove at the same time.
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(A)
        .remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
}

#[test]
fn with_spawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, A)).remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app
        .world_mut()
        .query_filtered::<&Replicated, Without<A>>();
    assert_eq!(components.iter(client_app.world()).len(), 1);
}

#[test]
fn with_despawn() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).len(), 1);

    // Un-replicate and remove at the same time.
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>()
        .remove::<Replicated>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(replicated.iter(client_app.world()).len(), 0);
}

#[test]
fn confirm_history() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, With<A>>()
        .single(client_app.world())
        .unwrap();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    // Clear previous events.
    client_app
        .world_mut()
        .resource_mut::<Events<EntityReplicated>>()
        .clear();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();

    let confirm_history = client_app
        .world_mut()
        .get::<ConfirmHistory>(client_entity)
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
fn hidden() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin {
                tick_schedule: PostUpdate.intern(),
                visibility_policy: VisibilityPolicy::Whitelist, // Hide all spawned entities by default.
                ..Default::default()
            }),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(
        replicated.iter(client_app.world()).len(),
        0,
        "client shouldn't know about hidden entity"
    );
}

#[derive(Component, Deserialize, Serialize)]
struct A;

#[derive(Component, Deserialize, Serialize)]
struct B;

#[derive(Component, Deserialize, Serialize)]
struct NotReplicated;

#[derive(Component)]
struct ReplaceMarker;

#[derive(Component, Deserialize, Serialize)]
struct Original;

#[derive(Component, Deserialize, Serialize)]
struct Replaced;

/// Deserializes [`OriginalComponent`], but ignores it and inserts [`ReplacedComponent`].
fn replace(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<Original>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    rule_fns.deserialize(ctx, message)?;
    entity.insert(Replaced);

    Ok(())
}
