pub mod client_entity_map;
pub mod client_visibility;
pub mod event;
pub mod related_entities;
pub(super) mod removal_buffer;
pub(super) mod replication_messages;
pub mod server_tick;
mod server_world;

use core::{ops::Range, time::Duration};

use bevy::{
    ecs::{
        archetype::Archetypes, component::StorageType, entity::Entities, system::SystemChangeTick,
    },
    prelude::*,
    ptr::Ptr,
    time::common_conditions::on_timer,
};
use bytes::Buf;
use log::{debug, trace};

use crate::{
    postcard_utils,
    prelude::*,
    shared::{
        backend::channels::ClientChannel,
        event::server_event::BufferedServerEvents,
        replication::{
            client_ticks::{ClientTicks, EntityBuffer},
            registry::{
                ReplicationRegistry, component_fns::ComponentFns, ctx::SerializeCtx,
                rule_fns::UntypedRuleFns,
            },
            rules::{ReplicationRules, component::ComponentRule},
            track_mutate_messages::TrackMutateMessages,
        },
    },
};
use client_visibility::Visibility;
use related_entities::RelatedEntities;
use removal_buffer::{RemovalBuffer, RemovalReader};
use replication_messages::{
    mutations::Mutations, serialized_data::SerializedData, updates::Updates,
};
use server_tick::ServerTick;
use server_world::ServerWorld;

pub struct ServerPlugin {
    /// Tick configuration.
    ///
    /// By default it's 30 ticks per second.
    pub tick_policy: TickPolicy,

    /// Visibility configuration.
    pub visibility_policy: VisibilityPolicy,

    /// The time after which mutations will be considered lost if an acknowledgment is not received for them.
    ///
    /// In practice mutations will live at least `mutations_timeout`, and at most `2*mutations_timeout`.
    pub mutations_timeout: Duration,
}

impl Default for ServerPlugin {
    fn default() -> Self {
        Self {
            tick_policy: TickPolicy::MaxTickRate(30),
            visibility_policy: Default::default(),
            mutations_timeout: Duration::from_secs(10),
        }
    }
}

/// Server functionality and replication sending.
///
/// Can be disabled for client-only apps.
impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DespawnBuffer>()
            .init_resource::<RemovalBuffer>()
            .init_resource::<RepliconServer>()
            .init_resource::<ServerTick>()
            .init_resource::<EntityBuffer>()
            .init_resource::<BufferedServerEvents>()
            .init_resource::<RelatedEntities>()
            .configure_sets(
                PreUpdate,
                (ServerSet::ReceivePackets, ServerSet::Receive).chain(),
            )
            .configure_sets(
                PostUpdate,
                (
                    ServerSet::PrepareSend,
                    ServerSet::Send,
                    ServerSet::SendPackets,
                )
                    .chain(),
            )
            .add_observer(handle_connects)
            .add_observer(handle_disconnects)
            .add_observer(buffer_despawns)
            .add_systems(
                PreUpdate,
                (
                    receive_acks,
                    cleanup_acks(self.mutations_timeout).run_if(on_timer(self.mutations_timeout)),
                )
                    .chain()
                    .in_set(ServerSet::Receive)
                    .run_if(server_running),
            )
            .add_systems(
                PostUpdate,
                (
                    (
                        buffer_removals,
                        send_replication.run_if(resource_changed::<ServerTick>),
                    )
                        .chain()
                        .in_set(ServerSet::Send)
                        .run_if(server_running),
                    reset.run_if(server_just_stopped),
                ),
            );

        debug!("using tick policy `{:?}`", self.tick_policy);
        match self.tick_policy {
            TickPolicy::MaxTickRate(max_tick_rate) => {
                let tick_time = Duration::from_millis(1000 / max_tick_rate as u64);
                app.add_systems(
                    PostUpdate,
                    increment_tick
                        .before(send_replication)
                        .run_if(server_running)
                        .run_if(on_timer(tick_time)),
                );
            }
            TickPolicy::EveryFrame => {
                app.add_systems(
                    PostUpdate,
                    increment_tick
                        .before(send_replication)
                        .run_if(server_running),
                );
            }
            TickPolicy::Manual => (),
        }

        debug!("using visibility policy `{:?}`", self.visibility_policy);
        match self.visibility_policy {
            VisibilityPolicy::All => {}
            VisibilityPolicy::Blacklist => {
                app.register_required_components_with::<AuthorizedClient, _>(
                    ClientVisibility::blacklist,
                );
            }
            VisibilityPolicy::Whitelist => {
                app.register_required_components_with::<AuthorizedClient, _>(
                    ClientVisibility::whitelist,
                );
            }
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
    }

    fn finish(&self, app: &mut App) {
        app.world_mut()
            .resource_scope(|world, mut server: Mut<RepliconServer>| {
                let channels = world.resource::<RepliconChannels>();
                server.setup_client_channels(channels.client_channels().len());
            });
    }
}

