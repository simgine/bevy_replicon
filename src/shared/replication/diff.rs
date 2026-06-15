//! Patch-based diff replication for components.
//!
//! See [`Diffable`] for the main user-facing API and example.

pub mod patch_index;

use core::iter;

use alloc::{
    collections::{VecDeque, vec_deque},
    format,
    vec::Vec,
};

use bevy::{
    ecs::{
        component::{ComponentId, Mutable},
        system::EntityCommands,
        world::EntityWorldMut,
    },
    platform::collections::HashMap,
    prelude::*,
    ptr::Ptr,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize, Serializer, de::DeserializeOwned, ser::SerializeSeq};

use crate::{
    postcard_utils,
    shared::replication::{
        deferred_entity::DeferredEntity,
        registry::{
            ReplicationRegistry,
            ctx::{RemoveCtx, SerializeCtx, WriteCtx},
            rule_fns::{DeserializeFn, RuleFns},
        },
    },
};
use patch_index::PatchIndex;

/// Component whose mutations can be represented as an ordered history of patches.
///
/// Diff replication is useful when a component is large, but most changes can be
/// represented by a small semantic patch. A common example is a component that stores
/// a growing [`VecDeque`] of points for a trail/path.
/// Sending the full queue after every push can become expensive; sending a patch
/// like `PushBack(point)` or `PopFront(count)` only transmits the part that changed.
///
/// The component remains the authoritative state. The user provides a patch type and
/// implements [`Self::apply_patch`] to describe how each patch changes the component.
/// When the server mutates the component through [`DiffEntityExt::apply_patch`],
/// Replicon applies the patch locally and records it in a [`PatchHistory`]. For each
/// client, the server sends either the patches after that client's latest
/// acknowledged patch cursor, or a full snapshot if the needed patches are no longer
/// retained. On the receiver, patches are deduplicated, buffered until they can be
/// applied in order, and then applied to the local component. Components can override
/// [`Self::HISTORY_LEN`] to tune how many patches are kept before snapshot fallback
/// becomes necessary.
///
/// Direct component mutations are still supported after history exists, but they are not
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
/// #[derive(Clone, Copy, Deserialize, Serialize)]
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

/// Patch history associated with a [`Diffable`].
///
/// This component is registered as a required component for diff components.
/// It is not replicated directly.
#[derive(Component, Debug)]
pub struct PatchHistory<C: Diffable> {
    last_index: Option<PatchIndex>,
    patches: VecDeque<C::Patch>,
}

impl<C: Diffable> PatchHistory<C> {
    /// Records a patch and returns the assigned patch index.
    pub fn record(&mut self, patch: C::Patch) {
        let index = self.last_index.map_or(PatchIndex::new(0), |i| i + 1);

        self.last_index = Some(index);
        self.patches.push_back(patch);
        self.prune_to_limit();
    }

    /// Returns the latest patch index.
    pub fn current_cursor(&self) -> Option<PatchIndex> {
        self.last_index
    }

    /// Returns retained patches after `cursor`.
    ///
    /// Returns `None` if patches can't be used and the sender should fall back
    /// to a snapshot.
    pub(crate) fn patches_after(&self, cursor: PatchIndex) -> Option<BatchSlice<'_, C::Patch>> {
        let last_index = self.last_index?;
        if self.patches.is_empty() {
            return None;
        }

        let missing_count = last_index.distance_after(cursor) as usize;
        if missing_count == 0 {
            // Client is already at the latest cursor.
            // The component was mutated directly.
            return None;
        }

        if missing_count > self.patches.len() {
            // Client cursor is outside the history window.
            return None;
        }

        let start = self.patches.len() - missing_count;

        Some(BatchSlice {
            first_index: cursor + 1,
            patches: self.patches.range(start..),
        })
    }

    fn prune_to_limit(&mut self) {
        let excess = self.patches.len().saturating_sub(C::HISTORY_LEN);
        if excess > 0 {
            self.patches.drain(..excess);
        }
    }
}

pub(crate) struct BatchSlice<'a, Patch> {
    first_index: PatchIndex,
    patches: vec_deque::Iter<'a, Patch>,
}

impl<Patch: Serialize> Serialize for BatchSlice<'_, Patch> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.patches.len()))?;
        for patch in self.patches.clone() {
            seq.serialize_element(patch)?;
        }
        seq.end()
    }
}

impl<C: Diffable> Default for PatchHistory<C> {
    fn default() -> Self {
        Self {
            last_index: None,
            patches: Default::default(),
        }
    }
}

/// Receiver-side state for applying diff patches exactly once and in order.
#[derive(Component, Debug)]
pub struct PatchBuffer<C: Diffable> {
    last_applied: Option<PatchIndex>,
    pending: HashMap<PatchIndex, C::Patch>,
}

