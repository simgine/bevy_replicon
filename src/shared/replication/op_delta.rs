//! Operation-based delta replication for components.
//!
//! Op-delta replication is useful when a component is large, but most changes can be
//! represented by a small semantic operation. A common example is a component that stores
//! a growing [`VecDeque`] of points for a trail/path. Sending the full
//! queue after every push can become expensive; sending an operation like
//! `PushBack(point)` or `PopFront(count)` only transmits the part that changed.
//!
//! The component remains the authoritative state. The user provides an operation type and
//! implements [`OpDeltaComponent::apply_op`] to describe how each operation changes the
//! component. When the server mutates the component through
//! [`OpDeltaEntityExt::apply_op_delta`], Replicon applies the operation locally and records
//! it in an [`OpDeltaLog`]. For each client, the server sends either the operations after
//! that client's latest acknowledged operation cursor, or a full snapshot if the needed
//! operations are no longer retained. On the receiver, operations are deduplicated,
//! buffered until they can be applied in order, and then applied to the local component.
//! Components can override [`OpDeltaComponent::MAX_RETAINED_OPS`] to tune how many
//! operations are kept before snapshot fallback becomes necessary.
//!
//! Direct component mutations are still supported for correctness, but they are not
//! recorded as operations and will be sent as a snapshot fallback.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::collections::VecDeque;
//!
//! use bevy::{prelude::*, state::app::StatesPlugin};
//! use bevy_replicon::prelude::*;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Clone, Copy, Deserialize, Serialize)]
//! struct Point {
//!     x: f32,
//!     y: f32,
//! }
//!
//! #[derive(Component, Deserialize, Serialize)]
//! struct Trail(VecDeque<Point>);
//!
//! #[derive(Clone, Deserialize, Serialize)]
//! enum TrailOp {
//!     PushBack(Point),
//!     PopFront(usize),
//! }
//!
//! impl OpDeltaComponent for Trail {
//!     type Op = TrailOp;
//!     const MAX_RETAINED_OPS: usize = 256;
//!
//!     fn apply_op(&mut self, op: &Self::Op) -> Result<()> {
//!         match *op {
//!             TrailOp::PushBack(point) => self.0.push_back(point),
//!             TrailOp::PopFront(count) => {
//!                 for _ in 0..count {
//!                     self.0.pop_front();
//!                 }
//!             }
//!         }
//!
//!         Ok(())
//!     }
//! }
//!
//! let mut app = App::new();
//! app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
//!     .replicate_op_delta::<Trail>()
//!     .finish();
//!
//! let entity = app
//!     .world_mut()
//!     .spawn((Replicated, Trail(VecDeque::new())))
//!     .id();
//!
//! let point = Point { x: 1.0, y: 2.0 };
//! let _ = app
//!     .world_mut()
//!     .entity_mut(entity)
//!     .apply_op_delta::<Trail>(TrailOp::PushBack(point));
//! ```

use alloc::{
    collections::{BTreeMap, VecDeque},
    format,
    vec::Vec,
};
use core::marker::PhantomData;

use bevy::{
    ecs::{
        component::{ComponentId, Mutable},
        world::EntityWorldMut,
    },
    prelude::*,
    ptr::Ptr,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

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

/// Monotonic index assigned to an op-delta operation.
pub type OpIndex = u64;

/// Component whose mutations can be represented as an ordered log of operations.
///
/// The component still stores the authoritative state. Operations are recorded in
/// [`OpDeltaLog`] only when the component is changed through [`OpDeltaEntityExt`].
pub trait OpDeltaComponent:
    Component<Mutability = Mutable> + Serialize + DeserializeOwned + Sized
{
    /// Operation that transforms this component from one state to the next.
    type Op: Clone + Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Maximum number of operations retained for delta serialization.
    ///
    /// If a client acknowledges an operation older than the retained range,
    /// Replicon will fall back to sending a full component snapshot.
    const MAX_RETAINED_OPS: usize = 64;

    /// Applies an operation to the component state.
    fn apply_op(&mut self, op: &Self::Op) -> Result<()>;
}

/// A single operation with its monotonic sequence number.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SequencedOp<Op> {
    pub seq: OpIndex,
    pub op: Op,
}

