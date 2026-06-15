extern crate alloc;

use alloc::collections::VecDeque;

use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    bytes::Bytes,
    postcard_utils,
    prelude::*,
    shared::{
        backend::{
            channels::ServerChannel, client_messages::ClientMessages,
            server_messages::ServerMessages,
        },
        replication::{
            deferred_entity::DeferredEntity,
            diff::{DiffWire, PatchBuffer},
            receive_markers::MarkerConfig,
            registry::{
                ctx::{RemoveCtx, WriteCtx},
                rule_fns::RuleFns,
                test_fns::TestFnsEntityExt,
            },
            rules::ReplicationRules,
        },
    },
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[derive(Clone, Component, Debug, Deserialize, Serialize)]
struct Points(VecDeque<Vec2>);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum PointPatch {
    PushBack(Vec2),
    PopFront(usize),
}

impl Diffable for Points {
    type Patch = PointPatch;

    fn apply_patch(&mut self, patch: &Self::Patch) -> Result<()> {
        match *patch {
            PointPatch::PushBack(point) => self.0.push_back(point),
            PointPatch::PopFront(count) => {
                for _ in 0..count {
                    self.0.pop_front();
                }
            }
        }

        Ok(())
    }
}

#[derive(Component)]
struct HistoryMarker;

#[derive(Component, Default)]
struct PointHistory(Vec<(RepliconTick, Option<PatchIndex>, Points)>);

#[test]
fn initial_snapshot_patches_and_direct_snapshot_fallback_replicate() {
    let (mut server_app, mut client_app) = setup_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = spawn_replicated_points(&mut server_app, [(1.0, 1.0)]);

    replicate_and_ack(&mut server_app, &mut client_app);
    assert_client_points(&mut client_app, [(1.0, 1.0)]);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::new(2.0, 2.0)))
        .unwrap();
    replicate_and_ack(&mut server_app, &mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::new(3.0, 3.0)))
        .unwrap();
    replicate_and_ack(&mut server_app, &mut client_app);

    assert_client_points(&mut client_app, [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PopFront(1))
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
fn lost_patch_is_included_in_next_unacked_diff() {
    let (mut server_app, mut client_app) = setup_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = spawn_replicated_points(&mut server_app, [(1.0, 1.0)]);
    replicate_and_ack(&mut server_app, &mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::new(2.0, 2.0)))
        .unwrap();
    server_app.update();
    let dropped = drain_server_channel(&mut server_app, ServerChannel::Mutations);
    assert_eq!(dropped.len(), 1, "first patch should be sent as a mutation");

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::new(3.0, 3.0)))
        .unwrap();
    server_app.update();
    deliver_server_messages(&mut server_app, &mut client_app);
    client_app.update();

    assert_client_points(&mut client_app, [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);
}

#[test]
fn cumulative_diff_applies_before_older_subset_diff() {
    let (mut server_app, mut client_app) = setup_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = spawn_replicated_points(&mut server_app, [(0.0, 0.0)]);
    replicate_and_ack(&mut server_app, &mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::splat(1.0)))
        .unwrap();
    server_app.update();
    let mutation_0_1 = drain_server_channel(&mut server_app, ServerChannel::Mutations);
    assert_eq!(mutation_0_1.len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::splat(2.0)))
        .unwrap();
    server_app.update();
    let mutation_0_2 = drain_server_channel(&mut server_app, ServerChannel::Mutations);
    assert_eq!(mutation_0_2.len(), 1);

    deliver_messages_to_client(&mut client_app, mutation_0_2);
    client_app.update();
    assert_client_point_values(&mut client_app, 0..=2);

    deliver_messages_to_client(&mut client_app, mutation_0_1);
    client_app.update();
    assert_client_point_values(&mut client_app, 0..=2);
}

#[test]
fn prediction_history_records_older_state_after_cumulative_diff_arrives_first() {
    let (mut server_app, mut client_app) = setup_history_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = spawn_replicated_points(&mut server_app, [(0.0, 0.0)]);
    replicate_and_ack(&mut server_app, &mut client_app);

    let client_entity = single_client_entity(&mut client_app);
    client_app
        .world_mut()
        .entity_mut(client_entity)
        .insert((HistoryMarker, PointHistory::default()));

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::splat(1.0)))
        .unwrap();
    server_app.update();
    let mutation_0_1 = drain_server_channel(&mut server_app, ServerChannel::Mutations);
    assert_eq!(mutation_0_1.len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::splat(2.0)))
        .unwrap();
    server_app.update();
    let mutation_0_2 = drain_server_channel(&mut server_app, ServerChannel::Mutations);
    assert_eq!(mutation_0_2.len(), 1);

    deliver_messages_to_client(&mut client_app, mutation_0_2);
    client_app.update();
    deliver_messages_to_client(&mut client_app, mutation_0_1);
    client_app.update();

    let mut history = point_history_values(&mut client_app);
    history.sort_by_key(|(tick, _)| *tick);
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].1, vec![(0.0, 0.0), (1.0, 1.0)]);
    assert_eq!(history[1].1, vec![(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)]);
}

