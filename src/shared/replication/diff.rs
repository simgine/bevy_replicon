//! Patch-based diff replication for components.
//!
//! See [`Diffable`] for the main user-facing API and example.

use alloc::{
    collections::{BTreeMap, VecDeque},
    format,
    vec::Vec,
};
use core::marker::PhantomData;

use bevy::{
    ecs::{
        component::{ComponentId, Mutable},
        system::EntityCommands,
        world::{EntityMut, EntityWorldMut},
    },
    prelude::*,
    ptr::Ptr,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize, de::DeserializeOwned, ser::SerializeSeq};

use crate::{
    postcard_utils,
    shared::replication::{
        deferred_entity::DeferredEntity,
        registry::{
            ctx::{RemoveCtx, SerializeCtx, WriteCtx},
            rule_fns::DeserializeFn,
            rule_fns::RuleFns,
        },
    },
};

/// Monotonic index assigned to a diff patch.
pub type PatchIndex = u64;

/// Component whose mutations can be represented as an ordered log of patches.
///
/// Diff replication is useful when a component is large, but most changes can be
/// represented by a small semantic patch. A common example is a component that stores
/// a growing [`VecDeque`](std::collections::VecDeque) of points for a trail/path.
/// Sending the full queue after every push can become expensive; sending a patch
/// like `PushBack(point)` or `PopFront(count)` only transmits the part that changed.
///
/// The component remains the authoritative state. The user provides a patch type and
/// implements [`Self::apply_patch`] to describe how each patch changes the component.
/// When the server mutates the component through [`DiffEntityExt::apply_patch`],
/// Replicon applies the patch locally and records it in a [`DiffLog`]. For each
/// client, the server sends either the patches after that client's latest
/// acknowledged patch cursor, or a full snapshot if the needed patches are no longer
/// retained. On the receiver, patches are deduplicated, buffered until they can be
/// applied in order, and then applied to the local component. Components can override
/// [`Self::HISTORY_LEN`] to tune how many patches are kept before snapshot fallback
/// becomes necessary.
///
/// Direct component mutations are still supported for correctness, but they are not
/// recorded as patches and will be sent as a snapshot fallback.
///
/// # Example
///
/// ```rust
/// use std::collections::VecDeque;
///
/// use bevy::{prelude::*, state::app::StatesPlugin};
/// use bevy_replicon::prelude::*;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Copy, Deserialize, Serialize)]
/// struct Point {
///     x: f32,
///     y: f32,
/// }
///
/// #[derive(Component, Deserialize, Serialize)]
/// struct Trail(VecDeque<Point>);
///
/// #[derive(Clone, Deserialize, Serialize)]
/// enum TrailPatch {
///     PushBack(Point),
///     PopFront(usize),
/// }
///
/// impl Diffable for Trail {
///     type Patch = TrailPatch;
///     const HISTORY_LEN: usize = 256;
///
///     fn apply_patch(&mut self, patch: &Self::Patch) -> Result<()> {
///         match *patch {
///             TrailPatch::PushBack(point) => self.0.push_back(point),
///             TrailPatch::PopFront(count) => {
///                 for _ in 0..count {
///                     self.0.pop_front();
///                 }
///             }
///         }
///
///         Ok(())
///     }
/// }
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
///     .replicate_diff::<Trail>()
///     .finish();
///
/// let entity = app
///     .world_mut()
///     .spawn((Replicated, Trail(VecDeque::new())))
///     .id();
///
/// let point = Point { x: 1.0, y: 2.0 };
/// let _ = app
///     .world_mut()
///     .entity_mut(entity)
///     .apply_patch::<Trail>(TrailPatch::PushBack(point));
/// ```
pub trait Diffable: Component<Mutability = Mutable> + Serialize + DeserializeOwned + Sized {
    /// Patch that transforms this component from one state to the next.
    type Patch: Clone + Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Maximum number of patches retained for diff serialization.
    ///
    /// If a client acknowledges a patch older than the retained range,
    /// Replicon will fall back to sending a full component snapshot.
    const HISTORY_LEN: usize = 64;

    /// Applies a patch to the component state.
    fn apply_patch(&mut self, patch: &Self::Patch) -> Result<()>;
}

/// Patch log associated with a [`Diffable`].
///
/// This component is registered as a required component for diff components.
/// It is not replicated directly.
#[derive(Component, Debug)]
pub struct DiffLog<C: Diffable> {
    last_index: PatchIndex,
    patches: VecDeque<C::Patch>,
    _marker: PhantomData<fn() -> C>,
}

