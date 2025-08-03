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

use bevy::prelude::*;
use serde::{
    Deserializer, Serialize, Serializer,
    de::{self, SeqAccess, Visitor},
};

/// Serializes an entity by writing its index and generation as separate numbers.
///
/// This reduces the space used for serializers with varint encoding.
///
/// The index is first prepended with a bit flag to indicate if the generation
/// is serialized or not. It is not serialized if <= 1; note that generations are [`NonZeroU32`](core::num::NonZeroU32)
/// and a value of zero is used in [`Option<Entity>`] to signify [`None`], so generation 1 is the first
/// generation.
///
/// See also [`deserialize`] and [`postcard_utils::entity_to_extend_mut`](crate::postcard_utils::entity_to_extend_mut).
pub fn serialize<S: Serializer>(entity: &Entity, serializer: S) -> Result<S::Ok, S::Error> {
    let mut flagged_index = (entity.index() as u64) << 1;
    let flag = entity.generation() > 1;
    flagged_index |= flag as u64;

    if flag {
        let generation = entity.generation() - 1;
        (flagged_index, generation).serialize(serializer)
    } else {
        flagged_index.serialize(serializer)
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
        let flagged_index: u64 = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        let has_generation = (flagged_index & 1) != 0;

        let generation = if has_generation {
            let generation: u32 = seq
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(1, &self))?;
            generation as u64 + 1
        } else {
            1
        };

        let bits = (flagged_index >> 1) | (generation << 32);
        Ok(Entity::from_bits(bits))
    }
}
