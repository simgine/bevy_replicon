use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    prelude::*,
    shared::replication::{
        deferred_entity::DeferredEntity,
        registry::{ctx::WriteCtx, receive_fns},
    },
    test_app::ServerTestAppExt,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[test]
fn acknowledged_diff() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<Points>()
        .replicate::<Value>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Points::new(vec![0]), Value(0)))
        .id();

    server_app.update();
    client_app.set_receive_fns::<Points>(track_write, receive_fns::default_remove::<Points>);

    // Hold the initial update so mutations are buffered despite being ACKed.
    let (_, channel, initial) = server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .next()
        .unwrap();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(1))
        .unwrap();
    server_app
        .world_mut()
        .get_mut::<Value>(server_entity)
        .unwrap()
        .0 = 1;

    server_app.update();

    // The mutation is ACKed on receipt, even though its initial update message is missing.
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    let mut points_query = client_app.world_mut().query::<(Entity, &Points, &Value)>();
    assert_eq!(points_query.iter(client_app.world()).count(), 0);

    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    server_app
        .world_mut()
        .get_mut::<Value>(server_entity)
        .unwrap()
        .0 = 2;
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(2))
        .unwrap();

    server_app.update();

    // The server now sends only diff 2. The client processes the newer mutation
    // message before the buffered first mutation that it depends on.
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(channel, initial);
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let (client_entity, points, value) = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1, 2]);
    assert_eq!(points.applied_diffs, 2);
    assert_eq!(value.0, 2, "the older dependency must not overwrite Value");

    let writes = client_app
        .world()
        .resource::<ReplicationStorage>()
        .get::<WriteCalls>(client_entity)
        .unwrap();
    assert_eq!(
        writes.0, 3,
        "the older dependency must use the custom write function"
    );
}

#[test]
fn acknowledged_snapshot() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<Points>()
        .replicate::<Value>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Points::new(vec![0]), Value(0)))
        .id();

    server_app.update();

    // Hold the initial update so mutations are buffered despite being ACKed.
    let (_, channel, initial) = server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .next()
        .unwrap();

    server_app
        .world_mut()
        .get_mut::<Points>(server_entity)
        .unwrap()
        .values
        .push(1);
    server_app
        .world_mut()
        .get_mut::<Value>(server_entity)
        .unwrap()
        .0 = 1;

    server_app.update();
    let (_, mutation_channel, first) = server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .next()
        .unwrap();

    // The mutation is ACKed on receipt, even though its initial update message is missing.
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(mutation_channel, first.clone());
    client_app.update();
    let mut points_query = client_app.world_mut().query::<(Entity, &Points, &Value)>();
    assert_eq!(points_query.iter(client_app.world()).count(), 0);

    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    server_app
        .world_mut()
        .get_mut::<Value>(server_entity)
        .unwrap()
        .0 = 2;
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(2))
        .unwrap();

    server_app.update();

    // The server now sends only diff 2. The client processes the newer mutation
    // message before the buffered first mutation that it depends on.
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(channel, initial);
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let (_, points, value) = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1, 2]);
    assert_eq!(points.applied_diffs, 1);
    assert_eq!(value.0, 2, "the older dependency must not overwrite Value");

    // Replaying the snapshot must not reset the diff cursor or ordinary component.
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(mutation_channel, first);
    client_app.update();

    let (_, points, value) = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1, 2]);
    assert_eq!(points.applied_diffs, 1);
    assert_eq!(value.0, 2);
}

#[test]
fn acknowledged_diff_merged_with_insertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<Points>()
        .replicate::<Marker>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Points::new(vec![0])))
        .id();

    server_app.update();

    // Hold the initial update so mutations are buffered despite being ACKed.
    let (_, channel, initial) = server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .next()
        .unwrap();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(1))
        .unwrap();

    server_app.update();

    // The mutation is ACKed on receipt, even though its initial update message is missing.
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    let mut points_query = client_app.world_mut().query::<(&Points, Has<Marker>)>();
    assert_eq!(points_query.iter(client_app.world()).count(), 0);

    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(2))
        .unwrap();
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Marker);

    server_app.update();

    // The server now sends only diff 2. The client processes the newer update
    // message before the buffered first mutation that it depends on.
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(channel, initial);
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let (points, has_marker) = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1, 2]);
    assert_eq!(points.applied_diffs, 2);
    assert!(has_marker);
}

