use bevy::{
    ecs::{
        resource::ResourceEntities,
        system::{ParamBuilder, QueryParamBuilder},
        world::{FilteredEntityMut, FilteredEntityRef},
    },
    prelude::*,
};

use super::ServerUpdateTick;
use crate::{
    prelude::*,
    shared::{
        get_resource_by_id, get_resource_entity_mut, get_resource_mut_by_id,
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

        if registry.has_any_server() {
            let receive_builder = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_server() {
                        builder.optional(|builder| {
                            builder.mut_id(message.messages_id());
                        });
                    }
                }),
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_server() {
                        builder.optional(|builder| {
                            builder.mut_id(message.queue_id());
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
            );

            let receive_fn = receive_builder
                .clone()
                .build_state(app.world_mut())
                .build_system(receive);

            let enter_receive_fn = receive_builder
                .build_state(app.world_mut())
                .build_system(receive);

            app.add_systems(
                PreUpdate,
                receive_fn
                    .after(super::receive_replication)
                    .run_if(in_state(ClientState::Connected))
                    .in_set(ClientSystems::Receive),
            )
            .add_systems(
                OnEnter(ClientState::Connected),
                enter_receive_fn
                    .after(super::receive_replication)
                    .in_set(ClientSystems::Receive),
            );
        }

        if registry.has_server_events() {
            let trigger_builder = (
                QueryParamBuilder::new(|builder| {
                    for event in registry.iter_server_events() {
                        builder.optional(|builder| {
                            builder.mut_id(event.message().messages_id());
                        });
                    }
                }),
                ParamBuilder,
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

            app.add_systems(
                PreUpdate,
                trigger_fn.after(receive).in_set(ClientSystems::Receive),
            )
            .add_systems(
                OnEnter(ClientState::Connected),
                enter_trigger_fn
                    .after(receive)
                    .in_set(ClientSystems::Receive),
            );
        }

        if registry.has_any_client() {
            let send_fn = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_client() {
                        builder.optional(|builder| {
                            builder.ref_id(message.messages_id());
                        });
                    }
                }),
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_client() {
                        builder.optional(|builder| {
                            builder.mut_id(message.reader_id());
                        });
                    }
                }),
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(send);

            let send_locally_fn = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_client() {
                        builder.optional(|builder| {
                            builder.mut_id(message.from_messages_id());
                        });
                    }
                }),
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_client() {
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
                    send_fn.run_if(in_state(ClientState::Connected)),
                    send_locally_fn.run_if(in_state(ClientState::Disconnected)),
                )
                    .chain()
                    .in_set(ClientSystems::Send),
            );
        }

        if registry.has_any_shared() {
            let send_shared_fn = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_shared() {
                        builder.optional(|builder| {
                            builder.mut_id(message.messages_id());
                        });
                    }
                }),
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
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(send_shared);

            let send_shared_locally_fn = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_shared() {
                        builder.optional(|builder| {
                            builder.mut_id(message.shared_messages_id());
                        });
                    }
                }),
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_shared() {
                        builder.optional(|builder| {
                            builder.mut_id(message.messages_id());
                        });
                    }
                }),
                ParamBuilder,
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(send_shared_locally);

            app.add_systems(
                PostUpdate,
                (
                    send_shared_fn.run_if(in_state(ClientState::Connected)),
                    send_shared_locally_fn.run_if(in_state(ClientState::Disconnected)),
                )
                    .chain()
                    .in_set(ClientSystems::Send),
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
                PostUpdate,
                trigger_shared_fn
                    .after(send_shared_locally)
                    .in_set(ClientSystems::Send),
            );
        }

        if !registry.is_empty() {
            let reset_fn = (
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_client() {
                        builder.optional(|builder| {
                            builder.mut_id(message.messages_id());
                        });
                    }
                }),
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_server() {
                        builder.optional(|builder| {
                            builder.mut_id(message.queue_id());
                        });
                    }
                }),
                QueryParamBuilder::new(|builder| {
                    for message in registry.iter_all_shared() {
                        builder.optional(|builder| {
                            builder.mut_id(message.messages_id());
                        });
                    }
                }),
                ParamBuilder,
                ParamBuilder,
            )
                .build_state(app.world_mut())
                .build_system(reset);

            app.add_systems(
                OnExit(ClientState::Connected),
                reset_fn.in_set(ClientSystems::Reset),
            );
        }

        app.insert_resource(registry);
    }
}

fn send(
    messages: Query<FilteredEntityRef>,
    mut readers: Query<FilteredEntityMut>,
    mut client_messages: ResMut<ClientMessages>,
    mut storage: ResMut<ReplicationStorage>,
    type_registry: Res<AppTypeRegistry>,
    entity_map: Res<ServerEntityMap>,
    registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    let mut ctx = ClientSendCtx {
        storage: &mut storage,
        entity_map: &entity_map,
        type_registry: &type_registry,
        invalid_entities: Vec::new(),
    };

    for message in registry.iter_all_client() {
        let messages = get_resource_by_id(message.messages_id(), &messages, resource_entities)
            .expect("messages resource should be accessible");
        let mut reader =
            get_resource_entity_mut(message.reader_id(), &mut readers, resource_entities);
        let reader = reader
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.reader_id(), entity))
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

