use core::{any::TypeId, mem};

use bevy::prelude::*;
use bytes::Bytes;
use serde::{Serialize, de::DeserializeOwned};

use super::ctx::{SerializeCtx, WriteCtx};
use crate::{
    postcard_utils,
    prelude::*,
    shared::replication::diff::{ComponentDelta, ComponentDeltaRef, DiffBuffer, DiffHistory},
};

/// Type-erased version of [`RuleFns`].
///
/// Stored inside [`ReplicationRegistry`](super::ReplicationRegistry) after registration.
pub(crate) struct UntypedRuleFns {
    type_id: TypeId,
    type_name: ShortName<'static>,

    serialize: unsafe fn(),
    deserialize: unsafe fn(),
    deserialize_in_place: unsafe fn(),
    consume: unsafe fn(),
}

impl UntypedRuleFns {
    /// Restores the original [`RuleFns`] from which this type was created.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same `C` with which this instance was created.
    pub(super) unsafe fn typed<C: Component>(&self) -> RuleFns<C> {
        debug_assert_eq!(
            self.type_id,
            TypeId::of::<C>(),
            "trying to call rule functions with `{}`, but they were created with `{}`",
            ShortName::of::<C>(),
            self.type_name,
        );

        RuleFns {
            serialize: unsafe { mem::transmute::<unsafe fn(), SerializeFn<C>>(self.serialize) },
            deserialize: unsafe {
                mem::transmute::<unsafe fn(), DeserializeFn<C>>(self.deserialize)
            },
            deserialize_in_place: unsafe {
                mem::transmute::<unsafe fn(), DeserializeInPlaceFn<C>>(self.deserialize_in_place)
            },
            consume: unsafe { mem::transmute::<unsafe fn(), ConsumeFn<C>>(self.consume) },
        }
    }
}

impl<C: Component> From<RuleFns<C>> for UntypedRuleFns {
    fn from(value: RuleFns<C>) -> Self {
        // SAFETY: these functions won't be called until the type is restored.
        Self {
            type_id: TypeId::of::<C>(),
            type_name: ShortName::of::<C>(),
            serialize: unsafe { mem::transmute::<SerializeFn<C>, unsafe fn()>(value.serialize) },
            deserialize: unsafe {
                mem::transmute::<DeserializeFn<C>, unsafe fn()>(value.deserialize)
            },
            deserialize_in_place: unsafe {
                mem::transmute::<DeserializeInPlaceFn<C>, unsafe fn()>(value.deserialize_in_place)
            },
            consume: unsafe { mem::transmute::<ConsumeFn<C>, unsafe fn()>(value.consume) },
        }
    }
}

/// Serialization and deserialization functions for a component.
///
/// See also [`AppRuleExt`]
/// and [`ReplicationRule`](crate::shared::replication::rules::ReplicationRule).
pub struct RuleFns<C> {
    serialize: SerializeFn<C>,
    deserialize: DeserializeFn<C>,
    deserialize_in_place: DeserializeInPlaceFn<C>,
    consume: ConsumeFn<C>,
}

impl<C: Component> RuleFns<C> {
    /// Creates a new instance.
    ///
    /// For more details see [`AppRuleExt::replicate_with`].
    pub fn new(serialize: SerializeFn<C>, deserialize: DeserializeFn<C>) -> Self {
        Self {
            serialize,
            deserialize,
            deserialize_in_place: in_place_as_deserialize::<C>,
            consume: consume_as_deserialize,
        }
    }

    /// Like [`Self::default`], but converts the component into `T` before serialization
    /// and back into `C` after deserialization.
    ///
    /// For more details see [`AppRuleExt::replicate_as`].
    pub fn new_as<T>() -> Self
    where
        T: Serialize + DeserializeOwned,
        C: Clone + Into<T> + From<T>,
    {
        Self::new(serialize_as, deserialize_as)
    }

    /// Replaces default [`in_place_as_deserialize`] with a custom function.
    ///
    /// This function will be called when a component is already present on an entity.
    /// For insertion [`Self::deserialize`] will be called instead.
    pub fn with_in_place(mut self, deserialize_in_place: DeserializeInPlaceFn<C>) -> Self {
        self.deserialize_in_place = deserialize_in_place;
        self
    }

