use bevy::{ecs::component::ComponentId, prelude::*};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    prelude::*,
    shared::replication::registry::{FnsId, ReplicationRegistry, command_fns::MutWrite},
};

/// Component for [`ReplicationRule`](super::ReplicationRule).
#[derive(Clone, Copy, Debug)]
pub struct ComponentRule {
    /// ID of the replicated component.
    pub id: ComponentId,
    /// Associated serialization and deserialization functions.
    pub fns_id: FnsId,
    /// Send rate configuration.
    pub send_rate: SendRate,
}

impl ComponentRule {
    /// Creates a new instance with the default send rate.
    pub fn new(id: ComponentId, fns_id: FnsId) -> Self {
        Self {
            id,
            fns_id,
            send_rate: Default::default(),
        }
    }
}

/// Describes how often component changes should be replicated.
///
/// Used inside [`ComponentRule`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SendRate {
    /// Replicate any change every tick.
    ///
    /// If multiple changes occur in the same tick,
    /// only the latest value will be replicated.
    #[default]
    EveryTick,

    /// Replicates only the initial value and removal.
    ///
    /// Component mutations won't be sent.
    Once,

    /// Replicate mutations at a specified interval.
    ///
    /// If multiple mutations occur within the interval,
    /// only the latest value at the time of sending will
    /// be replicated.
    ///
    /// Does not affect initial values or removals.
    ///
    /// For example, with a period of 2, any mutation
    /// will be replicated every second tick.
    Periodic(u32),
}

impl SendRate {
    /// Returns `true` if a mutation for a component in a replication rule should be replicated on this tick.
    pub fn send_mutations(self, tick: RepliconTick) -> bool {
        match self {
            SendRate::EveryTick => true,
            SendRate::Once => false,
            SendRate::Periodic(period) => tick.get() % period == 0,
        }
    }
}

/// Parameters that can be turned into a component replication rule.
///
/// Used for [`IntoComponentRules`] to accept either [`RuleFns`] or a tuple combining
/// [`RuleFns`] with an associated [`SendRate`].
///
/// See [`AppRuleExt::replicate_with`] for more details.
pub trait IntoComponentRule {
    /// Turns into a component replication rule and registers its functions in [`ReplicationRegistry`].
    fn into_rule(self, world: &mut World, registry: &mut ReplicationRegistry) -> ComponentRule;
}

impl<C: Component<Mutability: MutWrite<C>>> IntoComponentRule for RuleFns<C> {
    fn into_rule(self, world: &mut World, registry: &mut ReplicationRegistry) -> ComponentRule {
        let (id, fns_id) = registry.register_rule_fns(world, self);
        ComponentRule::new(id, fns_id)
    }
}

impl<C: Component<Mutability: MutWrite<C>>> IntoComponentRule for (RuleFns<C>, SendRate) {
    fn into_rule(self, world: &mut World, registry: &mut ReplicationRegistry) -> ComponentRule {
        let (rule_fns, send_rate) = self;
        let (id, fns_id) = registry.register_rule_fns(world, rule_fns);
        ComponentRule {
            id,
            fns_id,
            send_rate,
        }
    }
}

/// Parameters that can be turned into a list of component replication rules.
///
/// Implemented for tuples of [`IntoComponentRule`].
///
/// See [`AppRuleExt::replicate_with`] for more details.
pub trait IntoComponentRules {
    /// Priority when registered with [`AppRuleExt::replicate_with`].
    ///
    /// Equals the number of components in a rule.
    const DEFAULT_PRIORITY: usize;

    /// Turns into a replication rule and registers its functions in [`ReplicationRegistry`].
    fn into_rules(
        self,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) -> Vec<ComponentRule>;
}

impl<C: IntoComponentRule> IntoComponentRules for C {
    const DEFAULT_PRIORITY: usize = 1;

    fn into_rules(
        self,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) -> Vec<ComponentRule> {
        vec![self.into_rule(world, registry)]
    }
}

macro_rules! impl_into_component_rules {
    ($(($n:tt, $R:ident)),*) => {
        impl<$($R: IntoComponentRule),*> IntoComponentRules for ($($R,)*) {
            // Uses dummy variable `n` to add 1 for each tuple element.
            const DEFAULT_PRIORITY: usize = 0 $(+ { let _ = $n; 1 })*;

            fn into_rules(
                self,
                world: &mut World,
                registry: &mut ReplicationRegistry,
            ) -> Vec<ComponentRule> {
                vec![
                    $(
                        self.$n.into_rule(world, registry),
                    )*
                ]
            }
        }
    }
}

variadics_please::all_tuples_enumerated!(impl_into_component_rules, 1, 15, R);

/// Component replication rules associated with a bundle and its priority.
///
/// See [`AppRuleExt::replicate_bundle`] for more details.
pub trait BundleRules {
    /// Priority when registered with [`AppRuleExt::replicate_bundle`].
    ///
    /// Equals the number of components in a bundle.
    const DEFAULT_PRIORITY: usize;

    /// Creates the associated component replication rules and registers their functions in [`ReplicationRegistry`].
    fn component_rules(world: &mut World, registry: &mut ReplicationRegistry)
    -> Vec<ComponentRule>;
}

macro_rules! impl_into_bundle_rules {
    ($N:expr, $($C:ident),*) => {
        impl<$($C: Component<Mutability: MutWrite<$C>> + Serialize + DeserializeOwned),*> BundleRules for ($($C,)*) {
            const DEFAULT_PRIORITY: usize = $N;

            fn component_rules(world: &mut World, registry: &mut ReplicationRegistry) -> Vec<ComponentRule> {
                vec![
                    $(
                        {
                            let (id, fns_id) = registry.register_rule_fns(world, RuleFns::<$C>::default());
                            ComponentRule {
                                id,
                                fns_id,
                                send_rate: Default::default(),
                            }
                        },
                    )*
                ]
            }
        }
    }
}

variadics_please::all_tuples_with_size!(impl_into_bundle_rules, 1, 15, C);