impl<C: Diffable> PatchBuffer<C> {
    pub fn new(cursor: Option<PatchIndex>) -> Self {
        Self {
            last_applied: cursor,
            pending: Default::default(),
        }
    }

    /// Returns the latest patch index applied to the live component.
    pub fn last_applied(&self) -> Option<PatchIndex> {
        self.last_applied
    }

    /// Queues newly received patches and returns patches that can be applied now.
    ///
    /// Patches must be applied sequentially by [`PatchIndex`]. If a patch arrives
    /// ahead of a missing predecessor, it stays pending until the missing patch is
    /// received. Duplicate or already-applied patches are ignored.
    pub fn queue_and_take_ready(
        &mut self,
        first_index: PatchIndex,
        patches: Vec<C::Patch>,
    ) -> impl Iterator<Item = C::Patch> + '_ {
        for (offset, patch) in patches.into_iter().enumerate() {
            let index = first_index + offset as u16;
            if self
                .last_applied
                .is_none_or(|last_applied| index.is_newer_than(last_applied))
            {
                self.pending.entry(index).or_insert(patch);
            }
        }

        iter::from_fn(move || {
            let next_index = self.last_applied.map_or(PatchIndex::new(0), |i| i + 1);
            let patch = self.pending.remove(&next_index)?;
            self.last_applied = Some(next_index);
            Some(patch)
        })
    }
}

impl<C: Diffable> Default for PatchBuffer<C> {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Wire format for diff replicated components.
#[derive(Deserialize, Serialize)]
#[serde(bound(deserialize = "C: Diffable"))]
pub enum DiffWire<C: Diffable> {
    Snapshot {
        cursor: Option<PatchIndex>,
        value: C,
    },
    Patches {
        first_index: PatchIndex,
        patches: Vec<C::Patch>,
    },
}

#[derive(Serialize)]
enum DiffWireRef<'a, C: Diffable> {
    Snapshot {
        cursor: Option<PatchIndex>,
        value: &'a C,
    },
    Patches {
        first_index: PatchIndex,
        patches: BatchSlice<'a, C::Patch>,
    },
}

/// Extension trait for recording diff patches on an entity.
pub trait DiffEntityExt {
    /// Applies `patch` to component `C` and records it in the entity's [`PatchHistory`].
    ///
    /// For [`EntityCommands`], this queues the patch application. Missing components
    /// or patch application errors are reported when commands are applied.
    fn apply_patch<C: Diffable>(&mut self, patch: C::Patch) -> Result<()>;
}

impl DiffEntityExt for EntityWorldMut<'_> {
    fn apply_patch<C: Diffable>(&mut self, patch: C::Patch) -> Result<()> {
        let entity = self.id();
        let mut component = self
            .get_mut::<C>()
            .ok_or_else(|| format!("`{entity}` doesn't have `{}`", ShortName::of::<C>()))?;
        component.apply_patch(&patch)?;

        let mut history = self.get_mut::<PatchHistory<C>>().ok_or_else(|| {
            format!(
                "`{entity}` doesn't have `{}`",
                ShortName::of::<PatchHistory<C>>()
            )
        })?;

        history.record(patch);

        Ok(())
    }
}

impl DiffEntityExt for EntityCommands<'_> {
    fn apply_patch<C: Diffable>(&mut self, patch: C::Patch) -> Result<()> {
        self.queue(move |mut entity: EntityWorldMut| entity.apply_patch::<C>(patch));
        Ok(())
    }
}

/// Diff functions for server-side serialization.
///
/// Diff components still use [`RuleFns`](crate::shared::replication::registry::rule_fns::RuleFns)
/// for snapshot payloads and receive-side deserialization. `DiffFns` stores the
/// extra state needed to serialize patches: the `PatchHistory<C>` component ID and a
/// type-erased serializer that can read both the component and its patch history.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DiffFns {
    register: fn(&mut World, &mut ReplicationRegistry) -> ComponentId,
    serialize_mutation: unsafe fn(
        &SerializeCtx,
        Ptr,
        Ptr,
        Option<PatchIndex>,
        &mut Vec<u8>,
    ) -> Result<Option<PatchIndex>>,
}

impl DiffFns {
    pub(crate) fn new<C: Diffable>() -> Self {
        Self {
            register: register_diff_state::<C>,
            serialize_mutation: serialize_mutation::<C>,
        }
    }

    pub(crate) fn register(
        &self,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) -> ComponentId {
        (self.register)(world, registry)
    }

