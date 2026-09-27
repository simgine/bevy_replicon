use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};
use serde::{Deserialize, Serialize};

#[test]
fn component() {
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
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut points_query = client_app.world_mut().query::<&Points>();
    let points = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0]);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(1))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1]);
    assert_eq!(points.applied_diffs, 1);
}

#[test]
fn resource() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_resource_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.insert_resource(Points::new(vec![0]));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = client_app.world().resource::<Points>();
    assert_eq!(points.values, [0]);

    // Exceed the history window to verify that acknowledgments advance the cursor.
    for index in 1..=Points::HISTORY_LEN + 1 {
        server_app
            .world_mut()
            .apply_resource_diff::<Points>(AddPoint(index))
            .unwrap();

        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let points = client_app.world().resource::<Points>();
        assert_eq!(points.values, (0..=index).collect::<Vec<_>>());
        assert_eq!(points.applied_diffs, index);
    }
}

#[test]
fn message_loss() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_resource_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.insert_resource(Points::new(vec![0]));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = client_app.world().resource::<Points>();
    assert_eq!(points.values.len(), 1);

    server_app
        .world_mut()
        .apply_resource_diff::<Points>(AddPoint(1))
        .unwrap();

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(messages.drain_sent().len(), 1);

    server_app
        .world_mut()
        .apply_resource_diff::<Points>(AddPoint(2))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = client_app.world().resource::<Points>();
    assert_eq!(points.values, [0, 1, 2]);
    assert_eq!(points.applied_diffs, 2);
}

#[test]
fn outside_of_history_window() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_resource_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.insert_resource(Points::new(vec![0]));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = client_app.world().resource::<Points>();
    assert_eq!(points.values.len(), 1);

    for index in 1..=Points::HISTORY_LEN {
        server_app
            .world_mut()
            .apply_resource_diff::<Points>(AddPoint(index))
            .unwrap();
    }

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(messages.drain_sent().len(), 1);

    server_app
        .world_mut()
        .apply_resource_diff::<Points>(AddPoint(100))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = client_app.world().resource::<Points>();
    assert_eq!(points.values, [0, 1, 2, 3, 4, 5, 100]);
    assert_eq!(points.values.len(), Points::HISTORY_LEN + 2);
    assert_eq!(points.applied_diffs, 0);
}

#[test]
fn external_mutation() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_resource_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.insert_resource(Points::new(vec![0]));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = client_app.world().resource::<Points>();
    assert_eq!(points.values.len(), 1);

    let mut points = server_app.world_mut().resource_mut::<Points>();
    points.values.push(1);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = client_app.world().resource::<Points>();
    assert_eq!(points.values, [0, 1]);
    assert_eq!(points.applied_diffs, 0);

    // Acknowledge the snapshot before sending the next diff.
    server_app.exchange_with_client(&mut client_app);
    server_app
        .world_mut()
        .apply_resource_diff::<Points>(AddPoint(2))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = client_app.world().resource::<Points>();
    assert_eq!(points.values, [0, 1, 2]);
    assert_eq!(points.applied_diffs, 1);
}

#[test]
fn cached_snapshot() {
    let mut server_app = App::new();
    let mut client_app1 = App::new();
    let mut client_app2 = App::new();
    for app in [&mut server_app, &mut client_app1, &mut client_app2] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_resource_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app1);
    server_app.connect_client(&mut client_app2);
    server_app.insert_resource(Points::new(vec![0]));

    // Both clients receive the same serialized snapshot in this tick.
    server_app.update();
    for app in [&mut client_app1, &mut client_app2] {
        server_app.exchange_with_client(app);
        app.update();
        assert_eq!(app.world().resource::<Points>().values, [0]);
    }

    server_app
        .world_mut()
        .apply_resource_diff::<Points>(AddPoint(1))
        .unwrap();

    server_app.update();
    for app in [&mut client_app1, &mut client_app2] {
        server_app.exchange_with_client(app);
        app.update();

        let points = app.world().resource::<Points>();
        assert_eq!(points.values, [0, 1]);
        assert_eq!(points.applied_diffs, 1);
    }
}

#[test]
fn snapshot_merged_with_insertion() {
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
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut entity = server_app.world_mut().entity_mut(server_entity);
    // Mutating `Points` directly forces a snapshot.
    entity.get_mut::<Points>().unwrap().values.push(1);

    // Inserting `Marker` moves the `Points` snapshot into the update message.
    // Its cursor must be preserved for the next diff.
    entity.insert(Marker);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut points_query = client_app.world_mut().query::<(&Points, Has<Marker>)>();
    let (points, has_marker) = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1]);
    assert_eq!(points.applied_diffs, 0);
    assert!(has_marker);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(2))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let (points, _) = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1, 2]);
    assert_eq!(points.applied_diffs, 1);
}

#[test]
fn snapshot_merged_with_removal() {
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
        .spawn((Replicated, Points::new(vec![0]), Marker))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut entity = server_app.world_mut().entity_mut(server_entity);
    // Mutating `Points` directly forces a snapshot.
    entity.get_mut::<Points>().unwrap().values.push(1);

    // Removing `Marker` moves the `Points` snapshot into the update message.
    // Its cursor must be preserved for the next diff.
    entity.remove::<Marker>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut points_query = client_app.world_mut().query::<(&Points, Has<Marker>)>();
    let (points, has_marker) = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1]);
    assert_eq!(points.applied_diffs, 0);
    assert!(!has_marker);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_diff::<Points>(AddPoint(2))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let (points, _) = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.values, [0, 1, 2]);
    assert_eq!(points.applied_diffs, 1);
}

#[derive(Component, Deserialize, Serialize)]
struct Marker;

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
