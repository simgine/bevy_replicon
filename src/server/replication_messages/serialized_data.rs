use core::ops::Range;

use bevy::{prelude::*, ptr::Ptr};

use crate::{
    postcard_utils,
    prelude::*,
    shared::replication::registry::{
        FnsId, component_fns::ComponentFns, ctx::SerializeCtx, rule_fns::UntypedRuleFns,
    },
};

/// Single continuous buffer that stores serialized data for messages.
///
/// See [`Updates`](super::updates::Updates) and
/// [`MutateMessage`](super::mutations::MutateMessage).
#[derive(Default, Deref, DerefMut)]
pub(crate) struct SerializedData(Vec<u8>);

impl SerializedData {
    pub(crate) fn write_mappings(
        &mut self,
        mappings: impl Iterator<Item = (Entity, Entity)>,
    ) -> Result<Range<usize>> {
        let start = self.len();

        for (server_entity, client_entity) in mappings {
            self.write_entity(server_entity)?;
            self.write_entity(client_entity)?;
        }

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_fn_ids(
        &mut self,
        fn_ids: impl Iterator<Item = FnsId>,
    ) -> Result<Range<usize>> {
        let start = self.len();

        for fns_id in fn_ids {
            postcard_utils::to_extend_mut(&fns_id, &mut self.0)?;
        }

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_component(
        &mut self,
        rule_fns: &UntypedRuleFns,
        component_fns: &ComponentFns,
        ctx: &SerializeCtx,
        fns_id: FnsId,
        ptr: Ptr,
    ) -> Result<Range<usize>> {
        let start = self.len();

        postcard_utils::to_extend_mut(&fns_id, &mut self.0)?;
        // SAFETY: `component_fns`, `ptr` and `rule_fns` were created for the same component type.
        unsafe { component_fns.serialize(ctx, rule_fns, ptr, &mut self.0)? };

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_entity(&mut self, entity: Entity) -> Result<Range<usize>> {
        let start = self.len();

        postcard_utils::entity_to_extend_mut(&entity, &mut self.0)?;

        let end = self.len();

        Ok(start..end)
    }

    pub(crate) fn write_tick(&mut self, tick: RepliconTick) -> Result<Range<usize>> {
        let start = self.len();

        postcard_utils::to_extend_mut(&tick, &mut self.0)?;

        let end = self.len();

        Ok(start..end)
    }
}