#[test]
fn pruned_patches_fall_back_to_snapshot_and_then_resume_patches() {
    let (mut server_app, mut client_app) = setup_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = spawn_replicated_points(&mut server_app, [(0.0, 0.0)]);
    replicate_and_ack(&mut server_app, &mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::splat(1.0)))
        .unwrap();
    replicate_and_ack(&mut server_app, &mut client_app);
    assert_client_point_values(&mut client_app, 0..=1);

    for value in 2..=66 {
        server_app
            .world_mut()
            .entity_mut(server_entity)
            .apply_patch::<Points>(PointPatch::PushBack(Vec2::splat(value as f32)))
            .unwrap();
        server_app.update();
        drain_server_channel(&mut server_app, ServerChannel::Mutations);
    }

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::splat(67.0)))
        .unwrap();
    replicate_and_ack(&mut server_app, &mut client_app);
    assert_client_point_values(&mut client_app, 0..=67);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(PointPatch::PushBack(Vec2::splat(68.0)))
        .unwrap();
    replicate_and_ack(&mut server_app, &mut client_app);
    assert_client_point_values(&mut client_app, 0..=68);
}

#[test]
fn removal_removes_receiver_state() {
    let (mut server_app, mut client_app) = setup_apps();
    server_app.connect_client(&mut client_app);

    let server_entity = spawn_replicated_points(&mut server_app, [(1.0, 1.0)]);
    replicate_and_ack(&mut server_app, &mut client_app);

    let client_entity = single_client_entity(&mut client_app);
    let entity = client_app.world().entity(client_entity);
    assert!(entity.contains::<Points>());
    assert!(entity.contains::<PatchBuffer<Points>>());
    assert!(entity.contains::<PatchHistory<Points>>());

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<Points>();
    replicate_and_ack(&mut server_app, &mut client_app);

    let entity = client_app.world().entity(client_entity);
    assert!(!entity.contains::<Points>());
    assert!(!entity.contains::<PatchBuffer<Points>>());
    assert!(!entity.contains::<PatchHistory<Points>>());
}

#[test]
fn duplicate_patches_are_ignored_by_receiver() {
    let mut app = setup_app();
    let fns_id = points_fns_id(&app);
    let mut entity = app.world_mut().spawn_empty();

    entity.apply_write(
        wire(DiffWire::Snapshot {
            cursor: None,
            value: points([(1.0, 1.0)]),
        }),
        fns_id,
        RepliconTick::default(),
    );
    entity.apply_write(
        wire(DiffWire::Patches {
            first_index: PatchIndex::new(0),
            patches: vec![PointPatch::PushBack(Vec2::new(2.0, 2.0))],
        }),
        fns_id,
        RepliconTick::default(),
    );
    entity.apply_write(
        wire(DiffWire::Patches {
            first_index: PatchIndex::new(0),
            patches: vec![PointPatch::PushBack(Vec2::new(2.0, 2.0))],
        }),
        fns_id,
        RepliconTick::default(),
    );

    assert_entity_points(&entity, [(1.0, 1.0), (2.0, 2.0)]);
}

