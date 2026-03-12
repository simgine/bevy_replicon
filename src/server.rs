pub mod client_pools;
pub mod message;
pub mod related_entities;
pub(super) mod removal_buffer;
pub mod replicated_archetypes;
pub(super) mod replication_messages;
mod replication_query;
pub mod server_tick;
pub mod visibility;

use core::{mem, time::Duration};

use bevy::{
    ecs::{
        archetype::Archetypes,
        change_detection::{CheckChangeTicks, Tick},
        entity::{Entities, EntityHash, EntityHashMap},
        intern::Interned,
        schedule::ScheduleLabel,
        system::SystemChangeTick,
    },
    platform::collections::{HashSet, hash_map::Entry},
    prelude::*,
    time::common_conditions::on_timer,
};
use bytes::Buf;
use log::{Level, debug, log_enabled, trace, warn};

use crate::{
    postcard_utils,
    prelude::*,
    server::{
        replicated_archetypes::ReplicatedArchetypes,
        replication_messages::{
            mutations::MutationsSplit,
            serialized_data::{EntityMapping, MessageWrite, WritableComponent},
        },
        visibility::registry::FilterRegistry,
    },
    shared::{
        backend::channels::ClientChannel,
        message::server_message::message_buffer::MessageBuffer,
        replication::{
            client_ticks::{ClientTicks, EntityTicks},
            registry::{ReplicationRegistry, component_mask::ComponentMask},
            rules::ReplicationRules,
            track_mutate_messages::TrackMutateMessages,
        },
    },
};
use client_pools::ClientPools;
use related_entities::RelatedEntities;
use removal_buffer::RemovalBuffer;
use replication_messages::{
    mutations::Mutations, serialized_data::SerializedData, updates::Updates,
};
use replication_query::ReplicationQuery;
use server_tick::ServerTick;
use visibility::client_visibility::ClientVisibility;

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

fn check_mutation_ticks(check: On<CheckChangeTicks>, mut clients: Query<&mut ClientTicks>) {
    debug!(
        "checking mutation ticks for overflow for {:?}",
        check.present_tick()
    );
    for mut ticks in &mut clients {
        for entity_ticks in ticks.entities.values_mut() {
            entity_ticks.system_tick.check_tick(*check);
        }
    }
}

/// Increments current server tick which causes the server to replicate this frame.
pub fn increment_tick(mut server_tick: ResMut<ServerTick>) {
    trace!("incrementing `{:?}`", *server_tick);
    server_tick.increment();
}

fn buffer_removals(
    remove: On<Remove>,
    entities: &Entities,
    archetypes: &Archetypes,
    state: Res<State<ServerState>>,
    mut replicated_archetypes: ResMut<ReplicatedArchetypes>,
    rules: Res<ReplicationRules>,
    registry: Option<Res<ReplicationRegistry>>,
    mut removals: ResMut<RemovalBuffer>,
) {
    if *state != ServerState::Running {
        return;
    }

    let components = remove.trigger().components;
    if components.contains(&replicated_archetypes.marker_id()) {
        trace!("ignoring removals for despawned `{}`", remove.entity);
        return;
    }

    // Observers can't use run conditions. We return early on the client, but system parameters
    // are validated before the observer runs. Because of this, the registry may not be present
    // in the world during replication receive, so it needs to be optional.
    let registry = registry.expect("registry should always exist on the server");

    replicated_archetypes.update(archetypes, &rules);
    let location = entities.get_spawned(remove.entity).unwrap();
    let Some(archetype) = replicated_archetypes.get(location.archetype_id) else {
        // `Replicated` component is missing.
        trace!(
            "ignoring `{components:?}` removal for non-replicated `{}`",
            remove.entity
        );
        return;
    };

    removals.insert(remove.entity, components, archetype, &registry);
}

fn buffer_despawn(
    remove: On<Remove, Replicated>,
    mut despawn_buffer: ResMut<DespawnBuffer>,
    state: Res<State<ServerState>>,
) {
    if *state == ServerState::Running {
        trace!("buffering despawn of `{}`", remove.entity);
        despawn_buffer.push(remove.entity);
    }
}

