pub mod patch_index;

use core::iter;

use alloc::collections::vec_deque::{self, VecDeque};
use bevy::{
    ecs::{change_detection::Tick, component::Mutable},
    platform::collections::HashMap,
    prelude::*,
};
use serde::{Deserialize, Serialize, Serializer, de::DeserializeOwned, ser::SerializeSeq};

use crate::shared::replication::storage::ReplicationStorage;
use patch_index::PatchIndex;

/**
Component whose mutations can be represented as an ordered history of patches.

When a replicated component changes, the whole value gets sent. If you only change
part of a component, it's usually better to split the component to avoid sending
unchanged parts over the network. However, it's not always possible or convenient
and this is where diff replication is useful.

Bevy change detection works at component granularity, so it cannot tell which
field or collection element changed. Computing a diff would also be expensive,
especially for collections with many elements. To avoid this, we require users
to define a [`Self::Patch`] which describes a possible change and
[`Self::apply_diff`] which applies the change to the component.

To record changes, apply them via [`EntityCommandsPatchExt::apply_diff`]
or [`EntityPatchExt::apply_diff`]. Internally, patches are recorded in [`PatchHistory`].
For each client, the server sends either the patches after that client's latest
acknowledged patch cursor, or a full snapshot if the needed patches are no longer
retained. On the receiver, patches are deduplicated, buffered until they can be
applied in order, and then applied to the local component. Components can override
[`Self::HISTORY_LEN`] to tune how many patches are kept before snapshot fallback
becomes necessary.

You may mutate the component directly, but it won't be recorded as a patch.
Doing so will automatically reset the history and the change will be sent as a snapshot.

# Example

```
use std::collections::VecDeque;

# use bevy::state::app::StatesPlugin;
use bevy::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

# let mut app = App::new();
# app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins));
app.replicate_diff::<Trail>();

let mut entity = app.world_mut().spawn((Replicated, Trail(VecDeque::new())));

let point = Point { x: 1.0, y: 2.0 };
entity
    .apply_diff::<Trail>(TrailDiff::PushBack(point))
    .unwrap();

let trail = entity.get::<Trail>().unwrap();
assert_eq!(trail.0, [point]);

#[derive(Component, Serialize, Deserialize, PartialEq, Debug)]
struct Trail(VecDeque<Point>);

#[derive(Serialize, Deserialize, Clone, Copy)]
enum TrailDiff {
    PushBack(Point),
    PopFront(usize),
}

impl Diffable for Trail {
    type Patch = TrailDiff;
    const HISTORY_LEN: usize = 256;

    fn apply_diff(&mut self, patch: &Self::Patch) -> Result<()> {
        match *patch {
            TrailDiff::PushBack(point) => self.0.push_back(point),
            TrailDiff::PopFront(count) => {
                for _ in 0..count {
                    self.0.pop_front();
                }
            }
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}
```
*/
pub trait Diffable: Component<Mutability = Mutable> + Serialize + DeserializeOwned + Sized {
    /// A recordable change that transforms this component.
    type Patch: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Maximum number of patches retained for diff serialization.
    ///
    /// The value cannot exceed [`PatchIndex::MAX_NEWER_DISTANCE`].
    /// This keeps wrapping patch index comparisons unambiguous.
    /// The invariant is checked in debug builds.
    ///
    /// If a client acknowledges a patch outside the retained range,
    /// diff serialization falls back to a full component snapshot.
    const HISTORY_LEN: usize = 64;

    /// Applies a patch to the component.
    fn apply_diff(&mut self, patch: &Self::Patch) -> Result<()>;
}

/// Extension trait for [`EntityWorldMut`] to apply patches.
pub trait EntityPatchExt {
    /// Applies patch to component `C` and records it in the entity's [`PatchHistory`].
    ///
    /// Returns an error if the entity does not have a component of type `C`.
    fn apply_diff<C: Diffable>(&mut self, patch: C::Patch) -> Result<()>;
}

impl EntityPatchExt for EntityWorldMut<'_> {
    fn apply_diff<C: Diffable>(&mut self, patch: C::Patch) -> Result<()> {
        let entity = self.id();
        let mut component = self
            .get_mut::<C>()
            .ok_or_else(|| format!("`{entity}` doesn't have `{}`", ShortName::of::<C>()))?;

        let before_patch = component.last_changed();
        component.apply_diff(&patch)?;
        let after_patch = component.last_changed();

        let mut storage = self.resource_mut::<ReplicationStorage>();
        let history = storage.get_or_default::<PatchHistory<C>>(entity);
        history.record(patch, before_patch, after_patch);

        Ok(())
    }
}

