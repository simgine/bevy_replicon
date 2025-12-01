use core::ops::Range;

use bevy::{ecs::component::ComponentId, prelude::*, ptr::Ptr};

use crate::{
    postcard_utils,
    prelude::*,
    shared::replication::registry::{FnsId, ctx::SerializeCtx, serde_fns::SerdeFns},
};

/// Single continuous buffer that stores serialized data for messages.
///
/// See [`Updates`](super::updates::Updates) and
/// [`MutateMessage`](super::mutations::MutateMessage).
#[derive(Resource, Deref, DerefMut, Default)]
pub(crate) struct SerializedData(Vec<u8>);

/// Custom serialization for replication messages.
pub(crate) trait MessageWrite {
    /// Writes data for replication messages and returns a range that points to it.
    fn write(&self, serialized: &mut SerializedData) -> Result<Range<usize>>;

    /// Like [`Self::write`], but returns the value from the range if it's [`Some`].
    fn write_cached(
        &self,
        serialized: &mut SerializedData,
        cached_range: &mut Option<Range<usize>>,
    ) -> Result<Range<usize>> {
        if let Some(range) = cached_range.clone() {
            return Ok(range);
        }

        let range = self.write(serialized)?;
        *cached_range = Some(range.clone());

        Ok(range)
    }
}

pub(crate) struct WritableComponent<'a> {
    pub(crate) fns: SerdeFns<'a>,
    pub(crate) ptr: Ptr<'a>,
    pub(crate) fns_id: FnsId,
    pub(crate) ctx: SerializeCtx<'a>,
}

impl<'a> WritableComponent<'a> {
    /// Creates a new instance for component data that can be written into a replication message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `fns` and `ptr` was created for the same type.
    pub(crate) unsafe fn new(
        fns: SerdeFns<'a>,
        ptr: Ptr<'a>,
        fns_id: FnsId,
        component_id: ComponentId,
        server_tick: RepliconTick,
        type_registry: &'a AppTypeRegistry,
    ) -> Self {
        Self {
            fns,
            ptr,
            fns_id,
            ctx: SerializeCtx {
                component_id,
                server_tick,
                type_registry,
            },
        }
    }
}

impl MessageWrite for WritableComponent<'_> {
    fn write(&self, serialized: &mut SerializedData) -> Result<Range<usize>> {
        let start = serialized.len();

        postcard_utils::to_extend_mut(&self.fns_id, &mut serialized.0)?;
        // SAFETY: `fns` and `ptr` were created for the same component type.
        unsafe { self.fns.serialize(&self.ctx, self.ptr, &mut serialized.0)? };

        let end = serialized.len();

        Ok(start..end)
    }
}

pub(crate) struct EntityMapping {
    pub(crate) entity: Entity,
    pub(crate) hash: u64,
}

impl MessageWrite for EntityMapping {
    fn write(&self, serialized: &mut SerializedData) -> Result<Range<usize>> {
        let start = serialized.len();

        self.entity.write(serialized)?;
        serialized.extend(self.hash.to_le_bytes()); // Use fixint encoding because it's more efficient for hashes.

        let end = serialized.len();

        Ok(start..end)
    }
}

impl MessageWrite for Entity {
    fn write(&self, serialized: &mut SerializedData) -> Result<Range<usize>> {
        let start = serialized.len();

        postcard_utils::entity_to_extend_mut(self, &mut serialized.0)?;

        let end = serialized.len();

        Ok(start..end)
    }
}

impl MessageWrite for FnsId {
    fn write(&self, serialized: &mut SerializedData) -> Result<Range<usize>> {
        let start = serialized.len();

        postcard_utils::to_extend_mut(self, &mut serialized.0)?;

        let end = serialized.len();

        Ok(start..end)
    }
}

impl MessageWrite for RepliconTick {
    fn write(&self, serialized: &mut SerializedData) -> Result<Range<usize>> {
        let start = serialized.len();

        postcard_utils::to_extend_mut(self, &mut serialized.0)?;

        let end = serialized.len();

        Ok(start..end)
    }
}
