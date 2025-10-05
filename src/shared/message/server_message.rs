pub(crate) mod message_buffer;
mod message_queue;

use core::any::{self, TypeId};

use bevy::{
    ecs::{component::ComponentId, entity::MapEntities},
    prelude::*,
    ptr::{Ptr, PtrMut},
};
use bytes::Bytes;
use log::{debug, error, warn};
use postcard::experimental::max_size::MaxSize;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    ctx::{ClientReceiveCtx, ServerSendCtx},
    message_fns::{DeserializeFn, MessageFns, SerializeFn, UntypedMessageFns},
    registry::RemoteMessageRegistry,
};
use crate::{postcard_utils, prelude::*};
use message_buffer::{MessageBuffer, SerializedMessage};
use message_queue::MessageQueue;

/// An extension trait for [`App`] for creating server messages.
///
/// See also [`ClientMessageAppExt`] for client messages and [`ServerMessageAppExt`] for events.
pub trait ServerMessageAppExt {
    /// Registers a remote server message.
    ///
    /// After writing [`ToClients<M>`] message on the server, `M` message will be written on clients.
    ///
    /// If [`ClientMessagePlugin`] is enabled and [`ClientId::Server`] is a recipient of the message, then
    /// [`ToClients<M>`] message will be drained after sending to clients and `M` message will be written
    /// on the server as well.
    ///
    /// Calling [`App::add_message`] is not necessary. Can used for regular messages that were
    /// previously registered.
    ///
    /// Unlike client messages, server messages are tied to replication. See [`Self::make_message_independent`]
    /// for more details.
    ///
    /// See also the [corresponding section](../index.html#from-client-to-server) from the quick start guide.
    fn add_server_message<M: Message + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_server_message_with(channel, default_serialize::<M>, default_deserialize::<M>)
    }

    /// Same as [`Self::add_server_message`], but additionally maps server entities to client inside the message after receiving.
    ///
    /// Always use it for messages that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    fn add_mapped_server_message<M: Message + Serialize + DeserializeOwned + MapEntities>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_server_message_with(
            channel,
            default_serialize::<M>,
            default_deserialize_mapped::<M>,
        )
    }

    /**
    Same as [`Self::add_server_message`], but uses the specified functions for serialization and deserialization.

    See also [`postcard_utils`] and [`ServerEventAppExt::add_server_event_with`].

    # Examples

    Register a message with [`Box<dyn PartialReflect>`]:

    ```
    use bevy::{
        prelude::*,
        reflect::serde::{ReflectDeserializer, ReflectSerializer},
        state::app::StatesPlugin,
    };
    use bevy_replicon::{
        bytes::Bytes,
        postcard_utils::{BufFlavor, ExtendMutFlavor},
        shared::message::ctx::{ClientReceiveCtx, ServerSendCtx},
        prelude::*,
    };
    use postcard::{Deserializer, Serializer};
    use serde::{de::DeserializeSeed, Serialize};

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));
    app.add_server_message_with(Channel::Ordered, serialize_dynamic, deserialize_dynamic);

    fn serialize_dynamic(
        ctx: &mut ServerSendCtx,
        dynamic: &Dynamic,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        let mut serializer = Serializer { output: ExtendMutFlavor::new(message) };
        let registry = ctx.type_registry.read();
        ReflectSerializer::new(&*dynamic.0, &registry).serialize(&mut serializer)?;
        Ok(())
    }

    fn deserialize_dynamic(
        ctx: &mut ClientReceiveCtx,
        message: &mut Bytes,
    ) -> Result<Dynamic> {
        let mut deserializer = Deserializer::from_flavor(BufFlavor::new(message));
        let registry = ctx.type_registry.read();
        let reflect = ReflectDeserializer::new(&registry).deserialize(&mut deserializer)?;
        Ok(Dynamic(reflect))
    }

    #[derive(Message)]
    struct Dynamic(Box<dyn PartialReflect>);
    ```

    See also [`AppRuleExt::replicate_with`] for more examples with custom ser/de.
    */
    fn add_server_message_with<M: Message>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ServerSendCtx, M>,
        deserialize: DeserializeFn<ClientReceiveCtx, M>,
    ) -> &mut Self;

    /// Marks the message `M` as an independent message.
    ///
    /// By default, all server messages are buffered on server until server tick
    /// and queued on client until all insertions, removals and despawns
    /// (value mutations doesn't count) are replicated for the tick on which the
    /// message was written. This is necessary to ensure that the executed logic
    /// during the message does not affect components or entities that the client
    /// has not yet received.
    ///
    /// For more details about replication see the documentation on
    /// [`ServerChannel`](crate::shared::backend::channels::ServerChannel).
    ///
    /// However, if you know your message doesn't rely on that, you can mark it
    /// as independent to always emit it immediately. For example, a chat
    /// message - which does not hold references to any entities - may be
    /// marked as independent.
    ///
    /// <div class="warning">
    ///
    /// Use this method very carefully; it can lead to logic errors that are
    /// very difficult to debug!
    ///
    /// </div>
    ///
    /// See also [`ServerEventAppExt::make_event_independent`].
    fn make_message_independent<M: Message>(&mut self) -> &mut Self;
}

