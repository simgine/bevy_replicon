use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    client::confirm_history::{ConfirmHistory, EntityReplicated},
    prelude::*,
    server::server_tick::ServerTick,
    shared::replication::registry::receive_fns,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn single() {
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

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    let mut required = client_app.world_mut().query::<&Required>();
    assert_eq!(required.iter(client_app.world()).len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
    assert_eq!(required.iter(client_app.world()).len(), 1);
}

#[test]
fn multiple() {
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
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A, B)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<(&A, &B)>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    let before_archetypes = client_app.world().archetypes().len();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<(A, B)>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
    assert_eq!(
        client_app.world().archetypes().len() - before_archetypes,
        1,
        "should cause only a single archetype move"
    );
}

#[test]
fn receive_fns() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .set_receive_fns::<A>(
            receive_fns::default_write,
            receive_fns::remove_with_requires::<A>,
        )
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    let mut required = client_app.world_mut().query::<&Required>();
    assert_eq!(required.iter(client_app.world()).len(), 1);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
    assert_eq!(required.iter(client_app.world()).len(), 0);
}

#[test]
fn marker() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .register_marker::<ReplaceMarker>()
        .replicate::<A>()
        .set_marker_fns::<ReplaceMarker, A>(
            receive_fns::default_write,
            receive_fns::remove_with_requires::<A>,
        )
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, A, Signature::from(0)))
        .id();

    let client_entity = client_app
        .world_mut()
        .spawn((ReplaceMarker, Signature::from(0)))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    assert!(!client_entity.contains::<A>());
    assert!(!client_entity.contains::<Required>());
}

#[test]
fn group() {
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

    let server_entity = server_app.world_mut().spawn((Replicated, (A, B))).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, (With<A>, With<B>)>()
        .single(client_app.world())
        .unwrap();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<(A, B)>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    assert!(!client_entity.contains::<A>());
    assert!(!client_entity.contains::<B>());
}

#[test]
fn not_replicated() {
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

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, NotReplicated))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, (With<Remote>, Without<NotReplicated>)>()
        .single(client_app.world())
        .unwrap();

    client_app
        .world_mut()
        .entity_mut(client_entity)
        .insert(NotReplicated);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<NotReplicated>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    assert!(client_entity.contains::<NotReplicated>());
}

#[test]
fn with_client_despawn() {
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

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut with_components = client_app.world_mut().query_filtered::<Entity, With<A>>();
    let client_entity = with_components.single(client_app.world()).unwrap();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    client_app.world_mut().entity_mut(client_entity).despawn();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).len(), 0);
}

#[test]
fn after_insertion() {
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

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    // Insert and remove at the same time.
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(A)
        .remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
}

#[test]
fn after_spawn() {
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

    server_app.world_mut().spawn((Replicated, A)).remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut remote = client_app
        .world_mut()
        .query_filtered::<&Remote, Without<A>>();
    assert_eq!(remote.iter(client_app.world()).len(), 1);
}

#[test]
fn after_unreplicate() {
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

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut remote = client_app
        .world_mut()
        .query_filtered::<Entity, With<Remote>>();
    let client_entity = remote.single(client_app.world()).unwrap();

    // Un-replicate and remove at the same time.
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<Replicated>()
        .remove::<A>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    assert!(client_entity.contains::<Remote>());
    assert!(client_entity.contains::<A>());
}

#[test]
fn confirm_history() {
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

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, With<A>>()
        .single(client_app.world())
        .unwrap();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    // Clear previous messages.
    client_app
        .world_mut()
        .resource_mut::<Messages<EntityReplicated>>()
        .clear();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();

    let confirm_history = client_app
        .world_mut()
        .get::<ConfirmHistory>(client_entity)
        .unwrap();
    assert!(confirm_history.contains(tick));

    let mut replicated = client_app
        .world_mut()
        .resource_mut::<Messages<EntityReplicated>>();
    let [replicated] = replicated.drain().collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(replicated.entity, client_entity);
    assert_eq!(replicated.tick, tick);
}

#[test]
fn hidden_entity() {
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

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, EntityVisibility, A))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(
        messages.drain_sent().len(),
        0,
        "client shouldn't receive removal for a hidden entity"
    );
}

#[test]
fn hidden_component() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
        .add_visibility_filter::<ComponentVisibility>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, ComponentVisibility, A))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>();

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(
        messages.drain_sent().len(),
        0,
        "client shouldn't receive removal for a hidden component"
    );
}

