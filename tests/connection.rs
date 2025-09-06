use core::marker::PhantomData;

use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    prelude::*,
    server::server_tick::ServerTick,
    shared::backend::connected_client::{ConnectedClient, NetworkId, NetworkIdMap},
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn client_connect_disconnect() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .finish();
    }

    server_app.connect_client(&mut client_app);

    let mut clients = server_app
        .world_mut()
        .query_filtered::<Entity, With<AuthorizedClient>>();
    assert_eq!(clients.iter(server_app.world()).len(), 1);

    server_app.disconnect_client(&mut client_app);

    assert_eq!(clients.iter(server_app.world()).len(), 0);
}

#[test]
fn server_start_stop() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .set(RepliconSharedPlugin {
                    auth_method: AuthMethod::Custom,
                })
                .set(ServerPlugin {
                    tick_schedule: TickSchedule::PostUpdate,
                    ..Default::default()
                }),
        ))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let mut clients = server_app
        .world_mut()
        .query_filtered::<Entity, With<ConnectedClient>>();
    assert_eq!(clients.iter(server_app.world()).len(), 1);
    assert_ne!(server_app.world().resource::<ServerTick>().get(), 0);

    server_app
        .world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Stopped);

    server_app.update();

    assert_eq!(clients.iter(server_app.world()).len(), 0);
    assert_eq!(server_app.world().resource::<ServerTick>().get(), 0);
}

#[test]
fn protocol_mismatch() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    server_app
        .add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
        .add_client_event::<TestEvent>(Channel::Ordered)
        .finish();
    client_app
        .init_resource::<TriggerCounter<ProtocolMismatch>>()
        .add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
        .finish();

    server_app.connect_client(&mut client_app);

    let mut clients = server_app
        .world_mut()
        .query_filtered::<Entity, With<AuthorizedClient>>();
    assert_eq!(clients.iter(server_app.world()).len(), 0);

    let counter = client_app
        .world()
        .resource::<TriggerCounter<ProtocolMismatch>>();
    assert_eq!(counter.triggers, 1);
}

#[test]
fn custom_auth() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(RepliconSharedPlugin {
                auth_method: AuthMethod::Custom,
            }),
        ))
        .finish();
    }

    server_app.connect_client(&mut client_app);

    let mut clients = server_app
        .world_mut()
        .query_filtered::<&ConnectedClient, Without<AuthorizedClient>>();
    assert_eq!(clients.iter(server_app.world()).count(), 1);
}

#[test]
fn disabled_auth() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(RepliconSharedPlugin {
            auth_method: AuthMethod::None,
        }),
    ))
    .finish();

    let entity = app.world_mut().spawn(ConnectedClient { max_size: 1200 });
    assert!(entity.contains::<AuthorizedClient>());
}

#[test]
fn network_id_map() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
        .finish();

    let client_entity = app.world_mut().spawn(NetworkId::new(0)).id();
    assert_eq!(app.world().resource::<NetworkIdMap>().len(), 1);

    app.world_mut().despawn(client_entity);
    assert!(app.world().resource::<NetworkIdMap>().is_empty());
}

#[derive(Event, Serialize, Deserialize)]
struct TestEvent;

#[derive(Resource)]
struct TriggerCounter<E: Event> {
    triggers: usize,
    marker: PhantomData<E>,
}

impl<E: Event> FromWorld for TriggerCounter<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(|_trigger: Trigger<E>, mut counter: ResMut<Self>| {
            counter.triggers += 1;
        });

        Self {
            triggers: 0,
            marker: PhantomData,
        }
    }
}
