use bevy::{prelude::*, ptr::Ptr};
use bytes::Bytes;

use super::{
    command_fns::{MutWrite, UntypedCommandFns},
    ctx::{RemoveCtx, SerializeCtx, WriteCtx},
    rule_fns::UntypedRuleFns,
};
use crate::shared::replication::{
    command_markers::{CommandMarkerIndex, CommandMarkers, EntityMarkers},
    deferred_entity::DeferredEntity,
};

/// Type-erased functions for a component.
///
/// Stores type-erased command functions and functions that will restore original types.
pub(crate) struct ComponentFns {
    serialize: UntypedSerializeFn,
    write: UntypedWriteFn,
    consume: UntypedConsumeFn,
    commands: UntypedCommandFns,
    markers: Vec<Option<UntypedCommandFns>>,
}

impl ComponentFns {
    /// Creates a new instance for `C` with the specified number of empty marker function slots.
    pub(super) fn new<C: Component<Mutability: MutWrite<C>>>(marker_slots: usize) -> Self {
        Self {
            serialize: untyped_serialize::<C>,
            write: untyped_write::<C>,
            consume: untyped_consume::<C>,
            commands: UntypedCommandFns::default_fns::<C>(),
            markers: vec![None; marker_slots],
        }
    }

    /// Adds new empty slot for a marker.
    ///
    /// Use [`Self::set_marker_fns`] to assign functions to it.
    pub(super) fn add_marker_slot(&mut self, marker_id: CommandMarkerIndex) {
        self.markers.insert(*marker_id, None);
    }

    /// Assigns functions to a marker slot.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `command_fns` was created for the same type as this instance.
    ///
    /// # Panics
    ///
    /// Panics if there is no such slot for the marker. Use [`Self::add_marker_slot`] to assign.
    pub(super) unsafe fn set_marker_fns(
        &mut self,
        marker_id: CommandMarkerIndex,
        command_fns: UntypedCommandFns,
    ) {
        let fns = self
            .markers
            .get_mut(*marker_id)
            .unwrap_or_else(|| panic!("command fns should have a slot for {marker_id:?}"));

        debug_assert!(
            fns.is_none(),
            "function for {marker_id:?} can't be set twice"
        );

        *fns = Some(command_fns);
    }

    /// Sets default functions that will be called when there are no marker matches.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `command_fns` was created for the same type as this instance.
    pub(super) unsafe fn set_command_fns(&mut self, command_fns: UntypedCommandFns) {
        self.commands = command_fns;
    }

    /// Restores erased type from `ptr` and `rule_fns` to the type for which this instance was created.
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
        let command_fns = self
            .markers
            .iter()
            .zip(entity_markers.markers())
            .filter(|&(_, contains)| *contains)
            .find_map(|(&fns, _)| fns)
            .unwrap_or(self.commands);

        unsafe { (self.write)(ctx, &command_fns, rule_fns, entity, message) }
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
        command_markers: &CommandMarkers,
        entity: &mut DeferredEntity,
        message: &mut Bytes,
    ) -> Result<()> {
        if let Some(command_fns) = self
            .markers
            .iter()
            .zip(entity_markers.markers())
            .zip(command_markers.iter_require_history())
            .filter(|&((_, contains), _)| *contains)
            .find_map(|((&fns, _), need_history)| fns.map(|fns| (fns, need_history)))
            .and_then(|(fns, need_history)| need_history.then_some(fns))
        {
            unsafe { (self.write)(ctx, &command_fns, rule_fns, entity, message) }
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
        let command_fns = self
            .markers
            .iter()
            .zip(entity_markers.markers())
            .filter(|&(_, contains)| *contains)
            .find_map(|(&fns, _)| fns)
            .unwrap_or(self.commands);

        command_fns.remove(ctx, entity)
    }
}

/// Signature of component serialization functions that restore the original type.
type UntypedSerializeFn =
    unsafe fn(&SerializeCtx, &UntypedRuleFns, Ptr, &mut Vec<u8>) -> Result<()>;

/// Signature of component writing functions that restore the original type.
type UntypedWriteFn = unsafe fn(
    &mut WriteCtx,
    &UntypedCommandFns,
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

/// Resolves `rule_fns` to `C` and calls [`UntypedCommandFns::write`] for `C`.
///
/// # Safety
///
/// The caller must ensure that `rule_fns` was created for `C`.
unsafe fn untyped_write<C: Component>(
    ctx: &mut WriteCtx,
    command_fns: &UntypedCommandFns,
    rule_fns: &UntypedRuleFns,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    unsafe { command_fns.write::<C>(ctx, &rule_fns.typed::<C>(), entity, message) }
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
