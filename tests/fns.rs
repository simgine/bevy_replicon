use bevy::prelude::*;
use bevy_replicon::{
    prelude::*,
    shared::{
        replication::{
            command_markers::MarkerConfig,
            deferred_entity::DeferredEntity,
            registry::{
                ReplicationRegistry, command_fns,
                ctx::{DespawnCtx, WriteCtx},
                test_fns::TestFnsEntityExt,
            },
        },
        replicon_tick::RepliconTick,
    },
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
#[should_panic]
fn serialize_missing_component() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app.world_mut().spawn_empty();
    let _ = entity.serialize(fns_id, tick);
}

#[test]
fn write() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app.world_mut().spawn(OriginalComponent);
    let data = entity.serialize(fns_id, tick);
    entity.remove::<OriginalComponent>();
    entity.apply_write(data, fns_id, tick);
    assert!(entity.contains::<OriginalComponent>());
}

#[test]
fn remove() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app.world_mut().spawn(OriginalComponent);
    entity.apply_remove(fns_id, tick);
    assert!(!entity.contains::<OriginalComponent>());
}

#[test]
fn write_with_command() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .set_command_fns(replace, command_fns::default_remove::<ReplacedComponent>);

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app.world_mut().spawn(OriginalComponent);
    let data = entity.serialize(fns_id, tick);
    entity.apply_write(data, fns_id, tick);
    assert!(entity.contains::<ReplacedComponent>());
}

#[test]
fn remove_with_command() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .set_command_fns(replace, command_fns::default_remove::<ReplacedComponent>);

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app.world_mut().spawn(ReplacedComponent);
    entity.apply_remove(fns_id, tick);
    assert!(!entity.contains::<ReplacedComponent>());
}

#[test]
fn write_without_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        );

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app.world_mut().spawn(OriginalComponent);
    let data = entity.serialize(fns_id, tick);
    entity.remove::<OriginalComponent>();
    entity.apply_write(data, fns_id, tick);
    assert!(entity.contains::<OriginalComponent>());
}

#[test]
fn remove_without_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        );

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app.world_mut().spawn(OriginalComponent);
    entity.apply_remove(fns_id, tick);
    assert!(!entity.contains::<OriginalComponent>());
}

#[test]
fn write_with_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        );

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app.world_mut().spawn((OriginalComponent, ReplaceMarker));
    let data = entity.serialize(fns_id, tick);
    entity.apply_write(data, fns_id, tick);
    assert!(entity.contains::<ReplacedComponent>());
}

#[test]
fn remove_with_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        );

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app.world_mut().spawn((ReplacedComponent, ReplaceMarker));
    entity.apply_remove(fns_id, tick);
    assert!(!entity.contains::<ReplacedComponent>());
}

#[test]
fn write_with_multiple_markers() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<Marker>()
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        )
        .set_marker_fns::<Marker, _>(
            command_fns::default_write::<OriginalComponent>,
            command_fns::default_remove::<OriginalComponent>,
        );

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app
        .world_mut()
        .spawn((OriginalComponent, ReplaceMarker, Marker));
    let data = entity.serialize(fns_id, tick);
    entity.apply_write(data, fns_id, tick);
    assert!(
        entity.contains::<ReplacedComponent>(),
        "last marker should take priority"
    );
}

#[test]
fn remove_with_multiple_markers() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker::<Marker>()
        .register_marker::<ReplaceMarker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        )
        .set_marker_fns::<Marker, _>(
            command_fns::default_write::<OriginalComponent>,
            command_fns::default_remove::<OriginalComponent>,
        );

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app
        .world_mut()
        .spawn((ReplacedComponent, ReplaceMarker, Marker));
    entity.apply_remove(fns_id, tick);
    assert!(
        !entity.contains::<ReplacedComponent>(),
        "last marker should take priority"
    );
}

#[test]
fn write_with_priority_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker_with::<ReplaceMarker>(MarkerConfig {
            priority: 1,
            ..Default::default()
        })
        .register_marker::<Marker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        )
        .set_marker_fns::<Marker, _>(
            command_fns::default_write::<OriginalComponent>,
            command_fns::default_remove::<OriginalComponent>,
        );

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app
        .world_mut()
        .spawn((OriginalComponent, ReplaceMarker, Marker));
    let data = entity.serialize(fns_id, tick);
    entity.apply_write(data, fns_id, tick);
    assert!(entity.contains::<ReplacedComponent>());
}

#[test]
fn remove_with_priority_marker() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins))
        .register_marker_with::<ReplaceMarker>(MarkerConfig {
            priority: 1,
            ..Default::default()
        })
        .register_marker::<Marker>()
        .set_marker_fns::<ReplaceMarker, _>(
            replace,
            command_fns::default_remove::<ReplacedComponent>,
        )
        .set_marker_fns::<Marker, _>(
            command_fns::default_write::<OriginalComponent>,
            command_fns::default_remove::<OriginalComponent>,
        );

    let tick = RepliconTick::default();
    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(world, RuleFns::<OriginalComponent>::default())
            });

    let mut entity = app
        .world_mut()
        .spawn((ReplacedComponent, ReplaceMarker, Marker));
    entity.apply_remove(fns_id, tick);
    assert!(!entity.contains::<ReplacedComponent>());
}

#[test]
fn despawn() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));

    let mut registry = app.world_mut().resource_mut::<ReplicationRegistry>();
    registry.despawn = mark_despawned;

    let tick = RepliconTick::default();
    let entity = app.world_mut().spawn_empty();
    let id = entity.id(); // Take ID since despawn function consumes entity.
    entity.apply_despawn(tick);
    assert!(app.world().get::<Despawned>(id).is_some());
}

#[derive(Component, Deserialize, Serialize)]
struct OriginalComponent;

#[derive(Component, Deserialize, Serialize)]
struct ReplacedComponent;

#[derive(Component)]
struct Despawned;

#[derive(Component, Deserialize, Serialize)]
struct ReplaceMarker;

#[derive(Component, Deserialize, Serialize)]
struct Marker;

/// Deserializes [`OriginalComponent`], but ignores it and inserts [`ReplacedComponent`].
fn replace(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<OriginalComponent>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    rule_fns.deserialize(ctx, message)?;
    entity.insert(ReplacedComponent);

    Ok(())
}

/// Adds special [`Despawned`] marker instead of despawning an entity.
fn mark_despawned(_ctx: &DespawnCtx, mut entity: EntityWorldMut) {
    entity.insert(Despawned);
}
