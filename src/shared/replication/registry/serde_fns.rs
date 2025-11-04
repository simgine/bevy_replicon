use bevy::{prelude::*, ptr::Ptr};
use bytes::Bytes;

use super::ctx::{RemoveCtx, SerializeCtx, WriteCtx};
use crate::shared::replication::{
    command_markers::{CommandMarkers, EntityMarkers},
    deferred_entity::DeferredEntity,
    registry::{component_fns::ComponentFns, rule_fns::UntypedRuleFns},
};

/// Wraps component and rule functions.
///
/// Rule functions can be defined differently for the same component,
/// but component functions that restore the type will be the same.
/// This is why they are sorted separately in [`ReplicationRegistry`](super::ReplicationRegistry).
///
/// However, always working with two function structs is verbose and potentially unsafe
/// if they correspond to different underlying types. This struct reduces boilerplate and
/// improves safety by encapsulating the unsafety within its creation.
pub(crate) struct SerdeFns<'a> {
    component_fns: &'a ComponentFns,
    rule_fns: &'a UntypedRuleFns,
}

impl<'a> SerdeFns<'a> {
    /// Creates a new instance for serialization and deserialization functions.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `RuleFns` and `ComponentFns` belong to the same type.
    pub(super) unsafe fn new(
        component_fns: &'a ComponentFns,
        rule_fns: &'a UntypedRuleFns,
    ) -> Self {
        Self {
            component_fns,
            rule_fns,
        }
    }
    /// Restores the erased type from `ptr` to the type for which this instance was created,
    /// and serializes it.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` was created for the same type as this instance.
    pub(crate) unsafe fn serialize(
        &self,
        ctx: &SerializeCtx,
        ptr: Ptr,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        // SAFETY: `RuleFns`, `ComponentFns` and `ptr` belong to the same type.
        unsafe {
            self.component_fns
                .serialize(ctx, self.rule_fns, ptr, message)
        }
    }

    /// Calls the assigned writing function based on entity markers.
    pub(crate) fn write(
        &self,
        ctx: &mut WriteCtx,
        entity_markers: &EntityMarkers,
        entity: &mut DeferredEntity,
        message: &mut Bytes,
    ) -> Result<()> {
        // SAFETY: `RuleFns` and `ComponentFns` belong to the same type.
        unsafe {
            self.component_fns
                .write(ctx, self.rule_fns, entity_markers, entity, message)
        }
    }

    /// Calls the assigned writing or consuming function based on entity markers.
    pub(crate) fn consume_or_write(
        &self,
        ctx: &mut WriteCtx,
        entity_markers: &EntityMarkers,
        command_markers: &CommandMarkers,
        entity: &mut DeferredEntity,
        message: &mut Bytes,
    ) -> Result<()> {
        // SAFETY: `RuleFns` and `ComponentFns` belong to the same type.
        unsafe {
            self.component_fns.consume_or_write(
                ctx,
                self.rule_fns,
                entity_markers,
                command_markers,
                entity,
                message,
            )
        }
    }

    /// Same as [`Self::write`], but calls the assigned remove function.
    pub(crate) fn remove(
        &self,
        ctx: &mut RemoveCtx,
        entity_markers: &EntityMarkers,
        entity: &mut DeferredEntity,
    ) {
        self.component_fns.remove(ctx, entity_markers, entity);
    }
}
