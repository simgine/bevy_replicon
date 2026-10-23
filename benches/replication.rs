use core::time::Duration;

use bevy::{
    platform::time::Instant, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy,
};
use bevy_replicon::{
    prelude::*,
    server::visibility::{client_visibility::ClientVisibility, registry::FilterRegistry},
    shared::replication::registry::ReplicationRegistry,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use criterion::{Criterion, criterion_group, criterion_main};
use serde::{Deserialize, Serialize};

criterion_main!(benches);

criterion_group!(benches, replicate);

/// Entities replicated together in the changes and dense-mutation benchmarks.
const BATCH_ENTITY_COUNT: usize = 500;

/// Already-replicated entities scanned by the idle, sparse, and mixed-archetype benchmarks.
const WORLD_ENTITY_COUNT: usize = 50_000;

/// Entities mutated per tick in the sparse and mixed-archetype benchmarks (1% of the world).
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
            &format!("mixed_archetypes_send/{clients_count}_clients"),
            |b| b.iter_custom(|iter| mixed_archetypes_send(iter, clients_count)),
        );
        c.bench_function(
            &format!("mixed_structural_send/{clients_count}_clients"),
            |b| b.iter_custom(|iter| mixed_churn_send(iter, clients_count, Churn::Structural)),
        );
        c.bench_function(
            &format!("mixed_visibility_send/{clients_count}_clients"),
            |b| b.iter_custom(|iter| mixed_churn_send(iter, clients_count, Churn::Visibility)),
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

/// Same mutation count as the sparse benchmark, but isolates the active entities
/// so the large static archetype can take the clean-archetype fast path.
fn mixed_archetypes_send(iter: u64, clients_count: usize) -> Duration {
    let mut server_app = create_app();
    let mut client_apps = Vec::new();
    for _ in 0..clients_count {
        client_apps.push(create_app());
    }

    for client_app in &mut client_apps {
        server_app.connect_client(client_app);
    }

    server_app.world_mut().spawn_batch(vec![
        (Replicated, Position::default());
        WORLD_ENTITY_COUNT - SPARSE_MUTATION_COUNT
    ]);
    server_app.world_mut().spawn_batch(vec![
        (Replicated, Position::default(), Dynamic);
        SPARSE_MUTATION_COUNT
    ]);
    let mut query = server_app
        .world_mut()
        .query_filtered::<&mut Position, With<Dynamic>>();
    assert_eq!(query.iter(server_app.world()).len(), SPARSE_MUTATION_COUNT);

    server_app.update();
    for client_app in &mut client_apps {
        server_app.exchange_with_client(client_app);
        client_app.update();

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), WORLD_ENTITY_COUNT);
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
        }
    }

    elapsed
}

/// The mixed-archetype benchmark plus one static entity per tick that either moves
/// between archetypes or swaps visibility for the first client.
fn mixed_churn_send(iter: u64, clients_count: usize, churn: Churn) -> Duration {
    let mut server_app = create_app();
    let mut client_apps = Vec::new();
    for _ in 0..clients_count {
        client_apps.push(create_app());
    }

    let bit = server_app
        .world_mut()
        .resource_scope(|world, mut filters: Mut<FilterRegistry>| {
            world.resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                filters.register_scope::<Entity>(world, &mut registry, ScopeLifetime::WhileVisible)
            })
        });

    for client_app in &mut client_apps {
        server_app.connect_client(client_app);
    }

    let statics: Vec<Entity> = server_app
        .world_mut()
        .spawn_batch(vec![
            (Replicated, Position::default());
            WORLD_ENTITY_COUNT - SPARSE_MUTATION_COUNT
        ])
        .collect();
    server_app.world_mut().spawn_batch(vec![
        (Replicated, Position::default(), Dynamic);
        SPARSE_MUTATION_COUNT
    ]);
    let mut query = server_app
        .world_mut()
        .query_filtered::<&mut Position, With<Dynamic>>();

    server_app.update();
    for client_app in &mut client_apps {
        server_app.exchange_with_client(client_app);
        client_app.update();
    }

    let first_client = **client_apps[0].world().resource::<TestClientEntity>();
    let mut elapsed = Duration::ZERO;
    for tick in 0..iter as usize {
        for mut component in query.iter_mut(server_app.world_mut()) {
            component.x += 1.0;
        }

        let entity = statics[tick % 2];
        let flip = tick % 4 < 2;
        match churn {
            Churn::Structural => {
                let mut entity = server_app.world_mut().entity_mut(entity);
                if flip {
                    entity.insert(Tag);
                } else {
                    entity.remove::<Tag>();
                }
            }
            Churn::Visibility => {
                let mut visibility = server_app
                    .world_mut()
                    .get_mut::<ClientVisibility>(first_client)
                    .unwrap();
                visibility.set(entity, bit, !flip);
            }
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

#[derive(Clone, Copy)]
enum Churn {
    Structural,
    Visibility,
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

#[derive(Component, Clone)]
struct Dynamic;

#[derive(Component)]
struct Tag;

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
