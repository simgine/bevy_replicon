pub mod component;
pub mod filter;

use core::cmp::Reverse;

use bevy::{
    ecs::{archetype::Archetype, component::ComponentId},
    platform::collections::HashSet,
    prelude::*,
};
use serde::{Serialize, de::DeserializeOwned};

use super::registry::{ReplicationRegistry, command_fns::MutWrite};
use crate::prelude::*;
use component::{BundleRules, ComponentRule, IntoComponentRules};
use filter::{FilterRule, FilterRules};

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
        self.replicate_filtered::<C, ()>()
    }

    /// Like [`Self::replicate`], but uses [`SendRate::Once`].
    fn replicate_once<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_once_filtered::<C, ()>()
    }

    /// Like [`Self::replicate`], but uses [`SendRate::Periodic`] with the given tick period.
    fn replicate_periodic<C>(&mut self, period: u32) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_periodic_filtered::<C, ()>(period)
    }

    /// Like [`Self::replicate`], but lets you specify archetype filters an entity must match to replicate.
    ///
    /// Supports [`With`], [`Without`], [`Or`], and tuples of them, similar to the second generic parameter of [`Query`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use bevy::prelude::*;
    /// # use bevy_replicon::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # let mut app = App::new();
    /// # app.add_plugins(RepliconPlugins);
    /// app.replicate_filtered::<Transform, With<Player>>() // Replicate `Transform` only for players.
    ///     .replicate_filtered::<Health, Or<(With<Player>, With<Enemy>)>>() // Replicate `Health` only for player and enemies.
    ///     .replicate_filtered::<Platform, (With<Active>, Without<Moving>)>(); // Replicate only active and non-moving platforms.
    /// # #[derive(Component)]
    /// # struct Player;
    /// # #[derive(Component)]
    /// # struct Enemy;
    /// # #[derive(Component, Serialize, Deserialize)]
    /// # struct Health;
    /// # #[derive(Component, Serialize, Deserialize)]
    /// # struct Platform;
    /// # #[derive(Component)]
    /// # struct Moving;
    /// # #[derive(Component)]
    /// # struct Active;
    /// ```
    fn replicate_filtered<C, F: FilterRules>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_with_filtered::<_, F>(RuleFns::<C>::default())
    }

    /// Like [`Self::replicate_filtered`], but for [`Self::replicate_once`].
    fn replicate_once_filtered<C, F: FilterRules>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_with_filtered::<_, F>((RuleFns::<C>::default(), SendRate::Once))
    }

    /// Like [`Self::replicate_filtered`], but for [`Self::replicate_periodic`].
    fn replicate_periodic_filtered<C, F: FilterRules>(&mut self, period: u32) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_with_filtered::<_, F>((RuleFns::<C>::default(), SendRate::Periodic(period)))
    }

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
    fn replicate_with<R: IntoComponentRules>(&mut self, component_rules: R) -> &mut Self {
        self.replicate_with_filtered::<_, ()>(component_rules)
    }

    /// Like [`Self::replicate_filtered`], but for [`Self::replicate_with`].
    ///
    /// It’s recommended to omit the first parameter and let the compiler infer it from the arguments.
    ///
    /// # Examples
    ///
    /// ```
    /// # use bevy::prelude::*;
    /// # use bevy_replicon::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # let mut app = App::new();
    /// # app.add_plugins(RepliconPlugins);
    /// app.replicate_with_filtered::<_, With<StaticBox>>((
    ///     RuleFns::<Health>::default(),
    ///     (RuleFns::<Transform>::default(), SendRate::Once),
    /// ));
    /// # #[derive(Component)]
    /// # struct StaticBox;
    /// # #[derive(Component, Serialize, Deserialize)]
    /// # struct Health;
    /// ```
    fn replicate_with_filtered<R: IntoComponentRules, F: FilterRules>(
        &mut self,
        component_rules: R,
    ) -> &mut Self {
        self.replicate_with_priority_filtered::<_, F>(
            R::DEFAULT_PRIORITY + F::DEFAULT_PRIORITY,
            component_rules,
        )
    }

    /// Same as [`Self::replicate_with`], but uses the specified priority instead of the default one.
    ///
    /// The default priority equals the total number of components in the rule
    fn replicate_with_priority<R: IntoComponentRules>(
        &mut self,
        priority: usize,
        component_rules: R,
    ) -> &mut Self {
        self.replicate_with_priority_filtered::<_, ()>(priority, component_rules)
    }

    /// Like [`Self::replicate_filtered`], but for [`Self::replicate_with_priority`].
    ///
    /// The default priority equals the total number of components **and** filters in the rule
    fn replicate_with_priority_filtered<R: IntoComponentRules, F: FilterRules>(
        &mut self,
        priority: usize,
        component_rules: R,
    ) -> &mut Self;

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
            rules::component::{BundleRules, ComponentRule},
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

    impl BundleRules for PlayerBundle {
        const DEFAULT_PRIORITY: usize = 2; // Usually equals to the number of components, but can be customized.

        fn component_rules(world: &mut World, registry: &mut ReplicationRegistry) -> Vec<ComponentRule> {
            // Customize serlialization to serialize only `translation`.
            let (transform_id, transform_fns_id) = registry.register_rule_fns(
                world,
                RuleFns::new(serialize_translation, deserialize_translation),
            );
            let transform = ComponentRule::new(transform_id, transform_fns_id);

            // Serialize `player` as usual.
            let (player_id, player_fns_id) = registry.register_rule_fns(world, RuleFns::<Player>::default());
            let player = ComponentRule::new(player_id, player_fns_id);

            // We skip `replication` registration since it's a special component.
            // It's automatically inserted on clients after replication and
            // deserialization from scenes.

            vec![transform, player]
        }
    }

    # fn serialize_translation(_: &SerializeCtx, _: &Transform, _: &mut Vec<u8>) -> Result<()> { unimplemented!() }
    # fn deserialize_translation(_: &mut WriteCtx, _: &mut Bytes) -> Result<Transform> { unimplemented!() }
    ```
    **/
    fn replicate_bundle<B: BundleRules>(&mut self) -> &mut Self {
        self.replicate_bundle_filtered::<B, ()>()
    }

    fn replicate_bundle_filtered<B: BundleRules, F: FilterRules>(&mut self) -> &mut Self {
        self.replicate_bundle_with_filtered::<B, F>(B::DEFAULT_PRIORITY + F::DEFAULT_PRIORITY)
    }

    fn replicate_bundle_with<B: BundleRules>(&mut self, priority: usize) -> &mut Self {
        self.replicate_bundle_with_filtered::<B, ()>(priority)
    }

    fn replicate_bundle_with_filtered<B: BundleRules, F: FilterRules>(
        &mut self,
        priority: usize,
    ) -> &mut Self;
}

impl AppRuleExt for App {
    fn replicate_with_priority_filtered<R: IntoComponentRules, F: FilterRules>(
        &mut self,
        priority: usize,
        component_rules: R,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .replicate::<R>(priority);

        let components =
            self.world_mut()
                .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                    component_rules.into_rules(world, &mut registry)
                });

        let filters = F::filter_rules(self.world_mut());

        self.world_mut()
            .resource_mut::<ReplicationRules>()
            .insert(ReplicationRule {
                priority,
                components,
                filters,
            });

        self
    }

    fn replicate_bundle_with_filtered<B: BundleRules, F: FilterRules>(
        &mut self,
        priority: usize,
    ) -> &mut Self {
        self.world_mut()
            .resource_mut::<ProtocolHasher>()
            .replicate_bundle::<B>();

        let components =
            self.world_mut()
                .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                    B::component_rules(world, &mut registry)
                });

        let filters = F::filter_rules(self.world_mut());

        self.world_mut()
            .resource_mut::<ReplicationRules>()
            .insert(ReplicationRule {
                priority,
                components,
                filters,
            });

        self
    }
}

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
///
/// Created using methods from [`AppRuleExt`].
#[derive(Clone, Debug)]
pub struct ReplicationRule {
    /// Priority for this rule.
    ///
    /// Usually equal to the number of serialized components,
    /// but can be adjusted by the user.
    pub priority: usize,

    /// Components for the rule.
    pub components: Vec<ComponentRule>,

    /// Associated filters.
    pub filters: Vec<FilterRule>,
}

impl ReplicationRule {
    /// Determines whether an archetype contains all components required by the rule.
    #[must_use]
    pub(crate) fn matches(&self, archetype: &Archetype) -> bool {
        if !self.filters.iter().all(|filter| filter.matches(archetype)) {
            return false;
        }

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

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[test]
    fn single() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate::<A>()
            .replicate_once::<B>()
            .replicate_periodic::<C>(1);

        let rules = app
            .world_mut()
            .remove_resource::<ReplicationRules>()
            .unwrap();
        let [rule_a, rule_b, rule_c] = rules.0.try_into().unwrap();
        assert_eq!(rule_a.priority, 1);
        assert_eq!(rule_b.priority, 1);
        assert_eq!(rule_c.priority, 1);

        let a = app.world_mut().spawn(A).archetype().id();
        let b = app.world_mut().spawn(B).archetype().id();
        let c = app.world_mut().spawn(C).archetype().id();

        let a = app.world().archetypes().get(a).unwrap();
        let b = app.world().archetypes().get(b).unwrap();
        let c = app.world().archetypes().get(c).unwrap();

        assert!(rule_a.matches(a));
        assert!(!rule_a.matches(b));
        assert!(!rule_a.matches(c));

        assert!(!rule_b.matches(a));
        assert!(rule_b.matches(b));
        assert!(!rule_b.matches(c));

        assert!(!rule_c.matches(a));
        assert!(!rule_c.matches(b));
        assert!(rule_c.matches(c));
    }

    #[test]
    fn single_filtered() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate_filtered::<A, With<B>>()
            .replicate_once_filtered::<B, Or<(With<A>, With<C>)>>()
            .replicate_periodic_filtered::<C, (Without<A>, With<B>)>(1);

        let rules = app
            .world_mut()
            .remove_resource::<ReplicationRules>()
            .unwrap();
        let [rule_b_ac, rule_c_ab, rule_a_b] = rules.0.try_into().unwrap();
        assert_eq!(rule_b_ac.priority, 3);
        assert_eq!(rule_c_ab.priority, 3);
        assert_eq!(rule_a_b.priority, 2);

        let abc = app.world_mut().spawn((A, B, C)).archetype().id();
        let bcd = app.world_mut().spawn((B, C, D)).archetype().id();
        let cda = app.world_mut().spawn((C, D, A)).archetype().id();

        let abc = app.world().archetypes().get(abc).unwrap();
        let bcd = app.world().archetypes().get(bcd).unwrap();
        let cda = app.world().archetypes().get(cda).unwrap();

        assert!(rule_b_ac.matches(abc));
        assert!(rule_b_ac.matches(bcd));
        assert!(!rule_b_ac.matches(cda));

        assert!(!rule_c_ab.matches(abc));
        assert!(rule_c_ab.matches(bcd));
        assert!(!rule_c_ab.matches(cda));

        assert!(rule_a_b.matches(abc));
        assert!(!rule_a_b.matches(bcd));
        assert!(!rule_a_b.matches(cda));
    }

    #[test]
    fn with() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate_with((RuleFns::<A>::default(), SendRate::Once))
            .replicate_with((RuleFns::<B>::default(), RuleFns::<C>::default()))
            .replicate_with_priority(
                4,
                (
                    RuleFns::<C>::default(),
                    (RuleFns::<D>::default(), SendRate::Periodic(1)),
                ),
            );

        let rules = app
            .world_mut()
            .remove_resource::<ReplicationRules>()
            .unwrap();
        let [rule_cd, rule_bc, rule_a] = rules.0.try_into().unwrap();
        assert_eq!(rule_cd.priority, 4);
        assert_eq!(rule_bc.priority, 2);
        assert_eq!(rule_a.priority, 1);

        let abc = app.world_mut().spawn((A, B, C)).archetype().id();
        let bcd = app.world_mut().spawn((B, C, D)).archetype().id();
        let cda = app.world_mut().spawn((C, D, A)).archetype().id();

        let abc = app.world().archetypes().get(abc).unwrap();
        let bcd = app.world().archetypes().get(bcd).unwrap();
        let cda = app.world().archetypes().get(cda).unwrap();

        assert!(!rule_cd.matches(abc));
        assert!(rule_cd.matches(bcd));
        assert!(rule_cd.matches(cda));

        assert!(rule_bc.matches(abc));
        assert!(rule_bc.matches(bcd));
        assert!(!rule_bc.matches(cda));

        assert!(rule_a.matches(abc));
        assert!(!rule_a.matches(bcd));
        assert!(rule_a.matches(cda));
    }

    #[test]
    fn with_filtered() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate_with_filtered::<_, With<B>>((RuleFns::<A>::default(), SendRate::Once))
            .replicate_with_filtered::<_, Or<(With<A>, With<D>)>>((
                RuleFns::<B>::default(),
                RuleFns::<C>::default(),
            ))
            .replicate_with_priority_filtered::<_, (Without<A>, With<B>)>(
                5,
                (
                    RuleFns::<C>::default(),
                    (RuleFns::<D>::default(), SendRate::Periodic(1)),
                ),
            );

        let rules = app
            .world_mut()
            .remove_resource::<ReplicationRules>()
            .unwrap();
        let [rule_cd_ab, rule_bc_ad, rule_a_b] = rules.0.try_into().unwrap();
        assert_eq!(rule_cd_ab.priority, 5);
        assert_eq!(rule_bc_ad.priority, 4);
        assert_eq!(rule_a_b.priority, 2);

        let abc = app.world_mut().spawn((A, B, C)).archetype().id();
        let bcd = app.world_mut().spawn((B, C, D)).archetype().id();
        let cda = app.world_mut().spawn((C, D, A)).archetype().id();

        let abc = app.world().archetypes().get(abc).unwrap();
        let bcd = app.world().archetypes().get(bcd).unwrap();
        let cda = app.world().archetypes().get(cda).unwrap();

        assert!(!rule_cd_ab.matches(abc));
        assert!(rule_cd_ab.matches(bcd));
        assert!(!rule_cd_ab.matches(cda));

        assert!(rule_bc_ad.matches(abc));
        assert!(rule_bc_ad.matches(bcd));
        assert!(!rule_bc_ad.matches(cda));

        assert!(rule_a_b.matches(abc));
        assert!(!rule_a_b.matches(bcd));
        assert!(!rule_a_b.matches(cda));
    }

    #[test]
    fn bundle() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate_bundle::<(A, B)>()
            .replicate_bundle::<(B, C)>()
            .replicate_bundle_with::<(C, D)>(4);

        let rules = app
            .world_mut()
            .remove_resource::<ReplicationRules>()
            .unwrap();
        let [rule_cd, rule_ab, rule_bc] = rules.0.try_into().unwrap();
        assert_eq!(rule_cd.priority, 4);
        assert_eq!(rule_ab.priority, 2);
        assert_eq!(rule_bc.priority, 2);

        let abc = app.world_mut().spawn((A, B, C)).archetype().id();
        let bcd = app.world_mut().spawn((B, C, D)).archetype().id();
        let cda = app.world_mut().spawn((C, D, A)).archetype().id();

        let abc = app.world().archetypes().get(abc).unwrap();
        let bcd = app.world().archetypes().get(bcd).unwrap();
        let cda = app.world().archetypes().get(cda).unwrap();

        assert!(!rule_cd.matches(abc));
        assert!(rule_cd.matches(bcd));
        assert!(rule_cd.matches(cda));

        assert!(rule_ab.matches(abc));
        assert!(!rule_ab.matches(bcd));
        assert!(!rule_ab.matches(cda));

        assert!(rule_bc.matches(abc));
        assert!(rule_bc.matches(bcd));
        assert!(!rule_bc.matches(cda));
    }

    #[test]
    fn bundle_filtered() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate_bundle_filtered::<(A, B), With<C>>()
            .replicate_bundle_filtered::<(B, C), Or<(With<A>, With<D>)>>()
            .replicate_bundle_with_filtered::<(C, D), (Without<A>, With<B>)>(5);

        let rules = app
            .world_mut()
            .remove_resource::<ReplicationRules>()
            .unwrap();
        let [rule_cd_ab, rule_bc_ad, rule_ab_c] = rules.0.try_into().unwrap();
        assert_eq!(rule_cd_ab.priority, 5);
        assert_eq!(rule_bc_ad.priority, 4);
        assert_eq!(rule_ab_c.priority, 3);

        let abc = app.world_mut().spawn((A, B, C)).archetype().id();
        let bcd = app.world_mut().spawn((B, C, D)).archetype().id();
        let cda = app.world_mut().spawn((C, D, A)).archetype().id();

        let abc = app.world().archetypes().get(abc).unwrap();
        let bcd = app.world().archetypes().get(bcd).unwrap();
        let cda = app.world().archetypes().get(cda).unwrap();

        assert!(!rule_cd_ab.matches(abc));
        assert!(rule_cd_ab.matches(bcd));
        assert!(!rule_cd_ab.matches(cda));

        assert!(rule_bc_ad.matches(abc));
        assert!(rule_bc_ad.matches(bcd));
        assert!(!rule_bc_ad.matches(cda));

        assert!(rule_ab_c.matches(abc));
        assert!(!rule_ab_c.matches(bcd));
        assert!(!rule_ab_c.matches(cda));
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
