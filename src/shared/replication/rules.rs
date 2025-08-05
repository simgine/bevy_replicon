use core::cmp::Reverse;

use bevy::{
    ecs::{archetype::Archetype, component::ComponentId},
    platform::collections::HashSet,
    prelude::*,
};
use serde::{Serialize, de::DeserializeOwned};

use super::registry::{FnsId, ReplicationRegistry, command_fns::MutWrite};
use crate::prelude::*;

/// Replication functions for [`App`].
pub trait AppRuleExt {
    /// Defines a [`ReplicationRule`] for a single component.
    ///
    /// If present on an entity with [`Replicated`] component,
    /// it will be serialized and deserialized as-is using [`postcard`]
    /// and sent at [`SendRate::EveryTick`]. To customize this, use [`Self::replicate_with`].
    ///
    /// See also the section on [`components`](../../index.html#components) from the quick start guide.
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_with(RuleFns::<C>::default())
    }

    /// Same as [`Self::replicate`], but uses [`SendRate::Once`].
    fn replicate_once<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_with((RuleFns::<C>::default(), SendRate::Once))
    }

    /// Sames as [`Self::replicate`], but uses [`SendRate::Periodic`] with the given tick period.
    fn replicate_periodic<C>(&mut self, period: u32) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_with((RuleFns::<C>::default(), SendRate::Periodic(period)))
    }

    /**
    Defines a [`ReplicationRule`] for a bundle.

    Implemented for tuples of components. Use it to conveniently create a rule with
    default ser/de functions and [`SendRate::EveryTick`] for all components.
    To customize this, use [`Self::replicate_with`].

    Can also be implemented manually for user-defined bundles.

    # Examples

    ```
    use bevy::prelude::*;
    use bevy_replicon::{
        bytes::Bytes,
        shared::replication::{
            registry::{
                ctx::{SerializeCtx, WriteCtx},
                ReplicationRegistry,
            },
            rules::{ReplicationBundle, ReplicationRule, ComponentRule},
        },
        prelude::*,
    };
    use serde::{Deserialize, Serialize};

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    app.replicate_bundle::<(Name, City)>() // Tuple of components is also a bundle!
        .replicate_bundle::<PlayerBundle>();

    #[derive(Component, Deserialize, Serialize)]
    struct City;

    #[derive(Bundle)]
    struct PlayerBundle {
        transform: Transform,
        player: Player,
    }

    #[derive(Component, Deserialize, Serialize)]
    struct Player;

    impl ReplicationBundle for PlayerBundle {
        fn register(world: &mut World, registry: &mut ReplicationRegistry) -> ReplicationRule {
            // Customize serlialization to serialize only `translation`.
            let (transform_id, transform_fns_id) = registry.register_rule_fns(
                world,
                RuleFns::new(serialize_translation, deserialize_translation),
            );
            let transform_rule = ComponentRule::new(transform_id, transform_fns_id);

            // Serialize `player` as usual.
            let (player_id, player_fns_id) = registry.register_rule_fns(world, RuleFns::<Player>::default());
            let player_rule = ComponentRule::new(player_id, player_fns_id);

            // We skip `replication` registration since it's a special component.
            // It's automatically inserted on clients after replication and
            // deserialization from scenes.

            ReplicationRule::new(vec![transform_rule, player_rule])
        }
    }

    # fn serialize_translation(_: &SerializeCtx, _: &Transform, _: &mut Vec<u8>) -> Result<()> { unimplemented!() }
    # fn deserialize_translation(_: &mut WriteCtx, _: &mut Bytes) -> Result<Transform> { unimplemented!() }
    ```
    **/
    fn replicate_bundle<B: ReplicationBundle>(&mut self) -> &mut Self;

    /**
    Defines a customizable [`ReplicationRule`].

    Can be used to customize how a component is passed over the network, or
    for components that don't implement [`Serialize`] or [`DeserializeOwned`].

    You can also pass a tuple of [`RuleFns`] to define a rule for multiple components.
    These components will only be replicated if all of them are present on the entity.
    To assign a [`SendRate`] to a component, wrap its [`RuleFns`] in a tuple with the
    desired rate.

    If an entity matches multiple rules, the functions from the rule with higher priority
    will take precedence for overlapping components. For example, a rule for `Health`
    and a `Player` marker will take precedence over a rule for `Health` alone. This can
    be used to specialize serialization for a specific set of components.

    If you remove a single component from such a rule from an entity, only one
    removal will be sent to clients. The other components in the rule will remain
    present on both the server and the clients. Replication for them will be stopped,
    unless they match another rule.

    <div class="warning">

    If your component contains an [`Entity`] inside, don't forget to call [`Component::map_entities`]
    in your deserialization function.

    </div>

    You can also override how the component will be written, see [`AppMarkerExt`].

    See also [`postcard_utils`](crate::postcard_utils) for serialization helpers.

    # Examples

    Pass [`RuleFns`] to ser/de only specific field:

    ```
    use bevy::prelude::*;
    use bevy_replicon::{
        bytes::Bytes,
        postcard_utils,
        shared::replication::registry::{
            ctx::{SerializeCtx, WriteCtx},
            rule_fns::DeserializeFn,
        },
        prelude::*,
    };

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    // We override in-place as well to apply only translation when the component is already inserted.
    app.replicate_with(
        RuleFns::new(serialize_translation, deserialize_translation)
            .with_in_place(deserialize_transform_in_place),
    );

    /// Serializes only `translation` from [`Transform`].
    fn serialize_translation(
        _ctx: &SerializeCtx,
        transform: &Transform,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        postcard_utils::to_extend_mut(&transform.translation, message)?;
        Ok(())
    }

    /// Deserializes `translation` and creates [`Transform`] from it.
    ///
    /// Called by Replicon on component insertions.
    fn deserialize_translation(
        _ctx: &mut WriteCtx,
        message: &mut Bytes,
    ) -> Result<Transform> {
        let translation: Vec3 = postcard_utils::from_buf(message)?;
        Ok(Transform::from_translation(translation))
    }

    /// Applies the assigned deserialization function and assigns only translation.
    ///
    /// Called by Replicon on component mutations.
    fn deserialize_transform_in_place(
        deserialize: DeserializeFn<Transform>,
        ctx: &mut WriteCtx,
        component: &mut Transform,
        message: &mut Bytes,
    ) -> Result<()> {
        let transform = (deserialize)(ctx, message)?;
        component.translation = transform.translation;
        Ok(())
    }
    ```

    A rule with multiple components:

    ```
    use bevy::prelude::*;
    use bevy_replicon::prelude::*;
    use serde::{Deserialize, Serialize};

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    app.replicate_with((
        // You can also use `replicate_bundle` if you don't want
        // to tweak functions or send rate.
        RuleFns::<Player>::default(),
        RuleFns::<Position>::default(),
    ))
    .replicate_with((
        RuleFns::<MovingPlatform>::default(),
        // Send position only once.
        (RuleFns::<Position>::default(), SendRate::Once),
    ));

    #[derive(Component, Deserialize, Serialize)]
    struct Player;

    #[derive(Component, Deserialize, Serialize)]
    struct MovingPlatform;

    #[derive(Component, Deserialize, Serialize)]
    struct Position(Vec2);
    ```

    Ser/de with compression:

    ```
    use bevy::prelude::*;
    use bevy_replicon::{
        bytes::Bytes,
        postcard_utils,
        shared::replication::registry::{
            ctx::{SerializeCtx, WriteCtx},
            rule_fns::RuleFns,
        },
        postcard,
        prelude::*,
    };
    use bytes::Buf;
    use serde::{Deserialize, Serialize};

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    app.replicate_with(RuleFns::new(
        serialize_big_component,
        deserialize_big_component,
    ));

    fn serialize_big_component(
        _ctx: &SerializeCtx,
        component: &BigComponent,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        // Serialize as usual, but track size.
        let start = message.len();
        postcard_utils::to_extend_mut(component, message)?;
        let end = message.len();

        // Compress serialized slice.
        // Could be `zstd`, for example.
        let compressed = compress(&mut message[start..end]);

        // Replace serialized slice with compressed data prepended by its size.
        message.truncate(start);
        postcard_utils::to_extend_mut(&compressed.len(), message)?;
        message.extend(compressed);

        Ok(())
    }

    fn deserialize_big_component(
        _ctx: &mut WriteCtx,
        message: &mut Bytes,
    ) -> Result<BigComponent> {
        // Read size first to know how much data is encoded.
        let size = postcard_utils::from_buf(message)?;

        // Apply decompression and advance the reading cursor.
        let decompressed = decompress(&message[..size]);
        message.advance(size);

        let component = postcard::from_bytes(&decompressed)?;
        Ok(component)
    }

    #[derive(Component, Deserialize, Serialize)]
    struct BigComponent(Vec<u64>);
    # fn compress(data: &[u8]) -> Vec<u8> { unimplemented!() }
    # fn decompress(data: &[u8]) -> Vec<u8> { unimplemented!() }
    ```

    Custom ser/de with entity mapping:

    ```
    use bevy::prelude::*;
    use bevy_replicon::{
        bytes::Bytes,
        postcard_utils,
        shared::replication::registry::{
            ctx::{SerializeCtx, WriteCtx},
            rule_fns::RuleFns,
        },
        postcard,
        prelude::*,
    };
    use serde::{Deserialize, Serialize};

    let mut app = App::new();
    app.add_plugins(RepliconPlugins);
    app.replicate_with(RuleFns::new(
        serialize_mapped_component,
        deserialize_mapped_component,
    ));

    /// Serializes [`MappedComponent`], but skips [`MappedComponent::unused_field`].
    fn serialize_mapped_component(
        _ctx: &SerializeCtx,
        component: &MappedComponent,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        postcard_utils::to_extend_mut(&component.entity, message)?;
        Ok(())
    }

    /// Deserializes an entity and creates [`MappedComponent`] from it.
    fn deserialize_mapped_component(
        ctx: &mut WriteCtx,
        message: &mut Bytes,
    ) -> Result<MappedComponent> {
        let entity = postcard_utils::from_buf(message)?;
        let mut component = MappedComponent {
            entity,
            unused_field: Default::default(),
        };
        MappedComponent::map_entities(&mut component, ctx); // Important to call!
        Ok(component)
    }

    #[derive(Component, Deserialize, Serialize)]
    struct MappedComponent {
        #[entities]
        entity: Entity,
        unused_field: Vec<bool>,
    }
    ```

    Component with [`Box<dyn PartialReflect>`]:

    ```
    use bevy::{
        prelude::*,
        reflect::serde::{ReflectDeserializer, ReflectSerializer},
    };
    use bevy_replicon::{
        bytes::Bytes,
        postcard_utils::{BufFlavor, ExtendMutFlavor},
        shared::replication::registry::{
            ctx::{SerializeCtx, WriteCtx},
            rule_fns::RuleFns,
        },
        postcard::{self, Deserializer, Serializer},
        prelude::*,
    };
    use serde::{de::DeserializeSeed, Serialize};

    let mut app = App::new();
    app.add_plugins(RepliconPlugins);
    app.replicate_with(RuleFns::new(serialize_reflect, deserialize_reflect));

    fn serialize_reflect(
        ctx: &SerializeCtx,
        component: &ReflectedComponent,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        let mut serializer = Serializer {
            output: ExtendMutFlavor::new(message),
        };
        let registry = ctx.type_registry.read();
        ReflectSerializer::new(&*component.0, &registry).serialize(&mut serializer)?;
        Ok(())
    }

    fn deserialize_reflect(
        ctx: &mut WriteCtx,
        message: &mut Bytes,
    ) -> Result<ReflectedComponent> {
        let mut deserializer = Deserializer::from_flavor(BufFlavor::new(message));
        let registry = ctx.type_registry.read();
        let reflect = ReflectDeserializer::new(&registry).deserialize(&mut deserializer)?;
        Ok(ReflectedComponent(reflect))
    }

    #[derive(Component)]
    struct ReflectedComponent(Box<dyn PartialReflect>);
    ```
    **/
    fn replicate_with<R: IntoReplicationRule>(&mut self, rule: R) -> &mut Self {
        self.replicate_with_priority(R::DEFAULT_PRIORITY, rule)
    }

    /// Same as [`Self::replicate_with`], but uses the specified priority instead of the default one.
    fn replicate_with_priority<R: IntoReplicationRule>(
        &mut self,
        priority: usize,
        rule: R,
    ) -> &mut Self;
}