/// Operation log associated with an [`OpDeltaComponent`].
///
/// This component is registered as a required component for op-delta components.
/// It is not replicated directly.
#[derive(Component, Debug)]
pub struct OpDeltaLog<C: OpDeltaComponent> {
    last_seq: OpIndex,
    ops: VecDeque<SequencedOp<C::Op>>,
    _marker: PhantomData<fn() -> C>,
}

impl<C: OpDeltaComponent> OpDeltaLog<C> {
    /// Records an operation and returns the assigned sequence number.
    pub fn record(&mut self, op: C::Op) -> OpIndex {
        self.last_seq += 1;
        let seq = self.last_seq;
        self.ops.push_back(SequencedOp { seq, op });
        self.prune_to_limit();
        seq
    }

    /// Returns the latest operation sequence number.
    pub fn current_cursor(&self) -> OpIndex {
        self.last_seq
    }

    /// Returns all retained operations after `cursor`.
    ///
    /// Returns `None` if operations needed to continue from `cursor` were already
    /// pruned and the sender must fall back to a snapshot.
    pub fn ops_after(&self, cursor: OpIndex) -> Option<Vec<SequencedOp<C::Op>>> {
        if let Some(first) = self.ops.front()
            && cursor.saturating_add(1) < first.seq
        {
            return None;
        }

        Some(
            self.ops
                .iter()
                .filter(|op| op.seq > cursor)
                .cloned()
                .collect(),
        )
    }

    fn prune_to_limit(&mut self) {
        while self.ops.len() > C::MAX_RETAINED_OPS {
            self.ops.pop_front();
        }
    }
}

impl<C: OpDeltaComponent> Default for OpDeltaLog<C> {
    fn default() -> Self {
        Self {
            last_seq: 0,
            ops: Default::default(),
            _marker: PhantomData,
        }
    }
}

/// Receiver-side state for applying op deltas exactly once and in order.
#[derive(Component, Debug)]
pub struct OpDeltaReceiver<C: OpDeltaComponent> {
    last_applied: OpIndex,
    pending: BTreeMap<OpIndex, C::Op>,
}

impl<C: OpDeltaComponent> OpDeltaReceiver<C> {
    pub fn new(cursor: OpIndex) -> Self {
        Self {
            last_applied: cursor,
            pending: Default::default(),
        }
    }

    /// Queues newly received operations and returns operations that can be applied now.
    ///
    /// Operations must be applied sequentially by [`OpIndex`]. If an operation arrives
    /// ahead of a missing predecessor, it stays pending until the missing operation is
    /// received. Duplicate or already-applied operations are ignored.
    pub fn queue_and_take_ready(&mut self, ops: Vec<SequencedOp<C::Op>>) -> Vec<C::Op> {
        for SequencedOp { seq, op } in ops {
            if seq > self.last_applied {
                self.pending.entry(seq).or_insert(op);
            }
        }

        let mut ready = Vec::new();
        while let Some(op) = self.pending.remove(&(self.last_applied + 1)) {
            self.last_applied += 1;
            ready.push(op);
        }

        ready
    }
}

impl<C: OpDeltaComponent> Default for OpDeltaReceiver<C> {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Wire format for op-delta replicated components.
#[derive(Deserialize, Serialize)]
pub enum OpDeltaWire<C, Op> {
    Snapshot {
        cursor: OpIndex,
        value: C,
    },
    Ops {
        base_cursor: OpIndex,
        cursor: OpIndex,
        ops: Vec<SequencedOp<Op>>,
    },
}

#[derive(Serialize)]
enum OpDeltaWireRef<'a, C, Op> {
    Snapshot {
        cursor: OpIndex,
        value: &'a C,
    },
    Ops {
        base_cursor: OpIndex,
        cursor: OpIndex,
        ops: &'a [SequencedOp<Op>],
    },
}

