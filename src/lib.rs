/*!
A server-authoritative replication crate for [Bevy](https://bevyengine.org).

# Quick start

The library doesn't provide any I/O, so you need to add a
[messaging backend](https://github.com/simgine/bevy_replicon#messaging-backends).
If you want to write an integration yourself, see [`shared::backend`] module.

## Prelude

We provide a [`prelude`] module, which exports most of the typically used traits and types.

## Plugins

Add [`RepliconPlugins`] and plugins for your chosen messaging backend to your app:

```
use bevy::prelude::*;
use bevy_replicon::prelude::*;
# use bevy::app::PluginGroupBuilder;

let mut app = App::new();
app.add_plugins((MinimalPlugins, RepliconPlugins, MyMessagingPlugins));
# struct MyMessagingPlugins;
# impl PluginGroup for MyMessagingPlugins {
#     fn build(self) -> PluginGroupBuilder {
#         PluginGroupBuilder::start::<Self>()
#     }
# }
```

## Server and client creation

This part is specific to your messaging backend. For `bevy_replicon_renet`,
see [this section](https://docs.rs/bevy_replicon_renet#server-and-client-creation).

Backends manage [`RepliconServer`] and [`RepliconClient`] resources. They can be used
to obtain things like state or statistic in backend-independent way.

On server connected clients represented as entities with [`ConnectedClient`] component.
Their data represented as components, such as [`NetworkStats`]. Users can also attach their
own metadata to them or even replicate these entiteis back to clients.

These entities are automatically spawned and despawned by the messaging backend. You can
also despawn them yourself to trigger a disconnect or use the [`DisconnectRequest`] event
to disconnect after sending messages.

You can use [`Trigger<OnAdd, ConnectedClient>`] to react to new connections,
or use backend-provided events if you need the disconnect reason.

## Replication

It's a process of exchanging data in order to keep the world in sync. Replicon
provides a high-level API to automate server-authoritative replication.

Replication happens only from server to clients. It's necessary to prevent cheating.
If you need to send information from clients to the server, use
[events](#network-events-and-triggers).

For implementation details see [`ServerChannel`](shared::backend::channels::ServerChannel).

### Tick rate

Typically updates are not sent every frame. Instead, they are sent at a certain interval
to save traffic.

On server current tick stored in [`RepliconTick`] resource. Replication runs when this resource changes.

You can use [`TickPolicy::Manual`] and then add the [`increment_tick`](server::increment_tick)
system to [`FixedUpdate`]:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# let mut app = App::new();
app.add_plugins(
    RepliconPlugins
        .build()
        .disable::<ClientPlugin>()
        .set(ServerPlugin {
            tick_policy: TickPolicy::Manual,
            ..Default::default()
        }),
)
.add_systems(FixedPreUpdate, bevy_replicon::server::increment_tick);
```

### Entities

By default no entities are replicated. Add the [`Replicated`] marker
component on the server for entities you want to replicate.

On clients [`Replicated`] will be automatically inserted to newly-replicated entities.

If you remove the [`Replicated`] component from an entity on the server, it will be despawned on all clients.

Entity IDs differ between clients and server. As a result, clients maps server entities to local entities
on receive. These mappings are stored in the [`ServerEntityMap`](shared::server_entity_map::ServerEntityMap)
resource.

### Components

Components will be replicated only on entities marked for replication.
By default no components are replicated, you need to define rules for it.

Use [`AppRuleExt::replicate`] to create a replication rule for a single component:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.replicate::<ExampleComponent>();

#[derive(Component, Deserialize, Serialize)]
struct ExampleComponent;
```

If your component contains an entity, it cannot be deserialized as is
because entities inside components also need to be mapped. Therefore,
to properly replicate such components, mark fields containing entities with
`#[entities]`. See [`Component::map_entities`] for details.

By default all components are serialized with [`postcard`].
In order to serialize Bevy components you need to enable the `serialize` feature on Bevy.

If your component doesn't implement serde traits or you want to customize the serialization
(for example, quantize, skip some fields or apply compression), you can use
[`AppRuleExt::replicate_with`].

You can also create a rule for multiple components. Use [`AppRuleExt::replicate_bundle`],
or pass a tuple of [`RuleFns`] to [`AppRuleExt::replicate_with`]. The components will only
be replicated if all of them are present on the entity. This also allows you to specialize
serialization and deserialization based on specific entity components.

All functions also have `_filtered` variants that additionally accept archetypal filters,
similar to [`Query`]. They are separate functions because Rust doesn't allow default generics
for functions. For more details, see [`AppRuleExt::replicate_filtered`].

#### Required components

You don't want to replicate all components because not all of them are
necessary to send over the network. Components that can be calculated on the client can
be inserted using Bevy's required components feature.

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
// Replicate only transform and player marker.
app.replicate::<Transform>()
    .replicate::<Player>()
    .add_observer(init_player_mesh);

fn init_player_mesh(
    trigger: Trigger<OnAdd, Mesh2d>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut players: Query<&mut Mesh2d>,
) {
    let mut mesh = players.get_mut(trigger.target()).unwrap();
    **mesh = meshes.add(Capsule2d::default());
}

/// Main player component.
///
/// [`NotReplicatedComponent`] and [`Replicated`] will be implicitly inserted after spawning on
/// both the server and clients.
///
/// [`Replicated`] is always inserted on the client after replication, regardless of whether it is marked
/// as required. However, it may still be useful to mark it as required if you want to avoid
/// inserting it explicitly on the server.
#[derive(Component, Deserialize, Serialize)]
#[require(Replicated, NotReplicatedComponent, Mesh2d)]
struct Player;

#[derive(Default, Component)]
struct NotReplicatedComponent;
```

This pairs nicely with server state serialization and keeps saves clean.
You can use [`scene::replicate_into`] to fill [`DynamicScene`] with replicated entities and their components.
On deserialization all missing required components will be inserted, and initialization
systems will restore the correct game state.

If a component can't be used with the required components due to the inability to insert it
without world access, you can create an observer for a replicated marker and insert the actual
component inside it. However, it's preferred to use required components when possible. For example,
it's better to require a [`Handle<T>`] with a default value that doesn't point to any asset
and initialize it later in a hook or observer. This way you avoid archetype moves in ECS.

#### Mutability

There are two ways to change a component value on an entity: re-inserting it or mutating it.

We use Bevy’s change detection to track and send changes. However, it does not distinguish between modifications
and re-insertions. This is why we simply send the list of changes and decide how to apply them on the client.
By default, this behavior is based on [`Component::Mutability`].

When a component is [`Mutable`](bevy::ecs::component::Mutable), we check whether it already exists on the entity.
If it doesn’t, we insert it. If it does, we mutate it. This means that if you insert a component into an entity
that already has it on the server, the client will treat it as a mutation. As a result, triggers may behave
differently on the client and server. If your game logic relies on this semantic, mark your component as
[`Immutable`](bevy::ecs::component::Immutable). For such components, replication will always be applied via insertion.

This behavior is also configurable via [client markers](#client-markers).

#### Component relations

Some components depend on each other. For example, [`ChildOf`] and [`Children`]. You can enable
replication only for [`ChildOf`] and [`Children`] will be updated automatically on insertion.
This will emit a [`B0004`](https://bevy.org/learn/errors/b0004) warning which can be safely ignored.
See [#19776](https://github.com/bevyengine/bevy/issues/19776) for more details.

You can also ensure that their mutations arrive in sync by using [`SyncRelatedAppExt::sync_related_entities`].

#### Deterministic replication

Up until now, we've covered only authoritative replication (AR), where the server is the source of truth
and continuously sends changes. However, sometimes you may want to send data only once and simulate independently,
relying on determinism. This approach is called deterministic replication (DR).

For example, you might use AR for things like player health, and DR for moving platform positions to reduce
network traffic.

Use [`AppRuleExt::replicate_once`] to replicate only the initial value of a component.
You can configure this per-component within a replication rule using [`AppRuleExt::replicate_with`].

See also [server events](#from-server-to-client), which are also useful for DR.

## Network events and triggers

This replaces RPCs (remote procedure calls) in other engines and,
unlike replication, can be sent both from server to clients and from clients to
server.

### From client to server

To send specific events from client to server, you need to register the event
with [`ClientEventAppExt::add_client_event`] instead of [`App::add_event`].

Events include [`Channel`] to configure delivery guarantees (reliability and
ordering).

These events will appear on server as [`FromClient`] wrapper event that
contains sender ID and the sent event.

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_client_event::<ExampleEvent>(Channel::Ordered)
    .add_systems(
        PreUpdate,
        receive_events
            .after(ServerSet::Receive)
            .run_if(server_running),
    )
    .add_systems(
        PostUpdate,
        send_events.before(ClientSet::Send).run_if(client_connected),
    );

fn send_events(mut events: EventWriter<ExampleEvent>) {
    events.write_default();
}

fn receive_events(mut events: EventReader<FromClient<ExampleEvent>>) {
    for FromClient { client_id, event } in events.read() {
        info!("received event `{event:?}` from client `{client_id}`");
    }
}

#[derive(Debug, Default, Deserialize, Event, Serialize)]
struct ExampleEvent;
```

If an event contains an entity, implement
[`MapEntities`](bevy::ecs::entity::MapEntities) for it and use use
[`ClientEventAppExt::add_mapped_client_event`] instead.

There is also [`ClientEventAppExt::add_client_event_with`] to register an event with special serialization and
deserialization functions. This could be used for sending events that contain [`Box<dyn PartialReflect>`], which
require access to the [`AppTypeRegistry`] resource. Don't forget to validate the contents of every
[`Box<dyn PartialReflect>`] from a client, it could be anything!

Alternatively, you can use triggers with a similar API. First, you need to register the event
using [`ClientTriggerAppExt::add_client_trigger`], and then use [`ClientTriggerExt::client_trigger`].

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_client_trigger::<ExampleEvent>(Channel::Ordered)
    .add_observer(receive_events)
    .add_systems(Update, send_events.run_if(client_connected));

fn send_events(mut commands: Commands) {
    commands.client_trigger(ExampleEvent);
}

fn receive_events(trigger: Trigger<FromClient<ExampleEvent>>) {
    info!("received event `{:?}` from client `{}`", **trigger, trigger.client_id);
}
# #[derive(Event, Debug, Deserialize, Serialize)]
# struct ExampleEvent;
```

Trigger targets are also supported via [`ClientTriggerExt::client_trigger_targets`], no change
in registration needed. Target entities will be automatically mapped to server entities before sending.

For event triggers with entities inside use [`ClientTriggerAppExt::add_mapped_client_trigger`].
Similar to events, serialization can also be customized with [`ClientTriggerAppExt::add_client_trigger_with`].

### From server to client

A similar technique is used to send events from server to clients. To do this,
register the event with [`ServerEventAppExt::add_server_event`]
and send it from server using [`ToClients`]. This wrapper contains send parameters
and the event itself.

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_server_event::<ExampleEvent>(Channel::Ordered)
    .add_systems(
        PreUpdate,
        receive_events
            .after(ClientSet::Receive)
            .run_if(client_connected),
    )
    .add_systems(
        PostUpdate,
        send_events.before(ServerSet::Send).run_if(server_running),
    );

fn send_events(mut events: EventWriter<ToClients<ExampleEvent>>) {
    events.write(ToClients {
        mode: SendMode::Broadcast,
        event: ExampleEvent,
    });
}

fn receive_events(mut events: EventReader<ExampleEvent>) {
    for event in events.read() {
        info!("received event {event:?} from server");
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Event, Serialize)]
struct ExampleEvent;
```

Just like for client events, we provide [`ServerEventAppExt::add_mapped_server_event`]
and [`ServerEventAppExt::add_server_event_with`].

Trigger-based API available for server events as well. First, you need to register the event
with [`ServerTriggerAppExt::add_server_trigger`] and then use [`ServerTriggerExt::server_trigger`]:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
# use serde::{Deserialize, Serialize};
# let mut app = App::new();
# app.add_plugins(RepliconPlugins);
app.add_server_trigger::<ExampleEvent>(Channel::Ordered)
    .add_observer(receive_events)
    .add_systems(Update, send_events.run_if(server_running));

fn send_events(mut commands: Commands) {
    commands.server_trigger(ToClients {
        mode: SendMode::Broadcast,
        event: ExampleEvent,
    });
}

fn receive_events(trigger: Trigger<ExampleEvent>) {
    info!("received event {:?} from server", *trigger);
}
# #[derive(Event, Debug, Deserialize, Serialize)]
# struct ExampleEvent;
```

And just like for client trigger, we provide [`ServerTriggerAppExt::add_mapped_server_trigger`]
and [`ServerTriggerAppExt::add_server_trigger_with`].

We guarantee that clients will never receive events that point to an entity or require specific
component to be presentt which client haven't received yet. For more details see the documentation on
[`ServerEventAppExt::make_event_independent`].

## Abstracting over configurations

Depending on the game, you may need to support some of these configurations:

- Client
- Dedicated (headless) server
- Listen server (where the server is also a client)
- Singleplayer

Theere are 2 ways to support multiple configurations at the same time.

### Classic way

Just split client and server logic. Then for listen server and singleplayer run both the server and client,
just don't accept outside connections for singleplayer.

However, **running the client and server in a single app is not supported**. We rely on change detection to
decide on which data to send, and since the world is shared, applying replication will trigger changes.
To avoid this, you can use one of the following workarounds:

- Two Bevy apps inside a single process, running in separate threads.
- Two executables. After starting the client app, the server starts in the background.

It's not easy to set up and requires more resources due to the synchronization between two worlds.
This is why, while it's possible to use Replicon this way, we recommend a different approach.

### The recommended way

Instead of recreating full client-server logic, we provide a way to emulate client and server functionality
without actually running them:

- Client configuration runs only the client and its logic.
- Dedicated server configuration runs only the server and its logic (all client logic usually compiled out).
- Listen server configuration runs only the server and both logics.
- Singleplayer configuration doesn't run the client or server but runs both logics.

To achieve this, just use provided [run conditions](shared::common_conditions):

- Use [`server_or_singleplayer`] for systems that require server authority. For example, systems that
  apply damage or send server events.
- Use client or server conditions like [`client_connecting`], [`client_connected`], [`server_running`], etc.
  **only** for miscellaneous things, like display a connection message or a menu to kick connected players
  (things that actually require server or client running)
- For everything else don't use Replicon's conditions.

We also provide [`ClientSet`] and [`ServerSet`] to schedule your system at specific time in the frame.
For example, you can run your systems right after receive using [`ClientSet::Receive`] or [`ServerSet::Receive`].

Everything else is done automatically by the crate. All provided
[examples](https://github.com/simgine/bevy_replicon/tree/master/bevy_replicon_example_backend/examples)
use this approach.

Internally we run replication sending system only if [`server_running`] and replication receiving
only if [`client_connected`]. This way for singleplayer replication systems won't run at all and
for listen server replication will only be sending (server world is already in the correct state).

For events it's a bit trickier. For all client events we internally drain events as `E` and re-emit
them as [`FromClient<E>`] locally with [`ClientId::Server`] if [`server_or_singleplayer`] is `true`.
For server events we drain [`ToClients<E>`] and, if the [`ClientId::Server`] is the recipient of the event,
re-emit it as `E` locally.

## Organizing your game code

If you support a dedicated server, it's recommended to split your game logic into "client", "server", and "shared"
crates. This way, you can compile out all unnecessary code for the dedicated server configuration.
Alternatively, you can use [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html) and split
the logic into modules.

We provide `client` and `server` features to disable unneeded functionality for this use case.
You will need to disable similar features on your messaing backend crate too.

<div class="warning">

Make sure that the component and event registration order is the same on the client and server. Simply put all
registration code in your "shared" crate.

</div>

If you don't need to support a dedicated server configuration, it's easier to keep the logic grouped together.
Splitting your game into crates is still useful, but it should be done logically rather than on server/client
basis.

## Advanced features

### Authorization

Connected clients are considered authorized when they have the [`AuthorizedClient`] component. Without this
component, clients can only receive events marked as independent via [`ServerEventAppExt::make_event_independent`]
or [`ServerTriggerAppExt::make_trigger_independent`]. Additionally, some components on connected clients are only present
after replication starts. See the required components for [`AuthorizedClient`] for details.

By default, this component is automatically inserted when the client and server [`ProtocolHash`] matches.
This behavior can be customized via [`RepliconSharedPlugin::auth_method`].

### Client visibility

You can control which parts of the world are visible for each client by setting visibility policy
in [`ServerPlugin`] to [`VisibilityPolicy::Whitelist`] or [`VisibilityPolicy::Blacklist`].

To set which entity is visible, you need to use the [`ClientVisibility`] component
on authorized clients.

Check also the [corresponding section](https://github.com/simgine/bevy_replicon#visibility)
in our README for more high-level abstractions.

### Prioritization

By default, all unacknowledged mutations are sent every tick. This can be expensive if you
have many entities with mutating components. The [`PriorityMap`] component lets you configure
how often mutations are sent for each entity on authorized clients. See its documentation for
more details.

In addition, [`ClientVisibility`] can be used to further reduce bandwidth by hiding entities
that are irrelevant to a given client.

### Spawning the client on an entity first

If you want the server to replicate an entity into a client entity that was already spawned, see [`ClientEntityMap`].

This can be useful for certain types of games. For example, spawning bullets on the client immediately without
waiting for replication.

### Interpolation and/or client-side prediction

Due to network round-trip time and [tick rate](#tick-rate), you may notice that the state isn't updated
immediately. This might be fine depending on your type of game. However, sometimes this needs to be visually hidden.

To make value updates look smooth, you can just to interpolate the received state. If the input delay doesn't matter
for your type of game, it can be enough.

But if your game is fast-paced, waiting for server to receive your inputs and replicate the state back might
might be unacceptable. The solution is to predict simulation on the client.

However, it introduces another problem - misspredictions. For example, player 1 might predict movement to point X,
while player 2 might have stunned it, the player 1 just didn't receive it yet. To solve this, client must apply
the received state from server and replay its inputs.

How much to predict also depends on the game. Common approaches are:

- Predict individual entities. Common for shooters or games where you don't have many entities. In the case of
  a misprediction or non-deterministic mismatch, the state will be corrected. Determinism is important for this
  approach to reduce the number of rollbacks.
- Predict the entire world. Common for physics-based games or games with many entities. With this approach,
  all predicted entities are rolled back to the oldest received tick. For example, if one entity have confirmed tick 1
  and another entity have confirmed tick 2, both entities are rolled back to tick 1. This approach is usually more expensive
  but produces better results for physics. Additionally, if there are many predicted entities, it might even be faster
  since there's no need to check each entity for misprediction. The more entities you predict, the more likely it is
  that at least one will trigger a world rollback. So with this approach client usually just always rollbacks.

We don't have these features built-in, but we provide a low-level API to implement these abstractions on top.
Check the [corresponding section](https://github.com/simgine/bevy_replicon#interpolation-andor-rollback)
in our README for existing implementations.

If you run at a low tick rate, you may also need to smooth transforms in listen server or single player modes.
This is different from client-side - it should happen between the previous and the current [`FixedMain`](bevy::app::FixedMain)
runs. See the [interpolation and/or rollback](https://github.com/simgine/bevy_replicon?tab=readme-ov-file#interpolation-andor-rollback)
section for available solutions.

#### Client markers

To apply interpolation or store value history for client-side prediction, you need to override how components are
written. However, the server knows nothing about archetypes on the client, and while some entities need to be predicted,
others might need to be interpolated.

This is why writing functions are marker-based. First, you register a marker using [`AppMarkerExt::register_marker<M>`].
Then you can override how specific component is written and removed using [`AppMarkerExt::set_marker_fns<M, C>`].

You can control marker priority or enable processing of old values using [`AppMarkerExt::register_marker_with<M>`].

### Ticks information

This requires an understanding of how replication works. See the documentation on
[`ServerChannel`](shared::backend::channels::ServerChannel) and [this section](#eventual-consistency) for more details.

To get information about confirmed ticks for individual entities, we provide
[`ConfirmHistory`](client::confirm_history::ConfirmHistory) along with the [`EntityReplicated`](client::confirm_history::ConfirmHistory)
trigger. This component is updated when any replication for its entity is received. However, we don't update this component if an entity
hasn't changed for performance reasons.

This means that to check if a tick is confirmed for an entity, you also need to check the received messages for this
tick. The [`ServerUpdateTick`](client::ServerUpdateTick) resource stores the last received tick from an update message.
The [`ServerMutateTicks`](client::server_mutate_ticks::ServerMutateTicks) resource and
[`MutateTickReceived`](client::server_mutate_ticks::MutateTickReceived) trigger provide information about received mutate
messages for the past 64 ticks.

A tick for an entity is confirmed if one of the following is true:
- [`ServerUpdateTick`](client::ServerUpdateTick) is greater than the tick.
- [`ConfirmHistory`](client::confirm_history::ConfirmHistory) is greater than the tick.
- [`ServerMutateTicks`](client::server_mutate_ticks::ServerMutateTicks) reports that for at least one of the next ticks, all update
  messages have been received.

### Optimizing entity serialization

Serialization of [`Entity`] is optimized for use in scenes, but it’s not very efficient for networking. Because of this, we use our own implementation.
See the [`compact_entity`] module if you want to apply this optimization to entities in your own types. You can also use
[`postcard_utils::entity_to_extend_mut`] and [`postcard_utils::entity_from_buf`] for manual writing.

# Eventual consistency

All events, inserts, removals and despawns will be applied to clients in the same order as on the server.

However, if you insert/mutate a component and immediately remove it, the client will only receive the removal because the component value
won't exist in the [`World`] during the replication process. But removal followed by insertion will work as expected since we buffer removals.

Entity component mutations may be applied to clients in a different order than on the server.
For example, if two entities are spawned in tick 1 on the server and their components are mutated in tick 2,
then the client is guaranteed to see the spawns at the same tick, but the component mutations may appear later (but not earlier).

If a component is dependent on other data, mutations to the component will only be applied to the client when that data has arrived.
So if your component references another entity, mutations to that component will only be applied when the referenced entity has been spawned on the client.

Mutations for despawned entities will be discarded automatically, but events or components may reference despawned entities and should be handled with that in mind.

Clients should never assume their world state is the same as the server's on any given tick value-wise.
World state on the client is only "eventually consistent" with the server's.

# Troubleshooting

If you face any issue, try to enable logging to see what is going on.
To enable logging, you can temporarily set `RUST_LOG` environment variable to `bevy_replicon=debug`
(or `bevy_replicon=trace` for more noisy output) like this:

```bash
RUST_LOG=bevy_replicon=debug cargo run
```

You can also filter by specific module like this:

```bash
RUST_LOG=bevy_replicon::shared::protocol=debug cargo run
```

The exact method depends on the OS shell.

Alternatively you can configure `LogPlugin` from Bevy to make it permanent.

For deserialization errors on client we use `error` level which should be visible by default.
But on server we use `debug` for it to avoid flooding server logs with errors caused by clients.
*/
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![no_std]

