use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    prelude::*,
    shared::server_entity_map::ServerEntityMap,
    test_app::{ServerTestAppExt, TestClientEntity},
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn client_replicates_even_with_entity_filter() {
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

    let client_entity = **client_app.world().resource::<TestClientEntity>();

    server_app
        .world_mut()
        .entity_mut(client_entity)
        .insert((Replicated, EntityVisibility, A));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    assert!(entity_map.to_client().get(&client_entity).is_some());
}

#[test]
fn component_replicates_even_with_component_filter() {
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

    let client_entity = **client_app.world().resource::<TestClientEntity>();

    server_app
        .world_mut()
        .entity_mut(client_entity)
        .insert((Replicated, ComponentVisibility, A));

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let entity_map = client_app.world().resource::<ServerEntityMap>();
    let replicated_client_entity = *entity_map
        .to_client()
        .get(&client_entity)
        .expect("client entity itself should replicate");

    assert!(
        client_app
            .world()
            .get::<A>(replicated_client_entity)
            .is_some()
    );
}

#[derive(Component, Deserialize, Serialize)]
struct A;

#[derive(Component)]
#[component(immutable)]
struct ClientFilter;

#[derive(Component, Default)]
#[component(immutable)]
struct EntityVisibility;

impl VisibilityFilter for EntityVisibility {
    type ClientComponent = ClientFilter;
    type Scope = Entity;

    fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
        component.is_some()
    }
}

#[derive(Component, Default)]
#[component(immutable)]
struct ComponentVisibility;

impl VisibilityFilter for ComponentVisibility {
    type ClientComponent = ClientFilter;
    type Scope = SingleComponent<A>;

    fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
        component.is_some()
    }
}
