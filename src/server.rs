pub mod message;
pub mod server_tick;
pub mod visibility;

use core::time::Duration;

use bevy::{
    ecs::{entity::EntityHashMap, intern::Interned, schedule::ScheduleLabel},
    platform::collections::HashSet,
    prelude::*,
    time::common_conditions::on_timer,
};
use log::{Level, debug, log_enabled, trace};

use crate::{
    prelude::*,
    server::{
        server_tick::ServerTick, visibility::client_visibility::ClientVisibility,
        visibility::registry::FilterRegistry,
    },
    shared::{
        message::server_message::message_buffer::MessageBuffer,
        replication::{
            rules::ReplicationRules,
            send::{
                DespawnBuffer, ServerChangeTick, buffer_despawn, buffer_removals,
                check_mutation_ticks, cleanup_acks,
                client_pools::ClientPools,
                client_ticks::ClientTicks,
                collect_changes, collect_despawns, collect_mappings, collect_removals,
                prepare_messages, receive_acks,
                related_entities::RelatedEntities,
                removal_buffer::RemovalBuffer,
                replicated_archetypes::ReplicatedArchetypes,
                replication_messages::{
                    mutations::Mutations, serialized_data::SerializedData, updates::Updates,
                },
                send_messages,
            },
        },
    },
};

pub struct ServerPlugin {
    /// Schedule in which [`ServerTick`] is incremented.
    ///
    /// By default it's set to [`FixedPostUpdate`].
    /// Use [`Self::new`] to avoid calling [`ScheduleLabel::intern`].
    ///
    /// You can also set it to `None` to trigger replication by manually
    /// incrementing [`ServerTick`] or scheduling [`increment_tick`].
    ///
    /// # Examples
    ///
    /// Run every frame.
    ///
    /// ```
    /// use bevy::{ecs::schedule::ScheduleLabel, prelude::*, state::app::StatesPlugin};
    /// use bevy_replicon::prelude::*;
    ///
    /// # let mut app = App::new();
    /// app.add_plugins((
    ///     MinimalPlugins,
    ///     StatesPlugin,
    ///     RepliconPlugins.build().set(ServerPlugin {
    ///         // `ScheduleLabel` needs to be imported to call `intern`.
    ///         tick_schedule: Some(PostUpdate.intern()),
    ///         ..Default::default()
    ///     }),
    /// ));
    /// ```
    pub tick_schedule: Option<Interned<dyn ScheduleLabel>>,

    /// The time after which mutations will be considered lost if an acknowledgment is not received for them.
    ///
    /// In practice mutations will live at least `mutations_timeout`, and at most `2*mutations_timeout`.
    pub mutations_timeout: Duration,
}

impl ServerPlugin {
    /// Creates a plugin with the given [`Self::tick_schedule`].
    pub fn new(tick_schedule: impl ScheduleLabel) -> Self {
        Self {
            tick_schedule: Some(tick_schedule.intern()),
            mutations_timeout: Duration::from_secs(10),
        }
    }
}

impl Default for ServerPlugin {
    fn default() -> Self {
        Self::new(FixedPostUpdate)
    }
}

