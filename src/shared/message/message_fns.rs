use alloc::vec::Vec;
use core::{
    any::{self, TypeId},
    mem,
};

use bevy::prelude::*;
use bytes::Bytes;

/// Type-erased version of [`EventFns`].
///
/// Stored inside events after their creation.
#[derive(Clone, Copy)]
pub(super) struct UntypedEventFns {
    serialize_ctx_id: TypeId,
    serialize_ctx_name: &'static str,
    deserialize_ctx_id: TypeId,
    deserialize_ctx_name: &'static str,
    event_id: TypeId,
    event_name: &'static str,
    inner_id: TypeId,
    inner_name: &'static str,

    serialize_adapter: unsafe fn(),
    deserialize_adapter: unsafe fn(),
    serialize: unsafe fn(),
    deserialize: unsafe fn(),
}

impl UntypedEventFns {
    /// Restores the original [`EventFns`] from which this type was created.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same generics with which this instance was created.
    pub(super) unsafe fn typed<S, D, E: 'static, I: 'static>(self) -> EventFns<S, D, E, I> {
        // `TypeId` can only be obtained for `'static` types, but we can't impose this requirement because our context has a lifetime.
        // So, we use the `typeid` crate for non-static `TypeId`, as we don't care about the lifetime and only need to check the type.
        // This crate is already used by `erased_serde`, so we don't add an extra dependency.
        debug_assert_eq!(
            self.serialize_ctx_id,
            typeid::of::<S>(),
            "trying to call event functions with serialize context `{}`, but they were created with `{}`",
            any::type_name::<S>(),
            self.serialize_ctx_name,
        );
        debug_assert_eq!(
            self.deserialize_ctx_id,
            typeid::of::<D>(),
            "trying to call event functions with deserialize context `{}`, but they were created with `{}`",
            any::type_name::<D>(),
            self.deserialize_ctx_name,
        );
        debug_assert_eq!(
            self.event_id,
            TypeId::of::<E>(),
            "trying to call event functions with event `{}`, but they were created with `{}`",
            any::type_name::<E>(),
            self.event_name,
        );
        debug_assert_eq!(
            self.inner_id,
            TypeId::of::<I>(),
            "trying to call event functions with inner type `{}`, but they were created with `{}`",
            any::type_name::<I>(),
            self.inner_name,
        );

        EventFns {
            serialize_adapter: unsafe {
                mem::transmute::<unsafe fn(), AdapterSerializeFn<S, E, I>>(self.serialize_adapter)
            },
            deserialize_adapter: unsafe {
                mem::transmute::<unsafe fn(), AdapterDeserializeFn<D, E, I>>(
                    self.deserialize_adapter,
                )
            },
            serialize: unsafe { mem::transmute::<unsafe fn(), SerializeFn<S, I>>(self.serialize) },
            deserialize: unsafe {
                mem::transmute::<unsafe fn(), DeserializeFn<D, I>>(self.deserialize)
            },
        }
    }
}

impl<S, D, E: 'static, I: 'static> From<EventFns<S, D, E, I>> for UntypedEventFns {
    fn from(value: EventFns<S, D, E, I>) -> Self {
        // SAFETY: these functions won't be called until the type is restored.
        Self {
            serialize_ctx_id: typeid::of::<S>(),
            serialize_ctx_name: any::type_name::<S>(),
            deserialize_ctx_id: typeid::of::<D>(),
            deserialize_ctx_name: any::type_name::<D>(),
            event_id: TypeId::of::<E>(),
            event_name: any::type_name::<E>(),
            inner_id: TypeId::of::<I>(),
            inner_name: any::type_name::<I>(),
            serialize_adapter: unsafe {
                mem::transmute::<AdapterSerializeFn<S, E, I>, unsafe fn()>(value.serialize_adapter)
            },
            deserialize_adapter: unsafe {
                mem::transmute::<AdapterDeserializeFn<D, E, I>, unsafe fn()>(
                    value.deserialize_adapter,
                )
            },
            serialize: unsafe { mem::transmute::<SerializeFn<S, I>, unsafe fn()>(value.serialize) },
            deserialize: unsafe {
                mem::transmute::<DeserializeFn<D, I>, unsafe fn()>(value.deserialize)
            },
        }
    }
}