#[test]
fn hidden_all_except() {
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
        .add_visibility_filter::<AllExceptVisibilityA>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Client has the filter component, so the whole entity is visible.
    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert(AllExceptVisibilityA);
    server_app
        .world_mut()
        .spawn((Replicated, A, B, AllExceptVisibilityA));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut b = client_app.world_mut().query::<&B>();
    assert_eq!(
        b.iter(client_app.world()).len(),
        1,
        "allowed by all except should replicate while visible"
    );

    // Drop the filter component, so the allow-list applies and `B` is removed on the client.
    server_app
        .world_mut()
        .entity_mut(client)
        .remove::<AllExceptVisibilityA>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut a = client_app.world_mut().query::<&A>();
    assert_eq!(a.iter(client_app.world()).len(), 1, "allowed by all except");
    let mut b = client_app.world_mut().query::<&B>();
    assert_eq!(b.iter(client_app.world()).len(), 0, "hidden by all except");
}

// Two `AllExcept` filters on the same entity, `C` is hidden by both.
#[test]
fn multiple_hidden_all_except() {
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
        .replicate::<C>()
        .add_visibility_filter::<AllExceptVisibilityA>()
        .add_visibility_filter::<AllExceptVisibilityB>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert((AllExceptVisibilityA, AllExceptVisibilityB));
    server_app.world_mut().spawn((
        Replicated,
        A,
        B,
        C,
        AllExceptVisibilityA,
        AllExceptVisibilityB,
    ));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut c = client_app.world_mut().query::<&C>();
    assert_eq!(
        c.iter(client_app.world()).len(),
        1,
        "fully visible at first"
    );

    // Both filters deny: one allows only `A`, the other only `B`. Their allow-lists
    // intersect, so everything is hidden and `C` is hidden by both.
    server_app
        .world_mut()
        .entity_mut(client)
        .remove::<(AllExceptVisibilityA, AllExceptVisibilityB)>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut a = client_app.world_mut().query::<&A>();
    assert_eq!(
        a.iter(client_app.world()).len(),
        0,
        "hidden by the all except B filter"
    );
    let mut b = client_app.world_mut().query::<&B>();
    assert_eq!(
        b.iter(client_app.world()).len(),
        0,
        "hidden by the all except A filter"
    );
    let mut c = client_app.world_mut().query::<&C>();
    assert_eq!(
        c.iter(client_app.world()).len(),
        0,
        "hidden by both filters"
    );
}

// `Components` and `AllExcept` filters on the same entity, `A` is hidden by both.
#[test]
fn hidden_component_and_all_expect() {
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
        .replicate::<C>()
        .add_visibility_filter::<ComponentVisibility>()
        .add_visibility_filter::<AllExceptVisibilityB>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert((ComponentVisibility, AllExceptVisibilityB));
    server_app.world_mut().spawn((
        Replicated,
        A,
        B,
        C,
        ComponentVisibility,
        AllExceptVisibilityB,
    ));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut a = client_app.world_mut().query::<&A>();
    assert_eq!(
        a.iter(client_app.world()).len(),
        1,
        "fully visible at first"
    );

    // Hide-list hides `A`; allow-list keeps only `B`, so it also hides `A` and `C`.
    server_app
        .world_mut()
        .entity_mut(client)
        .remove::<(ComponentVisibility, AllExceptVisibilityB)>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut a = client_app.world_mut().query::<&A>();
    assert_eq!(
        a.iter(client_app.world()).len(),
        0,
        "hidden by both filters"
    );
    let mut b = client_app.world_mut().query::<&B>();
    assert_eq!(b.iter(client_app.world()).len(), 1, "allowed by all except");
    let mut c = client_app.world_mut().query::<&C>();
    assert_eq!(c.iter(client_app.world()).len(), 0, "hidden by all except");
}

#[test]
fn visibility_lose() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<A>()
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
        .spawn((Replicated, A, ComponentVisibility));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    server_app
        .world_mut()
        .entity_mut(client)
        .remove::<ComponentVisibility>();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).len(), 0);
}

#[derive(Component, Deserialize, Serialize)]
#[require(Required)]
struct A;

#[derive(Component, Deserialize, Serialize)]
struct B;

#[derive(Component, Deserialize, Serialize)]
struct C;

#[derive(Component, Deserialize, Serialize)]
struct NotReplicated;

#[derive(Component)]
struct ReplaceMarker;

#[derive(Component, Default)]
struct Required;

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

#[derive(Component)]
#[component(immutable)]
struct AllExceptVisibilityA;

impl VisibilityFilter for AllExceptVisibilityA {
    type ClientComponent = Self;
    type Scope = AllExcept<SingleComponent<A>>;

    fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
        component.is_some()
    }
}

#[derive(Component)]
#[component(immutable)]
struct AllExceptVisibilityB;

impl VisibilityFilter for AllExceptVisibilityB {
    type ClientComponent = Self;
    type Scope = AllExcept<SingleComponent<B>>;

    fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
        component.is_some()
    }
}