fn cleanup_acks(
    mutations_timeout: Duration,
) -> impl FnMut(Query<&mut ClientTicks>, ResMut<ClientPools>, Res<Time<Real>>) {
    move |mut clients: Query<&mut ClientTicks>,
          mut pools: ResMut<ClientPools>,
          time: Res<Time<Real>>| {
        let min_timestamp = time.elapsed().saturating_sub(mutations_timeout);
        for mut ticks in &mut clients {
            ticks.cleanup_older_mutations(min_timestamp, |mutate_info| {
                pools.recycle_entities(mem::take(&mut mutate_info.entities));
            });
        }
    }
}

fn receive_acks(
    mut messages: ResMut<ServerMessages>,
    mut pools: ResMut<ClientPools>,
    mut clients: Query<&mut ClientTicks>,
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
                    if let Some(entities) = ticks.ack_mutate_message(client, mutate_index) {
                        pools.recycle_entities(entities);
                    }
                }
                Err(e) => {
                    debug!("unable to deserialize mutate index from client `{client}`: {e}")
                }
            }
        }
    }
}

fn prepare_messages(
    change_tick: SystemChangeTick,
    mut related_entities: ResMut<RelatedEntities>,
    mut server_change_tick: ResMut<ServerChangeTick>,
    mut pools: ResMut<ClientPools>,
    clients: Query<(&mut Updates, &mut Mutations)>,
) {
    **server_change_tick = change_tick.this_run();
    related_entities.rebuild_graphs();

    for (mut updates, mut mutations) in clients {
        updates.clear(&mut pools);
        mutations.clear(&mut pools);
        mutations.resize_related(&mut pools, related_entities.graphs_count());
    }
}

/// Collects and writes any new entity mappings that happened in this tick.
fn collect_mappings(
    despawn_buffer: Res<DespawnBuffer>,
    registry: Res<FilterRegistry>,
    mut serialized: ResMut<SerializedData>,
    entities: Query<(Entity, &Signature), With<Replicated>>,
    mut clients: Query<(
        Entity,
        &mut Updates,
        &mut ClientTicks,
        &mut ClientVisibility,
    )>,
) -> Result<()> {
    for (entity, signature) in entities {
        let hash = signature.hash();
        let mapping = EntityMapping { entity, hash };
        let mut mapping_range = None;

        if let Some(client) = signature.client() {
            let Ok((_, mut message, ticks, visibility)) = clients.get_mut(client) else {
                continue;
            };
            if should_send_mapping(entity, &despawn_buffer, &registry, &visibility, &ticks) {
                trace!(
                    "writing mapping `{entity}` to 0x{hash:016x} dedicated for client `{client}`"
                );
                let mapping_range = mapping.write_cached(&mut serialized, &mut mapping_range)?;
                message.add_mapping(mapping_range);
            }
        } else {
            for (client, mut message, ticks, visibility) in &mut clients {
                if should_send_mapping(entity, &despawn_buffer, &registry, &visibility, &ticks) {
                    trace!("writing mapping `{entity}` to 0x{hash:016x} for client `{client}`");
                    let mapping_range =
                        mapping.write_cached(&mut serialized, &mut mapping_range)?;
                    message.add_mapping(mapping_range);
                }
            }
        }
    }

    Ok(())
}

fn should_send_mapping(
    entity: Entity,
    despawn_buffer: &DespawnBuffer,
    registry: &FilterRegistry,
    visibility: &ClientVisibility,
    ticks: &ClientTicks,
) -> bool {
    // Since despawns processed later, we need to explicitly check for them here
    // because we can't distinguish between a despawn and removal of a visibility filter.
    if visibility.get(entity).is_hidden(registry) || despawn_buffer.contains(&entity) {
        return false;
    }

    // Check if the client already received the entity.
    !ticks.entities.contains_key(&entity)
}

