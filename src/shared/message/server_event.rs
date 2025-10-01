use core::any;

use bevy::{ecs::entity::MapEntities, prelude::*, ptr::PtrMut};
use log::debug;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    ctx::{ClientReceiveCtx, ServerSendCtx},
    message_fns::{DeserializeFn, MessageFns, SerializeFn},
    registry::RemoteMessageRegistry,
    server_message::{self, ServerMessage},
};
use crate::prelude::*;

/// An extension trait for [`App`] for creating server events.
///
/// See also [`ServerTriggerExt`] for triggering, [`ClientEventAppExt`] for client events
/// and [`ServerMessageAppExt`] for messages.
pub trait ServerEventAppExt {
    /// Registers a remote event that can be triggered using [`ServerTriggerExt::server_trigger`].
    ///
    /// After triggering [`ToClients<E>`] event on the server, `E` event will be triggered on clients.
    ///
    /// If [`ClientMessagePlugin`] is enabled and [`ClientId::Server`] is a recipient of the event,
    /// then `E` event will be emitted on the server as well.
    ///
    /// See also the [corresponding section](../index.html#from-client-to-server) from the quick start guide.
    fn add_server_event<'a, E>(&mut self, channel: Channel) -> &mut Self
    where
        E: Event<Trigger<'a>: Default> + Serialize + DeserializeOwned,
    {
        self.add_server_event_with(
            channel,
            server_message::default_serialize::<E>,
            server_message::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_server_event`], but additionally maps client entities to server inside the event before receiving.
    ///
    /// Always use it for events that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    fn add_mapped_server_event<'a, E>(&mut self, channel: Channel) -> &mut Self
    where
        E: Event<Trigger<'a>: Default> + Serialize + DeserializeOwned + MapEntities,
    {
        self.add_server_event_with(
            channel,
            server_message::default_serialize::<E>,
            server_message::default_deserialize_mapped::<E>,
        )
    }

    /// Same as [`Self::add_server_event`], but uses the specified functions for serialization and deserialization.
    ///
    /// See also [`ServerMessageAppExt::add_server_message_with`].
    fn add_server_event_with<'a, E: Event<Trigger<'a>: Default>>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ServerSendCtx, E>,
        deserialize: DeserializeFn<ClientReceiveCtx, E>,
    ) -> &mut Self;

    /// Like [`ServerMessageAppExt::make_message_independent`], but for triggers.
    fn make_event_independent<E: Event>(&mut self) -> &mut Self;
}

impl ServerEventAppExt for App {
    fn add_server_event_with<'a, E: Event<Trigger<'a>: Default>>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ServerSendCtx, E>,
        deserialize: DeserializeFn<ClientReceiveCtx, E>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_server_event::<E>();

        let fns = MessageFns::new(serialize, deserialize).with_convert::<ServerTriggerEvent<E>>();
        let event = ServerEvent::new(self, channel, fns);
        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        registry.register_server_event(event);

        self
    }

    fn make_event_independent<E: Event>(&mut self) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .make_event_independent::<E>();

        let messages_id = self
            .world()
            .components()
            .resource_id::<Messages<ServerTriggerEvent<E>>>()
            .unwrap_or_else(|| {
                panic!(
                    "event `{}` should be previously registered",
                    any::type_name::<E>()
                )
            });

        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        let event = registry
            .iter_server_events_mut()
            .find(|e| e.message().messages_id() == messages_id)
            .unwrap_or_else(|| {
                panic!(
                    "message `{}` should be previously registered as a server message",
                    any::type_name::<E>()
                )
            });

        event.message_mut().independent = true;

        self
    }
}

/// Small abstraction on top of [`ServerEvent`] that stores a function to trigger them.
pub(crate) struct ServerEvent {
    message: ServerMessage,
    trigger: TriggerFn,
}

impl ServerEvent {
    fn new<'a, E: Event<Trigger<'a>: Default>>(
        app: &mut App,
        channel: Channel,
        fns: MessageFns<ServerSendCtx, ClientReceiveCtx, ServerTriggerEvent<E>, E>,
    ) -> Self {
        Self {
            message: ServerMessage::new(app, channel, fns),
            trigger: Self::trigger_typed::<E>,
        }
    }

    /// Drains received [`ServerTriggerEvent<E>`] messages and triggers them as `E`.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<ServerTriggerEvent<E>>`]
    /// and this instance was created for `E`.
    pub(crate) fn trigger(&self, commands: &mut Commands, messages: PtrMut) {
        unsafe { (self.trigger)(commands, messages) }
    }

    /// Typed version of [`Self::trigger`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<ServerTriggerEvent<E>>`].
    unsafe fn trigger_typed<'a, E: Event<Trigger<'a>: Default>>(
        commands: &mut Commands,
        messages: PtrMut,
    ) {
        let messages: &mut Messages<ServerTriggerEvent<E>> = unsafe { messages.deref_mut() };
        for message in messages.drain() {
            debug!("triggering `{}`", any::type_name::<E>());
            commands.trigger(message.event);
        }
    }

    pub(crate) fn message(&self) -> &ServerMessage {
        &self.message
    }

    pub(super) fn message_mut(&mut self) -> &mut ServerMessage {
        &mut self.message
    }
}

/// Signature of server trigger functions.
type TriggerFn = unsafe fn(&mut Commands, PtrMut);

/// Extension trait for triggering server events.
///
/// See also [`ServerEventAppExt`].
pub trait ServerTriggerExt {
    /// Like [`Commands::trigger`], but triggers `E` on server and locally
    /// if [`ClientId::Server`] is a recipient of the event).
    fn server_trigger(&mut self, event: ToClients<impl Event>);
}

impl ServerTriggerExt for Commands<'_, '_> {
    fn server_trigger(&mut self, event: ToClients<impl Event>) {
        self.write_message(ToClients {
            mode: event.mode,
            message: ServerTriggerEvent {
                event: event.message,
            },
        });
    }
}

impl ServerTriggerExt for World {
    fn server_trigger(&mut self, event: ToClients<impl Event>) {
        self.write_message(ToClients {
            mode: event.mode,
            message: ServerTriggerEvent {
                event: event.message,
            },
        });
    }
}

/// A message that used under the hood for server events.
///
/// Events are implemented through messages in order to reuse their logic.
/// So we write this message instead and, after receiving it, drain it to trigger regular events.
/// This also allows us to avoid requiring [`Clone`] because events can't be drained.
#[derive(Message)]
struct ServerTriggerEvent<E> {
    event: E,
}

impl<E> From<E> for ServerTriggerEvent<E> {
    fn from(event: E) -> Self {
        Self { event }
    }
}

impl<E> AsRef<E> for ServerTriggerEvent<E> {
    fn as_ref(&self) -> &E {
        &self.event
    }
}
