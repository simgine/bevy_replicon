pub mod confirm_history;
#[cfg(feature = "client_diagnostics")]
pub mod diagnostics;
pub mod message;
pub mod server_mutate_ticks;

use bevy::prelude::*;
use bytes::{Buf, Bytes};
use log::{debug, error, trace};
use postcard::experimental::max_size::MaxSize;

use crate::{
    postcard_utils,
    prelude::*,
    shared::{
        backend::channels::{ClientChannel, ServerChannel},
        replication::{
            command_markers::{CommandMarkers, EntityMarkers},
            deferred_entity::{DeferredChanges, DeferredEntity},
            mutate_index::MutateIndex,
            registry::{
                ReplicationRegistry,
                ctx::{DespawnCtx, RemoveCtx, WriteCtx},
            },
            signature::SignatureMap,
            track_mutate_messages::TrackMutateMessages,
            update_message_flags::UpdateMessageFlags,
        },
        server_entity_map::{EntityEntry, ServerEntityMap},
    },
};
use confirm_history::{ConfirmHistory, EntityReplicated};
use server_mutate_ticks::{MutateTickReceived, ServerMutateTicks};

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
                (
                    ClientSystems::ResetEvents,
                    ClientSystems::Receive,
                    ClientSystems::Diagnostics,
                )
                    .chain(),
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

/// Receives and applies replication messages from the server.
///
/// Update messages are sent over the [`ServerChannel::Updates`] and are applied first to ensure valid state
/// for component mutations.
///
/// Mutate messages are sent over [`ServerChannel::Mutations`], which means they may appear
/// ahead-of or behind update messages from the same server tick. A mutation will only be applied if its
/// update tick has already appeared in an update message, otherwise it will be buffered while waiting.
/// Since component mutations can arrive in any order, they will only be applied if they correspond to a more
/// recent server tick than the last acked server tick for each entity.
///
/// Buffered mutate messages are processed last.
///
/// Acknowledgments for received mutate messages are sent back to the server.
///
/// See also [`ReplicationMessages`](crate::server::replication_messages::ReplicationMessages).
pub(super) fn receive_replication(
    world: &mut World,
    mut changes: Local<DeferredChanges>,
    mut entity_markers: Local<EntityMarkers>,
) {
    world.resource_scope(|world, mut messages: Mut<ClientMessages>| {
        world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
            world.resource_scope(|world, mut signature_map: Mut<SignatureMap>| {
                world.resource_scope(|world, mut buffered_mutations: Mut<BufferedMutations>| {
                    world.resource_scope(|world, command_markers: Mut<CommandMarkers>| {
                        world.resource_scope(|world, registry: Mut<ReplicationRegistry>| {
                            world.resource_scope(
                                |world, mut replicated: Mut<Messages<EntityReplicated>>| {
                                    let type_registry = world.resource::<AppTypeRegistry>().clone();
                                    let mut stats =
                                        world.remove_resource::<ClientReplicationStats>();
                                    let mut mutate_ticks =
                                        world.remove_resource::<ServerMutateTicks>();
                                    let mut params = ReceiveParams {
                                        changes: &mut changes,
                                        entity_markers: &mut entity_markers,
                                        entity_map: &mut entity_map,
                                        signature_map: &mut signature_map,
                                        replicated: &mut replicated,
                                        mutate_ticks: mutate_ticks.as_mut(),
                                        stats: stats.as_mut(),
                                        command_markers: &command_markers,
                                        registry: &registry,
                                        type_registry: &type_registry,
                                    };

                                    apply_replication(
                                        world,
                                        &mut params,
                                        &mut messages,
                                        &mut buffered_mutations,
                                    );

                                    if let Some(stats) = stats {
                                        world.insert_resource(stats);
                                    }
                                    if let Some(mutate_ticks) = mutate_ticks {
                                        world.insert_resource(mutate_ticks);
                                    }
                                },
                            )
                        })
                    })
                })
            })
        })
    })
}

