use core::time::Duration;

use bevy::{
    platform::time::Instant, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy,
};
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};
use criterion::{Criterion, criterion_group, criterion_main};
use serde::{Deserialize, Serialize};

criterion_main!(benches);

criterion_group!(benches, replicate);

/// Entities replicated together in the changes and dense-mutation benchmarks.
const BATCH_ENTITY_COUNT: usize = 500;

/// Already-replicated entities scanned by the idle and sparse-mutation benchmarks.
const WORLD_ENTITY_COUNT: usize = 50_000;

/// Entities mutated per tick in the sparse benchmark (1% of the world).
const SPARSE_MUTATION_COUNT: usize = WORLD_ENTITY_COUNT / 100;

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
            |b| b.iter_custom(|iter| world_send(iter, clients_count, SPARSE_MUTATION_COUNT)),
        );
        c.bench_function(
            &format!("once_mutations_send/{clients_count}_clients"),
            |b| b.iter_custom(|iter| once_mutations_send(iter, clients_count)),
        );
    }

    c.bench_function("changes_receive", |b| b.iter_custom(changes_receive));
    c.bench_function("mutations_receive", |b| b.iter_custom(mutations_receive));
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
            .spawn_batch(vec![(Replicated, Position::default()); BATCH_ENTITY_COUNT]);

        let instant = Instant::now();
        server_app.update();
        elapsed += instant.elapsed();

        for client_app in &mut client_apps {
            server_app.exchange_with_client(client_app);
            client_app.update();

            let mut remote = client_app.world_mut().query::<&Remote>();
            assert_eq!(remote.iter(client_app.world()).len(), BATCH_ENTITY_COUNT);
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
        .spawn_batch(vec![(Replicated, Position::default()); BATCH_ENTITY_COUNT]);
    let mut query = server_app.world_mut().query::<&mut Position>();

    server_app.update();
    for client_app in &mut client_apps {
        server_app.exchange_with_client(client_app);
        client_app.update();

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), BATCH_ENTITY_COUNT);
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
            assert_eq!(remote.iter(client_app.world()).len(), BATCH_ENTITY_COUNT);
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
        .spawn_batch(vec![(Replicated, Position::default()); WORLD_ENTITY_COUNT]);
    let mut query = server_app.world_mut().query::<&mut Position>();

    server_app.update();
    for client_app in &mut client_apps {
        server_app.exchange_with_client(client_app);
        client_app.update();

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), WORLD_ENTITY_COUNT);
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

fn once_mutations_send(iter: u64, clients_count: usize) -> Duration {
    let mut server_app = create_app_with_mode(ReplicationMode::Once);
    let mut client_apps = Vec::new();
    for _ in 0..clients_count {
        client_apps.push(create_app_with_mode(ReplicationMode::Once));
    }

    for client_app in &mut client_apps {
        server_app.connect_client(client_app);
    }

    server_app
        .world_mut()
        .spawn_batch(vec![(Replicated, Position::default()); WORLD_ENTITY_COUNT]);
    let mut query = server_app.world_mut().query::<&mut Position>();

    server_app.update();
    for client_app in &mut client_apps {
        server_app.exchange_with_client(client_app);
        client_app.update();
        server_app.exchange_with_client(client_app);

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), WORLD_ENTITY_COUNT);
    }

    // A `Once` mutation remains newer than the client's tick even on later idle ticks.
    for mut component in query.iter_mut(server_app.world_mut()) {
        component.x += 1.0;
    }

    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        let instant = Instant::now();
        server_app.update();
        elapsed += instant.elapsed();

        for client_app in &mut client_apps {
            server_app.exchange_with_client(client_app);
            client_app.update();
        }
    }

    for client_app in &mut client_apps {
        let mut positions = client_app.world_mut().query::<&Position>();
        assert!(
            positions
                .iter(client_app.world())
                .all(|position| position.x == 1.0)
        );
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
            .spawn_batch(vec![(Replicated, Position::default()); BATCH_ENTITY_COUNT]);

        server_app.update();
        server_app.exchange_with_client(&mut client_app);

        let instant = Instant::now();
        client_app.update();
        elapsed += instant.elapsed();

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), BATCH_ENTITY_COUNT);
    }

    elapsed
}

fn mutations_receive(iter: u64) -> Duration {
    let mut server_app = create_app();
    let mut client_app = create_app();

    server_app.connect_client(&mut client_app);

    server_app
        .world_mut()
        .spawn_batch(vec![(Replicated, Position::default()); BATCH_ENTITY_COUNT]);
    let mut query = server_app.world_mut().query::<&mut Position>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), BATCH_ENTITY_COUNT);

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
        assert_eq!(remote.iter(client_app.world()).len(), BATCH_ENTITY_COUNT);
    }

    elapsed
}

fn create_app() -> App {
    create_app_with_mode(ReplicationMode::OnChange)
}

fn create_app_with_mode(mode: ReplicationMode) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
    .replicate_with((RuleFns::<Position>::default(), mode))
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
