use core::any::TypeId;

use bevy::{
    ecs::{component::ComponentId, entity::MapEntities},
    prelude::*,
    ptr::PtrMut,
};
use bytes::Bytes;
use log::{debug, error, warn};
use serde::{Serialize, de::DeserializeOwned};

use super::{
    client_message,
    ctx::{ClientSendCtx, ServerReceiveCtx},
    message_fns::{DeserializeFn, MessageFns, SerializeFn, UntypedMessageFns},
    registry::RemoteMessageRegistry,
};
use crate::prelude::*;

/// An extension trait for [`App`] for creating broadcast messages.
///
/// See also [`BroadcastEventAppExt`] for events, [`ClientMessageAppExt`] for regular client messages
/// and [`ServerMessageAppExt`] for server messages.
pub trait BroadcastMessageAppExt {
    /// Registers a remote broadcast message.
    ///
    /// Similar to [`ClientMessageAppExt::add_client_message`], but the message is emitted as
    /// [`Broadcast<M>`] both on the sender (with [`Broadcaster::Local`]) and on the receiver
    /// (with [`Broadcaster::Remote`]). Useful for sharing logic between client-side prediction
    /// and authoritative server processing.
    ///
    /// On a listen server, locally written messages are emitted as [`Broadcast<M>`] with
    /// [`Broadcaster::Local`].
    ///
    /// Calling [`App::add_message`] is not necessary. Can be used for regular messages that were
    /// previously registered. But be careful, since all messages `M` are drained,
    /// which could break Bevy or third-party plugin systems that read `M`.
    fn add_broadcast_message<M: Message + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_broadcast_message_with(
            channel,
            client_message::default_serialize::<M>,
            client_message::default_deserialize::<M>,
        )
    }

    /// Same as [`Self::add_broadcast_message`], but additionally maps client entities to server inside the message before sending.
    ///
    /// Always use it for messages that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    fn add_mapped_broadcast_message<M>(&mut self, channel: Channel) -> &mut Self
    where
        M: Message + Serialize + DeserializeOwned + MapEntities + Clone,
    {
        self.add_broadcast_message_with(
            channel,
            client_message::default_serialize_mapped::<M>,
            client_message::default_deserialize::<M>,
        )
    }

    /// Same as [`Self::add_broadcast_message`], but uses the specified functions for serialization and deserialization.
    ///
    /// See also [`ClientMessageAppExt::add_client_message_with`].
    fn add_broadcast_message_with<M: Message>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, M>,
        deserialize: DeserializeFn<ServerReceiveCtx, M>,
    ) -> &mut Self;
}

impl BroadcastMessageAppExt for App {
    fn add_broadcast_message_with<M: Message>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, M>,
        deserialize: DeserializeFn<ServerReceiveCtx, M>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_broadcast_message::<M>();

        let fns = MessageFns::new(serialize, deserialize);
        let message = BroadcastMessage::new(self, channel, fns);
        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        registry.register_broadcast_message(message);

        self
    }
}

/// Type-erased functions and metadata for a registered broadcast message.
///
/// Needed to erase message types to process them in a single system.
pub(crate) struct BroadcastMessage {
    /// ID of [`Messages<M>`] resource.
    messages_id: ComponentId,

    /// ID of [`Messages<Broadcast<M>>`] resource.
    broadcast_id: ComponentId,

    /// Used channel.
    channel_id: usize,

    /// ID of `M`.
    type_id: TypeId,

    send: SendFn,
    receive: ReceiveFn,
    send_locally: SendLocallyFn,
    reset: ResetFn,
    fns: UntypedMessageFns,
}