/// Serialization and deserialization functions for an event.
///
/// For triggers, we want to allow users to customize these functions, but it would be inconvenient
/// to write serialization and deserialization logic for the trigger adapter instead of the actual type.
/// Since closures can't be used, we provide adapter functions that accept regular serialization functions.
/// By default, these adapter functions simply call the passed function, but they can be overridden
/// to perform the type conversion.
pub(super) struct EventFns<S, D, E, I = E> {
    serialize_adapter: AdapterSerializeFn<S, E, I>,
    deserialize_adapter: AdapterDeserializeFn<D, E, I>,
    serialize: SerializeFn<S, I>,
    deserialize: DeserializeFn<D, I>,
}

impl<S, D, E> EventFns<S, D, E, E> {
    /// Creates a new instance with default adapter functions.
    pub(super) fn new(serialize: SerializeFn<S, E>, deserialize: DeserializeFn<D, E>) -> Self {
        Self {
            serialize_adapter: default_serialize_adapter::<S, E>,
            deserialize_adapter: default_deserialize_adapter::<D, E>,
            serialize,
            deserialize,
        }
    }
}

impl<S, D, E, I> EventFns<S, D, E, I> {
    /// Adds conversion to type `T` before serialization and after deserialiation.
    pub(super) fn with_convert<T: AsRef<I> + From<I>>(self) -> EventFns<S, D, T, I> {
        EventFns {
            serialize_adapter: convert_serialize_adapter,
            deserialize_adapter: convert_deserialize_adapter,
            serialize: self.serialize,
            deserialize: self.deserialize,
        }
    }

    pub(super) fn serialize(self, ctx: &mut S, event: &E, message: &mut Vec<u8>) -> Result<()> {
        (self.serialize_adapter)(ctx, event, message, self.serialize)
    }

    pub(super) fn deserialize(self, ctx: &mut D, message: &mut Bytes) -> Result<E> {
        (self.deserialize_adapter)(ctx, message, self.deserialize)
    }
}

fn default_serialize_adapter<C, E>(
    ctx: &mut C,
    event: &E,
    message: &mut Vec<u8>,
    serialize: SerializeFn<C, E>,
) -> Result<()> {
    (serialize)(ctx, event, message)
}

fn default_deserialize_adapter<C, E>(
    ctx: &mut C,
    message: &mut Bytes,
    deserialize: DeserializeFn<C, E>,
) -> Result<E> {
    (deserialize)(ctx, message)
}

fn convert_serialize_adapter<C, E: AsRef<I>, I>(
    ctx: &mut C,
    event: &E,
    message: &mut Vec<u8>,
    serialize: SerializeFn<C, I>,
) -> Result<()> {
    (serialize)(ctx, event.as_ref(), message)
}

fn convert_deserialize_adapter<C, E: From<I>, I>(
    ctx: &mut C,
    message: &mut Bytes,
    deserialize: DeserializeFn<C, I>,
) -> Result<E> {
    let event = (deserialize)(ctx, message)?;
    Ok(E::from(event))
}

/// Signature of event serialization functions.
pub type SerializeFn<C, E> = fn(&mut C, &E, &mut Vec<u8>) -> Result<()>;

/// Signature of event deserialization functions.
pub type DeserializeFn<C, E> = fn(&mut C, &mut Bytes) -> Result<E>;

/// Signature of adapter serialization functions.
pub(super) type AdapterSerializeFn<C, E, I> =
    fn(&mut C, &E, &mut Vec<u8>, SerializeFn<C, I>) -> Result<()>;

/// Signature of adapter deserialization functions.
pub(super) type AdapterDeserializeFn<C, E, I> =
    fn(&mut C, &mut Bytes, DeserializeFn<C, I>) -> Result<E>;