fn send_shared(
    mut messages: Query<FilteredEntityMut>,
    mut shared_messages: Query<FilteredEntityMut>,
    mut client_messages: ResMut<ClientMessages>,
    mut storage: ResMut<ReplicationStorage>,
    type_registry: Res<AppTypeRegistry>,
    entity_map: Res<ServerEntityMap>,
    registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    let mut ctx = ClientSendCtx {
        storage: &mut storage,
        entity_map: &entity_map,
        type_registry: &type_registry,
        invalid_entities: Vec::new(),
    };

    for message in registry.iter_all_shared() {
        let mut messages =
            get_resource_entity_mut(message.messages_id(), &mut messages, resource_entities);
        let messages = messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.messages_id(), entity))
            .expect("messages resource should be accessible");
        let mut shared_messages = get_resource_entity_mut(
            message.shared_messages_id(),
            &mut shared_messages,
            resource_entities,
        );
        let shared_messages = shared_messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.shared_messages_id(), entity))
            .expect("shared messages resource should be accessible");

        // SAFETY: passed pointers were obtained using this message data.
        unsafe {
            message.send(
                &mut ctx,
                messages.into_inner(),
                shared_messages.into_inner(),
                &mut client_messages,
            );
        }
    }
}

fn receive(
    mut messages: Query<FilteredEntityMut>,
    mut queues: Query<FilteredEntityMut>,
    mut client_messages: ResMut<ClientMessages>,
    mut storage: ResMut<ReplicationStorage>,
    type_registry: Res<AppTypeRegistry>,
    entity_map: Res<ServerEntityMap>,
    message_registry: Res<RemoteMessageRegistry>,
    update_tick: Res<ServerUpdateTick>,
    resource_entities: &ResourceEntities,
) {
    let mut ctx = ClientReceiveCtx {
        storage: &mut storage,
        type_registry: &type_registry,
        entity_map: &entity_map,
        invalid_entities: Vec::new(),
    };

    for message in message_registry.iter_all_server() {
        let mut messages =
            get_resource_entity_mut(message.messages_id(), &mut messages, resource_entities);
        let messages = messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.messages_id(), entity))
            .expect("messages resource should be accessible");
        let mut queue = get_resource_entity_mut(message.queue_id(), &mut queues, resource_entities);
        let queue = queue
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.queue_id(), entity))
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
    mut messages: Query<FilteredEntityMut>,
    mut commands: Commands,
    registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    for event in registry.iter_server_events() {
        let mut messages = get_resource_entity_mut(
            event.message().messages_id(),
            &mut messages,
            resource_entities,
        );
        let messages = messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(event.message().messages_id(), entity))
            .expect("messages resource should be accessible");
        event.trigger(&mut commands, messages.into_inner());
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
    mut from_messages: Query<FilteredEntityMut>,
    mut messages: Query<FilteredEntityMut>,
    registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    for message in registry.iter_all_client() {
        let mut from_messages = get_resource_entity_mut(
            message.from_messages_id(),
            &mut from_messages,
            resource_entities,
        );
        let from_messages = from_messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.from_messages_id(), entity))
            .expect("from clients messages resource should be accessible");

        let mut messages =
            get_resource_entity_mut(message.messages_id(), &mut messages, resource_entities);
        let messages = messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.messages_id(), entity))
            .expect("messages resource should be accessible");

        // SAFETY: passed pointers were obtained using this message data.
        unsafe { message.send_locally(from_messages.into_inner(), messages.into_inner()) };
    }
}

fn send_shared_locally(
    mut shared_messages: Query<FilteredEntityMut>,
    mut messages: Query<FilteredEntityMut>,
    registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    for message in registry.iter_all_shared() {
        let mut shared_messages = get_resource_entity_mut(
            message.shared_messages_id(),
            &mut shared_messages,
            resource_entities,
        );
        let shared_messages = shared_messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.shared_messages_id(), entity))
            .expect("shared messages resource should be accessible");

        let mut messages =
            get_resource_entity_mut(message.messages_id(), &mut messages, resource_entities);
        let messages = messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.messages_id(), entity))
            .expect("messages resource should be accessible");

        // SAFETY: passed pointers were obtained using this message data.
        unsafe { message.send_locally(shared_messages.into_inner(), messages.into_inner()) };
    }
}

fn reset(
    mut messages: Query<FilteredEntityMut>,
    mut queues: Query<FilteredEntityMut>,
    mut shared_messages: Query<FilteredEntityMut>,
    registry: Res<RemoteMessageRegistry>,
    resource_entities: &ResourceEntities,
) {
    for message in registry.iter_all_client() {
        let mut messages =
            get_resource_entity_mut(message.messages_id(), &mut messages, resource_entities);
        let messages = messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.messages_id(), entity))
            .expect("messages resource should be accessible");

        // SAFETY: passed pointer was obtained using this message data.
        unsafe { message.reset(messages.into_inner()) };
    }

    for message in registry.iter_all_server() {
        let mut queue = get_resource_entity_mut(message.queue_id(), &mut queues, resource_entities);
        let queue = queue
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.queue_id(), entity))
            .expect("queue resource should be accessible");

        // SAFETY: passed pointer was obtained using this message data.
        unsafe { message.reset(queue.into_inner()) };
    }

    for message in registry.iter_all_shared() {
        let mut shared_messages = get_resource_entity_mut(
            message.messages_id(),
            &mut shared_messages,
            resource_entities,
        );
        let shared_messages = shared_messages
            .as_mut()
            .and_then(|entity| get_resource_mut_by_id(message.messages_id(), entity))
            .expect("shared messages resource should be accessible");

        // SAFETY: passed pointer was obtained using this message data.
        unsafe { message.reset(shared_messages.into_inner()) };
    }
}