impl ServerMessageAppExt for App {
    fn add_server_message_with<M: Message>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ServerSendCtx, M>,
        deserialize: DeserializeFn<ClientReceiveCtx, M>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_server_message::<M>();

        let fns = MessageFns::new(serialize, deserialize);
        let message = ServerMessage::new(self, channel, fns);
        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        registry.register_server_message(message);

        self
    }

    fn make_message_independent<M: Message>(&mut self) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .make_message_independent::<M>();

        let messages_id = self
            .world()
            .components()
            .resource_id::<Messages<M>>()
            .unwrap_or_else(|| {
                panic!(
                    "message `{}` should be previously registered",
                    any::type_name::<M>()
                )
            });

        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        let message = registry
            .iter_server_messages_mut()
            .find(|m| m.messages_id() == messages_id)
            .unwrap_or_else(|| {
                panic!(
                    "message `{}` should be previously registered as a server message",
                    any::type_name::<M>()
                )
            });

        message.independent = true;

        self
    }
}

/// Type-erased functions and metadata for a registered server message.
///
/// Needed to erase message types to process them in a single system.
pub(crate) struct ServerMessage {
    /// Whether this message depends on replication or not.
    ///
    /// Things like a chat message do not have to wait for replication to
    /// be synced. If set to `true`, the message will always be applied
    /// immediately.
    pub(super) independent: bool,

    /// ID of [`Messages<M>`].
    messages_id: ComponentId,

    /// ID of [`Messages<ToClients<M>>`].
    to_messages_id: ComponentId,

    /// ID of [`MessageQueue<T>`].
    queue_id: ComponentId,

    /// Used channel.
    channel_id: usize,

    /// ID of `M`.
    type_id: TypeId,

    send_or_buffer: SendOrBufferFn,
    receive: ReceiveFn,
    send_locally: SendLocallyFn,
    reset: ResetFn,
    fns: UntypedMessageFns,
}

