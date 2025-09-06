use core::any::{self, TypeId};

use bevy::{
    ecs::{component::ComponentId, entity::MapEntities, event::EventCursor},
    prelude::*,
    ptr::{Ptr, PtrMut},
};
use bytes::Bytes;
use log::{debug, error, warn};
use serde::{Serialize, de::DeserializeOwned};

use super::{
    ctx::{ClientSendCtx, ServerReceiveCtx},
    event_fns::{EventDeserializeFn, EventFns, EventSerializeFn, UntypedEventFns},
    remote_event_registry::RemoteEventRegistry,
};
use crate::{postcard_utils, prelude::*};

/// An extension trait for [`App`] for creating client events.
///
/// See also [`ServerEventAppExt`] for server events and [`ClientTriggerAppExt`] for triggers.
pub trait ClientEventAppExt {
    /// Registers a remote client event.
    ///
    /// After emitting `E` event on the client, [`FromClient<E>`] event will be emitted on the server.
    ///
    /// If [`ServerEventPlugin`] is enabled and the client state is [`ClientState::Disconnected`], the event will be drained
    /// right after sending and re-emitted locally as [`FromClient<E>`] event with [`FromClient::client_id`]
    /// equal to [`ClientId::Server`].
    ///
    /// Calling [`App::add_event`] is not necessary. Can used for regular events that were
    /// previously registered. But be careful, since on listen servers all events `E` are drained,
    /// which could break other Bevy or third-party plugin systems that listen for `E`.
    ///
    /// See also the [corresponding section](../index.html#from-client-to-server) from the quick start guide.
    fn add_client_event<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_client_event_with(channel, default_serialize::<E>, default_deserialize::<E>)
    }

    /// Same as [`Self::add_client_event`], but additionally maps client entities to server inside the event before sending.
    ///
    /// Always use it for events that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    ///
    /// [`Clone`] is required because, before sending, we need to map entities from the client to the server without
    /// modifying the original component.
    fn add_mapped_client_event<E: Event + Serialize + DeserializeOwned + MapEntities + Clone>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_client_event_with(
            channel,
            default_serialize_mapped::<E>,
            default_deserialize::<E>,
        )
    }

    /**
    Same as [`Self::add_client_event`], but uses the specified functions for serialization and deserialization.

    See also [`postcard_utils`] and [`ClientTriggerAppExt::add_client_trigger_with`].

    # Examples

    Register an event with [`Box<dyn PartialReflect>`]:

    ```
    use bevy::{
        prelude::*,
        reflect::serde::{ReflectDeserializer, ReflectSerializer},
        state::app::StatesPlugin,
    };
    use bevy_replicon::{
        bytes::Bytes,
        postcard_utils::{BufFlavor, ExtendMutFlavor},
        shared::event::ctx::{ClientSendCtx, ServerReceiveCtx},
        prelude::*,
    };
    use postcard::{Deserializer, Serializer};
    use serde::{de::DeserializeSeed, Serialize};

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));
    app.add_client_event_with(Channel::Ordered, serialize_reflect, deserialize_reflect);

    fn serialize_reflect(
        ctx: &mut ClientSendCtx,
        event: &ReflectEvent,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        let mut serializer = Serializer { output: ExtendMutFlavor::new(message) };
        let registry = ctx.type_registry.read();
        ReflectSerializer::new(&*event.0, &registry).serialize(&mut serializer)?;
        Ok(())
    }

    fn deserialize_reflect(
        ctx: &mut ServerReceiveCtx,
        message: &mut Bytes,
    ) -> Result<ReflectEvent> {
        let mut deserializer = Deserializer::from_flavor(BufFlavor::new(message));
        let registry = ctx.type_registry.read();
        let reflect = ReflectDeserializer::new(&registry).deserialize(&mut deserializer)?;
        Ok(ReflectEvent(reflect))
    }

    #[derive(Event)]
    struct ReflectEvent(Box<dyn PartialReflect>);
    ```
    */
    fn add_client_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: EventSerializeFn<ClientSendCtx, E>,
        deserialize: EventDeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self;
}

impl ClientEventAppExt for App {
    fn add_client_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: EventSerializeFn<ClientSendCtx, E>,
        deserialize: EventDeserializeFn<ServerReceiveCtx, E>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_client_event::<E>();

        let event_fns = EventFns::new(serialize, deserialize);
        let event = ClientEvent::new(self, channel, event_fns);
        let mut event_registry = self.world_mut().resource_mut::<RemoteEventRegistry>();
        event_registry.register_client_event(event);

        self
    }
}