impl<C: Diffable> DiffLog<C> {
    /// Records a patch and returns the assigned patch index.
    pub fn record(&mut self, patch: C::Patch) -> PatchIndex {
        self.last_index += 1;
        let index = self.last_index;
        self.patches.push_back(patch);
        self.prune_to_limit();
        index
    }

    /// Returns the latest patch index.
    pub fn current_cursor(&self) -> PatchIndex {
        self.last_index
    }

    /// Returns all retained patches after `cursor`.
    ///
    /// Returns `None` if patches needed to continue from `cursor` were already
    /// pruned and the sender must fall back to a snapshot.
    pub(crate) fn patches_after(&self, cursor: PatchIndex) -> Option<PatchSlice<'_, C::Patch>> {
        let first_index = self.first_index();
        if cursor < first_index - 1 {
            return None;
        }

        let start = if cursor >= self.last_index {
            self.patches.len()
        } else {
            (cursor + 1 - first_index) as usize
        };
        Some(PatchSlice {
            first_index: first_index + start as PatchIndex,
            patches: &self.patches,
            start,
        })
    }

    fn first_index(&self) -> PatchIndex {
        debug_assert!(self.patches.len() as PatchIndex <= self.last_index);
        self.last_index - self.patches.len() as PatchIndex + 1
    }

    fn prune_to_limit(&mut self) {
        let excess = self.patches.len().saturating_sub(C::HISTORY_LEN);
        if excess > 0 {
            self.patches.drain(..excess);
        }
    }
}

pub(crate) struct PatchSlice<'a, Patch> {
    first_index: PatchIndex,
    patches: &'a VecDeque<Patch>,
    start: usize,
}

impl<Patch> PatchSlice<'_, Patch> {
    fn is_empty(&self) -> bool {
        self.start == self.patches.len()
    }

    fn first_index(&self) -> PatchIndex {
        self.first_index
    }
}

impl<Patch: Serialize> Serialize for PatchSlice<'_, Patch> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> core::result::Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.patches.len() - self.start))?;
        for patch in self.patches.iter().skip(self.start) {
            seq.serialize_element(patch)?;
        }
        seq.end()
    }
}

impl<C: Diffable> Default for DiffLog<C> {
    fn default() -> Self {
        Self {
            last_index: 0,
            patches: Default::default(),
            _marker: PhantomData,
        }
    }
}

/// Receiver-side state for applying diff patches exactly once and in order.
#[derive(Component, Debug)]
pub struct DiffReceiver<C: Diffable> {
    last_applied: PatchIndex,
    pending: BTreeMap<PatchIndex, C::Patch>,
}

impl<C: Diffable> DiffReceiver<C> {
    pub fn new(cursor: PatchIndex) -> Self {
        Self {
            last_applied: cursor,
            pending: Default::default(),
        }
    }

    /// Queues newly received patches and returns patches that can be applied now.
    ///
    /// Patches must be applied sequentially by [`PatchIndex`]. If a patch arrives
    /// ahead of a missing predecessor, it stays pending until the missing patch is
    /// received. Duplicate or already-applied patches are ignored.
    pub fn queue_and_take_ready(
        &mut self,
        first_patch_index: PatchIndex,
        patches: Vec<C::Patch>,
    ) -> Vec<C::Patch> {
        for (offset, patch) in patches.into_iter().enumerate() {
            let index = first_patch_index + offset as PatchIndex;
            if index > self.last_applied {
                self.pending.entry(index).or_insert(patch);
            }
        }

        let mut ready = Vec::new();
        while let Some(patch) = self.pending.remove(&(self.last_applied + 1)) {
            self.last_applied += 1;
            ready.push(patch);
        }

        ready
    }
}

impl<C: Diffable> Default for DiffReceiver<C> {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Wire format for diff replicated components.
#[derive(Deserialize, Serialize)]
pub enum DiffWire<C, Patch> {
    Snapshot {
        cursor: PatchIndex,
        value: C,
    },
    Patches {
        first_patch_index: PatchIndex,
        patches: Vec<Patch>,
    },
}

#[derive(Serialize)]
enum DiffWireRef<'a, C, Patch> {
    Snapshot {
        cursor: PatchIndex,
        value: &'a C,
    },
    Patches {
        first_patch_index: PatchIndex,
        patches: PatchSlice<'a, Patch>,
    },
}

