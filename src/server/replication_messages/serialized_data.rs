use core::ops::Range;

use bevy::{prelude::*, ptr::Ptr};

use crate::{
    postcard_utils,
    prelude::*,
    shared::replication::registry::{FnsId, ctx::SerializeCtx, serde_fns::SerdeFns},
};

/// Single continuous buffer that stores serialized data for messages.
///
/// See [`Updates`](super::updates::Updates) and
/// [`MutateMessage`](super::mutations::MutateMessage).
#[derive(Default, Deref, DerefMut)]
pub(crate) struct SerializedData(Vec<u8>);

impl SerializedData {
    pub(crate) fn write_mapping(&mut self, entity: Entity, hash: u64) -> Result<Range<usize>> {
        let start = self.len();

        self.write_entity(entity)?;
        self.extend(hash.to_le_bytes()); // Use fixint encoding because it's more efficient for hashes.

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_fns_id(&mut self, fns_id: FnsId) -> Result<Range<usize>> {
        let start = self.len();

        postcard_utils::to_extend_mut(&fns_id, &mut self.0)?;

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_component(
        &mut self,
        fns: &SerdeFns,
        ctx: &SerializeCtx,
        fns_id: FnsId,
        ptr: Ptr,
    ) -> Result<Range<usize>> {
        let start = self.len();

        postcard_utils::to_extend_mut(&fns_id, &mut self.0)?;
        // SAFETY: `fns` and `ptr` were created for the same component type.
        unsafe { fns.serialize(ctx, ptr, &mut self.0)? };

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_entity(&mut self, entity: Entity) -> Result<Range<usize>> {
        let start = self.len();

        postcard_utils::entity_to_extend_mut(&entity, &mut self.0)?;

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_tick(&mut self, tick: RepliconTick) -> Result<Range<usize>> {
        let start = self.len();

        postcard_utils::to_extend_mut(&tick, &mut self.0)?;

        let end = self.len();

        Ok(start..end)
    }
}
