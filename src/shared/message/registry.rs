use core::any::TypeId;

use bevy::prelude::*;

use super::{
    client_event::ClientEvent, client_message::ClientMessage, server_event::ServerEvent,
    server_message::ServerMessage,
};

/// Registered server and client messages and events.
#[derive(Resource, Default)]
pub struct RemoteMessageRegistry {
    // We use store events separately for quick iteration over them,
    // but they are messages under the hood.
    server_messages: Vec<ServerMessage>,
    client_messages: Vec<ClientMessage>,
    server_events: Vec<ServerEvent>,
    client_events: Vec<ClientEvent>,
}

impl RemoteMessageRegistry {
    pub(super) fn register_server_message(&mut self, message: ServerMessage) {
        self.server_messages.push(message);
    }

    pub(super) fn register_client_message(&mut self, message: ClientMessage) {
        self.client_messages.push(message);
    }

    pub(super) fn register_server_event(&mut self, event: ServerEvent) {
        self.server_events.push(event);
    }

    pub(super) fn register_client_event(&mut self, event: ClientEvent) {
        self.client_events.push(event);
    }

    pub(super) fn iter_server_messages_mut(&mut self) -> impl Iterator<Item = &mut ServerMessage> {
        self.server_messages.iter_mut()
    }

    pub(super) fn iter_server_events_mut(&mut self) -> impl Iterator<Item = &mut ServerEvent> {
        self.server_events.iter_mut()
    }

    pub(crate) fn iter_all_server(&self) -> impl Iterator<Item = &ServerMessage> {
        self.server_messages
            .iter()
            .chain(self.server_events.iter().map(|e| e.message()))
    }

    pub(crate) fn iter_all_client(&self) -> impl Iterator<Item = &ClientMessage> {
        self.client_messages
            .iter()
            .chain(self.client_events.iter().map(|e| e.message()))
    }

    pub(crate) fn iter_server_events(&self) -> impl Iterator<Item = &ServerEvent> {
        self.server_events.iter()
    }

    pub(crate) fn iter_client_events(&self) -> impl Iterator<Item = &ClientEvent> {
        self.client_events.iter()
    }

    /// Returns registered channel ID for server message `M`.
    ///
    /// See also [`ServerMessageAppExt::add_server_message`](super::server_message::ServerMessageAppExt::add_server_message).
    pub fn server_message_channel<M: Message>(&self) -> Option<usize> {
        self.server_messages
            .iter()
            .find(|m| m.type_id() == TypeId::of::<M>())
            .map(|m| m.channel_id())
    }

    /// Returns registered channel ID for server event `E`.
    ///
    /// See also [`ServerEventAppExt::add_server_event`](super::server_event::ServerEventAppExt::add_server_event).
    pub fn server_event_channel<E: Event>(&self) -> Option<usize> {
        self.server_events
            .iter()
            .find(|e| e.type_id() == TypeId::of::<E>())
            .map(|e| e.message().channel_id())
    }

    /// Returns registered channel ID for client message `M`.
    ///
    /// See also [`ClientMessageAppExt::add_client_message`](super::client_message::ClientMessageAppExt::add_client_message).
    pub fn client_message_channel<M: Message>(&self) -> Option<usize> {
        self.client_messages
            .iter()
            .find(|m| m.type_id() == TypeId::of::<M>())
            .map(|m| m.channel_id())
    }

    /// Returns registered channel ID for client event `E`.
    ///
    /// See also [`ClientEventAppExt::add_client_event`](super::client_event::ClientEventAppExt::add_client_event).
    pub fn client_event_channel<E: Event>(&self) -> Option<usize> {
        self.client_events
            .iter()
            .find(|e| e.type_id() == TypeId::of::<E>())
            .map(|e| e.message().channel_id())
    }
}