#[test]
fn acknowledged_diff_outside_confirm_history() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<Points>()
        .replicate::<Value>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Points::new(vec![0]), Value(0)))
        .id();

    server_app.update();

    // Hold the initial update so mutations are buffered despite being ACKed.
    let (_, channel, initial) = server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .next()
        .unwrap();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(1))
        .unwrap();
    server_app
        .world_mut()
        .get_mut::<Value>(server_entity)
        .unwrap()
        .0 = 1;

    server_app.update();

    // The mutation is ACKed on receipt, even though its initial update message is missing.
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    let mut points_query = client_app.world_mut().query::<(Entity, &Points, &Value)>();
    assert_eq!(points_query.iter(client_app.world()).count(), 0);

    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    // A diff dependency must survive even when its tick no longer fits in the
    // entity's confirmation bitmask after the newer message is processed.
    for _ in 0..u64::BITS {
        server_app.update();
    }

    server_app
        .world_mut()
        .get_mut::<Value>(server_entity)
        .unwrap()
        .0 = 2;
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(2))
        .unwrap();

    server_app.update();

    // The server now sends only diff 2. The client processes the newer mutation
    // message before the buffered first mutation that it depends on.
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(channel, initial);
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let (_, points, value) = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1, 2]);
    assert_eq!(points.applied_diffs, 2);
    assert_eq!(value.0, 2, "the older dependency must not overwrite Value");
}

#[test]
fn snapshot_after_removal() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Points::new(vec![0])))
        .id();

    server_app.update();

    // Hold the initial update so mutations are buffered despite being ACKed.
    let (_, channel, initial) = server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .next()
        .unwrap();

    server_app
        .world_mut()
        .get_mut::<Points>(server_entity)
        .unwrap()
        .values
        .push(1);

    server_app.update();
    let (_, mutation_channel, old_mutation) = server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .next()
        .unwrap();

    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(mutation_channel, old_mutation.clone());
    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<Points>();

    server_app.update();

    // The old buffered mutation becomes eligible only after the component was
    // removed.
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(channel, initial);
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(mutation_channel, old_mutation);
    client_app.update();

    let mut points_query = client_app.world_mut().query::<&Points>();
    assert_eq!(points_query.iter(client_app.world()).count(), 0);
}

#[test]
fn mutation_after_reinsertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Points::new(vec![0])))
        .id();

    server_app.update();

    // Hold the initial update so mutations are buffered despite being ACKed.
    let (_, channel, initial) = server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .next()
        .unwrap();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(1))
        .unwrap();

    server_app.update();
    let (_, mutation_channel, old_mutation) = server_app
        .world_mut()
        .resource_mut::<ServerMessages>()
        .drain_sent()
        .next()
        .unwrap();

    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(mutation_channel, old_mutation.clone());
    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<Points>();

    server_app.update();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Points::new(vec![10]));

    server_app.update();

    // The old buffered mutation becomes eligible only after the component was
    // removed and reinserted with an unrelated snapshot and cursor.
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(channel, initial);
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    client_app
        .world_mut()
        .resource_mut::<ClientMessages>()
        .insert_received(mutation_channel, old_mutation);
    client_app.update();

    let mut points_query = client_app.world_mut().query::<&Points>();
    let points = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [10]);
    assert_eq!(points.applied_diffs, 0);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(11))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [10, 11]);
    assert_eq!(points.applied_diffs, 1);
}

#[derive(Default)]
struct WriteCalls(usize);

fn track_write(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<Points>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    ctx.get_or_default::<WriteCalls>().0 += 1;
    receive_fns::default_write(ctx, rule_fns, entity, message)
}

#[derive(Component, Deserialize, Serialize)]
struct Marker;

#[derive(Component, Deserialize, Serialize)]
struct Value(usize);

#[derive(Resource, Deserialize, Serialize, Debug, Clone)]
struct Points {
    values: Vec<usize>,
    // Snapshots reset this counter instead of copying the server's diff applications.
    #[serde(skip)]
    applied_diffs: usize,
}

impl Points {
    fn new(values: Vec<usize>) -> Self {
        Self {
            values,
            applied_diffs: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct AddPoint(usize);

impl Diffable for Points {
    type Diff = AddPoint;
    const HISTORY_LEN: usize = 5;

    fn apply_diff(&mut self, diff: &Self::Diff) -> Result<()> {
        self.values.push(diff.0);
        self.applied_diffs += 1;
        Ok(())
    }
}
