use bevy::{
    ecs::system::{FilteredResourcesMutParamBuilder, FilteredResourcesParamBuilder, ParamBuilder},
    prelude::*,
};

use super::ServerUpdateTick;
use crate::{
    prelude::*,
    shared::{
        message::{
            ctx::{ClientReceiveCtx, ClientSendCtx},
            registry::RemoteMessageRegistry,
        },
        server_entity_map::ServerEntityMap,
    },
};

/// Sending messages and events from a client to the server.
///
/// Requires [`ClientPlugin`].
/// Can be disabled for apps that act only as servers.
pub struct ClientMessagePlugin;

impl Plugin for ClientMessagePlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        // Construct systems dynamically after all plugins initialization
        // because we need to access resources by registered IDs.
        let registry = app
            .world_mut()
            .remove_resource::<RemoteMessageRegistry>()
            .expect("message registry should be initialized on app build");

        let send_fn = (
            FilteredResourcesParamBuilder::new(|builder| {
                for message in registry.iter_all_client() {
                    builder.add_read_by_id(message.messages_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for message in registry.iter_all_client() {
                    builder.add_write_by_id(message.reader_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(send);

        let receive_builder = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for message in registry.iter_all_server() {
                    builder.add_write_by_id(message.messages_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for message in registry.iter_all_server() {
                    builder.add_write_by_id(message.queue_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
            ParamBuilder,
        );

        let receive_fn = receive_builder
            .clone()
            .build_state(app.world_mut())
            .build_system(receive);

        let enter_receive_fn = receive_builder
            .build_state(app.world_mut())
            .build_system(receive);

        let trigger_builder = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in registry.iter_server_events() {
                    builder.add_write_by_id(event.message().messages_id());
                }
            }),
            ParamBuilder,
            ParamBuilder,
        );

        let trigger_fn = trigger_builder
            .clone()
            .build_state(app.world_mut())
            .build_system(trigger);

        let enter_trigger_fn = trigger_builder
            .build_state(app.world_mut())
            .build_system(trigger);

        let send_locally_fn = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for message in registry.iter_all_client() {
                    builder.add_write_by_id(message.from_messages_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for message in registry.iter_all_client() {
                    builder.add_write_by_id(message.messages_id());
                }
            }),
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(send_locally);

        let reset_fn = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for message in registry.iter_all_client() {
                    builder.add_write_by_id(message.messages_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for message in registry.iter_all_server() {
                    builder.add_write_by_id(message.queue_id());
                }
            }),
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(reset);

        app.insert_resource(registry)
            .add_systems(
                PreUpdate,
                (
                    receive_fn.run_if(in_state(ClientState::Connected)),
                    trigger_fn,
                )
                    .chain()
                    .after(super::receive_replication)
                    .in_set(ClientSystems::Receive),
            )
            .add_systems(
                OnEnter(ClientState::Connected),
                (
                    reset_fn.in_set(ClientSystems::ResetEvents),
                    (enter_receive_fn, enter_trigger_fn)
                        .chain()
                        .after(super::receive_replication)
                        .in_set(ClientSystems::Receive),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    send_fn.run_if(in_state(ClientState::Connected)),
                    send_locally_fn.run_if(in_state(ClientState::Disconnected)),
                )
                    .chain()
                    .in_set(ClientSystems::Send),
            );
    }
}

fn send(
    messages: FilteredResources,
    mut readers: FilteredResourcesMut,
    mut client_messages: ResMut<ClientMessages>,
    type_registry: Res<AppTypeRegistry>,
    entity_map: Res<ServerEntityMap>,
    registry: Res<RemoteMessageRegistry>,
) {
    let mut ctx = ClientSendCtx {
        entity_map: &entity_map,
        type_registry: &type_registry,
        invalid_entities: Vec::new(),
    };

    for message in registry.iter_all_client() {
        let messages = messages
            .get_by_id(message.messages_id())
            .expect("messages resource should be accessible");
        let reader = readers
            .get_mut_by_id(message.reader_id())
            .expect("message reader resource should be accessible");

        // SAFETY: passed pointers were obtained using this message data.
        unsafe {
            message.send(
                &mut ctx,
                &messages,
                reader.into_inner(),
                &mut client_messages,
            );
        }
    }
}

fn receive(
    mut messages: FilteredResourcesMut,
    mut queues: FilteredResourcesMut,
    mut client_messages: ResMut<ClientMessages>,
    type_registry: Res<AppTypeRegistry>,
    entity_map: Res<ServerEntityMap>,
    message_registry: Res<RemoteMessageRegistry>,
    update_tick: Res<ServerUpdateTick>,
) {
    let mut ctx = ClientReceiveCtx {
        type_registry: &type_registry,
        entity_map: &entity_map,
        invalid_entities: Vec::new(),
    };

    for message in message_registry.iter_all_server() {
        let messages = messages
            .get_mut_by_id(message.messages_id())
            .expect("messages resource should be accessible");
        let queue = queues
            .get_mut_by_id(message.queue_id())
            .expect("queue resource should be accessible");

        // SAFETY: passed pointers were obtained using this message data.
        unsafe {
            message.receive(
                &mut ctx,
                messages.into_inner(),
                queue.into_inner(),
                &mut client_messages,
                **update_tick,
            )
        };
    }
}

fn trigger(
    mut messages: FilteredResourcesMut,
    mut commands: Commands,
    registry: Res<RemoteMessageRegistry>,
) {
    for event in registry.iter_server_events() {
        let messages = messages
            .get_mut_by_id(event.message().messages_id())
            .expect("messages resource should be accessible");
        event.trigger(&mut commands, messages.into_inner());
    }
}

fn send_locally(
    mut from_messages: FilteredResourcesMut,
    mut messages: FilteredResourcesMut,
    registry: Res<RemoteMessageRegistry>,
) {
    for message in registry.iter_all_client() {
        let from_messages = from_messages
            .get_mut_by_id(message.from_messages_id())
            .expect("from messages resource should be accessible");
        let messages = messages
            .get_mut_by_id(message.messages_id())
            .expect("messages resource should be accessible");

        // SAFETY: passed pointers were obtained using this message data.
        unsafe { message.send_locally(from_messages.into_inner(), messages.into_inner()) };
    }
}

fn reset(
    mut messages: FilteredResourcesMut,
    mut queues: FilteredResourcesMut,
    registry: Res<RemoteMessageRegistry>,
) {
    for message in registry.iter_all_client() {
        let messages = messages
            .get_mut_by_id(message.messages_id())
            .expect("messages resource should be accessible");

        // SAFETY: passed pointer was obtained using this message data.
        unsafe { message.reset(messages.into_inner()) };
    }

    for messages in registry.iter_all_server() {
        let queue = queues
            .get_mut_by_id(messages.queue_id())
            .expect("queue resource should be accessible");

        // SAFETY: passed pointer was obtained using this message data.
        unsafe { messages.reset(queue.into_inner()) };
    }
}
