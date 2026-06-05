use core::ops::Range;

use bevy::{ecs::component::ComponentId, prelude::*, ptr::Ptr};

use crate::{
    postcard_utils,
    prelude::*,
    shared::replication::{
        diff::{DiffFns, PatchIndex},
        registry::{FnsId, ctx::SerializeCtx, serde_fns::SerdeFns},
    },
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
    diff: Option<WritableDiffComponent<'a>>,
}

struct WritableDiffComponent<'a> {
    fns: DiffFns,
    log: Ptr<'a>,
}

pub(crate) struct WrittenComponent {
    pub(crate) range: Range<usize>,
    pub(crate) patch_cursor: Option<PatchIndex>,
}

impl<'a> WritableComponent<'a> {
    /// Creates a new instance for component data that can be written into a replication message.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `fns` and `ptr` was created for the same type.
    /// If `diff` is set, its log pointer must point to the matching `DiffLog` for
    /// that same component type.
    pub(crate) unsafe fn new(
        fns: SerdeFns<'a>,
        ptr: Ptr<'a>,
        diff: Option<(DiffFns, Ptr<'a>)>,
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
            diff: diff.map(|(fns, log)| WritableDiffComponent { fns, log }),
        }
    }

    /// Writes component data for a mutation message.
    ///
    /// Non-diff components can reuse the same serialized range for all clients.
    /// Diff components depend on each client's last ACKed patch cursor, so they
    /// are serialized per client and return the patch cursor that should advance
    /// when the mutation message is ACKed.
    pub(crate) fn write_mutation(
        &self,
        serialized: &mut SerializedData,
        cached_range: &mut Option<Range<usize>>,
        acked_patch_cursor: PatchIndex,
    ) -> Result<WrittenComponent> {
        if self.diff.is_some() {
            self.write_uncached(serialized, Some(acked_patch_cursor))
        } else {
            Ok(WrittenComponent {
                range: self.write_cached(serialized, cached_range)?,
                patch_cursor: None,
            })
        }
    }

    fn write_uncached(
        &self,
        serialized: &mut SerializedData,
        acked_patch_cursor: Option<PatchIndex>,
    ) -> Result<WrittenComponent> {
        let start = serialized.len();

        postcard_utils::to_extend_mut(&self.fns_id, &mut serialized.0)?;
        let patch_cursor = if let Some(diff) = &self.diff {
            // SAFETY: `diff`, `ptr` and `log` were created for the same component type.
            Some(unsafe {
                diff.fns.serialize_mutation(
                    &self.ctx,
                    self.ptr,
                    diff.log,
                    acked_patch_cursor,
                    &mut serialized.0,
                )?
            })
        } else {
            // SAFETY: `fns` and `ptr` were created for the same component type.
            unsafe { self.fns.serialize(&self.ctx, self.ptr, &mut serialized.0)? };
            None
        };

        let end = serialized.len();

        Ok(WrittenComponent {
            range: start..end,
            patch_cursor,
        })
    }
}

impl MessageWrite for WritableComponent<'_> {
    fn write(&self, serialized: &mut SerializedData) -> Result<Range<usize>> {
        Ok(self.write_uncached(serialized, None)?.range)
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