/// Increments current server tick which causes the server to replicate this frame.
pub fn increment_tick(mut server_tick: ResMut<ServerTick>) {
    server_tick.increment();
    trace!("incremented {server_tick:?}");
}

fn handle_connects(
    trigger: Trigger<OnAdd, ConnectedClient>,
    mut buffered_events: ResMut<BufferedServerEvents>,
) {
    debug!("client `{}` connected", trigger.target());
    buffered_events.exclude_client(trigger.target());
}

fn handle_disconnects(
    trigger: Trigger<OnRemove, ConnectedClient>,
    mut server: ResMut<RepliconServer>,
) {
    debug!("client `{}` disconnected", trigger.target());
    server.remove_client(trigger.target());
}

fn check_protocol(
    trigger: Trigger<FromClient<ProtocolHash>>,
    mut commands: Commands,
    mut events: EventWriter<DisconnectRequest>,
    protocol: Res<ProtocolHash>,
) {
    let client = trigger
        .client_id
        .entity()
        .expect("protocol hash sent only from clients");

    if **trigger == *protocol {
        debug!("marking client `{client}` as authorized");
        commands.entity(client).insert(AuthorizedClient);
    } else {
        debug!(
            "disconnecting client `{client}` due to protocol mismatch (client: `{:?}`, server: `{:?}`)",
            **trigger, *protocol
        );
        commands.server_trigger(ToClients {
            mode: SendMode::Direct(trigger.client_id),
            event: ProtocolMismatch,
        });
        events.write(DisconnectRequest { client });
    }
}

fn cleanup_acks(
    mutations_timeout: Duration,
) -> impl FnMut(Query<&mut ClientTicks>, ResMut<EntityBuffer>, Res<Time>) {
    move |mut clients: Query<&mut ClientTicks>,
          mut entity_buffer: ResMut<EntityBuffer>,
          time: Res<Time>| {
        let min_timestamp = time.elapsed().saturating_sub(mutations_timeout);
        for mut ticks in &mut clients {
            ticks.cleanup_older_mutations(&mut entity_buffer, min_timestamp);
        }
    }
}

fn receive_acks(
    change_tick: SystemChangeTick,
    mut server: ResMut<RepliconServer>,
    mut clients: Query<&mut ClientTicks>,
    mut entity_buffer: ResMut<EntityBuffer>,
) {
    for (client, mut message) in server.receive(ClientChannel::MutationAcks) {
        while message.has_remaining() {
            match postcard_utils::from_buf(&mut message) {
                Ok(mutate_index) => {
                    let mut ticks = clients.get_mut(client).unwrap_or_else(|_| {
                        panic!(
                            "messages from client `{client}` should have been removed on disconnect"
                        )
                    });
                    ticks.ack_mutate_message(
                        client,
                        &mut entity_buffer,
                        change_tick.this_run(),
                        mutate_index,
                    );
                }
                Err(e) => {
                    debug!("unable to deserialize mutate index from client `{client}`: {e}")
                }
            }
        }
    }
}

fn buffer_despawns(
    trigger: Trigger<OnRemove, Replicated>,
    mut despawn_buffer: ResMut<DespawnBuffer>,
    server: Res<RepliconServer>,
) {
    if server.is_running() {
        despawn_buffer.push(trigger.target());
    }
}

fn buffer_removals(
    entities: &Entities,
    archetypes: &Archetypes,
    mut removal_reader: RemovalReader,
    mut removal_buffer: ResMut<RemovalBuffer>,
    rules: Res<ReplicationRules>,
) {
    for (&entity, removed_components) in removal_reader.read() {
        let location = entities
            .get(entity)
            .expect("removals count only existing entities");
        let archetype = archetypes.get(location.archetype_id).unwrap();

        removal_buffer.update(&rules, archetype, entity, removed_components);
    }
}

