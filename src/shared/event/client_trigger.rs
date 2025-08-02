use core::any;

use bevy::{ecs::entity::MapEntities, prelude::*, ptr::PtrMut};
use bytes::Bytes;
use log::debug;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    client_event::{self, ClientEvent},
    ctx::{ClientSendCtx, ServerReceiveCtx},
    event_fns::{EventDeserializeFn, EventFns, EventSerializeFn},
    remote_event_registry::RemoteEventRegistry,
    remote_targets::RemoteTargets,
};
use crate::{
    prelude::*,
    shared::{entity_serde, postcard_utils},
};

/// An extension trait for [`App`] for creating client triggers.
///
/// See also [`ClientTriggerExt`] for triggering, [`ServerTriggerAppExt`] for server triggers
/// and [`ClientEventAppExt`] for events.
pub trait ClientTriggerAppExt {
    /// Registers a remote event that can be triggered using [`ClientTriggerExt::client_trigger`].
    ///
    /// After triggering `E` event on the client, [`FromClient<E>`] event will be triggered on the server.
    ///
    /// If [`ServerEventPlugin`] is enabled and [`RepliconClient`] is inactive, the event will also be triggered
    /// locally as [`FromClient<E>`] event with [`FromClient::client`] equal to [`SERVER`].
    ///
    /// See also the [corresponding section](../index.html#from-client-to-server) from the quick start guide.
    fn add_client_trigger<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_client_trigger_with(
            channel,
            client_event::default_serialize::<E>,
            client_event::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_client_trigger`], but additionally maps client entities to server inside the event before sending.
    ///
    /// Always use it for events that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    fn add_mapped_client_trigger<E: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_client_trigger_with(
            channel,
            client_event::default_serialize_mapped::<E>,
            client_event::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_client_trigger`], but uses the specified functions for serialization and deserialization.
    ///
    /// See also [`ClientEventAppExt::add_client_event_with`].
    fn add_client_trigger_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: EventSerializeFn<ClientSendCtx, E>,
        deserialize: EventDeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self;
}

impl ClientTriggerAppExt for App {
    fn add_client_trigger_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: EventSerializeFn<ClientSendCtx, E>,
        deserialize: EventDeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_client_trigger::<E>();

        let event_fns = EventFns::new(serialize, deserialize)
            .with_outer(trigger_serialize, trigger_deserialize);

        let trigger = ClientTrigger::new(self, channel, event_fns);
        let mut event_registry = self.world_mut().resource_mut::<RemoteEventRegistry>();
        event_registry.register_client_trigger(trigger);

        self
    }
}

/// Small abstraction on top of [`ClientEvent`] that stores a function to trigger them.
pub(crate) struct ClientTrigger {
    event: ClientEvent,
    trigger: TriggerFn,
}

impl ClientTrigger {
    fn new<E: Event>(
        app: &mut App,
        channel: Channel,
        event_fns: EventFns<ClientSendCtx, ServerReceiveCtx, ClientTriggerEvent<E>, E>,
    ) -> Self {
        Self {
            event: ClientEvent::new(app, channel, event_fns),
            trigger: Self::trigger_typed::<E>,
        }
    }

    pub(crate) fn trigger(&self, commands: &mut Commands, events: PtrMut) {
        unsafe {
            (self.trigger)(commands, events);
        }
    }

    /// Drains received [`FromClient<TriggerEvent<E>>`] events and triggers them as [`FromClient<E>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `client_events` is [`Events<FromClient<TriggerEvent<E>>>`]
    /// and this instance was created for `E`.
    unsafe fn trigger_typed<E: Event>(commands: &mut Commands, client_events: PtrMut) {
        let client_events: &mut Events<FromClient<ClientTriggerEvent<E>>> =
            unsafe { client_events.deref_mut() };
        for FromClient { client_id, event } in client_events.drain() {
            debug!(
                "triggering `{}` from `{client_id}`",
                any::type_name::<FromClient<E>>()
            );
            commands.trigger_targets(
                FromClient {
                    client_id,
                    event: event.event,
                },
                event.targets,
            );
        }
    }

    pub(crate) fn event(&self) -> &ClientEvent {
        &self.event
    }
}

/// Signature of client trigger functions.
type TriggerFn = unsafe fn(&mut Commands, PtrMut);

/// Serializes targets for [`TriggerEvent`], maps them and delegates the event
/// serialiaztion to `serialize`.
///
/// Used as outer function for [`EventFns`].
fn trigger_serialize<'a, E>(
    ctx: &mut ClientSendCtx<'a>,
    trigger: &ClientTriggerEvent<E>,
    message: &mut Vec<u8>,
    serialize: EventSerializeFn<ClientSendCtx<'a>, E>,
) -> Result<()> {
    postcard_utils::to_extend_mut(&trigger.targets.len(), message)?;
    for &entity in &trigger.targets {
        let entity = ctx.get_mapped(entity);
        entity_serde::serialize_entity(message, entity)?;
    }

    (serialize)(ctx, &trigger.event, message)
}

/// Deserializes targets for [`TriggerEvent`], maps them and delegates the event
/// deserialiaztion to `deserialize`.
///
/// Used as outer function for [`EventFns`].
fn trigger_deserialize<'a, E>(
    ctx: &mut ServerReceiveCtx<'a>,
    message: &mut Bytes,
    deserialize: EventDeserializeFn<ServerReceiveCtx<'a>, E>,
) -> Result<ClientTriggerEvent<E>> {
    let len = postcard_utils::from_buf(message)?;
    let mut targets = Vec::with_capacity(len);
    for _ in 0..len {
        let entity = entity_serde::deserialize_entity(message)?;
        targets.push(entity);
    }

    let event = (deserialize)(ctx, message)?;

    Ok(ClientTriggerEvent { event, targets })
}

/// Extension trait for triggering client events.
///
/// See also [`ClientTriggerAppExt`].
pub trait ClientTriggerExt {
    /// Like [`Commands::trigger`], but triggers [`FromClient`] on server and locally if [`RepliconClient`] is inactive.
    fn client_trigger(&mut self, event: impl Event);

    /// Like [`Self::client_trigger`], but allows you to specify target entities, similar to [`Commands::trigger_targets`].
    fn client_trigger_targets(&mut self, event: impl Event, targets: impl RemoteTargets);
}

impl ClientTriggerExt for Commands<'_, '_> {
    fn client_trigger(&mut self, event: impl Event) {
        self.client_trigger_targets(event, []);
    }

    fn client_trigger_targets(&mut self, event: impl Event, targets: impl RemoteTargets) {
        self.send_event(ClientTriggerEvent {
            event,
            targets: targets.into_entities(),
        });
    }
}

impl ClientTriggerExt for World {
    fn client_trigger(&mut self, event: impl Event) {
        self.client_trigger_targets(event, []);
    }

    fn client_trigger_targets(&mut self, event: impl Event, targets: impl RemoteTargets) {
        self.send_event(ClientTriggerEvent {
            event,
            targets: targets.into_entities(),
        });
    }
}

/// An event that used under the hood for client triggers.
///
/// We can't just observe for triggers like we do for events since we need access to all its targets
/// and we need to buffer them. This is why we just emit this event instead and after receive drain it
/// to trigger regular events.
#[derive(Event)]
struct ClientTriggerEvent<E> {
    event: E,
    targets: Vec<Entity>,
}