/// Extension trait for recording diff patches on an entity.
pub trait DiffEntityExt {
    /// Applies `patch` to component `C` and records it in the entity's [`DiffLog`].
    ///
    /// For [`EntityCommands`], this queues the patch application. Missing components
    /// or patch application errors are reported when commands are applied.
    fn apply_patch<C: Diffable>(&mut self, patch: C::Patch) -> Result<()>;
}

impl DiffEntityExt for EntityWorldMut<'_> {
    fn apply_patch<C: Diffable>(&mut self, patch: C::Patch) -> Result<()> {
        let entity = self.id();
        {
            let mut component = self.get_mut::<C>().ok_or_else(|| {
                format!("entity `{entity}` is missing `{}`", ShortName::of::<C>())
            })?;
            component.apply_patch(&patch)?;
        }

        let mut log = self.get_mut::<DiffLog<C>>().ok_or_else(|| {
            format!(
                "entity `{}` is missing `{}`; register `{}` with `replicate_diff`",
                entity,
                ShortName::of::<DiffLog<C>>(),
                ShortName::of::<C>(),
            )
        })?;
        log.record(patch);

        Ok(())
    }
}

impl DiffEntityExt for EntityMut<'_> {
    fn apply_patch<C: Diffable>(&mut self, patch: C::Patch) -> Result<()> {
        let entity = self.id();
        {
            let mut component = self.get_mut::<C>().ok_or_else(|| {
                format!("entity `{entity}` is missing `{}`", ShortName::of::<C>())
            })?;
            component.apply_patch(&patch)?;
        }

        let mut log = self.get_mut::<DiffLog<C>>().ok_or_else(|| {
            format!(
                "entity `{}` is missing `{}`; register `{}` with `replicate_diff`",
                entity,
                ShortName::of::<DiffLog<C>>(),
                ShortName::of::<C>(),
            )
        })?;
        log.record(patch);

        Ok(())
    }
}

impl DiffEntityExt for EntityCommands<'_> {
    fn apply_patch<C: Diffable>(&mut self, patch: C::Patch) -> Result<()> {
        self.queue(move |mut entity: EntityWorldMut| entity.apply_patch::<C>(patch));
        Ok(())
    }
}

/// Sender-side functions for diff replication.
///
/// Diff components still use [`RuleFns`](crate::shared::replication::registry::rule_fns::RuleFns)
/// for snapshot payloads and receive-side deserialization. `DiffFns` stores the
/// extra sender-only state needed to serialize patches: the `DiffLog<C>`
/// component ID and a type-erased serializer that can read both the component
/// and its log.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DiffFns {
    /// Component ID for `DiffLog<C>` associated with the diff component.
    pub(crate) log_component_id: ComponentId,
    serialize_mutation:
        unsafe fn(&SerializeCtx, Ptr, Ptr, Option<PatchIndex>, &mut Vec<u8>) -> Result<PatchIndex>,
}

impl DiffFns {
    pub(crate) fn new<C: Diffable>(world: &mut World) -> Self {
        Self {
            log_component_id: world.register_component::<DiffLog<C>>(),
            serialize_mutation: serialize_mutation::<C>,
        }
    }

    /// Serializes patches after `acked_cursor`, or a snapshot if required.
    ///
    /// If `acked_cursor` is [`None`], the receiver has no base component state
    /// and the payload is forced to be a snapshot.
    ///
    /// # Safety
    ///
    /// `component` must point to `C`, and `log` must point to `DiffLog<C>`.
    pub(crate) unsafe fn serialize_mutation(
        &self,
        ctx: &SerializeCtx,
        component: Ptr,
        log: Ptr,
        acked_cursor: Option<PatchIndex>,
        message: &mut Vec<u8>,
    ) -> Result<PatchIndex> {
        unsafe { (self.serialize_mutation)(ctx, component, log, acked_cursor, message) }
    }
}

