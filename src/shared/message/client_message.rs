use core::any::{self, TypeId};

use bevy::{
    ecs::{component::ComponentId, entity::MapEntities, message::MessageCursor},
    prelude::*,
    ptr::{Ptr, PtrMut},
};
use bytes::Bytes;
use log::{debug, error, warn};
use serde::{Serialize, de::DeserializeOwned};

use super::{
    ctx::{ClientSendCtx, ServerReceiveCtx},
    message_fns::{DeserializeFn, MessageFns, SerializeFn, UntypedMessageFns},
    registry::RemoteMessageRegistry,
};
use crate::{postcard_utils, prelude::*};

/// An extension trait for [`App`] for creating client messages.
///
/// See also [`ServerMessageAppExt`] for server messages and [`ClientMessageAppExt`] for events.
pub trait ClientMessageAppExt {
    /// Registers a remote client message.
    ///
    /// After writing `E` message on the client, [`FromClient<E>`] message will be written on the server.
    ///
    /// If [`ServerMessagePlugin`] is enabled and the client state is [`ClientState::Disconnected`], the message will be drained
    /// right after writing and will be written locally as [`FromClient<E>`] message with [`FromClient::client_id`]
    /// equal to [`ClientId::Server`].
    ///
    /// Calling [`App::add_message`] is not necessary. Can used for regular messages that were
    /// previously registered. But be careful, since on listen servers all messages `E` are drained,
    /// which could break Bevy or third-party plugin systems that read `E`.
    ///
    /// See also the [corresponding section](../index.html#from-client-to-server) from the quick start guide.
    fn add_client_message<E: Message + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_client_message_with(channel, default_serialize::<E>, default_deserialize::<E>)
    }

    /// Same as [`Self::add_client_message`], but additionally maps client entities to server inside the message before sending.
    ///
    /// Always use it for messages that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    ///
    /// [`Clone`] is required because, before sending, we need to map entities from the client to the server without
    /// modifying the original component.
    fn add_mapped_client_message<E>(&mut self, channel: Channel) -> &mut Self
    where
        E: Message + Serialize + DeserializeOwned + MapEntities + Clone,
    {
        self.add_client_message_with(
            channel,
            default_serialize_mapped::<E>,
            default_deserialize::<E>,
        )
    }

    /**
    Same as [`Self::add_client_message`], but uses the specified functions for serialization and deserialization.

    See also [`postcard_utils`] and [`ClientEventAppExt::add_client_event_with`].

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
        shared::message::ctx::{ClientSendCtx, ServerReceiveCtx},
        prelude::*,
    };
    use postcard::{Deserializer, Serializer};
    use serde::{de::DeserializeSeed, Serialize};

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));
    app.add_client_message_with(Channel::Ordered, serialize_dynamic, deserialize_dynamic);

    fn serialize_dynamic(
        ctx: &mut ClientSendCtx,
        dynamic: &Dynamic,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        let mut serializer = Serializer { output: ExtendMutFlavor::new(message) };
        let registry = ctx.type_registry.read();
        ReflectSerializer::new(&*dynamic.0, &registry).serialize(&mut serializer)?;
        Ok(())
    }

    fn deserialize_dynamic(
        ctx: &mut ServerReceiveCtx,
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
    fn add_client_message_with<E: Message>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self;
}

impl ClientMessageAppExt for App {
    fn add_client_message_with<E: Message>(
        &mut self,
        channel: Channel,
        serialize: SerializeFn<ClientSendCtx, E>,
        deserialize: DeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_client_message::<E>();

        let fns = MessageFns::new(serialize, deserialize);
        let message = ClientMessage::new(self, channel, fns);
        let mut registry = self.world_mut().resource_mut::<RemoteMessageRegistry>();
        registry.register_client_message(message);

        self
    }
}

/// Type-erased functions and metadata for a registered client messages.
///
/// Needed to erase message types to process them in a single system.
pub(crate) struct ClientMessage {
    /// ID of [`Messages<E>`] resource.
    messages_id: ComponentId,

    /// ID of [`ClientMessageReader<E>`] resource.
    reader_id: ComponentId,

    /// ID of [`Messages<FromClient<E>>`] resource.
    from_messages_id: ComponentId,

    /// Used channel.
    channel_id: usize,

    /// ID of `E`.
    type_id: TypeId,

    send: SendFn,
    receive: ReceiveFn,
    send_locally: SendLocallyFn,
    reset: ResetFn,
    fns: UntypedMessageFns,
}

impl ClientMessage {
    pub(super) fn new<E: Message, I: 'static>(
        app: &mut App,
        channel: Channel,
        fns: MessageFns<ClientSendCtx, ServerReceiveCtx, E, I>,
    ) -> Self {
        let channel_id = app
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel);

        app.add_message::<E>()
            .add_message::<FromClient<E>>()
            .init_resource::<ClientMessageReader<E>>();

        let messages_id = app.world().resource_id::<Messages<E>>().unwrap();
        let from_messages_id = app
            .world()
            .resource_id::<Messages<FromClient<E>>>()
            .unwrap();
        let reader_id = app.world().resource_id::<ClientMessageReader<E>>().unwrap();

        Self {
            messages_id,
            reader_id,
            from_messages_id,
            channel_id,
            type_id: TypeId::of::<E>(),
            send: Self::send_typed::<E, I>,
            receive: Self::receive_typed::<E, I>,
            send_locally: Self::send_locally_typed::<E>,
            reset: Self::reset_typed::<E>,
            fns: fns.into(),
        }
    }

    pub(crate) fn messages_id(&self) -> ComponentId {
        self.messages_id
    }

    pub(crate) fn reader_id(&self) -> ComponentId {
        self.reader_id
    }

    pub(crate) fn from_messages_id(&self) -> ComponentId {
        self.from_messages_id
    }

    pub(super) fn channel_id(&self) -> usize {
        self.channel_id
    }

    pub(super) fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Sends a message to the server.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<E>`], `reader` is [`ClientMessageReader<E>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn send(
        &self,
        ctx: &mut ClientSendCtx,
        messages: &Ptr,
        reader: PtrMut,
        client_messages: &mut ClientMessages,
    ) {
        unsafe { (self.send)(self, ctx, messages, reader, client_messages) };
    }

    /// Typed version of [`Self::send`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<E>`], `reader` is [`ClientMessageReader<E>`],
    /// and this instance was created for `E` and `I`.
    unsafe fn send_typed<E: Message, I: 'static>(
        &self,
        ctx: &mut ClientSendCtx,
        messages: &Ptr,
        reader: PtrMut,
        client_messages: &mut ClientMessages,
    ) {
        let reader: &mut ClientMessageReader<E> = unsafe { reader.deref_mut() };
        let messages = unsafe { messages.deref() };
        for message in reader.read(messages) {
            let mut message_bytes = Vec::new();
            if let Err(e) = unsafe { self.serialize::<E, I>(ctx, message, &mut message_bytes) } {
                error!(
                    "ignoring message `{}` that failed to serialize: {e}",
                    any::type_name::<E>()
                );
                continue;
            }

            debug!("sending message `{}`", any::type_name::<E>());
            client_messages.send(self.channel_id, message_bytes);
        }
    }

    /// Receives messages from a client.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `from_messages` is [`Messages<FromClient<E>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn receive(
        &self,
        ctx: &mut ServerReceiveCtx,
        from_messages: PtrMut,
        server_messages: &mut ServerMessages,
    ) {
        unsafe { (self.receive)(self, ctx, from_messages, server_messages) }
    }

    /// Typed version of [`Self::receive`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `from_messages` is [`Messages<FromClient<E>>`]
    /// and this instance was created for `E` and `I`.
    unsafe fn receive_typed<E: Message, I: 'static>(
        &self,
        ctx: &mut ServerReceiveCtx,
        from_messages: PtrMut,
        server_messages: &mut ServerMessages,
    ) {
        let from_messages: &mut Messages<FromClient<E>> = unsafe { from_messages.deref_mut() };
        for (client, mut message) in server_messages.receive(self.channel_id) {
            match unsafe { self.deserialize::<E, I>(ctx, &mut message) } {
                Ok(message) => {
                    debug!(
                        "writing message `{}` from client `{client}`",
                        any::type_name::<E>()
                    );
                    from_messages.write(FromClient {
                        client_id: client.into(),
                        message,
                    });
                }
                Err(e) => debug!(
                    "ignoring message `{}` from client `{client}` that failed to deserialize: {e}",
                    any::type_name::<E>()
                ),
            }
        }
    }

    /// Drains messages `E` and writes them as [`FromClient<E>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<E>`], `from_messages` is [`Messages<FromClient<E>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn send_locally(&self, from_messages: PtrMut, messages: PtrMut) {
        unsafe { (self.send_locally)(from_messages, messages) }
    }

    /// Typed version of [`ClientMessage::send_locally`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<E>`] and `from_messages` is [`Messages<FromClient<E>>`].
    unsafe fn send_locally_typed<E: Message>(from_messages: PtrMut, messages: PtrMut) {
        let from_messages: &mut Messages<FromClient<E>> = unsafe { from_messages.deref_mut() };
        let messages: &mut Messages<E> = unsafe { messages.deref_mut() };
        if !messages.is_empty() {
            debug!(
                "writing {} message(s) `{}` locally",
                messages.len(),
                any::type_name::<E>()
            );
            from_messages.write_batch(messages.drain().map(|message| FromClient {
                client_id: ClientId::Server,
                message,
            }));
        }
    }

    /// Drains all messages.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<E>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn reset(&self, messages: PtrMut) {
        unsafe { (self.reset)(messages) }
    }

    /// Typed version of [`ClientMessage::reset`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `messages` is [`Messages<E>`].
    unsafe fn reset_typed<E: Message>(messages: PtrMut) {
        let messages: &mut Messages<E> = unsafe { messages.deref_mut() };
        let drained_count = messages.drain().count();
        if drained_count > 0 {
            warn!(
                "discarded {drained_count} messages of type `{}` that were buffered before the connection",
                any::type_name::<E>()
            );
        }
    }

    /// Serializes a message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E` and `I`.
    unsafe fn serialize<E: 'static, I: 'static>(
        &self,
        ctx: &mut ClientSendCtx,
        message: &E,
        message_bytes: &mut Vec<u8>,
    ) -> Result<()> {
        unsafe {
            self.fns
                .typed::<ClientSendCtx, ServerReceiveCtx, E, I>()
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
    /// The caller must ensure that this instance was created for `E` and `I`.
    unsafe fn deserialize<E: 'static, I: 'static>(
        &self,
        ctx: &mut ServerReceiveCtx,
        message: &mut Bytes,
    ) -> Result<E> {
        unsafe {
            self.fns
                .typed::<ClientSendCtx, ServerReceiveCtx, E, I>()
                .deserialize(ctx, message)
        }
    }
}

/// Signature of client message sending functions.
type SendFn = unsafe fn(&ClientMessage, &mut ClientSendCtx, &Ptr, PtrMut, &mut ClientMessages);

/// Signature of client message receiving functions.
type ReceiveFn = unsafe fn(&ClientMessage, &mut ServerReceiveCtx, PtrMut, &mut ServerMessages);

/// Signature of client message sending functions.
type SendLocallyFn = unsafe fn(PtrMut, PtrMut);

/// Signature of client message reset functions.
type ResetFn = unsafe fn(PtrMut);

/// Tracks read messages for [`ClientMessagePlugin::send`].
///
/// Unlike with server messages, we don't always drain all messages in [`ClientMessagePlugin::send_locally`].
#[derive(Resource, Deref, DerefMut)]
struct ClientMessageReader<E: Message>(MessageCursor<E>);

impl<E: Message> FromWorld for ClientMessageReader<E> {
    fn from_world(world: &mut World) -> Self {
        let messages = world.resource::<Messages<E>>();
        Self(messages.get_cursor())
    }
}

/// A remote message from a client.
///
/// Emitted only on server.
#[derive(Message, Event, Deref, DerefMut, Debug, Clone, Copy)]
pub struct FromClient<T> {
    /// Sender of the message.
    ///
    /// See also [`ConnectedClient`].
    pub client_id: ClientId,

    /// Transmitted message.
    #[deref]
    pub message: T,
}

/// Default message serialization function.
pub fn default_serialize<E: Serialize>(
    _ctx: &mut ClientSendCtx,
    message: &E,
    message_bytes: &mut Vec<u8>,
) -> Result<()> {
    postcard_utils::to_extend_mut(message, message_bytes)?;
    Ok(())
}

/// Like [`default_serialize`], but also maps entities.
pub fn default_serialize_mapped<E: MapEntities + Clone + Serialize>(
    ctx: &mut ClientSendCtx,
    message: &E,
    message_bytes: &mut Vec<u8>,
) -> Result<()> {
    let mut message = message.clone();
    message.map_entities(ctx);
    postcard_utils::to_extend_mut(&message, message_bytes)?;
    Ok(())
}

/// Default message deserialization function.
pub fn default_deserialize<E: DeserializeOwned>(
    _ctx: &mut ServerReceiveCtx,
    message: &mut Bytes,
) -> Result<E> {
    let message = postcard_utils::from_buf(message)?;
    Ok(message)
}