impl AppRuleExt for App {
    fn replicate_with_priority<R: IntoReplicationRule>(
        &mut self,
        priority: usize,
        rule: R,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .replicate::<R>(priority);

        let rule =
            self.world_mut()
                .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                    rule.register(priority, world, &mut registry)
                });

        self.world_mut()
            .resource_mut::<ReplicationRules>()
            .insert(rule);

        self
    }

    fn replicate_bundle<B: ReplicationBundle>(&mut self) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .replicate_bundle::<B>();

        let rule =
            self.world_mut()
                .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                    B::register(world, &mut registry)
                });

        self.world_mut()
            .resource_mut::<ReplicationRules>()
            .insert(rule);

        self
    }
}

/// Parameters that can be turned into a replication rule.
///
/// Implemented for tuples of [`IntoComponentRule`].
///
/// See [`AppRuleExt::replicate_with`] for more details.
pub trait IntoReplicationRule {
    /// Priority when registered with [`AppRuleExt::replicate_with`].
    ///
    /// Equals the number of components in a rule.
    const DEFAULT_PRIORITY: usize;

    /// Turns into a replication rule and registers its functions in [`ReplicationRegistry`].
    fn register(
        self,
        priority: usize,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) -> ReplicationRule;
}

impl<C: IntoComponentRule> IntoReplicationRule for C {
    const DEFAULT_PRIORITY: usize = 1;