/// Extension trait for [`EntityCommands`] to apply patches.
pub trait EntityCommandsPatchExt {
    /// Queues patch application to component `C` and records it in the entity's [`PatchHistory`].
    fn apply_diff<C: Diffable>(&mut self, patch: C::Patch) -> &mut Self;
}

impl EntityCommandsPatchExt for EntityCommands<'_> {
    fn apply_diff<C: Diffable>(&mut self, patch: C::Patch) -> &mut Self {
        self.queue(move |mut entity: EntityWorldMut| entity.apply_diff::<C>(patch))
    }
}

/// Patch history associated with a component.
///
/// Stored inside [`ReplicationStorage`].
#[derive(Debug, Clone)]
pub struct PatchHistory<C: Diffable> {
    next_index: PatchIndex,
    last_changed: Option<Tick>,
    patches: VecDeque<C::Patch>,
}

impl<C: Diffable> PatchHistory<C> {
    /// Records a patch.
    ///
    /// Ticks are used to verify that the component wasn't mutated outside
    /// the diff API.
    ///
    /// If an external mutation is detected, the patch history is cleared,
    /// forcing snapshot serialization. The patch will be dropped since no
    /// client could've seen the base value.
    fn record(&mut self, patch: C::Patch, before_patch: Tick, after_patch: Tick) {
        debug_assert!(
            C::HISTORY_LEN <= PatchIndex::MAX_NEWER_DISTANCE as usize,
            "`{}::HISTORY_LEN` cannot exceed {}",
            ShortName::of::<C>(),
            PatchIndex::MAX_NEWER_DISTANCE
        );

        if self.last_changed.is_some_and(|tick| tick != before_patch) {
            // The component was mutated externally. Increment the patch
            // index twice: once for the previous change(s) and one for the current.
            self.patches.clear();
            self.last_changed = Some(after_patch);
            self.next_index += 2;
            return;
        }

        self.next_index += 1;
        self.last_changed = Some(after_patch);

        self.patches.push_back(patch);
        let excess = self.patches.len().saturating_sub(C::HISTORY_LEN);
        if excess > 0 {
            self.patches.drain(..excess);
        }
    }

    /// Returns patches between the client cursor and the current patch index.
    ///
    /// If the returned patch iterator is empty, the sender should fall back to a
    /// snapshot. This function does not distinguish between "can't be patched"
    /// and "nothing to send", because the serialization path has already
    /// determined that this client should receive a change.
    ///
    /// Tick is used to verify that the component was not mutated outside
    /// the diff API. If an external mutation is detected, the patch history is cleared and
    /// an empty iterator is returned, forcing snapshot serialization.
    pub fn patches_after(
        &mut self,
        cursor: Option<PatchIndex>,
        last_changed: Tick,
    ) -> (PatchIndex, PatchesIter<'_, C::Patch>) {
        if self.last_changed.is_none_or(|tick| tick != last_changed) {
            // The component was mutated externally.
            self.patches.clear();
            self.last_changed = Some(last_changed);
            let current = self.next_index;
            self.next_index += 1;

            return (current, PatchesIter::empty(&self.patches));
        }

        let current = self.current_index();
        let Some(cursor) = cursor else {
            return (current, PatchesIter::empty(&self.patches));
        };

        let missing_count = current.distance_after(cursor) as usize;
        if self.patches.len() <= missing_count {
            return (current, PatchesIter::empty(&self.patches));
        }

        let start = self.patches.len() - missing_count;

        (current, PatchesIter::new(&self.patches, start))
    }

    /// Returns index of the latest patch.
    pub fn current_index(&self) -> PatchIndex {
        self.next_index - 1
    }
}

impl<C: Diffable> Default for PatchHistory<C> {
    fn default() -> Self {
        Self {
            last_changed: None,
            next_index: PatchIndex::default(),
            patches: Default::default(),
        }
    }
}

/// A deserializable component diff.
///
/// See also [`WireDiffRef`].
#[derive(Deserialize)]
#[serde(bound(deserialize = "C: Diffable"))]
pub enum WireDiff<C: Diffable> {
    Snapshot {
        /// Patch cursor established by this snapshot.
        index: PatchIndex,
        /// Component value at `index`.
        component: C,
    },
    Patches {
        /// Patch cursor after applying all patches.
        index: PatchIndex,
        /// Patches to apply, in order, to advance to `index`.
        patches: Vec<C::Patch>,
    },
}

