use bevy::prelude::*;
use log::debug;

/**
A resource that exists on the server for mapping server entities to
entities that clients have already spawned. The mappings are sent to clients as part of replication
and injected into the client's [`ServerEntityMap`](crate::shared::server_entity_map::ServerEntityMap).

Sometimes you don't want to wait for the server to spawn something before it appears on the
client – when a client performs an action, they can immediately simulate it on the client,
then match up that entity with the eventual replicated server spawn, rather than have replication spawn
a brand new entity on the client.

In this situation, the client can send the server its pre-spawned entity id, then the server can spawn its own entity
and inject the mapping into its [`ClientEntityMap`].

Replication packets will send a list of such mappings to clients, which will
be inserted into the client's [`ServerEntityMap`](crate::shared::server_entity_map::ServerEntityMap). Using replication
to propagate the mappings ensures any replication messages related to the pre-mapped
server entities will synchronize with updating the client's [`ServerEntityMap`](crate::shared::server_entity_map::ServerEntityMap).

It's a required component for [`AuthorizedClient`](super::AuthorizedClient). So if you want to map entities before enabling
replication, you need to insert this component, already filled with entities.

### Examples

```
use bevy::prelude::*;
use bevy_replicon::prelude::*;

#[derive(Event)]
struct SpawnBullet(Entity);

#[derive(Component)]
struct Bullet;

/// System that shoots a bullet and spawns it on the client.
fn shoot_bullet(mut commands: Commands, mut bullet_events: EventWriter<SpawnBullet>) {
    let entity = commands.spawn(Bullet).id();
    bullet_events.write(SpawnBullet(entity));
}

/// Validation to check if client is not cheating or the simulation is correct.
///
/// Depending on the type of game you may want to correct the client or disconnect it.
/// In this example we just always confirm the spawn.
fn confirm_bullet(
    mut commands: Commands,
    mut bullet_events: EventReader<FromClient<SpawnBullet>>,
    mut clients: Query<&mut ClientEntityMap>,
) {
    for event in bullet_events.read() {
        let bullet = commands.spawn(Bullet).id(); // You can insert more components, they will be sent to the client's entity correctly.
        if let ClientId::Client(client) = event.client_id {
            let mut entity_map = clients.get_mut(client).unwrap();
            entity_map.insert(bullet, event.0);
        }
    }
}
```

If the client is connected and receives the replication data for the server entity mapping,
replicated data will be applied to the client's original entity instead of spawning a new one.
You can detect when the mapping is replicated by querying for [`Added<Replicated>`] on your original
client entity.

If client's original entity is not found, a new entity will be spawned on the client,
just the same as when no client entity is provided.

See also [`RepliconSharedPlugin::auth_method`](crate::shared::RepliconSharedPlugin::auth_method).
**/
#[derive(Debug, Default, Deref, Component)]
pub struct ClientEntityMap(pub(super) Vec<(Entity, Entity)>);

impl ClientEntityMap {
    /// Registers a mapping from server to client entity.
    ///
    /// This will be sent as part of replication data and added to the client's
    /// [`ServerEntityMap`](crate::shared::server_entity_map::ServerEntityMap).
    pub fn insert(&mut self, server_entity: Entity, client_entity: Entity) {
        debug!("mapping server's `{server_entity}` to client's `{client_entity}`");
        self.0.push((server_entity, client_entity));
    }
}