fn reset(
    mut messages: ResMut<ClientMessages>,
    mut stats: ResMut<ClientStats>,
    mut update_tick: ResMut<ServerUpdateTick>,
    mut entity_map: ResMut<ServerEntityMap>,
    mut buffered_mutations: ResMut<BufferedMutations>,
    mutate_ticks: Option<ResMut<ServerMutateTicks>>,
    replication_stats: Option<ResMut<ClientReplicationStats>>,
) {
    messages.clear();
    *stats = Default::default();
    *update_tick = Default::default();
    entity_map.clear();
    buffered_mutations.clear();
    if let Some(mut mutate_ticks) = mutate_ticks {
        mutate_ticks.clear();
    }
    if let Some(mut replication_stats) = replication_stats {
        *replication_stats = Default::default();
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

/// Reads all received messages and applies them.
///
/// Sends acknowledgments for mutate messages back.
fn apply_replication(
    world: &mut World,
    params: &mut ReceiveParams,
    messages: &mut ClientMessages,
    buffered_mutations: &mut BufferedMutations,
) {
    for mut message in messages.receive(ServerChannel::Updates) {
        if let Err(e) = apply_update_message(world, params, &mut message) {
            error!("unable to apply update message: {e}");
        }
    }

    // Unlike update messages, we read all mutate messages first, sort them by tick
    // in descending order to ensure that the last mutation will be applied first.
    // Since mutate messages manually split by packet size, we apply all messages,
    // but skip outdated data per-entity by checking last received tick for it
    // (unless user requested history via marker).
    let update_tick = *world.resource::<ServerUpdateTick>();
    let acks_size =
        MutateIndex::POSTCARD_MAX_SIZE * messages.received_count(ServerChannel::Mutations);
    if acks_size != 0 {
        let mut acks = Vec::with_capacity(acks_size);
        for message in messages.receive(ServerChannel::Mutations) {
            if let Err(e) = buffer_mutate_message(params, buffered_mutations, message, &mut acks) {
                error!("unable to buffer mutate message: {e}");
            }
        }
        messages.send(ClientChannel::MutationAcks, acks);
    }

    apply_mutate_messages(world, params, buffered_mutations, update_tick);
}

/// Reads and applies an update message.
///
/// For details see [`replication_messages`](crate::server::replication_messages).
fn apply_update_message(
    world: &mut World,
    params: &mut ReceiveParams,
    message: &mut Bytes,
) -> Result<()> {
    if let Some(stats) = &mut params.stats {
        stats.messages += 1;
        stats.bytes += message.len();
    }

    let flags: UpdateMessageFlags = postcard_utils::from_buf(message)?;
    debug_assert!(!flags.is_empty(), "message can't be empty");

    let message_tick = postcard_utils::from_buf(message)?;
    trace!("applying update message with `{flags:?}` for {message_tick:?}");
    world.resource_mut::<ServerUpdateTick>().0 = message_tick;

    let last_flag = flags.last();
    for (_, flag) in flags.iter_names() {
        let array_kind = if flag != last_flag {
            ArrayKind::Sized
        } else {
            ArrayKind::Dynamic
        };

        match flag {
            UpdateMessageFlags::MAPPINGS => {
                let len = apply_array(array_kind, message, |message| {
                    apply_entity_mapping(world, params, message)
                })
                .map_err(|e| format!("unable to apply mappings: {e}"))?;
                if let Some(stats) = &mut params.stats {
                    stats.mappings += len;
                }
            }
            UpdateMessageFlags::DESPAWNS => {
                let len = apply_array(array_kind, message, |message| {
                    apply_despawn(world, params, message, message_tick)
                })
                .map_err(|e| format!("unable to apply despawns: {e}"))?;
                if let Some(stats) = &mut params.stats {
                    stats.despawns += len;
                }
            }
            UpdateMessageFlags::REMOVALS => {
                let len = apply_array(array_kind, message, |message| {
                    apply_removals(world, params, message, message_tick)
                })
                .map_err(|e| format!("unable to apply removals: {e}"))?;
                if let Some(stats) = &mut params.stats {
                    stats.entities_changed += len;
                }
            }
            UpdateMessageFlags::CHANGES => {
                debug_assert_eq!(array_kind, ArrayKind::Dynamic);
                let len = apply_array(array_kind, message, |message| {
                    apply_changes(world, params, message, message_tick)
                })
                .map_err(|e| format!("unable to apply changes: {e}"))?;
                if let Some(stats) = &mut params.stats {
                    stats.entities_changed += len;
                }
            }
            _ => unreachable!("iteration should yield only named flags"),
        }
    }

    Ok(())
}

/// Reads and buffers mutate message.
///
/// For details see [`replication_messages`](crate::server::replication_messages).
///
/// Returns mutate index to be used for acknowledgment.
fn buffer_mutate_message(
    params: &mut ReceiveParams,
    buffered_mutations: &mut BufferedMutations,
    mut message: Bytes,
    acks: &mut Vec<u8>,
) -> Result<()> {
    if let Some(stats) = &mut params.stats {
        stats.messages += 1;
        stats.bytes += message.len();
    }

    let update_tick = postcard_utils::from_buf(&mut message)?;
    let message_tick = postcard_utils::from_buf(&mut message)?;
    let messages_count = if params.mutate_ticks.is_some() {
        postcard_utils::from_buf(&mut message)?
    } else {
        1
    };
    let mutate_index: MutateIndex = postcard_utils::from_buf(&mut message)?;
    trace!("received mutate message for {message_tick:?}");
    buffered_mutations.insert(BufferedMutate {
        update_tick,
        message_tick,
        messages_count,
        message,
    });

    postcard_utils::to_extend_mut(&mutate_index, acks)?;

    Ok(())
}

/// Applies mutations from [`BufferedMutations`].
///
/// If the mutate message can't be applied yet (because the update message with the
/// corresponding tick hasn't arrived), it will be kept in the buffer.
fn apply_mutate_messages(
    world: &mut World,
    params: &mut ReceiveParams,
    buffered_mutations: &mut BufferedMutations,
    update_tick: ServerUpdateTick,
) {
    buffered_mutations.0.retain_mut(|mutate| {
        if mutate.update_tick > *update_tick {
            return true;
        }

        trace!("applying mutate message for {:?}", mutate.message_tick);
        let len = apply_array(ArrayKind::Dynamic, &mut mutate.message, |message| {
            apply_mutations(world, params, message, mutate.message_tick)
        });

        match len {
            Ok(len) => {
                if let Some(stats) = &mut params.stats {
                    stats.entities_changed += len;
                }
            }
            Err(e) => error!(
                "unable to apply mutate message for tick `{:?}`: {e}",
                mutate.message_tick
            ),
        }

        if let Some(mutate_ticks) = &mut params.mutate_ticks
            && mutate_ticks.confirm(mutate.message_tick, mutate.messages_count)
        {
            world.write_message(MutateTickReceived {
                tick: mutate.message_tick,
            });
        }

        false
    });
}

/// Deserializes and applies the mapping from a server entity to a client
/// entity by comparing hashes calculated from the [`Signature`] component.
fn apply_entity_mapping(
    world: &mut World,
    params: &mut ReceiveParams,
    message: &mut Bytes,
) -> Result<()> {
    let server_entity = postcard_utils::entity_from_buf(message)?;
    let hash = u64::from_le_bytes(postcard_utils::from_buf(message)?); // Hash uses fixint encoding.

    let Some(client_entity) = params.signature_map.get(hash) else {
        debug!(
            "skipping unknown hash 0x{hash:016x} for `{server_entity}` (client entity may have been despawned already)"
        );
        return Ok(());
    };

    debug!("mapping `{server_entity}` to `{client_entity}` using hash 0x{hash:016x}");
    params.entity_map.insert(server_entity, client_entity);
    world.entity_mut(client_entity).insert(Replicated);

    Ok(())
}

/// Deserializes and applies entity despawn from update message.
fn apply_despawn(
    world: &mut World,
    params: &mut ReceiveParams,
    message: &mut Bytes,
    message_tick: RepliconTick,
) -> Result<()> {
    // The entity might have already been despawned because of hierarchy or
    // with the last replication message, but the server might not yet have received confirmation
    // from the client and could include the deletion in the this message.
    let server_entity = postcard_utils::entity_from_buf(message)?;
    if let Some(client_entity) = params
        .entity_map
        .server_entry(server_entity)
        .remove()
        .and_then(|entity| world.get_entity_mut(entity).ok())
    {
        trace!("applying despawn for `{}`", client_entity.id());
        params.signature_map.remove(client_entity.id()); // // Requires manual removal since the map is removed from the world and inaccessible to triggers.
        let ctx = DespawnCtx { message_tick };
        (params.registry.despawn)(&ctx, client_entity);
    }

    Ok(())
}

/// Deserializes and applies component removals for an entity.
fn apply_removals(
    world: &mut World,
    params: &mut ReceiveParams,
    message: &mut Bytes,
    message_tick: RepliconTick,
) -> Result<()> {
    let server_entity = postcard_utils::entity_from_buf(message)?;

    let mut client_entity = match params.entity_map.server_entry(server_entity) {
        EntityEntry::Occupied(entry) => {
            DeferredEntity::new(world.get_entity_mut(entry.get())?, params.changes)
        }
        EntityEntry::Vacant(entry) => {
            // It's possible to receive a removal when an entity is spawned and has a component removed in the same tick.
            // We could serialize the size of the removals instead of the total number of removals and just advance the cursor,
            // but it's a very rare case and not worth optimizing for.
            let mut client_entity = DeferredEntity::new(world.spawn_empty(), params.changes);
            client_entity.insert(Replicated);
            entry.insert(client_entity.id());
            client_entity
        }
    };

    params
        .entity_markers
        .read(params.command_markers, &*client_entity);

    confirm_tick(&mut client_entity, params.replicated, message_tick);

    let len = apply_array(ArrayKind::Sized, message, |message| {
        let fns_id = postcard_utils::from_buf(message)?;
        let (component_id, component_fns, _) = params.registry.get(fns_id);
        let mut ctx = RemoveCtx {
            message_tick,
            component_id,
        };
        trace!(
            "applying removal for `{}` with `{fns_id:?}`",
            client_entity.id()
        );

        component_fns.remove(&mut ctx, params.entity_markers, &mut client_entity);

        Ok(())
    })?;

    if let Some(stats) = &mut params.stats {
        stats.components_changed += len;
    }

    client_entity.flush();

    Ok(())
}

/// Deserializes and applies component insertions and/or mutations for an entity.
fn apply_changes(
    world: &mut World,
    params: &mut ReceiveParams,
    message: &mut Bytes,
    message_tick: RepliconTick,
) -> Result<()> {
    let server_entity = postcard_utils::entity_from_buf(message)?;

    let world_cell = world.as_unsafe_world_cell();
    let entities = world_cell.entities();
    // SAFETY: split into `Entities` and `DeferredEntity`.
    // The latter won't apply any structural changes until `flush`, and `Entities` won't be used afterward.
    let world = unsafe { world_cell.world_mut() };

    let mut client_entity = match params.entity_map.server_entry(server_entity) {
        EntityEntry::Occupied(entry) => {
            DeferredEntity::new(world.get_entity_mut(entry.get())?, params.changes)
        }
        EntityEntry::Vacant(entry) => {
            let mut client_entity = DeferredEntity::new(world.spawn_empty(), params.changes);
            client_entity.insert(Replicated);
            entry.insert(client_entity.id());
            client_entity
        }
    };

    params
        .entity_markers
        .read(params.command_markers, &*client_entity);

    confirm_tick(&mut client_entity, params.replicated, message_tick);

    let len = apply_array(ArrayKind::Sized, message, |message| {
        let fns_id = postcard_utils::from_buf(message)?;
        let (component_id, component_fns, rule_fns) = params.registry.get(fns_id);
        let mut ctx = WriteCtx {
            entity_map: params.entity_map,
            type_registry: params.type_registry,
            component_id,
            message_tick,
            entities,
            ignore_mapping: false,
        };
        trace!(
            "applying change for `{}` with `{fns_id:?}`",
            client_entity.id(),
        );

        // SAFETY: `rule_fns` and `component_fns` were created for the same type.
        unsafe {
            component_fns.write(
                &mut ctx,
                rule_fns,
                params.entity_markers,
                &mut client_entity,
                message,
            )?;
        }

        Ok(())
    })?;

    if let Some(stats) = &mut params.stats {
        stats.components_changed += len;
    }

    client_entity.flush();

    Ok(())
}

fn apply_array(
    kind: ArrayKind,
    message: &mut Bytes,
    mut f: impl FnMut(&mut Bytes) -> Result<()>,
) -> Result<usize> {
    match kind {
        ArrayKind::Sized => {
            let len = postcard_utils::from_buf(message)?;
            for _ in 0..len {
                (f)(message)?;
            }

            Ok(len)
        }
        ArrayKind::Dynamic => {
            let mut len = 0;
            while message.has_remaining() {
                (f)(message)?;
                len += 1;
            }

            Ok(len)
        }
    }
}

/// Type of serialized array.
#[derive(PartialEq, Eq, Debug)]
enum ArrayKind {
    /// Size is serialized before the array.
    Sized,
    /// Size is unknown, means that all bytes needs to be consumed.
    Dynamic,
}

fn confirm_tick(
    entity: &mut DeferredEntity,
    replicated: &mut Messages<EntityReplicated>,
    tick: RepliconTick,
) {
    if let Some(mut history) = entity.get_mut::<ConfirmHistory>() {
        history.set_last_tick(tick);
    } else {
        entity.insert(ConfirmHistory::new(tick));
    }
    replicated.write(EntityReplicated {
        entity: entity.id(),
        tick,
    });
}

/// Deserializes and applies component mutations for an entity.
fn apply_mutations(
    world: &mut World,
    params: &mut ReceiveParams,
    message: &mut Bytes,
    message_tick: RepliconTick,
) -> Result<()> {
    let server_entity = postcard_utils::entity_from_buf(message)?;
    let data_size: usize = postcard_utils::from_buf(message)?;

    let Some(&client_entity) = params.entity_map.to_client().get(&server_entity) else {
        // Mutation could arrive after a despawn from update message.
        debug!("ignoring mutations received for unknown server's {server_entity:?}");
        message.advance(data_size);
        return Ok(());
    };

    let world_cell = world.as_unsafe_world_cell();
    let entities = world_cell.entities();
    // SAFETY: split into `Entities` and `DeferredEntity`.
    // The latter won't apply any structural changes until `flush`, and `Entities` won't be used afterward.
    let world = unsafe { world_cell.world_mut() };

    let mut client_entity =
        DeferredEntity::new(world.get_entity_mut(client_entity)?, params.changes);
    params
        .entity_markers
        .read(params.command_markers, &*client_entity);

    let Some(mut history) = client_entity.get_mut::<ConfirmHistory>() else {
        return Err(format!(
            "entity `{}` missing history component inserted on the first update message",
            client_entity.id()
        )
        .into());
    };

    let new_tick = message_tick > history.last_tick();
    if new_tick {
        history.set_last_tick(message_tick);
    } else {
        if !params.entity_markers.need_history() {
            trace!("ignoring outdated mutations for `{}`", client_entity.id());
            message.advance(data_size);
            return Ok(());
        }

        let ago = history.last_tick().get().wrapping_sub(message_tick.get());
        if ago >= u64::BITS {
            trace!(
                "discarding {ago} ticks old mutations for `{}`",
                client_entity.id()
            );
            message.advance(data_size);
            return Ok(());
        }

        history.set(ago);
    }
    params.replicated.write(EntityReplicated {
        entity: client_entity.id(),
        tick: message_tick,
    });

    let mut data = message.split_to(data_size);
    let mut components_count = 0;
    while data.has_remaining() {
        let fns_id = postcard_utils::from_buf(&mut data)?;
        let (component_id, component_fns, rule_fns) = params.registry.get(fns_id);
        let mut ctx = WriteCtx {
            entity_map: params.entity_map,
            type_registry: params.type_registry,
            component_id,
            message_tick,
            entities,
            ignore_mapping: false,
        };
        trace!(
            "applying mutation for `{}` with `{fns_id:?}`",
            client_entity.id(),
        );

        // SAFETY: `rule_fns` and `component_fns` were created for the same type.
        unsafe {
            if new_tick {
                component_fns.write(
                    &mut ctx,
                    rule_fns,
                    params.entity_markers,
                    &mut client_entity,
                    &mut data,
                )?;
            } else {
                component_fns.consume_or_write(
                    &mut ctx,
                    rule_fns,
                    params.entity_markers,
                    params.command_markers,
                    &mut client_entity,
                    &mut data,
                )?;
            }
        }

        components_count += 1;
    }

    if let Some(stats) = &mut params.stats {
        stats.components_changed += components_count;
    }

    client_entity.flush();

    Ok(())
}

/// Borrowed resources from the world and locals.
///
/// To avoid passing a lot of arguments into all receive functions.
struct ReceiveParams<'a> {
    changes: &'a mut DeferredChanges,
    entity_markers: &'a mut EntityMarkers,
    entity_map: &'a mut ServerEntityMap,
    signature_map: &'a mut SignatureMap,
    replicated: &'a mut Messages<EntityReplicated>,
    mutate_ticks: Option<&'a mut ServerMutateTicks>,
    stats: Option<&'a mut ClientReplicationStats>,
    command_markers: &'a CommandMarkers,
    registry: &'a ReplicationRegistry,
    type_registry: &'a AppTypeRegistry,
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
    /// Systems that reset queued server events.
    ///
    /// This is a separate set from [`ClientSystems::Reset`] to avoid sending events that were sent before the connection.
    ///
    /// Runs in [`OnEnter`] for [`ClientState::Connected`].
    ResetEvents,
    /// Systems that reset the client.
    ///
    /// Runs in [`OnExit`] for [`ClientState::Connected`].
    Reset,
}

/// Last received tick for update messages from the server.
///
/// In other words, the last [`RepliconTick`] with a removal, insertion, spawn or despawn.
/// This value is not updated when mutation messages are received from the server.
///
/// See also [`ServerMutateTicks`].
#[derive(Clone, Copy, Debug, Default, Deref, Resource)]
pub struct ServerUpdateTick(RepliconTick);

/// Cached buffered mutate messages, used to synchronize mutations with update messages.
#[derive(Default, Resource)]
pub(crate) struct BufferedMutations(Vec<BufferedMutate>);

impl BufferedMutations {
    fn clear(&mut self) {
        self.0.clear();
    }

    /// Inserts a new buffered message, maintaining sorting by their message tick in descending order.
    fn insert(&mut self, mutation: BufferedMutate) {
        let index = self
            .0
            .partition_point(|other_mutation| mutation.message_tick < other_mutation.message_tick);
        self.0.insert(index, mutation);
    }
}

/// Partially-deserialized mutate message that is waiting for its tick to appear in an update message.
///
/// See also [`crate::server::replication_messages`].
pub(super) struct BufferedMutate {
    /// Required tick to wait for.
    update_tick: RepliconTick,

    /// The tick this mutations corresponds to.
    message_tick: RepliconTick,

    /// Total number of mutate messages sent by the server for this tick.
    ///
    /// May not be equal to the number of received messages.
    messages_count: usize,

    /// Mutations data.
    message: Bytes,
}

/// Replication stats during message processing.
///
/// Statistic will be collected only if the resource is present.
/// The resource is not added by default.
///
/// See also [`ClientDiagnosticsPlugin`]
/// for automatic integration with Bevy diagnostics.
#[derive(Clone, Copy, Default, Resource, Debug)]
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
