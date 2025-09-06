use bevy::{
    ecs::system::{FilteredResourcesMutParamBuilder, FilteredResourcesParamBuilder, ParamBuilder},
    prelude::*,
};

use super::server_tick::ServerTick;
use crate::{
    prelude::*,
    shared::{
        event::{
            ctx::{ServerReceiveCtx, ServerSendCtx},
            remote_event_registry::RemoteEventRegistry,
            server_event::BufferedServerEvents,
        },
        replication::client_ticks::ClientTicks,
    },
};

/// Sending events from the server to clients.
///
/// Requires [`ServerPlugin`].
/// Can be disabled for apps that act only as clients.
pub struct ServerEventPlugin;

impl Plugin for ServerEventPlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        // Construct systems dynamically after all plugins initialization
        // because we need to access resources by registered IDs.
        let event_registry = app
            .world_mut()
            .remove_resource::<RemoteEventRegistry>()
            .expect("event registry should be initialized on app build");

        let send_or_buffer_fn = (
            FilteredResourcesParamBuilder::new(|builder| {
                for event in event_registry.iter_all_server() {
                    builder.add_read_by_id(event.server_events_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(send_or_buffer);

        let receive_fn = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_all_client() {
                    builder.add_write_by_id(event.client_events_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(receive);

        let trigger_fn = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for trigger in event_registry.iter_client_triggers() {
                    builder.add_write_by_id(trigger.event().client_events_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(trigger);

        let resend_locally_fn = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_all_server() {
                    builder.add_write_by_id(event.server_events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_all_server() {
                    builder.add_write_by_id(event.events_id());
                }
            }),
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(resend_locally);

        app.insert_resource(event_registry)
            .add_systems(
                PreUpdate,
                (
                    receive_fn.run_if(in_state(ServerState::Running)),
                    trigger_fn.run_if(in_state(ClientState::Disconnected)),
                )
                    .chain()
                    .in_set(ServerSet::Receive),
            )
            .add_systems(
                PostUpdate,
                (
                    send_or_buffer_fn.run_if(in_state(ServerState::Running)),
                    send_buffered
                        .run_if(in_state(ServerState::Running))
                        .run_if(resource_changed::<ServerTick>),
                    resend_locally_fn.run_if(in_state(ClientState::Disconnected)),
                )
                    .chain()
                    .after(super::send_replication)
                    .in_set(ServerSet::Send),
            );
    }
}

fn send_or_buffer(
    server_events: FilteredResources,
    mut messages: ResMut<ServerMessages>,
    mut buffered_events: ResMut<BufferedServerEvents>,
    type_registry: Res<AppTypeRegistry>,
    event_registry: Res<RemoteEventRegistry>,
    clients: Query<Entity, With<ConnectedClient>>,
) {
    buffered_events.start_tick();
    let mut ctx = ServerSendCtx {
        type_registry: &type_registry,
    };

    for event in event_registry.iter_all_server() {
        let server_events = server_events
            .get_by_id(event.server_events_id())
            .expect("server events resource should be accessible");

        // SAFETY: passed pointer was obtained using this event data.
        unsafe {
            event.send_or_buffer(
                &mut ctx,
                &server_events,
                &mut messages,
                &clients,
                &mut buffered_events,
            );
        }
    }
}

fn send_buffered(
    mut messages: ResMut<ServerMessages>,
    mut buffered_events: ResMut<BufferedServerEvents>,
    clients: Query<(Entity, Option<&ClientTicks>), With<ConnectedClient>>,
) {
    buffered_events
        .send_all(&mut messages, &clients)
        .expect("buffered server events should send");
}

fn receive(
    mut client_events: FilteredResourcesMut,
    mut messages: ResMut<ServerMessages>,
    type_registry: Res<AppTypeRegistry>,
    event_registry: Res<RemoteEventRegistry>,
) {
    let mut ctx = ServerReceiveCtx {
        type_registry: &type_registry,
    };

    for event in event_registry.iter_all_client() {
        let client_events = client_events
            .get_mut_by_id(event.client_events_id())
            .expect("client events resource should be accessible");

        // SAFETY: passed pointer was obtained using this event data.
        unsafe { event.receive(&mut ctx, client_events.into_inner(), &mut messages) };
    }
}

fn trigger(
    mut client_events: FilteredResourcesMut,
    mut commands: Commands,
    event_registry: Res<RemoteEventRegistry>,
) {
    for trigger in event_registry.iter_client_triggers() {
        let client_events = client_events
            .get_mut_by_id(trigger.event().client_events_id())
            .expect("client events resource should be accessible");
        trigger.trigger(&mut commands, client_events.into_inner());
    }
}

fn resend_locally(
    mut server_events: FilteredResourcesMut,
    mut events: FilteredResourcesMut,
    event_registry: Res<RemoteEventRegistry>,
) {
    for event in event_registry.iter_all_server() {
        let server_events = server_events
            .get_mut_by_id(event.server_events_id())
            .expect("server events resource should be accessible");
        let events = events
            .get_mut_by_id(event.events_id())
            .expect("events resource should be accessible");

        // SAFETY: passed pointers were obtained using this event data.
        unsafe { event.resend_locally(server_events.into_inner(), events.into_inner()) };
    }
}