    /// Replaces the default [`consume_as_deserialize`] with a custom function.
    ///
    /// This function will be called to handle stale component updates for entities
    /// with a marker that indicates the entity's history should be consumed instead of discarded.
    ///
    /// If no markers on an entity request history, then stale updates will be skipped entirely
    /// by just advancing the cursor (without calling any consume functions).
    ///
    /// If you want to ignore a component, just use its expected size to advance the cursor
    /// without deserializing (but be careful if the component is dynamically sized).
    ///
    /// See [`MarkerConfig::need_history`](crate::shared::replication::receive_markers::MarkerConfig::need_history)
    /// for details.
    pub fn with_consume(mut self, consume: ConsumeFn<C>) -> Self {
        self.consume = consume;
        self
    }

    /// Serializes a component into a message.
    pub(super) fn serialize(
        &self,
        ctx: &mut SerializeCtx,
        component: &C,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        (self.serialize)(ctx, component, message)
    }

    /// Deserializes a component from a message.
    ///
    /// Use this function when inserting a new component.
    pub fn deserialize(&self, ctx: &mut WriteCtx, message: &mut Bytes) -> Result<C> {
        (self.deserialize)(ctx, message)
    }

    /// Same as [`Self::deserialize`], but instead of returning a component, it updates the passed reference.
    ///
    /// Use this function for updating an existing component.
    pub fn deserialize_in_place(
        &self,
        ctx: &mut WriteCtx,
        component: &mut C,
        message: &mut Bytes,
    ) -> Result<()> {
        (self.deserialize_in_place)(self.deserialize, ctx, component, message)
    }

    /// Consumes a component from a message.
    pub(super) fn consume(&self, ctx: &mut WriteCtx, message: &mut Bytes) -> Result<()> {
        (self.consume)(self.deserialize, ctx, message)
    }
}

impl<C: Diffable> RuleFns<C> {
    /// Creates a new instance for diff-based replication.
    pub fn new_diff() -> Self {
        Self::new(serialize_diff::<C>, deserialize_diff::<C>)
            .with_in_place(deserialize_diff_in_place)
    }
}

impl<C: Component + Serialize + DeserializeOwned> Default for RuleFns<C> {
    /// Creates a new instance with default functions for a component.
    ///
    /// See also [`default_serialize`], [`default_deserialize`] and [`in_place_as_deserialize`].
    fn default() -> Self {
        Self::new(default_serialize::<C>, default_deserialize::<C>)
    }
}

/// Signature of component serialization functions.
pub type SerializeFn<C> = fn(&mut SerializeCtx, &C, &mut Vec<u8>) -> Result<()>;

/// Signature of component deserialization functions.
pub type DeserializeFn<C> = fn(&mut WriteCtx, &mut Bytes) -> Result<C>;

/// Signature of component in-place deserialization functions.
pub type DeserializeInPlaceFn<C> =
    fn(DeserializeFn<C>, &mut WriteCtx, &mut C, &mut Bytes) -> Result<()>;

/// Signature of component consume functions.
pub type ConsumeFn<C> = fn(DeserializeFn<C>, &mut WriteCtx, &mut Bytes) -> Result<()>;

/// Default component serialization function.
pub fn default_serialize<C: Component + Serialize>(
    _ctx: &mut SerializeCtx,
    component: &C,
    message: &mut Vec<u8>,
) -> Result<()> {
    postcard_utils::to_extend_mut(component, message)?;
    Ok(())
}

/// Default component deserialization function.
pub fn default_deserialize<C: Component + DeserializeOwned>(
    ctx: &mut WriteCtx,
    message: &mut Bytes,
) -> Result<C> {
    let mut component: C = postcard_utils::from_buf(message)?;
    C::map_entities(&mut component, ctx);
    Ok(component)
}