impl BroadcastMessage {
    pub(super) fn new<M: Message, I: 'static>(
        app: &mut App,
        channel: Channel,
        fns: MessageFns<ClientSendCtx, ServerReceiveCtx, M, I>,
    ) -> Self {
        let channel_id = app
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel);

        app.add_message::<M>().add_message::<Broadcast<M>>();

        let messages_id = app.world().resource_id::<Messages<M>>().unwrap();
        let broadcast_id = app.world().resource_id::<Messages<Broadcast<M>>>().unwrap();

        Self {
            messages_id,
            broadcast_id,
            channel_id,
            type_id: TypeId::of::<M>(),
            send: Self::send_typed::<M, I>,
            receive: Self::receive_typed::<M, I>,
            send_locally: Self::send_locally_typed::<M>,
            reset: Self::reset_typed::<M>,
            fns: fns.into(),
        }
    }

    pub(crate) fn messages_id(&self) -> ComponentId {
        self.messages_id
    }

    pub(crate) fn broadcast_id(&self) -> ComponentId {
        self.broadcast_id
    }

    pub(super) fn channel_id(&self) -> usize {
        self.channel_id
    }

    pub(super) fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Sends a broadcast message to the server and writes it locally as [`Broadcast<M>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`], `broadcasts` is [`Messages<Broadcast<M>>`]
    /// and this instance was created for `M`.
    pub(crate) unsafe fn send(
        &self,
        ctx: &mut ClientSendCtx,
        messages: PtrMut,
        broadcasts: PtrMut,
        client_messages: &mut ClientMessages,
    ) {
        unsafe { (self.send)(self, ctx, messages, broadcasts, client_messages) };
    }

    /// Typed version of [`Self::send`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`], `broadcasts` is [`Messages<Broadcast<M>>`],
    /// and this instance was created for `M` and `I`.
    unsafe fn send_typed<M: Message, I: 'static>(
        &self,
        ctx: &mut ClientSendCtx,
        messages: PtrMut,
        broadcasts: PtrMut,
        client_messages: &mut ClientMessages,
    ) {
        let messages: &mut Messages<M> = unsafe { messages.deref_mut() };
        let broadcasts: &mut Messages<Broadcast<M>> = unsafe { broadcasts.deref_mut() };
        for message in messages.drain() {
            let mut message_bytes = Vec::new();
            if let Err(e) = unsafe { self.serialize::<M, I>(ctx, &message, &mut message_bytes) } {
                error!(
                    "ignoring message `{}` that failed to serialize: {e}",
                    ShortName::of::<M>()
                );
                continue;
            }

            debug!("sending broadcast message `{}`", ShortName::of::<M>());
            client_messages.send(self.channel_id, message_bytes);
            broadcasts.write(Broadcast {
                broadcaster: Broadcaster::Local,
                message,
            });
        }
    }

    /// Receives broadcast messages from clients.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `broadcasts` is [`Messages<Broadcast<M>>`]
    /// and this instance was created for `M`.
    pub(crate) unsafe fn receive(
        &self,
        ctx: &mut ServerReceiveCtx,
        broadcasts: PtrMut,
        server_messages: &mut ServerMessages,
    ) {
        unsafe { (self.receive)(self, ctx, broadcasts, server_messages) }
    }

    /// Typed version of [`Self::receive`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `broadcasts` is [`Messages<Broadcast<M>>`]
    /// and this instance was created for `M` and `I`.
    unsafe fn receive_typed<M: Message, I: 'static>(
        &self,
        ctx: &mut ServerReceiveCtx,
        broadcasts: PtrMut,
        server_messages: &mut ServerMessages,
    ) {
        let broadcasts: &mut Messages<Broadcast<M>> = unsafe { broadcasts.deref_mut() };
        for (client, mut message) in server_messages.receive(self.channel_id) {
            match unsafe { self.deserialize::<M, I>(ctx, &mut message) } {
                Ok(message) => {
                    debug!(
                        "writing broadcast message `{}` from client `{client}`",
                        ShortName::of::<M>()
                    );
                    broadcasts.write(Broadcast {
                        broadcaster: Broadcaster::Remote(client.into()),
                        message,
                    });
                }
                Err(e) => debug!(
                    "ignoring message `{}` from client `{client}` that failed to deserialize: {e}",
                    ShortName::of::<M>()
                ),
            }
        }
    }

    /// Drains messages `M` and writes them as [`Broadcast<M>`] with [`Broadcaster::Local`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`], `broadcasts` is [`Messages<Broadcast<M>>`]
    /// and this instance was created for `M`.
    pub(crate) unsafe fn send_locally(&self, broadcasts: PtrMut, messages: PtrMut) {
        unsafe { (self.send_locally)(broadcasts, messages) }
    }

    /// Typed version of [`Self::send_locally`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`] and `broadcasts` is [`Messages<Broadcast<M>>`].
    unsafe fn send_locally_typed<M: Message>(broadcasts: PtrMut, messages: PtrMut) {
        let broadcasts: &mut Messages<Broadcast<M>> = unsafe { broadcasts.deref_mut() };
        let messages: &mut Messages<M> = unsafe { messages.deref_mut() };
        if !messages.is_empty() {
            debug!(
                "writing {} broadcast message(s) `{}` locally",
                messages.len(),
                ShortName::of::<M>()
            );
            broadcasts.write_batch(messages.drain().map(|message| Broadcast {
                broadcaster: Broadcaster::Local,
                message,
            }));
        }
    }

    /// Drains all messages.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`]
    /// and this instance was created for `M`.
    pub(crate) unsafe fn reset(&self, messages: PtrMut) {
        unsafe { (self.reset)(messages) }
    }

    /// Typed version of [`Self::reset`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`].
    unsafe fn reset_typed<M: Message>(messages: PtrMut) {
        let messages: &mut Messages<M> = unsafe { messages.deref_mut() };
        let drained_count = messages.drain().count();
        if drained_count > 0 {
            warn!(
                "discarded {drained_count} broadcast messages of type `{}` due to a disconnect",
                ShortName::of::<M>()
            );
        }
    }

    /// Serializes a message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `M` and `I`.
    unsafe fn serialize<M: 'static, I: 'static>(
        &self,
        ctx: &mut ClientSendCtx,
        message: &M,
        message_bytes: &mut Vec<u8>,
    ) -> Result<()> {
        unsafe {
            self.fns
                .typed::<ClientSendCtx, ServerReceiveCtx, M, I>()
                .serialize(ctx, message, message_bytes)?;
        }

        if ctx.invalid_entities.is_empty() {
            Ok(())
        } else {
            let error_text = format!(
                "unable to map entities `{:?}` for the server, \
                make sure that the message references entities visible to the server",
                ctx.invalid_entities,
            );
            ctx.invalid_entities.clear();
            Err(error_text.into())
        }
    }

    /// Deserializes a message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `M` and `I`.
    unsafe fn deserialize<M: 'static, I: 'static>(
        &self,
        ctx: &mut ServerReceiveCtx,
        message: &mut Bytes,
    ) -> Result<M> {
        unsafe {
            self.fns
                .typed::<ClientSendCtx, ServerReceiveCtx, M, I>()
                .deserialize(ctx, message)
        }
    }
}