impl ServerMessage {
    pub(super) fn new<M: Message, I: 'static>(
        app: &mut App,
        channel: Channel,
        fns: MessageFns<ServerSendCtx, ClientReceiveCtx, M, I>,
    ) -> Self {
        let channel_id = app
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_server_channel(channel);

        app.add_message::<M>()
            .add_message::<ToClients<M>>()
            .init_resource::<MessageQueue<M>>();

        let messages_id = app.world().resource_id::<Messages<M>>().unwrap();
        let to_messages_id = app.world().resource_id::<Messages<ToClients<M>>>().unwrap();
        let queue_id = app.world().resource_id::<MessageQueue<M>>().unwrap();

        Self {
            independent: false,
            messages_id,
            to_messages_id,
            queue_id,
            channel_id,
            type_id: TypeId::of::<M>(),
            send_or_buffer: Self::send_or_buffer_typed::<M, I>,
            receive: Self::receive_typed::<M, I>,
            send_locally: Self::send_locally_typed::<M>,
            reset: Self::reset_typed::<M>,
            fns: fns.into(),
        }
    }

    pub(crate) fn messages_id(&self) -> ComponentId {
        self.messages_id
    }

    pub(crate) fn to_messages_id(&self) -> ComponentId {
        self.to_messages_id
    }

    pub(crate) fn queue_id(&self) -> ComponentId {
        self.queue_id
    }

    pub(super) fn channel_id(&self) -> usize {
        self.channel_id
    }

    pub(super) fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Sends a message to client(s).
    ///
    /// # Safety
    ///
    /// The caller must ensure that `to_messages` is [`Messages<ToClients<M>>`]
    /// and this instance was created for `M`.
    pub(crate) unsafe fn send_or_buffer(
        &self,
        ctx: &mut ServerSendCtx,
        to_messages: &Ptr,
        server_messages: &mut ServerMessages,
        clients: &Query<Entity, With<ConnectedClient>>,
        message_buffer: &mut MessageBuffer,
    ) {
        unsafe {
            (self.send_or_buffer)(
                self,
                ctx,
                to_messages,
                server_messages,
                clients,
                message_buffer,
            )
        }
    }

    /// Typed version of [`Self::send_or_buffer`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `to_messages` is [`Messages<ToClients<M>>`]
    /// and this instance was created for `M` and `I`.
    unsafe fn send_or_buffer_typed<M: Message, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        to_messages: &Ptr,
        server_messages: &mut ServerMessages,
        clients: &Query<Entity, With<ConnectedClient>>,
        message_buffer: &mut MessageBuffer,
    ) {
        let to_messages: &Messages<ToClients<M>> = unsafe { to_messages.deref() };
        // For server messages we don't track read message because
        // all of them will always be drained in the local sending system.
        for ToClients { message, mode } in to_messages.get_cursor().read(to_messages) {
            debug!(
                "sending message `{}` with `{mode:?}`",
                any::type_name::<M>()
            );

            if self.independent {
                unsafe {
                    self.send_independent_message::<M, I>(
                        ctx,
                        message,
                        mode,
                        server_messages,
                        clients,
                    )
                    .expect("independent server message should be serializable");
                }
            } else {
                unsafe {
                    self.buffer_message::<M, I>(ctx, message, *mode, message_buffer)
                        .expect("server message should be serializable");
                }
            }
        }
    }

    /// Sends independent remote message `M` based on a mode.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `M` and `I`.
    ///
    /// For regular messages see [`Self::buffer_message`].
    unsafe fn send_independent_message<M: Message, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        message: &M,
        mode: &SendMode,
        server_messages: &mut ServerMessages,
        clients: &Query<Entity, With<ConnectedClient>>,
    ) -> Result<()> {
        let mut message_bytes = Vec::new();
        unsafe { self.serialize::<M, I>(ctx, message, &mut message_bytes)? }
        let message_bytes: Bytes = message_bytes.into();

        match *mode {
            SendMode::Broadcast => {
                for client_entity in clients {
                    server_messages.send(client_entity, self.channel_id, message_bytes.clone());
                }
            }
            SendMode::BroadcastExcept(ignored_id) => {
                for client in clients {
                    if ignored_id != client.into() {
                        server_messages.send(client, self.channel_id, message_bytes.clone());
                    }
                }
            }
            SendMode::Direct(client_id) => {
                if let ClientId::Client(client) = client_id {
                    server_messages.send(client, self.channel_id, message_bytes.clone());
                }
            }
        }

        Ok(())
    }

    /// Buffers message `M` based on mode.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `M` and `I`.
    ///
    /// For independent messages see [`Self::send_independent_message`].
    unsafe fn buffer_message<M: Message, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        message: &M,
        mode: SendMode,
        message_buffer: &mut MessageBuffer,
    ) -> Result<()> {
        let message_bytes = unsafe { self.serialize_with_padding::<M, I>(ctx, message)? };
        message_buffer.insert(mode, self.channel_id, message_bytes);
        Ok(())
    }

    /// Helper for serializing a server message.
    ///
    /// Will prepend padding bytes for where the update tick will be inserted to the injected message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `M` and `I`.
    unsafe fn serialize_with_padding<M: Message, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        message: &M,
    ) -> Result<SerializedMessage> {
        let mut message_bytes = vec![0; RepliconTick::POSTCARD_MAX_SIZE]; // Padding for the tick.
        unsafe { self.serialize::<M, I>(ctx, message, &mut message_bytes)? }
        let message = SerializedMessage::Raw(message_bytes);

        Ok(message)
    }

    /// Receives messages from the server.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`], `queue` is [`MessageQueue<M>`],
    /// and this instance was created for `M`.
    pub(crate) unsafe fn receive(
        &self,
        ctx: &mut ClientReceiveCtx,
        messages: PtrMut,
        queue: PtrMut,
        client_messages: &mut ClientMessages,
        update_tick: RepliconTick,
    ) {
        unsafe { (self.receive)(self, ctx, messages, queue, client_messages, update_tick) }
    }

    /// Typed version of [`ServerMessage::receive`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`], `queue` is [`MessageQueue<M>`]
    /// and this instance was created for `M` and `I`.
    unsafe fn receive_typed<M: Message, I: 'static>(
        &self,
        ctx: &mut ClientReceiveCtx,
        messages: PtrMut,
        queue: PtrMut,
        client_messages: &mut ClientMessages,
        update_tick: RepliconTick,
    ) {
        let messages: &mut Messages<M> = unsafe { messages.deref_mut() };
        let queue: &mut MessageQueue<M> = unsafe { queue.deref_mut() };

        while let Some((tick, serialized_messages)) = queue.pop_if_le(update_tick) {
            for mut message in serialized_messages {
                match unsafe { self.deserialize::<M, I>(ctx, &mut message) } {
                    Ok(message) => {
                        debug!(
                            "writing message `{}` from queue with `{tick:?}`",
                            any::type_name::<M>()
                        );
                        messages.write(message);
                    }
                    Err(e) => error!(
                        "ignoring message `{}` from queue with `{tick:?}` that failed to deserialize: {e}",
                        any::type_name::<M>()
                    ),
                }
            }
        }

        for mut message in client_messages.receive(self.channel_id) {
            if !self.independent {
                let tick = match postcard_utils::from_buf(&mut message) {
                    Ok(tick) => tick,
                    Err(e) => {
                        error!(
                            "ignoring message `{}` because it's tick failed to deserialize: {e}",
                            any::type_name::<M>()
                        );
                        continue;
                    }
                };
                if tick > update_tick {
                    debug!(
                        "queuing message `{}` with `{tick:?}`",
                        any::type_name::<M>()
                    );
                    queue.insert(tick, message);
                    continue;
                } else {
                    debug!(
                        "receiving message `{}` with `{tick:?}`",
                        any::type_name::<M>()
                    );
                }
            }

            match unsafe { self.deserialize::<M, I>(ctx, &mut message) } {
                Ok(message) => {
                    debug!("writing message `{}`", any::type_name::<M>());
                    messages.write(message);
                }
                Err(e) => error!(
                    "ignoring message `{}` that failed to deserialize: {e}",
                    any::type_name::<M>()
                ),
            }
        }
    }

    /// Drains messages [`ToClients<M>`] and writes them as `M` if the server is in the list of the message recipients.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`], `to_messages` is [`Messages<ToClients<M>>`],
    /// and this instance was created for `M`.
    pub(crate) unsafe fn send_locally(&self, to_messages: PtrMut, messages: PtrMut) {
        unsafe { (self.send_locally)(to_messages, messages) }
    }

    /// Typed version of [`Self::send_locally`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<M>`] and `to_messages` is [`Messages<ToClients<M>>`].
    unsafe fn send_locally_typed<M: Message>(to_messages: PtrMut, messages: PtrMut) {
        let to_messages: &mut Messages<ToClients<M>> = unsafe { to_messages.deref_mut() };
        let messages: &mut Messages<M> = unsafe { messages.deref_mut() };
        for ToClients { message, mode } in to_messages.drain() {
            debug!("writing message `{}` locally", any::type_name::<M>());
            match mode {
                SendMode::Broadcast => {
                    messages.write(message);
                }
                SendMode::BroadcastExcept(ignored_id) => {
                    if ignored_id != ClientId::Server {
                        messages.write(message);
                    }
                }
                SendMode::Direct(client_id) => {
                    if client_id == ClientId::Server {
                        messages.write(message);
                    }
                }
            }
        }
    }

    /// Clears queued messages.
    ///
    /// We clear messages while waiting for a connection to ensure clean reconnects.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `queue` is [`MessageQueue<M>`]
    /// and this instance was created for `M`.
    pub(crate) unsafe fn reset(&self, queue: PtrMut) {
        unsafe { (self.reset)(queue) }
    }

    /// Typed version of [`Self::reset`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `queue` is [`MessageQueue<M>`].
    unsafe fn reset_typed<M: Message>(queue: PtrMut) {
        let queue: &mut MessageQueue<M> = unsafe { queue.deref_mut() };
        if !queue.is_empty() {
            warn!(
                "discarding {} queued messages due to a disconnect",
                queue.len()
            );
        }
        queue.clear();
    }

    /// Serializes a messages.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `M` and `I`.
    unsafe fn serialize<M: Message, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        message: &M,
        message_bytes: &mut Vec<u8>,
    ) -> Result<()> {
        unsafe {
            self.fns
                .typed::<ServerSendCtx, ClientReceiveCtx, M, I>()
                .serialize(ctx, message, message_bytes)
        }
    }

    /// Deserializes a message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `M` and `I`.
    unsafe fn deserialize<M: Message, I: 'static>(
        &self,
        ctx: &mut ClientReceiveCtx,
        message: &mut Bytes,
    ) -> Result<M> {
        let message = unsafe {
            self.fns
                .typed::<ServerSendCtx, ClientReceiveCtx, M, I>()
                .deserialize(ctx, message)?
        };

        if ctx.invalid_entities.is_empty() {
            Ok(message)
        } else {
            let msg = format!(
                "unable to map entities `{:?}` from the server, \
                make sure that the message references entities visible to the client",
                ctx.invalid_entities
            );
            ctx.invalid_entities.clear();
            Err(msg.into())
        }
    }
}

