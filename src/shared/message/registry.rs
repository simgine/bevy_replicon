use core::any::TypeId;

use bevy::prelude::*;

use super::{
    client_event::ClientTrigger, client_message::ClientEvent, server_event::ServerTrigger,
    server_message::ServerEvent,
};

/// Registered server and client events.
#[derive(Resource, Default)]
pub struct RemoteEventRegistry {
    // We use store triggers separately for quick iteration over them,
    // but they are events under the hood.
    server_events: Vec<ServerEvent>,
    client_events: Vec<ClientEvent>,
    server_triggers: Vec<ServerTrigger>,
    client_triggers: Vec<ClientTrigger>,
}

impl RemoteEventRegistry {
    pub(super) fn register_server_event(&mut self, event: ServerEvent) {
        self.server_events.push(event);
    }

    pub(super) fn register_client_event(&mut self, event: ClientEvent) {
        self.client_events.push(event);
    }

    pub(super) fn register_server_trigger(&mut self, trigger: ServerTrigger) {
        self.server_triggers.push(trigger);
    }

    pub(super) fn register_client_trigger(&mut self, trigger: ClientTrigger) {
        self.client_triggers.push(trigger);
    }

    pub(super) fn iter_server_events_mut(&mut self) -> impl Iterator<Item = &mut ServerEvent> {
        self.server_events.iter_mut()
    }

    pub(super) fn iter_server_triggers_mut(&mut self) -> impl Iterator<Item = &mut ServerTrigger> {
        self.server_triggers.iter_mut()
    }

    pub(crate) fn iter_all_server(&self) -> impl Iterator<Item = &ServerEvent> {
        self.server_events
            .iter()
            .chain(self.server_triggers.iter().map(|trigger| trigger.event()))
    }

    pub(crate) fn iter_all_client(&self) -> impl Iterator<Item = &ClientEvent> {
        self.client_events
            .iter()
            .chain(self.client_triggers.iter().map(|trigger| trigger.event()))
    }

    pub(crate) fn iter_server_triggers(&self) -> impl Iterator<Item = &ServerTrigger> {
        self.server_triggers.iter()
    }

    pub(crate) fn iter_client_triggers(&self) -> impl Iterator<Item = &ClientTrigger> {
        self.client_triggers.iter()
    }

    /// Returns registered channel ID for server event or trigger `E`.
    ///
    /// See also [`ServerEventAppExt::add_server_event`](super::server_event::ServerEventAppExt::add_server_event)
    /// and [`ServerTriggerAppExt::add_server_trigger`](super::server_trigger::ServerTriggerAppExt::add_server_trigger).
    // TODO: typing
    pub fn server_channel<E: 'static>(&self) -> Option<usize> {
        self.iter_all_server()
            .find(|event| event.type_id() == TypeId::of::<E>())
            .map(|event| event.channel_id())
    }

    /// Returns registered channel ID for client event or trigger `E`.
    ///
    /// See also [`ClientEventAppExt::add_client_event`](super::client_event::ClientEventAppExt::add_client_event)
    /// and [`ClientTriggerAppExt::add_client_trigger`](super::client_trigger::ClientTriggerAppExt::add_client_trigger).
    // TODO: typing
    pub fn client_channel<E: 'static>(&self) -> Option<usize> {
        self.iter_all_client()
            .find(|event| event.type_id() == TypeId::of::<E>())
            .map(|event| event.channel_id())
    }
}
