use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{prelude::*, scene};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn replicated_entity() {
    let mut app = App::new();
    app.add_plugins((StatesPlugin, RepliconPlugins))
        .register_type::<TestComponent>()
        .register_type::<NonReflectedComponent>()
        .replicate::<TestComponent>()
        .replicate::<ReflectedComponent>() // Reflected, but the type is not registered.
        .replicate::<NonReflectedComponent>()
        .finish();

    let entity = app
        .world_mut()
        .spawn((
            Replicated,
            TestComponent,
            ReflectedComponent,
            NonReflectedComponent,
        ))
        .id();

    let mut scene = DynamicScene::default();
    scene::replicate_into(&mut scene, app.world());

    assert!(scene.resources.is_empty());
    assert_eq!(scene.entities.len(), 1);

    let dyn_entity = &scene.entities[0];
    assert_eq!(dyn_entity.entity, entity);
    assert_eq!(
        dyn_entity.components.len(),
        1,
        "entity should have only registered components with `#[reflect(Component)]`"
    );
}

#[test]
fn empty_entity() {
    let mut app = App::new();
    app.add_plugins((StatesPlugin, RepliconPlugins)).finish();

    let entity = app.world_mut().spawn(Replicated).id();

    // Extend with replicated components.
    let mut scene = DynamicScene::default();
    scene::replicate_into(&mut scene, app.world());

    assert!(scene.resources.is_empty());
    assert_eq!(scene.entities.len(), 1);

    let dyn_entity = &scene.entities[0];
    assert_eq!(dyn_entity.entity, entity);
    assert!(dyn_entity.components.is_empty());
}

#[test]
fn not_replicated_entity() {
    let mut app = App::new();
    app.add_plugins((StatesPlugin, RepliconPlugins))
        .register_type::<TestComponent>()
        .replicate::<TestComponent>()
        .finish();

    app.world_mut().spawn(TestComponent);

    let mut scene = DynamicScene::default();
    scene::replicate_into(&mut scene, app.world());

    assert!(scene.resources.is_empty());
    assert!(scene.entities.is_empty());
}

#[test]
fn entity_update() {
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
    let mut scene = DynamicSceneBuilder::from_world(app.world())
        .allow_component::<ReflectedComponent>()
        .extract_entity(entity)
        .build();

    // Update already extracted entity with replicated components.
    scene::replicate_into(&mut scene, app.world());

    assert!(scene.resources.is_empty());
    assert_eq!(scene.entities.len(), 1);

    let dyn_entity = &scene.entities[0];
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