/// Collects [`ReplicationMessages`] and sends them.
fn send_replication(
    mut serialized: Local<SerializedData>,
    change_tick: SystemChangeTick,
    world: ServerWorld,
    mut clients: Query<(
        Entity,
        &mut Updates,
        &mut Mutations,
        &ConnectedClient,
        &mut ClientEntityMap,
        &mut ClientTicks,
        Option<&mut ClientVisibility>,
    )>,
    mut related_entities: ResMut<RelatedEntities>,
    mut removal_buffer: ResMut<RemovalBuffer>,
    mut entity_buffer: ResMut<EntityBuffer>,
    mut despawn_buffer: ResMut<DespawnBuffer>,
    mut server: ResMut<RepliconServer>,
    track_mutate_messages: Res<TrackMutateMessages>,
    registry: Res<ReplicationRegistry>,
    type_registry: Res<AppTypeRegistry>,
    server_tick: Res<ServerTick>,
    time: Res<Time>,
) -> Result<()> {
    related_entities.rebuild_graphs();

    for (_, mut updates, mut mutations, ..) in &mut clients {
        updates.clear();
        mutations.clear();
        mutations.resize_related(related_entities.graphs_count());
    }

    collect_mappings(&mut serialized, &mut clients)?;
    collect_despawns(&mut serialized, &mut clients, &mut despawn_buffer)?;
    collect_removals(&mut serialized, &mut clients, &removal_buffer)?;
    collect_changes(
        &mut serialized,
        &mut clients,
        &registry,
        &type_registry,
        &related_entities,
        &removal_buffer,
        &world,
        &change_tick,
        **server_tick,
    )?;
    removal_buffer.clear();

    send_messages(
        &mut clients,
        &mut server,
        **server_tick,
        **track_mutate_messages,
        &mut serialized,
        &mut entity_buffer,
        change_tick,
        &time,
    )?;
    serialized.clear();

    Ok(())
}

fn reset(
    mut commands: Commands,
    mut server_tick: ResMut<ServerTick>,
    mut related_entities: ResMut<RelatedEntities>,
    clients: Query<Entity, With<ConnectedClient>>,
    mut buffered_events: ResMut<BufferedServerEvents>,
) {
    *server_tick = Default::default();
    buffered_events.clear();
    related_entities.clear();
    for entity in &clients {
        commands.entity(entity).despawn();
    }
}

fn send_messages(
    clients: &mut Query<(
        Entity,
        &mut Updates,
        &mut Mutations,
        &ConnectedClient,
        &mut ClientEntityMap,
        &mut ClientTicks,
        Option<&mut ClientVisibility>,
    )>,
    server: &mut RepliconServer,
    server_tick: RepliconTick,
    track_mutate_messages: bool,
    serialized: &mut SerializedData,
    entity_buffer: &mut EntityBuffer,
    change_tick: SystemChangeTick,
    time: &Time,
) -> Result<()> {
    let mut server_tick_range = None;
    for (client_entity, updates, mut mutations, client, .., mut ticks, visibility) in clients {
        if !updates.is_empty() {
            ticks.set_update_tick(server_tick);
            let server_tick = write_tick_cached(&mut server_tick_range, serialized, server_tick)?;

            updates.send(server, client_entity, serialized, server_tick)?;
        }

        if !mutations.is_empty() || track_mutate_messages {
            let server_tick = write_tick_cached(&mut server_tick_range, serialized, server_tick)?;

            mutations.send(
                server,
                client_entity,
                &mut ticks,
                entity_buffer,
                serialized,
                track_mutate_messages,
                server_tick,
                change_tick.this_run(),
                time.elapsed(),
                client.max_size,
            )?;
        }

        if let Some(mut visibility) = visibility {
            visibility.update();
        }
    }

    Ok(())
}

/// Collects and writes any new entity mappings that happened in this tick.
fn collect_mappings(
    serialized: &mut SerializedData,
    clients: &mut Query<(
        Entity,
        &mut Updates,
        &mut Mutations,
        &ConnectedClient,
        &mut ClientEntityMap,
        &mut ClientTicks,
        Option<&mut ClientVisibility>,
    )>,
) -> Result<()> {
    for (client_entity, mut message, _, _, mut entity_map, ..) in clients {
        if entity_map.is_empty() {
            continue;
        }

        trace!("writing mappings for client `{client_entity}`");
        let len = entity_map.len();
        let mappings = serialized.write_mappings(entity_map.0.drain(..))?;
        message.set_mappings(mappings, len);
    }

    Ok(())
}

