use bevy::{
    ecs::system::{FilteredResourcesMutParamBuilder, FilteredResourcesParamBuilder, ParamBuilder},
    prelude::*,
};

use super::server_tick::ServerTick;
use crate::{
    prelude::*,
    shared::{
        message::{
            ctx::{ServerReceiveCtx, ServerSendCtx},
            registry::RemoteMessageRegistry,
            server_message::message_buffer::MessageBuffer,
        },
        replication::client_ticks::ClientTicks,
    },
};

/// Sending messages and events from the server to clients.
///
/// Requires [`ServerPlugin`].
/// Can be disabled for apps that act only as clients.
pub struct ServerMessagePlugin;

impl Plugin for ServerMessagePlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        // Construct systems dynamically after all plugins initialization
        // because we need to access resources by registered IDs.
        let registry = app
            .world_mut()
            .remove_resource::<RemoteMessageRegistry>()
            .expect("message registry should be initialized on app build");

        let send_or_buffer_fn = (
            FilteredResourcesParamBuilder::new(|builder| {
                for message in registry.iter_all_server() {
                    builder.add_read_by_id(message.to_messages_id());
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
                for message in registry.iter_all_client() {
                    builder.add_write_by_id(message.from_messages_id());
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
                for event in registry.iter_client_events() {
                    builder.add_write_by_id(event.message().from_messages_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(trigger);

        let resend_locally_fn = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for message in registry.iter_all_server() {
                    builder.add_write_by_id(message.to_messages_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for message in registry.iter_all_server() {
                    builder.add_write_by_id(message.messages_id());
                }
            }),
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(resend_locally);

        app.insert_resource(registry)
            .add_systems(
                PreUpdate,
                (
                    receive_fn.run_if(in_state(ServerState::Running)),
                    trigger_fn.run_if(in_state(ClientState::Disconnected)),
                )
                    .chain()
                    .in_set(ServerSystems::Receive),
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
                    .in_set(ServerSystems::Send),
            );
    }
}

fn send_or_buffer(
    to_messages: FilteredResources,
    mut server_messages: ResMut<ServerMessages>,
    mut message_buffer: ResMut<MessageBuffer>,
    type_registry: Res<AppTypeRegistry>,
    message_registry: Res<RemoteMessageRegistry>,
    clients: Query<Entity, With<ConnectedClient>>,
) {
    message_buffer.start_tick();
    let mut ctx = ServerSendCtx {
        type_registry: &type_registry,
    };

    for message in message_registry.iter_all_server() {
        let to_messages = to_messages
            .get_by_id(message.to_messages_id())
            .expect("to messages resource should be accessible");

        // SAFETY: passed pointer was obtained using this message data.
        unsafe {
            message.send_or_buffer(
                &mut ctx,
                &to_messages,
                &mut server_messages,
                &clients,
                &mut message_buffer,
            );
        }
    }
}

fn send_buffered(
    mut messages: ResMut<ServerMessages>,
    mut message_buffer: ResMut<MessageBuffer>,
    clients: Query<(Entity, Option<&ClientTicks>), With<ConnectedClient>>,
) {
    message_buffer
        .send_all(&mut messages, &clients)
        .expect("buffered server events should send");
}

fn receive(
    mut from_messages: FilteredResourcesMut,
    mut server_messages: ResMut<ServerMessages>,
    type_registry: Res<AppTypeRegistry>,
    message_registry: Res<RemoteMessageRegistry>,
) {
    let mut ctx = ServerReceiveCtx {
        type_registry: &type_registry,
    };

    for message in message_registry.iter_all_client() {
        let from_messages = from_messages
            .get_mut_by_id(message.from_messages_id())
            .expect("from messages resource should be accessible");

        // SAFETY: passed pointer was obtained using this message data.
        unsafe { message.receive(&mut ctx, from_messages.into_inner(), &mut server_messages) };
    }
}

fn trigger(
    mut from_messages: FilteredResourcesMut,
    mut commands: Commands,
    registry: Res<RemoteMessageRegistry>,
) {
    for event in registry.iter_client_events() {
        let from_messages = from_messages
            .get_mut_by_id(event.message().from_messages_id())
            .expect("client messages resource should be accessible");
        // SAFETY: passed pointer was obtained using this message data.
        unsafe { event.trigger(&mut commands, from_messages.into_inner()) };
    }
}

fn resend_locally(
    mut to_messages: FilteredResourcesMut,
    mut messages: FilteredResourcesMut,
    registry: Res<RemoteMessageRegistry>,
) {
    for message in registry.iter_all_server() {
        let to_messages = to_messages
            .get_mut_by_id(message.to_messages_id())
            .expect("to messages resource should be accessible");
        let messages = messages
            .get_mut_by_id(message.messages_id())
            .expect("messages resource should be accessible");

        // SAFETY: passed pointers were obtained using this message data.
        unsafe { message.resend_locally(to_messages.into_inner(), messages.into_inner()) };
    }
}
