use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    prelude::*,
    shared::{
        replication::delta::{Cached, diffable::Diffable, rules::AppRuleExt},
        server_entity_map::ServerEntityMap,
    },
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};

#[test]
fn ensure_cached_exists() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<A>(RepliconTick::new(10))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let _ = server_app.world_mut().spawn((Replicated, A(0))).id();

    let _ = server_app
        .world_mut()
        .query::<&Cached<A>>()
        .single(server_app.world())
        .expect("There should be a cached value in the server!");

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let _ = client_app
        .world_mut()
        .query::<&Cached<A>>()
        .single(client_app.world())
        .expect("There should be a cached value in the client too!");
}

#[test]
fn cached_last_tick_updates_appropriately() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<A>(RepliconTick::new(2))
        .finish();
    }

    // helper lambda to quickly get last ticks of cached.
    let get_last_ticks = |server: &mut App, client: &mut App| -> (u32, u32) {
        let mut server_world = server.world_mut();
        let server_last_cached_tick = server_world
            .query::<&Cached<A>>()
            .single_mut(&mut server_world)
            .unwrap()
            .last_tick
            .get();

        let mut client_world = client.world_mut();
        let client_last_cached_tick = client_world
            .query::<&Cached<A>>()
            .single_mut(&mut client_world)
            .unwrap()
            .last_tick
            .get();

        (server_last_cached_tick, client_last_cached_tick)
    };

    server_app.connect_client(&mut client_app);

    let _ = server_app.world_mut().spawn((Replicated, A(0))).id();

    assert_eq!(get_last_ticks(&mut server_app, &mut client_app), (0, 0));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(get_last_ticks(&mut server_app, &mut client_app), (0, 0));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(
        get_last_ticks(&mut server_app, &mut client_app),
        (2, 2),
        "Last ticks should have advanced!"
    );
}

#[test]
fn regular_diff_test() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<A>(RepliconTick::new(10))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A(0))).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    // change value
    let mut component = server_app.world_mut().get_mut::<A>(server_entity).unwrap();
    component.0 = 1;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = client_app
        .world_mut()
        .query::<&A>()
        .single(client_app.world())
        .unwrap();
    assert_eq!(component.0, 1, "mutated value should be updated on client");
}

#[test]
fn component_removal_removes_cached() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_diff::<A>(RepliconTick::new(10))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A(0))).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    assert!(
        server_app
            .world_mut()
            .query::<&Cached<A>>()
            .single(server_app.world())
            .is_ok()
    );
    assert!(
        client_app
            .world_mut()
            .query::<&Cached<A>>()
            .single(client_app.world())
            .is_ok()
    );

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let server_cached_does_not_exist = server_app
        .world_mut()
        .query::<&Cached<A>>()
        .single(server_app.world())
        .is_err();
    assert!(server_cached_does_not_exist, "Cached should be removed");

    let client_cache_does_not_exist = client_app
        .world_mut()
        .query::<&Cached<A>>()
        .single(client_app.world())
        .is_err();
    assert!(client_cache_does_not_exist, "Cached should be removed");
}

// more tests could be included
// for cases such as
// - client gains visibility of an entity in between snapshots
// - check if cached doesnt pollute archetype tables?

#[derive(Component, Serialize, Deserialize)]
struct A(i32);

#[derive(Component, Serialize, Deserialize)]
struct ADiff(i32);

impl Diffable for A {
    type Diff = ADiff;

    fn diff(&self, target: &Self) -> Self::Diff {
        ADiff(target.0 - self.0)
    }

    fn apply(&mut self, diff: Self::Diff) {
        self.0 += diff.0;
    }
}
