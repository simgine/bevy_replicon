//! Custom serialization for entity to pack it more efficiently.

use bevy::prelude::*;
use bytes::Bytes;

use crate::postcard_utils;

/// Deserializes `entity` from compressed index and generation.
///
/// For details see [`serialize_entity`].
pub fn deserialize_entity(message: &mut Bytes) -> Result<Entity> {
    let flagged_index: u64 = postcard_utils::from_buf(message)?;
    let has_generation = (flagged_index & 1) > 0;
    let generation = if has_generation {
        postcard_utils::from_buf::<u32, _>(message)? + 1
    } else {
        1u32
    };

    let bits = ((generation as u64) << 32) | (flagged_index >> 1);

    Ok(Entity::from_bits(bits))
}

/// Serializes `entity` by writing its index and generation as separate varints.
///
/// The index is first prepended with a bit flag to indicate if the generation
/// is serialized or not. It is not serialized if <= 1; note that generations are [`NonZeroU32`](core::num::NonZeroU32)
/// and a value of zero is used in [`Option<Entity>`] to signify [`None`], so generation 1 is the first
/// generation.
///
/// See also [`deserialize_entity`].
pub fn serialize_entity(message: &mut Vec<u8>, entity: Entity) -> Result<()> {
    let mut flagged_index = (entity.index() as u64) << 1;
    let flag = entity.generation() > 1;
    flagged_index |= flag as u64;

    postcard_utils::to_extend_mut(&flagged_index, message)?;
    if flag {
        let generation = entity.generation() - 1;
        postcard_utils::to_extend_mut(&generation, message)?;
    }

    Ok(())
}