/// Collect entity despawns from this tick into update messages.
fn collect_despawns(
    serialized: &mut SerializedData,
    clients: &mut Query<(
        Entity,
        &mut Updates,
        &mut Mutations,
        &ConnectedClient,
        &mut ClientEntityMap,
        &mut ClientTicks,
        Option<&mut ClientVisibility>,
    )>,
    despawn_buffer: &mut DespawnBuffer,
) -> Result<()> {
    for entity in despawn_buffer.drain(..) {
        let entity_range = serialized.write_entity(entity)?;
        for (client_entity, mut message, .., mut ticks, visibility) in &mut *clients {
            if let Some(mut visibility) = visibility {
                if visibility.is_visible(entity) {
                    trace!("writing despawn for `{entity}` for client `{client_entity}`");
                    message.add_despawn(entity_range.clone());
                }
                visibility.remove_despawned(entity);
            } else {
                trace!("writing despawn for `{entity}` for client `{client_entity}`");
                message.add_despawn(entity_range.clone());
            }
            ticks.remove_entity(entity);
        }
    }

    for (client_entity, mut message, .., mut ticks, visibility) in clients {
        if let Some(mut visibility) = visibility {
            for entity in visibility.drain_lost() {
                trace!("writing visibility lost for `{entity}` for client `{client_entity}`");
                let entity_range = serialized.write_entity(entity)?;
                message.add_despawn(entity_range);
                ticks.remove_entity(entity);
            }
        }
    }

    Ok(())
}

/// Collects component removals from this tick into update messages.
fn collect_removals(
    serialized: &mut SerializedData,
    clients: &mut Query<(
        Entity,
        &mut Updates,
        &mut Mutations,
        &ConnectedClient,
        &mut ClientEntityMap,
        &mut ClientTicks,
        Option<&mut ClientVisibility>,
    )>,
    removal_buffer: &RemovalBuffer,
) -> Result<()> {
    for (&entity, remove_ids) in removal_buffer.iter() {
        let entity_range = serialized.write_entity(entity)?;
        let ids_len = remove_ids.len();
        let fn_ids = serialized.write_fn_ids(remove_ids.iter().map(|&(_, fns_id)| fns_id))?;
        for (client_entity, mut message, .., visibility) in &mut *clients {
            if visibility.is_none_or(|v| v.is_visible(entity)) {
                trace!(
                    "writing removals for `{entity}` with `{remove_ids:?}` for client `{client_entity}`"
                );
                message.add_removals(entity_range.clone(), ids_len, fn_ids.clone());
            }
        }
    }

    Ok(())
}

