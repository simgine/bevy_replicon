use core::any::Any;

use bevy::{
    ecs::entity::EntityHashMap,
    prelude::*,
    utils::{TypeIdMap, TypeIdMapExt},
};

/**
Helpers to access replication storage for the entity associated with the context.

Implemented for replication contexts that operate on a specific entity, such as
[`SerializeCtx`](crate::shared::replication::registry::ctx::SerializeCtx) and
[`WriteCtx`](crate::shared::replication::registry::ctx::WriteCtx).

Methods access the [`TypeMap`] associated with the current entity in
[`ReplicationStorage::entities`].

# Examples

Store serialization state associated with the entity currently being serialized:

```
# use bevy::state::app::StatesPlugin;
use bevy::prelude::*;
use bevy_replicon::{
    bytes::Bytes,
    postcard_utils,
    prelude::*,
    shared::replication::registry::{
        ctx::{SerializeCtx, WriteCtx},
        rule_fns::RuleFns,
    },
};
use serde::{Deserialize, Serialize};

# let mut app = App::new();
# app.add_plugins((StatesPlugin, RepliconPlugins));
app.add_observer(store_position_precision)
    .replicate_with(RuleFns::new(serialize_position, deserialize_position));

fn store_position_precision(
    add: On<Add, Precision>,
    precision: Query<&Precision>,
    mut storage: ResMut<ReplicationStorage>,
) {
    let precision = *precision.get(add.entity).unwrap();
    storage.insert(add.entity, precision);
}

fn serialize_position(
    ctx: &mut SerializeCtx,
    position: &Position,
    message: &mut Vec<u8>,
) -> Result<()> {
    let precision = ctx.get_mut::<Precision>().copied().unwrap_or_default();
    let scale = precision.scale();

    let quantized = QuantizedPosition {
        precision,
        x: quantize(position.x, scale),
        y: quantize(position.y, scale),
    };

    postcard_utils::to_extend_mut(&quantized, message)?;
    Ok(())
}

fn deserialize_position(_ctx: &mut WriteCtx, message: &mut Bytes) -> Result<Position> {
    let quantized: QuantizedPosition = postcard_utils::from_buf(message)?;
    let scale = quantized.precision.scale();

    Ok(Position {
        x: quantized.x as f32 / scale,
        y: quantized.y as f32 / scale,
    })
}

#[derive(Component, Clone, Copy)]
struct Position {
    x: f32,
    y: f32,
}

#[derive(Component, Serialize, Deserialize, Default, Clone, Copy)]
enum Precision {
    #[default]
    OneDigit,
    TwoDigits,
}

impl Precision {
    fn scale(self) -> f32 {
        match self {
            Self::OneDigit => 10.0,
            Self::TwoDigits => 100.0,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct QuantizedPosition {
    precision: Precision,
    x: i16,
    y: i16,
}

fn quantize(value: f32, scale: f32) -> i16 {
    (value * scale)
        .round()
        .clamp(i16::MIN as f32, i16::MAX as f32) as i16
}
```
*/
pub trait EntityStorageCtx {
    /// Returns the entity associated with this context.
    fn entity(&self) -> Entity;

    /// Returns a reference to the replication storage resource.
    fn storage(&self) -> &ReplicationStorage;

    /// Returns a mutable reference to the replication storage resource.
    fn storage_mut(&mut self) -> &mut ReplicationStorage;

    /// Returns the value of type `T` from the entity storage.
    ///
    /// Initializes the value with `f` if it is not present yet.
    fn get_or_init<T: Send + Sync + 'static>(&mut self, f: impl FnOnce() -> T) -> &mut T {
        let entity = self.entity();
        self.storage_mut().get_or_init(entity, f)
    }

    /// Returns the value of type `T` from the entity storage.
    ///
    /// Inserts its default value if it is not present yet.
    fn get_or_default<T: Send + Sync + Default + 'static>(&mut self) -> &mut T {
        let entity = self.entity();
        self.storage_mut().get_or_default(entity)
    }

    /// Inserts `value` into the entity storage.
    ///
    /// Returns the previous value of the same type, if one was present.
    fn insert<T: Send + Sync + 'static>(&mut self, value: T) -> Option<T> {
        let entity = self.entity();
        self.storage_mut().insert(entity, value)
    }

    /// Returns a reference to the value of type `T` from the entity storage.
    fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        let entity = self.entity();
        self.storage().get(entity)
    }

    /// Returns a mutable reference to the value of type `T` from the entity storage.
    fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        let entity = self.entity();
        self.storage_mut().get_mut(entity)
    }

    /// Removes and returns the value of type `T` from the entity storage.
    fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        let entity = self.entity();
        self.storage_mut().remove(entity)
    }
}