    fn register(
        self,
        priority: usize,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) -> ReplicationRule {
        ReplicationRule {
            priority,
            components: vec![self.register_component(world, registry)],
        }
    }
}

macro_rules! impl_into_replication_rule {
    ($(($n:tt, $C:ident)),*) => {
        impl<$($C: IntoComponentRule),*> IntoReplicationRule for ($($C,)*) {
            const DEFAULT_PRIORITY: usize = 0 $(+ { let _ = $n; 1 })*;

            fn register(self, priority: usize, world: &mut World, registry: &mut ReplicationRegistry) -> ReplicationRule {
                let components = vec![
                    $(
                        self.$n.register_component(world, registry),
                    )*
                ];

                ReplicationRule {
                    priority,
                    components,
                }
            }
        }
    }
}

variadics_please::all_tuples_enumerated!(impl_into_replication_rule, 1, 15, R);

/// Parameters for that can be turned into a component replication rule.
///
/// Used for [`IntoReplicationRule`] to accept either [`RuleFns`] or a tuple combining
/// [`RuleFns`] with an associated [`SendRate`].
///
/// See [`AppRuleExt::replicate_with`] for more details.
pub trait IntoComponentRule {
    /// Turns into a component replication rule and registers its functions in [`ReplicationRegistry`].
    fn register_component(
        self,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) -> ComponentRule;
}