    /// Serializes patches after `base_cursor`, or a snapshot if required.
    ///
    /// If `base_cursor` is [`None`], or if the needed patches were already
    /// pruned, this falls back to a snapshot.
    ///
    /// # Safety
    ///
    /// `component` must point to `C`, and `history` must point to `PatchHistory<C>`.
    pub(crate) unsafe fn serialize_mutation(
        &self,
        ctx: &SerializeCtx,
        component: Ptr,
        history: Ptr,
        base_cursor: Option<PatchIndex>,
        message: &mut Vec<u8>,
    ) -> Result<Option<PatchIndex>> {
        unsafe { (self.serialize_mutation)(ctx, component, history, base_cursor, message) }
    }
}

fn register_diff_state<C: Diffable>(
    world: &mut World,
    registry: &mut ReplicationRegistry,
) -> ComponentId {
    world.register_required_components::<C, PatchHistory<C>>();
    registry.set_receive_fns::<C>(world, write::<C>, remove::<C>);
    world.register_component::<PatchHistory<C>>()
}

unsafe fn serialize_mutation<C: Diffable>(
    _ctx: &SerializeCtx,
    component: Ptr,
    history: Ptr,
    base_cursor: Option<PatchIndex>,
    message: &mut Vec<u8>,
) -> Result<Option<PatchIndex>> {
    let component = unsafe { component.deref::<C>() };
    let history = unsafe { history.deref::<PatchHistory<C>>() };
    let cursor = history.current_cursor();

    let wire = match base_cursor.and_then(|cursor| history.patches_after(cursor)) {
        Some(slice) => DiffWireRef::Patches {
            first_index: slice.first_index,
            patches: slice,
        },
        None => DiffWireRef::Snapshot {
            cursor,
            value: component,
        },
    };
    postcard_utils::to_extend_mut(&wire, message)?;

    Ok(cursor)
}

/// Serializes a full snapshot when only the component is available.
///
/// The normal server path uses [`DiffFns::serialize_mutation`] because it can
/// access the component's [`PatchHistory`]. This function is the [`RuleFns`] snapshot
/// serializer for generic paths that only receive `&C`.
pub(crate) fn serialize_snapshot_without_history<C: Diffable>(
    _ctx: &SerializeCtx,
    component: &C,
    message: &mut Vec<u8>,
) -> Result<()> {
    let wire = DiffWireRef::Snapshot {
        cursor: None,
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
    ctx: &mut WriteCtx,
    message: &mut Bytes,
) -> Result<C> {
    match postcard_utils::from_buf(message)? {
        DiffWire::Snapshot { mut value, .. } => {
            C::map_entities(&mut value, ctx);
            Ok(value)
        }
        DiffWire::Patches { .. } => Err(format!(
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
    let _wire: DiffWire<C> = postcard_utils::from_buf(message)?;
    Ok(())
}

pub(crate) fn write<C: Diffable>(
    ctx: &mut WriteCtx,
    _rule_fns: &RuleFns<C>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    // This is the live receive path for diff components. Snapshots replace or
    // insert the component and reset the receiver cursor; patches are queued and
    // applied only once all earlier patches have been applied.
    match postcard_utils::from_buf(message)? {
        DiffWire::Snapshot { cursor, mut value } => {
            C::map_entities(&mut value, ctx);
            if let Some(mut component) = entity.get_mut::<C>() {
                *component = value;
            } else {
                entity.insert(value);
            }
            entity.insert(PatchBuffer::<C>::new(cursor));
        }
        DiffWire::Patches {
            first_index,
            patches,
        } => {
            // SAFETY: components don't alias.
            let (mut component, mut buffer) =
                unsafe { entity.get_components_mut_unchecked::<(&mut C, &mut PatchBuffer<C>)>()? };
            for patch in buffer.queue_and_take_ready(first_index, patches) {
                component.apply_patch(&patch)?;
            }
        }
    }

    Ok(())
}

pub(crate) fn remove<C: Diffable>(_ctx: &mut RemoveCtx, entity: &mut DeferredEntity) {
    entity
        .remove_with_requires::<C>()
        .remove::<PatchBuffer<C>>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Component, Deserialize, Serialize)]
    struct TestDiff(u8);

    impl Diffable for TestDiff {
        type Patch = u8;

        fn apply_patch(&mut self, patch: &Self::Patch) -> Result<()> {
            self.0 = *patch;
            Ok(())
        }
    }

    #[test]
    fn batches_after_returns_retained_batches_after_cursor() {
        let mut history = PatchHistory::<TestDiff>::default();
        history.record(1);
        history.record(2);

        let slice = history.patches_after(PatchIndex::new(0)).unwrap();
        assert_eq!(slice.first_index.get(), 1);
        assert_ne!(slice.patches.len(), 0);
    }
}
