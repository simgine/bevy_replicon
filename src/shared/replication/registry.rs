pub mod command_fns;
pub mod component_fns;
pub mod ctx;
pub mod rule_fns;
pub mod test_fns;

use core::any;

use bevy::{ecs::component::ComponentId, prelude::*};
use log::trace;
use serde::{Deserialize, Serialize};

use super::command_markers::CommandMarkerIndex;
use crate::prelude::*;
use command_fns::{MutWrite, RemoveFn, UntypedCommandFns, WriteFn};
use component_fns::ComponentFns;
use ctx::DespawnCtx;
use rule_fns::UntypedRuleFns;

/// Stores configurable replication functions.
#[derive(Resource)]
pub struct ReplicationRegistry {
    /// Custom function to handle entity despawning.
    ///
    /// By default uses [`despawn`].
    /// Useful if you need to intercept despawns and handle them in a special way.
    pub despawn: DespawnFn,

    /// Functions for replicated components.
    ///
    /// Unique for each component.
    components: Vec<(ComponentId, ComponentFns)>,

    /// Serialization/deserialization functions for a component and
    /// the component's index in [`Self::components`].
    ///
    /// Can be registered multiple times for the same component for a different
    /// [`ReplicationRule`](super::rules::ReplicationRule)
    rules: Vec<(UntypedRuleFns, usize)>,

    /// Number of registered markers.
    ///
    /// Used to initialize new [`ComponentFns`] with the registered number of slots.
    marker_slots: usize,
}

impl ReplicationRegistry {
    /// Registers marker slot for component functions.
    ///
    /// Should be used after calling
    /// [`CommandMarkers::insert`](super::command_markers::CommandMarkers::insert)
    pub(super) fn register_marker(&mut self, marker_id: CommandMarkerIndex) {
        self.marker_slots += 1;
        for (_, command_fns) in &mut self.components {
            command_fns.add_marker_slot(marker_id);
        }
    }

    /// Associates command functions with a marker for a component.
    ///
    /// **Must** be called **after** calling [`Self::register_marker`] with `marker_id`.
    ///
    /// See also [`Self::set_command_fns`].
    ///
    /// # Panics
    ///
    /// Panics if the marker wasn't registered. Use [`Self::register_marker`] first.
    pub(super) fn set_marker_fns<C: Component<Mutability: MutWrite<C>>>(
        &mut self,
        world: &mut World,
        marker_id: CommandMarkerIndex,
        write: WriteFn<C>,
        remove: RemoveFn,
    ) {
        let (index, _) = self.init_component_fns::<C>(world);
        let (_, component_fns) = &mut self.components[index];
        let command_fns = UntypedCommandFns::new(write, remove);

        // SAFETY: `component_fns` and `command_fns` were created for `C`.
        unsafe {
            component_fns.set_marker_fns(marker_id, command_fns);
        }
    }

    /// Sets default functions for a component when there are no markers.
    ///
    /// See also [`Self::set_marker_fns`].
    pub(super) fn set_command_fns<C: Component<Mutability: MutWrite<C>>>(
        &mut self,
        world: &mut World,
        write: WriteFn<C>,
        remove: RemoveFn,
    ) {
        let (index, _) = self.init_component_fns::<C>(world);
        let (_, component_fns) = &mut self.components[index];
        let command_fns = UntypedCommandFns::new(write, remove);

        // SAFETY: `component_fns` and `command_fns` were created for `C`.
        unsafe {
            component_fns.set_command_fns(command_fns);
        }
    }

    /// Registers serialization/deserialization functions for a component.
    ///
    /// Returned data can be assigned to a
    /// [`ReplicationRule`](super::rules::ReplicationRule)
    pub fn register_rule_fns<C: Component<Mutability: MutWrite<C>>>(
        &mut self,
        world: &mut World,
        rule_fns: RuleFns<C>,
    ) -> (ComponentId, FnsId) {
        let (index, component_id) = self.init_component_fns::<C>(world);
        self.rules.push((rule_fns.into(), index));
        let fns_id = FnsId(self.rules.len() - 1);

        trace!("registering `{fns_id:?}` for `{}`", any::type_name::<C>());
        (component_id, fns_id)
    }

    /// Initializes [`ComponentFns`] for a component and returns its index and ID.
    ///
    /// If a [`ComponentFns`] has already been created for this component,
    /// then it returns its index instead of creating a new one.
    fn init_component_fns<C: Component<Mutability: MutWrite<C>>>(
        &mut self,
        world: &mut World,
    ) -> (usize, ComponentId) {
        let component_id = world.register_component::<C>();
        let index = self
            .components
            .iter()
            .position(|&(id, _)| id == component_id)
            .unwrap_or_else(|| {
                self.components
                    .push((component_id, ComponentFns::new::<C>(self.marker_slots)));
                self.components.len() - 1
            });

        (index, component_id)
    }

    /// Returns associates functions.
    ///
    /// See also [`Self::register_rule_fns`].
    pub(crate) fn get(&self, fns_id: FnsId) -> (ComponentId, &ComponentFns, &UntypedRuleFns) {
        let (rule_fns, index) = self
            .rules
            .get(fns_id.0)
            .unwrap_or_else(|| panic!("replication `{fns_id:?}` should be registered first"));

        // SAFETY: index obtained from `rules` is always valid.
        let (component_id, command_fns) = unsafe { self.components.get_unchecked(*index) };

        (*component_id, command_fns, rule_fns)
    }
}

impl Default for ReplicationRegistry {
    fn default() -> Self {
        Self {
            despawn,
            components: Default::default(),
            rules: Default::default(),
            marker_slots: 0,
        }
    }
}

/// ID of replicaton functions for a component.
///
/// Can be obtained from [`ReplicationRegistry::register_rule_fns`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct FnsId(usize);

/// Signature of the entity despawn function.
pub type DespawnFn = fn(&DespawnCtx, EntityWorldMut);

/// Default entity despawn function.
pub fn despawn(_ctx: &DespawnCtx, entity: EntityWorldMut) {
    entity.despawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_fns() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        registry.register_rule_fns(&mut world, RuleFns::<A>::default());
        assert_eq!(registry.rules.len(), 1);
        assert_eq!(registry.components.len(), 1);
    }

    #[test]
    fn duplicate_rule_fns() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        registry.register_rule_fns(&mut world, RuleFns::<A>::default());
        registry.register_rule_fns(&mut world, RuleFns::<A>::default());

        assert_eq!(registry.rules.len(), 2);
        assert_eq!(
            registry.components.len(),
            1,
            "multiple serde registrations for the same component should result only in a single command functions instance"
        );
    }

    #[test]
    fn different_rule_fns() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        registry.register_rule_fns(&mut world, RuleFns::<A>::default());
        registry.register_rule_fns(&mut world, RuleFns::<B>::default());

        assert_eq!(registry.rules.len(), 2);
        assert_eq!(registry.components.len(), 2);
    }

    #[derive(Component, Serialize, Deserialize)]
    struct A;

    #[derive(Component, Deserialize, Serialize)]
    struct B;
}
