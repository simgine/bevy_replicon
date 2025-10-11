use core::any::TypeId;

use bevy::{ecs::entity::MapEntities, prelude::*, ptr::PtrMut};
use log::debug;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    client_message::{self, ClientMessage},
    ctx::{ClientSendCtx, ServerReceiveCtx},
    message_fns::{DeserializeFn, MessageFns, SerializeFn},
    registry::RemoteMessageRegistry,
};
use crate::prelude::*;

/// An extension trait for [`App`] for creating client events.
///
/// See also [`ClientTriggerExt`] for triggering, [`ServerEventAppExt`] for server events
/// and [`ClientMessageAppExt`] for messages.
pub trait ClientEventAppExt {
    /// Registers a remote event that can be triggered using [`ClientTriggerExt::client_trigger`].
    ///
    /// After triggering `E` event on the client, [`FromClient<E>`] event will be triggered on the server.
    ///
    /// If [`ServerMessagePlugin`] is enabled and the client state is [`ClientState::Disconnected`], the event will also be triggered
    /// locally as [`FromClient<E>`] event with [`FromClient::client_id`] equal to [`ClientId::Server`].
    ///
    /// See also the [corresponding section](../index.html#from-client-to-server) from the quick start guide.
    fn add_client_event<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_client_event_with(
            channel,
            client_message::default_serialize::<E>,
            client_message::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_client_event`], but additionally maps client entities to server inside the event before sending.
    ///
    /// Always use it for events that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    fn add_mapped_client_event<E: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_client_event_with(
            channel,
            client_message::default_serialize_mapped::<E>,
            client_message::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_client_event`], but uses the specified functions for serialization and deserialization.
    ///
    /// See also [`ClientMessageAppExt::add_client_message_with`].
    fn add_client_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self;
}

impl ClientEventAppExt for App {
    fn add_client_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_client_event::<E>();

        let fns = MessageFns::new(serialize, deserialize).with_convert::<ClientMessageEvent<E>>();
        let event = ClientEvent::new(self, channel, fns);
        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        registry.register_client_event(event);

        self
    }
}

/// Small abstraction on top of [`ClientEvent`] that stores a function to trigger them.
pub(crate) struct ClientEvent {
    type_id: TypeId,
    message: ClientMessage,
    trigger: TriggerFn,
}

impl ClientEvent {
    fn new<E: Event>(
        app: &mut App,
        channel: Channel,
        fns: MessageFns<ClientSendCtx, ServerReceiveCtx, ClientMessageEvent<E>, E>,
    ) -> Self {
        Self {
            type_id: TypeId::of::<E>(),
            message: ClientMessage::new(app, channel, fns),
            trigger: Self::trigger_typed::<E>,
        }
    }

    /// Drains received [`FromClient<TriggerEvent<E>>`] messages and triggers them as [`FromClient<E>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<FromClient<ClientMessageEvent<E>>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn trigger(&self, commands: &mut Commands, from_messages: PtrMut) {
        unsafe { (self.trigger)(commands, from_messages) }
    }

    /// Typed version of [`Self::trigger`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<FromClient<ClientMessageEvent<E>>>`].
    unsafe fn trigger_typed<E: Event>(commands: &mut Commands, from_messages: PtrMut) {
        let from_messages: &mut Messages<FromClient<ClientMessageEvent<E>>> =
            unsafe { from_messages.deref_mut() };
        for FromClient { client_id, message } in from_messages.drain() {
            debug!(
                "triggering `{}` from `{client_id}`",
                ShortName::of::<FromClient<E>>()
            );
            commands.trigger(FromClient {
                client_id,
                message: message.event,
            });
        }
    }

    pub(super) fn type_id(&self) -> TypeId {
        self.type_id
    }

    pub(crate) fn message(&self) -> &ClientMessage {
        &self.message
    }
}

/// Signature of client trigger functions.
type TriggerFn = unsafe fn(&mut Commands, PtrMut);

/// Extension trait for triggering client events.
///
/// See also [`ClientEventAppExt`].
pub trait ClientTriggerExt {
    /// Like [`Commands::trigger`], but triggers [`FromClient`] on server and locally if the client state is [`ClientState::Disconnected`].
    fn client_trigger(&mut self, event: impl Event);
}

impl ClientTriggerExt for Commands<'_, '_> {
    fn client_trigger(&mut self, event: impl Event) {
        self.write_message(ClientMessageEvent { event });
    }
}

impl ClientTriggerExt for World {
    fn client_trigger(&mut self, event: impl Event) {
        self.write_message(ClientMessageEvent { event });
    }
}

/// A message that used under the hood for client events.
///
/// Events are implemented through messages in order to reuse their logic.
/// So we send this message instead and, after receiving it, drain it to trigger regular events.
/// This also allows us to avoid requiring [`Clone`] because events can't be drained.
#[derive(Message)]
struct ClientMessageEvent<E> {
    event: E,
}

impl<E> From<E> for ClientMessageEvent<E> {
    fn from(event: E) -> Self {
        Self { event }
    }
}

impl<E> AsRef<E> for ClientMessageEvent<E> {
    fn as_ref(&self) -> &E {
        &self.event
    }
}