impl<C: Component<Mutability: MutWrite<C>>> IntoComponentRule for RuleFns<C> {
    fn register_component(
        self,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) -> ComponentRule {
        let (id, fns_id) = registry.register_rule_fns(world, self);
        ComponentRule {
            id,
            fns_id,
            send_rate: Default::default(),
        }
    }
}

impl<C: Component<Mutability: MutWrite<C>>> IntoComponentRule for (RuleFns<C>, SendRate) {
    fn register_component(
        self,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) -> ComponentRule {
        let (rule_fns, send_rate) = self;
        let (id, fns_id) = registry.register_rule_fns(world, rule_fns);
        ComponentRule {
            id,
            fns_id,
            send_rate,
        }
    }
}

/// Replication rule associated with a bundle.
///
/// See [`AppRuleExt::replicate_bundle`] for more details.
pub trait ReplicationBundle {
    /// Creates the associated replication rules and registers its functions in [`ReplicationRegistry`].
    fn register(world: &mut World, registry: &mut ReplicationRegistry) -> ReplicationRule;
}

macro_rules! impl_replication_bundle {
    ($($C:ident),*) => {
        impl<$($C: Component<Mutability: MutWrite<$C>> + Serialize + DeserializeOwned),*> ReplicationBundle for ($($C,)*) {
            fn register(world: &mut World, registry: &mut ReplicationRegistry) -> ReplicationRule {
                let components = vec![
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
                ];

                ReplicationRule::new(components)
            }
        }
    }
}

variadics_please::all_tuples!(impl_replication_bundle, 1, 15, C);

/// All registered rules for components replication.
#[derive(Default, Deref, Resource, Clone)]
pub struct ReplicationRules(Vec<ReplicationRule>);

impl ReplicationRules {
    /// Inserts a new rule, maintaining sorting by their priority in descending order.
    fn insert(&mut self, rule: ReplicationRule) {
        match self.binary_search_by_key(&Reverse(rule.priority), |rule| Reverse(rule.priority)) {
            Ok(index) => {
                // Insert last to preserve entry creation order.
                let last_priority_index = self
                    .iter()
                    .skip(index + 1)
                    .position(|other| other.priority != rule.priority)
                    .unwrap_or_default();
                self.0.insert(index + last_priority_index + 1, rule);
            }
            Err(index) => self.0.insert(index, rule),
        }
    }
}

/// Describes how component(s) will be replicated.
#[derive(Clone, Debug)]
pub struct ReplicationRule {
    /// Priority for this rule.
    ///
    /// Usually equal to the number of serialized components,
    /// but can be adjusted by the user.
    pub priority: usize,

    /// Components for the rule.
    pub components: Vec<ComponentRule>,
}

impl ReplicationRule {
    /// Creates a new rule with priority equal to the number of serializable components.
    pub fn new(components: Vec<ComponentRule>) -> Self {
        Self {
            priority: components.len(),
            components,
        }
    }

