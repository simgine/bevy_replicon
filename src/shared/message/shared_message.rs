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

/// An extension trait for [`App`] for creating shared messages.
///
/// They're like client messages, but also emitted locally in the same way as on the server.
///
/// See also [`SharedEventAppExt`] for events, [`ClientMessageAppExt`] for regular client messages
/// and [`ServerMessageAppExt`] for server messages.
pub trait SharedMessageAppExt {
    /// Registers a remote shared message.
    ///
    /// Similar to [`ClientMessageAppExt::add_client_message`], but the message is emitted as
    /// [`LocalOrRemote<M>`] both on the sender (with [`Sender::Local`]) and on the receiver
    /// (with [`Sender::Remote`]). Useful for sharing logic between client-side prediction
    /// and authoritative server processing.
    ///
    /// On a listen server, locally written messages are emitted as [`LocalOrRemote<M>`] with
    /// [`Sender::Local`].
    ///
    /// Calling [`App::add_message`] is not necessary. Can be used for regular messages that were
    /// previously registered. But be careful, since all messages `M` are drained,
    /// which could break Bevy or third-party plugin systems that read `M`.
    fn add_shared_message<M: Message + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_shared_message_with(
            channel,
            client_message::default_serialize::<M>,
            client_message::default_deserialize::<M>,
        )
    }

    /// Same as [`Self::add_shared_message`], but additionally maps client entities to server inside the message before sending.
    ///
    /// Always use it for messages that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    fn add_mapped_shared_message<M>(&mut self, channel: Channel) -> &mut Self
    where
        M: Message + Serialize + DeserializeOwned + MapEntities + Clone,
    {
        self.add_shared_message_with(
            channel,
            client_message::default_serialize_mapped::<M>,
            client_message::default_deserialize::<M>,
        )
    }

    /// Same as [`Self::add_shared_message`], but uses the specified functions for serialization and deserialization.
    ///
    /// See also [`ClientMessageAppExt::add_client_message_with`].
    fn add_shared_message_with<M: Message>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, M>,
        deserialize: DeserializeFn<ServerReceiveCtx, M>,
    ) -> &mut Self;
}

impl SharedMessageAppExt for App {
    fn add_shared_message_with<M: Message>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, M>,
        deserialize: DeserializeFn<ServerReceiveCtx, M>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_shared_message::<M>();

        let fns = MessageFns::new(serialize, deserialize);
        let message = SharedMessage::new(self, channel, fns);
        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        registry.register_shared_message(message);

        self
    }
}

/// Type-erased functions and metadata for a registered shared message.
///
/// Needed to erase message types to process them in a single system.
pub(crate) struct SharedMessage {
    /// ID of [`Messages<M>`] resource.
    messages_id: ComponentId,

    /// ID of [`Messages<LocalOrRemote<M>>`] resource.
    shared_messages_id: ComponentId,

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

impl SharedMessage {
    pub(super) fn new<M: Message, I: 'static>(
        app: &mut App,
        channel: Channel,
        fns: MessageFns<ClientSendCtx, ServerReceiveCtx, M, I>,
    ) -> Self {
        let channel_id = app
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel);

        app.add_message::<M>().add_message::<LocalOrRemote<M>>();

        let messages_id = app.world().resource_id::<Messages<M>>().unwrap();
        let shared_messages_id = app
            .world()
            .resource_id::<Messages<LocalOrRemote<M>>>()
            .unwrap();

        Self {
            messages_id,
            shared_messages_id,
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

    pub(crate) fn shared_messages_id(&self) -> ComponentId {
        self.shared_messages_id
    }

    pub(super) fn channel_id(&self) -> usize {
        self.channel_id
    }

    pub(super) fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Sends a shared message to the server and writes it locally as [`LocalOrRemote<M>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`], `shared_messages` is [`Messages<LocalOrRemote<M>>`]
    /// and this instance was created for `M`.
    pub(crate) unsafe fn send(
        &self,
        ctx: &mut ClientSendCtx,
        messages: PtrMut,
        shared_messages: PtrMut,
        client_messages: &mut ClientMessages,
    ) {
        unsafe { (self.send)(self, ctx, messages, shared_messages, client_messages) };
    }

    /// Typed version of [`Self::send`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`], `shared_messages` is [`Messages<LocalOrRemote<M>>`],
    /// and this instance was created for `M` and `I`.
    unsafe fn send_typed<M: Message, I: 'static>(
        &self,
        ctx: &mut ClientSendCtx,
        messages: PtrMut,
        shared_messages: PtrMut,
        client_messages: &mut ClientMessages,
    ) {
        let messages: &mut Messages<M> = unsafe { messages.deref_mut() };
        let shared_messages: &mut Messages<LocalOrRemote<M>> =
            unsafe { shared_messages.deref_mut() };
        for message in messages.drain() {
            let mut message_bytes = Vec::new();
            if let Err(e) = unsafe { self.serialize::<M, I>(ctx, &message, &mut message_bytes) } {
                error!(
                    "ignoring message `{}` that failed to serialize: {e}",
                    ShortName::of::<M>()
                );
                continue;
            }

            debug!("sending shared message `{}`", ShortName::of::<M>());
            client_messages.send(self.channel_id, message_bytes);
            shared_messages.write(LocalOrRemote {
                sender: Sender::Local,
                message,
            });
        }
    }

