# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Component-based visibility API, which works similarly to layers in physics. See `AppVisibilityExt` for more details.

### Changed

- `ClientVisibility` no longer allows controlling visibility. It is now a low-level layer that holds cached visibility information. Use the new visibility API instead.
- Include `TrackAppExt::track_mutate_messages` in the replication protocol, since it affects the serialization format.
- Better memory reuse for client components.

### Fixed

- Avoid sending despawn for entities that weren't sent.

### Removed

- `VisibilityPolicy`. No longer needed because visibility is now ergonomically controlled by components.

## [0.36.1] - 2025-10-11

### Changed

- Check mutation ticks for overflow to avoid issues with long-running sessions.
- Log short type names.

## [0.36.0] - 2025-10-04

### Added

- `Debug` derive for `FromClient`.

### Fixed

- Replication of entities spawned without components before connection.

### Changed

- Rename `client_event_registry` module into `registry`.
- Replace "event" with "message" and "trigger" with "event" in types, methods and modules according to the new naming in Bevy.
- Split `RemoteMessageRegistry::*_channel` methods for messages and events into dedicated methods for each type.

### Removed

- `trigger_*_targets`. Targets in Bevy now stored inside events. When you move entities to your events, dont't forget to use `register_*_event_mapped` in order to properly map them.

## [0.35.1] - 2025-10-03

### Added

- Log state changes.

## [0.35.0] - 2025-09-26

### Added

- `PriorityMap` component for authorized clients to control how often mutations are sent.
- `Signature` component to match entities between client and server using hashes.
- `*_filtered` methods for `AppRuleExt` that allow using `With`, `Without` and `Or` filters to define replication rules.
- `compact_entity` with functions for `serde` to pack `Entity` more efficienly.
- `postcard_utils::entity_to_extend_mut` and `postcard_utils::entity_from_buf` helpers that use `compact_entity`.
- `AppRuleExt::replicate_bundle_with` to customize the priority of the bundle.
- `AppRuleExt::replicate_as*` functions to convert the component into a different struct before serialization and after deserialiation.

### Changed