/// Extension trait for recording op-delta mutations on an entity.
pub trait OpDeltaEntityExt {
    /// Applies `op` to component `C` and records it in the entity's [`OpDeltaLog`].
    fn apply_op_delta<C: OpDeltaComponent>(&mut self, op: C::Op) -> Result<()>;
}

impl OpDeltaEntityExt for EntityWorldMut<'_> {
    fn apply_op_delta<C: OpDeltaComponent>(&mut self, op: C::Op) -> Result<()> {
        let entity = self.id();
        {
            let mut component = self.get_mut::<C>().ok_or_else(|| {
                format!("entity `{entity}` is missing `{}`", ShortName::of::<C>())
            })?;
            component.apply_op(&op)?;
        }

        let mut log = self.get_mut::<OpDeltaLog<C>>().ok_or_else(|| {
            format!(
                "entity `{}` is missing `{}`; register `{}` with `replicate_op_delta`",
                entity,
                ShortName::of::<OpDeltaLog<C>>(),
                ShortName::of::<C>(),
            )
        })?;
        log.record(op);

        Ok(())
    }
}

/// Type-erased functions for op-delta serialization.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpDeltaFns {
    pub(crate) log_component_id: ComponentId,
    serialize_snapshot: unsafe fn(&SerializeCtx, Ptr, Ptr, &mut Vec<u8>) -> Result<OpIndex>,
    serialize_mutation:
        unsafe fn(&SerializeCtx, Ptr, Ptr, OpIndex, &mut Vec<u8>) -> Result<OpIndex>,
}

impl OpDeltaFns {
    pub(crate) fn new<C: OpDeltaComponent>(world: &mut World) -> Self {
        Self {
            log_component_id: world.register_component::<OpDeltaLog<C>>(),
            serialize_snapshot: serialize_snapshot::<C>,
            serialize_mutation: serialize_mutation::<C>,
        }
    }

    /// Serializes a full snapshot and returns the cursor represented by it.
    ///
    /// # Safety
    ///
    /// `component` must point to `C`, and `log` must point to `OpDeltaLog<C>`.
    pub(crate) unsafe fn serialize_snapshot(
        &self,
        ctx: &SerializeCtx,
        component: Ptr,
        log: Ptr,
        message: &mut Vec<u8>,
    ) -> Result<OpIndex> {
        unsafe { (self.serialize_snapshot)(ctx, component, log, message) }
    }

    /// Serializes operations after `acked_cursor`, or a snapshot if required.
    ///
    /// # Safety
    ///
    /// `component` must point to `C`, and `log` must point to `OpDeltaLog<C>`.
    pub(crate) unsafe fn serialize_mutation(
        &self,
        ctx: &SerializeCtx,
        component: Ptr,
        log: Ptr,
        acked_cursor: OpIndex,
        message: &mut Vec<u8>,
    ) -> Result<OpIndex> {
        unsafe { (self.serialize_mutation)(ctx, component, log, acked_cursor, message) }
    }
}

unsafe fn serialize_snapshot<C: OpDeltaComponent>(
    _ctx: &SerializeCtx,
    component: Ptr,
    log: Ptr,
    message: &mut Vec<u8>,
) -> Result<OpIndex> {
    let component = unsafe { component.deref::<C>() };
    let log = unsafe { log.deref::<OpDeltaLog<C>>() };
    let cursor = log.current_cursor();
    let wire: OpDeltaWireRef<'_, C, C::Op> = OpDeltaWireRef::Snapshot {
        cursor,
        value: component,
    };
    postcard_utils::to_extend_mut(&wire, message)?;
    Ok(cursor)
}