/**
Storage for arbitrary state accessible from custom serialization/deserialization functions.

Can also be accessed outside those functions as a [`Resource`] to pass state to and
from them.

Values are keyed by their concrete type.

This resource won't be available to observers or hooks that run while receiving replication,
because it's temporarily removed from the world to make it accessible in
[`WriteCtx`](crate::shared::replication::registry::ctx::WriteCtx).

# Examples

Changing compression at runtime:

```
# use bevy::state::app::StatesPlugin;
use bevy::prelude::*;
use bevy_replicon::{
    bytes::Bytes,
    postcard_utils,
    prelude::*,
    shared::replication::registry::{
        ctx::{SerializeCtx, WriteCtx},
        rule_fns::RuleFns,
    },
};
use bytes::Buf;
use serde::{Deserialize, Serialize};

# let mut app = App::new();
# app.add_plugins((StatesPlugin, RepliconPlugins));
app.add_observer(store_compression_algorithm)
    .replicate_with(RuleFns::new(serialize_big_component, deserialize_big_component));

fn store_compression_algorithm(
    compression: On<CompressionChange>,
    mut storage: ResMut<ReplicationStorage>,
) {
    storage.global.insert(compression.algorithm);
}

fn serialize_big_component(
    ctx: &mut SerializeCtx,
    component: &BigComponent,
    message: &mut Vec<u8>,
) -> Result<()> {
    let algorithm = ctx
        .storage
        .global
        .get::<CompressionAlgorithm>()
        .copied()
        .unwrap_or_default();

    // Write the algorithm first, so the receiver knows how to decompress.
    postcard_utils::to_extend_mut(&algorithm, message)?;

    // Serialize as usual, but track size.
    let start = message.len();
    postcard_utils::to_extend_mut(component, message)?;
    let end = message.len();

    // Compress serialized slice using the selected algorithm.
    let compressed = compress(algorithm, &message[start..end]);

    // Replace serialized slice with compressed data prepended by its size.
    message.truncate(start);
    postcard_utils::to_extend_mut(&compressed.len(), message)?;
    message.extend(compressed);

    Ok(())
}

fn deserialize_big_component(
    _ctx: &mut WriteCtx,
    message: &mut Bytes,
) -> Result<BigComponent> {
    // Read the algorithm used by the sender.
    let algorithm = postcard_utils::from_buf(message)?;

    // Read size to know how much data is encoded.
    let size = postcard_utils::from_buf(message)?;

    // Apply decompression and advance the reading cursor.
    let decompressed = decompress(algorithm, &message[..size]);
    message.advance(size);

    let component = postcard::from_bytes(&decompressed)?;
    Ok(component)
}

#[derive(Component, Deserialize, Serialize)]
struct BigComponent(Vec<u64>);

#[derive(Event)]
struct CompressionChange {
    algorithm: CompressionAlgorithm,
}

#[derive(Serialize, Deserialize, Default, Clone, Copy)]
enum CompressionAlgorithm {
    #[default]
    None,
    Lz4,
    Zstd,
}

# fn compress(_algorithm: CompressionAlgorithm, data: &[u8]) -> Vec<u8> { unimplemented!() }
# fn decompress(_algorithm: CompressionAlgorithm, data: &[u8]) -> Vec<u8> { unimplemented!() }
```
*/
#[derive(Resource, Default)]
pub struct ReplicationStorage {
    /// Storage for data associated with networked entities.
    ///
    /// Use [`EntityStorageCtx`] helper methods when working from a context
    /// that already knows the current entity.
    ///
    /// Entries are removed automatically when
    /// [`Replicated`](crate::prelude::Replicated) or [`Remote`](crate::prelude::Remote)
    /// is removed from the associated entity.
    pub entities: EntityHashMap<TypeMap>,