/// Collect entity despawns from this tick into update messages.
fn collect_despawns(
    registry: Res<FilterRegistry>,
    mut serialized: ResMut<SerializedData>,
    mut pools: ResMut<ClientPools>,
    mut despawn_buffer: ResMut<DespawnBuffer>,
    mut clients: Query<(
        Entity,
        &mut Updates,
        &mut ClientTicks,
        &mut PriorityMap,
        &mut ClientVisibility,
    )>,
) -> Result<()> {
    for entity in despawn_buffer.drain(..) {
        let entity_range = entity.write(&mut serialized)?;
        for (client, mut message, mut ticks, mut priority, mut visibility) in &mut clients {
            if let Some(entity_ticks) = ticks.entities.remove(&entity) {
                // Write despawn only if the entity was previously sent because
                // spawn and despawn could happen during the same tick.
                trace!("writing despawn for `{entity}` for client `{client}`");
                message.add_despawn(entity_range.clone());
                pools.recycle_components(entity_ticks.components);
            }
            visibility.remove_despawned(entity);
            priority.remove(&entity);
        }
    }

    for (client, mut message, mut ticks, mut priority, visibility) in clients {
        for (entity, filter_mask) in visibility.iter_lost() {
            // Skip visibility changes that hide only components.
            if !filter_mask.is_hidden(&registry) {
                continue;
            }

            if let Some(entity_ticks) = ticks.entities.remove(&entity) {
                trace!("writing visibility lost for `{entity}` for client `{client}`");
                let entity_range = entity.write(&mut serialized)?;
                message.add_despawn(entity_range);
                pools.recycle_components(entity_ticks.components);
            }
            priority.remove(&entity);
        }
    }

    Ok(())
}

/// Collects component removals from this tick into update messages.
///
/// The removal buffer will be cleaned later in [`collect_changes`].
fn collect_removals(
    archetypes: &Archetypes,
    entities: &Entities,
    removal_buffer: Res<RemovalBuffer>,
    rules: Res<ReplicationRules>,
    registry: Res<ReplicationRegistry>,
    filter_registry: Res<FilterRegistry>,
    mut replicated_archetypes: ResMut<ReplicatedArchetypes>,
    mut serialized: ResMut<SerializedData>,
    mut pools: ResMut<ClientPools>,
    mut clients: Query<(
        Entity,
        &mut Updates,
        &mut ClientTicks,
        &mut ClientVisibility,
    )>,
) -> Result<()> {
    replicated_archetypes.update(archetypes, &rules);

    for (&entity, remove_ids) in removal_buffer.iter() {
        let mut entity_range = None;
        for (_, mut message, _, _) in &mut clients {
            message.start_entity_removals();
        }

        for &(component_index, fns_id) in remove_ids {
            let mut fns_id_range = None;
            for (client, mut message, mut ticks, _) in &mut clients {
                // Only send removals for components that were previously sent.
                // If the entity was despawned or lost visibility, it was removed
                // from ticks earlier during despawn collection.
                let Some(entity_ticks) = ticks.entities.get_mut(&entity) else {
                    continue;
                };
                if !entity_ticks.components.contains(component_index) {
                    continue;
                }

                trace!("writing `{fns_id:?}` removal for `{entity}` for client `{client}`");
                if !message.removals_entity_added() {
                    let entity_range = entity.write_cached(&mut serialized, &mut entity_range)?;
                    message.add_removals_entity(&mut pools, entity_range);
                }
                let fns_id_range = fns_id.write_cached(&mut serialized, &mut fns_id_range)?;
                message.add_removal(fns_id_range);
                entity_ticks.components.remove(component_index);
            }
        }
    }

    for (client, mut message, mut ticks, mut visibility) in &mut clients {
        for (entity, filter_mask) in visibility.drain_lost() {
            if filter_mask.is_hidden(&filter_registry) {
                // Was processed earlier during collecting despawns.
                continue;
            }
            let Some(entity_ticks) = ticks.entities.get_mut(&entity) else {
                // The client didn't see this entity.
                continue;
            };
            let Ok(location) = entities.get_spawned(entity) else {
                warn!(
                    "`{entity}` was despawned after despawn processing but before sending, \
                     so the despawn will be sent on the next tick; \
                     consider ordering your despawn before `{:?}`",
                    ServerSystems::Send
                );
                continue;
            };
            let archetype = replicated_archetypes
                .get(location.archetype_id)
                .unwrap_or_else(|| {
                    panic!("`{entity}` should be replicated because the client knows about it")
                });

            let mut entity_range = None;
            message.start_entity_removals();

            for components in filter_mask.hidden_components(&filter_registry) {
                for component_index in components.iter() {
                    if !entity_ticks.components.contains(component_index) {
                        // The client didn't see this component.
                        continue;
                    }

                    let &(id, _) = registry.get_by_index(component_index).unwrap_or_else(|| {
                        panic!(
                            "`{component_index:?}` should've been registered to be marked as lost"
                        )
                    });
                    let rule = archetype.find_rule(id).unwrap_or_else(|| {
                        panic!("`{id:?}` should match a rule since the client knows about it")
                    });

                    trace!(
                        "writing `{:?}` lost for `{entity}` for client `{client}`",
                        rule.fns_id
                    );
                    if !message.removals_entity_added() {
                        let entity_range =
                            entity.write_cached(&mut serialized, &mut entity_range)?;
                        message.add_removals_entity(&mut pools, entity_range);
                    }
                    let fns_id_range = rule.fns_id.write(&mut serialized)?;
                    message.add_removal(fns_id_range);
                    entity_ticks.components.remove(component_index);
                }
            }
        }
    }

    Ok(())
}