/// Server functionality and replication sending.
///
/// Can be disabled for client-only apps.
impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnBuffer>()
            .init_resource::<RemovalBuffer>()
            .init_resource::<SerializedData>()
            .init_resource::<ServerMessages>()
            .init_resource::<ServerTick>()
            .init_resource::<ServerChangeTick>()
            .init_resource::<ClientPools>()
            .init_resource::<ReplicatedArchetypes>()
            .init_resource::<MessageBuffer>()
            .init_resource::<RelatedEntities>()
            .init_resource::<FilterRegistry>()
            .configure_sets(
                PreUpdate,
                (ServerSystems::ReceivePackets, ServerSystems::Receive).chain(),
            )
            .configure_sets(
                PostUpdate,
                (
                    ServerSystems::IncrementTick,
                    ServerSystems::Send,
                    ServerSystems::SendPackets,
                )
                    .chain(),
            )
            .add_observer(handle_connect)
            .add_observer(handle_disconnect)
            .add_observer(check_mutation_ticks)
            .add_observer(buffer_despawn)
            .add_systems(
                PreUpdate,
                (
                    receive_acks,
                    cleanup_acks(self.mutations_timeout).run_if(on_timer(self.mutations_timeout)),
                )
                    .chain()
                    .in_set(ServerSystems::Receive)
                    .run_if(in_state(ServerState::Running)),
            )
            .add_systems(OnExit(ServerState::Running), reset)
            .add_systems(
                PostUpdate,
                (
                    prepare_messages,
                    collect_mappings,
                    collect_despawns,
                    collect_removals,
                    collect_changes,
                    send_messages,
                )
                    .chain()
                    .run_if(resource_changed::<ServerTick>)
                    .in_set(ServerSystems::Send)
                    .run_if(in_state(ServerState::Running)),
            );

        if let Some(tick_schedule) = self.tick_schedule {
            debug!("using tick schedule `{tick_schedule:?}`");
            app.add_systems(
                tick_schedule,
                increment_tick
                    .in_set(ServerSystems::IncrementTick)
                    .run_if(in_state(ServerState::Running)),
            );
        }

        let auth_method = app.world().resource::<AuthMethod>();
        debug!("using authorization method `{auth_method:?}`");
        match auth_method {
            AuthMethod::ProtocolCheck => {
                app.add_observer(check_protocol);
            }
            AuthMethod::None => {
                app.register_required_components::<ConnectedClient, AuthorizedClient>();
            }
            AuthMethod::Custom => (),
        }

        if log_enabled!(Level::Debug) {
            app.add_systems(OnEnter(ServerState::Running), || debug!("running"))
                .add_systems(OnEnter(ServerState::Stopped), || debug!("stopped"));
        }
    }

    fn finish(&self, app: &mut App) {
        // Multiple rules can include components with the same ID,
        // we collect them here to deduplicate.
        let rules = app.world().resource::<ReplicationRules>();
        let replicated_ids: HashSet<_> = rules
            .iter()
            .flat_map(|rule| &rule.components)
            .map(|component| component.id)
            .collect();

        // Removal observer without any components will trigger on any removal.
        if !replicated_ids.is_empty() {
            let mut remove_observer = Observer::new(buffer_removals);
            for id in replicated_ids {
                remove_observer = remove_observer.with_component(id);
            }
            app.world_mut().spawn(remove_observer);
        }

        app.world_mut()
            .resource_scope(|world, mut messages: Mut<ServerMessages>| {
                let channels = world.resource::<RepliconChannels>();
                messages.setup_client_channels(channels.client_channels().len());
            });
    }
}

fn handle_connect(add: On<Add, ConnectedClient>, mut message_buffer: ResMut<MessageBuffer>) {
    debug!("client `{}` connected", add.entity);
    message_buffer.exclude_client(add.entity);
}

fn handle_disconnect(remove: On<Remove, ConnectedClient>, mut messages: ResMut<ServerMessages>) {
    debug!("client `{}` disconnected", remove.entity);
    messages.remove_client(remove.entity);
}

fn check_protocol(
    client_protocol: On<FromClient<ProtocolHash>>,
    mut commands: Commands,
    mut disconnects: MessageWriter<DisconnectRequest>,
    protocol: Res<ProtocolHash>,
) {
    let client = client_protocol
        .client_id
        .entity()
        .expect("protocol hash sent only from clients");

    if **client_protocol == *protocol {
        debug!("marking client `{client}` as authorized");
        commands.entity(client).insert(AuthorizedClient);
    } else {
        debug!(
            "disconnecting client `{client}` due to protocol mismatch (client: `{:?}`, server: `{:?}`)",
            **client_protocol, *protocol
        );
        commands.server_trigger(ToClients {
            mode: SendMode::Direct(client_protocol.client_id),
            message: ProtocolMismatch,
        });
        disconnects.write(DisconnectRequest { client });
    }
}