    /// Determines whether an archetype contains all components required by the rule.
    pub(crate) fn matches(&self, archetype: &Archetype) -> bool {
        self.components
            .iter()
            .all(|component| archetype.contains(component.id))
    }

    /// Determines whether the rule is applicable to an archetype with removals included and contains at least one removal.
    ///
    /// Returns `true` if all components in this rule are found in either `removed_components` or the
    /// `post_removal_archetype`, and at least one component is found in `removed_components`.
    /// Returning true means the entity with this archetype satisfied this
    /// rule in the previous tick, but then a component within this rule was removed from the entity.
    pub(crate) fn matches_removals(
        &self,
        post_removal_archetype: &Archetype,
        removed_components: &HashSet<ComponentId>,
    ) -> bool {
        let mut matches = false;
        for component in &self.components {
            if removed_components.contains(&component.id) {
                matches = true;
            } else if !post_removal_archetype.contains(component.id) {
                return false;
            }
        }

        matches
    }
}

/// Component for [`ReplicationRule`].
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
    /// Returns `true` if a mutation for should be replicated on this tick.
    pub fn send_mutations(self, tick: RepliconTick) -> bool {
        match self {
            SendRate::EveryTick => true,
            SendRate::Once => false,
            SendRate::Periodic(period) => tick.get() % period == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[test]
    fn registration() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<A>()
            .replicate_with((RuleFns::<A>::default(), RuleFns::<B>::default()))
            .replicate_once::<B>()
            .replicate_with((
                (RuleFns::<B>::default(), SendRate::Once),
                (RuleFns::<C>::default(), SendRate::Periodic(2)),
            ))
            .replicate_periodic::<C>(1)
            .replicate_with_priority(4, (RuleFns::<C>::default(), RuleFns::<D>::default()))
            .replicate_with((RuleFns::<D>::default(), SendRate::Once))
            .replicate_bundle::<(A, B)>();

        let a = app.world().component_id::<A>().unwrap();
        let b = app.world().component_id::<B>().unwrap();
        let c = app.world().component_id::<C>().unwrap();
        let d = app.world().component_id::<D>().unwrap();

        let rules = &**app.world().resource::<ReplicationRules>();

        assert_eq!(rules[0].priority, 4);
        assert_eq!(rules[0].components[0].id, c);
        assert_eq!(rules[0].components[0].send_rate, SendRate::EveryTick);
        assert_eq!(rules[0].components[1].id, d);
        assert_eq!(rules[0].components[1].send_rate, SendRate::EveryTick);

        assert_eq!(rules[1].priority, 2);
        assert_eq!(rules[1].components[0].id, a);
        assert_eq!(rules[1].components[0].send_rate, SendRate::EveryTick);
        assert_eq!(rules[1].components[1].id, b);
        assert_eq!(rules[1].components[1].send_rate, SendRate::EveryTick);

        assert_eq!(rules[2].priority, 2);
        assert_eq!(rules[2].components[0].id, b);
        assert_eq!(rules[2].components[0].send_rate, SendRate::Once);
        assert_eq!(rules[2].components[1].id, c);
        assert_eq!(rules[2].components[1].send_rate, SendRate::Periodic(2));

        assert_eq!(rules[3].priority, 2);
        assert_eq!(rules[3].components[0].id, a);
        assert_eq!(rules[3].components[0].send_rate, SendRate::EveryTick);
        assert_eq!(rules[3].components[1].id, b);
        assert_eq!(rules[3].components[1].send_rate, SendRate::EveryTick);

        assert_eq!(rules[4].priority, 1);
        assert_eq!(rules[4].components[0].id, a);
        assert_eq!(rules[4].components[0].send_rate, SendRate::EveryTick);

        assert_eq!(rules[5].priority, 1);
        assert_eq!(rules[5].components[0].id, b);
        assert_eq!(rules[5].components[0].send_rate, SendRate::Once);

        assert_eq!(rules[6].priority, 1);
        assert_eq!(rules[6].components[0].id, c);
        assert_eq!(rules[6].components[0].send_rate, SendRate::Periodic(1));

        assert_eq!(rules[7].priority, 1);
        assert_eq!(rules[7].components[0].id, d);
        assert_eq!(rules[7].components[0].send_rate, SendRate::Once);
    }

    #[derive(Serialize, Deserialize, Component)]
    struct A;

    #[derive(Serialize, Deserialize, Component)]
    struct B;

    #[derive(Serialize, Deserialize, Component)]
    struct C;

    #[derive(Serialize, Deserialize, Component)]
    struct D;
}
