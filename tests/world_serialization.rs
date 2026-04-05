use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{prelude::*, world_serialization};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
#[should_panic] // Fails because https://github.com/bevyengine/bevy/pull/23446 accidentally enabled reflect auto registration.
fn replicated() {
    let mut app = App::new();
    app.add_plugins((StatesPlugin, RepliconPlugins))
        .register_type::<TestComponent>()
        .register_type::<NonReflectedComponent>()
        .replicate::<TestComponent>()
        .replicate::<ReflectedComponent>() // Reflected, but the type is not registered.
        .replicate::<NonReflectedComponent>()
        .finish();

    let replicated = app
        .world_mut()
        .spawn((
            Replicated,
            TestComponent,
            ReflectedComponent,
            NonReflectedComponent,
        ))
        .id();
    let remote = app.world_mut().spawn((Remote, TestComponent)).id();

    let mut dyn_world = DynamicWorld::default();
    world_serialization::replicate_into(&mut dyn_world, app.world());

    let replicated_dyn = dyn_world
        .entities
        .iter()
        .find(|entity| entity.entity == replicated)
        .unwrap();

    let remote_dyn = dyn_world
        .entities
        .iter()
        .find(|entity| entity.entity == remote)
        .unwrap();

    assert_eq!(replicated_dyn.entity, replicated);
    assert_eq!(
        replicated_dyn.components.len(),
        1,
        "entity should have only registered components with `#[reflect(Component)]`"
    );

    assert_eq!(remote_dyn.entity, remote);
    assert_eq!(remote_dyn.components.len(), 1);
}

#[test]
fn empty() {
    let mut app = App::new();
    app.add_plugins((StatesPlugin, RepliconPlugins)).finish();

    let entity = app.world_mut().spawn(Replicated).id();

    // Extend with replicated components.
    let mut dyn_world = DynamicWorld::default();
    world_serialization::replicate_into(&mut dyn_world, app.world());

    assert!(dyn_world.resources.is_empty());
    assert_eq!(dyn_world.entities.len(), 1);

    let dyn_entity = &dyn_world.entities[0];
    assert_eq!(dyn_entity.entity, entity);
    assert!(dyn_entity.components.is_empty());
}

#[test]
fn not_replicated() {
    let mut app = App::new();
    app.add_plugins((StatesPlugin, RepliconPlugins))
        .register_type::<TestComponent>()
        .replicate::<TestComponent>()
        .finish();

    app.world_mut().spawn(TestComponent);

    let mut dyn_world = DynamicWorld::default();
    world_serialization::replicate_into(&mut dyn_world, app.world());

    assert!(dyn_world.resources.is_empty());
    assert!(dyn_world.entities.is_empty());
}

#[test]
fn update_existing() {
    let mut app = App::new();
    app.add_plugins((StatesPlugin, RepliconPlugins))
        .register_type::<TestComponent>()
        .replicate::<TestComponent>()
        .register_type::<ReflectedComponent>()
        .finish();

    let entity = app
        .world_mut()
        .spawn((Replicated, TestComponent, ReflectedComponent))
        .id();

    // Populate scene only with a single non-replicated component.
    let registry = app.world().resource::<AppTypeRegistry>().read();
    let mut dyn_world = DynamicWorldBuilder::from_world(app.world(), &registry)
        .allow_component::<ReflectedComponent>()
        .extract_entity(entity)
        .build();

    // Update already extracted entity with replicated components.
    world_serialization::replicate_into(&mut dyn_world, app.world());

    assert!(dyn_world.resources.is_empty());
    assert_eq!(dyn_world.entities.len(), 1);

    let dyn_entity = &dyn_world.entities[0];
    assert_eq!(dyn_entity.entity, entity);
    assert_eq!(dyn_entity.components.len(), 2);
}

#[derive(Component, Default, Deserialize, Reflect, Serialize)]
#[reflect(Component)]
struct TestComponent;

#[derive(Component, Default, Deserialize, Reflect, Serialize)]
#[reflect(Component)]
struct ReflectedComponent;

/// Component that have `Reflect` derive, but without `#[reflect(Component)]`
#[derive(Component, Default, Deserialize, Reflect, Serialize)]
struct NonReflectedComponent;
