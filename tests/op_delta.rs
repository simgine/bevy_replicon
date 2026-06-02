extern crate alloc;

use alloc::collections::VecDeque;

use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    postcard_utils,
    prelude::*,
    shared::{
        backend::{
            channels::ServerChannel, client_messages::ClientMessages,
            server_messages::ServerMessages,
        },
        replication::{
            op_delta::{OpDeltaReceiver, OpDeltaWire},
            registry::test_fns::TestFnsEntityExt,
            rules::ReplicationRules,
        },
    },
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[derive(Component, Debug, Deserialize, Serialize)]
struct Points(VecDeque<Vec2>);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum PointOp {
    PushBack(Vec2),
    PopFront(usize),
}

impl OpDeltaComponent for Points {
    type Op = PointOp;

    fn apply_op(&mut self, op: &Self::Op) -> Result<()> {
        match *op {
            PointOp::PushBack(point) => self.0.push_back(point),
            PointOp::PopFront(count) => {
                for _ in 0..count {
                    self.0.pop_front();
                }
            }
        }

        Ok(())
    }
}

#[test]
fn initial_snapshot_ops_and_direct_snapshot_fallback_replicate() {
    let (mut server_app, mut client_app) = setup_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, points([(1.0, 1.0)])))
        .id();
    assert!(
        server_app
            .world()
            .entity(server_entity)
            .contains::<OpDeltaLog<Points>>(),
        "op-delta components should automatically get an operation log"
    );

    replicate_and_ack(&mut server_app, &mut client_app);
    assert_client_points(&mut client_app, [(1.0, 1.0)]);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_op_delta::<Points>(PointOp::PushBack(Vec2::new(2.0, 2.0)))
        .unwrap();
    replicate_and_ack(&mut server_app, &mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_op_delta::<Points>(PointOp::PushBack(Vec2::new(3.0, 3.0)))
        .unwrap();
    replicate_and_ack(&mut server_app, &mut client_app);

    assert_client_points(&mut client_app, [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_op_delta::<Points>(PointOp::PopFront(1))
        .unwrap();
    replicate_and_ack(&mut server_app, &mut client_app);

    assert_client_points(&mut client_app, [(2.0, 2.0), (3.0, 3.0)]);

    server_app
        .world_mut()
        .get_mut::<Points>(server_entity)
        .unwrap()
        .0
        .push_back(Vec2::new(4.0, 4.0));
    replicate_and_ack(&mut server_app, &mut client_app);

    assert_client_points(&mut client_app, [(2.0, 2.0), (3.0, 3.0), (4.0, 4.0)]);
}

#[test]
fn lost_op_is_included_in_next_unacked_delta() {
    let (mut server_app, mut client_app) = setup_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, points([(1.0, 1.0)])))
        .id();
    replicate_and_ack(&mut server_app, &mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_op_delta::<Points>(PointOp::PushBack(Vec2::new(2.0, 2.0)))
        .unwrap();
    server_app.update();
    let dropped = drain_server_channel(&mut server_app, ServerChannel::Mutations);
    assert_eq!(dropped.len(), 1, "first op should be sent as a mutation");

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_op_delta::<Points>(PointOp::PushBack(Vec2::new(3.0, 3.0)))
        .unwrap();
    server_app.update();
    deliver_server_messages(&mut server_app, &mut client_app);
    client_app.update();

    assert_client_points(&mut client_app, [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);
}

#[test]
fn pruned_ops_fall_back_to_snapshot_and_then_resume_ops() {
    let (mut server_app, mut client_app) = setup_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, points([(0.0, 0.0)])))
        .id();
    replicate_and_ack(&mut server_app, &mut client_app);

    for value in 1..=65 {
        server_app
            .world_mut()
            .entity_mut(server_entity)
            .apply_op_delta::<Points>(PointOp::PushBack(Vec2::splat(value as f32)))
            .unwrap();
    }
    replicate_and_ack(&mut server_app, &mut client_app);
    assert_client_point_values(&mut client_app, 0..=65);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_op_delta::<Points>(PointOp::PushBack(Vec2::splat(66.0)))
        .unwrap();
    replicate_and_ack(&mut server_app, &mut client_app);
    assert_client_point_values(&mut client_app, 0..=66);
}

#[test]
fn removal_removes_receiver_state() {
    let (mut server_app, mut client_app) = setup_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, points([(1.0, 1.0)])))
        .id();
    replicate_and_ack(&mut server_app, &mut client_app);

    let client_entity = single_client_entity(&mut client_app);
    let entity = client_app.world().entity(client_entity);
    assert!(entity.contains::<Points>());
    assert!(entity.contains::<OpDeltaReceiver<Points>>());
    assert!(entity.contains::<OpDeltaLog<Points>>());

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<Points>();
    replicate_and_ack(&mut server_app, &mut client_app);

    let entity = client_app.world().entity(client_entity);
    assert!(!entity.contains::<Points>());
    assert!(!entity.contains::<OpDeltaReceiver<Points>>());
    assert!(!entity.contains::<OpDeltaLog<Points>>());
}

#[test]
fn duplicate_ops_are_ignored_by_receiver() {
    let mut app = setup_app();
    let fns_id = points_fns_id(&app);
    let mut entity = app.world_mut().spawn_empty();

    entity.apply_write(
        snapshot(0, points([(1.0, 1.0)])),
        fns_id,
        RepliconTick::default(),
    );
    entity.apply_write(
        ops(0, 1, [(1, PointOp::PushBack(Vec2::new(2.0, 2.0)))]),
        fns_id,
        RepliconTick::default(),
    );
    entity.apply_write(
        ops(0, 1, [(1, PointOp::PushBack(Vec2::new(2.0, 2.0)))]),
        fns_id,
        RepliconTick::default(),
    );

    assert_entity_points(&entity, [(1.0, 1.0), (2.0, 2.0)]);
}

