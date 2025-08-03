//! Extensions for postcard to make streaming serialization and deserialiation more ergonomic.

use alloc::slice;

use bevy::prelude::*;
use bytes::Buf;
use postcard::{
    Deserializer, Serializer, de_flavors::Flavor as DeFlavor, ser_flavors::Flavor as SerFlavor,
};
use serde::{Deserialize, Serialize};

use crate::compact_entity;

// TODO: replace with https://github.com/jamesmunns/postcard/pull/210 after release.
/// Serializes a value to an [`Extend`] writer.
///
/// Similar to [`postcard::to_extend`], but it takes the writer by reference instead of by value
/// to remain in control of the writer.
///
/// See also [`from_buf`] and [`entity_to_extend_mut`].
///
/// # Examples
///
/// ```
/// use bevy::prelude::*;
/// use bevy_replicon::postcard_utils;
///
/// let transform = Transform::default();
/// let mut message = Vec::new();
/// postcard_utils::to_extend_mut(&transform, &mut message).unwrap();
/// assert!(!message.is_empty());
/// ```
pub fn to_extend_mut<T: Serialize + ?Sized, W: Extend<u8>>(
    value: &T,
    writer: &mut W,
) -> postcard::Result<()> {
    postcard::serialize_with_flavor(value, ExtendMutFlavor::new(writer))
}

/// Like [`to_extend_mut`], but uses [`compact_entity`] instead of the default serde implementation for an entity.
pub fn entity_to_extend_mut<W: Extend<u8>>(
    entity: &Entity,
    writer: &mut W,
) -> postcard::Result<()> {
    let mut serializer = Serializer {
        output: ExtendMutFlavor::new(writer),
    };
    compact_entity::serialize(entity, &mut serializer)
}

/// A serialization flavor for an [`Extend<u8>`].
///
/// It's similar to [`ExtendFlavor`](postcard::ser_flavors::ExtendFlavor), but accepts a mutable reference.
///
/// Most of the time you can use more convenient [`to_extend_mut`] helper, unless you need access to [`postcard::Serializer`].
///
/// # Examples
///
/// Serializing a reflected type:
///
/// ```
/// use bevy::{
///     prelude::*,
///     reflect::{serde::ReflectSerializer, TypeRegistry},
/// };
/// use bevy_replicon::postcard_utils::ExtendMutFlavor;
/// use postcard::Serializer;
/// use serde::Serialize;
///
/// let mut registry = TypeRegistry::default();
/// registry.register::<Transform>();
/// let transform = Transform::default();
/// let mut message = Vec::new();
/// let mut serializer = Serializer { output: ExtendMutFlavor::new(&mut message) };
/// ReflectSerializer::new(transform.as_partial_reflect(), &registry).serialize(&mut serializer).unwrap();
/// assert!(!message.is_empty());
/// ```
pub struct ExtendMutFlavor<'a, T: Extend<u8>> {
    bytes: &'a mut T,
}

impl<'a, T: Extend<u8>> ExtendMutFlavor<'a, T> {
    /// Creates a new instance from the given collection.
    pub fn new(bytes: &'a mut T) -> Self {
        Self { bytes }
    }
}

impl<T: Extend<u8>> SerFlavor for ExtendMutFlavor<'_, T> {
    type Output = ();

    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        self.bytes.extend([data]);
        Ok(())
    }

    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        self.bytes.extend(data.iter().copied());
        Ok(())
    }

    fn finalize(self) -> postcard::Result<Self::Output> {
        Ok(())
    }
}

/// Deserializes a message from a buffer.
///
/// Similar to [`postcard::take_from_bytes`], but accepts a sliding buffer
/// avoiding the need for the caller to reassign the original slice with the returned unused portion.
///
/// See also [`to_extend_mut`] and [`entity_from_buf`].
///
/// # Examples
///
/// ```
/// use bevy::prelude::*;
/// use bevy_replicon::{bytes::Bytes, postcard_utils};
///
/// # let transform = Transform::default();
/// # let mut message = Vec::new();
/// # postcard_utils::to_extend_mut(&transform, &mut message).unwrap();
/// let mut message: Bytes = message.into();
/// let new_transform: Transform = postcard_utils::from_buf(&mut message).unwrap();
/// # assert_eq!(transform, new_transform);
/// # assert!(message.is_empty());
/// ```
pub fn from_buf<'de, T: Deserialize<'de>, B: Buf>(buf: &'de mut B) -> postcard::Result<T> {
    let mut deserializer = Deserializer::from_flavor(BufFlavor::new(buf));
    T::deserialize(&mut deserializer)
}