    /// Storage for data not tied to a specific entity.
    pub global: TypeMap,
}

impl ReplicationStorage {
    /// Returns the value of type `T` from the entity storage.
    ///
    /// Initializes the value with `f` if it is not present yet.
    pub fn get_or_init<T: Send + Sync + 'static>(
        &mut self,
        entity: Entity,
        f: impl FnOnce() -> T,
    ) -> &mut T {
        self.entities.entry(entity).or_default().get_or_init(f)
    }

    /// Returns the value of type `T` from the entity storage.
    ///
    /// Inserts its default value if it is not present yet.
    pub fn get_or_default<T: Send + Sync + Default + 'static>(&mut self, entity: Entity) -> &mut T {
        self.get_or_init(entity, T::default)
    }

    /// Inserts `value` into the entity storage.
    ///
    /// Returns the previous value of the same type, if one was present.
    pub fn insert<T: Send + Sync + 'static>(&mut self, entity: Entity, value: T) -> Option<T> {
        self.entities.entry(entity).or_default().insert(value)
    }

    /// Returns a reference to the value of type `T` from the entity storage.
    pub fn get<T: Send + Sync + 'static>(&self, entity: Entity) -> Option<&T> {
        self.entities.get(&entity)?.get()
    }

    /// Returns a mutable reference to the value of type `T` from the entity storage.
    pub fn get_mut<T: Send + Sync + 'static>(&mut self, entity: Entity) -> Option<&mut T> {
        self.entities.get_mut(&entity)?.get_mut()
    }

    /// Removes and returns the value of type `T` from the entity storage.
    pub fn remove<T: Send + Sync + 'static>(&mut self, entity: Entity) -> Option<T> {
        self.entities.get_mut(&entity)?.remove()
    }
}

/// Type-indexed map used by [`ReplicationStorage`].
///
/// Stores values by their concrete type. Can contain only one value of each type.
#[derive(Default)]
pub struct TypeMap {
    values: TypeIdMap<Box<dyn Any + Send + Sync>>,
}

impl TypeMap {
    /// Returns the value of type `T`.
    ///
    /// Initializes the value with `f` if it is not present yet.
    pub fn get_or_init<T: Send + Sync + 'static>(&mut self, f: impl FnOnce() -> T) -> &mut T {
        self.values
            .entry_type::<T>()
            .or_insert_with(|| Box::new(f()))
            .downcast_mut::<T>()
            .unwrap()
    }

    /// Returns the value of type `T`.
    ///
    /// Inserts its default value if it is not present yet.
    pub fn get_or_default<T: Send + Sync + Default + 'static>(&mut self) -> &mut T {
        self.get_or_init(T::default)
    }

    /// Inserts `value` into the map.
    ///
    /// Returns the previous value of the same type, if one was present.
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) -> Option<T> {
        let value = self
            .values
            .insert_type::<T>(Box::new(value))?
            .downcast::<T>();

        // SAFETY: values are keyed by their type.
        Some(unsafe { *value.unwrap_unchecked() })
    }

    /// Returns a reference to the value of type `T`.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        let value = self.values.get_type::<T>()?.downcast_ref::<T>();

        // SAFETY: values are keyed by their type.
        Some(unsafe { value.unwrap_unchecked() })
    }

    /// Returns a mutable reference to the value of type `T`.
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        let value = self.values.get_type_mut::<T>()?.downcast_mut::<T>();

        // SAFETY: values are keyed by their type.
        Some(unsafe { value.unwrap_unchecked() })
    }

    /// Removes and returns the value of type `T`.
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        let value = self.values.remove_type::<T>()?.downcast::<T>();

        // SAFETY: values are keyed by their type.
        Some(unsafe { *value.unwrap_unchecked() })
    }

    /// Removes all stored values.
    pub fn clear(&mut self) {
        self.values.clear();
    }
}