/// Signature of broadcast message sending functions.
type SendFn = unsafe fn(&BroadcastMessage, &mut ClientSendCtx, PtrMut, PtrMut, &mut ClientMessages);

/// Signature of broadcast message receiving functions.
type ReceiveFn = unsafe fn(&BroadcastMessage, &mut ServerReceiveCtx, PtrMut, &mut ServerMessages);

/// Signature of broadcast message local-sending functions.
type SendLocallyFn = unsafe fn(PtrMut, PtrMut);

/// Signature of broadcast message reset functions.
type ResetFn = unsafe fn(PtrMut);

/// A remote message that originates from a client.
///
/// Emitted both on the sender (with [`Broadcaster::Local`]) and on the receiver
/// (with [`Broadcaster::Remote`]).
#[derive(Message, Event, Deref, DerefMut, Debug, Clone, Copy)]
pub struct Broadcast<T> {
    /// Origin of the message.
    pub broadcaster: Broadcaster,

    /// Transmitted message.
    #[deref]
    pub message: T,
}

impl<E: EntityEvent> EntityEvent for Broadcast<E> {
    fn event_target(&self) -> Entity {
        self.message.event_target()
    }
}

/// Origin of a [`Broadcast`] message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Broadcaster {
    /// Written locally by this app.
    Local,

    /// Received over the network from a remote client.
    Remote(ClientId),
}

impl Broadcaster {
    /// Returns `true` if the message was written locally.
    pub fn is_local(self) -> bool {
        matches!(self, Self::Local)
    }

    /// Returns `true` if the message was received from a remote client.
    pub fn is_remote(self) -> bool {
        matches!(self, Self::Remote(_))
    }
}