/// Default component in-place deserialization function.
///
/// This implementation just assigns the value from the passed deserialization function.
pub fn in_place_as_deserialize<C: Component>(
    deserialize: DeserializeFn<C>,
    ctx: &mut WriteCtx,
    component: &mut C,
    message: &mut Bytes,
) -> Result<()> {
    *component = (deserialize)(ctx, message)?;
    Ok(())
}

/// Default component consume function.
///
/// This implementation just calls deserialization function and ignores its result.
pub fn consume_as_deserialize<C: Component>(
    deserialize: DeserializeFn<C>,
    ctx: &mut WriteCtx,
    message: &mut Bytes,
) -> Result<()> {
    ctx.ignore_mapping = true;
    (deserialize)(ctx, message)?;
    ctx.ignore_mapping = false;
    Ok(())
}

/// Converts `C` into `T` and serializes it.
pub fn serialize_as<C: Component + Clone + Into<T>, T: Serialize>(
    _ctx: &mut SerializeCtx,
    component: &C,
    message: &mut Vec<u8>,
) -> Result<()> {
    let serializable = component.clone().into();
    postcard_utils::to_extend_mut(&serializable, message)?;
    Ok(())
}

/// Deserializes `T` and converts it into `C`.
pub fn deserialize_as<C: Component + From<T>, T: DeserializeOwned>(
    ctx: &mut WriteCtx,
    message: &mut Bytes,
) -> Result<C> {
    let deserialized: T = postcard_utils::from_buf(message)?;
    let mut component = deserialized.into();
    C::map_entities(&mut component, ctx);
    Ok(component)
}

/// Serializes a component diff.
pub fn serialize_diff<C: Diffable>(
    ctx: &mut SerializeCtx,
    component: &C,
    message: &mut Vec<u8>,
) -> Result<()> {
    let last_changed = ctx.last_changed;
    let diff_cursor = ctx.diff_cursor;
    let history = ctx.get_or_default::<DiffHistory<C>>();

    let (index, diffs) = history.diffs_after(diff_cursor, last_changed);
    let delta = if diffs.len() == 0 {
        ComponentDeltaRef::Snapshot { index, component }
    } else {
        ComponentDeltaRef::Diffs { index, diffs }
    };

    postcard_utils::to_extend_mut(&delta, message)?;

    ctx.diff_cursor = Some(index);

    Ok(())
}

/// Deserializes a component diff.
///
/// Deserializes only snapshots because it's called only when the component is missing.
pub fn deserialize_diff<C: Diffable>(ctx: &mut WriteCtx, message: &mut Bytes) -> Result<C> {
    match postcard_utils::from_buf(message)? {
        ComponentDelta::Snapshot {
            index,
            mut component,
        } => {
            let buffer = ctx.get_or_default::<DiffBuffer<C>>();
            buffer.set_last_applied(index);

            C::map_entities(&mut component, ctx);
            Ok(component)
        }
        ComponentDelta::Diffs { .. } => Err(format!(
            "cannot apply diffs to `{}` that is not present on the entity",
            ShortName::of::<C>()
        )
        .into()),
    }
}
/// Deserializes a component diff and applies it to the passed component.
///
/// Snapshots replace the current component value and reset the diff buffer to
/// the snapshot's diff cursor. Diffs are buffered and applied only when
/// all preceding diffs have been received, ensuring diffs are applied once
/// and in order.
pub fn deserialize_diff_in_place<C: Diffable>(
    _deserialize: DeserializeFn<C>,
    ctx: &mut WriteCtx,
    component: &mut C,
    message: &mut Bytes,
) -> Result<()> {
    match postcard_utils::from_buf(message)? {
        ComponentDelta::<C>::Snapshot {
            index,
            component: new_component,
        } => {
            let buffer = ctx.get_or_default::<DiffBuffer<C>>();
            buffer.set_last_applied(index);

            *component = new_component;
            C::map_entities(component, ctx);
        }
        ComponentDelta::<C>::Diffs { index, diffs } => {
            let buffer = ctx.get_or_default::<DiffBuffer<C>>();
            buffer.push(index, diffs);
            for diff in buffer.drain_ready() {
                component.apply_diff(&diff)?;
            }
        }
    }
    Ok(())
}
