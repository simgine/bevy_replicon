mod client_event_queue;

use core::{
    any::{self, TypeId},
    mem,
};

use bevy::{
    ecs::{
        component::ComponentId,
        entity::{MapEntities, hash_set::EntityHashSet},
    },
    prelude::*,
    ptr::{Ptr, PtrMut},
};
use bytes::Bytes;
use client_event_queue::ClientEventQueue;
use log::{debug, error, warn};
use postcard::experimental::{max_size::MaxSize, serialized_size};
use serde::{Serialize, de::DeserializeOwned};

use super::{
    ctx::{ClientReceiveCtx, ServerSendCtx},
    event_fns::{EventDeserializeFn, EventFns, EventSerializeFn, UntypedEventFns},
    remote_event_registry::RemoteEventRegistry,
};
use crate::{
    prelude::*,
    shared::{postcard_utils, replication::client_ticks::ClientTicks},
};

/// An extension trait for [`App`] for creating server events.
///
/// See also [`ClientEventAppExt`] for client events and [`ServerTriggerAppExt`] for triggers.
pub trait ServerEventAppExt {
    /// Registers a remote server event.
    ///
    /// After emitting [`ToClients<E>`] event on the server, `E` event  will be emitted on clients.
    ///
    /// If [`ClientEventPlugin`] is enabled and [`SERVER`] is a recipient of the event, then
    /// [`ToClients<E>`] event will be drained after sending to clients and `E` event will be emitted
    /// on the server as well.
    ///
    /// Calling [`App::add_event`] is not necessary. Can used for regular events that were
    /// previously registered.
    ///
    /// Unlike client events, server events are tied to replication. See [`Self::make_event_independent`]
    /// for more details.
    ///
    /// See also the [corresponding section](../index.html#from-client-to-server) from the quick start guide.
    fn add_server_event<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_server_event_with(channel, default_serialize::<E>, default_deserialize::<E>)
    }

    /// Same as [`Self::add_server_event`], but additionally maps server entities to client inside the event after receiving.
    ///
    /// Always use it for events that contain entities. Entities must be annotated with `#[entities]`.
    /// For details, see [`Component::map_entities`].
    fn add_mapped_server_event<E: Event + Serialize + DeserializeOwned + MapEntities>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_server_event_with(
            channel,
            default_serialize::<E>,
            default_deserialize_mapped::<E>,
        )
    }

    /**
    Same as [`Self::add_server_event`], but uses the specified functions for serialization and deserialization.

    See also [`postcard_utils`] and [`ServerTriggerAppExt::add_server_trigger_with`].

    # Examples

    Register an event with [`Box<dyn PartialReflect>`]:

    ```
    use bevy::{
        prelude::*,
        reflect::serde::{ReflectDeserializer, ReflectSerializer},
    };
    use bevy_replicon::{
        bytes::Bytes,
        shared::{
            event::ctx::{ClientReceiveCtx, ServerSendCtx},
            postcard_utils::{BufFlavor, ExtendMutFlavor},
        },
        prelude::*,
    };
    use postcard::{Deserializer, Serializer};
    use serde::{de::DeserializeSeed, Serialize};

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RepliconPlugins));
    app.add_server_event_with(Channel::Ordered, serialize_reflect, deserialize_reflect);

    fn serialize_reflect(
        ctx: &mut ServerSendCtx,
        event: &ReflectEvent,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        let mut serializer = Serializer { output: ExtendMutFlavor::new(message) };
        let registry = ctx.type_registry.read();
        ReflectSerializer::new(&*event.0, &registry).serialize(&mut serializer)?;
        Ok(())
    }

    fn deserialize_reflect(
        ctx: &mut ClientReceiveCtx,
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
    fn add_server_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: EventSerializeFn<ServerSendCtx, E>,
        deserialize: EventDeserializeFn<ClientReceiveCtx, E>,
    ) -> &mut Self;

    /// Marks the event `E` as an independent event.
    ///
    /// By default, all server events are buffered on server until server tick
    /// and queued on client until all insertions, removals and despawns
    /// (value mutations doesn't count) are replicated for the tick on which the
    /// event was triggered. This is necessary to ensure that the executed logic
    /// during the event does not affect components or entities that the client
    /// has not yet received.
    ///
    /// For more details about replication see the documentation on
    /// [`ServerChannel`](crate::shared::backend::channels::ServerChannel).
    ///
    /// However, if you know your event doesn't rely on that, you can mark it
    /// as independent to always emit it immediately. For example, a chat
    /// message event - which does not hold references to any entities - may be
    /// marked as independent.
    ///
    /// <div class="warning">
    ///
    /// Use this method very carefully; it can lead to logic errors that are
    /// very difficult to debug!
    ///
    /// </div>
    ///
    /// See also [`ServerTriggerAppExt::make_event_independent`](crate::shared::event::server_trigger::ServerTriggerAppExt::make_trigger_independent).
    fn make_event_independent<E: Event>(&mut self) -> &mut Self;
}

impl ServerEventAppExt for App {
    fn add_server_event_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: EventSerializeFn<ServerSendCtx, E>,
        deserialize: EventDeserializeFn<ClientReceiveCtx, E>,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .add_server_event::<E>();

        let event_fns = EventFns::new(serialize, deserialize);
        let event = ServerEvent::new(self, channel, event_fns);
        let mut event_registry = self.world_mut().resource_mut::<RemoteEventRegistry>();
        event_registry.register_server_event(event);

        self
    }

    fn make_event_independent<E: Event>(&mut self) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .make_event_independent::<E>();

        let events_id = self
            .world()
            .components()
            .resource_id::<Events<E>>()
            .unwrap_or_else(|| {
                panic!(
                    "event `{}` should be previously registered",
                    any::type_name::<E>()
                )
            });

        let mut event_registry = self.world_mut().resource_mut::<RemoteEventRegistry>();
        let event = event_registry
            .iter_server_events_mut()
            .find(|event| event.events_id() == events_id)
            .unwrap_or_else(|| {
                panic!(
                    "event `{}` should be previously registered as a server event",
                    any::type_name::<E>()
                )
            });

        event.independent = true;

        self
    }
}

/// Type-erased functions and metadata for a registered server event.
///
/// Needed so events of different types can be processed together.
pub(crate) struct ServerEvent {
    /// Whether this event depends on replication or not.
    ///
    /// Events like a chat message event do not have to wait for replication to
    /// be synced. If set to `true`, the event will always be applied
    /// immediately.
    pub(super) independent: bool,

    /// ID of [`Events<E>`].
    events_id: ComponentId,

    /// ID of [`Events<ToClients<E>>`].
    server_events_id: ComponentId,

    /// ID of [`ServerEventQueue<T>`].
    queue_id: ComponentId,

    /// Used channel.
    channel_id: usize,

    /// ID of `E`.
    type_id: TypeId,

    send_or_buffer: SendOrBufferFn,
    receive: ReceiveFn,
    resend_locally: ResendLocallyFn,
    reset: ResetFn,
    event_fns: UntypedEventFns,
}

impl ServerEvent {
    pub(super) fn new<E: Event, I: 'static>(
        app: &mut App,
        channel: Channel,
        event_fns: EventFns<ServerSendCtx, ClientReceiveCtx, E, I>,
    ) -> Self {
        let channel_id = app
            .world_mut()
            .resource_mut::<RepliconChannels>()
            .create_server_channel(channel);

        app.add_event::<E>()
            .add_event::<ToClients<E>>()
            .init_resource::<ClientEventQueue<E>>();

        let events_id = app.world().resource_id::<Events<E>>().unwrap();
        let server_events_id = app.world().resource_id::<Events<ToClients<E>>>().unwrap();
        let queue_id = app.world().resource_id::<ClientEventQueue<E>>().unwrap();

        Self {
            independent: false,
            events_id,
            server_events_id,
            queue_id,
            channel_id,
            type_id: TypeId::of::<E>(),
            send_or_buffer: Self::send_or_buffer_typed::<E, I>,
            receive: Self::receive_typed::<E, I>,
            resend_locally: Self::resend_locally_typed::<E>,
            reset: Self::reset_typed::<E>,
            event_fns: event_fns.into(),
        }
    }

    pub(crate) fn events_id(&self) -> ComponentId {
        self.events_id
    }

    pub(crate) fn server_events_id(&self) -> ComponentId {
        self.server_events_id
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

    /// Sends an event to client(s).
    ///
    /// # Safety
    ///
    /// The caller must ensure that `server_events` is [`Events<ToClients<E>>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn send_or_buffer(
        &self,
        ctx: &mut ServerSendCtx,
        server_events: &Ptr,
        server: &mut RepliconServer,
        clients: &Query<Entity, With<ConnectedClient>>,
        buffered_events: &mut BufferedServerEvents,
    ) {
        unsafe { (self.send_or_buffer)(self, ctx, server_events, server, clients, buffered_events) }
    }

    /// Typed version of [`Self::send_or_buffer`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `server_events` is [`Events<ToClients<E>>`]
    /// and this instance was created for `E` and `I`.
    unsafe fn send_or_buffer_typed<E: Event, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        server_events: &Ptr,
        server: &mut RepliconServer,
        clients: &Query<Entity, With<ConnectedClient>>,
        buffered_events: &mut BufferedServerEvents,
    ) {
        let events: &Events<ToClients<E>> = unsafe { server_events.deref() };
        // For server events we don't track read events because
        // all of them will always be drained in the local resending system.
        for ToClients { event, mode } in events.get_cursor().read(events) {
            debug!("sending event `{}` with `{mode:?}`", any::type_name::<E>());

            if self.independent {
                unsafe {
                    self.send_independent_event::<E, I>(ctx, event, mode, server, clients)
                        .expect("independent server event should be serializable");
                }
            } else {
                unsafe {
                    self.buffer_event::<E, I>(ctx, event, *mode, buffered_events)
                        .expect("server event should be serializable");
                }
            }
        }
    }

    /// Sends independent event `E` based on a mode.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E` and `I`.
    ///
    /// For regular events see [`Self::buffer_event`].
    unsafe fn send_independent_event<E: Event, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        event: &E,
        mode: &SendMode,
        server: &mut RepliconServer,
        clients: &Query<Entity, With<ConnectedClient>>,
    ) -> Result<()> {
        let mut message = Vec::new();
        unsafe { self.serialize::<E, I>(ctx, event, &mut message)? }
        let message: Bytes = message.into();

        match *mode {
            SendMode::Broadcast => {
                for client_entity in clients {
                    server.send(client_entity, self.channel_id, message.clone());
                }
            }
            SendMode::BroadcastExcept(ignored_id) => {
                for client in clients {
                    if ignored_id != client.into() {
                        server.send(client, self.channel_id, message.clone());
                    }
                }
            }
            SendMode::Direct(client_id) => {
                if let ClientId::Client(client) = client_id {
                    server.send(client, self.channel_id, message.clone());
                }
            }
        }

        Ok(())
    }

    /// Buffers event `E` based on a mode.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E` and `I`.
    ///
    /// For independent events see [`Self::send_independent_event`].
    unsafe fn buffer_event<E: Event, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        event: &E,
        mode: SendMode,
        buffered_events: &mut BufferedServerEvents,
    ) -> Result<()> {
        let message = unsafe { self.serialize_with_padding::<E, I>(ctx, event)? };
        buffered_events.insert(mode, self.channel_id, message);
        Ok(())
    }

    /// Helper for serializing a server event.
    ///
    /// Will prepend padding bytes for where the update tick will be inserted to the injected message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E` and `I`.
    unsafe fn serialize_with_padding<E: Event, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        event: &E,
    ) -> Result<SerializedMessage> {
        let mut message = vec![0; RepliconTick::POSTCARD_MAX_SIZE]; // Padding for the tick.
        unsafe { self.serialize::<E, I>(ctx, event, &mut message)? }
        let message = SerializedMessage::Raw(message);

        Ok(message)
    }

    /// Receives events from the server.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `queue` is [`ServerEventQueue<E>`],
    /// and this instance was created for `E`.
    pub(crate) unsafe fn receive(
        &self,
        ctx: &mut ClientReceiveCtx,
        events: PtrMut,
        queue: PtrMut,
        client: &mut RepliconClient,
        update_tick: RepliconTick,
    ) {
        unsafe { (self.receive)(self, ctx, events, queue, client, update_tick) }
    }

    /// Typed version of [`ServerEvent::receive`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `queue` is [`ServerEventQueue<E>`]
    /// and this instance was created for `E` and `I`.
    unsafe fn receive_typed<E: Event, I: 'static>(
        &self,
        ctx: &mut ClientReceiveCtx,
        events: PtrMut,
        queue: PtrMut,
        client: &mut RepliconClient,
        update_tick: RepliconTick,
    ) {
        let events: &mut Events<E> = unsafe { events.deref_mut() };
        let queue: &mut ClientEventQueue<E> = unsafe { queue.deref_mut() };

        while let Some((tick, messages)) = queue.pop_if_le(update_tick) {
            for mut message in messages {
                match unsafe { self.deserialize::<E, I>(ctx, &mut message) } {
                    Ok(event) => {
                        debug!(
                            "applying event `{}` from queue with `{tick:?}`",
                            any::type_name::<E>()
                        );
                        events.send(event);
                    }
                    Err(e) => error!(
                        "ignoring event `{}` from queue with `{tick:?}` that failed to deserialize: {e}",
                        any::type_name::<E>()
                    ),
                }
            }
        }

        for mut message in client.receive(self.channel_id) {
            if !self.independent {
                let tick = match postcard_utils::from_buf(&mut message) {
                    Ok(tick) => tick,
                    Err(e) => {
                        error!(
                            "ignoring event `{}` because it's tick failed to deserialize: {e}",
                            any::type_name::<E>()
                        );
                        continue;
                    }
                };
                if tick > update_tick {
                    debug!("queuing event `{}` with `{tick:?}`", any::type_name::<E>());
                    queue.insert(tick, message);
                    continue;
                } else {
                    debug!(
                        "receiving event `{}` with `{tick:?}`",
                        any::type_name::<E>()
                    );
                }
            }

            match unsafe { self.deserialize::<E, I>(ctx, &mut message) } {
                Ok(event) => {
                    debug!("applying event `{}`", any::type_name::<E>());
                    events.send(event);
                }
                Err(e) => error!(
                    "ignoring event `{}` that failed to deserialize: {e}",
                    any::type_name::<E>()
                ),
            }
        }
    }

    /// Drains events [`ToClients<E>`] and re-emits them as `E` if the server is in the list of the event recipients.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`], `server_events` is [`Events<ToClients<E>>`],
    /// and this instance was created for `E`.
    pub(crate) unsafe fn resend_locally(&self, server_events: PtrMut, events: PtrMut) {
        unsafe { (self.resend_locally)(server_events, events) }
    }

    /// Typed version of [`Self::resend_locally`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<E>`] and `server_events` is [`Events<ToClients<E>>`].
    unsafe fn resend_locally_typed<E: Event>(server_events: PtrMut, events: PtrMut) {
        let server_events: &mut Events<ToClients<E>> = unsafe { server_events.deref_mut() };
        let events: &mut Events<E> = unsafe { events.deref_mut() };
        for ToClients { event, mode } in server_events.drain() {
            debug!("resending event `{}` locally", any::type_name::<E>());
            match mode {
                SendMode::Broadcast => {
                    events.send(event);
                }
                SendMode::BroadcastExcept(ignored_id) => {
                    if ignored_id != ClientId::Server {
                        events.send(event);
                    }
                }
                SendMode::Direct(client_id) => {
                    if client_id == ClientId::Server {
                        events.send(event);
                    }
                }
            }
        }
    }

    /// Clears queued events.
    ///
    /// We clear events while waiting for a connection to ensure clean reconnects.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `queue` is [`Events<E>`]
    /// and this instance was created for `E`.
    pub(crate) unsafe fn reset(&self, queue: PtrMut) {
        unsafe { (self.reset)(queue) }
    }

    /// Typed version of [`Self::reset`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `queue` is [`Events<E>`].
    unsafe fn reset_typed<E: Event>(queue: PtrMut) {
        let queue: &mut ClientEventQueue<E> = unsafe { queue.deref_mut() };
        if !queue.is_empty() {
            warn!(
                "discarding {} queued events due to a disconnect",
                queue.len()
            );
        }
        queue.clear();
    }

    /// Serializes an event into a message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E` and `I`.
    unsafe fn serialize<E: Event, I: 'static>(
        &self,
        ctx: &mut ServerSendCtx,
        event: &E,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        unsafe {
            self.event_fns
                .typed::<ServerSendCtx, ClientReceiveCtx, E, I>()
                .serialize(ctx, event, message)
        }
    }

    /// Deserializes an event from a message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this instance was created for `E` and `I`.
    unsafe fn deserialize<E: Event, I: 'static>(
        &self,
        ctx: &mut ClientReceiveCtx,
        message: &mut Bytes,
    ) -> Result<E> {
        let event = unsafe {
            self.event_fns
                .typed::<ServerSendCtx, ClientReceiveCtx, E, I>()
                .deserialize(ctx, message)?
        };

        if ctx.invalid_entities.is_empty() {
            Ok(event)
        } else {
            let msg = format!(
                "unable to map entities `{:?}` from the server, \
                make sure that the event references entities visible to the client",
                ctx.invalid_entities
            );
            ctx.invalid_entities.clear();
            Err(msg.into())
        }
    }
}

/// Signature of server event sending functions.
type SendOrBufferFn = unsafe fn(
    &ServerEvent,
    &mut ServerSendCtx,
    &Ptr,
    &mut RepliconServer,
    &Query<Entity, With<ConnectedClient>>,
    &mut BufferedServerEvents,
);

/// Signature of server event receiving functions.
type ReceiveFn = unsafe fn(
    &ServerEvent,
    &mut ClientReceiveCtx,
    PtrMut,
    PtrMut,
    &mut RepliconClient,
    RepliconTick,
);

/// Signature of server event resending functions.
type ResendLocallyFn = unsafe fn(PtrMut, PtrMut);

/// Signature of server event reset functions.
type ResetFn = unsafe fn(PtrMut);

/// Cached message for use in [`BufferedServerEvents`].
enum SerializedMessage {
    /// A message without serialized tick.
    ///
    /// `padding | message`
    ///
    /// The padding length equals max serialized bytes of [`RepliconTick`]. It should be overwritten before sending
    /// to clients.
    Raw(Vec<u8>),
    /// A message with serialized tick.
    ///
    /// `tick | message`
    Resolved {
        tick: RepliconTick,
        tick_size: usize,
        bytes: Bytes,
    },
}

impl SerializedMessage {
    /// Optimized to avoid reallocations when clients have the same update tick as other clients receiving the
    /// same message.
    fn get_bytes(&mut self, update_tick: RepliconTick) -> Result<Bytes> {
        match self {
            // Resolve the raw value into a message with serialized tick.
            Self::Raw(raw) => {
                let mut bytes = mem::take(raw);

                // Serialize the tick at the end of the pre-allocated space for it,
                // then shift the buffer to avoid reallocation.
                let tick_size = serialized_size(&update_tick)?;
                let padding = RepliconTick::POSTCARD_MAX_SIZE - tick_size;
                postcard::to_slice(&update_tick, &mut bytes[padding..])?;
                let bytes = Bytes::from(bytes).slice(padding..);

                *self = Self::Resolved {
                    tick: update_tick,
                    tick_size,
                    bytes: bytes.clone(),
                };
                Ok(bytes)
            }
            // Get the already-resolved value or reserialize with a different tick.
            Self::Resolved {
                tick,
                tick_size,
                bytes,
            } => {
                if *tick == update_tick {
                    return Ok(bytes.clone());
                }

                let new_tick_size = serialized_size(&update_tick)?;
                let mut new_bytes = Vec::with_capacity(new_tick_size + bytes.len() - *tick_size);
                postcard_utils::to_extend_mut(&update_tick, &mut new_bytes)?;
                new_bytes.extend_from_slice(&bytes[*tick_size..]);
                Ok(new_bytes.into())
            }
        }
    }
}

struct BufferedServerEvent {
    mode: SendMode,
    channel_id: usize,
    message: SerializedMessage,
}

impl BufferedServerEvent {
    fn send(
        &mut self,
        server: &mut RepliconServer,
        client_entity: Entity,
        client: &ClientTicks,
    ) -> Result<()> {
        let message = self.message.get_bytes(client.update_tick())?;
        server.send(client_entity, self.channel_id, message);
        Ok(())
    }
}

#[derive(Default)]
struct BufferedServerEventSet {
    events: Vec<BufferedServerEvent>,
    /// Client entities excluded from receiving events in this set because they connected after the events were sent.
    excluded: EntityHashSet,
}

impl BufferedServerEventSet {
    fn clear(&mut self) {
        self.events.clear();
        self.excluded.clear();
    }
}

/// Caches synchronization-dependent server events until they can be sent with an accurate update tick.
///
/// This exists because replication does not scan the world every tick. If a server event is sent in the same
/// tick as a spawn and the event references that spawn, then the server event's update tick needs to be synchronized
/// with that spawn on the client. We buffer the event until the spawn can be detected.
#[derive(Resource, Default)]
pub(crate) struct BufferedServerEvents {
    buffer: Vec<BufferedServerEventSet>,

    /// Caches unused sets to avoid reallocations when pushing into the buffer.
    ///
    /// These are cleared before insertion.
    cache: Vec<BufferedServerEventSet>,
}

impl BufferedServerEvents {
    pub(crate) fn start_tick(&mut self) {
        self.buffer.push(self.cache.pop().unwrap_or_default());
    }

    fn active_tick(&mut self) -> Option<&mut BufferedServerEventSet> {
        self.buffer.last_mut()
    }

    fn insert(&mut self, mode: SendMode, channel_id: usize, message: SerializedMessage) {
        let buffer = self
            .active_tick()
            .expect("`BufferedServerEvents::start_tick` should be called before buffering");

        buffer.events.push(BufferedServerEvent {
            mode,
            channel_id,
            message,
        });
    }

    /// Used to prevent newly-connected clients from receiving old events.
    pub(crate) fn exclude_client(&mut self, client: Entity) {
        for set in self.buffer.iter_mut() {
            set.excluded.insert(client);
        }
    }

    pub(crate) fn send_all(
        &mut self,
        server: &mut RepliconServer,
        clients: &Query<(Entity, Option<&ClientTicks>), With<ConnectedClient>>,
    ) -> Result<()> {
        for mut set in self.buffer.drain(..) {
            for mut event in set.events.drain(..) {
                match event.mode {
                    SendMode::Broadcast => {
                        for (client, ticks) in
                            clients.iter().filter(|(e, _)| !set.excluded.contains(e))
                        {
                            if let Some(ticks) = ticks {
                                event.send(server, client, ticks)?;
                            } else {
                                debug!(
                                    "ignoring broadcast for channel {} for non-authorized client `{client}`",
                                    event.channel_id
                                );
                            }
                        }
                    }
                    SendMode::BroadcastExcept(ignored_id) => {
                        for (client, ticks) in
                            clients.iter().filter(|(c, _)| !set.excluded.contains(c))
                        {
                            if ignored_id == client.into() {
                                continue;
                            }

                            if let Some(ticks) = ticks {
                                event.send(server, client, ticks)?;
                            } else {
                                debug!(
                                    "ignoring broadcast except `{ignored_id}` for channel {} for non-authorized client `{client}`",
                                    event.channel_id
                                );
                            }
                        }
                    }
                    SendMode::Direct(client_id) => {
                        if let ClientId::Client(client) = client_id
                            && !set.excluded.contains(&client)
                        {
                            if let Ok((_, ticks)) = clients.get(client) {
                                if let Some(ticks) = ticks {
                                    event.send(server, client, ticks)?;
                                } else {
                                    error!(
                                        "ignoring direct event for non-authorized client `{client}`, \
                                         mark it as independent to allow this"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            set.clear();
            self.cache.push(set);
        }
        Ok(())
    }

    pub(crate) fn clear(&mut self) {
        for mut set in self.buffer.drain(..) {
            set.clear();
            self.cache.push(set);
        }
    }
}

/// An event that will be send to client(s).
#[derive(Clone, Copy, Debug, Event, Deref, DerefMut)]
pub struct ToClients<T> {
    /// Recipients.
    pub mode: SendMode,
    /// Transmitted event.
    #[deref]
    pub event: T,
}

/// Type of server event sending.
#[derive(Clone, Copy, Debug)]
pub enum SendMode {
    /// Send to every client.
    Broadcast,
    /// Send to every client except the specified connected client.
    BroadcastExcept(ClientId),
    /// Send only to the specified client.
    Direct(ClientId),
}

/// Default event serialization function.
pub fn default_serialize<E: Event + Serialize>(
    _ctx: &mut ServerSendCtx,
    event: &E,
    message: &mut Vec<u8>,
) -> Result<()> {
    postcard_utils::to_extend_mut(event, message)?;
    Ok(())
}

/// Default event deserialization function.
pub fn default_deserialize<E: Event + DeserializeOwned>(
    _ctx: &mut ClientReceiveCtx,
    message: &mut Bytes,
) -> Result<E> {
    let event = postcard_utils::from_buf(message)?;
    Ok(event)
}

/// Default event deserialization function.
pub fn default_deserialize_mapped<E: Event + DeserializeOwned + MapEntities>(
    ctx: &mut ClientReceiveCtx,
    bytes: &mut Bytes,
) -> Result<E> {
    let mut event: E = postcard_utils::from_buf(bytes)?;
    event.map_entities(ctx);

    Ok(event)
}