/// Collects component changes from this tick into update and mutate messages since the last entity tick.
fn collect_changes(
    serialized: &mut SerializedData,
    clients: &mut Query<(
        Entity,
        &mut Updates,
        &mut Mutations,
        &ConnectedClient,
        &mut ClientEntityMap,
        &mut ClientTicks,
        Option<&mut ClientVisibility>,
    )>,
    registry: &ReplicationRegistry,
    type_registry: &AppTypeRegistry,
    related_entities: &RelatedEntities,
    removal_buffer: &RemovalBuffer,
    world: &ServerWorld,
    change_tick: &SystemChangeTick,
    server_tick: RepliconTick,
) -> Result<()> {
    for (archetype, replicated_archetype) in world.iter_archetypes() {
        for entity in archetype.entities() {
            let mut entity_range = None;
            for (_, mut updates, mut mutations, .., visibility) in &mut *clients {
                let visibility = visibility
                    .map(|v| v.state(entity.id()))
                    .unwrap_or(Visibility::Visible);
                updates.start_entity_changes(visibility);
                mutations.start_entity();
            }

            // SAFETY: all replicated archetypes have marker component with table storage.
            let (_, marker_ticks) = unsafe {
                world.get_component_unchecked(
                    entity,
                    archetype.table_id(),
                    StorageType::Table,
                    world.marker_id(),
                )
            };
            // If the marker was added in this tick, the entity just started replicating.
            // It could be a newly spawned entity or an old entity with just-enabled replication,
            // so we need to include even old components that were registered for replication.
            let marker_added =
                marker_ticks.is_added(change_tick.last_run(), change_tick.this_run());

            for &(component_rule, storage) in &replicated_archetype.components {
                let (component_id, component_fns, rule_fns) = registry.get(component_rule.fns_id);

                // SAFETY: component and storage were obtained from this archetype.
                let (component, ticks) = unsafe {
                    world.get_component_unchecked(
                        entity,
                        archetype.table_id(),
                        storage,
                        component_id,
                    )
                };

                let ctx = SerializeCtx {
                    server_tick,
                    component_id,
                    type_registry,
                };
                let mut component_range = None;
                for (client_entity, mut updates, mut mutations, .., client_ticks, _) in
                    &mut *clients
                {
                    if updates.entity_visibility() == Visibility::Hidden {
                        continue;
                    }

                    if let Some(tick) = client_ticks
                        .mutation_tick(entity.id())
                        .filter(|_| !marker_added)
                        .filter(|_| updates.entity_visibility() != Visibility::Gained)
                        .filter(|_| !ticks.is_added(change_tick.last_run(), change_tick.this_run()))
                    {
                        if component_rule.send_rate != SendRate::Once
                            && ticks.is_changed(tick, change_tick.this_run())
                        {
                            if !mutations.entity_added() {
                                let graph_index = related_entities.graph_index(entity.id());
                                let entity_range = write_entity_cached(
                                    &mut entity_range,
                                    serialized,
                                    entity.id(),
                                )?;
                                mutations.add_entity(entity.id(), graph_index, entity_range);
                            }
                            let component_range = write_component_cached(
                                &mut component_range,
                                serialized,
                                rule_fns,
                                component_fns,
                                &ctx,
                                component_rule,
                                component,
                            )?;

                            trace!(
                                "writing mutation for `{}` with `{:?}` for client `{client_entity}`",
                                entity.id(),
                                component_rule.fns_id,
                            );
                            mutations.add_component(component_range);
                        }
                    } else {
                        if !updates.changed_entity_added() {
                            let entity_range =
                                write_entity_cached(&mut entity_range, serialized, entity.id())?;
                            updates.add_changed_entity(entity_range);
                        }
                        let component_range = write_component_cached(
                            &mut component_range,
                            serialized,
                            rule_fns,
                            component_fns,
                            &ctx,
                            component_rule,
                            component,
                        )?;

                        trace!(
                            "writing insertion for `{}` with `{:?}` for client `{client_entity}`",
                            entity.id(),
                            component_rule.fns_id,
                        );
                        updates.add_inserted_component(component_range);
                    }
                }
            }

            for (client_entity, mut updates, mut mutations, .., mut ticks, _) in &mut *clients {
                let visibility = updates.entity_visibility();
                if visibility == Visibility::Hidden {
                    continue;
                }

                let new_entity = marker_added || visibility == Visibility::Gained;
                if new_entity
                    || updates.changed_entity_added()
                    || removal_buffer.contains_key(&entity.id())
                {
                    // If there is any insertion, removal, or it's a new entity for a client, include all mutations
                    // into update message and bump the last acknowledged tick to keep entity updates atomic.
                    if mutations.entity_added() {
                        trace!(
                            "merging mutations for `{}` with updates for client `{client_entity}`",
                            entity.id()
                        );
                        updates.take_added_entity(&mut mutations);
                    }
                    ticks.set_mutation_tick(entity.id(), change_tick.this_run());
                }

                if new_entity && !updates.changed_entity_added() {
                    trace!(
                        "writing empty `{}` for client `{client_entity}`",
                        entity.id()
                    );

                    // Force-write new entity even if it doesn't have any components.
                    let entity_range =
                        write_entity_cached(&mut entity_range, serialized, entity.id())?;
                    updates.add_changed_entity(entity_range);
                }
            }
        }
    }

    Ok(())
}

/// Writes an entity or re-uses previously written range if exists.
fn write_entity_cached(
    entity_range: &mut Option<Range<usize>>,
    serialized: &mut SerializedData,
    entity: Entity,
) -> Result<Range<usize>> {
    if let Some(range) = entity_range.clone() {
        return Ok(range);
    }

    let range = serialized.write_entity(entity)?;
    *entity_range = Some(range.clone());

    Ok(range)
}

