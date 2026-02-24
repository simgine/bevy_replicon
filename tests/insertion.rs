use bevy::{ecs::system::SystemState, prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    client::confirm_history::{ConfirmHistory, EntityReplicated},
    prelude::*,
    server::server_tick::ServerTick,
    shared::{
        replication::{
            deferred_entity::DeferredEntity,
            registry::{ctx::WriteCtx, receive_fns},
        },
        server_entity_map::ServerEntityMap,
    },
    test_app::{ServerTestAppExt, TestClientEntity},
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn table_storage() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<Table>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Table);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&Table>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
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
        .replicate::<SparseSet>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(SparseSet);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&SparseSet>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn immutable() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<Immutable>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Immutable(false));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&Immutable>();
    let component = components.single(client_app.world()).unwrap();
    assert!(!component.0);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Immutable(true));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let component = components.single(client_app.world()).unwrap();
    assert!(component.0);
}

#[test]
fn mapped_existing_entity() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<MappedComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();
    let server_map_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(MappedComponent(server_map_entity));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_map_entity = *client_app
        .world()
        .resource::<ServerEntityMap>()
        .to_client()
        .get(&server_map_entity)
        .unwrap();

    let mapped_component = client_app
        .world_mut()
        .query::<&MappedComponent>()
        .single(client_app.world())
        .unwrap();
    assert_eq!(mapped_component.0, client_map_entity);
}

#[test]
fn mapped_new_entity() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .replicate::<MappedComponent>()
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();
    let server_map_entity = server_app.world_mut().spawn_empty().id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(MappedComponent(server_map_entity));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mapped_component = client_app
        .world_mut()
        .query::<&MappedComponent>()
        .single(client_app.world())
        .unwrap();
    assert!(client_app.world().get_entity(mapped_component.0).is_ok());

    let mut remote = client_app.world_mut().query::<&Remote>();
    assert_eq!(remote.iter(client_app.world()).count(), 1);
}

#[test]
fn multiple_components() {
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let before_archetypes = client_app.world().archetypes().len();

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert((A, B));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&A, &B)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
    assert_eq!(
        client_app.world().archetypes().len() - before_archetypes,
        1,
        "should cause only a single archetype move"
    );
}

#[test]
fn multiple_components_sequential() {
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

    // Spawn an entity with replicated component.
    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<(&Remote, &A)>();
    assert_eq!(components.iter(client_app.world()).len(), 1);

    // Insert another replicated component.
    server_app.world_mut().entity_mut(server_entity).insert(B);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<(&Remote, &A, &B)>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[test]
fn rule_split_across_ticks() {
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

    let server_entity = server_app.world_mut().spawn((Replicated, A)).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut a_components = client_app.world_mut().query::<&A>();
    assert_eq!(a_components.iter(client_app.world()).count(), 0);

    // Insert component that should trigger replication of another component.
    server_app.world_mut().entity_mut(server_entity).insert(B);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut all_components = client_app.world_mut().query::<(&A, &B)>();
    assert_eq!(all_components.iter(client_app.world()).count(), 1);
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
        .replicate::<Original>()
        .set_receive_fns(replace, receive_fns::default_remove::<Replaced>)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Original);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app
        .world_mut()
        .query_filtered::<&Replaced, Without<Original>>();
    assert_eq!(components.iter(client_app.world()).count(), 1);
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
        .replicate::<Original>()
        .set_marker_fns::<ReplaceMarker, _>(replace, receive_fns::default_remove::<Replaced>)
        .finish();
    }

    server_app.connect_client(&mut client_app);

    // Make entity IDs different between client and server.
    client_app.world_mut().spawn_empty();

    let server_entity = server_app
        .world_mut()
        .spawn((Replicated, Signature::from(0)))
        .id();
    let client_entity = client_app
        .world_mut()
        .spawn((ReplaceMarker, Signature::from(0)))
        .id();
    assert_ne!(server_entity, client_entity);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert(Original);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = client_app.world().entity(client_entity);
    assert!(!client_entity.contains::<Original>());
    assert!(client_entity.contains::<Replaced>());
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app
        .world_mut()
        .entity_mut(server_entity)
        .insert((A, B));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut groups = client_app.world_mut().query::<(&A, &B)>();
    assert_eq!(groups.iter(client_app.world()).count(), 1);
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app.world_mut().entity_mut(server_entity).insert(A);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).count(), 0);
}

#[test]
fn after_removal() {
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

    // Insert and remove at the same time.
    server_app
        .world_mut()
        .entity_mut(server_entity)
        .remove::<A>()
        .insert(A);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).count(), 1);

    let mut system_state: SystemState<RemovedComponents<A>> =
        SystemState::new(client_app.world_mut());
    let removals = system_state.get(client_app.world());
    assert_eq!(
        removals.len(),
        1,
        "removal for the old value should also be triggered"
    );
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut remote = client_app
        .world_mut()
        .query_filtered::<Entity, With<Remote>>();
    let client_entity = remote.single(client_app.world()).unwrap();

    server_app.world_mut().entity_mut(server_entity).insert(A);

    client_app.world_mut().entity_mut(client_entity).despawn();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).len(), 0);
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

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app.world_mut().entity_mut(server_entity).insert(A);

    // Clear previous messages.
    client_app
        .world_mut()
        .resource_mut::<Messages<EntityReplicated>>()
        .clear();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let tick = **server_app.world().resource::<ServerTick>();

    let (client_entity, confirm_history) = client_app
        .world_mut()
        .query::<(Entity, &ConfirmHistory)>()
        .single(client_app.world())
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
        .spawn((Replicated, EntityVisibility))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app.world_mut().entity_mut(server_entity).insert(A);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(
        messages.drain_sent().count(),
        0,
        "client shouldn't receive insertion for a hidden entity"
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
        .spawn((Replicated, ComponentVisibility))
        .id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    server_app.world_mut().entity_mut(server_entity).insert(A);

    server_app.update();

    let mut messages = server_app.world_mut().resource_mut::<ServerMessages>();
    assert_eq!(
        messages.drain_sent().count(),
        0,
        "client shouldn't receive insertion for a hidden component"
    );
}

#[test]
fn visibility_gain() {
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

    server_app
        .world_mut()
        .spawn((Replicated, A, ComponentVisibility));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();
    server_app.exchange_with_client(&mut client_app);

    let mut components = client_app.world_mut().query::<&A>();
    assert_eq!(components.iter(client_app.world()).count(), 0);

    let client = **client_app.world().resource::<TestClientEntity>();
    server_app
        .world_mut()
        .entity_mut(client)
        .insert(ComponentVisibility);

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(components.iter(client_app.world()).count(), 1);
}

#[derive(Component, Deserialize, Serialize)]
#[component(storage = "Table")]
struct Table;

#[derive(Component, Deserialize, Serialize)]
#[component(storage = "SparseSet")]
struct SparseSet;

#[derive(Component, Deserialize, Serialize)]
struct MappedComponent(#[entities] Entity);

#[derive(Component, Deserialize, Serialize)]
#[component(immutable)]
struct Immutable(bool);

#[derive(Component, Deserialize, Serialize)]
struct A;

#[derive(Component, Deserialize, Serialize)]
struct B;

#[derive(Component)]
struct ReplaceMarker;

#[derive(Component, Deserialize, Serialize)]
struct Original;

#[derive(Component, Deserialize, Serialize)]
struct Replaced;

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

/// Deserializes [`OriginalComponent`], but ignores it and inserts [`ReplacedComponent`].
fn replace(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<Original>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    rule_fns.deserialize(ctx, message)?;
    entity.insert(Replaced);

    Ok(())
}
