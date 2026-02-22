use bevy::{prelude::*, ptr::Ptr};
use bytes::Bytes;

use super::{
    ctx::{RemoveCtx, SerializeCtx, WriteCtx},
    receive_fns::{MutWrite, UntypedReceiveFns},
    rule_fns::UntypedRuleFns,
};
use crate::shared::replication::{
    deferred_entity::DeferredEntity,
    receive_markers::{EntityMarkers, ReceiveMarkerIndex, ReceiveMarkers},
};

/// Type-erased functions for a component.
///
/// Stores type-erased receive functions and functions that will restore original types.
pub(crate) struct ComponentFns {
    serialize: UntypedSerializeFn,
    write: UntypedWriteFn,
    consume: UntypedConsumeFn,
    receive: UntypedReceiveFns,
    markers: Vec<Option<UntypedReceiveFns>>,
}

impl ComponentFns {
    /// Creates a new instance for `C` with the specified number of empty marker function slots.
    pub(super) fn new<C: Component<Mutability: MutWrite<C>>>(marker_slots: usize) -> Self {
        Self {
            serialize: untyped_serialize::<C>,
            write: untyped_write::<C>,
            consume: untyped_consume::<C>,
            receive: UntypedReceiveFns::default_fns::<C>(),
            markers: vec![None; marker_slots],
        }
    }

    /// Adds new empty slot for a marker.
    ///
    /// Use [`Self::set_marker_fns`] to assign functions to it.
    pub(super) fn add_marker_slot(&mut self, marker_id: ReceiveMarkerIndex) {
        self.markers.insert(*marker_id, None);
    }

    /// Assigns functions to a marker slot.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `receive_fns` was created for the same type as this instance.
    ///
    /// # Panics
    ///
    /// Panics if there is no such slot for the marker. Use [`Self::add_marker_slot`] to assign.
    pub(super) unsafe fn set_marker_fns(
        &mut self,
        marker_id: ReceiveMarkerIndex,
        receive_fns: UntypedReceiveFns,
    ) {
        let fns = self
            .markers
            .get_mut(*marker_id)
            .unwrap_or_else(|| panic!("receive fns should have a slot for {marker_id:?}"));

        debug_assert!(
            fns.is_none(),
            "function for {marker_id:?} can't be set twice"
        );

        *fns = Some(receive_fns);
    }

    /// Sets default functions that will be called when there are no marker matches.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `receive_fns` was created for the same type as this instance.
    pub(super) unsafe fn set_receive_fns(&mut self, receive_fns: UntypedReceiveFns) {
        self.receive = receive_fns;
    }

    /// Restores erased type from `ptr` and `rule_fns` to the type for which this instance was created,
    /// then serializes it.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` and `rule_fns` were created for the same type as this instance.
    pub(crate) unsafe fn serialize(
        &self,
        ctx: &SerializeCtx,
        rule_fns: &UntypedRuleFns,
        ptr: Ptr,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        unsafe { (self.serialize)(ctx, rule_fns, ptr, message) }
    }

    /// Calls the assigned writing function based on entity markers.
    ///
    /// The first-found write function whose marker is present on the entity will be selected
    /// (the functions are sorted by priority).
    /// If there is no such function, it will use the default function.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `rule_fns` was created for the same type as this instance.
    pub(crate) unsafe fn write(
        &self,
        ctx: &mut WriteCtx,
        rule_fns: &UntypedRuleFns,
        entity_markers: &EntityMarkers,
        entity: &mut DeferredEntity,
        message: &mut Bytes,
    ) -> Result<()> {
        let receive_fns = self
            .markers
            .iter()
            .zip(entity_markers.markers())
            .filter(|&(_, contains)| *contains)
            .find_map(|(&fns, _)| fns)
            .unwrap_or(self.receive);

        unsafe { (self.write)(ctx, &receive_fns, rule_fns, entity, message) }
    }