    /// Receives shared messages from clients.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `shared_messages` is [`Messages<LocalOrRemote<M>>`]
    /// and this instance was created for `M`.
    pub(crate) unsafe fn receive(
        &self,
        ctx: &mut ServerReceiveCtx,
        shared_messages: PtrMut,
        server_messages: &mut ServerMessages,
    ) {
        unsafe { (self.receive)(self, ctx, shared_messages, server_messages) }
    }

    /// Typed version of [`Self::receive`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `shared_messages` is [`Messages<LocalOrRemote<M>>`]
    /// and this instance was created for `M` and `I`.
    unsafe fn receive_typed<M: Message, I: 'static>(
        &self,
        ctx: &mut ServerReceiveCtx,
        shared_messages: PtrMut,
        server_messages: &mut ServerMessages,
    ) {
        let shared_messages: &mut Messages<LocalOrRemote<M>> =
            unsafe { shared_messages.deref_mut() };
        for (client, mut message) in server_messages.receive(self.channel_id) {
            match unsafe { self.deserialize::<M, I>(ctx, &mut message) } {
                Ok(message) => {
                    debug!(
                        "writing shared message `{}` from client `{client}`",
                        ShortName::of::<M>()
                    );
                    shared_messages.write(LocalOrRemote {
                        sender: Sender::Remote(client.into()),
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

    /// Drains messages `M` and writes them as [`LocalOrRemote<M>`] with [`Sender::Local`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`], `shared_messages` is [`Messages<LocalOrRemote<M>>`]
    /// and this instance was created for `M`.
    pub(crate) unsafe fn send_locally(&self, shared_messages: PtrMut, messages: PtrMut) {
        unsafe { (self.send_locally)(shared_messages, messages) }
    }

    /// Typed version of [`Self::send_locally`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`] and `shared_messages` is [`Messages<LocalOrRemote<M>>`].
    unsafe fn send_locally_typed<M: Message>(shared_messages: PtrMut, messages: PtrMut) {
        let shared_messages: &mut Messages<LocalOrRemote<M>> =
            unsafe { shared_messages.deref_mut() };
        let messages: &mut Messages<M> = unsafe { messages.deref_mut() };
        if !messages.is_empty() {
            debug!(
                "writing {} shared message(s) `{}` locally",
                messages.len(),
                ShortName::of::<M>()
            );
            shared_messages.write_batch(messages.drain().map(|message| LocalOrRemote {
                sender: Sender::Local,
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
                "discarded {drained_count} shared messages of type `{}` due to a disconnect",
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

/// Signature of shared message sending functions.
type SendFn = unsafe fn(&SharedMessage, &mut ClientSendCtx, PtrMut, PtrMut, &mut ClientMessages);

/// Signature of shared message receiving functions.
type ReceiveFn = unsafe fn(&SharedMessage, &mut ServerReceiveCtx, PtrMut, &mut ServerMessages);

/// Signature of shared message local-sending functions.
type SendLocallyFn = unsafe fn(PtrMut, PtrMut);

/// Signature of shared message reset functions.
type ResetFn = unsafe fn(PtrMut);

/// A remote message from a client.
///
/// Emitted both on the sender (with [`Sender::Local`]) and on the receiver
/// (with [`Sender::Remote`]).
#[derive(Message, Event, Deref, DerefMut, Debug, Clone, Copy)]
pub struct LocalOrRemote<T> {
    /// Sender of the message.
    pub sender: Sender,

    /// Transmitted message.
    #[deref]
    pub message: T,
}

impl<E: EntityEvent> EntityEvent for LocalOrRemote<E> {
    fn event_target(&self) -> Entity {
        self.message.event_target()
    }
}

/// Sender of a [`LocalOrRemote`] message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sender {
    /// Written locally by this app.
    Local,

    /// Received over the network from a remote client.
    Remote(ClientId),
}

impl Sender {
    /// Returns `true` if the message was received from a remote client.
    pub fn is_remote(self) -> bool {
        matches!(self, Self::Remote(_))
    }
}
