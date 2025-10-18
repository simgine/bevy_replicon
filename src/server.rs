pub mod client_visibility;
pub mod message;
pub mod related_entities;
pub(super) mod removal_buffer;
pub(super) mod replication_messages;
pub mod server_tick;
mod server_world;

use core::{ops::Range, time::Duration};

use bevy::{
    ecs::{
        archetype::Archetypes,
        component::{CheckChangeTicks, Tick},
        entity::{Entities, EntityHashMap},
        intern::Interned,
        schedule::ScheduleLabel,
        system::SystemChangeTick,
    },
    prelude::*,
    ptr::Ptr,
    time::common_conditions::on_timer,
};
use bytes::Buf;
use log::{Level, debug, log_enabled, trace};

use crate::{
    postcard_utils,
    prelude::*,
    shared::{
        backend::channels::ClientChannel,
        message::server_message::message_buffer::MessageBuffer,
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
use related_entities::RelatedEntities;
use removal_buffer::{RemovalBuffer, RemovalReader};
use replication_messages::{
    mutations::Mutations, serialized_data::SerializedData, updates::Updates,
};
use server_tick::ServerTick;
use server_world::ServerWorld;

pub struct ServerPlugin {
    /// Schedule in which [`ServerTick`] is incremented.
    ///
    /// By default it's set to [`FixedPostUpdate`].
    ///
    /// You can also use [`Self::new`].
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
    ///         tick_schedule: PostUpdate.intern(),
    ///         ..Default::default()
    ///     }),
    /// ));
    /// ```
    pub tick_schedule: Interned<dyn ScheduleLabel>,

    /// Visibility configuration.
    pub visibility_policy: VisibilityPolicy,

    /// The time after which mutations will be considered lost if an acknowledgment is not received for them.
    ///
    /// In practice mutations will live at least `mutations_timeout`, and at most `2*mutations_timeout`.
    pub mutations_timeout: Duration,
}

impl ServerPlugin {
    /// Creates a plugin with the given [`Self::tick_schedule`].
    pub fn new(tick_schedule: impl ScheduleLabel) -> Self {
        Self {
            tick_schedule: tick_schedule.intern(),
            visibility_policy: Default::default(),
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
            .init_resource::<ServerMessages>()
            .init_resource::<ServerTick>()
            .init_resource::<EntityBuffer>()
            .init_resource::<MessageBuffer>()
            .init_resource::<RelatedEntities>()
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
            .add_observer(handle_connects)
            .add_observer(handle_disconnects)
            .add_observer(buffer_despawns)
            .add_observer(check_mutation_ticks)
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
                    buffer_removals,
                    send_replication.run_if(resource_changed::<ServerTick>),
                )
                    .chain()
                    .in_set(ServerSystems::Send)
                    .run_if(in_state(ServerState::Running)),
            );

        debug!("using tick schedule `{:?}`", self.tick_schedule);
        app.add_systems(
            self.tick_schedule,
            increment_tick
                .in_set(ServerSystems::IncrementTick)
                .run_if(in_state(ServerState::Running)),
        );

        debug!("using visibility policy `{:?}`", self.visibility_policy);
        match self.visibility_policy {
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

        if log_enabled!(Level::Debug) {
            app.add_systems(OnEnter(ServerState::Running), || debug!("running"))
                .add_systems(OnEnter(ServerState::Stopped), || debug!("stopped"));
        }
    }

    fn finish(&self, app: &mut App) {
        app.world_mut()
            .resource_scope(|world, mut messages: Mut<ServerMessages>| {
                let channels = world.resource::<RepliconChannels>();
                messages.setup_client_channels(channels.client_channels().len());
            });
    }
}

/// Increments current server tick which causes the server to replicate this frame.
fn increment_tick(mut server_tick: ResMut<ServerTick>) {
    trace!("incrementing `{:?}`", *server_tick);
    server_tick.increment();
}

fn handle_connects(add: On<Add, ConnectedClient>, mut message_buffer: ResMut<MessageBuffer>) {
    debug!("client `{}` connected", add.entity);
    message_buffer.exclude_client(add.entity);
}

fn handle_disconnects(remove: On<Remove, ConnectedClient>, mut messages: ResMut<ServerMessages>) {
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
    mut messages: ResMut<ServerMessages>,
    mut clients: Query<&mut ClientTicks>,
    mut entity_buffer: ResMut<EntityBuffer>,
) {
    for (client, mut message) in messages.receive(ClientChannel::MutationAcks) {
        while message.has_remaining() {
            match postcard_utils::from_buf(&mut message) {
                Ok(mutate_index) => {
                    let mut ticks = clients.get_mut(client).unwrap_or_else(|_| {
                        panic!(
                            "messages from client `{client}` should have been removed on disconnect"
                        )
                    });
                    if let Some(entities) =
                        ticks.ack_mutate_message(client, change_tick.this_run(), mutate_index)
                    {
                        entity_buffer.push(entities);
                    }
                }
                Err(e) => {
                    debug!("unable to deserialize mutate index from client `{client}`: {e}")
                }
            }
        }
    }
}

fn buffer_despawns(
    remove: On<Remove, Replicated>,
    mut despawn_buffer: ResMut<DespawnBuffer>,
    state: Res<State<ServerState>>,
) {
    if *state == ServerState::Running {
        trace!("buffering despawn of `{}`", remove.entity);
        despawn_buffer.push(remove.entity);
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

fn check_mutation_ticks(check: On<CheckChangeTicks>, mut clients: Query<&mut ClientTicks>) {
    debug!(
        "checking mutation ticks for overflow for {:?}",
        check.present_tick()
    );
    for mut ticks in &mut clients {
        ticks.check_mutation_ticks(*check);
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
        &mut EntityCache,
        &mut ClientTicks,
        &mut PriorityMap,
        &mut ClientVisibility,
    )>,
    entities: Query<(Entity, Ref<Signature>)>,
    mut related_entities: ResMut<RelatedEntities>,
    mut removal_buffer: ResMut<RemovalBuffer>,
    mut entity_buffer: ResMut<EntityBuffer>,
    mut despawn_buffer: ResMut<DespawnBuffer>,
    mut messages: ResMut<ServerMessages>,
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

    collect_mappings(&mut serialized, &mut clients, &entities)?;
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
        &mut messages,
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
        &mut EntityCache,
        &mut ClientTicks,
        &mut PriorityMap,
        &mut ClientVisibility,
    )>,
    messages: &mut ServerMessages,
    server_tick: RepliconTick,
    track_mutate_messages: bool,
    serialized: &mut SerializedData,
    entity_buffer: &mut EntityBuffer,
    change_tick: SystemChangeTick,
    time: &Time,
) -> Result<()> {
    let mut server_tick_range = None;
    for (client_entity, updates, mut mutations, client, .., mut ticks, _, _) in clients {
        if !updates.is_empty() {
            ticks.set_update_tick(server_tick);
            let server_tick_range =
                write_tick_cached(&mut server_tick_range, serialized, server_tick)?;

            updates.send(messages, client_entity, serialized, server_tick_range)?;
        }

        if !mutations.is_empty() || track_mutate_messages {
            let server_tick_range =
                write_tick_cached(&mut server_tick_range, serialized, server_tick)?;

            mutations.send(
                messages,
                client_entity,
                &mut ticks,
                entity_buffer,
                serialized,
                track_mutate_messages,
                server_tick_range,
                server_tick,
                change_tick.this_run(),
                time.elapsed(),
                client.max_size,
            )?;
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
        &mut EntityCache,
        &mut ClientTicks,
        &mut PriorityMap,
        &mut ClientVisibility,
    )>,
    entities: &Query<(Entity, Ref<Signature>)>,
) -> Result<()> {
    for (entity, signature) in entities {
        let hash = signature.hash();
        let mut mapping_range = None;

        if let Some(client_entity) = signature.client() {
            let Ok((_, mut message, .., ticks, _, visibility)) = clients.get_mut(client_entity)
            else {
                continue;
            };
            if should_send_mapping(entity, &signature, &visibility, &ticks) {
                trace!(
                    "writing mapping `{entity}` to 0x{hash:016x} dedicated for client `{client_entity}`"
                );
                let mapping_range =
                    write_mapping_cached(&mut mapping_range, serialized, entity, hash)?;
                message.add_mapping(mapping_range);
            }
        } else {
            for (client_entity, mut message, .., ticks, _, visibility) in &mut *clients {
                if should_send_mapping(entity, &signature, &visibility, &ticks) {
                    trace!(
                        "writing mapping `{entity}` to 0x{hash:016x} for client `{client_entity}`"
                    );
                    let mapping_range =
                        write_mapping_cached(&mut mapping_range, serialized, entity, hash)?;
                    message.add_mapping(mapping_range);
                }
            }
        }
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
        &mut EntityCache,
        &mut ClientTicks,
        &mut PriorityMap,
        &mut ClientVisibility,
    )>,
    despawn_buffer: &mut DespawnBuffer,
) -> Result<()> {
    for entity in despawn_buffer.drain(..) {
        let entity_range = serialized.write_entity(entity)?;
        for (client_entity, mut message, .., mut ticks, mut priority, mut visibility) in
            &mut *clients
        {
            if visibility.is_visible(entity) {
                trace!("writing despawn for `{entity}` for client `{client_entity}`");
                message.add_despawn(entity_range.clone());
            }
            visibility.remove_despawned(entity);
            ticks.remove_entity(entity);
            priority.remove(&entity);
        }
    }

    for (client_entity, mut message, .., mut ticks, mut priority, mut visibility) in clients {
        for entity in visibility.drain_lost() {
            trace!("writing visibility lost for `{entity}` for client `{client_entity}`");
            let entity_range = serialized.write_entity(entity)?;
            message.add_despawn(entity_range);
            ticks.remove_entity(entity);
            priority.remove(&entity);
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
        &mut EntityCache,
        &mut ClientTicks,
        &mut PriorityMap,
        &mut ClientVisibility,
    )>,
    removal_buffer: &RemovalBuffer,
) -> Result<()> {
    for (&entity, remove_ids) in removal_buffer.iter() {
        let entity_range = serialized.write_entity(entity)?;
        let ids_len = remove_ids.len();
        let fn_ids = serialized.write_fn_ids(remove_ids.iter().map(|&(_, fns_id)| fns_id))?;
        for (client_entity, mut message, .., visibility) in &mut *clients {
            if visibility.is_visible(entity) {
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
        &mut EntityCache,
        &mut ClientTicks,
        &mut PriorityMap,
        &mut ClientVisibility,
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
            for (
                _,
                mut updates,
                mut mutations,
                ..,
                mut entity_cache,
                ticks,
                priority,
                visibility,
            ) in &mut *clients
            {
                *entity_cache = EntityCache {
                    mutation_tick: ticks.mutation_tick(entity.id()),
                    visible: visibility.is_visible(entity.id()),
                    base_priority: priority.get(&entity.id()).copied().unwrap_or(1.0),
                };
                updates.start_entity_changes();
                mutations.start_entity();
            }

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
                for (client_entity, mut updates, mut mutations, .., entity_cache, _, _, _) in
                    &mut *clients
                {
                    if !entity_cache.visible {
                        continue;
                    }

                    if let Some((last_system_tick, last_server_tick)) = entity_cache.mutation_tick
                        && !ticks.is_added(change_tick.last_run(), change_tick.this_run())
                    {
                        let tick_diff = server_tick - last_server_tick;
                        if component_rule.mode != ReplicationMode::Once
                            && entity_cache.base_priority * tick_diff as f32 >= 1.0
                            && ticks.is_changed(last_system_tick, change_tick.this_run())
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

            for (client_entity, mut updates, mut mutations, .., entity_cache, mut ticks, _, _) in
                &mut *clients
            {
                if !entity_cache.visible {
                    continue;
                }

                if entity_cache.is_new_for_client()
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
                    ticks.set_mutation_tick(entity.id(), change_tick.this_run(), server_tick);
                }

                if entity_cache.is_new_for_client() && !updates.changed_entity_added() {
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

fn should_send_mapping(
    entity: Entity,
    signature: &Ref<Signature>,
    visibility: &ClientVisibility,
    ticks: &ClientTicks,
) -> bool {
    if !visibility.is_visible(entity) {
        return false;
    }

    signature.is_added() || ticks.is_new_for_client(entity)
}

/// Writes a mapping or re-uses previously written range if exists.
fn write_mapping_cached(
    mapping_range: &mut Option<Range<usize>>,
    serialized: &mut SerializedData,
    entity: Entity,
    hash: u64,
) -> Result<Range<usize>> {
    if let Some(range) = mapping_range.clone() {
        return Ok(range);
    }

    let range = serialized.write_mapping(entity, hash)?;
    *mapping_range = Some(range.clone());

    Ok(range)
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
    /// [`SyncRelatedAppExt::sync_related_entities`].
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

/// Buffer with all despawned entities.
///
/// We treat removals of [`Replicated`] component as despawns
/// to avoid missing events in case the server's tick policy is
/// not [`TickPolicy::EveryFrame`].
#[derive(Default, Resource, Deref, DerefMut)]
struct DespawnBuffer(Vec<Entity>);

/// Marker that enables replication and all events for a client.
///
/// Until authorization happened, the client and server can still exchange network events that are marked as
/// independent via [`ServerMessageAppExt::make_message_independent`] or [`ServerEventAppExt::make_event_independent`].
/// **All other events will be ignored**.
///
/// See also [`ConnectedClient`] and [`RepliconSharedPlugin::auth_method`].
#[derive(Component, Default)]
#[require(ClientTicks, PriorityMap, EntityCache, Updates, Mutations)]
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
#[derive(Component, Deref, DerefMut, Debug, Default, Clone)]
pub struct PriorityMap(EntityHashMap<f32>);

/// Cached data from [`ClientTicks`] and [`ClientVisibility`] about the entity
/// currently being processed during [`collect_changes`].
///
/// Because we iterate over clients for each component, this information is
/// cached to avoid redundant lookups.
#[derive(Component, Default, Clone, Copy)]
struct EntityCache {
    mutation_tick: Option<(Tick, RepliconTick)>,
    visible: bool,
    base_priority: f32,
}

impl EntityCache {
    /// Returns whether this entity is new for the client.
    ///
    /// For details see [`ClientTicks::is_new_for_client`].
    fn is_new_for_client(&self) -> bool {
        self.mutation_tick.is_none()
    }
}
