use bevy::{ecs::entity::MapEntities, prelude::*, state::app::StatesPlugin, time::TimePlugin};
use bevy_replicon::{
    postcard_utils,
    prelude::*,
    shared::{
        message::{client_message, ctx::ClientSendCtx},
        server_entity_map::ServerEntityMap,
    },
    test_app::ServerTestAppExt,
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn regular() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_client_event::<Test>(Channel::Ordered)
            .finish();
    }
    server_app.init_resource::<EventReader<Test>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(Test);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<EventReader<Test>>();
    assert_eq!(reader.events.len(), 1);
}

#[test]
fn mapped() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        ))
        .add_mapped_client_event::<WithEntity>(Channel::Ordered)
        .finish();
    }
    server_app.init_resource::<EventReader<WithEntity>>();

    server_app.connect_client(&mut client_app);

    let server_entity = server_app.world_mut().spawn(Replicated).id();

    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    let client_entity = *client_app
        .world()
        .resource::<ServerEntityMap>()
        .to_client()
        .get(&server_entity)
        .unwrap();

    client_app
        .world_mut()
        .client_trigger(WithEntity(client_entity));

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<EventReader<WithEntity>>();
    let mapped_entities: Vec<_> = reader.events.iter().map(|event| event.0).collect();
    assert_eq!(mapped_entities, [server_entity]);
}

#[test]
fn without_plugins() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    server_app
        .add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .build()
                .disable::<ClientPlugin>()
                .disable::<ClientMessagePlugin>(),
        ))
        .add_client_event::<Test>(Channel::Ordered)
        .finish();
    client_app
        .add_plugins((
            MinimalPlugins,
            StatesPlugin,
            RepliconPlugins
                .build()
                .disable::<ServerPlugin>()
                .disable::<ServerMessagePlugin>(),
        ))
        .add_client_event::<Test>(Channel::Ordered)
        .finish();
    server_app.init_resource::<EventReader<Test>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(Test);

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();

    let reader = server_app.world().resource::<EventReader<Test>>();
    assert_eq!(reader.events.len(), 1);
}

#[test]
fn local_sending() {
    let mut app = App::new();
    app.add_plugins((TimePlugin, StatesPlugin, RepliconPlugins))
        .add_client_event::<Test>(Channel::Ordered)
        .finish();
    app.init_resource::<EventReader<Test>>();

    app.world_mut().client_trigger(Test);

    // Requires 2 updates because local sending runs
    // in `PostUpdate` and triggering runs in `PreUpdate`.
    app.update();
    app.update();

    let reader = app.world().resource::<EventReader<Test>>();
    assert_eq!(reader.events.len(), 1);
}

#[test]
fn with_disconnect() {
    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_client_event::<Test>(Channel::Ordered)
            .finish();
    }
    client_app.init_resource::<EventReader<Test>>();

    server_app.connect_client(&mut client_app);

    client_app.world_mut().client_trigger(Test);

    server_app.disconnect_client(&mut client_app);

    let reader = client_app.world().resource::<EventReader<Test>>();
    assert!(
        reader.events.is_empty(),
        "client shouldn't resend events locally after disconnect"
    );
}

#[test]
fn serialization_preallocates_capacity() {
    #[derive(Deserialize, Event, Serialize, Clone)]
    struct Large(u64, u64, u64, u64, u64, u64, u64, u64);

    fn serialize_large_checked(
        _ctx: &mut ClientSendCtx,
        event: &Large,
        message_bytes: &mut Vec<u8>,
    ) -> Result<()> {
        assert!(message_bytes.capacity() >= core::mem::size_of::<Large>());
        postcard_utils::to_extend_mut(event, message_bytes)?;
        Ok(())
    }

    let mut server_app = App::new();
    let mut client_app = App::new();
    for app in [&mut server_app, &mut client_app] {
        app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
            .add_client_event_with::<Large>(
                Channel::Ordered,
                serialize_large_checked,
                client_message::default_deserialize::<Large>,
            )
            .finish();
    }

    server_app.connect_client(&mut client_app);

    client_app
        .world_mut()
        .client_trigger(Large(1, 2, 3, 4, 5, 6, 7, 8));

    client_app.update();
    server_app.exchange_with_client(&mut client_app);
    server_app.update();
}

#[derive(Deserialize, Event, Serialize, Clone)]
struct Test;

#[derive(Deserialize, Event, Serialize, Clone, MapEntities)]
struct WithEntity(#[entities] Entity);

#[derive(Resource)]
struct EventReader<E: Event> {
    events: Vec<FromClient<E>>,
}

impl<E: Event + Clone> FromWorld for EventReader<E> {
    fn from_world(world: &mut World) -> Self {
        world.add_observer(|on: On<FromClient<E>>, mut reader: ResMut<Self>| {
            reader.events.push(on.event().clone());
        });

        Self {
            events: Default::default(),
        }
    }
}
