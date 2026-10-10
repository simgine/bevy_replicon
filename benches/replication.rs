use core::time::Duration;

use bevy::{
    platform::time::Instant, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy,
};
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};
use criterion::{Criterion, criterion_group, criterion_main};
use serde::{Deserialize, Serialize};

criterion_main!(benches);

criterion_group!(benches, replicate);

const ENTITIES: usize = 1000;
/// Large enough for a pass over a world the clients already hold to take milliseconds.
const WORLD_ENTITIES: usize = 50_000;
/// Share of the world mutated per tick in the sparse case.
const SPARSE_MUTATIONS: usize = WORLD_ENTITIES / 100;

fn replicate(c: &mut Criterion) {
    for clients_count in [1, 20] {
        c.bench_function(&format!("changes_send/{clients_count}_clients"), |b| {
            b.iter_custom(|iter| changes_send(iter, clients_count))
        });
        c.bench_function(&format!("mutations_send/{clients_count}_clients"), |b| {
            b.iter_custom(|iter| mutations_send(iter, clients_count))
        });
        c.bench_function(&format!("idle_send/{clients_count}_clients"), |b| {
            b.iter_custom(|iter| world_send(iter, clients_count, 0))
        });
        c.bench_function(
            &format!("sparse_mutations_send/{clients_count}_clients"),
            |b| b.iter_custom(|iter| world_send(iter, clients_count, SPARSE_MUTATIONS)),
        );
    }

    c.bench_function("changes_receive", |b| {
        b.iter_custom(|iter| changes_receive(iter))
    });
    c.bench_function("mutations_receive", |b| {
        b.iter_custom(|iter| mutations_receive(iter))
    });
}

fn changes_send(iter: u64, clients_count: usize) -> Duration {
    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        let mut server_app = create_app();
        let mut client_apps = Vec::new();
        for _ in 0..clients_count {
            client_apps.push(create_app());
        }

        for client_app in &mut client_apps {
            server_app.connect_client(client_app);
        }

        server_app
            .world_mut()
            .spawn_batch(vec![(Replicated, Position::default()); ENTITIES]);

        let instant = Instant::now();
        server_app.update();
        elapsed += instant.elapsed();

        for client_app in &mut client_apps {
            server_app.exchange_with_client(client_app);
            client_app.update();

            let mut remote = client_app.world_mut().query::<&Remote>();
            assert_eq!(remote.iter(client_app.world()).len(), ENTITIES);
        }
    }

    elapsed
}

fn mutations_send(iter: u64, clients_count: usize) -> Duration {
    let mut server_app = create_app();
    let mut client_apps = Vec::new();
    for _ in 0..clients_count {
        client_apps.push(create_app());
    }

    for client_app in &mut client_apps {
        server_app.connect_client(client_app);
    }

    server_app
        .world_mut()
        .spawn_batch(vec![(Replicated, Position::default()); ENTITIES]);
    let mut query = server_app.world_mut().query::<&mut Position>();

    server_app.update();
    for client_app in &mut client_apps {
        server_app.exchange_with_client(client_app);
        client_app.update();

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), ENTITIES);
    }

    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        for mut component in query.iter_mut(server_app.world_mut()) {
            component.x += 1.0;
        }

        let instant = Instant::now();
        server_app.update();
        elapsed += instant.elapsed();

        for client_app in &mut client_apps {
            server_app.exchange_with_client(client_app);
            client_app.update();

            let mut remote = client_app.world_mut().query::<&Remote>();
            assert_eq!(remote.iter(client_app.world()).len(), ENTITIES);
        }
    }

    elapsed
}

fn world_send(iter: u64, clients_count: usize, mutated: usize) -> Duration {
    let mut server_app = create_app();
    let mut client_apps = Vec::new();
    for _ in 0..clients_count {
        client_apps.push(create_app());
    }

    for client_app in &mut client_apps {
        server_app.connect_client(client_app);
    }

    server_app
        .world_mut()
        .spawn_batch(vec![(Replicated, Position::default()); WORLD_ENTITIES]);
    let mut query = server_app.world_mut().query::<&mut Position>();

    server_app.update();
    for client_app in &mut client_apps {
        server_app.exchange_with_client(client_app);
        client_app.update();

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), WORLD_ENTITIES);
    }

    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        for mut component in query.iter_mut(server_app.world_mut()).take(mutated) {
            component.x += 1.0;
        }

        let instant = Instant::now();
        server_app.update();
        elapsed += instant.elapsed();

        for client_app in &mut client_apps {
            server_app.exchange_with_client(client_app);
            client_app.update();
        }
    }

    elapsed
}

fn changes_receive(iter: u64) -> Duration {
    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        let mut server_app = create_app();
        let mut client_app = create_app();

        server_app.connect_client(&mut client_app);

        server_app
            .world_mut()
            .spawn_batch(vec![(Replicated, Position::default()); ENTITIES]);

        server_app.update();
        server_app.exchange_with_client(&mut client_app);

        let instant = Instant::now();
        client_app.update();
        elapsed += instant.elapsed();

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), ENTITIES);
    }

    elapsed
}

fn mutations_receive(iter: u64) -> Duration {
    let mut server_app = create_app();
    let mut client_app = create_app();

    server_app.connect_client(&mut client_app);

    server_app
        .world_mut()
        .spawn_batch(vec![(Replicated, Position::default()); ENTITIES]);
    let mut query = server_app.world_mut().query::<&mut Position>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), ENTITIES);

    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        for mut component in query.iter_mut(server_app.world_mut()) {
            component.x += 1.0;
        }

        server_app.update();
        server_app.exchange_with_client(&mut client_app);

        let instant = Instant::now();

        client_app.update();
        elapsed += instant.elapsed();

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), ENTITIES);
    }

    elapsed
}

fn create_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
    .replicate::<Position>()
    .finish();

    app
}

#[derive(Component, Serialize, Deserialize, Clone)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        }
    }
}