/// A serializable component diff.
///
/// Separate from [`WireDiff`] to avoid heap allocation.
#[derive(Serialize)]
pub enum WireDiffRef<'a, C: Diffable> {
    Snapshot {
        /// Patch cursor established by this snapshot.
        index: PatchIndex,
        /// Component value at `index`.
        component: &'a C,
    },
    Patches {
        /// Patch cursor after applying all patches.
        index: PatchIndex,
        /// Patches to apply, in order, to advance to `index`.
        patches: PatchesIter<'a, C::Patch>,
    },
}

/// Wraps a [`VecDeque`] iterator so it can implement [`Serialize`].
///
/// We can't use a slice because [`VecDeque`] is not contiguous.
#[must_use]
#[derive(Deref)]
pub struct PatchesIter<'a, P>(vec_deque::Iter<'a, P>);

impl<'a, P> PatchesIter<'a, P> {
    fn new(patches: &'a VecDeque<P>, start: usize) -> Self {
        Self(patches.range(start..))
    }

    fn empty(patches: &'a VecDeque<P>) -> Self {
        Self(patches.range(patches.len()..))
    }
}

impl<P: Serialize> Serialize for PatchesIter<'_, P> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for patch in self.0.clone() {
            seq.serialize_element(patch)?;
        }
        seq.end()
    }
}

/// Receiver-side buffer for applying diff patches exactly once and in order.
///
/// Needed because the client applies newer messages first, but those messages
/// may not contain all required patches. Clients acknowledge mutations when
/// they are received, not when they are applied, so the server may send later
/// patches before earlier patches are applied.
///
/// For example, a client may buffer a message with patch 0, acknowledge it, and
/// then receive a message that only contains patch 1 (because patch 0 was already
/// acknowledged). Since the message with patch 1 is processed first, patch 1
/// cannot be applied immediately and is stored in the buffer. Once the message
/// with patch 0 is processed, [`Self::drain_ready`] returns patch 0 followed by
/// patch 1.
#[derive(Component, Debug)]
pub struct PatchBuffer<C: Diffable> {
    last_applied: Option<PatchIndex>,
    pending: HashMap<PatchIndex, C::Patch>,
}

impl<C: Diffable> PatchBuffer<C> {
    /// Sets last applied value to the given index.
    ///
    /// Resets the history.
    pub fn set_last_applied(&mut self, last_applied: PatchIndex) {
        self.last_applied = Some(last_applied);
        self.pending.clear();
    }

    /// Queues newly received patches.
    ///
    /// If a patch arrives ahead of a missing predecessor, it will stay pending
    /// until the missing patch is received. Duplicate or already applied
    /// patches are ignored.
    pub fn push(&mut self, last_index: PatchIndex, patches: Vec<C::Patch>) {
        for (offset, patch) in patches.into_iter().rev().enumerate() {
            let index = last_index - offset as u16;
            if self
                .last_applied
                .is_none_or(|last_applied| index.is_newer_than(last_applied))
            {
                self.pending.entry(index).or_insert(patch);
            }
        }
    }

    /// Returns patches that can be applied.
    pub fn drain_ready(&mut self) -> impl Iterator<Item = C::Patch> + '_ {
        iter::from_fn(move || {
            let index = self.last_applied.map_or(PatchIndex::new(0), |i| i + 1);
            let patch = self.pending.remove(&index)?;
            self.last_applied = Some(index);
            Some(patch)
        })
    }

    /// Returns the latest patch index applied to the live component.
    pub fn last_applied(&self) -> Option<PatchIndex> {
        self.last_applied
    }
}

