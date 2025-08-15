use bevy::prelude::*;
use bevy_replicon::prelude::*;
use criterion::{Criterion, criterion_group, criterion_main};

criterion_main!(benches);

criterion_group!(benches, hierarchy_spawning, hierarchy_changes);

fn hierarchy_spawning(c: &mut Criterion) {
    let mut group = c.benchmark_group("hierarchy_spawning");

    group.bench_function("regular", |b| {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();

        b.iter(|| spawn_then_despawn(&mut app));
    });
    group.bench_function("related_without_server", |b| {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, RepliconPlugins))
            .sync_related_entities::<ChildOf>()
            .finish();

        b.iter(|| spawn_then_despawn(&mut app));
    });
    group.bench_function("related", |b| {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, RepliconPlugins))
            .sync_related_entities::<ChildOf>()
            .finish();

        let mut server = app.world_mut().resource_mut::<RepliconServer>();
        server.set_running(true);

        b.iter(|| spawn_then_despawn(&mut app));
    });
}

fn spawn_then_despawn(app: &mut App) {
    for entity in spawn_hierarchy(app.world_mut()) {
        app.world_mut().despawn(entity);
    }
}

fn hierarchy_changes(c: &mut Criterion) {
    let mut group = c.benchmark_group("hierarchy_changes");

    group.bench_function("regular", |b| {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, RepliconPlugins)).finish();

        spawn_hierarchy(app.world_mut());

        b.iter(|| trigger_hierarchy_change(&mut app));
    });
    group.bench_function("related_without_server", |b| {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, RepliconPlugins))
            .sync_related_entities::<ChildOf>()
            .finish();

        spawn_hierarchy(app.world_mut());

        b.iter(|| trigger_hierarchy_change(&mut app));
    });
    group.bench_function("related", |b| {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, RepliconPlugins))
            .sync_related_entities::<ChildOf>()
            .finish();

        spawn_hierarchy(app.world_mut());

        let mut server = app.world_mut().resource_mut::<RepliconServer>();
        server.set_running(true);

        b.iter(|| trigger_hierarchy_change(&mut app));
    });
}

// Spawn and despawn a small hierarchy to trigger graphs rebuild.
fn trigger_hierarchy_change(app: &mut App) {
    app.world_mut()
        .spawn((Replicated, children![Replicated]))
        .despawn();
    app.update();
}

fn spawn_hierarchy(world: &mut World) -> Vec<Entity> {
    let mut roots = Vec::new();
    roots.extend(world.spawn_batch([Replicated; 500]));
    roots.push(
        world
            .spawn((Replicated, Children::spawn(vec![Replicated; 500])))
            .id(),
    );
    for _ in 0..500 {
        roots.push(world.spawn((Replicated, children![Replicated])).id());
    }

    roots
}
