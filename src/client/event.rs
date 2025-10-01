use bevy::{
    ecs::system::{FilteredResourcesMutParamBuilder, FilteredResourcesParamBuilder, ParamBuilder},
    prelude::*,
};

use super::ServerUpdateTick;
use crate::{
    prelude::*,
    shared::{
        event::{
            ctx::{ClientReceiveCtx, ClientSendCtx},
            registry::RemoteEventRegistry,
        },
        server_entity_map::ServerEntityMap,
    },
};

/// Sending events from a client to the server.
///
/// Requires [`ClientPlugin`].
/// Can be disabled for apps that act only as servers.
pub struct ClientEventPlugin;

impl Plugin for ClientEventPlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        // Construct systems dynamically after all plugins initialization
        // because we need to access resources by registered IDs.
        let event_registry = app
            .world_mut()
            .remove_resource::<RemoteEventRegistry>()
            .expect("event registry should be initialized on app build");

        let send_fn = (
            FilteredResourcesParamBuilder::new(|builder| {
                for event in event_registry.iter_all_client() {
                    builder.add_read_by_id(event.events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_all_client() {
                    builder.add_write_by_id(event.reader_id());
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
                for event in event_registry.iter_all_server() {
                    builder.add_write_by_id(event.events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_all_server() {
                    builder.add_write_by_id(event.queue_id());
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
                for trigger in event_registry.iter_server_triggers() {
                    builder.add_write_by_id(trigger.event().events_id());
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

        let resend_locally_fn = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_all_client() {
                    builder.add_write_by_id(event.client_events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_all_client() {
                    builder.add_write_by_id(event.events_id());
                }
            }),
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(resend_locally);

        let reset_fn = (
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_all_client() {
                    builder.add_write_by_id(event.events_id());
                }
            }),
            FilteredResourcesMutParamBuilder::new(|builder| {
                for event in event_registry.iter_all_server() {
                    builder.add_write_by_id(event.queue_id());
                }
            }),
            ParamBuilder,
        )
            .build_state(app.world_mut())
            .build_system(reset);

        app.insert_resource(event_registry)
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
                    resend_locally_fn.run_if(in_state(ClientState::Disconnected)),
                )
                    .chain()
                    .in_set(ClientSystems::Send),
            );
    }
}

fn send(
    events: FilteredResources,
    mut readers: FilteredResourcesMut,
    mut messages: ResMut<ClientMessages>,
    type_registry: Res<AppTypeRegistry>,
    entity_map: Res<ServerEntityMap>,
    event_registry: Res<RemoteEventRegistry>,
) {
    let mut ctx = ClientSendCtx {
        entity_map: &entity_map,
        type_registry: &type_registry,
        invalid_entities: Vec::new(),
    };

    for event in event_registry.iter_all_client() {
        let events = events
            .get_by_id(event.events_id())
            .expect("events resource should be accessible");
        let reader = readers
            .get_mut_by_id(event.reader_id())
            .expect("event reader resource should be accessible");

        // SAFETY: passed pointers were obtained using this event data.
        unsafe {
            event.send(&mut ctx, &events, reader.into_inner(), &mut messages);
        }
    }
}

fn receive(
    mut events: FilteredResourcesMut,
    mut queues: FilteredResourcesMut,
    mut messages: ResMut<ClientMessages>,
    type_registry: Res<AppTypeRegistry>,
    entity_map: Res<ServerEntityMap>,
    event_registry: Res<RemoteEventRegistry>,
    update_tick: Res<ServerUpdateTick>,
) {
    let mut ctx = ClientReceiveCtx {
        type_registry: &type_registry,
        entity_map: &entity_map,
        invalid_entities: Vec::new(),
    };

    for event in event_registry.iter_all_server() {
        let events = events
            .get_mut_by_id(event.events_id())
            .expect("events resource should be accessible");
        let queue = queues
            .get_mut_by_id(event.queue_id())
            .expect("queue resource should be accessible");

        // SAFETY: passed pointers were obtained using this event data.
        unsafe {
            event.receive(
                &mut ctx,
                events.into_inner(),
                queue.into_inner(),
                &mut messages,
                **update_tick,
            )
        };
    }
}

fn trigger(
    mut events: FilteredResourcesMut,
    mut commands: Commands,
    event_registry: Res<RemoteEventRegistry>,
) {
    for trigger in event_registry.iter_server_triggers() {
        let events = events
            .get_mut_by_id(trigger.event().events_id())
            .expect("events resource should be accessible");
        trigger.trigger(&mut commands, events.into_inner());
    }
}

fn resend_locally(
    mut client_events: FilteredResourcesMut,
    mut events: FilteredResourcesMut,
    event_registry: Res<RemoteEventRegistry>,
) {
    for event in event_registry.iter_all_client() {
        let client_events = client_events
            .get_mut_by_id(event.client_events_id())
            .expect("client events resource should be accessible");
        let events = events
            .get_mut_by_id(event.events_id())
            .expect("events resource should be accessible");

        // SAFETY: passed pointers were obtained using this event data.
        unsafe { event.resend_locally(client_events.into_inner(), events.into_inner()) };
    }
}

fn reset(
    mut events: FilteredResourcesMut,
    mut queues: FilteredResourcesMut,
    event_registry: Res<RemoteEventRegistry>,
) {
    for event in event_registry.iter_all_client() {
        let events = events
            .get_mut_by_id(event.events_id())
            .expect("events resource should be accessible");

        // SAFETY: passed pointer was obtained using this event data.
        unsafe { event.reset(events.into_inner()) };
    }

    for event in event_registry.iter_all_server() {
        let queue = queues
            .get_mut_by_id(event.queue_id())
            .expect("event queue resource should be accessible");

        // SAFETY: passed pointer was obtained using this event data.
        unsafe { event.reset(queue.into_inner()) };
    }
}