extern crate alloc;

#[cfg(feature = "client")]
pub mod client;
pub mod compact_entity;
pub mod postcard_utils;
#[cfg(feature = "scene")]
pub mod scene;
#[cfg(feature = "server")]
pub mod server;
pub mod shared;
#[cfg(all(feature = "server", feature = "client"))]
pub mod test_app;

pub mod prelude {
    pub use super::{
        RepliconPlugins,
        shared::{
            AuthMethod, RepliconSharedPlugin,
            backend::{
                DisconnectRequest,
                channels::{Channel, RepliconChannels},
                connected_client::{ConnectedClient, NetworkStats},
                replicon_client::{RepliconClient, RepliconClientStatus},
                replicon_server::RepliconServer,
            },
            client_id::ClientId,
            common_conditions::*,
            event::{
                client_event::{ClientEventAppExt, FromClient},
                client_trigger::{ClientTriggerAppExt, ClientTriggerExt},
                server_event::{SendMode, ServerEventAppExt, ToClients},
                server_trigger::{ServerTriggerAppExt, ServerTriggerExt},
            },
            protocol::{ProtocolHash, ProtocolHasher, ProtocolMismatch},
            replication::{
                Replicated,
                command_markers::AppMarkerExt,
                registry::rule_fns::RuleFns,
                rules::{AppRuleExt, component::ReplicationMode},
            },
            replicon_tick::RepliconTick,
        },
    };

