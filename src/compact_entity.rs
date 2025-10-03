//! Custom functions [`serde`] to pack [`Entity`] more efficiently.
//!
//! # Examples
//!
//! Customize serialization using `with` attribute.
//!
//! ```
//! use serde::{Deserialize, Serialize};
//! use bevy::prelude::*;
//!
//! #[derive(Serialize, Deserialize)]
//! struct Enemy {
//!     #[serde(with = "bevy_replicon::compact_entity")]
//!     entity: Entity
//! }
//! ```

use core::fmt::{self, Formatter};

use bevy::{
    ecs::entity::{EntityGeneration, EntityRow},
    prelude::*,
};
use serde::{
    Deserializer, Serialize, Serializer,
    de::{self, SeqAccess, Visitor},
};

/// Serializes an entity by writing its index and generation as separate numbers.
///
/// This reduces the space required when using serializers with varint encoding.
///
/// Since the index can never be [`u32::MAX`], we reuse that extra niche to indicate
/// whether the generation isn't [`EntityGeneration::FIRST`]. If it doesn't,
/// the generation is skipped during serialization.
///
/// See also [`deserialize`] and [`postcard_utils::entity_to_extend_mut`](crate::postcard_utils::entity_to_extend_mut).
pub fn serialize<S: Serializer>(entity: &Entity, serializer: S) -> Result<S::Ok, S::Error> {
    let mut index = entity.index() << 1;
    let has_generation = entity.generation() != EntityGeneration::FIRST;
    index |= has_generation as u32;

    if has_generation {
        let generation = entity.generation().to_bits();
        (index, generation).serialize(serializer)
    } else {
        index.serialize(serializer)
    }
}

/// Deserializes an entity from compressed index and generation.
///
/// See also [`serialize`] and [`postcard_utils::entity_from_buf`](crate::postcard_utils::entity_from_buf).
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Entity, D::Error> {
    deserializer.deserialize_tuple(2, EntityVisitor)
}

struct EntityVisitor;

impl<'de> Visitor<'de> for EntityVisitor {
    type Value = Entity;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str("entity index with optional generation")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let index: u32 = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        let has_generation = (index & 1) != 0;

        let generation = if has_generation {
            seq.next_element()?
                .ok_or_else(|| de::Error::invalid_length(1, &self))?
        } else {
            0
        };

        // SAFETY: `index` is non-max after shift.
        let row = unsafe { EntityRow::from_raw_u32(index >> 1).unwrap_unchecked() };
        let generation = EntityGeneration::from_bits(generation);

        Ok(Entity::from_row_and_generation(row, generation))
    }
}