/// Collects component changes from this tick into update and mutate messages since the last entity tick.
fn collect_changes(
    archetypes: &Archetypes,
    query: ReplicationQuery,
    server_tick: Res<ServerTick>,
    change_tick: Res<ServerChangeTick>,
    registry: Res<ReplicationRegistry>,
    filter_registry: Res<FilterRegistry>,
    type_registry: Res<AppTypeRegistry>,
    related_entities: Res<RelatedEntities>,
    rules: Res<ReplicationRules>,
    mut replicated_archetypes: ResMut<ReplicatedArchetypes>,
    mut serialized: ResMut<SerializedData>,
    mut pools: ResMut<ClientPools>,
    mut removal_buffer: ResMut<RemovalBuffer>,
    mut clients: Query<(
        Entity,
        &mut Updates,
        &mut Mutations,
        &mut ClientTicks,
        &mut PriorityMap,
        &mut ClientVisibility,
    )>,
) -> Result<()> {
    replicated_archetypes.update(archetypes, &rules);

    for replicated_archetype in replicated_archetypes.iter() {
        // SAFETY: all IDs from replicated archetypes obtained from real archetypes.
        let archetype = unsafe { archetypes.get(replicated_archetype.id).unwrap_unchecked() };

        for entity in archetype.entities() {
            let mut entity_range = None;
            for (_, mut updates, mut mutations, ..) in &mut clients {
                updates.start_entity_changes();
                mutations.start_entity();
            }

            for &(rule, storage) in &replicated_archetype.components {
                let (component_index, component_id, fns) = registry.get(rule.fns_id);

                // SAFETY: component and storage were obtained from this archetype.
                let (ptr, ticks) = unsafe {
                    query.get_component_unchecked(
                        entity,
                        archetype.table_id(),
                        storage,
                        component_id,
                    )
                };

                // SAFETY: `fns` and `ptr` were created for the same component type.
                let component = unsafe {
                    WritableComponent::new(
                        fns,
                        ptr,
                        rule.fns_id,
                        component_id,
                        **server_tick,
                        &type_registry,
                    )
                };

                let mut component_range = None;
                for (client, mut updates, mut mutations, client_ticks, priority, visibility) in
                    &mut clients
                {
                    if visibility
                        .get(entity.id())
                        .is_component_hidden(&filter_registry, component_index)
                    {
                        continue;
                    }

                    if let Some(entity_ticks) = client_ticks.entities.get(&entity.id())
                        && entity_ticks.components.contains(component_index)
                    {
                        let base_priority = priority.get(&entity.id()).copied().unwrap_or(1.0);

                        let tick_diff = **server_tick - entity_ticks.server_tick;
                        if rule.mode != ReplicationMode::Once
                            && base_priority * tick_diff as f32 >= 1.0
                            && ticks.is_changed(entity_ticks.system_tick, **change_tick)
                        {
                            trace!(
                                "writing `{:?}` mutation for `{}` for client `{client}`",
                                rule.fns_id,
                                entity.id(),
                            );

                            if !mutations.entity_added() {
                                let graph_index = related_entities.graph_index(entity.id());
                                let entity_range = entity
                                    .id()
                                    .write_cached(&mut serialized, &mut entity_range)?;
                                mutations.add_entity(
                                    &mut pools,
                                    entity.id(),
                                    graph_index,
                                    entity_range,
                                );
                            }
                            let component_range =
                                component.write_cached(&mut serialized, &mut component_range)?;
                            mutations.add_component(component_range);
                        }
                    } else {
                        trace!(
                            "writing `{:?}` insertion for `{}` for client `{client}`",
                            rule.fns_id,
                            entity.id(),
                        );

                        if !updates.changed_entity_added() {
                            let entity_range = entity
                                .id()
                                .write_cached(&mut serialized, &mut entity_range)?;
                            updates.add_changed_entity(&mut pools, entity_range);
                        }
                        let component_range =
                            component.write_cached(&mut serialized, &mut component_range)?;
                        updates.add_inserted_component(component_range, component_index);
                    }
                }
            }

            for (client, mut updates, mut mutations, mut ticks, _, visibility) in &mut clients {
                if visibility.get(entity.id()).is_hidden(&filter_registry) {
                    continue;
                }

                let entity_ticks = ticks.entities.entry(entity.id());
                let new_for_client = matches!(entity_ticks, Entry::Vacant(_));
                if new_for_client
                    || updates.changed_entity_added()
                    || removal_buffer.contains_key(&entity.id())
                {
                    // If there is any insertion, removal, or it's a new entity for a client, include all mutations
                    // into update message and bump the last acknowledged tick to keep entity updates atomic.
                    if mutations.entity_added() {
                        trace!(
                            "merging mutations for `{}` with updates for client `{client}`",
                            entity.id()
                        );
                        updates.take_added_entity(&mut pools, &mut mutations);
                    }

                    update_ticks(
                        entity_ticks,
                        &mut pools,
                        **change_tick,
                        **server_tick,
                        updates.take_changed_components(),
                    );
                }

                if new_for_client && !updates.changed_entity_added() {
                    trace!("writing empty `{}` for client `{client}`", entity.id());

                    // Force-write new entity even if it doesn't have any components.
                    let entity_range = entity
                        .id()
                        .write_cached(&mut serialized, &mut entity_range)?;
                    updates.add_changed_entity(&mut pools, entity_range);
                }
            }
        }
    }

    removal_buffer.clear();

    Ok(())
}

