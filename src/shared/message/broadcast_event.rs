use core::any::TypeId;

use bevy::{ecs::entity::MapEntities, prelude::*, ptr::PtrMut};
use log::debug;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    broadcast_message::BroadcastMessage,
    client_message,
    ctx::{ClientSendCtx, ServerReceiveCtx},
    message_fns::{DeserializeFn, MessageFns, SerializeFn},
    registry::RemoteMessageRegistry,
};
use crate::prelude::*;

/// An extension trait for [`App`] for creating broadcast events.
///
/// See also [`BroadcastMessageAppExt`] for messages, [`ClientEventAppExt`] for regular client events
/// and [`ServerEventAppExt`] for server events.
pub trait BroadcastEventAppExt {
    /// Registers a remote event that can be triggered using [`BroadcastTriggerExt::broadcast_trigger`].
    ///
    /// After triggering `E` event on the client, [`Broadcast<E>`] event will be triggered locally
    /// with [`Broadcaster::Local`] and on the server with [`Broadcaster::Remote`].
    ///
    /// On a listen server, the event will be triggered locally with [`Broadcaster::Local`].
    fn add_broadcast_event<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_broadcast_event_with(
            channel,
            client_message::default_serialize::<E>,
            client_message::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_broadcast_event`], but additionally maps client entities to server inside the event before sending.
    ///
    /// Always use it for events that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    fn add_mapped_broadcast_event<E: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_broadcast_event_with(
            channel,
            client_message::default_serialize_mapped::<E>,
            client_message::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_broadcast_event`], but uses the specified functions for serialization and deserialization.
    ///
    /// See also [`BroadcastMessageAppExt::add_broadcast_message_with`].
    fn add_broadcast_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self;
}

impl BroadcastEventAppExt for App {
    fn add_broadcast_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_broadcast_event::<E>();

        let fns =
            MessageFns::new(serialize, deserialize).with_convert::<BroadcastEventMessage<E>>();
        let event = BroadcastEvent::new(self, channel, fns);
        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        registry.register_broadcast_event(event);

        self
    }
}

/// Small abstraction on top of [`BroadcastMessage`] that stores a function to trigger them.
pub(crate) struct BroadcastEvent {
    type_id: TypeId,
    message: BroadcastMessage,
    trigger: TriggerFn,
}

impl BroadcastEvent {
    fn new<E: Event>(
        app: &mut App,
        channel: Channel,
        fns: MessageFns<ClientSendCtx, ServerReceiveCtx, BroadcastEventMessage<E>, E>,
    ) -> Self {
        Self {
            type_id: TypeId::of::<E>(),
            message: BroadcastMessage::new(app, channel, fns),
            trigger: Self::trigger_typed::<E>,
        }
    }

    /// Drains received [`Broadcast<BroadcastEventMessage<E>>`] messages and triggers them as [`Broadcast<E>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `broadcasts` is [`Messages<Broadcast<BroadcastEventMessage<E>>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn trigger(&self, commands: &mut Commands, broadcasts: PtrMut) {
        unsafe { (self.trigger)(commands, broadcasts) }
    }

    /// Typed version of [`Self::trigger`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `broadcasts` is [`Messages<Broadcast<BroadcastEventMessage<E>>>`].
    unsafe fn trigger_typed<E: Event>(commands: &mut Commands, broadcasts: PtrMut) {
        let broadcasts: &mut Messages<Broadcast<BroadcastEventMessage<E>>> =
            unsafe { broadcasts.deref_mut() };
        for Broadcast {
            broadcaster,
            message,
        } in broadcasts.drain()
        {
            debug!(
                "triggering `{}` from `{broadcaster:?}`",
                ShortName::of::<Broadcast<E>>()
            );
            commands.trigger(Broadcast {
                broadcaster,
                message: message.event,
            });
        }
    }

    pub(super) fn type_id(&self) -> TypeId {
        self.type_id
    }

    pub(crate) fn message(&self) -> &BroadcastMessage {
        &self.message
    }
}

/// Signature of broadcast trigger functions.
type TriggerFn = unsafe fn(&mut Commands, PtrMut);

/// Extension trait for triggering broadcast events.
///
/// See also [`BroadcastEventAppExt`].
pub trait BroadcastTriggerExt {
    /// Like [`Commands::trigger`], but triggers [`Broadcast<E>`] locally and on the server.
    fn broadcast_trigger(&mut self, event: impl Event);
}

impl BroadcastTriggerExt for Commands<'_, '_> {
    fn broadcast_trigger(&mut self, event: impl Event) {
        self.write_message(BroadcastEventMessage { event });
    }
}

impl BroadcastTriggerExt for World {
    fn broadcast_trigger(&mut self, event: impl Event) {
        self.write_message(BroadcastEventMessage { event });
    }
}

/// A message that used under the hood for broadcast events.
///
/// Events are implemented through messages in order to reuse their logic.
/// So we send this message instead and, after receiving it, drain it to trigger regular events.
/// This also allows us to avoid requiring [`Clone`] because events can't be drained.
#[derive(Message)]
struct BroadcastEventMessage<E> {
    event: E,
}

impl<E> From<E> for BroadcastEventMessage<E> {
    fn from(event: E) -> Self {
        Self { event }
    }
}

impl<E> AsRef<E> for BroadcastEventMessage<E> {
    fn as_ref(&self) -> &E {
        &self.event
    }
}