/// Type-erased functions and metadata for a registered client event.
///
/// Needed so events of different types can be processed together.
pub(crate) struct ClientEvent {
    /// ID of [`Events<E>`] resource.
    events_id: ComponentId,

    /// ID of [`ClientEventReader<E>`] resource.
    reader_id: ComponentId,

    /// ID of [`Events<FromClient<E>>`] resource.
    client_events_id: ComponentId,

    /// Used channel.
    channel_id: usize,

    /// ID of `E`.
    type_id: TypeId,

    send: SendFn,
    receive: ReceiveFn,
    resend_locally: ResendLocallyFn,
    reset: ResetFn,
    event_fns: UntypedEventFns,
}

impl ClientEvent {
    pub(super) fn new<E: Event, I: 'static>(
        app: &mut App,
        channel: Channel,
        event_fns: EventFns<ClientSendCtx, ServerReceiveCtx, E, I>,
    ) -> Self {
        let channel_id = app
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_client_channel(channel);

        app.add_event::<E>()
            .add_event::<FromClient<E>>()
            .init_resource::<ClientEventReader<E>>();

        let events_id = app.world().resource_id::<Events<E>>().unwrap();
        let client_events_id = app.world().resource_id::<Events<FromClient<E>>>().unwrap();
        let reader_id = app.world().resource_id::<ClientEventReader<E>>().unwrap();

        Self {
            events_id,
            reader_id,
            client_events_id,
            channel_id,
            type_id: TypeId::of::<E>(),
            send: Self::send_typed::<E, I>,
            receive: Self::receive_typed::<E, I>,
            resend_locally: Self::resend_locally_typed::<E>,
            reset: Self::reset_typed::<E>,
            event_fns: event_fns.into(),
        }
    }

    pub(crate) fn events_id(&self) -> ComponentId {
        self.events_id
    }

    pub(crate) fn reader_id(&self) -> ComponentId {
        self.reader_id
    }

    pub(crate) fn client_events_id(&self) -> ComponentId {
        self.client_events_id
    }

    pub(super) fn channel_id(&self) -> usize {
        self.channel_id
    }

    pub(super) fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Sends an event to the server.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `reader` is [`ClientEventReader<E>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn send(
        &self,
        ctx: &mut ClientSendCtx,
        events: &Ptr,
        reader: PtrMut,
        messages: &mut ClientMessages,
    ) {
        unsafe { (self.send)(self, ctx, events, reader, messages) };
    }

    /// Typed version of [`Self::send`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `reader` is [`ClientEventReader<E>`],
    /// and this instance was created for `E` and `I`.
    unsafe fn send_typed<E: Event, I: 'static>(
        &self,
        ctx: &mut ClientSendCtx,
        events: &Ptr,
        reader: PtrMut,
        messages: &mut ClientMessages,
    ) {
        let reader: &mut ClientEventReader<E> = unsafe { reader.deref_mut() };
        let events = unsafe { events.deref() };
        for event in reader.read(events) {
            let mut message = Vec::new();
            if let Err(e) = unsafe { self.serialize::<E, I>(ctx, event, &mut message) } {
                error!(
                    "ignoring event `{}` that failed to serialize: {e}",
                    any::type_name::<E>()
                );
                continue;
            }

            debug!("sending event `{}`", any::type_name::<E>());
            messages.send(self.channel_id, message);
        }
    }

    /// Receives events from a client.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `client_events` is [`Events<FromClient<E>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn receive(
        &self,
        ctx: &mut ServerReceiveCtx,
        client_events: PtrMut,
        messages: &mut ServerMessages,
    ) {
        unsafe { (self.receive)(self, ctx, client_events, messages) }
    }

    /// Typed version of [`Self::receive`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `client_events` is [`Events<FromClient<E>>`]
    /// and this instance was created for `E` and `I`.
    unsafe fn receive_typed<E: Event, I: 'static>(
        &self,
        ctx: &mut ServerReceiveCtx,
        client_events: PtrMut,
        messages: &mut ServerMessages,
    ) {
        let client_events: &mut Events<FromClient<E>> = unsafe { client_events.deref_mut() };
        for (client, mut message) in messages.receive(self.channel_id) {
            match unsafe { self.deserialize::<E, I>(ctx, &mut message) } {
                Ok(event) => {
                    debug!(
                        "applying event `{}` from client `{client}`",
                        any::type_name::<E>()
                    );
                    client_events.send(FromClient {
                        client_id: client.into(),
                        event,
                    });
                }
                Err(e) => debug!(
                    "ignoring event `{}` from client `{client}` that failed to deserialize: {e}",
                    any::type_name::<E>()
                ),
            }
        }
    }

    /// Drains events `E` and re-emits them as [`FromClient<E>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `client_events` is [`Events<FromClient<E>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn resend_locally(&self, client_events: PtrMut, events: PtrMut) {
        unsafe { (self.resend_locally)(client_events, events) }
    }

    /// Typed version of [`ClientEvent::resend_locally`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`] and `server_events` is [`Events<ToClients<E>>`].
    unsafe fn resend_locally_typed<E: Event>(server_events: PtrMut, events: PtrMut) {
        let client_events: &mut Events<FromClient<E>> = unsafe { server_events.deref_mut() };
        let events: &mut Events<E> = unsafe { events.deref_mut() };
        if !events.is_empty() {
            debug!(
                "resending {} event(s) `{}` locally",
                events.len(),
                any::type_name::<E>()
            );
            client_events.send_batch(events.drain().map(|event| FromClient {
                client_id: ClientId::Server,
                event,
            }));
        }
    }

    /// Drains all events.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn reset(&self, events: PtrMut) {
        unsafe { (self.reset)(events) }
    }

    /// Typed version of [`ClientEvent::reset`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`].
    unsafe fn reset_typed<E: Event>(events: PtrMut) {
        let events: &mut Events<E> = unsafe { events.deref_mut() };
        let drained_count = events.drain().count();
        if drained_count > 0 {
            warn!(
                "discarded {drained_count} events of type `{}` that were buffered before the connection",
                any::type_name::<E>()
            );
        }
    }

    /// Serializes an event into a message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E` and `I`.
    unsafe fn serialize<E: 'static, I: 'static>(
        &self,
        ctx: &mut ClientSendCtx,
        event: &E,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        unsafe {
            self.event_fns
                .typed::<ClientSendCtx, ServerReceiveCtx, E, I>()
                .serialize(ctx, event, message)?;
        }

        if ctx.invalid_entities.is_empty() {
            Ok(())
        } else {
            let msg = format!(
                "unable to map entities `{:?}` for the server, \
                make sure that the event references entities visible to the server",
                ctx.invalid_entities,
            );
            ctx.invalid_entities.clear();
            Err(msg.into())
        }
    }

    /// Deserializes an event from a message.
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
            self.event_fns
                .typed::<ClientSendCtx, ServerReceiveCtx, E, I>()
                .deserialize(ctx, message)
        }
    }
}

