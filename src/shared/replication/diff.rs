//! Patch-based diff replication for components.
//!
//! See [`Diffable`] for the main user-facing API and example.

use core::iter;

use alloc::{
    collections::{BTreeMap, VecDeque},
    format,
    vec::Vec,
};

use bevy::{
    ecs::{
        component::{ComponentId, Mutable},
        system::EntityCommands,
        world::EntityWorldMut,
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
            ReplicationRegistry,
            ctx::{RemoveCtx, SerializeCtx, WriteCtx},
            rule_fns::{DeserializeFn, RuleFns},
        },
    },
};

/// Monotonic index assigned to a sent diff batch.
pub type PatchIndex = u64;

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
/// Components without sender-side [`PatchHistory`] don't match diff replication
/// rules. [`DiffEntityExt::apply_patch`] inserts this history automatically. Direct
/// component mutations are still supported after history exists, but they are not
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

    /// Maximum number of sent patch batches retained for diff serialization.
    ///
    /// If a client acknowledges a patch older than the retained range,
    /// Replicon will fall back to sending a full component snapshot.
    const HISTORY_LEN: usize = 64;

    /// Applies a patch to the component state.
    fn apply_patch(&mut self, patch: &Self::Patch) -> Result<()>;
}

/// Patch history associated with a [`Diffable`].
///
/// This sender-side component is inserted lazily when recording patches. It is
/// not replicated directly.
#[derive(Component, Debug)]
pub struct PatchHistory<C: Diffable> {
    last_index: Option<PatchIndex>,
    batches: VecDeque<PatchBatch<C::Patch>>,
    pending: Vec<C::Patch>,
}

impl<C: Diffable> PatchHistory<C> {
    /// Records a patch to be included in the next serialized diff batch.
    pub fn record(&mut self, patch: C::Patch) {
        self.pending.push(patch);
    }

    /// Returns the latest sealed patch index.
    pub fn current_cursor(&self) -> Option<PatchIndex> {
        self.last_index
    }

    /// Finishes patches recorded since the previous serialization into one batch.
    fn finish_pending(&mut self) -> Option<PatchIndex> {
        if self.pending.is_empty() {
            return self.last_index;
        }

        let index = self
            .last_index
            .map_or(0, |last_index| last_index.saturating_add(1));
        self.last_index = Some(index);
        self.batches.push_back(core::mem::take(&mut self.pending));
        self.prune_to_limit();
        self.last_index
    }

    /// Returns all retained patch batches after `cursor`.
    ///
    /// Returns `None` if batches needed to continue from `cursor` were already
    /// pruned and the sender must fall back to a snapshot.
    pub(crate) fn batches_after(&self, cursor: PatchIndex) -> Option<BatchSlice<'_, C::Patch>> {
        let Some(last_index) = self.last_index else {
            return Some(BatchSlice {
                first_index: 0,
                batches: &self.batches,
                start: 0,
            });
        };
        if self.batches.is_empty() {
            return (cursor == last_index).then_some(BatchSlice {
                first_index: last_index.saturating_add(1),
                batches: &self.batches,
                start: 0,
            });
        }

        let first_index = self.first_index();
        if first_index > 0 && cursor < first_index - 1 {
            return None;
        }

        let start = if cursor >= last_index {
            self.batches.len()
        } else {
            (cursor + 1 - first_index) as usize
        };
        Some(BatchSlice {
            first_index: first_index + start as PatchIndex,
            batches: &self.batches,
            start,
        })
    }

    fn first_index(&self) -> PatchIndex {
        let last_index = self
            .last_index
            .expect("patch index should only be requested when batches exist");
        debug_assert!(!self.batches.is_empty());
        debug_assert!(self.batches.len() as PatchIndex - 1 <= last_index);
        last_index - (self.batches.len() as PatchIndex - 1)
    }

    fn prune_to_limit(&mut self) {
        let excess = self.batches.len().saturating_sub(C::HISTORY_LEN);
        if excess > 0 {
            self.batches.drain(..excess);
        }
    }
}

pub type PatchBatch<Patch> = Vec<Patch>;

pub(crate) struct BatchSlice<'a, Patch> {
    first_index: PatchIndex,
    batches: &'a VecDeque<PatchBatch<Patch>>,
    start: usize,
}

impl<Patch> BatchSlice<'_, Patch> {
    fn is_empty(&self) -> bool {
        self.start == self.batches.len()
    }

    fn first_index(&self) -> PatchIndex {
        self.first_index
    }
}

impl<Patch: Serialize> Serialize for BatchSlice<'_, Patch> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> core::result::Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.batches.len() - self.start))?;
        for batch in self.batches.iter().skip(self.start) {
            seq.serialize_element(batch)?;
        }
        seq.end()
    }
}

impl<C: Diffable> Default for PatchHistory<C> {
    fn default() -> Self {
        Self {
            last_index: None,
            batches: Default::default(),
            pending: Default::default(),
        }
    }
}

/// Receiver-side state for applying diff patches exactly once and in order.
#[derive(Component, Debug)]
pub struct PatchBuffer<C: Diffable> {
    last_applied: Option<PatchIndex>,
    pending: BTreeMap<PatchIndex, PatchBatch<C::Patch>>,
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

