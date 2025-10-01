use core::any;

use bevy::{ecs::entity::MapEntities, prelude::*, ptr::PtrMut};
use log::debug;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    client_message::{self, ClientEvent},
    ctx::{ClientSendCtx, ServerReceiveCtx},
    message_fns::{DeserializeFn, EventFns, SerializeFn},
    registry::RemoteEventRegistry,
};
use crate::prelude::*;

/// An extension trait for [`App`] for creating client triggers.
///
/// See also [`ClientTriggerExt`] for triggering, [`ServerTriggerAppExt`] for server triggers
/// and [`ClientEventAppExt`] for events.
pub trait ClientTriggerAppExt {
    /// Registers a remote event that can be triggered using [`ClientTriggerExt::client_trigger`].
    ///
    /// After triggering `E` event on the client, [`FromClient<E>`] event will be triggered on the server.
    ///
    /// If [`ServerEventPlugin`] is enabled and the client state is [`ClientState::Disconnected`], the event will also be triggered
    /// locally as [`FromClient<E>`] event with [`FromClient::client_id`] equal to [`ClientId::Server`].
    ///
    /// See also the [corresponding section](../index.html#from-client-to-server) from the quick start guide.
    fn add_client_trigger<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_client_trigger_with(
            channel,
            client_message::default_serialize::<E>,
            client_message::default_deserialize::<E>,
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
            client_message::default_serialize_mapped::<E>,
            client_message::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_client_trigger`], but uses the specified functions for serialization and deserialization.
    ///
    /// See also [`ClientEventAppExt::add_client_event_with`].
    fn add_client_trigger_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self;
}

impl ClientTriggerAppExt for App {
    fn add_client_trigger_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_client_event::<E>();

        let event_fns =
            EventFns::new(serialize, deserialize).with_convert::<ClientTriggerEvent<E>>();

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
        let client_events: &mut Messages<FromClient<ClientTriggerEvent<E>>> =
            unsafe { client_events.deref_mut() };
        for FromClient { client_id, event } in client_events.drain() {
            debug!(
                "triggering `{}` from `{client_id}`",
                any::type_name::<FromClient<E>>()
            );
            commands.trigger(FromClient {
                client_id,
                event: event.event,
            });
        }
    }

    pub(crate) fn event(&self) -> &ClientEvent {
        &self.event
    }
}

/// Signature of client trigger functions.
type TriggerFn = unsafe fn(&mut Commands, PtrMut);

/// Extension trait for triggering client events.
///
/// See also [`ClientTriggerAppExt`].
pub trait ClientTriggerExt {
    /// Like [`Commands::trigger`], but triggers [`FromClient`] on server and locally if the client state is [`ClientState::Disconnected`].
    fn client_trigger(&mut self, event: impl Event);
}

impl ClientTriggerExt for Commands<'_, '_> {
    fn client_trigger(&mut self, event: impl Event) {
        self.write_message(ClientTriggerEvent { event });
    }
}

impl ClientTriggerExt for World {
    fn client_trigger(&mut self, event: impl Event) {
        self.write_message(ClientTriggerEvent { event });
    }
}

/// An event that used under the hood for client triggers.
///
/// We can't just observe for triggers like we do for events since we need access to all its targets
/// and we need to buffer them. This is why we just emit this event instead and after receive drain it
/// to trigger regular events.
#[derive(Message)]
struct ClientTriggerEvent<E> {
    event: E,
}

impl<E> From<E> for ClientTriggerEvent<E> {
    fn from(event: E) -> Self {
        Self { event }
    }
}

impl<E> AsRef<E> for ClientTriggerEvent<E> {
    fn as_ref(&self) -> &E {
        &self.event
    }
}