    /// Calls the assigned writing or consuming function based on entity markers.
    ///
    /// Selects the first-found write function like [`Self::write`], but if its marker doesn't require history,
    /// the consume function will be used instead.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `rule_fns` was created for the same type as this instance.
    pub(crate) unsafe fn consume_or_write(
        &self,
        ctx: &mut WriteCtx,
        rule_fns: &UntypedRuleFns,
        entity_markers: &EntityMarkers,
        receive_markers: &ReceiveMarkers,
        entity: &mut DeferredEntity,
        message: &mut Bytes,
    ) -> Result<()> {
        if let Some(receive_fns) = self
            .markers
            .iter()
            .zip(entity_markers.markers())
            .zip(receive_markers.iter_require_history())
            .filter(|&((_, contains), _)| *contains)
            .find_map(|((&fns, _), need_history)| fns.map(|fns| (fns, need_history)))
            .and_then(|(fns, need_history)| need_history.then_some(fns))
        {
            unsafe { (self.write)(ctx, &receive_fns, rule_fns, entity, message) }
        } else {
            unsafe { (self.consume)(ctx, rule_fns, message) }
        }
    }

    /// Same as [`Self::write`], but calls the assigned remove function.
    pub(crate) fn remove(
        &self,
        ctx: &mut RemoveCtx,
        entity_markers: &EntityMarkers,
        entity: &mut DeferredEntity,
    ) {
        let receive_fns = self
            .markers
            .iter()
            .zip(entity_markers.markers())
            .filter(|&(_, contains)| *contains)
            .find_map(|(&fns, _)| fns)
            .unwrap_or(self.receive);

        receive_fns.remove(ctx, entity)
    }
}

/// Signature of component serialization functions that restore the original type.
type UntypedSerializeFn =
    unsafe fn(&SerializeCtx, &UntypedRuleFns, Ptr, &mut Vec<u8>) -> Result<()>;

/// Signature of component writing functions that restore the original type.
type UntypedWriteFn = unsafe fn(
    &mut WriteCtx,
    &UntypedReceiveFns,
    &UntypedRuleFns,
    &mut DeferredEntity,
    &mut Bytes,
) -> Result<()>;

/// Signature of component consuming functions that restores the original type.
type UntypedConsumeFn = unsafe fn(&mut WriteCtx, &UntypedRuleFns, &mut Bytes) -> Result<()>;

/// Dereferences a component from a pointer and calls the passed serialization function.
///
/// # Safety
///
/// The caller must ensure that `ptr` and `rule_fns` were created for `C`.
unsafe fn untyped_serialize<C: Component>(
    ctx: &SerializeCtx,
    rule_fns: &UntypedRuleFns,
    ptr: Ptr,
    message: &mut Vec<u8>,
) -> Result<()> {
    unsafe {
        let rule_fns = rule_fns.typed::<C>();
        rule_fns.serialize(ctx, ptr.deref::<C>(), message)
    }
}

/// Resolves `rule_fns` to `C` and calls [`UntypedReceiveFns::write`] for `C`.
///
/// # Safety
///
/// The caller must ensure that `rule_fns` was created for `C`.
unsafe fn untyped_write<C: Component>(
    ctx: &mut WriteCtx,
    receive_fns: &UntypedReceiveFns,
    rule_fns: &UntypedRuleFns,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    unsafe { receive_fns.write::<C>(ctx, &rule_fns.typed::<C>(), entity, message) }
}

/// Resolves `rule_fns` to `C` and calls [`RuleFns::consume`](super::rule_fns::RuleFns) for `C`.
///
/// # Safety
///
/// The caller must ensure that `rule_fns` was created for `C`.
unsafe fn untyped_consume<C: Component>(
    ctx: &mut WriteCtx,
    rule_fns: &UntypedRuleFns,
    message: &mut Bytes,
) -> Result<()> {
    unsafe { rule_fns.typed::<C>().consume(ctx, message) }
}
