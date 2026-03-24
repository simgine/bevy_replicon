use bevy::ptr::Ptr;
use log::debug;
use postcard::experimental::max_size::MaxSize;

use super::{
    message_buffer::{MessageBuffer, SerializedMessage},
    *,
};

impl ServerMessage {
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
    pub(super) unsafe fn send_or_buffer_typed<M: Message, I: 'static>(
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
            debug!("sending message `{}` with `{mode:?}`", ShortName::of::<M>());

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
                for client in clients {
                    server_messages.send(client, self.channel_id, message_bytes.clone());
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
}

/// Signature of server message sending functions.
pub(super) type SendOrBufferFn = unsafe fn(
    &ServerMessage,
    &mut ServerSendCtx,
    &Ptr,
    &mut ServerMessages,
    &Query<Entity, With<ConnectedClient>>,
    &mut MessageBuffer,
);