    /// Queues newly received patch batches and returns batches that can be applied now.
    ///
    /// Batches must be applied sequentially by [`PatchIndex`]. If a batch arrives
    /// ahead of a missing predecessor, it stays pending until the missing batch is
    /// received. Duplicate or already-applied batches are ignored.
    pub fn queue_and_take_ready(
        &mut self,
        first_patch_index: PatchIndex,
        batches: Vec<PatchBatch<C::Patch>>,
    ) -> impl Iterator<Item = PatchBatch<C::Patch>> + '_ {
        for (offset, batch) in batches.into_iter().enumerate() {
            let index = first_patch_index + offset as PatchIndex;
            if self
                .last_applied
                .is_none_or(|last_applied| index > last_applied)
            {
                self.pending.entry(index).or_insert(batch);
            }
        }

        iter::from_fn(move || {
            let next_index = self.next_patch_index()?;
            let batch = self.pending.remove(&next_index)?;
            self.last_applied = Some(next_index);
            Some(batch)
        })
    }

    fn next_patch_index(&self) -> Option<PatchIndex> {
        match self.last_applied {
            Some(index) => index.checked_add(1),
            None => Some(0),
        }
    }
}

impl<C: Diffable> Default for PatchBuffer<C> {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Wire format for diff replicated components.
#[derive(Deserialize, Serialize)]
pub enum DiffWire<C, Patch> {
    Snapshot {
        cursor: Option<PatchIndex>,
        value: C,
    },
    Patches {
        first_patch_index: PatchIndex,
        patches: Vec<PatchBatch<Patch>>,
    },
}

#[derive(Serialize)]
enum DiffWireRef<'a, C, Patch> {
    Snapshot {
        cursor: Option<PatchIndex>,
        value: &'a C,
    },
    Patches {
        first_patch_index: PatchIndex,
        patches: BatchSlice<'a, Patch>,
    },
}

/// Extension trait for recording diff patches on an entity.
pub trait DiffEntityExt {
    /// Applies `patch` to component `C` and records it in the entity's [`PatchHistory`].
    ///
    /// [`EntityWorldMut`] and [`EntityCommands`] insert missing patch history before
    /// recording.
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

        if let Some(mut history) = self.get_mut::<PatchHistory<C>>() {
            history.record(patch);
        } else {
            let mut history = PatchHistory::<C>::default();
            history.record(patch);
            self.insert(history);
        }

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
    /// Component ID for `PatchHistory<C>` associated with the diff component.
    pub(crate) history_component_id: Option<ComponentId>,
    pub(crate) register_diff_state: fn(&mut World, &mut ReplicationRegistry) -> ComponentId,
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
            history_component_id: None,
            register_diff_state: register_diff_state::<C>,
            serialize_mutation: serialize_mutation::<C>,
        }
    }

    pub(crate) fn history_component_id(&self) -> ComponentId {
        self.history_component_id
            .expect("diff functions should be registered before use")
    }

    /// Serializes patches after `base_cursor`, or a snapshot if required.
    ///
    /// If `base_cursor` is [`None`], or if the needed batches were already
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

pub(crate) fn register_diff_state<C: Diffable>(
    world: &mut World,
    registry: &mut ReplicationRegistry,
) -> ComponentId {
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
    let history = unsafe { history.assert_unique().deref_mut::<PatchHistory<C>>() };
    let cursor = history.finish_pending();

    let wire: DiffWireRef<'_, C, C::Patch> =
        match base_cursor.and_then(|cursor| history.batches_after(cursor)) {
            Some(batches) if !batches.is_empty() => DiffWireRef::Patches {
                first_patch_index: batches.first_index(),
                patches: batches,
            },
            _ => DiffWireRef::Snapshot {
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
    let wire: DiffWireRef<'_, C, C::Patch> = DiffWireRef::Snapshot {
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
        DiffWire::<C, C::Patch>::Snapshot { mut value, .. } => {
            C::map_entities(&mut value, ctx);
            Ok(value)
        }
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
    ctx: &mut WriteCtx,
    _rule_fns: &RuleFns<C>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    // This is the live receive path for diff components. Snapshots replace or
    // insert the component and reset the receiver cursor; patches are queued and
    // applied only once all earlier patches have been applied.
    let wire: DiffWire<C, C::Patch> = postcard_utils::from_buf(message)?;

    match wire {
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
            first_patch_index,
            patches,
        } => {
            // SAFETY: components don't alias.
            let (mut component, mut buffer) =
                unsafe { entity.get_components_mut_unchecked::<(&mut C, &mut PatchBuffer<C>)>()? };
            for batch in buffer.queue_and_take_ready(first_patch_index, patches) {
                for patch in batch.iter() {
                    component.apply_patch(patch)?;
                }
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
        history.finish_pending();
        history.record(2);
        history.finish_pending();

        let batches = history.batches_after(0).unwrap();
        assert_eq!(batches.first_index(), 1);
        assert!(!batches.is_empty());
    }

    #[test]
    fn entity_world_mut_apply_patch_inserts_missing_history() {
        let mut world = World::new();
        let entity = world.spawn(TestDiff(0)).id();

        world.entity_mut(entity).apply_patch::<TestDiff>(1).unwrap();

        let entity = world.entity(entity);
        assert_eq!(entity.get::<TestDiff>().unwrap().0, 1);
        assert!(entity.contains::<PatchHistory<TestDiff>>());
    }
}
