use core::any::TypeId;

use bevy::{ecs::entity::MapEntities, prelude::*, ptr::PtrMut};
use log::debug;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    broadcast_message::SharedMessage,
    client_message,
    ctx::{ClientSendCtx, ServerReceiveCtx},
    message_fns::{DeserializeFn, MessageFns, SerializeFn},
    registry::RemoteMessageRegistry,
};
use crate::prelude::*;

/// An extension trait for [`App`] for creating shared events.
///
/// They're like client events, but also triggered locally in the same way as on the server.
///
/// See also [`SharedMessageAppExt`] for messages, [`ClientEventAppExt`] for regular client events
/// and [`ServerEventAppExt`] for server events.
pub trait SharedEventAppExt {
    /// Registers a remote event that can be triggered using [`SharedTriggerExt::shared_trigger`].
    ///
    /// After triggering `E` event on the client, [`LocalOrRemote<E>`] event will be triggered locally
    /// with [`Sender::Local`] and on the server with [`Sender::Remote`].
    ///
    /// On a listen server, the event will be triggered locally with [`Sender::Local`].
    fn add_shared_event<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_shared_event_with(
            channel,
            client_message::default_serialize::<E>,
            client_message::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_shared_event`], but additionally maps client entities to server inside the event before sending.
    ///
    /// Always use it for events that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    fn add_mapped_shared_event<E: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_shared_event_with(
            channel,
            client_message::default_serialize_mapped::<E>,
            client_message::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_shared_event`], but uses the specified functions for serialization and deserialization.
    ///
    /// See also [`SharedMessageAppExt::add_shared_message_with`].
    fn add_shared_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self;
}

impl SharedEventAppExt for App {
    fn add_shared_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_shared_event::<E>();

        let fns = MessageFns::new(serialize, deserialize).with_convert::<SharedEventMessage<E>>();
        let event = SharedEvent::new(self, channel, fns);
        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        registry.register_shared_event(event);

        self
    }
}

/// Small abstraction on top of [`SharedMessage`] that stores a function to trigger them.
pub(crate) struct SharedEvent {
    type_id: TypeId,
    message: SharedMessage,
    trigger: TriggerFn,
}

impl SharedEvent {
    fn new<E: Event>(
        app: &mut App,
        channel: Channel,
        fns: MessageFns<ClientSendCtx, ServerReceiveCtx, SharedEventMessage<E>, E>,
    ) -> Self {
        Self {
            type_id: TypeId::of::<E>(),
            message: SharedMessage::new(app, channel, fns),
            trigger: Self::trigger_typed::<E>,
        }
    }

    /// Drains received [`LocalOrRemote<SharedEventMessage<E>>`] messages and triggers them as [`LocalOrRemote<E>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `shared_messages` is [`Messages<LocalOrRemote<SharedEventMessage<E>>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn trigger(&self, commands: &mut Commands, shared_messages: PtrMut) {
        unsafe { (self.trigger)(commands, shared_messages) }
    }

    /// Typed version of [`Self::trigger`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `shared_messages` is [`Messages<LocalOrRemote<SharedEventMessage<E>>>`].
    unsafe fn trigger_typed<E: Event>(commands: &mut Commands, shared_messages: PtrMut) {
        let shared_messages: &mut Messages<LocalOrRemote<SharedEventMessage<E>>> =
            unsafe { shared_messages.deref_mut() };
        for LocalOrRemote { sender, message } in shared_messages.drain() {
            debug!(
                "triggering `{}` from `{sender:?}`",
                ShortName::of::<LocalOrRemote<E>>()
            );
            commands.trigger(LocalOrRemote {
                sender,
                message: message.event,
            });
        }
    }

    pub(super) fn type_id(&self) -> TypeId {
        self.type_id
    }

    pub(crate) fn message(&self) -> &SharedMessage {
        &self.message
    }
}

/// Signature of shared event trigger functions.
type TriggerFn = unsafe fn(&mut Commands, PtrMut);

/// Extension trait for triggering shared events.
///
/// See also [`SharedEventAppExt`].
pub trait SharedTriggerExt {
    /// Like [`Commands::trigger`], but triggers [`LocalOrRemote<E>`] locally and on the server.
    fn shared_trigger(&mut self, event: impl Event);
}

impl SharedTriggerExt for Commands<'_, '_> {
    fn shared_trigger(&mut self, event: impl Event) {
        self.write_message(SharedEventMessage { event });
    }
}

impl SharedTriggerExt for World {
    fn shared_trigger(&mut self, event: impl Event) {
        self.write_message(SharedEventMessage { event });
    }
}

/// A message that used under the hood for shared events.
///
/// Events are implemented through messages in order to reuse their logic.
/// So we send this message instead and, after receiving it, drain it to trigger regular events.
/// This also allows us to avoid requiring [`Clone`] because events can't be drained.
#[derive(Message)]
struct SharedEventMessage<E> {
    event: E,
}

impl<E> From<E> for SharedEventMessage<E> {
    fn from(event: E) -> Self {
        Self { event }
    }
}

impl<E> AsRef<E> for SharedEventMessage<E> {
    fn as_ref(&self) -> &E {
        &self.event
    }
}
