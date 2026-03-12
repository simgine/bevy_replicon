use core::mem;

use bevy::{ecs::entity::hash_set::EntityHashSet, prelude::*};
use bytes::Bytes;
use log::{debug, error};
use postcard::experimental::{max_size::MaxSize, serialized_size};

use crate::{postcard_utils, prelude::*, shared::replication::client_ticks::ClientTicks};

/// Caches synchronization-dependent server messages until they can be sent with an accurate update tick.
///
/// This exists because replication does not scan the world every tick. If a server message is sent in the same
/// tick as a spawn and the message references that spawn, then the server message's update tick needs to be synchronized
/// with that spawn on the client. We buffer the message until the spawn can be detected.
#[derive(Resource, Default)]
pub(crate) struct MessageBuffer {
    ticks: Vec<TickMessages>,

    /// Cached unused sets to avoid reallocations when pushing into the buffer.
    ///
    /// These are cleared before insertion.
    pool: Vec<TickMessages>,
}

impl MessageBuffer {
    pub(crate) fn start_tick(&mut self) {
        self.ticks.push(self.pool.pop().unwrap_or_default());
    }

    fn active_tick(&mut self) -> Option<&mut TickMessages> {
        self.ticks.last_mut()
    }

    pub(super) fn insert(&mut self, mode: SendMode, channel_id: usize, message: SerializedMessage) {
        let buffer = self
            .active_tick()
            .expect("`start_tick` should be called before buffering");

        buffer.messages.push(BufferedMessage {
            mode,
            channel_id,
            message,
        });
    }

    /// Used to prevent newly-connected clients from receiving old messages.
    pub(crate) fn exclude_client(&mut self, client: Entity) {
        for set in self.ticks.iter_mut() {
            set.excluded.insert(client);
        }
    }

    pub(crate) fn send_all(
        &mut self,
        messages: &mut ServerMessages,
        clients: &Query<(Entity, Option<&ClientTicks>), With<ConnectedClient>>,
    ) -> Result<()> {
        for mut tick in self.ticks.drain(..) {
            for mut message in tick.messages.drain(..) {
                match message.mode {
                    SendMode::Broadcast => {
                        for (client, ticks) in
                            clients.iter().filter(|(e, _)| !tick.excluded.contains(e))
                        {
                            if let Some(ticks) = ticks {
                                message.send(messages, client, ticks)?;
                            } else {
                                debug!(
                                    "ignoring broadcast for channel {} for non-authorized client `{client}`",
                                    message.channel_id
                                );
                            }
                        }
                    }
                    SendMode::BroadcastExcept(ignored_id) => {
                        for (client, ticks) in
                            clients.iter().filter(|(c, _)| !tick.excluded.contains(c))
                        {
                            if ignored_id == client.into() {
                                continue;
                            }

                            if let Some(ticks) = ticks {
                                message.send(messages, client, ticks)?;
                            } else {
                                debug!(
                                    "ignoring broadcast except `{ignored_id}` for channel {} for non-authorized client `{client}`",
                                    message.channel_id
                                );
                            }
                        }
                    }
                    SendMode::Direct(client_id) => {
                        if let ClientId::Client(client) = client_id
                            && let Ok((_, ticks)) = clients.get(client)
                            && !tick.excluded.contains(&client)
                        {
                            if let Some(ticks) = ticks {
                                message.send(messages, client, ticks)?;
                            } else {
                                error!(
                                    "ignoring direct message for non-authorized client `{client}`, \
                                         mark it as independent to allow this"
                                );
                            }
                        }
                    }
                }
            }
            tick.clear();
            self.pool.push(tick);
        }
        Ok(())
    }

    pub(crate) fn clear(&mut self) {
        for mut set in self.ticks.drain(..) {
            set.clear();
            self.pool.push(set);
        }
    }
}

#[derive(Default)]
struct TickMessages {
    messages: Vec<BufferedMessage>,
    /// Client entities excluded from receiving messages in this set because they connected after the messages were sent.
    excluded: EntityHashSet,
}

impl TickMessages {
    fn clear(&mut self) {
        self.messages.clear();
        self.excluded.clear();
    }
}

struct BufferedMessage {
    mode: SendMode,
    channel_id: usize,
    message: SerializedMessage,
}

impl BufferedMessage {
    fn send(
        &mut self,
        messages: &mut ServerMessages,
        client: Entity,
        ticks: &ClientTicks,
    ) -> Result<()> {
        let message = self.message.get_bytes(ticks.update_tick)?;
        messages.send(client, self.channel_id, message);
        Ok(())
    }
}

/// Cached message for use in [`MessageBuffer`].
pub(super) enum SerializedMessage {
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