fn update_ticks(
    entity_ticks: Entry<Entity, EntityTicks, EntityHash>,
    pools: &mut ClientPools,
    system_tick: Tick,
    server_tick: RepliconTick,
    components: ComponentMask,
) {
    match entity_ticks {
        Entry::Occupied(entry) => {
            let entity_ticks = entry.into_mut();
            entity_ticks.system_tick = system_tick;
            entity_ticks.server_tick = server_tick;
            entity_ticks.components |= &components;
            pools.recycle_components(components);
        }
        Entry::Vacant(entry) => {
            entry.insert(EntityTicks {
                server_tick,
                system_tick,
                components,
            });
        }
    }
}

/// Sends previously constructed [`Updates`] and [`Mutations`].
fn send_messages(
    mut split_buffer: Local<Vec<MutationsSplit>>,
    time: Res<Time<Real>>,
    server_tick: Res<ServerTick>,
    change_tick: Res<ServerChangeTick>,
    track_mutate_messages: Res<TrackMutateMessages>,
    mut serialized: ResMut<SerializedData>,
    mut pools: ResMut<ClientPools>,
    mut messages: ResMut<ServerMessages>,
    mut clients: Query<(
        Entity,
        &mut Updates,
        &mut Mutations,
        &ConnectedClient,
        &mut ClientTicks,
    )>,
) -> Result<()> {
    let mut server_tick_range = None;
    for (client, updates, mut mutations, connected, mut ticks) in &mut clients {
        if !updates.is_empty() {
            ticks.update_tick = **server_tick;
            let server_tick_range =
                server_tick.write_cached(&mut serialized, &mut server_tick_range)?;

            updates.send(&mut messages, client, &serialized, server_tick_range)?;
        }

        if !mutations.is_empty() || **track_mutate_messages {
            let server_tick_range =
                server_tick.write_cached(&mut serialized, &mut server_tick_range)?;

            mutations.send(
                &mut messages,
                client,
                &mut ticks,
                &mut split_buffer,
                &mut pools,
                &serialized,
                **track_mutate_messages,
                server_tick_range,
                **server_tick,
                **change_tick,
                time.elapsed(),
                connected.max_size,
            )?;
        }
    }

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

/// System tick used for change detection as the current tick.
///
/// Used to share the same tick in [`collect_changes`] and [`send_messages`].
#[derive(Resource, Deref, DerefMut, Default)]
struct ServerChangeTick(Tick);

/// Buffer with all despawned entities.
///
/// We treat removals of [`Replicated`] component as despawns
/// to avoid missing events in case the server's tick policy is
/// not [`TickPolicy::EveryFrame`].
#[derive(Resource, Deref, DerefMut, Default)]
struct DespawnBuffer(Vec<Entity>);

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