/// Signature of client event sending functions.
type SendFn = unsafe fn(&ClientEvent, &mut ClientSendCtx, &Ptr, PtrMut, &mut ClientMessages);

/// Signature of client event receiving functions.
type ReceiveFn = unsafe fn(&ClientEvent, &mut ServerReceiveCtx, PtrMut, &mut ServerMessages);

/// Signature of client event resending functions.
type ResendLocallyFn = unsafe fn(PtrMut, PtrMut);

/// Signature of client event reset functions.
type ResetFn = unsafe fn(PtrMut);

/// Tracks read events for [`ClientEventPlugin::send`].
///
/// Unlike with server events, we don't always drain all events in [`ClientEventPlugin::resend_locally`].
#[derive(Resource, Deref, DerefMut)]
struct ClientEventReader<E: Event>(EventCursor<E>);

impl<E: Event> FromWorld for ClientEventReader<E> {
    fn from_world(world: &mut World) -> Self {
        let events = world.resource::<Events<E>>();
        Self(events.get_cursor())
    }
}

/// An event indicating that a message from client was received.
///
/// Emitted only on server.
#[derive(Clone, Copy, Event, Deref, DerefMut)]
pub struct FromClient<T> {
    /// Sender of the event.
    ///
    /// See also [`ConnectedClient`].
    pub client_id: ClientId,

    /// Transmitted event.
    #[deref]
    pub event: T,
}

/// Default event serialization function.
pub fn default_serialize<E: Event + Serialize>(
    _ctx: &mut ClientSendCtx,
    event: &E,
    message: &mut Vec<u8>,
) -> Result<()> {
    postcard_utils::to_extend_mut(event, message)?;
    Ok(())
}

/// Like [`default_serialize`], but also maps entities.
pub fn default_serialize_mapped<E: Event + MapEntities + Clone + Serialize>(
    ctx: &mut ClientSendCtx,
    event: &E,
    message: &mut Vec<u8>,
) -> Result<()> {
    let mut event = event.clone();
    event.map_entities(ctx);
    postcard_utils::to_extend_mut(&event, message)?;
    Ok(())
}

/// Default event deserialization function.
pub fn default_deserialize<E: Event + DeserializeOwned>(
    _ctx: &mut ServerReceiveCtx,
    message: &mut Bytes,
) -> Result<E> {
    let event = postcard_utils::from_buf(message)?;
    Ok(event)
}