/// Signature of server message sending functions.
type SendOrBufferFn = unsafe fn(
    &ServerMessage,
    &mut ServerSendCtx,
    &Ptr,
    &mut ServerMessages,
    &Query<Entity, With<ConnectedClient>>,
    &mut MessageBuffer,
);

/// Signature of server message receiving functions.
type ReceiveFn = unsafe fn(
    &ServerMessage,
    &mut ClientReceiveCtx,
    PtrMut,
    PtrMut,
    &mut ClientMessages,
    RepliconTick,
);

/// Signature of server message sending functions.
type SendLocallyFn = unsafe fn(PtrMut, PtrMut);

/// Signature of server message reset functions.
type ResetFn = unsafe fn(PtrMut);

/// A remote message that will be send to client(s).
#[derive(Event, Message, Deref, DerefMut, Debug, Clone, Copy)]
pub struct ToClients<T> {
    /// Recipients.
    pub mode: SendMode,
    /// Transmitted message.
    #[deref]
    pub message: T,
}

/// Type of server message sending.
#[derive(Clone, Copy, Debug)]
pub enum SendMode {
    /// Send to every client.
    Broadcast,
    /// Send to every client except the specified connected client.
    BroadcastExcept(ClientId),
    /// Send only to the specified client.
    Direct(ClientId),
}

/// Default message serialization function.
pub fn default_serialize<M: Serialize>(
    _ctx: &mut ServerSendCtx,
    message: &M,
    message_bytes: &mut Vec<u8>,
) -> Result<()> {
    postcard_utils::to_extend_mut(message, message_bytes)?;
    Ok(())
}

/// Default message deserialization function.
pub fn default_deserialize<M: DeserializeOwned>(
    _ctx: &mut ClientReceiveCtx,
    message: &mut Bytes,
) -> Result<M> {
    let message = postcard_utils::from_buf(message)?;
    Ok(message)
}

/// Like [`default_deserialize`], but also maps entities.
pub fn default_deserialize_mapped<M: DeserializeOwned + MapEntities>(
    ctx: &mut ClientReceiveCtx,
    message: &mut Bytes,
) -> Result<M> {
    let mut message: M = postcard_utils::from_buf(message)?;
    message.map_entities(ctx);
    Ok(message)
}