#[test]
fn out_of_order_patches_wait_for_missing_predecessor() {
    let mut app = setup_app();
    let fns_id = points_fns_id(&app);
    let mut entity = app.world_mut().spawn_empty();

    entity.apply_write(
        wire(DiffWire::Snapshot {
            cursor: None,
            value: points([(1.0, 1.0)]),
        }),
        fns_id,
        RepliconTick::default(),
    );
    entity.apply_write(
        wire(DiffWire::Patches {
            first_index: PatchIndex::new(1),
            patches: vec![PointPatch::PushBack(Vec2::new(3.0, 3.0))],
        }),
        fns_id,
        RepliconTick::default(),
    );
    assert_entity_points(&entity, [(1.0, 1.0)]);

    entity.apply_write(
        wire(DiffWire::Patches {
            first_index: PatchIndex::new(0),
            patches: vec![PointPatch::PushBack(Vec2::new(2.0, 2.0))],
        }),
        fns_id,
        RepliconTick::default(),
    );
    assert_entity_points(&entity, [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);
}

#[test]
#[should_panic(expected = "writing data into an entity shouldn't fail")]
fn patches_before_snapshot_are_rejected() {
    let mut app = setup_app();
    let fns_id = points_fns_id(&app);
    let mut entity = app.world_mut().spawn_empty();

    entity.apply_write(
        wire(DiffWire::Patches {
            first_index: PatchIndex::new(0),
            patches: vec![PointPatch::PushBack(Vec2::new(1.0, 1.0))],
        }),
        fns_id,
        RepliconTick::default(),
    );
}

fn setup_apps() -> (App, App) {
    let server_app = setup_app();
    let client_app = setup_app();
    (server_app, client_app)
}

fn setup_history_apps() -> (App, App) {
    let server_app = setup_history_app();
    let client_app = setup_history_app();
    (server_app, client_app)
}

fn setup_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .replicate_diff::<Points>()
    .finish();
    app
}

fn setup_history_app() -> App {
    let mut app = setup_app();
    app.register_marker_with::<HistoryMarker>(MarkerConfig {
        priority: 100,
        need_history: true,
    })
    .set_marker_fns::<HistoryMarker, Points>(write_point_history, remove_point_history);
    app
}

fn spawn_replicated_points<const N: usize>(app: &mut App, points: [(f32, f32); N]) -> Entity {
    app.world_mut()
        .spawn((Replicated, self::points(points)))
        .id()
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
    deliver_messages_to_client(client_app, messages);
}

fn deliver_messages_to_client(client_app: &mut App, messages: Vec<(Entity, usize, Bytes)>) {
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

fn write_point_history(
    ctx: &mut WriteCtx,
    _rule_fns: &RuleFns<Points>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    let (cursor, value) = match postcard_utils::from_buf(message)? {
        DiffWire::Snapshot { cursor, value } => {
            entity.insert(PatchBuffer::<Points>::new(cursor));
            (cursor, value)
        }
        DiffWire::Patches {
            first_index,
            patches,
        } => {
            if patches.is_empty() {
                return Ok(());
            }
            // Patch N transforms state cursor N - 1 into cursor N. Patch 0
            // transforms the pre-patch base, represented by `None`, into
            // cursor `Some(0)`.
            let base_cursor = (first_index != PatchIndex::new(0)).then_some(first_index - 1);
            let cursor = Some(first_index + patches.len() as u16 - 1);
            // The base must come from a confirmed value in the history: consumers
            // like prediction/interpolation may locally mutate the live component,
            // so it can never be used as a patch base.
            let mut value = entity
                .get::<PointHistory>()
                .and_then(|history| {
                    history.0.iter().rev().find_map(|(_, cursor, value)| {
                        (*cursor == base_cursor).then(|| value.clone())
                    })
                })
                .ok_or_else(|| {
                    format!(
                        "received diff patches for `{}` without a confirmed base",
                        ShortName::of::<Points>()
                    )
                })?;
            for patch in patches {
                value.apply_patch(&patch)?;
            }
            (cursor, value)
        }
    };

    if let Some(mut history) = entity.get_mut::<PointHistory>() {
        history.0.push((ctx.message_tick, cursor, value));
    } else {
        entity.insert(PointHistory(vec![(ctx.message_tick, cursor, value)]));
    }

    Ok(())
}

fn remove_point_history(_ctx: &mut RemoveCtx, entity: &mut DeferredEntity) {
    entity.remove::<PointHistory>().remove::<Points>();
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

fn point_history_values(client_app: &mut App) -> Vec<(RepliconTick, Vec<(f32, f32)>)> {
    let mut history = client_app.world_mut().query::<&PointHistory>();
    history
        .single(client_app.world())
        .unwrap()
        .0
        .iter()
        .map(|(tick, _, points)| {
            (
                *tick,
                points.0.iter().map(|point| (point.x, point.y)).collect(),
            )
        })
        .collect()
}

fn points<const N: usize>(points: [(f32, f32); N]) -> Points {
    Points(points.into_iter().map(|(x, y)| Vec2::new(x, y)).collect())
}

fn wire(wire: DiffWire<Points>) -> Vec<u8> {
    let mut message = Vec::new();
    postcard_utils::to_extend_mut(&wire, &mut message).unwrap();
    message
}

fn points_fns_id(app: &App) -> bevy_replicon::shared::replication::registry::FnsId {
    app.world().resource::<ReplicationRules>()[0].components[0].fns_id
}
