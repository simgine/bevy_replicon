use core::ops::Range;

use bevy::{prelude::*, ptr::Ptr};

use crate::{
    postcard_utils,
    prelude::*,
    shared::replication::registry::{FnsId, ctx::SerializeCtx, serde_fns::SerdeFns},
};

/// Single continuous buffer that stores serialized data for messages.
///
/// Values written into the buffer are referenced by byte ranges instead of being
/// copied into each message. This allows multiple messages to point to the same
/// serialized data.
///
/// See [`Updates`](super::updates::Updates) and
/// [`MutateMessage`](super::mutations::MutateMessage).
#[derive(Resource, Deref, DerefMut, Default)]
pub(crate) struct SerializedData(Vec<u8>);

impl SerializedData {
    pub(crate) fn write_cached_mapping(
        &mut self,
        cached_range: &mut Option<Range<usize>>,
        entity: Entity,
        hash: u64,
    ) -> Result<Range<usize>> {
        self.write_cached(cached_range, |serialized| {
            serialized.write_mapping(entity, hash)
        })
    }

    pub(crate) fn write_cached_component(
        &mut self,
        ctx: &mut SerializeCtx,
        cached_range: &mut Option<Range<usize>>,
        component: &mut ErasedComponent,
    ) -> Result<Range<usize>> {
        self.write_cached(cached_range, |serialized| {
            serialized.write_component(ctx, component)
        })
    }

    pub(crate) fn write_cached_entity(
        &mut self,
        cached_range: &mut Option<Range<usize>>,
        entity: Entity,
    ) -> Result<Range<usize>> {
        self.write_cached(cached_range, |serialized| serialized.write_entity(entity))
    }

    pub(crate) fn write_cached_fns_id(
        &mut self,
        cached_range: &mut Option<Range<usize>>,
        fns_id: FnsId,
    ) -> Result<Range<usize>> {
        self.write_cached(cached_range, |serialized| serialized.write_fns_id(fns_id))
    }

    pub(crate) fn write_cached_tick(
        &mut self,
        cached_range: &mut Option<Range<usize>>,
        tick: RepliconTick,
    ) -> Result<Range<usize>> {
        self.write_cached(cached_range, |serialized| serialized.write_tick(tick))
    }

    /// Returns a previously written range, or writes the data and caches its range.
    ///
    /// Used when several replication messages need to reference the same
    /// serialized value.
    fn write_cached(
        &mut self,
        cached_range: &mut Option<Range<usize>>,
        write: impl FnOnce(&mut Self) -> Result<Range<usize>>,
    ) -> Result<Range<usize>> {
        if let Some(range) = cached_range.clone() {
            return Ok(range);
        }

        let range = write(self)?;
        *cached_range = Some(range.clone());

        Ok(range)
    }

    fn write_component(
        &mut self,
        ctx: &mut SerializeCtx,
        component: &mut ErasedComponent,
    ) -> Result<Range<usize>> {
        self.write_with(|bytes| {
            postcard_utils::to_extend_mut(&component.fns_id, bytes)?;

            // SAFETY: `fns` and `ptr` were created for the same component type.
            unsafe {
                component.fns.serialize(ctx, component.ptr, bytes)?;
            }

            Ok(())
        })
    }

    fn write_mapping(&mut self, entity: Entity, hash: u64) -> Result<Range<usize>> {
        self.write_with(|bytes| {
            postcard_utils::entity_to_extend_mut(&entity, bytes)?;
            bytes.extend(hash.to_le_bytes()); // Use fixint encoding because it's more efficient for hashes.
            Ok(())
        })
    }

    pub(crate) fn write_entity(&mut self, entity: Entity) -> Result<Range<usize>> {
        self.write_with(|bytes| {
            postcard_utils::entity_to_extend_mut(&entity, bytes)?;
            Ok(())
        })
    }

    pub(crate) fn write_fns_id(&mut self, fns_id: FnsId) -> Result<Range<usize>> {
        self.write_with(|bytes| {
            postcard_utils::to_extend_mut(&fns_id, bytes)?;
            Ok(())
        })
    }

    fn write_tick(&mut self, tick: RepliconTick) -> Result<Range<usize>> {
        self.write_with(|bytes| {
            postcard_utils::to_extend_mut(&tick, bytes)?;
            Ok(())
        })
    }

    /// Writes data for replication messages and returns a range that points to it.
    fn write_with(
        &mut self,
        write: impl FnOnce(&mut Vec<u8>) -> Result<()>,
    ) -> Result<Range<usize>> {
        let start = self.len();

        write(&mut self.0)?;

        let end = self.len();
        Ok(start..end)
    }
}

/// Wraps a component pointer and its associated functions.
///
/// Allows moving the unsafe precondition to construction.
pub(crate) struct ErasedComponent<'a> {
    fns: SerdeFns<'a>,
    ptr: Ptr<'a>,
    fns_id: FnsId,
}

impl<'a> ErasedComponent<'a> {
    /// Creates a new instance for component data that can be written into a replication message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `fns` and `ptr` was created for the same type.
    pub(crate) unsafe fn new(fns: SerdeFns<'a>, ptr: Ptr<'a>, fns_id: FnsId) -> Self {
        Self { fns, ptr, fns_id }
    }
}