/// Increments current server tick which causes the server to replicate this frame.
pub fn increment_tick(mut server_tick: ResMut<ServerTick>) {
    trace!("incrementing `{:?}`", *server_tick);
    server_tick.increment();
}

fn reset(
    mut commands: Commands,
    mut messages: ResMut<ServerMessages>,
    mut server_tick: ResMut<ServerTick>,
    mut related_entities: ResMut<RelatedEntities>,
    clients: Query<Entity, With<ConnectedClient>>,
    mut message_buffer: ResMut<MessageBuffer>,
) {
    messages.clear();
    *server_tick = Default::default();
    message_buffer.clear();
    related_entities.clear();
    for client in &clients {
        commands.entity(client).despawn();
    }
}

/// Set with replication and event systems related to server.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum ServerSystems {
    /// Systems that receive packets from the messaging backend and update [`ServerState`].
    ///
    /// Used by the messaging backend.
    ///
    /// Runs in [`PreUpdate`].
    ReceivePackets,
    /// Systems that read data from [`ServerMessages`].
    ///
    /// Runs in [`PreUpdate`].
    Receive,
    /// Systems that build the initial graph with all related entities registered via
    /// [`crate::shared::replication::send::related_entities::SyncRelatedAppExt::sync_related_entities`].
    ///
    /// The graph is kept in sync with observers.
    ///
    /// Runs in [`OnEnter`] for [`ServerState::Running`].
    ReadRelations,
    /// System that increments [`ServerTick`].
    ///
    /// Runs in [`ServerPlugin::tick_schedule`].
    IncrementTick,
    /// Systems that write data to [`ServerMessages`].
    ///
    /// Runs in [`PostUpdate`] if [`ServerTick`] changes.
    Send,
    /// Systems that send packets to the messaging backend.
    ///
    /// Used by the messaging backend.
    ///
    /// Runs in [`PostUpdate`] if [`ServerTick`] changes.
    SendPackets,
}

/// Marker that enables replication and all events for a client.
///
/// Until authorization happened, the client and server can still exchange network events that are marked as
/// independent via [`ServerMessageAppExt::make_message_independent`] or [`ServerEventAppExt::make_event_independent`].
/// **All other events will be ignored**.
///
/// See also [`ConnectedClient`] and [`RepliconSharedPlugin::auth_method`].
#[derive(Component, Reflect, Default)]
#[component(immutable)]
#[require(ClientTicks, ClientVisibility, PriorityMap, Updates, Mutations)]
pub struct AuthorizedClient;

/// Controls how often mutations are sent for an authorized client.
///
/// Associates entities with a priority number configurable by the user.
/// If the priority is not set, it defaults to 1.0.
///
/// During replication, we multiply the difference between the last acknowledged tick
/// and [`ServerTick`] by the priority. If the result is greater than or equal to 1.0,
/// we send mutations for this entity.
///
/// This means the priority accumulates across server ticks until an entity is acknowledged,
/// at which point its priority is reset. As a result, even low-priority objects eventually
/// reach a high enough priority to be considered for replication.
///
/// For example, if the base priority is 0.5, mutations for an entity will be sent
/// no more often than once every 2 ticks. With the default priority of 1.0,
/// all unacknowledged mutations will be sent every tick.
///
/// All of this only affects mutations. For any component insertion or removal, the changes
/// will be sent using [`ServerChannel::Updates`](crate::shared::backend::channels::ServerChannel::Updates).
/// See its documentation for more details.
#[derive(Component, Reflect, Deref, DerefMut, Default, Debug, Clone)]
pub struct PriorityMap(EntityHashMap<f32>);
