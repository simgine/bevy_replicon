use bevy::{ecs::entity_disabling::Disabled, prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    prelude::*,
    server::visibility::{client_visibility::ClientVisibility, registry::FilterRegistry},
    shared::{replication::registry::ReplicationRegistry, server_entity_map::ServerEntityMap},
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn new_client() {
    let mut server_app = App::new();
    let mut first_client_app = App::new();
    let mut second_client_app = App::new();
    for app in [
        &mut server_app,
        &mut first_client_app,
        &mut second_client_app,
    ] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut first_client_app);

    server_app.world_mut().spawn((Replicated, A(7)));
    server_app.world_mut().spawn(Replicated);

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut first_client_app);
        first_client_app.update();
        server_app.exchange_with_client(&mut first_client_app);
    }

    server_app.connect_client(&mut second_client_app);

    server_app.update();
    server_app.exchange_with_client(&mut second_client_app);
    second_client_app.update();

    for client_app in [&mut first_client_app, &mut second_client_app] {
        let mut components = client_app.world_mut().query::<&A>();
        let component = components.single(client_app.world()).unwrap();
        assert_eq!(component.0, 7);

        let mut remote = client_app.world_mut().query::<&Remote>();
        assert_eq!(remote.iter(client_app.world()).len(), 2);
    }
}

#[test]
fn rule_filters() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_filtered::<A, With<RuleMarker>>()
        .replicate_filtered::<B, Without<RuleMarker>>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Warm both archetypes before moving entities between them.
    let with_marker = server_app
        .world_mut()
        .spawn((Replicated, A(7), B(11), RuleMarker))
        .id();
    let without_marker = server_app
        .world_mut()
        .spawn((Replicated, A(13), B(17)))
        .id();

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    // Only the filters change; the replicated components keep their old ticks.
    server_app
        .world_mut()
        .entity_mut(with_marker)
        .remove::<RuleMarker>();
    server_app
        .world_mut()
        .entity_mut(without_marker)
        .insert(RuleMarker);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    let with_marker = *entity_map.to_client().get(&with_marker).unwrap();
    let without_marker = *entity_map.to_client().get(&without_marker).unwrap();
    assert_eq!(client_app.world().get::<B>(with_marker).unwrap().0, 11);
    assert_eq!(client_app.world().get::<A>(without_marker).unwrap().0, 13);
}

#[test]
fn bundle_completion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate_bundle::<(A, B)>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Create this archetype before the idle ticks so inserting B later moves
    // the other entity into an existing, clean archetype.
    server_app.world_mut().spawn((Replicated, A(1), B(2)));
    let server_entity = server_app.world_mut().spawn((Replicated, A(7))).id();

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(B(11));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    let client_entity = *entity_map.to_client().get(&server_entity).unwrap();
    let entity = client_app.world().entity(client_entity);
    assert_eq!(entity.get::<A>().unwrap().0, 7);
    assert_eq!(entity.get::<B>().unwrap().0, 11);
}

#[test]
fn entity_visibility_gain() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .add_visibility_filter::<EntityVisibility>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert(EntityVisibility);
    server_app
        .world_mut()
        .spawn((Replicated, A(7), EntityVisibility));

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    server_app
        .world_mut()
        .entity_mut(client)
        .remove::<EntityVisibility>();

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 0);

    server_app
        .world_mut()
        .entity_mut(client)
        .insert(EntityVisibility);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Remote, &A)>();
    let (_, component) = components.single(client_app.world()).unwrap();
    assert_eq!(component.0, 7);
}

#[test]
fn component_visibility_gain() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .replicate::<B>()
        .add_visibility_filter::<ComponentVisibility>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert(ComponentVisibility);
    server_app
        .world_mut()
        .spawn((Replicated, A(7), B(11), ComponentVisibility));

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    server_app
        .world_mut()
        .entity_mut(client)
        .remove::<ComponentVisibility>();

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut components = client_app.world_mut().query::<(Option<&A>, &B)>();
    let (a, b) = components.single(client_app.world()).unwrap();
    assert!(a.is_none());
    assert_eq!(b.0, 11);

    server_app
        .world_mut()
        .entity_mut(client)
        .insert(ComponentVisibility);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&A, &B)>();
    let (a, b) = components.single(client_app.world()).unwrap();
    assert_eq!(a.0, 7);
    assert_eq!(b.0, 11);
}

#[test]
fn manual_visibility_gain() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    let bit =
        server_app
            .world_mut()
            .resource_scope(|world, mut filter_registry: Mut<FilterRegistry>| {
                world.resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                    filter_registry.register_scope::<Entity>(
                        world,
                        &mut registry,
                        ScopeLifetime::WhileVisible,
                    )
                })
            });

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    let server_entity = server_app.world_mut().spawn((Replicated, A(7))).id();
    server_app
        .world_mut()
        .get_mut::<ClientVisibility>(client)
        .unwrap()
        .set(server_entity, bit, false);

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 0);

    server_app
        .world_mut()
        .get_mut::<ClientVisibility>(client)
        .unwrap()
        .set(server_entity, bit, true);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Remote, &A)>();
    let (_, component) = components.single(client_app.world()).unwrap();
    assert_eq!(component.0, 7);
}

