#[cfg(feature = "client_diagnostics")]
pub mod diagnostics;
pub mod message;

pub use crate::shared::replication::receive::{
    ServerUpdateTick, confirm_history, server_mutate_ticks,
};

use bevy::prelude::*;
use log::{Level, debug, error, log_enabled};

use crate::{
    prelude::*,
    shared::{
        replication::{
            receive::{
                BufferedMutations,
                confirm_history::EntityReplicated,
                receive_replication, reset,
                server_mutate_ticks::{MutateTickReceived, ServerMutateTicks},
            },
            track_mutate_messages::TrackMutateMessages,
        },
        server_entity_map::ServerEntityMap,
    },
};

/// Client functionality and replication receiving.
///
/// Can be disabled for server-only apps.
pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientMessages>()
            .init_resource::<ClientStats>()
            .init_resource::<ServerEntityMap>()
            .init_resource::<ServerUpdateTick>()
            .init_resource::<BufferedMutations>()
            .add_message::<EntityReplicated>()
            .add_message::<MutateTickReceived>()
            .configure_sets(
                PreUpdate,
                (
                    ClientSystems::ReceivePackets,
                    ClientSystems::Receive,
                    ClientSystems::Diagnostics,
                )
                    .chain(),
            )
            .configure_sets(
                OnEnter(ClientState::Connected),
                (ClientSystems::Receive, ClientSystems::Diagnostics).chain(),
            )
            .configure_sets(
                PostUpdate,
                (ClientSystems::Send, ClientSystems::SendPackets).chain(),
            )
            .add_systems(
                PreUpdate,
                receive_replication
                    .in_set(ClientSystems::Receive)
                    .run_if(in_state(ClientState::Connected)),
            )
            .add_systems(
                OnEnter(ClientState::Connected),
                receive_replication.in_set(ClientSystems::Receive),
            )
            .add_systems(
                OnExit(ClientState::Connected),
                reset.in_set(ClientSystems::Reset),
            );

        let auth_method = *app.world().resource::<AuthMethod>();
        debug!("using authorization method `{auth_method:?}`");
        if auth_method == AuthMethod::ProtocolCheck {
            app.add_observer(log_protocol_error).add_systems(
                OnEnter(ClientState::Connected),
                send_protocol_hash.in_set(ClientSystems::SendHash),
            );
        }

        if log_enabled!(Level::Debug) {
            app.add_systems(OnEnter(ClientState::Disconnected), || {
                debug!("disconnected")
            })
            .add_systems(OnEnter(ClientState::Connecting), || debug!("connecting"))
            .add_systems(OnEnter(ClientState::Connected), || debug!("connected"));
        }
    }

    fn finish(&self, app: &mut App) {
        if **app.world().resource::<TrackMutateMessages>() {
            app.init_resource::<ServerMutateTicks>();
        }

        app.world_mut()
            .resource_scope(|world, mut messages: Mut<ClientMessages>| {
                let channels = world.resource::<RepliconChannels>();
                messages.setup_server_channels(channels.server_channels().len());
            });
    }
}

fn send_protocol_hash(mut commands: Commands, protocol: Res<ProtocolHash>) {
    debug!("sending `{:?}` to the server", *protocol);
    commands.client_trigger(*protocol);
}

fn log_protocol_error(_on: On<ProtocolMismatch>) {
    error!(
        "server reported protocol mismatch; make sure replication rules and events registration order match with the server"
    );
}

/// Set with replication and event systems related to client.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum ClientSystems {
    /// Systems that receive packets from the messaging backend and update [`ClientState`].
    ///
    /// Used by messaging backend implementations.
    ///
    /// Runs in [`PreUpdate`].
    ReceivePackets,
    /// Systems that read data from [`ClientMessages`].
    ///
    /// Runs in [`PreUpdate`] and [`OnEnter`] for [`ClientState::Connected`] (to avoid 1 frame delay).
    Receive,
    /// Systems that populate Bevy's [`Diagnostics`](bevy::diagnostic::Diagnostics).
    ///
    /// Runs in [`PreUpdate`] and [`OnEnter`] for [`ClientState::Connected`] (to avoid 1 frame delay).
    Diagnostics,
    /// System that sends [`ProtocolHash`].
    ///
    /// Runs in [`OnEnter`] for [`ClientState::Connected`].
    SendHash,
    /// Systems that write data to [`ClientMessages`].
    ///
    /// Runs in [`PostUpdate`].
    Send,
    /// Systems that send packets to the messaging backend.
    ///
    /// Used by messaging backend implementations.
    ///
    /// Runs in [`PostUpdate`].
    SendPackets,
    /// Systems that reset the client.
    ///
    /// Runs in [`OnExit`] for [`ClientState::Connected`].
    Reset,
}

/// Replication stats during message processing.
///
/// Statistic will be collected only if the resource is present.
/// The resource is not added by default.
///
/// See also [`ClientDiagnosticsPlugin`]
/// for automatic integration with Bevy diagnostics.
#[derive(Resource, Default, Reflect, Debug, Clone, Copy)]
pub struct ClientReplicationStats {
    /// Incremented per entity that changes.
    pub entities_changed: usize,
    /// Incremented for every component that changes.
    pub components_changed: usize,
    /// Incremented per client mapping added.
    pub mappings: usize,
    /// Incremented per entity despawn.
    pub despawns: usize,
    /// Replication messages received.
    pub messages: usize,
    /// Replication bytes received in message payloads (without internal messaging plugin data).
    pub bytes: usize,
}

/// Marker for entities spawned by replication.
///
/// Automatically inserted for each newly received entity.
///
/// See also [`Replicated`].
#[derive(Component, Default, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct Remote;