- Rename `RepliconClientStatus` to `ClientState` and `RepliconServerStatus` to `ServerState`. They are now regular Bevy states. As result, we now require `StatesPlugin` to be added. It's present by default in `DefaultPlugins`, but with `MinimalPlugins` you have to add it manually.
- Replace `ServerPlugin::tick_policy` with `ServerPlugin::tick_schedule`, which specifies the schedule where the tick increments. By default, it's `FixedPostUpdate`. You can control how often it runs with `Time<Fixed>` resource. If you want to mimic `TickPolicy::EveryFrame`, set the schedule to `PostUpdate`.
- All contexts now store `AppTypeRegistry` instead of `TypeRegistry`. To get `TypeRegistry`, call `AppTypeRegistry::read`.
- All events now use `ClientId` wrapper instead of `Entity`.
- `AppTypeRegistry` now available on replication for observers.
- `postcard_utils` now a top-level module. Instead of `bevy_replicon::shared::postcard_utils`, it's now just `bevy_replicon::postcard_utils`.
- Rename `FromClient::client_entity` into `FromClient::client_id`.
- Rename `DisconnectRequest::client_entity` into `DisconnectRequest::client`.
- Rename `SendRate` into `ReplicationMode`. The `EveryTick` variant was also renated into `OnChange`.
- Rename `ClientSet` into `ClientSystems`.
- Rename `ServerSet` into `ServerSystems`.
- Rename `ComponentRule::send_rate` into `ComponentRule::mode`.
- Rename `replicon_channels` module into `channels`.
- Rename `replication_rules` module into `rules`.
- Rename `replication_registry` module into `registry`.
- Rename `IntoComponentRule::register_component` to `IntoComponentRule::into_rule` and move it to the `shared::replication::rules::component` module.
- Rename `NetworkStats` to `ClientStats` and move it from `shared::backend::connected_client` to `shared::backend`. It's also now a separate resource from `RepliconClient`.
- Rename `RepliconClient` into `ClientMessages`. It holds only messages now.
- Rename `RepliconServer` into `ServerMessages`. It holds only messages now.
- Replace `IntoReplicationRule` with `IntoComponentRules`, which returns only `Vec<ComponentRule>`.
- Replace `ReplicationBundle` with `BundleRules`, which returns only `Vec<ComponentRule>` and uses an associated constant to customize the priority.
- Hide `ServerEntityMap` mutation methods from public API.
- Hide `BufferedMutations` from public API.
- Hide `server::increment_tick` from public API.
- `ExampleClient::new` now accepts `impl Into<SocketAddr>`. This makes it easier for backend developers to port examples since they usually specify IP address.
- Switch from the FNV hasher to [xxh3](https://docs.rs/xxhash-rust). It's faster and produces high-quality hashes. The hasher is now also wrapped in [deterministic-hash](https://docs.rs/deterministic-hash), which ensures consistent endianness and handles `usize` portably.

## Removed

- `ClientEntityMap`. Use the newly added `Signature` component instead.
- `VisibilityPolicy::All`. Use `VisibilityPolicy::Blacklist` instead, which is the default now. There are not performance difference when the list is empty.
- `entity_serde::serialize_entity` and `entity_serde::deserialize_entity`. Use `postcard_utils::entity_to_extend_mut` and `postcard_utils::entity_from_buf` respectively; just swap the argument order.
- `SERVER`. Use `ClientId::Server` instead.
- `SendRate::Periodic`. Use `PriorityMap` instead.
- `TickPolicy`. Set the schedule directly to `ServerPlugin::tick_schedule`.
- All provided run conditions. Just use `in_state` or `OnEnter`/`OnExit` with `ServerState` and `ClientState` instead. `server_or_singleplayer` is just `in_state(ClientState::Disconnected)`.

## [0.34.4] - 2025-07-29

### Changed

- Make `DeferredEntity::new`, `DeferredEntity::flush` and `DeferredChanges` public.

## [0.34.3] - 2025-07-01

### Fixed

- Make `ProtocolHash` deterministic across platforms by using `fnv` instead of `foldhash`.
- Reset `ServerMutateTicks` on disconnect.
- Avoid calling `drop` on uninitialized memory for components with `Box` or `Arc` during buffered insertion.

## [0.34.1] - 2025-06-21

### Changed

- Log replication errors instead of panicking. We use panics only for things that should never happen, but users could sometimes trigger them by messing with entities, so we now log these errors to simplify debugging in those cases.
- Spawn all allocated entities after processing each replicated entity.

## [0.34.0] - 2025-06-15

### Added

- Authorization system. By default we just verify compatibility between client and server, but it's customizable via `RepliconSharedPlugin::auth_method`.
- Configurable `SendRate` for deterministic replication. Use `SendRate::Once` to send only the initial value, or `SendRate::Periodic` to only sync the state periodically.
- `AppRuleExt::replicate_with_priority` to configure replication rule priority.
- `DisconnectRequest` event to queue a disconnection for a specific client on the server.
- `ServerTriggerAppExt::make_trigger_independent`.

### Changed

- `AppRuleExt::replicate_with` now accepts `IntoReplicationRule` trait that allows to define rules with multiple components.
- Rename `GroupReplication` into `BundleReplication`.
- Rename `ReplicatedClient` into `AuthorizedClient`.
- Rename `AppRuleExt::replicate_group` into `AppRuleExt::replicate_bundle`.
- Rename `replication_registry::despawn_recursive` into `replication_registry::despawn`.
- Rename `shared::event::trigger` module into `shared::event::remote_targets`.
- Rename `ServerEventAppExt::make_independent` into `ServerEventAppExt::make_event_independent`. It never worked for triggers.
- `ReplicationRule` now stores `Vec<ComponentRule>` instead of `Vec<(ComponentId, FnsId)>`
- `RuleFns` now available from prelude.
- Initialize channels in `App::finish` instead of `Startup`. It's called automatically on `App::run`, but in tests you need to call `App::finish` manually.
- Rules created with the same priority now evaluated in their creation order.
- Component removals and insertions for an entity are now buffered and applied as bundles to avoid triggering observers without all components being inserted or removed. This also significantly improves performance by avoiding extra archetype moves and lookups.
- The `Replicated` component is no longer automatically inserted into non-replicated entities spawned from replicated components.
- Replace `ServerEntityMap::get_by_*` and `ServerEntityMap::remove_by_*` with an entry-based API. Use `ServerEntityMap::server_entry` or `ServerEntityMap::client_entry` instead.
- Split `ReplicationChannel` into `ServerChannel` and `ClientChannel` for clarity.
- Don't register an additional unreliable client channel for replication. While the server requires two channels, the client only needs one.
- Print error instead of panic on mapping overwrite in `ServerEntityMap`.
- Making `trace` logging level less verbose and more informative.

### Removed

- `WriteCtx::commands`. You can now insert and remove components directly through `DeferredEntity`.
- `RepliconClient::receive` and `RepliconServer::receive` are now private since they should only be called internally by Replicon.
- `ServerPlugin::replicate_after_connect`. Use `RepliconSharedPlugin::auth_method` with `AuthMethod::Custom` instead.
- Deprecated methods.

## [0.33.0] - 2025-04-27

### Changed

- Don't unwrap systems.

### Added

- Support for `no_std`.
- Relationships networking. Use `SyncRelatedAppExt::sync_related_entities<C>` to ensure that entities related by `C` are replicated in sync.
- Seamless support for immutable components. For these components, replication is always applied via insertion.
- `server_just_started` run condition.

### Changed

- Update to Bevy 0.16.
- All serde methods now use `bevy::ecs::error::Result` instead of `postcard::Result` for more informative errors.
- `AppRuleExt::replicate_mapped`, `RuleFns::default_mapped` and `default_deserialize_mapped` now deprecated. Entities inside components now mapped automatically, use methods without `_mapped` prefixes.
- Use an observer instead of a system to track despawns.

### Removed

- `parent_sync` module and corresponding feature. Just replicate `ChildOf` directly.

## [0.32.2] - 2025-04-16

### Changed

- Preserve removals before inserts

## [0.32.1] - 2025-04-05

### Changed

- Publicize `ClientTicks` component and its `update_tick` method.

## [0.32.0] - 2025-03-24

### Added

- `RemoteEventRegistry` to get channels for remote triggers and events.
- `ConditionerConfig` for `bevy_replicon_example_backend` to simulate various network conditions.

### Changed

- Rename `ChannelKind` into just `Channel`.
- Rename `channels` module into `replicon_channels`.
- Rename `core` module into `shared` and `RepliconCorePlugin` into `RepliconSharedPlugin`. To avoid ambiguity with Rust's `core`, which will be used for `no_std` support in the next release.
- Move `replicon_server`, `replicon_client`, `connected_client` and `replicon_channels` under `backend` module to group all backend-related API.
- All methods with `Into<Channel>` now just accept `Channel`.
- Use `usize` for channel ID. Backends now decide how many channels user can create.
- Don't insert `ClientVisibility` at all if `ServerPlugin::visibility_policy` is set to `VisibilityPolicy::All`. Previously all calls were just no-op.

### Removed

- `RepliconChannel` and all methods from `RepliconChannels`, except channel getters. Now all channel configuration needs to be done on the backend side.

## [0.31.1] - 2025-03-15

### Changed

- Rename `ClientId` into `NetworkId` and derive serde traits.
- Move `ConnectedClient::id` into a separate optional component for backends that doesn't provide persistent identifiers.

### Fixed

- Bump the `bevy` dependency to 0.15.3 since we use some fields that were made public in a patch release.

### Removed

- `ConnectedClient::new`, you can now construct the struct directly.

## [0.31.0] - 2025-03-13

### Added

- Derive `Debug` for `FnsId`.
- Derive `Deref` and `DerefMut` to underlying event in `ToClients` and `FromClient`.
- Derive `PartialEq` for `RepliconClientStatus`.
- `SerializeCtx::type_registry` and `WriteCtx::type_registry` to replicate components with reflection.

### Changed

- Connected clients are now represented as entities with `ConnectedClient` components. Backends are responsible for spawning and despawning entities with this component. `ClientId` is accessible from `ConnectedClient::id` in case you need to identify which client belongs to which connection.
- Statistics for connected clients now accessible via `ClientStats` component.
- Replicated entities now represented by connected clients with `ReplicatedClient` component.
- To access visibility, use `ClientVisibility` component on replicated entities.
- `ServerEntityMap` resource now a component on replicated entities. It now accepts entity to entity mappings directly instead of `ClientId` to `ClientMapping`.
- Replace statistic methods on `RepliconClient` with `RepliconClient::stats` method that returns `ClientStats` struct.
- Move `VisibilityPolicy` to `server` module.
- Move `ClientId` to `connected_client` module and remove from `prelude`.
- Use `TestClientEntity` instead of `ClientId` resource on clients in `ServerTestAppExt` to identify client entity.
- Rename `FromClient::client_id` into `FromClient::client_entity`.
- Rename `registry` in all event contexts into `type_registry`.
- Replace `bincode` with `postcard`. It has more suitable variable integer encoding and potentially unlocks `no_std` support. If you use custom ser/de functions, replace `DefaultOptions::new().serialize_into(message, event)` with `postcard_utils::to_extend_mut(event, message)` and `DefaultOptions::new().deserialize_from(cursor)` with `postcard_utils::from_buf(message)`.
- All serde methods now use `postcard::Result` instead of `bincode::Result`.
- All deserialization methods now accept `Bytes` instead of `std::io::Cursor` because deserialization from `std::io::Read` requires a temporary buffer. `Bytes` already provide cursor-like functionality. The crate now re-exported under `bevy_replicon::bytes`.
- Use varint for `RepliconTick` because `postcard` provides more efficient encoding for it.
- Improve panic message for non-registered functions.
- Allow update messages with mappings-only to map non-replicated entities.
- Log bytes count on receive.

### Fixed

- Local re-trigger for listen server mode.

### Removed

- `ClientId` from `prelude`. Most operations now done using `Entity` as identifier. But it could be useful
- `StartReplication` trigger. Just insert `ReplicatedClient` to enable replication.
- `ConnectedClients` and `ReplicatedClients` resources. Use components on connected clients instead.
- `ClientConnected` and `ClientDisconnected` triggers. Just observe for `Trigger<OnAdd, ConnectedClient>` or `Trigger<OnRemove, ConnectedClient>`. To get disconnect reason, obtain it from the ued backend.
- `ServerSet::TriggerConnectionEvents` variant. We no longer use events for connections.

## [0.30.1] - 2025-02-07

### Fixed

- Update `ReplicatedClients` immediately to let users set visibility on `ClientConnected` trigger.
- Send and receive on packet split in the example backend.

## [0.30.0] - 2025-02-04

### Added

- Export `core::entity_serde` with custom serde functions for entity.

### Changed

- `StartReplication` is now a trigger-event.
- `ServerEvent` now split into `ClientConnected` and `ClientDisconnected` that are trigger-events.
- `reason` field in `ClientDisconnected` now stores `DisconnectReason` enum.
- Event serialization functions now accept `&mut Vec<u8>` instead of `&mut Cursor<Vec<u8>>`.
- `RepliconChannels::create_server_channel` and `RepliconChannels::create_client_channel` now accept `impl Into<RepliconChannel>` instead of just `RepliconChannel`.
- Event reading and writing systems are no longer exclusive, thanks to system builders! However, to achieve this, we now set them up in `Plugin::finish`. So if you have tests for events, don't forget to call `App::finish` to configure the plugins properly.
- Use `debug!` instead of `trace!` for events. They are not very verbose.
- Rename `ServerSet::SendEvents` into `ServerSet::TriggerConnectionEvents`.
- Rename `core::event_registry` into `core::event`.
- Rename `ClientEventsPlugin` into `ClientEventPlugin` (singular).
- Rename `ServerEventsPlugin` into `ServerEventPlugin` (singular).
- Rename `client::events` into `client::event` (singular).
- Rename `server::events` into `server::event` (singular).

### Fixed

`ParentSync` now correctly syncs the hierarchy if spawned before `ClientSet::SyncHierarchy`.

## [0.29.2] - 2025-01-06

### Fixed

- Use `FromReflect` when replicating components into dynamic scenes.

## [0.29.1] - 2024-12-16

### Fixed

- Report bevy diagnostics correctly as a delta since last measurement collection.

## [0.29.0] - 2024-12-02

### Added

- RTT, bytes per second and packet loss information for `RepliconClient` and `ConnectedClients`.
- `ClientSet::Diagnostics` for systems that collect client diagnostics.

### Fixed

- Sending removals and despawns for hidden entities.

### Changed

- Update to Bevy 0.15.
- Make `core::replication::replication_rules::ReplicationRules` public.
- Various optimizations for replication messages to use fewer bytes.
- Accept `Vec<u8>` instead of `Cursor<Vec<u8>>` for serialization.
- `ConnectedClients` now store `ConnectedClient` instead of `ClientId` with more information about the client.
- All `TestFnsEntityExt` now accept `FnsId`.
- Move replication-related modules from `core` module under `core::replication`.
- Move `Replicated` to the `replication` module.
- Split the `ctx` module and move event-related contexts under `core::events_registry::ctx` and replication-related contexts under `core::replication_registry::ctx`.
- Separate paths from `diagnostics` module by `/` and their parent path now `client/replication` instead of `replication/client`.
- Provide replication statistics by sum instead of per second and use `usize` for it.
- Use fixed integer encoding for ticks for server events.
- Rename `ServerPlugin::change_timeout` into `ServerPlugin::mutations_timeout`.
- Rename `ServerInitTick` into `ServerUpdateTick`.
- Rename `ReplicatedClient::init_tick` into `ReplicatedClient::change_tick`.
- Rename `ReplicatedClient::get_change_tick` into `ReplicatedClient::mutation_tick`.
- Rename `ReplicationChannel::Init` into `ReplicationChannel::Updates`.
- Rename `ReplicationChannel::Update` into `ReplicationChannel::Mutations`.
- Rename `ClientStats` into `ClientReplicationStats`.
- Rename `ClientDiagnosticsPlugin::MESSAGES` into `ClientDiagnosticsPlugin::REPLICATION_MESSAGES`.
- Rename `ClientDiagnosticsPlugin::BYTES` into `ClientDiagnosticsPlugin::REPLICATION_BYTES`.
- Rename `ClientDiagnosticsPlugin::ENTITY_CHANGES` into `ClientDiagnosticsPlugin::ENTITIES_CHANGED`.
- Rename `ClientDiagnosticsPlugin::COMPONENT_CHANGES` into `ClientDiagnosticsPlugin::COMPONENTS_CHANGED`.

### Removed

- `FnsInfo`, use `(ComponentId, FnsId)` instead.
- Deprecated functions and structs from previous releases.

## [0.28.4] - 2024-10-15

### Added

- `ComponentId` to all component-related contexts.

### Changed

- All custom functions now accept the newly added `DeferredEntity` instead of `EntityMut`. This new entity type provides read-only access to the world.

### Fixed

- Synchronize server events with init messages properly when `ServerTick` is not updated every app tick.

## [0.28.3] - 2024-09-13

### Changed

- Ignore replicated components that don't have type registration or missing `#[reflect(Component)]` in `scene::replicate_into` instead of panicking.
- Rename `has_authority` condition into `server_or_singleplayer`. Old name still works, but marked as deprecated.

## [0.28.2] - 2024-09-09

### Changed

- Make `ReplicatedClients::new` public.

## [0.28.1] - 2024-09-04

### Fixed

- Client event buffering.

## [0.28.0] - 2024-09-03

### Added

- `ServerEventAppExt::make_independent` to let events be triggered without waiting for replication on the same tick.
- `ConnectedClients` (the same name as the old resource that was renamed into `ReplicatedClients`) with client IDs for all connected clients (but may not be replicated yet).
- `ServerPlugin::replicate_after_connect` to enable replication right after connection (enabled by default, same as old behavior).

### Changed

- Rename `connected_clients` into `replicated_clients`.
- Rename `ConnectedClients` to `ReplicatedClients`.
- Rename `ConnectedClient` to `ReplicatedClient`.

### Fixed

- Emit an error instead of panic on events deserialization on client.
- Buffering for events that have mapped entities.

## [0.27.0] - 2024-07-04

### Changed

- Update to Bevy `0.14.0`.
- Move `bevy_replicon_renet` to a [dedicated repository](https://github.com/simgine/bevy_replicon_renet).
- `ServerEventsPlugin` and `ClientEventsPlugin` can be disabled on client-only and server-only apps respectively.
- Put `ClientDiagnosticsPlugin` under `client_diagnostics` feature (disabled by default) and make it part of the `RepliconPlugins` group.
- Put `scene` module under `scene` feature (enabled by default).
- Put `parent_sync` module under `parent_sync` feature (enabled by default).
- Put `client` module under `client` feature (enabled by default).
- Put `server` module under `server` feature (enabled by default).
- `TestFnsEntityExt::serialize` now accepts `RepliconTick` for server tick instead of using `ServerTick` resource internally.
- Move `replicon_client`, `server_entity_map`, `replicon_server`, `connected_clients` under `core` module. These modules are needed for both client and server.
- Move `VisibilityPolicy` to `connected_clients` module.
- Move `server::events::event_data` module to `core::event_registry::server_event`.
- Move `client::events::event_data` module to `core::event_registry::client_event`.
- Move `ClientEventAppExt`, `client::events::SerializeFn`, `client::events::DeserializeFn`, `default_serialize`, `default_serialize_mapped`, `default_deserialize` and `FromClient` to `core::event_registry::client_event`.
- Move `ServerEventAppExt`, `server::events::SerializeFn`, `server::events::DeserializeFn`, `default_serialize`, `default_serialize_mapped`, `default_deserialize`, `ToClients` and `SendMode` to `core::event_registry::server_event`.
- Speedup removals caching.

### Fixed

- Do not divide values per seconds by the number of messages for `ClientDiagnosticsPlugin`.

## [0.26.3] - 2024-06-09

### Added

- Logging for sending and receiving messages.

### Changed

- Do not send empty ack messages from client.

## [0.26.2] - 2024-06-05

### Added

- `Debug`/`Clone` derives to `ServerEvent`.
- `Debug`/`Copy`/`Clone` derives to `TickPolicy`.
- `ClientSet::SyncHierarchy` for `ParentSync` updates.

## [0.26.1] - 2024-05-27

### Fixed

- Regression in server events reset logic.

## [0.26.0] - 2024-05-26

### Added

- `ClientEventsPlugin` and `ServerEventsPlugin` that are required for events (available from the `RepliconPlugins` group). Can be disabled if you don't use them.

### Changed

- Custom events are now registered with serialization and deserialization functions instead of systems. This makes the API more convenient since the purpose of custom systems was to customize serialization.
- All events are processed in one system instead of a separate system for each event. Bevy does a [similar optimization](https://github.com/bevyengine/bevy/pull/12936) for event updates. It won't be that noticeable since users register much fewer replicon events.
- Rename `ConnectedClient::change_tick` into `ConnectedClient::init_tick`.
- Rename `ConnectedClient::get_change_limit` into `ConnectedClient::get_change_tick`.
- Rename `Confirmed` into `ConfirmHistory`.
- Rename `replicon_channels` module into `channels`.
- Rename `replication_fns` and `ReplicationFns` into `replication_registry` and `ReplicationRegistry`.
- Rename "packets" into "messages" in client diagnostics.

### Fixed

- `bevy_replicon_renet` now properly sets `RepliconClientStatus::Connecting` when `RenetClient` is connecting.

## [0.25.3] - 2024-05-24

### Fixed

- Fix replication with a removal at the same tick.

## [0.25.2] - 2024-05-18

### Fixed

- Fix replicating previously spawned entities to a newly connected client with visibility policy different from `VisibilityPolicy::All`.

## [0.25.1] - 2024-05-16

### Fixed

- Fix possible overflow in `Confirmed::contains_any`.

## [0.25.0] - 2024-05-11

### Added

- `AppMarkerExt` to customize how components will be written based on markers present without overriding deserialization functions. Now third-party prediction crates could be integrated much easier.
- `AppRuleExt::replicate_group` and `GroupRegistration` trait to register and customize component groups. A group will be replicated only if all its components present on an entity.
- `ServerSet::StoreHierarchy` for systems that store hierarchy changes in `ParentSync`.
- More tracing.
- `Debug` impl for `RepliconClientStatus`.

### Changed

- `SerializeFn` now accepts regular `C` instead of `Ptr`.
- `DeserializeFn` now does only deserialization and returns `C`. Use the newly added marker-based API if you want to customize how component will be written. See docs for `AppMarkerExt` for details.
- Rename `AppReplicationExt` into `AppRuleExt`.
- `AppRuleExt::replicate_with` now accepts `RuleFns` struct with functions. You no longer can customize removals this way, use the mentioned marker-based API instead.
- Writing to entities on client now done via `EntityMut` and `Commands` instead of `EntityWorldMut`. It was needed to support the mentioned in-place deserialization and will possibly allow batching insertions in the future (for details see https://github.com/bevyengine/bevy/issues/10154).
- Return iterator from `RepliconClient::receive` instead of popping the last message. If you used `while` loop with it before, replace it with `for`.
- Use new `ServerInitTick` resource on client instead of `RepliconTick`. If you used `ServerEventAppExt::add_server_event_with`, use `ServerInitTick` instead of `RepliconTick` in your receive function.
- Use new `ServerTick` resource on server instead of `RepliconTick`.
- Replace `ServerEntityTicks` with `Confirmed` component. The component now also stores whether the last 64 ticks were received.
- Now serialization/deserialization, removal, despawn and writing functions accept context to access additional information.
- Move `replicon_tick` module under `server` module since now it's used only on server.
- Move `Replication` to `core` module.
- Move all functions-related logic from `ReplicationRules` into a new `ReplicationFns` and hide `ReplicationRules` from public API.
- Move `despawn_recursive` into `replication_fns` module.
- Rename `serialize_component` into `default_serialize` and move into `rule_fns` module.
- Rename `deserialize_component` into `default_deserialize` and move into `rule_fns` module.
- Rename `deserialize_mapped_component` into `default_deserialize_mapped` and move into `rule_fns` module.
- Rename `remove_component` into `default_remove` and move into `command_fns` module.

### Removed

- `dont_replicate` module. Use the newly added `AppRuleExt::replicate_group` or newtypes.

### Fixed

- Reversed order of the received messages from `RepliconClient`.

## [0.24.1] - 2024-03-07

### Fixed

- Fix compilation issue when multiple `PartialEq` impls are present for `usize`.

## [0.24.0] - 2024-03-06

### Added

- Provide `ServerTestAppExt` extension trait for exchanging messaging between apps in tests.

### Changed

- Rename `Replication` into `Replicated`. Old name is still available via deprecated alias.
- Abstract out all I/O and `renet` dependency. We will continue to provide first-party integration with renet via `bevy_replion_renet`. But users can write their integration with other messaging libraries. So now users need to additionally add messaging-related plugin. In case of `bevy_replion_renet` it's `RepliconRenetPlugins`.
- Replace usage of `RenetServer` and `RenetClient` with our `RepliconServer` and `RepliconClient` respectively. Use types from `renet` (or other library) only when you need to connect / disconnect or write some library-specific logic. In other cases prefer using the newly provided types to make your code messaging-library independent. Unlike the old types from renet, these resources are always present in the world. So instead of using `resource_(exists/added/removed)` for network-related conditions, use special conditions provided in `common_conditions` module.
- Move `has_authority` to `common_conditions` module.
- Replace conditions from `renet` with ours, see `common_conditions` module. Available in `prelude`.
- Replace usage of `ClientId` from `renet` with our own with the same name. In user code only the one from `bevy_replicon` should be used.
- Replace `SERVER_ID` constant with `ClientId::SERVER`.
- Replace use of `RenetConnectionStatus` with our `RepliconClientStatus`.
- Replace `ServerEvent` from `renet` with our own with the same name. In user code only the one from `bevy_replicon` should be used.
- Rename `replicon_core` module into `core`.
- Rename `EventType` into `ChannelKind` and move into `core` module.
- Replace usage of renet's `SendType` with our `RepliconChannel`.
- Rename `NetworkChannels` into `RepliconChannels` and move into `replicon_channels` module.
- Rename `ReplicationChannel::Reliable` and `ReplicationChannel::Unreliable` into `ReplicationChannel::Init` and `ReplicationChannel::Update` respectively.
- Channel creation methods in `RepliconChannels` now accept `RepliconChannel` with full channel configuration.
- Make `default_max_bytes` field in `RepliconChannels` public.
- Move `RepliconChannels::get_server_configs` and `RepliconChannels::get_client_configs` to create channels configs for `renet` into `RenetChannelsExt` extension trait provided by `bevy_replion_renet`. Make sure to import it to use these methods.
- Move `ClientEntityMap` and `ClientMapping` to `client_entity_map` submodule.
- Rename `ReplicationPlugins` into `RepliconPlugins`.
- Rename `ClientCache` into `ConnectedClients`.
- Rename `ClientState` into `ConnectedClient`.
- Replace `RepliconChannels::set_client_max_bytes` and `RepliconChannels::set_server_max_bytes` with `RepliconChannels::server_channel_mut` and `RepliconChannels::client_channel_mut` respectively with more rich configuration.
- Move `ClientEventChannel` to `client_event` module.
- Move `ServerEventChannel` to `server_event` module.
- `ClientMapper`, `ServerEntityMap`, `BufferedUpdates`, `ReplicationRules`, `ReplicationChannel`, `ClientEventChannel`, `ServerEventChannel`, `ServerEventQueue` and `EventMapper` are no longer in `prelude` module. Import them directly.

## [0.23.0] - 2024-02-22

### Changed

- Common conditions now follow the new pattern without returning a closure.

### Removed

- `Mapper` and `MapNetworkEntities` in favor of `EntityMapper` and `MapEntities` introduced in Bevy 0.13.0

### Fixed

- Make `scene::replicate_into` update previously added entities.

## [0.22.0] - 2024-02-17

### Changed

- Change `ClientEventAppExt::add_mapped_client_event` to clone the events instead of draining them. This means that mapped client events must now implement `Clone`

### Fixed

- Misuse of `Vec::reserve` that would cause excess allocations.

## [0.21.2] - 2024-01-27

### Changes

- Increase publicity of `ClientState` API.

## [0.21.1] - 2024-01-22

### Added

- `ClientCache::visibility_policy` returns the configured policy.

## [0.21.0] - 2024-01-22

### Added

- Control over client visibility of entities.

### Changed

- Rename `ClientsInfo` into `ClientCache`.
- Rename `ClientInfo` into `ClientState`.
- Allow calling `dont_replicate::<T>` with the insertion of `T` instead of after `Replication` insertion.
- `dont_replicate::<T>` can panic only in `debug_assertions` is enabled.

## [0.20.0] - 2024-01-13

### Changed

- API for custom server messages now uses `server_event::serialize_with` and `server_event::deserialize_with`. For more details see the example in the docs.
- Speedup serialization for multiple clients by reusing already serialized components and entities.
- Hide extra functionality from `ServerEventQueue`.
- Move server event reset system to new set `ClientSet::ResetEvents` in `PreUpdate`.
- Make `NetworkChannels` channel-creation methods public (`create_client_channel` and `create_server_channel`).
- Implement `Eq` and `PartialEq` on `EventType`.

### Removed

- `LastChangeTick` resource, `ClientsInfo` should be used instead.

### Fixed

- Don't panic when handling client acks if the ack references a despawned entity.

## [0.19.0] - 2024-01-07

### Added

- `renet_serde` feature which reexports `serde` feature from `bevy_renet`.
- `ClientSet::Reset` which can be disabled by external users.
- `ServerEntityMap::remove_by_client` for manual client cleanup.
- `BufferedUpdates`, `ServerEntityTicks` to public API.

### Changed

- Move the client reset system to `PreUpdate` to let clients react more promptly to resets.
- Replace `Ignored<T>` with `CommandDontReplicateExt::dont_replicate`.
- `Replication` entities with no replicated components will now be spawned on the client anyway.

## [0.18.2] - 2023-12-27

### Fixed

- Fix missing removals and despawns caused by events cleanup.

## [0.18.1] - 2023-12-21

### Changed

- Cache replicated archetypes for faster iteration.

### Fixed

- Fix crash caused by registering the same type for client and server events.
- Fix replication for entities when `Replication` component is added after spawn.

## [0.18.0] - 2023-12-19

### Changed

- Send all component mappings, inserts, removals and despawns over reliable channel in form of deltas and component updates over unreliable channel packed by packet size. This significantly reduces the possibility of packet loss.
- Replace `REPLICATION_CHANNEL_ID` with `ReplicationChannel` enum. The previous constant corresponded to the unreliable channel.
- Server events use tick with the last change instead of waiting for replication message without changes.
- Include despawns before removals to optimize space for case where despawns are presents and removals aren't.
- `TickPolicy::EveryFrame` and `TickPolicy::MaxTickRate` now increment tick only if `RenetServer` exists.
- `ServerSet::Send` now always runs. Replication sending system still runs on `RepliconTick` change.
- `ClientMapping` no longer contains `tick` field.
- Use `EntityHashMap` instead of `HashMap` with entities as keys.
- Use `Cursor<&[u8]>` instead of `Cursor<Bytes>`.
- Replace `LastRepliconTick` with `RepliconTick` on client.
- Move `ClientMapper` and `ServerEntityMap` to `client_mapper` submodule.
- Rename `replicate_into_scene` into `replicate_into` and move it to `scene` module.
- Derive `Debug` for `Replication` and `Ignored<T>`.

### Removed

- `AckedTicks` resource.
- `TicksMap` resource.

### Fixed

- Fix missing reset of `RepliconTick` on server disconnect.
- Fix replication of removals that happened after replication on the same frame.

## [0.17.0] - 2023-11-13

### Added

- Tracing for replication messages.
- `Debug` derive for `LastRepliconTick`.

### Changed

- Update to Bevy 0.12.

## [0.16.0] - 2023-10-30

### Added

- API to configure max channel usage bytes in `NetworkChannels`.

### Changed

- Rename `SendPolicy` into `EventType`.
- Rename `NetworkChannels::server_channels` into `NetworkChannels::get_server_configs`.
- Rename `NetworkChannels::client_channels` into `NetworkChannels::get_client_configs`.

## [0.15.1] - 2023-10-22

### Changed

- Register `Replication` type and add `#[reflect(Component)]`.

## [0.15.0] - 2023-10-21

### Added

- `network_event::server_event::send` helper for server events in custom sending functions.
- Optional `ClientDiagnosticsPlugin`, which writes diagnostics every second.
- `Reflect` derive for `Replication`.

### Changed

- Optimize despawn tracking.
- Hide `id` field in `EventChannel` and add `Clone` and `Copy` impls for it.
- Remove special functions for reflect events and advise users to write them manually instead. Reflect events are easier now because sometimes you can directly use reflect serializers from Bevy instead of manually writing serde traits.
- Do no trigger server events before world update arrival.

### Removed

- `Debug` requirement for events.

## [0.14.0] - 2023-10-05

### Added

- The ability to pre-spawn entities on client.

### Changed

- Rename `NetworkEntityMap` to `ServerEntityMap`.

## [0.13.0] - 2023-10-04

### Added

- The ability to set custom despawn and component removal functions.
- `TickPolicy::EveryFrame` to update `RepliconTick` every frame.

### Changed

- Use more compact varint encoding for entities.
- Now all replication functions accept `RepliconTick`.
- Rename `NetworkTick` into `RepliconTick` and move it into `server` module.
- Rename `LastTick` into `LastRepliconTick`.

### Removed

- `derive_more` dependency.

### Fixed

- Fix the entire world was always sent instead of changes.
- Fix crash with several entities spawned and updated.

## [0.12.0] - 2023-10-01

### Added

- High-level API to extract replicated entities into `DynamicScene`.

### Changed

- Hide `ReplicationRules` from public API.
- Move logic related to replication rules to `replicon_core::replication_rules` module.

## [0.11.0] - 2023-09-25

### Changed

- Serialize all components and events using varint.
- Serialize entities in optimal way by writing its index and generation as separate varints.
- Hide `ReplicationId`, `ReplicationInfo` and related methods from `ReplicationRules` from public API.
- Rename `ReplicationRules::replication_id` into `ReplicationRules::replication_marker_id`.
- Use serialization buffer cache per client for replication.
- Correctly handle old values on packet reordering.
- Bevy's `Tick` was replaced with dedicated type `NetworkTick` that increments on server update, so it can be used to provide information about time. `AckedTick` was replaced with `ServerTicks` that also contains mappings from `NetworkTick` to Bevy's `Tick` and current `NetworkTick`.
- Functions in `AppReplicationExt::replicate_with` now accept bytes cursor for memory reuse and return serialization errors.
- Rename `ReplicationCore` into `RepliconCore` with its module for clarity.
- `MapNetworkEntities` now accepts generic `Mapper` and doesn't have error handling and deserialiation functions now accept `NetworkEntityMap`. This allowed us to lazily map entities on client without extra allocation.
- Make `LastTick` public.

## [0.10.0] - 2023-09-13

### Changed

- `MapEventEntities` was renamed into `MapNetworkEntities` and now used for both components and events. Built-in `MapEntities` trait from Bevy is not suited for network case for now.
- `AppReplicationExt::not_replicate_with` was replaced with component marker `Ignored<T>`.
- Reflection was replaced with plain serialization. Now components need to implement serde traits and no longer need any reflection. This reduced reduced message sizes a lot. Because of this mapped components now need to be registered with `AppReplicationExt::replicate_mapped`.
- Derive `Clone` and `Copy` for `Replication`.
- Make `ServerPlugin` fields private and add `ServerPlugin::new`.
- Make `AckedTicks` public.
- Make `NetworkEntityMap` public.

## [0.9.1] - 2023-08-05

### Fixed

- Fix event cleanup.

## [0.9.0] - 2023-08-01

### Added

- `ClientSet` now available from `prelude`.

### Changed

- Move `has_authority` to `server` module.

## [0.8.0] - 2023-07-28

### Changed

- Put systems in `PreUpdate` and `PostUpdate` to avoid one frame delay.
- Reorganize `ServerSet` and move client-related systems to `ClientSet`.

## [0.7.1] - 2023-07-20

### Changed

- Re-export `transport` module from `bevy_renet`.

## [0.7.0] - 2023-07-20

### Changed

- Update to `bevy` 0.11.
- Mappable network events now need to implement `MapEventEntities` instead of `MapEntities`.

### Removed

- `ClientState` and `ServerState`, use conditions from `bevy_renet` and `resource_added` / `resource_exists` / `resource_removed`.
- `ServerSet::Authority`, use `has_authority` instead.

## [0.6.1] - 2023-07-09

### Changed

- Update `ParentSync` in `CoreSet::PostUpdate` to avoid one frame delay.

## [0.6.0] - 2023-07-08

### Added

- `SendPolicy` added to API of event-creation for user control of delivery guarantee (reliability and ordering).

### Changed

- `ParentSync` no longer accepts parent entity and just synchronizes hierarchy automatically if present.

## [0.5.0] - 2023-06-26

### Added

- `ServerSet::ReceiveEvent` and `ServerSet::SendEvent` for more fine-grained control of scheduling for event handling.

### Changed

- Update server to use `TickPolicy` instead of requiring a tick rate.

### Fixed

- Unspecified system ordering could cause tick acks to be ordered on the wrong side of world diff handling.
- Crash after adding events without `ServerPlugin` or `ClientPlugin`.

## [0.4.0] - 2023-05-26

### Changed

- Swap `registry` and `event` arguments in `BuildEventSerializer` for consistency with `ReflectSerializer`.
- Update to `bevy_renet` 0.0.12.

## [0.3.0] - 2023-04-15

### Added

- Support for sending events that contains `Box<dyn Reflect>` via custom serialization implementation.

### Changed

- Accept receiving system in `add_client_event_with` and sending system in `add_server_event_with`.
- Make `EventChannel<T>` public.

## [0.2.3] - 2023-04-09

### Fixed

- Panic that could occur when deleting `RenetServer` or `RenetClient` resources.

## [0.2.2] - 2023-04-05

### Fixed

- Do not panic if an entity was already despawned on client.

## [0.2.1] - 2023-04-02

### Fixed

- Incorrect last tick detection.

## [0.2.0] - 2023-04-01

### Changed

- Use `#[reflect(MapEntities)]` from Bevy 0.10.1 instead of custom `#[reflect(MapEntity)]`.

### Fixed

- Tick checks after overflow.

## [0.1.0] - 2023-03-28

Initial release after separation from [Project Harmonia](https://github.com/simgine/project_harmonia).

[unreleased]: https://github.com/simgine/bevy_replicon/compare/v0.36.1...HEAD
[0.36.1]: https://github.com/simgine/bevy_replicon/compare/v0.36.0...v0.36.1
[0.36.0]: https://github.com/simgine/bevy_replicon/compare/v0.35.1...v0.36.0
[0.35.1]: https://github.com/simgine/bevy_replicon/compare/v0.35.0...v0.35.1
[0.35.0]: https://github.com/simgine/bevy_replicon/compare/v0.34.4...v0.35.0
[0.34.4]: https://github.com/simgine/bevy_replicon/compare/v0.34.3...v0.34.4
[0.34.3]: https://github.com/simgine/bevy_replicon/compare/v0.34.1...v0.34.3
[0.34.1]: https://github.com/simgine/bevy_replicon/compare/v0.34.0...v0.34.1
[0.34.0]: https://github.com/simgine/bevy_replicon/compare/v0.33.0...v0.34.0
[0.33.0]: https://github.com/simgine/bevy_replicon/compare/v0.32.2...v0.33.0
[0.32.2]: https://github.com/simgine/bevy_replicon/compare/v0.32.1...v0.32.2
[0.32.1]: https://github.com/simgine/bevy_replicon/compare/v0.32.0...v0.32.1
[0.32.0]: https://github.com/simgine/bevy_replicon/compare/v0.31.1...v0.32.0
[0.31.1]: https://github.com/simgine/bevy_replicon/compare/v0.31.0...v0.31.1
[0.31.0]: https://github.com/simgine/bevy_replicon/compare/v0.30.1...v0.31.0
[0.30.1]: https://github.com/simgine/bevy_replicon/compare/v0.30.0...v0.30.1
[0.30.0]: https://github.com/simgine/bevy_replicon/compare/v0.29.2...v0.30.0
[0.29.2]: https://github.com/simgine/bevy_replicon/compare/v0.29.1...v0.29.2
[0.29.1]: https://github.com/simgine/bevy_replicon/compare/v0.29.0...v0.29.1
[0.29.0]: https://github.com/simgine/bevy_replicon/compare/v0.28.4...v0.29.0
[0.28.4]: https://github.com/simgine/bevy_replicon/compare/v0.28.3...v0.28.4
[0.28.3]: https://github.com/simgine/bevy_replicon/compare/v0.28.2...v0.28.3
[0.28.2]: https://github.com/simgine/bevy_replicon/compare/v0.28.1...v0.28.2
[0.28.1]: https://github.com/simgine/bevy_replicon/compare/v0.28.0...v0.28.1
[0.28.0]: https://github.com/simgine/bevy_replicon/compare/v0.27.0...v0.28.0
[0.27.0]: https://github.com/simgine/bevy_replicon/compare/v0.26.3...v0.27.0
[0.26.3]: https://github.com/simgine/bevy_replicon/compare/v0.26.2...v0.26.3
[0.26.2]: https://github.com/simgine/bevy_replicon/compare/v0.26.1...v0.26.2
[0.26.1]: https://github.com/simgine/bevy_replicon/compare/v0.26.0...v0.26.1
[0.26.0]: https://github.com/simgine/bevy_replicon/compare/v0.25.3...v0.26.0
[0.25.3]: https://github.com/simgine/bevy_replicon/compare/v0.25.2...v0.25.3
[0.25.2]: https://github.com/simgine/bevy_replicon/compare/v0.25.1...v0.25.2
[0.25.1]: https://github.com/simgine/bevy_replicon/compare/v0.25.0...v0.25.1
[0.25.0]: https://github.com/simgine/bevy_replicon/compare/v0.24.1...v0.25.0
[0.24.1]: https://github.com/simgine/bevy_replicon/compare/v0.24.0...v0.24.1
[0.24.0]: https://github.com/simgine/bevy_replicon/compare/v0.23.0...v0.24.0
[0.23.0]: https://github.com/simgine/bevy_replicon/compare/v0.22.0...v0.23.0
[0.22.0]: https://github.com/simgine/bevy_replicon/compare/v0.21.2...v0.22.0
[0.21.2]: https://github.com/simgine/bevy_replicon/compare/v0.21.1...v0.21.2
[0.21.1]: https://github.com/simgine/bevy_replicon/compare/v0.21.0...v0.21.1
[0.21.0]: https://github.com/simgine/bevy_replicon/compare/v0.20.0...v0.21.0
[0.20.0]: https://github.com/simgine/bevy_replicon/compare/v0.19.0...v0.20.0
[0.19.0]: https://github.com/simgine/bevy_replicon/compare/v0.18.2...v0.19.0
[0.18.2]: https://github.com/simgine/bevy_replicon/compare/v0.18.1...v0.18.2
[0.18.1]: https://github.com/simgine/bevy_replicon/compare/v0.18.0...v0.18.1
[0.18.0]: https://github.com/simgine/bevy_replicon/compare/v0.17.0...v0.18.0
[0.17.0]: https://github.com/simgine/bevy_replicon/compare/v0.16.0...v0.17.0
[0.16.0]: https://github.com/simgine/bevy_replicon/compare/v0.15.1...v0.16.0
[0.15.1]: https://github.com/simgine/bevy_replicon/compare/v0.15.0...v0.15.1
[0.15.0]: https://github.com/simgine/bevy_replicon/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/simgine/bevy_replicon/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/simgine/bevy_replicon/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/simgine/bevy_replicon/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/simgine/bevy_replicon/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/simgine/bevy_replicon/compare/v0.9.1...v0.10.0
[0.9.1]: https://github.com/simgine/bevy_replicon/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/simgine/bevy_replicon/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/simgine/bevy_replicon/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/simgine/bevy_replicon/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/simgine/bevy_replicon/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/simgine/bevy_replicon/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/simgine/bevy_replicon/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/simgine/bevy_replicon/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/simgine/bevy_replicon/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/simgine/bevy_replicon/compare/v0.2.3...v0.3.0
[0.2.3]: https://github.com/simgine/bevy_replicon/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/simgine/bevy_replicon/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/simgine/bevy_replicon/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/simgine/bevy_replicon/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/simgine/bevy_replicon/releases/tag/v0.1.0
