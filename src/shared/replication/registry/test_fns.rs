use bevy::prelude::*;
use bytes::Bytes;

use super::{
    FnsId, ReplicationRegistry,
    ctx::{DespawnCtx, RemoveCtx, SerializeCtx, WriteCtx},
};
use crate::{
    prelude::*,
    shared::{
        replication::{
            command_markers::{CommandMarkers, EntityMarkers},
            deferred_entity::{DeferredChanges, DeferredEntity},
            registry::ctx::EntitySpawner,
        },
        server_entity_map::ServerEntityMap,
    },
};

/**
Extension for [`EntityWorldMut`] to call registered replication functions for [`FnsId`].

See also [`ReplicationRegistry::register_rule_fns`].

# Example

This example shows how to call registered functions on an entity:

```
use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    shared::{
        replication::registry::{
            test_fns::TestFnsEntityExt, ReplicationRegistry,
        },
        replicon_tick::RepliconTick,
    },
    prelude::*,
};
use serde::{Deserialize, Serialize};

let mut app = App::new();
app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));

let tick = RepliconTick::default();

// Register rule functions manually to obtain `FnsId`.
let (_, fns_id) = app
    .world_mut()
    .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
        registry.register_rule_fns(world, RuleFns::<ExampleComponent>::default())
    });

let mut entity = app.world_mut().spawn(ExampleComponent);
let data = entity.serialize(fns_id, tick);
entity.remove::<ExampleComponent>();

entity.apply_write(data, fns_id, tick);
assert!(entity.contains::<ExampleComponent>());

entity.apply_remove(fns_id, tick);
assert!(!entity.contains::<ExampleComponent>());

entity.apply_despawn(tick);
let mut components = app.world_mut().query::<&ExampleComponent>();
assert_eq!(components.iter(app.world()).len(), 0);

#[derive(Component, Serialize, Deserialize)]
struct ExampleComponent;
```
**/
pub trait TestFnsEntityExt {
    /// Returns a component serialized using a registered function for it.
    ///
    /// See also [`ReplicationRegistry::register_rule_fns`].
    #[must_use]
    fn serialize(&mut self, fns_id: FnsId, server_tick: RepliconTick) -> Vec<u8>;

    /// Deserializes a component using a registered function for it and
    /// writes it into an entity using a write function based on markers.
    ///
    /// See also [`AppMarkerExt`].
    fn apply_write(
        &mut self,
        bytes: impl Into<Bytes>,
        fns_id: FnsId,
        message_tick: RepliconTick,
    ) -> &mut Self;

    /// Removes a component using a registered function for it.
    ///
    /// See also [`AppMarkerExt`].
    fn apply_remove(&mut self, fns_id: FnsId, message_tick: RepliconTick) -> &mut Self;

    /// Despawns an entity using [`ReplicationRegistry::despawn`].
    fn apply_despawn(self, message_tick: RepliconTick);
}

impl TestFnsEntityExt for EntityWorldMut<'_> {
    fn serialize(&mut self, fns_id: FnsId, server_tick: RepliconTick) -> Vec<u8> {
        let type_registry = self.world().resource::<AppTypeRegistry>();
        let registry = self.world().resource::<ReplicationRegistry>();
        let (_, component_id, fns) = registry.get(fns_id);
        let mut message = Vec::new();
        let ctx = SerializeCtx {
            server_tick,
            component_id,
            type_registry,
        };
        let ptr = self.get_by_id(component_id).unwrap_or_else(|_| {
            let components = self.world().components();
            let component_name = components
                .get_name(component_id)
                .expect("function should require valid component ID");
            panic!("serialization function require entity to have {component_name}");
        });

        unsafe {
            fns.serialize(&ctx, ptr, &mut message)
                .expect("serialization into memory should never fail");
        }

        message
    }

    fn apply_write(
        &mut self,
        data: impl Into<Bytes>,
        fns_id: FnsId,
        message_tick: RepliconTick,
    ) -> &mut Self {
        let mut entity_markers = self.world_scope(EntityMarkers::from_world);
        let command_markers = self.world().resource::<CommandMarkers>();
        entity_markers.read(command_markers, &*self);

        let entity = self.id();
        self.world_scope(|world| {
            world.resource_scope(|world, mut entity_map: Mut<ServerEntityMap>| {
                world.resource_scope(|world, registry: Mut<ReplicationRegistry>| {
                    let type_registry = world.resource::<AppTypeRegistry>().clone();
                    let world_cell = world.as_unsafe_world_cell();
                    // SAFETY: split into `EntitySpawner` and `DeferredEntity`.
                    // The latter won't apply any structural changes until `flush`, and `EntitySpawner` won't be used afterward.
                    let mut spawner = EntitySpawner::new(unsafe { world_cell.world_mut() });
                    let world = unsafe { world_cell.world_mut() };

                    let mut changes = DeferredChanges::default();
                    let mut entity = DeferredEntity::new(world.entity_mut(entity), &mut changes);

                    let (_, component_id, fns) = registry.get(fns_id);
                    let mut ctx = WriteCtx {
                        entity_map: &mut entity_map,
                        type_registry: &type_registry,
                        component_id,
                        message_tick,
                        spawner: &mut spawner,
                        ignore_mapping: false,
                    };

                    fns.write(&mut ctx, &entity_markers, &mut entity, &mut data.into())
                        .expect("writing data into an entity shouldn't fail");

                    entity.flush();
                })
            })
        });

        self
    }

    fn apply_remove(&mut self, fns_id: FnsId, message_tick: RepliconTick) -> &mut Self {
        let mut entity_markers = self.world_scope(EntityMarkers::from_world);
        let command_markers = self.world().resource::<CommandMarkers>();
        entity_markers.read(command_markers, &*self);

        let entity = self.id();
        self.world_scope(|world| {
            world.resource_scope(|world, registry: Mut<ReplicationRegistry>| {
                let mut changes = DeferredChanges::default();
                let mut entity = DeferredEntity::new(world.entity_mut(entity), &mut changes);

                let (_, component_id, fns) = registry.get(fns_id);
                let mut ctx = RemoveCtx {
                    message_tick,
                    component_id,
                };

                fns.remove(&mut ctx, &entity_markers, &mut entity);

                entity.flush();
            })
        });

        self
    }

    fn apply_despawn(self, message_tick: RepliconTick) {
        let registry = self.world().resource::<ReplicationRegistry>();
        let ctx = DespawnCtx { message_tick };
        (registry.despawn)(&ctx, self);
    }
}