/// Like [`from_buf`], but uses [`compact_entity`] instead of the default serde implementation for an entity.
pub fn entity_from_buf<B: Buf>(buf: &mut B) -> postcard::Result<Entity> {
    let mut deserializer = Deserializer::from_flavor(BufFlavor::new(buf));
    compact_entity::deserialize(&mut deserializer)
}

/// A deserialization flavor for a borrowed buffer.
///
/// Unlike [`Slice`](postcard::de_flavors::Slice), deserialization advances buffer's cursor.
///
/// Most of the time you can use more convenient [`from_buf`] helper, unless you need access to [`postcard::Deserializer`].
///
/// # Examples
///
/// Deserializing a reflected type:
///
/// ```
/// # use bevy::{prelude::*, reflect::{serde::ReflectSerializer, TypeRegistry}};
/// # use bevy_replicon::postcard_utils::ExtendMutFlavor;
/// # use postcard::Serializer;
/// # use serde::Serialize;
/// use bevy::reflect::serde::ReflectDeserializer;
/// use bevy_replicon::{bytes::Bytes, postcard_utils::BufFlavor};
/// use postcard::Deserializer;
/// use serde::de::DeserializeSeed;
///
/// # let mut registry = TypeRegistry::default();
/// # registry.register::<Transform>();
/// # let transform = Transform::default();
/// # let mut message = Vec::new();
/// # let mut serializer = Serializer { output: ExtendMutFlavor::new(&mut message) };
/// # ReflectSerializer::new(transform.as_partial_reflect(), &registry).serialize(&mut serializer).unwrap();
/// let mut message: Bytes = message.into();
/// let mut deserializer = Deserializer::from_flavor(BufFlavor::new(&mut message));
/// let reflect = ReflectDeserializer::new(&registry).deserialize(&mut deserializer).unwrap();
/// # assert!(transform.reflect_partial_eq(&*reflect).unwrap());
/// # assert!(message.is_empty());
/// ```
pub struct BufFlavor<'a, T: Buf> {
    buf: &'a mut T,
}

impl<'a, T: Buf> BufFlavor<'a, T> {
    /// Creates a new instance from a buffer.
    pub fn new(buf: &'a mut T) -> Self {
        Self { buf }
    }
}

impl<'a, T: Buf> DeFlavor<'a> for BufFlavor<'a, T> {
    type Remainder = ();
    type Source = &'a [u8];

    fn pop(&mut self) -> postcard::Result<u8> {
        self.buf
            .try_get_u8()
            .map_err(|_| postcard::Error::DeserializeUnexpectedEnd)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.buf.remaining())
    }

    fn try_take_n(&mut self, ct: usize) -> postcard::Result<&'a [u8]> {
        if self.buf.remaining() < ct {
            return Err(postcard::Error::DeserializeUnexpectedEnd);
        }

        // SAFETY: slice was validated and its lifetime is tied to the buffer.
        let buf = unsafe { slice::from_raw_parts(self.buf.chunk().as_ptr(), ct) };
        self.buf.advance(ct);

        Ok(buf)
    }

    fn finalize(self) -> postcard::Result<Self::Remainder> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_without_generation() {
        let expected_entity = Entity::from_raw(1);
        let mut buffer = Vec::new();
        entity_to_extend_mut(&expected_entity, &mut buffer).unwrap();
        assert_eq!(buffer.len(), 1);

        let entity = entity_from_buf(&mut &*buffer).unwrap();
        assert_eq!(entity, expected_entity);
    }

    #[test]
    fn entity_with_generation() {
        let expected_entity = Entity::from_bits(1 | (2 << 32));
        let mut buffer = Vec::new();
        entity_to_extend_mut(&expected_entity, &mut buffer).unwrap();
        assert_eq!(buffer.len(), 2);

        let entity = entity_from_buf(&mut &*buffer).unwrap();
        assert_eq!(entity, expected_entity);
    }
}
