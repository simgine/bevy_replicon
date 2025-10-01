use core::fmt::{self, Display, Formatter};

use bevy::prelude::*;

/// Unique client ID for the current session.
///
/// Used for [`ToClients`](super::message::server_message::ToClients) and
/// [`FromClient`](super::message::client_message::FromClient).
///
/// See also [`NetworkId`](super::backend::connected_client::NetworkId) for a persistent identifier.
#[derive(Reflect, Debug, Hash, PartialEq, Eq, Ord, PartialOrd, Clone, Copy)]
pub enum ClientId {
    /// Connected client entity.
    Client(Entity),
    /// Server that is also a client (listen server).
    Server,
}

impl ClientId {
    /// Returns associated entity for [`Self::Client`].
    pub fn entity(self) -> Option<Entity> {
        match self {
            ClientId::Client(entity) => Some(entity),
            ClientId::Server => None,
        }
    }
}

impl From<Entity> for ClientId {
    fn from(value: Entity) -> Self {
        Self::Client(value)
    }
}

impl Display for ClientId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ClientId::Client(entity) => entity.fmt(f),
            ClientId::Server => write!(f, "Server"),
        }
    }
}
