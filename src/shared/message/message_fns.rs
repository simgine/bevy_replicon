use alloc::vec::Vec;
use core::{any::TypeId, mem};

use bevy::prelude::*;
use bytes::Bytes;

/// Type-erased version of [`MessageFns`].
///
/// Stored inside messages after their creation.
#[derive(Clone, Copy)]
pub(super) struct UntypedMessageFns {
    serialize_ctx_id: TypeId,
    serialize_ctx_name: ShortName<'static>,
    deserialize_ctx_id: TypeId,
    deserialize_ctx_name: ShortName<'static>,
    message_id: TypeId,
    message_name: ShortName<'static>,
    inner_id: TypeId,
    inner_name: ShortName<'static>,

    serialize_adapter: unsafe fn(),
    deserialize_adapter: unsafe fn(),
    serialize: unsafe fn(),
    deserialize: unsafe fn(),
}

impl UntypedMessageFns {
    /// Restores the original [`MessageFns`] from which this type was created.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same generics with which this instance was created.
    pub(super) unsafe fn typed<S, D, M: 'static, I: 'static>(self) -> MessageFns<S, D, M, I> {
        // `TypeId` can only be obtained for `'static` types, but we can't impose this requirement because our context has a lifetime.
        // So, we use the `typeid` crate for non-static `TypeId`, as we don't care about the lifetime and only need to check the type.
        // This crate is already used by `erased_serde`, so we don't add an extra dependency.
        debug_assert_eq!(
            self.serialize_ctx_id,
            typeid::of::<S>(),
            "trying to call message functions with serialize context `{}`, but they were created with `{}`",
            ShortName::of::<S>(),
            self.serialize_ctx_name,
        );
        debug_assert_eq!(
            self.deserialize_ctx_id,
            typeid::of::<D>(),
            "trying to call message functions with deserialize context `{}`, but they were created with `{}`",
            ShortName::of::<D>(),
            self.deserialize_ctx_name,
        );
        debug_assert_eq!(
            self.message_id,
            TypeId::of::<M>(),
            "trying to call message functions with message `{}`, but they were created with `{}`",
            ShortName::of::<M>(),
            self.message_name,
        );
        debug_assert_eq!(
            self.inner_id,
            TypeId::of::<I>(),
            "trying to call message functions with inner type `{}`, but they were created with `{}`",
            ShortName::of::<I>(),
            self.inner_name,
        );

        MessageFns {
            serialize_adapter: unsafe {
                mem::transmute::<unsafe fn(), AdapterSerializeFn<S, M, I>>(self.serialize_adapter)
            },
            deserialize_adapter: unsafe {
                mem::transmute::<unsafe fn(), AdapterDeserializeFn<D, M, I>>(
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

impl<S, D, M: 'static, I: 'static> From<MessageFns<S, D, M, I>> for UntypedMessageFns {
    fn from(value: MessageFns<S, D, M, I>) -> Self {
        // SAFETY: these functions won't be called until the type is restored.
        Self {
            serialize_ctx_id: typeid::of::<S>(),
            serialize_ctx_name: ShortName::of::<S>(),
            deserialize_ctx_id: typeid::of::<D>(),
            deserialize_ctx_name: ShortName::of::<D>(),
            message_id: TypeId::of::<M>(),
            message_name: ShortName::of::<M>(),
            inner_id: TypeId::of::<I>(),
            inner_name: ShortName::of::<I>(),
            serialize_adapter: unsafe {
                mem::transmute::<AdapterSerializeFn<S, M, I>, unsafe fn()>(value.serialize_adapter)
            },
            deserialize_adapter: unsafe {
                mem::transmute::<AdapterDeserializeFn<D, M, I>, unsafe fn()>(
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

/// Serialization and deserialization functions for a message.
///
/// For events, we want to allow users to customize these functions, but it would be inconvenient
/// to write serialization and deserialization logic for the event adapter instead of the actual type.
/// Since closures can't be used, we provide adapter functions that accept regular serialization functions.
/// By default, these adapter functions simply call the passed function, but they can be overridden
/// to perform the type conversion.
pub(super) struct MessageFns<S, D, M, I = M> {
    serialize_adapter: AdapterSerializeFn<S, M, I>,
    deserialize_adapter: AdapterDeserializeFn<D, M, I>,
    serialize: SerializeFn<S, I>,
    deserialize: DeserializeFn<D, I>,
}

impl<S, D, M> MessageFns<S, D, M, M> {
    /// Creates a new instance with default adapter functions.
    pub(super) fn new(serialize: SerializeFn<S, M>, deserialize: DeserializeFn<D, M>) -> Self {
        Self {
            serialize_adapter: default_serialize_adapter::<S, M>,
            deserialize_adapter: default_deserialize_adapter::<D, M>,
            serialize,
            deserialize,
        }
    }
}

impl<S, D, M, I> MessageFns<S, D, M, I> {
    /// Adds conversion to type `T` before serialization and after deserialiation.
    pub(super) fn with_convert<T: AsRef<I> + From<I>>(self) -> MessageFns<S, D, T, I> {
        MessageFns {
            serialize_adapter: convert_serialize_adapter,
            deserialize_adapter: convert_deserialize_adapter,
            serialize: self.serialize,
            deserialize: self.deserialize,
        }
    }

    pub(super) fn serialize(
        self,
        ctx: &mut S,
        message: &M,
        message_bytes: &mut Vec<u8>,
    ) -> Result<()> {
        (self.serialize_adapter)(ctx, message, message_bytes, self.serialize)
    }

    pub(super) fn deserialize(self, ctx: &mut D, message: &mut Bytes) -> Result<M> {
        (self.deserialize_adapter)(ctx, message, self.deserialize)
    }
}

fn default_serialize_adapter<C, M>(
    ctx: &mut C,
    message: &M,
    message_bytes: &mut Vec<u8>,
    serialize: SerializeFn<C, M>,
) -> Result<()> {
    (serialize)(ctx, message, message_bytes)
}

fn default_deserialize_adapter<C, M>(
    ctx: &mut C,
    message: &mut Bytes,
    deserialize: DeserializeFn<C, M>,
) -> Result<M> {
    (deserialize)(ctx, message)
}

fn convert_serialize_adapter<C, M: AsRef<I>, I>(
    ctx: &mut C,
    message: &M,
    message_bytes: &mut Vec<u8>,
    serialize: SerializeFn<C, I>,
) -> Result<()> {
    (serialize)(ctx, message.as_ref(), message_bytes)
}

fn convert_deserialize_adapter<C, M: From<I>, I>(
    ctx: &mut C,
    message: &mut Bytes,
    deserialize: DeserializeFn<C, I>,
) -> Result<M> {
    let message = (deserialize)(ctx, message)?;
    Ok(M::from(message))
}

/// Signature of message serialization functions.
pub type SerializeFn<C, M> = fn(&mut C, &M, &mut Vec<u8>) -> Result<()>;

/// Signature of message deserialization functions.
pub type DeserializeFn<C, M> = fn(&mut C, &mut Bytes) -> Result<M>;

/// Signature of message adapter serialization functions.
pub(super) type AdapterSerializeFn<C, M, I> =
    fn(&mut C, &M, &mut Vec<u8>, SerializeFn<C, I>) -> Result<()>;

/// Signature of message adapter deserialization functions.
pub(super) type AdapterDeserializeFn<C, M, I> =
    fn(&mut C, &mut Bytes, DeserializeFn<C, I>) -> Result<M>;