    #[cfg(feature = "client")]
    pub use super::client::{
        ClientPlugin, ClientReplicationStats, ClientSet, event::ClientEventPlugin,
    };

    #[cfg(feature = "server")]
    pub use super::server::{
        AuthorizedClient, PriorityMap, ServerPlugin, ServerSet, TickPolicy, VisibilityPolicy,
        client_entity_map::ClientEntityMap, client_visibility::ClientVisibility,
        event::ServerEventPlugin, related_entities::SyncRelatedAppExt,
    };

    #[cfg(feature = "client_diagnostics")]
    pub use super::client::diagnostics::ClientDiagnosticsPlugin;
}

pub use bytes;
pub use postcard;

use bevy::{app::PluginGroupBuilder, prelude::*};
use prelude::*;

/// Plugin group for all replicon plugins.
///
/// Contains the following:
/// * [`RepliconSharedPlugin`].
/// * [`ServerPlugin`] - with feature `server`.
/// * [`ServerEventPlugin`] - with feature `server`.
/// * [`ClientPlugin`] - with feature `client`.
/// * [`ClientEventPlugin`] - with feature `client`.
/// * [`ClientDiagnosticsPlugin`] - with feature `client_diagnostics`.
pub struct RepliconPlugins;

impl PluginGroup for RepliconPlugins {
    fn build(self) -> PluginGroupBuilder {
        let mut group = PluginGroupBuilder::start::<Self>();
        group = group.add(RepliconSharedPlugin::default());

        #[cfg(feature = "server")]
        {
            group = group.add(ServerPlugin::default()).add(ServerEventPlugin);
        }

        #[cfg(feature = "client")]
        {
            group = group.add(ClientPlugin).add(ClientEventPlugin);
        }

        #[cfg(feature = "client_diagnostics")]
        {
            group = group.add(ClientDiagnosticsPlugin);
        }

        group
    }
}