impl<C: Diffable> Default for PatchBuffer<C> {
    fn default() -> Self {
        Self {
            last_applied: None,
            pending: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn too_long_history() {
        let mut history = PatchHistory::<TooLongHistory>::default();
        history.record((), Tick::new(0), Tick::new(1));
    }

    #[test]
    fn history_recording() {
        let mut history = PatchHistory::<Value>::default();

        history.record(ValueChange::Add(0), Tick::new(0), Tick::new(1));
        assert_eq!(history.current_index().get(), 0);
        assert_eq!(history.patches, [ValueChange::Add(0)]);
    }

    #[test]
    fn history_trimming() {
        let mut history = PatchHistory::<Value>::default();

        history.record(ValueChange::Add(0), Tick::new(0), Tick::new(1));
        history.record(ValueChange::Add(1), Tick::new(1), Tick::new(2));
        history.record(ValueChange::Add(2), Tick::new(2), Tick::new(3));
        history.record(ValueChange::Add(3), Tick::new(3), Tick::new(4));
        assert_eq!(history.current_index().get(), 3);
        assert_eq!(
            history.patches,
            [
                ValueChange::Add(1),
                ValueChange::Add(2),
                ValueChange::Add(3)
            ],
            "history should retain at most `HISTORY_LEN` patches"
        );
    }

    #[test]
    fn history_reset_on_record() {
        let mut history = PatchHistory::<Value>::default();

        history.record(ValueChange::Add(0), Tick::new(0), Tick::new(1));
        history.record(ValueChange::Add(1), Tick::new(10), Tick::new(11));
        assert_eq!(history.current_index().get(), 2);
        assert!(
            history.patches.is_empty(),
            "should reset if the recorded tick differs"
        );
    }

    #[test]
    fn history_patches_after() {
        let mut history = PatchHistory::<Value>::default();

        history.record(ValueChange::Add(0), Tick::new(0), Tick::new(1));
        history.record(ValueChange::Add(1), Tick::new(1), Tick::new(2));
        history.record(ValueChange::Add(2), Tick::new(2), Tick::new(3));

        let (index, patches) = history.patches_after(None, Tick::new(3));
        assert_eq!(index.get(), 2);
        assert_eq!(patches.len(), 0);

        let (index, patches) = history.patches_after(Some(PatchIndex::new(1)), Tick::new(3));
        assert_eq!(index.get(), 2);
        assert_eq!(
            patches.0.copied().collect::<Vec<_>>(),
            [ValueChange::Add(2)]
        );

        let (index, patches) = history.patches_after(Some(PatchIndex::new(2)), Tick::new(3));
        assert_eq!(index.get(), 2);
        assert_eq!(patches.len(), 0);

        let (index, patches) = history.patches_after(Some(PatchIndex::new(1)), Tick::new(4));
        assert_eq!(index.get(), 3);
        assert_eq!(patches.len(), 0);
        assert!(
            history.patches.is_empty(),
            "should reset if the recorded tick differs"
        );

        let (index, patches) = history.patches_after(Some(PatchIndex::new(2)), Tick::new(4));
        assert_eq!(index.get(), 3);
        assert_eq!(
            patches.len(),
            0,
            "shouldn't return any patches since the history is now empty"
        );
    }

    #[test]
    fn history_reset_on_patches() {
        let mut history = PatchHistory::<Value>::default();

        history.record(ValueChange::Add(0), Tick::new(0), Tick::new(1));
        history.record(ValueChange::Add(1), Tick::new(1), Tick::new(2));

        let (index, patches) = history.patches_after(Some(PatchIndex::new(0)), Tick::new(3));
        assert_eq!(index.get(), 2);
        assert_eq!(patches.len(), 0);
        assert!(
            history.patches.is_empty(),
            "should reset if the recorded tick differs"
        );

        let (index, patches) = history.patches_after(Some(PatchIndex::new(1)), Tick::new(3));
        assert_eq!(index.get(), 2);
        assert_eq!(
            patches.len(),
            0,
            "shouldn't return any patches since the history is now empty"
        );
    }

    #[test]
    fn buffering() {
        let mut buffer = PatchBuffer::<Value>::default();

        let patches = [ValueChange::Add(0), ValueChange::Add(1)];
        buffer.push(PatchIndex::new(1), patches.into());
        assert_eq!(buffer.pending.len(), 2);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(ready, patches);
        assert_eq!(buffer.last_applied, Some(PatchIndex::new(1)));
    }

    #[test]
    fn buffering_with_intersection() {
        let mut buffer = PatchBuffer::<Value>::default();

        buffer.push(
            PatchIndex::new(1),
            vec![ValueChange::Add(0), ValueChange::Add(1)],
        );
        buffer.push(
            PatchIndex::new(2),
            vec![ValueChange::Add(1), ValueChange::Add(2)],
        );
        assert_eq!(buffer.pending.len(), 3);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(
            ready,
            [
                ValueChange::Add(0),
                ValueChange::Add(1),
                ValueChange::Add(2)
            ]
        );
        assert_eq!(buffer.last_applied, Some(PatchIndex::new(2)));
    }

    #[test]
    fn buffering_out_of_order() {
        let mut buffer = PatchBuffer::<Value>::default();

        buffer.push(
            PatchIndex::new(3),
            vec![ValueChange::Add(2), ValueChange::Add(3)],
        );
        buffer.push(
            PatchIndex::new(1),
            vec![ValueChange::Add(0), ValueChange::Add(1)],
        );
        assert_eq!(buffer.pending.len(), 4);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(
            ready,
            [
                ValueChange::Add(0),
                ValueChange::Add(1),
                ValueChange::Add(2),
                ValueChange::Add(3)
            ]
        );
        assert_eq!(buffer.last_applied, Some(PatchIndex::new(3)));
    }

    #[test]
    fn buffering_with_missing() {
        let mut buffer = PatchBuffer::<Value>::default();

        buffer.push(PatchIndex::new(0), vec![ValueChange::Add(0)]);
        buffer.push(PatchIndex::new(2), vec![ValueChange::Add(2)]);
        assert_eq!(buffer.pending.len(), 2);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(
            ready,
            [ValueChange::Add(0)],
            "patch 2 requires patch 1 in the buffer"
        );
        assert_eq!(buffer.last_applied, Some(PatchIndex::new(0)));

        buffer.push(PatchIndex::new(1), vec![ValueChange::Add(1)]);
        assert_eq!(buffer.pending.len(), 2);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(
            ready,
            [ValueChange::Add(1), ValueChange::Add(2),],
            "patch 2 should be ready after receiving patch 1"
        );
        assert_eq!(buffer.last_applied, Some(PatchIndex::new(2)));
    }

    #[test]
    fn entity_patching() {
        let mut world = World::new();
        world.init_resource::<ReplicationStorage>();

        let mut entity = world.spawn(Value::default());
        entity.apply_diff::<Value>(ValueChange::Add(10)).unwrap();
        entity.apply_diff::<Value>(ValueChange::Sub(3)).unwrap();
        assert_eq!(entity.get::<Value>().copied(), Some(Value(7)));

        let entity = entity.id();
        let storage = world.resource::<ReplicationStorage>();
        let history = storage.get::<PatchHistory<Value>>(entity).unwrap();
        assert_eq!(history.patches, [ValueChange::Add(10), ValueChange::Sub(3)]);
    }

    #[test]
    fn entity_patching_on_missing() {
        let mut world = World::new();
        world.init_resource::<ReplicationStorage>();

        let mut entity = world.spawn_empty();
        assert!(entity.apply_diff::<Value>(ValueChange::Add(10)).is_err());
        assert!(entity.get::<Value>().is_none());
    }

    #[test]
    fn entity_patching_with_external_mutation() {
        let mut world = World::new();
        world.init_resource::<ReplicationStorage>();

        let mut entity = world.spawn(Value::default());
        entity.apply_diff::<Value>(ValueChange::Add(10)).unwrap();
        let entity_id = entity.id();

        world.increment_change_tick();

        let mut value = world.get_mut::<Value>(entity_id).unwrap();
        assert_eq!(*value, Value(10));
        value.set_changed(); // Mock external change after patch application.

        let mut entity = world.entity_mut(entity_id);
        entity.apply_diff::<Value>(ValueChange::Sub(3)).unwrap();
        assert_eq!(entity.get::<Value>().copied(), Some(Value(7)));

        let storage = world.resource::<ReplicationStorage>();
        let history = storage.get::<PatchHistory<Value>>(entity_id).unwrap();
        assert!(
            history.patches.is_empty(),
            "history should be cleared on external mutation"
        );
    }

    #[test]
    fn entity_commands_patching() {
        let mut world = World::new();
        world.init_resource::<ReplicationStorage>();

        let entity = world.spawn(Value::default()).id();
        let mut commands = world.commands();
        commands
            .entity(entity)
            .apply_diff::<Value>(ValueChange::Add(10))
            .apply_diff::<Value>(ValueChange::Sub(3));

        world.flush();

        assert_eq!(world.get::<Value>(entity).copied(), Some(Value(7)));

        let storage = world.resource::<ReplicationStorage>();
        let history = storage.get::<PatchHistory<Value>>(entity).unwrap();
        assert_eq!(history.patches, [ValueChange::Add(10), ValueChange::Sub(3)]);
    }

    #[derive(Component, Serialize, Deserialize)]
    struct TooLongHistory;

    impl Diffable for TooLongHistory {
        const HISTORY_LEN: usize = u16::MAX as usize;
        type Patch = ();

        fn apply_diff(&mut self, _patch: &Self::Patch) -> Result<()> {
            Ok(())
        }
    }

    #[derive(Component, Default, Deserialize, Serialize, PartialEq, Debug, Clone, Copy)]
    struct Value(u8);

    impl Diffable for Value {
        const HISTORY_LEN: usize = 3;
        type Patch = ValueChange;

        fn apply_diff(&mut self, patch: &Self::Patch) -> Result<()> {
            match *patch {
                ValueChange::Add(value) => self.0 += value,
                ValueChange::Sub(value) => self.0 -= value,
            }

            Ok(())
        }
    }

    #[derive(Debug, Deserialize, Serialize, PartialEq, Clone, Copy)]
    enum ValueChange {
        Add(u8),
        Sub(u8),
    }
}