/// Writes a component or re-uses previously written range if exists.
fn write_component_cached(
    component_range: &mut Option<Range<usize>>,
    serialized: &mut SerializedData,
    rule_fns: &UntypedRuleFns,
    component_fns: &ComponentFns,
    ctx: &SerializeCtx,
    component_rule: ComponentRule,
    component: Ptr<'_>,
) -> Result<Range<usize>> {
    if let Some(component_range) = component_range.clone() {
        return Ok(component_range);
    }

    let range = serialized.write_component(
        rule_fns,
        component_fns,
        ctx,
        component_rule.fns_id,
        component,
    )?;
    *component_range = Some(range.clone());

    Ok(range)
}

/// Writes an entity or re-uses previously written range if exists.
fn write_tick_cached(
    tick_range: &mut Option<Range<usize>>,
    serialized: &mut SerializedData,
    tick: RepliconTick,
) -> Result<Range<usize>> {
    if let Some(range) = tick_range.clone() {
        return Ok(range);
    }

    let range = serialized.write_tick(tick)?;
    *tick_range = Some(range.clone());

    Ok(range)
}

/// Set with replication and event systems related to server.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum ServerSet {
    /// Systems that receive packets from the messaging backend.
    ///
    /// Used by the messaging backend.
    ///
    /// Runs in [`PreUpdate`].
    ReceivePackets,
    /// Systems that receive data from [`RepliconServer`].
    ///
    /// Used by `bevy_replicon`.
    ///
    /// Runs in [`PreUpdate`].
    Receive,
    /// Systems that prepare for sending data to [`RepliconServer`].
    ///
    /// Can be used by backends to add custom logic before sending data, such as transition to a stopped state.
    PrepareSend,
    /// Systems that send data to [`RepliconServer`].
    ///
    /// Used by `bevy_replicon`.
    ///
    /// Runs in [`PostUpdate`] on server tick, see [`TickPolicy`].
    Send,
    /// Systems that send packets to the messaging backend.
    ///
    /// Used by the messaging backend.
    ///
    /// Runs in [`PostUpdate`] on server tick, see [`TickPolicy`].
    SendPackets,
}

/// Controls how often [`RepliconTick`] is incremented on the server.
///
/// When [`RepliconTick`] is mutated, the server's replication
/// system will run. This means the tick policy controls how often server state is replicated.
///
/// Note that component mutations are replicated over the unreliable channel, so if a component mutate message is lost
/// then component mutations won't be resent until the server's replication system runs again.
#[derive(Debug, Copy, Clone)]
pub enum TickPolicy {
    /// The replicon tick is incremented at most max ticks per second. In practice the tick rate may be lower if the
    /// app's update cycle duration is too long.
    MaxTickRate(u16),
    /// The replicon tick is incremented every frame.
    EveryFrame,
    /// The user should manually schedule [`increment_tick`] or increment [`RepliconTick`].
    Manual,
}

/// Marker that enables replication and all events for a client.
///
/// Until authorization happened, the client and server can still exchange network events that are marked as
/// independent via [`ServerEventAppExt::make_event_independent`] or [`ServerTriggerAppExt::make_trigger_independent`].
/// **All other events will be ignored**.
///
/// See also [`ConnectedClient`] and [`RepliconSharedPlugin::auth_method`].
#[derive(Component, Default)]
#[require(ClientTicks, ClientEntityMap, Updates, Mutations)]
pub struct AuthorizedClient;

/// Controls how visibility will be managed via [`ClientVisibility`].
#[derive(Default, Debug, Clone, Copy)]
pub enum VisibilityPolicy {
    /// All entities are visible by default and visibility can't be changed.
    #[default]
    All,
    /// All entities are visible by default and should be explicitly registered to be hidden.
    Blacklist,
    /// All entities are hidden by default and should be explicitly registered to be visible.
    Whitelist,
}

/// Buffer with all despawned entities.
///
/// We treat removals of [`Replicated`] component as despawns
/// to avoid missing events in case the server's tick policy is
/// not [`TickPolicy::EveryFrame`].
#[derive(Default, Resource, Deref, DerefMut)]
struct DespawnBuffer(Vec<Entity>);