unsafe fn serialize_mutation<C: Diffable>(
    _ctx: &SerializeCtx,
    component: Ptr,
    log: Ptr,
    acked_cursor: Option<PatchIndex>,
    message: &mut Vec<u8>,
) -> Result<PatchIndex> {
    let component = unsafe { component.deref::<C>() };
    let log = unsafe { log.deref::<DiffLog<C>>() };
    let cursor = log.current_cursor();

    match acked_cursor.and_then(|cursor| log.patches_after(cursor)) {
        Some(patches) if !patches.is_empty() => {
            let wire: DiffWireRef<'_, C, C::Patch> = DiffWireRef::Patches {
                first_patch_index: patches.first_index(),
                patches,
            };
            postcard_utils::to_extend_mut(&wire, message)?;
        }
        _ => {
            let wire: DiffWireRef<'_, C, C::Patch> = DiffWireRef::Snapshot {
                cursor,
                value: component,
            };
            postcard_utils::to_extend_mut(&wire, message)?;
        }
    }

    Ok(cursor)
}

/// Serializes a full snapshot when only the component is available.
///
/// The normal server path uses [`DiffFns::serialize_mutation`] because it can
/// access the component's [`DiffLog`]. This function is the [`RuleFns`] snapshot
/// serializer for generic paths that only receive `&C`.
pub(crate) fn serialize_snapshot_without_log<C: Diffable>(
    _ctx: &SerializeCtx,
    component: &C,
    message: &mut Vec<u8>,
) -> Result<()> {
    let wire: DiffWireRef<'_, C, C::Patch> = DiffWireRef::Snapshot {
        cursor: 0,
        value: component,
    };
    postcard_utils::to_extend_mut(&wire, message)?;
    Ok(())
}

/// Deserializes a diff snapshot payload into a component value.
///
/// Live replication uses [`write`] so it can handle both snapshots and patches.
/// This function exists for [`RuleFns`] paths that need to deserialize a
/// standalone `C`; patch payloads are rejected because they require receiver
/// history to apply.
pub(crate) fn deserialize_snapshot<C: Diffable>(
    _ctx: &mut WriteCtx,
    message: &mut Bytes,
) -> Result<C> {
    match postcard_utils::from_buf(message)? {
        DiffWire::<C, C::Patch>::Snapshot { value, .. } => Ok(value),
        DiffWire::<C, C::Patch>::Patches { .. } => Err(format!(
            "cannot deserialize diff patches into `{}`",
            ShortName::of::<C>()
        )
        .into()),
    }
}

/// Consumes a diff payload without applying it.
///
/// This is used for stale mutation messages when a receive marker requests
/// history for some components on the entity but not this component. In that
/// path Replicon still has to advance through every component payload in the
/// mutation message. The default consume implementation deserializes a `C`,
/// but diff mutation payloads may contain [`DiffWire::Patches`], which is
/// not a standalone component value. Parsing and dropping the full wire format
/// lets us skip both snapshots and patches safely.
pub(crate) fn consume<C: Diffable>(
    _deserialize: DeserializeFn<C>,
    _ctx: &mut WriteCtx,
    message: &mut Bytes,
) -> Result<()> {
    let _wire: DiffWire<C, C::Patch> = postcard_utils::from_buf(message)?;
    Ok(())
}

pub(crate) fn write<C: Diffable>(
    _ctx: &mut WriteCtx,
    _rule_fns: &RuleFns<C>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    // This is the live receive path for diff components. Snapshots replace or
    // insert the component and reset the receiver cursor; patches are queued and
    // applied only once all earlier patches have been applied.
    let wire: DiffWire<C, C::Patch> = postcard_utils::from_buf(message)?;

    match wire {
        DiffWire::Snapshot { cursor, value } => {
            if let Some(mut component) = entity.get_mut::<C>() {
                *component = value;
            } else {
                entity.insert(value);
            }
            entity.insert(DiffReceiver::<C>::new(cursor));
        }
        DiffWire::Patches {
            first_patch_index,
            patches,
        } => {
            let ready_patches = {
                let mut receiver = entity.get_mut::<DiffReceiver<C>>().ok_or_else(|| {
                    format!(
                        "received diff patches for `{}` before a snapshot",
                        ShortName::of::<C>()
                    )
                })?;
                receiver.queue_and_take_ready(first_patch_index, patches)
            };

            let mut component = entity.get_mut::<C>().ok_or_else(|| {
                format!(
                    "received diff patches for missing `{}`",
                    ShortName::of::<C>()
                )
            })?;
            for patch in ready_patches {
                component.apply_patch(&patch)?;
            }
        }
    }

    Ok(())
}

pub(crate) fn remove<C: Diffable>(_ctx: &mut RemoveCtx, entity: &mut DeferredEntity) {
    entity
        .remove::<C>()
        .remove::<DiffLog<C>>()
        .remove::<DiffReceiver<C>>();
}
