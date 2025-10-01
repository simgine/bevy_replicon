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

    /// Returns registered channel ID for server message or event `E`.
    ///
    /// See also [`ServerMessageAppExt::add_server_message`](super::server_message::ServerMessageAppExt::add_server_message)
    /// and [`ServerEventAppExt::add_server_event`](super::server_event::ServerEventAppExt::add_server_event).
    // TODO: typing
    pub fn server_channel<E: 'static>(&self) -> Option<usize> {
        self.iter_all_server()
            .find(|m| m.type_id() == TypeId::of::<E>())
            .map(|m| m.channel_id())
    }

    /// Returns registered channel ID for client message or event `E`.
    ///
    /// See also [`ClientMessageAppExt::add_client_message`](super::client_message::ClientMessageAppExt::add_client_message)
    /// and [`ClientEventAppExt::add_client_event`](super::client_event::ClientEventAppExt::add_client_event).
    // TODO: typing
    pub fn client_channel<E: 'static>(&self) -> Option<usize> {
        self.iter_all_client()
            .find(|event| event.type_id() == TypeId::of::<E>())
            .map(|event| event.channel_id())
    }
}
