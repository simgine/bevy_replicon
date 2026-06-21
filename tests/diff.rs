use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};
use serde::{Deserialize, Serialize};

#[test]
fn patching() {
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
        .spawn((Replicated, Points(vec![0])))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut points_query = client_app.world_mut().query::<&Points>();
    let points = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.0, [0]);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(AddPoint(1))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.0, [0, 1]);
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
        .replicate_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Points(vec![0])))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut points_query = client_app.world_mut().query::<&Points>();
    assert_eq!(points_query.iter(client_app.world()).len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(AddPoint(1))
        .unwrap();

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(messages.drain_sent().len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(AddPoint(2))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.0, [0, 1, 2]);
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
        .replicate_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Points(vec![0])))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut points_query = client_app.world_mut().query::<&Points>();
    assert_eq!(points_query.iter(client_app.world()).len(), 1);

    for index in 1..=Points::HISTORY_LEN {
        server_app
            .world_mut()
            .entity_mut(server_entity)
            .apply_patch::<Points>(AddPoint(index as u8))
            .unwrap();
    }

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(messages.drain_sent().len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .apply_patch::<Points>(AddPoint(100))
        .unwrap();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.0, [0, 1, 2, 3, 4, 5, 100]);
    assert_eq!(points.0.len(), Points::HISTORY_LEN + 2);
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
        .replicate_diff::<Points>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Points(vec![0])))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut points_query = client_app.world_mut().query::<&Points>();
    assert_eq!(points_query.iter(client_app.world()).len(), 1);

    let mut points = server_app
        .world_mut()
        .get_mut::<Points>(server_entity)
        .unwrap();
    points.0.push(1);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let points = points_query.single(client_app.world()).unwrap();
    assert_eq!(points.0, [0, 1]);
}

#[derive(Component, Deserialize, Serialize, Debug, Clone)]
struct Points(Vec<u8>);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct AddPoint(u8);

impl Diffable for Points {
    type Patch = AddPoint;
    const HISTORY_LEN: usize = 5;

    fn apply_patch(&mut self, patch: &Self::Patch) -> Result<()> {
        self.0.push(patch.0);
        Ok(())
    }
}