#[test]
fn deferred_mutation() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, A(7), ReplicatePriority(0.1)))
        .id();

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut component = server_app.world_mut().get_mut::<A>(server_entity).unwrap();
    component.0 = 11;

    let mut components = client_app.world_mut().query::<&A>();
    for _ in 0..2 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let component = components.single(client_app.world()).unwrap();
        assert_eq!(component.0, 7, "mutation should be deprioritized");
    }

    // Priority must accumulate even without further changes.
    for _ in 0..10 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let component = components.single(client_app.world()).unwrap();
    assert_eq!(component.0, 11);
}

#[test]
fn missed_mutation_and_ack() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A(7))).id();

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut component = server_app.world_mut().get_mut::<A>(server_entity).unwrap();
    component.0 = 11;

    let mut components = client_app.world_mut().query::<Ref<A>>();
    for _ in 0..3 {
        server_app.update();

        // Drop the mutation, including retries on unchanged ticks.
        let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
        assert_eq!(messages.drain_sent().len(), 1);

        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);

        let component = components.single(client_app.world()).unwrap();
        assert_eq!(component.0, 7);
    }

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = components.single(client_app.world()).unwrap();
    assert_eq!(component.0, 11);
    let received_tick = component.last_changed();

    // Drop the acknowledgment.
    let mut messages = client_app.world_mut().resource_mut::<ClientMessages>();
    assert_eq!(messages.drain_sent().count(), 1);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let component = components.single(client_app.world()).unwrap();
    let retry_tick = component.last_changed();
    assert_ne!(received_tick, retry_tick, "mutation should be resent");

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let component = components.single(client_app.world()).unwrap();
    assert_eq!(component.0, 11);
    assert_eq!(
        component.last_changed(),
        retry_tick,
        "acknowledged mutations should stop being resent"
    );
}

#[test]
fn pending_mutation_with_insertion() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A(7))).id();
    // Warm the destination archetype before the mutation.
    server_app.world_mut().spawn((Replicated, A(1), Unrelated));

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut component = server_app.world_mut().get_mut::<A>(server_entity).unwrap();
    component.0 = 11;

    server_app.update();

    // Drop the mutation before moving the entity into a clean archetype.
    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(messages.drain_sent().len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Unrelated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    let client_entity = *entity_map.to_client().get(&server_entity).unwrap();
    assert_eq!(client_app.world().get::<A>(client_entity).unwrap().0, 11);
}

#[test]
fn pending_mutation_with_removal() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, A(7), Unrelated))
        .id();
    // Warm the destination archetype before the mutation.
    server_app.world_mut().spawn((Replicated, A(1)));

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut component = server_app.world_mut().get_mut::<A>(server_entity).unwrap();
    component.0 = 11;

    server_app.update();

    // Drop the mutation before moving the entity into a clean archetype.
    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(messages.drain_sent().len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<Unrelated>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    let client_entity = *entity_map.to_client().get(&server_entity).unwrap();
    assert_eq!(client_app.world().get::<A>(client_entity).unwrap().0, 11);
}

#[test]
fn old_component() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn((Replicated, A(1)));
    let server_entity = server_app.world_mut().spawn(A(7)).id();

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(remote.iter(client_app.world()).len(), 2);

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    let client_entity = *entity_map.to_client().get(&server_entity).unwrap();
    assert_eq!(client_app.world().get::<A>(client_entity).unwrap().0, 7);
}

#[test]
fn empty() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn(Replicated);

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 1);

    server_app.world_mut().spawn(Replicated);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(remote.iter(client_app.world()).len(), 2);
}

#[test]
fn sparse_set_storage() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .replicate::<SparseSet>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, A(1), SparseSet(7)))
        .id();

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut component = server_app
        .world_mut()
        .get_mut::<SparseSet>(server_entity)
        .unwrap();
    component.0 = 11;

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&A, &SparseSet)>();
    let (a, sparse_set) = components.single(client_app.world()).unwrap();
    assert_eq!(a.0, 1);
    assert_eq!(sparse_set.0, 11);
}

#[test]
fn reenabled_client() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A(7))).id();

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app.world_mut().entity_mut(client).insert(Disabled);

    let mut component = server_app.world_mut().get_mut::<A>(server_entity).unwrap();
    component.0 = 11;

    for _ in 0..4 {
        server_app.update();
        server_app.exchange_with_client(&mut client_app);
        client_app.update();
        server_app.exchange_with_client(&mut client_app);
    }

    let mut components = client_app.world_mut().query::<&A>();
    let component = components.single(client_app.world()).unwrap();
    assert_eq!(
        component.0, 7,
        "disabled clients should not receive mutations"
    );

    server_app
        .world_mut()
        .entity_mut(client)
        .remove::<Disabled>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = components.single(client_app.world()).unwrap();
    assert_eq!(component.0, 11);
}

#[derive(Component, Deserialize, Serialize)]
struct A(u32);

#[derive(Component, Deserialize, Serialize)]
struct B(u32);

#[derive(Component)]
struct RuleMarker;

#[derive(Component)]
struct Unrelated;

#[derive(Component, Deserialize, Serialize)]
#[component(storage = "SparseSet")]
struct SparseSet(u32);

#[derive(Component)]
#[component(immutable)]
struct EntityVisibility;

impl VisibilityFilter for EntityVisibility {
    type ClientComponent = Self;
    type Scope = Entity;

    fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
        component.is_some()
    }
}

#[derive(Component)]
#[component(immutable)]
struct ComponentVisibility;

impl VisibilityFilter for ComponentVisibility {
    type ClientComponent = Self;
    type Scope = SingleComponent<A>;

    fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
        component.is_some()
    }
}
