pub mod diff_index;

use core::iter;

use alloc::collections::vec_deque::{self, VecDeque};
use bevy::{
    ecs::{change_detection::Tick, component::Mutable},
    platform::collections::HashMap,
    prelude::*,
};
use serde::{Deserialize, Serialize, Serializer, de::DeserializeOwned, ser::SerializeSeq};

use crate::shared::replication::storage::ReplicationStorage;
use diff_index::DiffIndex;

/**
Component whose mutations can be represented as an ordered history of diffs.

When a replicated component changes, the whole value gets sent. If you only change
part of a component, it's usually better to split the component to avoid sending
unchanged parts over the network. However, it's not always possible or convenient
and this is where diff replication is useful.

Bevy change detection works at component granularity, so it cannot tell which
field or collection element changed. Computing a diff would also be expensive,
especially for collections with many elements. To avoid this, we require users
to define a [`Self::Diff`] which describes a possible change and
[`Self::apply_diff`] which applies the change to the component.

To record changes, apply them via [`EntityCommandsDiffExt::apply_diff`]
or [`EntityDiffExt::apply_diff`]. Internally, diffs are recorded in [`DiffHistory`].
For each client, the server sends either the diffs after that client's latest
acknowledged cursor, or a full snapshot if the needed diffs are no longer
retained. On the receiver, diffs are deduplicated, buffered until they can be
applied in order, and then applied to the local component. Components can override
[`Self::HISTORY_LEN`] to tune how many diffs are kept before snapshot fallback
becomes necessary.

You may mutate the component directly, but it won't be recorded as a diff.
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
    type Diff = TrailDiff;
    const HISTORY_LEN: usize = 256;

    fn apply_diff(&mut self, diff: &Self::Diff) -> Result<()> {
        match *diff {
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
    type Diff: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Maximum number of diffs retained for diff serialization.
    ///
    /// The value cannot exceed [`DiffIndex::MAX_NEWER_DISTANCE`].
    /// This keeps wrapping diff index comparisons unambiguous.
    /// The invariant is checked in debug builds.
    ///
    /// If a client acknowledges a diff outside the retained range,
    /// diff serialization falls back to a full component snapshot.
    const HISTORY_LEN: usize = 64;

    /// Applies a diff to the component.
    fn apply_diff(&mut self, diff: &Self::Diff) -> Result<()>;
}

/// Extension trait for [`EntityWorldMut`] to apply diffs.
///
/// See also [`EntityCommandsDiffExt`].
pub trait EntityDiffExt {
    /// Applies a diff to component `C` and records it in the entity's [`DiffHistory`].
    ///
    /// Returns an error if the entity does not have component `C`.
    fn apply_diff<C: Diffable>(&mut self, diff: C::Diff) -> Result<()>;
}

impl EntityDiffExt for EntityWorldMut<'_> {
    fn apply_diff<C: Diffable>(&mut self, diff: C::Diff) -> Result<()> {
        let entity = self.id();
        let mut component = self
            .get_mut::<C>()
            .ok_or_else(|| format!("`{entity}` doesn't have `{}`", ShortName::of::<C>()))?;

        let before_diff = component.last_changed();
        component.apply_diff(&diff)?;
        let after_diff = component.last_changed();

        let mut storage = self.resource_mut::<ReplicationStorage>();
        let history = storage.get_or_default::<DiffHistory<C>>(entity);
        history.record(diff, before_diff, after_diff);

        Ok(())
    }
}

/// Extension trait for [`EntityCommands`] to apply diffs.
///
/// See also [`EntityDiffExt`].
pub trait EntityCommandsDiffExt {
    /// Queues application of a diff to component `C` and records it in the entity's [`DiffHistory`].
    fn apply_diff<C: Diffable>(&mut self, diff: C::Diff) -> &mut Self;
}

impl EntityCommandsDiffExt for EntityCommands<'_> {
    fn apply_diff<C: Diffable>(&mut self, diff: C::Diff) -> &mut Self {
        self.queue(move |mut entity: EntityWorldMut| entity.apply_diff::<C>(diff))
    }
}

/// Extension trait for [`EntityCommands`] to apply diffs to resources.
///
/// See also [`CommandsDiffExt`].
pub trait WorldDiffExt {
    /// Applies a diff to resource `R` and records it in the global [`DiffHistory`].
    ///
    /// Returns an error if resource `R` does not exist.
    fn apply_resource_diff<R: Resource + Diffable>(&mut self, diff: R::Diff) -> Result<()>;
}

impl WorldDiffExt for World {
    fn apply_resource_diff<R: Resource + Diffable>(&mut self, diff: R::Diff) -> Result<()> {
        let Some(entity) = self
            .component_id::<R>()
            .and_then(|id| self.resource_entities().get(id))
        else {
            return Err(format!("missing resource `{}`", ShortName::of::<R>()).into());
        };

        self.entity_mut(entity).apply_diff::<R>(diff)
    }
}

/// Extension trait for [`Commands`] to apply diffs to resources.
///
/// See also [`WorldDiffExt`].
pub trait CommandsDiffExt {
    /// Queues application of a diff to resource `R` and records it in the global [`DiffHistory`].
    fn apply_resource_diff<R: Resource + Diffable>(&mut self, diff: R::Diff) -> &mut Self;
}

impl CommandsDiffExt for Commands<'_, '_> {
    fn apply_resource_diff<R: Resource + Diffable>(&mut self, diff: R::Diff) -> &mut Self {
        self.queue(move |entity: &mut World| entity.apply_resource_diff::<R>(diff));
        self
    }
}

/// Diff history associated with a component.
///
/// Stored inside [`ReplicationStorage`].
#[derive(Debug, Clone)]
pub struct DiffHistory<C: Diffable> {
    next_index: DiffIndex,
    last_changed: Option<Tick>,
    diffs: VecDeque<C::Diff>,
}

impl<C: Diffable> DiffHistory<C> {
    /// Records a diff.
    ///
    /// Ticks are used to verify that the component wasn't mutated outside
    /// the diff API.
    ///
    /// If an external mutation is detected, the diff history is cleared,
    /// forcing snapshot serialization. The diff will be dropped since no
    /// client could've seen the base value.
    fn record(&mut self, diff: C::Diff, before_diff: Tick, after_diff: Tick) {
        debug_assert!(
            C::HISTORY_LEN <= DiffIndex::MAX_NEWER_DISTANCE as usize,
            "`{}::HISTORY_LEN` cannot exceed {}",
            ShortName::of::<C>(),
            DiffIndex::MAX_NEWER_DISTANCE
        );

        if self.last_changed.is_some_and(|tick| tick != before_diff) {
            // The component was mutated externally. Increment the
            // index twice: once for the previous change(s) and one for the current.
            self.diffs.clear();
            self.last_changed = Some(after_diff);
            self.next_index += 2;
            return;
        }

        self.next_index += 1;
        self.last_changed = Some(after_diff);

        self.diffs.push_back(diff);
        let excess = self.diffs.len().saturating_sub(C::HISTORY_LEN);
        if excess > 0 {
            self.diffs.drain(..excess);
        }
    }

    /// Returns diffs between the client cursor and the current diff index.
    ///
    /// If the returned diff iterator is empty, the sender should fall back to a
    /// snapshot. This function does not distinguish between "can't be diffed"
    /// and "nothing to send", because the serialization path has already
    /// determined that this client should receive a change.
    ///
    /// Tick is used to verify that the component was not mutated outside
    /// the diff API. If an external mutation is detected, the diff history is cleared and
    /// an empty iterator is returned, forcing snapshot serialization.
    pub fn diffs_after(
        &mut self,
        cursor: Option<DiffIndex>,
        last_changed: Tick,
    ) -> (DiffIndex, DiffIter<'_, C::Diff>) {
        if self.last_changed.is_none_or(|tick| tick != last_changed) {
            // The component was mutated externally.
            self.diffs.clear();
            self.last_changed = Some(last_changed);
            let current = self.next_index;
            self.next_index += 1;

            return (current, DiffIter::empty(&self.diffs));
        }

        let current = self.current_index();
        let Some(cursor) = cursor else {
            return (current, DiffIter::empty(&self.diffs));
        };

        let missing_count = current.distance_after(cursor) as usize;
        if self.diffs.len() <= missing_count {
            return (current, DiffIter::empty(&self.diffs));
        }

        let start = self.diffs.len() - missing_count;

        (current, DiffIter::new(&self.diffs, start))
    }

    /// Returns index of the latest diff.
    pub fn current_index(&self) -> DiffIndex {
        self.next_index - 1
    }
}

impl<C: Diffable> Default for DiffHistory<C> {
    fn default() -> Self {
        Self {
            last_changed: None,
            next_index: DiffIndex::default(),
            diffs: Default::default(),
        }
    }
}

/// A deserializable component delta.
///
/// See also [`ComponentDeltaRef`].
#[derive(Deserialize)]
#[serde(bound(deserialize = "C: Diffable"))]
pub enum ComponentDelta<C: Diffable> {
    Snapshot {
        /// Diff cursor established by this snapshot.
        index: DiffIndex,
        /// Component value at `index`.
        component: C,
    },
    Diffs {
        /// Diff cursor after applying all diffs.
        index: DiffIndex,
        /// Diffs to apply, in order, to advance to `index`.
        diffs: Vec<C::Diff>,
    },
}

/// A serializable component delta.
///
/// Separate from [`ComponentDelta`] to avoid heap allocation.
#[derive(Serialize)]
pub enum ComponentDeltaRef<'a, C: Diffable> {
    Snapshot {
        /// Diff cursor established by this snapshot.
        index: DiffIndex,
        /// Component value at `index`.
        component: &'a C,
    },
    Diffs {
        /// Diff cursor after applying all diffs.
        index: DiffIndex,
        /// Diffs to apply, in order, to advance to `index`.
        diffs: DiffIter<'a, C::Diff>,
    },
}

/// Wraps a [`VecDeque`] iterator so it can implement [`Serialize`].
///
/// We can't use a slice because [`VecDeque`] is not contiguous.
#[must_use]
#[derive(Deref)]
pub struct DiffIter<'a, T>(vec_deque::Iter<'a, T>);

impl<'a, T> DiffIter<'a, T> {
    fn new(diffs: &'a VecDeque<T>, start: usize) -> Self {
        Self(diffs.range(start..))
    }

    fn empty(diffs: &'a VecDeque<T>) -> Self {
        Self(diffs.range(diffs.len()..))
    }
}

impl<T: Serialize> Serialize for DiffIter<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for diff in self.0.clone() {
            seq.serialize_element(diff)?;
        }
        seq.end()
    }
}

/// Receiver-side buffer for applying diff diffs exactly once and in order.
///
/// Needed because the client applies newer messages first, but those messages
/// may not contain all required diffs. Clients acknowledge mutations when
/// they are received, not when they are applied, so the server may send later
/// diffs before earlier diffs are applied.
///
/// For example, a client may buffer a message with diff 0, acknowledge it, and
/// then receive a message that only contains diff 1 (because diff 0 was already
/// acknowledged). Since the message with diff 1 is processed first, diff 1
/// cannot be applied immediately and is stored in the buffer. Once the message
/// with diff 0 is processed, [`Self::drain_ready`] returns diff 0 followed by
/// diff 1.
#[derive(Component, Debug)]
pub struct DiffBuffer<C: Diffable> {
    last_applied: Option<DiffIndex>,
    pending: HashMap<DiffIndex, C::Diff>,
}

impl<C: Diffable> DiffBuffer<C> {
    /// Sets last applied value to the given index.
    ///
    /// Resets the history.
    pub fn set_last_applied(&mut self, last_applied: DiffIndex) {
        self.last_applied = Some(last_applied);
        self.pending.clear();
    }

    /// Queues newly received diffs.
    ///
    /// If a diff arrives ahead of a missing predecessor, it will stay pending
    /// until the missing diff is received. Duplicate or already applied
    /// diffs are ignored.
    pub fn push(&mut self, last_index: DiffIndex, diffs: Vec<C::Diff>) {
        for (offset, diff) in diffs.into_iter().rev().enumerate() {
            let index = last_index - offset as u16;
            if self
                .last_applied
                .is_none_or(|last_applied| index.is_newer_than(last_applied))
            {
                self.pending.insert(index, diff);
            }
        }
    }

    /// Returns diffs that can be applied.
    pub fn drain_ready(&mut self) -> impl Iterator<Item = C::Diff> + '_ {
        iter::from_fn(move || {
            let index = self.last_applied.map_or(DiffIndex::new(0), |i| i + 1);
            let diff = self.pending.remove(&index)?;
            self.last_applied = Some(index);
            Some(diff)
        })
    }

    /// Returns the latest diff index applied to the live component.
    pub fn last_applied(&self) -> Option<DiffIndex> {
        self.last_applied
    }
}

impl<C: Diffable> Default for DiffBuffer<C> {
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
        let mut history = DiffHistory::<TooLongHistory>::default();
        history.record((), Tick::new(0), Tick::new(1));
    }

    #[test]
    fn history_recording() {
        let mut history = DiffHistory::<Value>::default();

        history.record(ValueDiff::Add(0), Tick::new(0), Tick::new(1));
        assert_eq!(history.current_index().get(), 0);
        assert_eq!(history.diffs, [ValueDiff::Add(0)]);
    }

    #[test]
    fn history_trimming() {
        let mut history = DiffHistory::<Value>::default();

        history.record(ValueDiff::Add(0), Tick::new(0), Tick::new(1));
        history.record(ValueDiff::Add(1), Tick::new(1), Tick::new(2));
        history.record(ValueDiff::Add(2), Tick::new(2), Tick::new(3));
        history.record(ValueDiff::Add(3), Tick::new(3), Tick::new(4));
        assert_eq!(history.current_index().get(), 3);
        assert_eq!(
            history.diffs,
            [ValueDiff::Add(1), ValueDiff::Add(2), ValueDiff::Add(3)],
            "history should retain at most `HISTORY_LEN` diffs"
        );
    }

    #[test]
    fn history_reset_on_record() {
        let mut history = DiffHistory::<Value>::default();

        history.record(ValueDiff::Add(0), Tick::new(0), Tick::new(1));
        history.record(ValueDiff::Add(1), Tick::new(10), Tick::new(11));
        assert_eq!(history.current_index().get(), 2);
        assert!(
            history.diffs.is_empty(),
            "should reset if the recorded tick differs"
        );
    }

    #[test]
    fn history_diffs_after() {
        let mut history = DiffHistory::<Value>::default();

        history.record(ValueDiff::Add(0), Tick::new(0), Tick::new(1));
        history.record(ValueDiff::Add(1), Tick::new(1), Tick::new(2));
        history.record(ValueDiff::Add(2), Tick::new(2), Tick::new(3));
        history.record(ValueDiff::Add(3), Tick::new(3), Tick::new(4));

        let (index, diffs) = history.diffs_after(None, Tick::new(4));
        assert_eq!(index.get(), 3);
        assert_eq!(diffs.len(), 0);

        let (index, diffs) = history.diffs_after(Some(DiffIndex::new(0)), Tick::new(4));
        assert_eq!(index.get(), 3);
        assert_eq!(
            diffs.len(),
            0,
            "shouldn't return diffs for indices outside of the history"
        );

        let (index, diffs) = history.diffs_after(Some(DiffIndex::new(1)), Tick::new(4));
        assert_eq!(index.get(), 3);
        assert_eq!(
            diffs.0.copied().collect::<Vec<_>>(),
            [ValueDiff::Add(2), ValueDiff::Add(3)]
        );

        let (index, diffs) = history.diffs_after(Some(DiffIndex::new(3)), Tick::new(4));
        assert_eq!(index.get(), 3);
        assert_eq!(diffs.len(), 0);
    }

    #[test]
    fn history_reset_on_diffs_after() {
        let mut history = DiffHistory::<Value>::default();

        history.record(ValueDiff::Add(0), Tick::new(0), Tick::new(1));
        history.record(ValueDiff::Add(1), Tick::new(1), Tick::new(2));

        let (index, diffs) = history.diffs_after(Some(DiffIndex::new(0)), Tick::new(3));
        assert_eq!(index.get(), 2);
        assert_eq!(diffs.len(), 0);
        assert!(
            history.diffs.is_empty(),
            "should reset if the recorded tick differs"
        );

        let (index, diffs) = history.diffs_after(Some(DiffIndex::new(1)), Tick::new(3));
        assert_eq!(index.get(), 2);
        assert_eq!(
            diffs.len(),
            0,
            "shouldn't return any diffs since the history is now empty"
        );
    }

    #[test]
    fn buffering() {
        let mut buffer = DiffBuffer::<Value>::default();

        let diffs = [ValueDiff::Add(0), ValueDiff::Add(1)];
        buffer.push(DiffIndex::new(1), diffs.into());
        assert_eq!(buffer.pending.len(), 2);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(ready, diffs);
        assert_eq!(buffer.last_applied, Some(DiffIndex::new(1)));
    }

    #[test]
    fn buffering_with_intersection() {
        let mut buffer = DiffBuffer::<Value>::default();

        buffer.push(
            DiffIndex::new(1),
            vec![ValueDiff::Add(0), ValueDiff::Add(1)],
        );
        buffer.push(
            DiffIndex::new(2),
            vec![ValueDiff::Add(1), ValueDiff::Add(2)],
        );
        assert_eq!(buffer.pending.len(), 3);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(
            ready,
            [ValueDiff::Add(0), ValueDiff::Add(1), ValueDiff::Add(2)]
        );
        assert_eq!(buffer.last_applied, Some(DiffIndex::new(2)));
    }

    #[test]
    fn buffering_out_of_order() {
        let mut buffer = DiffBuffer::<Value>::default();

        buffer.push(
            DiffIndex::new(3),
            vec![ValueDiff::Add(2), ValueDiff::Add(3)],
        );
        buffer.push(
            DiffIndex::new(1),
            vec![ValueDiff::Add(0), ValueDiff::Add(1)],
        );
        assert_eq!(buffer.pending.len(), 4);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(
            ready,
            [
                ValueDiff::Add(0),
                ValueDiff::Add(1),
                ValueDiff::Add(2),
                ValueDiff::Add(3)
            ]
        );
        assert_eq!(buffer.last_applied, Some(DiffIndex::new(3)));
    }

    #[test]
    fn buffering_with_missing() {
        let mut buffer = DiffBuffer::<Value>::default();

        buffer.push(DiffIndex::new(0), vec![ValueDiff::Add(0)]);
        buffer.push(DiffIndex::new(2), vec![ValueDiff::Add(2)]);
        assert_eq!(buffer.pending.len(), 2);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(
            ready,
            [ValueDiff::Add(0)],
            "diff 2 requires diff 1 in the buffer"
        );
        assert_eq!(buffer.last_applied, Some(DiffIndex::new(0)));

        buffer.push(DiffIndex::new(1), vec![ValueDiff::Add(1)]);
        assert_eq!(buffer.pending.len(), 2);

        let ready: Vec<_> = buffer.drain_ready().collect();
        assert_eq!(
            ready,
            [ValueDiff::Add(1), ValueDiff::Add(2),],
            "diff 2 should be ready after receiving diff 1"
        );
        assert_eq!(buffer.last_applied, Some(DiffIndex::new(2)));
    }

    #[test]
    fn apply_diff_command() {
        let mut world = World::new();
        world.init_resource::<ReplicationStorage>();

        let entity = world.spawn(Value::default()).id();
        let mut commands = world.commands();
        commands
            .entity(entity)
            .apply_diff::<Value>(ValueDiff::Add(10))
            .apply_diff::<Value>(ValueDiff::Sub(3));

        world.flush();

        assert_eq!(world.get::<Value>(entity).copied(), Some(Value(7)));

        let storage = world.resource::<ReplicationStorage>();
        let history = storage.get::<DiffHistory<Value>>(entity).unwrap();
        assert_eq!(history.diffs, [ValueDiff::Add(10), ValueDiff::Sub(3)]);
    }

    #[test]
    fn apply_resource_diff_command() {
        let mut world = World::new();
        world.init_resource::<ReplicationStorage>();

        let entity = world.spawn(Value::default()).id();
        let mut commands = world.commands();
        commands
            .apply_resource_diff::<Value>(ValueDiff::Add(10))
            .apply_resource_diff::<Value>(ValueDiff::Sub(3));

        world.flush();

        assert_eq!(*world.resource::<Value>(), Value(7));

        let storage = world.resource::<ReplicationStorage>();
        let history = storage.get::<DiffHistory<Value>>(entity).unwrap();
        assert_eq!(history.diffs, [ValueDiff::Add(10), ValueDiff::Sub(3)]);
    }

    #[test]
    fn apply_missing_component() {
        let mut world = World::new();
        world.init_resource::<ReplicationStorage>();

        let mut entity = world.spawn_empty();
        assert!(entity.apply_diff::<Value>(ValueDiff::Add(10)).is_err());
        assert!(!world.contains_resource::<Value>());
    }

    #[test]
    fn apply_missing_resource() {
        let mut world = World::new();
        world.init_resource::<ReplicationStorage>();

        assert!(
            world
                .apply_resource_diff::<Value>(ValueDiff::Add(10))
                .is_err()
        );
        assert!(!world.contains_resource::<Value>());
    }

    #[test]
    fn apply_with_external_mutation() {
        let mut world = World::new();
        world.init_resource::<ReplicationStorage>();

        let entity = world.spawn(Value::default()).id();

        world
            .apply_resource_diff::<Value>(ValueDiff::Add(10))
            .unwrap();

        world.increment_change_tick();

        let mut value = world.resource_mut::<Value>();
        assert_eq!(*value, Value(10));
        value.set_changed(); // Mock external change after diff application.

        world
            .apply_resource_diff::<Value>(ValueDiff::Sub(3))
            .unwrap();
        assert_eq!(*world.resource::<Value>(), Value(7));

        let storage = world.resource::<ReplicationStorage>();
        let history = storage.get::<DiffHistory<Value>>(entity).unwrap();
        assert!(
            history.diffs.is_empty(),
            "history should be cleared on external mutation"
        );
    }

    #[derive(Component, Serialize, Deserialize)]
    struct TooLongHistory;

    impl Diffable for TooLongHistory {
        const HISTORY_LEN: usize = u16::MAX as usize;
        type Diff = ();

        fn apply_diff(&mut self, _diff: &Self::Diff) -> Result<()> {
            Ok(())
        }
    }

    #[derive(Resource, Default, Deserialize, Serialize, PartialEq, Debug, Clone, Copy)]
    struct Value(u8);

    impl Diffable for Value {
        const HISTORY_LEN: usize = 3;
        type Diff = ValueDiff;

        fn apply_diff(&mut self, diff: &Self::Diff) -> Result<()> {
            match *diff {
                ValueDiff::Add(value) => self.0 += value,
                ValueDiff::Sub(value) => self.0 -= value,
            }

            Ok(())
        }
    }

    #[derive(Debug, Deserialize, Serialize, PartialEq, Clone, Copy)]
    enum ValueDiff {
        Add(u8),
        Sub(u8),
    }
}
