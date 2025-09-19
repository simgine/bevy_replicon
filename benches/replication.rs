use core::time::Duration;

use bevy::{
    ecs::component::Mutable, platform::time::Instant, prelude::*, state::app::StatesPlugin,
};
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

criterion_main!(benches);

criterion_group!(
    benches,
    replicate::<UsizeComponent>,
    replicate::<StringComponent>,
    replicate::<StructComponent>
);

const ENTITIES: usize = 1000;

fn replicate<C: BenchmarkComponent>(c: &mut Criterion) {
    let mut g = c.benchmark_group(C::NAME);

    for clients_count in [1, 20] {
        g.bench_function(BenchmarkId::new("changes_send", clients_count), |b| {
            b.iter_custom(|iter| changes_send::<C>(iter, clients_count))
        });
        g.bench_function(BenchmarkId::new("mutations_send", clients_count), |b| {
            b.iter_custom(|iter| mutations_send::<C>(iter, clients_count))
        });
    }

    g.bench_function("changes_receive", |b| {
        b.iter_custom(|iter| changes_receive::<C>(iter))
    });
    g.bench_function("mutations_receive", |b| {
        b.iter_custom(|iter| mutations_receive::<C>(iter))
    });
}

fn changes_send<C: BenchmarkComponent>(iter: u64, clients_count: usize) -> Duration {
    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        let mut server_app = create_app::<C>();
        let mut client_apps = Vec::new();
        for _ in 0..clients_count {
            client_apps.push(create_app::<C>());
        }

        for client_app in &mut client_apps {
            server_app.connect_client(client_app);
        }

        server_app
            .world_mut()
            .spawn_batch(vec![(Replicated, C::default()); ENTITIES]);

        let instant = Instant::now();
        server_app.update();
        elapsed += instant.elapsed();

        for client_app in &mut client_apps {
            server_app.exchange_with_client(client_app);
            client_app.update();

            let mut replicated = client_app.world_mut().query::<&Replicated>();
            assert_eq!(replicated.iter(client_app.world()).count(), ENTITIES);
        }
    }

    elapsed
}

fn mutations_send<C: BenchmarkComponent>(iter: u64, clients_count: usize) -> Duration {
    let mut server_app = create_app::<C>();
    let mut client_apps = Vec::new();
    for _ in 0..clients_count {
        client_apps.push(create_app::<C>());
    }

    for client_app in &mut client_apps {
        server_app.connect_client(client_app);
    }

    server_app
        .world_mut()
        .spawn_batch(vec![(Replicated, C::default()); ENTITIES]);
    let mut query = server_app.world_mut().query::<&mut C>();

    server_app.update();
    for client_app in &mut client_apps {
        server_app.exchange_with_client(client_app);
        client_app.update();

        let mut replicated = client_app.world_mut().query::<&Replicated>();
        assert_eq!(replicated.iter(client_app.world()).count(), ENTITIES);
    }

    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        for mut component in query.iter_mut(server_app.world_mut()) {
            component.set_changed();
        }

        let instant = Instant::now();
        server_app.update();
        elapsed += instant.elapsed();

        for client_app in &mut client_apps {
            server_app.exchange_with_client(client_app);
            client_app.update();

            let mut replicated = client_app.world_mut().query::<&Replicated>();
            assert_eq!(replicated.iter(client_app.world()).count(), ENTITIES);
        }
    }

    elapsed
}

fn changes_receive<C: BenchmarkComponent>(iter: u64) -> Duration {
    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        let mut server_app = create_app::<C>();
        let mut client_app = create_app::<C>();

        server_app.connect_client(&mut client_app);

        server_app
            .world_mut()
            .spawn_batch(vec![(Replicated, C::default()); ENTITIES]);

        server_app.update();
        server_app.exchange_with_client(&mut client_app);

        let instant = Instant::now();
        client_app.update();
        elapsed += instant.elapsed();

        let mut replicated = client_app.world_mut().query::<&Replicated>();
        assert_eq!(replicated.iter(client_app.world()).count(), ENTITIES);
    }

    elapsed
}

fn mutations_receive<C: BenchmarkComponent>(iter: u64) -> Duration {
    let mut server_app = create_app::<C>();
    let mut client_app = create_app::<C>();

    server_app.connect_client(&mut client_app);

    server_app
        .world_mut()
        .spawn_batch(vec![(Replicated, C::default()); ENTITIES]);
    let mut query = server_app.world_mut().query::<&mut C>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    let mut replicated = client_app.world_mut().query::<&Replicated>();
    assert_eq!(replicated.iter(client_app.world()).count(), ENTITIES);

    let mut elapsed = Duration::ZERO;
    for _ in 0..iter {
        for mut component in query.iter_mut(server_app.world_mut()) {
            component.set_changed();
        }

        server_app.update();
        server_app.exchange_with_client(&mut client_app);

        let instant = Instant::now();

        client_app.update();
        elapsed += instant.elapsed();

        let mut replicated = client_app.world_mut().query::<&Replicated>();
        assert_eq!(replicated.iter(client_app.world()).count(), ENTITIES);
    }

    elapsed
}

fn create_app<C: BenchmarkComponent>() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ))
    .replicate::<C>()
    .finish();

    app
}

trait BenchmarkComponent:
    Component<Mutability = Mutable> + Default + Serialize + DeserializeOwned + Clone
{
    const NAME: &'static str;
}

#[derive(Clone, Component, Default, Deserialize, Serialize)]
struct UsizeComponent(usize);

impl BenchmarkComponent for UsizeComponent {
    const NAME: &'static str = "usize_component";
}

#[derive(Component, Clone, Serialize, Deserialize)]
struct StringComponent(String);

impl BenchmarkComponent for StringComponent {
    const NAME: &'static str = "string_component";
}

impl Default for StringComponent {
    fn default() -> Self {
        Self(".".repeat(60))
    }
}

impl BenchmarkComponent for StructComponent {
    const NAME: &'static str = "struct_component";
}

#[derive(Component, Clone, Serialize, Deserialize)]
struct StructComponent {
    x: u32,
    y: u32,
    b: f32,
    a: f32,
    n: String,
}

impl Default for StructComponent {
    fn default() -> Self {
        Self {
            x: 22u32,
            y: 22u32,
            b: 1.5f32,
            a: 20.0f32,
            n: String::from("abcdef123"),
        }
    }
}