unsafe fn serialize_mutation<C: OpDeltaComponent>(
    _ctx: &SerializeCtx,
    component: Ptr,
    log: Ptr,
    acked_cursor: OpIndex,
    message: &mut Vec<u8>,
) -> Result<OpIndex> {
    let component = unsafe { component.deref::<C>() };
    let log = unsafe { log.deref::<OpDeltaLog<C>>() };
    let cursor = log.current_cursor();

    match log.ops_after(acked_cursor) {
        Some(ops) if ops.is_empty() => {
            let wire: OpDeltaWireRef<'_, C, C::Op> = OpDeltaWireRef::Snapshot {
                cursor,
                value: component,
            };
            postcard_utils::to_extend_mut(&wire, message)?;
        }
        Some(ops) => {
            let wire: OpDeltaWireRef<'_, C, C::Op> = OpDeltaWireRef::Ops {
                base_cursor: acked_cursor,
                cursor,
                ops: &ops,
            };
            postcard_utils::to_extend_mut(&wire, message)?;
        }
        None => {
            let wire: OpDeltaWireRef<'_, C, C::Op> = OpDeltaWireRef::Snapshot {
                cursor,
                value: component,
            };
            postcard_utils::to_extend_mut(&wire, message)?;
        }
    }

    Ok(cursor)
}

pub(crate) fn serialize_without_log<C: OpDeltaComponent>(
    _ctx: &SerializeCtx,
    component: &C,
    message: &mut Vec<u8>,
) -> Result<()> {
    let wire: OpDeltaWireRef<'_, C, C::Op> = OpDeltaWireRef::Snapshot {
        cursor: 0,
        value: component,
    };
    postcard_utils::to_extend_mut(&wire, message)?;
    Ok(())
}

pub(crate) fn deserialize_snapshot<C: OpDeltaComponent>(
    _ctx: &mut WriteCtx,
    message: &mut Bytes,
) -> Result<C> {
    match postcard_utils::from_buf(message)? {
        OpDeltaWire::<C, C::Op>::Snapshot { value, .. } => Ok(value),
        OpDeltaWire::<C, C::Op>::Ops { .. } => Err(format!(
            "cannot deserialize op-delta ops into `{}`",
            ShortName::of::<C>()
        )
        .into()),
    }
}

/// Consumes an op-delta payload without applying it.
///
/// This is used for stale mutation messages when a receive marker requests
/// history for some components on the entity but not this component. In that
/// path Replicon still has to advance through every component payload in the
/// mutation message. The default consume implementation deserializes a `C`,
/// but op-delta mutation payloads may contain [`OpDeltaWire::Ops`], which is
/// not a standalone component value. Parsing and dropping the full wire format
/// lets us skip both snapshots and ops safely.
pub(crate) fn consume<C: OpDeltaComponent>(
    _deserialize: DeserializeFn<C>,
    _ctx: &mut WriteCtx,
    message: &mut Bytes,
) -> Result<()> {
    let _wire: OpDeltaWire<C, C::Op> = postcard_utils::from_buf(message)?;
    Ok(())
}

pub(crate) fn write<C: OpDeltaComponent>(
    _ctx: &mut WriteCtx,
    _rule_fns: &RuleFns<C>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    let wire: OpDeltaWire<C, C::Op> = postcard_utils::from_buf(message)?;

    match wire {
        OpDeltaWire::Snapshot { cursor, value } => {
            if let Some(mut component) = entity.get_mut::<C>() {
                *component = value;
            } else {
                entity.insert(value);
            }
            entity.insert(OpDeltaReceiver::<C>::new(cursor));
        }
        OpDeltaWire::Ops { ops, .. } => {
            let ready_ops = {
                let mut receiver = entity.get_mut::<OpDeltaReceiver<C>>().ok_or_else(|| {
                    format!(
                        "received op-delta operations for `{}` before a snapshot",
                        ShortName::of::<C>()
                    )
                })?;
                receiver.queue_and_take_ready(ops)
            };

            let mut component = entity.get_mut::<C>().ok_or_else(|| {
                format!(
                    "received op-delta operations for missing `{}`",
                    ShortName::of::<C>()
                )
            })?;
            for op in ready_ops {
                component.apply_op(&op)?;
            }
        }
    }

    Ok(())
}

pub(crate) fn remove<C: OpDeltaComponent>(_ctx: &mut RemoveCtx, entity: &mut DeferredEntity) {
    entity
        .remove::<C>()
        .remove::<OpDeltaLog<C>>()
        .remove::<OpDeltaReceiver<C>>();
}