#[test]
fn out_of_order_ops_wait_for_missing_predecessor() {
    let mut app = setup_app();
    let fns_id = points_fns_id(&app);
    let mut entity = app.world_mut().spawn_empty();

    entity.apply_write(
        snapshot(0, points([(1.0, 1.0)])),
        fns_id,
        RepliconTick::default(),
    );
    entity.apply_write(
        ops(1, 2, [(2, PointOp::PushBack(Vec2::new(3.0, 3.0)))]),
        fns_id,
        RepliconTick::default(),
    );
    assert_entity_points(&entity, [(1.0, 1.0)]);

    entity.apply_write(
        ops(0, 1, [(1, PointOp::PushBack(Vec2::new(2.0, 2.0)))]),
        fns_id,
        RepliconTick::default(),
    );
    assert_entity_points(&entity, [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);
}

#[test]
#[should_panic(expected = "writing data into an entity shouldn't fail")]
fn ops_before_snapshot_are_rejected() {
    let mut app = setup_app();
    let fns_id = points_fns_id(&app);
    let mut entity = app.world_mut().spawn_empty();

    entity.apply_write(
        ops(0, 1, [(1, PointOp::PushBack(Vec2::new(1.0, 1.0)))]),
        fns_id,
        RepliconTick::default(),
    );
}

fn setup_apps() -> (App, App) {
    let server_app = setup_app();
    let client_app = setup_app();
    (server_app, client_app)
}

fn setup_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .replicate_op_delta::<Points>()
    .finish();
    app
}

fn replicate_and_ack(server_app: &mut App, client_app: &mut App) {
    server_app.update();
    server_app.exchange_with_client(client_app);
    client_app.update();
    server_app.exchange_with_client(client_app);
    server_app.update();
}

fn deliver_server_messages(server_app: &mut App, client_app: &mut App) {
    let messages = drain_server_messages_where(server_app, |_| true);
    let mut client_messages = client_app.world_mut().resource_mut::<ClientMessages>();
    for (_, channel_id, message) in messages {
        client_messages.insert_received(channel_id, message);
    }
}

fn drain_server_channel(
    server_app: &mut App,
    channel: ServerChannel,
) -> Vec<(Entity, usize, bytes::Bytes)> {
    let channel = usize::from(channel);
    drain_server_messages_where(server_app, |channel_id| channel_id == channel)
}

fn drain_server_messages_where(
    server_app: &mut App,
    mut filter: impl FnMut(usize) -> bool,
) -> Vec<(Entity, usize, bytes::Bytes)> {
    let (retained, drained) = {
        let mut server_messages = server_app.world_mut().resource_mut::<ServerMessages>();
        let mut retained = Vec::new();
        let mut drained = Vec::new();
        for message in server_messages.drain_sent() {
            if filter(message.1) {
                drained.push(message);
            } else {
                retained.push(message);
            }
        }

        (retained, drained)
    };

    let mut server_messages = server_app.world_mut().resource_mut::<ServerMessages>();
    for (client, channel_id, message) in retained {
        server_messages.send(client, channel_id, message);
    }

    drained
}

fn assert_client_points<const N: usize>(client_app: &mut App, expected: [(f32, f32); N]) {
    assert_client_points_slice(client_app, &expected);
}

fn assert_client_point_values(client_app: &mut App, expected: impl IntoIterator<Item = i32>) {
    let expected: Vec<_> = expected
        .into_iter()
        .map(|value| {
            let value = value as f32;
            (value, value)
        })
        .collect();
    assert_client_points_slice(client_app, &expected);
}

fn assert_client_points_slice(client_app: &mut App, expected: &[(f32, f32)]) {
    let mut points = client_app.world_mut().query::<&Points>();
    let points = points.single(client_app.world()).unwrap();
    let points: Vec<_> = points.0.iter().map(|point| (point.x, point.y)).collect();
    assert_eq!(points, expected);
}

fn single_client_entity(client_app: &mut App) -> Entity {
    let mut entities = client_app
        .world_mut()
        .query_filtered::<Entity, With<Remote>>();
    entities.single(client_app.world()).unwrap()
}

fn assert_entity_points<const N: usize>(entity: &EntityWorldMut, expected: [(f32, f32); N]) {
    let points = entity.get::<Points>().unwrap();
    let points: Vec<_> = points.0.iter().map(|point| (point.x, point.y)).collect();
    assert_eq!(points, expected);
}

fn points<const N: usize>(points: [(f32, f32); N]) -> Points {
    Points(points.into_iter().map(|(x, y)| Vec2::new(x, y)).collect())
}

fn snapshot(cursor: OpIndex, value: Points) -> Vec<u8> {
    wire(OpDeltaWire::Snapshot { cursor, value })
}

fn ops<const N: usize>(
    base_cursor: OpIndex,
    cursor: OpIndex,
    ops: [(OpIndex, PointOp); N],
) -> Vec<u8> {
    wire(OpDeltaWire::Ops {
        base_cursor,
        cursor,
        ops: ops
            .into_iter()
            .map(|(seq, op)| SequencedOp { seq, op })
            .collect(),
    })
}

fn wire(wire: OpDeltaWire<Points, PointOp>) -> Vec<u8> {
    let mut message = Vec::new();
    postcard_utils::to_extend_mut(&wire, &mut message).unwrap();
    message
}

fn points_fns_id(app: &App) -> bevy_replicon::shared::replication::registry::FnsId {
    app.world().resource::<ReplicationRules>()[0].components[0].fns_id
}
