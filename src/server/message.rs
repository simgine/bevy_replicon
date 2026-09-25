use bevy::{
    ecs::{
        resource::ResourceEntities,
        system::{ParamBuilder, QueryParamBuilder},
        world::{FilteredEntityMut, FilteredEntityRef},
    },
    prelude::*,
};

use super::server_tick::ServerTick;
use crate::{
    prelude::*,
    shared::{
        get_resource_by_id, get_resource_entity_mut, get_resource_mut_by_id,
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

        if registry.has_any_client() {
            let receive_fn = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_client() {
                        builder.optional(|builder| {
                            builder.mut_id(message.from_messages_id());
                        });
                    }
                }),
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(receive);

            app.add_systems(
                PreUpdate,
                receive_fn
                    .run_if(in_state(ServerState::Running))
                    .in_set(ServerSystems::Receive),
            );
        }

        if registry.has_client_events() {
            let trigger_fn = (
                QueryParamBuilder::new(|builder| {
                    for event in registry.iter_client_events() {
                        builder.optional(|builder| {
                            builder.mut_id(event.message().from_messages_id());
                        });
                    }
                }),
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(trigger);

            app.add_systems(
                PreUpdate,
                trigger_fn
                    .after(receive)
                    .run_if(in_state(ClientState::Disconnected))
                    .in_set(ServerSystems::Receive),
            );
        }

        if registry.has_any_shared() {
            let receive_shared_fn = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_shared() {
                        builder.optional(|builder| {
                            builder.mut_id(message.shared_messages_id());
                        });
                    }
                }),
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(receive_shared);

            app.add_systems(
                PreUpdate,
                receive_shared_fn
                    .run_if(in_state(ServerState::Running))
                    .in_set(ServerSystems::Receive),
            );
        }

        if registry.has_shared_events() {
            let trigger_shared_fn = (
                QueryParamBuilder::new(|builder| {
                    for event in registry.iter_shared_events() {
                        builder.optional(|builder| {
                            builder.mut_id(event.message().shared_messages_id());
                        });
                    }
                }),
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(trigger_shared);

            app.add_systems(
                PreUpdate,
                trigger_shared_fn
                    .after(receive_shared)
                    .in_set(ServerSystems::Receive),
            );
        }

        if registry.has_any_server() {
            let send_or_buffer_fn = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_server() {
                        builder.optional(|builder| {
                            builder.ref_id(message.to_messages_id());
                        });
                    }
                }),
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(send_or_buffer);

            let send_locally_fn = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_server() {
                        builder.optional(|builder| {
                            builder.mut_id(message.to_messages_id());
                        });
                    }
                }),
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_server() {
                        builder.optional(|builder| {
                            builder.mut_id(message.messages_id());
                        });
                    }
                }),
                ParamBuilder,
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(send_locally);

            app.add_systems(
                PostUpdate,
                (
                    send_or_buffer_fn.run_if(in_state(ServerState::Running)),
                    send_buffered
                        .run_if(in_state(ServerState::Running))
                        .run_if(resource_changed::<ServerTick>),
                    send_locally_fn.run_if(in_state(ClientState::Disconnected)),
                )
                    .chain()
                    .after(super::send_messages)
                    .in_set(ServerSystems::Send),
            );
        }

        app.insert_resource(registry);
    }
}

fn send_or_buffer(
    to_messages: Query<FilteredEntityRef>,
    mut server_messages: ResMut<ServerMessages>,
    mut message_buffer: ResMut<MessageBuffer>,
    mut storage: ResMut<ReplicationStorage>,
    type_registry: Res<AppTypeRegistry>,
    message_registry: Res<RemoteMessageRegistry>,
    clients: Query<Entity, With<ConnectedClient>>,
    resource_entities: &ResourceEntities,
) {
    message_buffer.start_tick();
    let mut ctx = ServerSendCtx {
        storage: &mut storage,
        type_registry: &type_registry,
    };

    for message in message_registry.iter_all_server() {
        let to_messages =
            get_resource_by_id(message.to_messages_id(), &to_messages, resource_entities)
                .expect("to clients messages resource should be accessible");

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
    mut from_messages: Query<FilteredEntityMut>,
    mut server_messages: ResMut<ServerMessages>,
    mut storage: ResMut<ReplicationStorage>,
    type_registry: Res<AppTypeRegistry>,
    message_registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    let mut ctx = ServerReceiveCtx {
        storage: &mut storage,
        type_registry: &type_registry,
    };

    for message in message_registry.iter_all_client() {
        let mut from_messages = get_resource_entity_mut(
            message.from_messages_id(),
            &mut from_messages,
            resource_entities,
        );
        let from_messages = from_messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.from_messages_id(), entity))
            .expect("from clients messages resource should be accessible");

        // SAFETY: passed pointer was obtained using this message data.
        unsafe { message.receive(&mut ctx, from_messages.into_inner(), &mut server_messages) };
    }
}

fn receive_shared(
    mut shared_messages: Query<FilteredEntityMut>,
    mut server_messages: ResMut<ServerMessages>,
    mut storage: ResMut<ReplicationStorage>,
    type_registry: Res<AppTypeRegistry>,
    message_registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    let mut ctx = ServerReceiveCtx {
        storage: &mut storage,
        type_registry: &type_registry,
    };

    for message in message_registry.iter_all_shared() {
        let mut shared_messages = get_resource_entity_mut(
            message.shared_messages_id(),
            &mut shared_messages,
            resource_entities,
        );
        let shared_messages = shared_messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.shared_messages_id(), entity))
            .expect("shared messages resource should be accessible");

        // SAFETY: passed pointer was obtained using this message data.
        unsafe { message.receive(&mut ctx, shared_messages.into_inner(), &mut server_messages) };
    }
}

fn trigger(
    mut from_messages: Query<FilteredEntityMut>,
    mut commands: Commands,
    registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    for event in registry.iter_client_events() {
        let mut from_messages = get_resource_entity_mut(
            event.message().from_messages_id(),
            &mut from_messages,
            resource_entities,
        );
        let from_messages = from_messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(event.message().from_messages_id(), entity))
            .expect("from clients messages resource should be accessible");
        // SAFETY: passed pointer was obtained using this message data.
        unsafe { event.trigger(&mut commands, from_messages.into_inner()) };
    }
}

fn trigger_shared(
    mut shared_messages: Query<FilteredEntityMut>,
    mut commands: Commands,
    registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    for event in registry.iter_shared_events() {
        let mut shared_messages = get_resource_entity_mut(
            event.message().shared_messages_id(),
            &mut shared_messages,
            resource_entities,
        );
        let shared_messages = shared_messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(event.message().shared_messages_id(), entity))
            .expect("shared messages resource should be accessible");
        // SAFETY: passed pointer was obtained using this event data.
        unsafe { event.trigger(&mut commands, shared_messages.into_inner()) };
    }
}

fn send_locally(
    mut to_messages: Query<FilteredEntityMut>,
    mut messages: Query<FilteredEntityMut>,
    registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    for message in registry.iter_all_server() {
        let mut to_messages = get_resource_entity_mut(
            message.to_messages_id(),
            &mut to_messages,
            resource_entities,
        );
        let to_messages = to_messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.to_messages_id(), entity))
            .expect("to messages resource should be accessible");

        let mut messages =
            get_resource_entity_mut(message.messages_id(), &mut messages, resource_entities);
        let messages = messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.messages_id(), entity))
            .expect("messages resource should be accessible");

        // SAFETY: passed pointers were obtained using this message data.
        unsafe { message.send_locally(to_messages.into_inner(), messages.into_inner()) };
    }
}
