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
    /// and sent at [`ReplicationMode::OnChange`]. To customize this, use [`Self::replicate_with`].
    ///
    /// See also the section on [`components`](../../index.html#components) from the quick start guide.
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_filtered::<C, ()>()
    }

    /// Like [`Self::replicate`], but uses [`ReplicationMode::Once`].
    fn replicate_once<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        self.replicate_once_filtered::<C, ()>()
    }

    /// Like [`Self::replicate`], but converts the component into `T` before serialization
    /// and back into `C` after deserialization.
    ///
    /// Useful for customizing how the component is sent over the network.
    /// In some cases, this is more convenient than passing custom ser/de functions
    /// with [`Self::replicate_with`], because you only need to implement
    /// [`From<C>`] for `T` and [`From<T>`] for `C`.
    ///
    /// # Examples
    ///
    /// Quantize position:
    ///
    /// ```
    /// # use bevy::state::app::StatesPlugin;
    /// use bevy::{math::I16Vec2, prelude::*};
    /// use bevy_replicon::prelude::*;
    /// use serde::{Deserialize, Serialize};
    ///
    /// # let mut app = App::new();
    /// # app.add_plugins((StatesPlugin, RepliconPlugins));
    /// app.replicate_as::<Position, QuantizedPosition>();
    ///
    /// #[derive(Component, Deref, Clone, Copy)]
    /// struct Position(Vec2);
    ///
    /// /// Quantized representation of [`Position`] sent over the network.
    /// #[derive(Deref, Serialize, Deserialize)]
    /// struct QuantizedPosition(I16Vec2);
    ///
    /// /// Scale factor for quantizing.
    /// ///
    /// /// Each unit in world space is multiplied by this factor before rounding.
    /// /// With this scale we keep two decimal places of precision (0.01 units).
    /// /// The representable range is from [`i16::MIN`] to [`i16::MAX`] divided by this value,
    /// /// which is `-327.68..=327.67` per axis. Values outside this range will overflow,
    /// /// so world positions should stay within it.
    /// const SCALE: f32 = 100.0;
    ///
    /// impl From<Position> for QuantizedPosition {
    ///     fn from(position: Position) -> Self {
    ///         Self((*position * SCALE).round().as_i16vec2())
    ///     }
    /// }
    ///
    /// impl From<QuantizedPosition> for Position {
    ///     fn from(position: QuantizedPosition) -> Self {
    ///         Position(position.as_vec2() / SCALE)
    ///     }
    /// }
    ///
    /// ```
    ///
    /// Ignore scale.
    ///
    /// This will overwrite the scale value with the default.
    /// If you want to preserve it, use [`Self::replicate_with`] to provide
    /// in-place deserialization.
    ///
    /// ```
    /// # use bevy::state::app::StatesPlugin;
    /// use bevy::prelude::*;
    /// use bevy_replicon::prelude::*;
    /// use serde::{Deserialize, Serialize};
    ///
    /// # let mut app = App::new();
    /// # app.add_plugins((StatesPlugin, RepliconPlugins));
    /// app.replicate_as::<Transform, TransformWithoutScale>();
    ///
    /// #[derive(Serialize, Deserialize, Clone, Copy)]
    /// struct TransformWithoutScale {
    ///     translation: Vec3,
    ///     rotation: Quat,
    /// }
    ///
    /// impl From<Transform> for TransformWithoutScale {
    ///     fn from(value: Transform) -> Self {
    ///         Self {
    ///             translation: value.translation,
    ///             rotation: value.rotation,
    ///         }
    ///     }
    /// }
    ///
    /// impl From<TransformWithoutScale> for Transform {
    ///     fn from(value: TransformWithoutScale) -> Self {
    ///         Self {
    ///             translation: value.translation,
    ///             rotation: value.rotation,
    ///             ..Default::default()
    ///         }
    ///     }
    /// }
    /// ```
    fn replicate_as<C, T>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Clone + Into<T> + From<T>,
        T: Serialize + DeserializeOwned,
    {
        self.replicate_filtered_as::<C, T, ()>()
    }

    /// Like [`Self::replicate_as`], but uses [`ReplicationMode::Once`].
    fn replicate_once_as<C, T>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Clone + Into<T> + From<T>,
        T: Serialize + DeserializeOwned,
    {
        self.replicate_once_filtered_as::<C, T, ()>()
    }

    /// Like [`Self::replicate`], but lets you specify archetype filters an entity must match to replicate.
    ///
    /// Supports [`With`], [`Without`], [`Or`], and tuples of them, similar to the second generic parameter of [`Query`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use bevy::{prelude::*, state::app::StatesPlugin};
    /// # use bevy_replicon::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # let mut app = App::new();
    /// # app.add_plugins((StatesPlugin, RepliconPlugins));
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
        self.replicate_with_filtered::<_, F>((RuleFns::<C>::default(), ReplicationMode::Once))
    }

    /// Like [`Self::replicate_as`], but also adds filters like [`Self::replicate_filtered`].
    fn replicate_filtered_as<C, T, F: FilterRules>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Clone + Into<T> + From<T>,
        T: Serialize + DeserializeOwned,
    {
        self.replicate_with_filtered::<_, F>(RuleFns::<C>::new_as::<T>())
    }

    /// Like [`Self::replicate_filtered_as`], but for [`Self::replicate_once`].
    fn replicate_once_filtered_as<C, T, F: FilterRules>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Clone + Into<T> + From<T>,
        T: Serialize + DeserializeOwned,
    {
        self.replicate_with_filtered::<_, F>((RuleFns::<C>::new_as::<T>(), ReplicationMode::Once))
    }

    /**
    Defines a customizable [`ReplicationRule`].

    Can be used to customize how a component is passed over the network, or
    for components that don't implement [`Serialize`] or [`DeserializeOwned`].

    You can also pass a tuple of [`RuleFns`] to define a rule for multiple components.
    These components will only be replicated if all of them are present on the entity.
    To assign a [`ReplicationMode`] to a component, wrap its [`RuleFns`] in a tuple with the
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

    Skip scale serialization.

    Unlike with the example from [`Self::replicate_as`], this
    will preserve the original scale value on deserialiation.

    ```
    # use bevy::state::app::StatesPlugin;
    use bevy::prelude::*;
    use bevy_replicon::{
        bytes::Bytes,
        prelude::*,
        shared::replication::registry::{ctx::WriteCtx, rule_fns::DeserializeFn},
    };
    use serde::{Deserialize, Serialize};

    # let mut app = App::new();
    # app.add_plugins((StatesPlugin, RepliconPlugins));
    app.replicate_with(
        RuleFns::<Transform>::new_as::<TransformWithoutScale>()
            .with_in_place(deserialize_in_place_without_scale),
    );

    #[derive(Serialize, Deserialize, Clone, Copy)]
    struct TransformWithoutScale {
        translation: Vec3,
        rotation: Quat,
    }

    impl From<Transform> for TransformWithoutScale {
        fn from(value: Transform) -> Self {
            Self {
                translation: value.translation,
                rotation: value.rotation,
            }
        }
    }

    impl From<TransformWithoutScale> for Transform {
        fn from(value: TransformWithoutScale) -> Self {
            Self {
                translation: value.translation,
                rotation: value.rotation,
                ..Default::default()
            }
        }
    }

    /// Applies the assigned deserialization function and assigns only translation and rotation.
    ///
    /// Called by Replicon on component mutations.
    fn deserialize_in_place_without_scale(
        deserialize: DeserializeFn<Transform>,
        ctx: &mut WriteCtx,
        component: &mut Transform,
        message: &mut Bytes,
    ) -> Result<()> {
        let transform = (deserialize)(ctx, message)?;
        component.translation = transform.translation;
        component.rotation = transform.rotation;
        Ok(())
    }

    ```

    A rule with multiple components:

    ```
    # use bevy::state::app::StatesPlugin;
    use bevy::prelude::*;
    use bevy_replicon::prelude::*;
    use serde::{Deserialize, Serialize};

    # let mut app = App::new();
    # app.add_plugins((StatesPlugin, RepliconPlugins));
    app.replicate_with((
        // You can also use `replicate_bundle` if you don't want
        // to tweak functions or send rate.
        RuleFns::<Player>::default(),
        RuleFns::<Position>::default(),
    ))
    .replicate_with((
        RuleFns::<MovingPlatform>::default(),
        // Send position only once.
        (RuleFns::<Position>::default(), ReplicationMode::Once),
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
    # use bevy::state::app::StatesPlugin;
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
    # app.add_plugins((StatesPlugin, RepliconPlugins));
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
    # use bevy::state::app::StatesPlugin;
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

    # let mut app = App::new();
    # app.add_plugins((StatesPlugin, RepliconPlugins));
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
    # use bevy::state::app::StatesPlugin;
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

    # let mut app = App::new();
    # app.add_plugins((StatesPlugin, RepliconPlugins));
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

    Component with regular fields and [`Box<dyn PartialReflect>`]. Requires writing manual serde
    implementations. See [serde book](https://serde.rs/custom-serialization.html) for more details.

    ```
    use std::{
        any,
        fmt::{self, Formatter},
    };

    # use bevy::state::app::StatesPlugin;
    use bevy::{
        prelude::*,
        reflect::{
            TypeRegistry,
            serde::{ReflectDeserializer, ReflectSerializer},
        },
    };
    use bevy_replicon::{
        bytes::Bytes,
        postcard,
        postcard_utils::{BufFlavor, ExtendMutFlavor},
        prelude::*,
        shared::replication::registry::{
            ctx::{SerializeCtx, WriteCtx},
            rule_fns::RuleFns,
        },
    };
    use serde::{
        Deserialize, Serialize,
        de::{self, DeserializeSeed, MapAccess, Visitor},
        ser::SerializeStruct,
    };

    # let mut app = App::new();
    # app.add_plugins((StatesPlugin, RepliconPlugins));
    app.replicate_with(RuleFns::new(serialize_reflect, deserialize_reflect));

    fn serialize_reflect(
        ctx: &SerializeCtx,
        component: &WithReflectComponent,
        message: &mut Vec<u8>,
    ) -> Result<()> {
        let mut serializer = postcard::Serializer {
            output: ExtendMutFlavor::new(message),
        };
        let reflect_serializer = WithReflectSerializer {
            component,
            registry: &ctx.type_registry.read(),
        };
        reflect_serializer.serialize(&mut serializer)?;
        Ok(())
    }

    fn deserialize_reflect(
        ctx: &mut WriteCtx,
        message: &mut Bytes,
    ) -> Result<WithReflectComponent> {
        let mut deserializer = postcard::Deserializer::from_flavor(BufFlavor::new(message));
        let reflect_deserializer = WithReflectDeserializer {
            registry: &ctx.type_registry.read(),
        };
        let component = reflect_deserializer.deserialize(&mut deserializer)?;
        Ok(component)
    }

    #[derive(Component)]
    struct WithReflectComponent {
        regular: String,
        reflect: Box<dyn PartialReflect>,
    }
    #[derive(Deserialize)]
    #[serde(field_identifier, rename_all = "lowercase")]
    enum WithReflectField {
        Regular,
        Reflect,
    }

    struct WithReflectSerializer<'a> {
        component: &'a WithReflectComponent,
        registry: &'a TypeRegistry,
    }

    impl serde::Serialize for WithReflectSerializer<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut state =
                serializer.serialize_struct(any::type_name::<WithReflectComponent>(), 3)?;
            state.serialize_field("regular", &self.component.regular)?;
            state.serialize_field(
                "reflect",
                &ReflectSerializer::new(&*self.component.reflect, self.registry),
            )?;

            state.end()
        }
    }

    struct WithReflectDeserializer<'a> {
        registry: &'a TypeRegistry,
    }

    impl<'de> DeserializeSeed<'de> for WithReflectDeserializer<'_> {
        type Value = WithReflectComponent;

        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_struct(
                any::type_name::<WithReflectComponent>(),
                &["regular", "reflect"],
                self,
            )
        }
    }

    impl<'de> Visitor<'de> for WithReflectDeserializer<'_> {
        type Value = WithReflectComponent;

        fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
            formatter.write_str(any::type_name::<Self::Value>())
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut regular = None;
            let mut reflect = None;
            while let Some(key) = map.next_key()? {
                match key {
                    WithReflectField::Regular => {
                        if regular.is_some() {
                            return Err(de::Error::duplicate_field("regular"));
                        }
                        regular = Some(map.next_value()?);
                    }
                    WithReflectField::Reflect => {
                        if reflect.is_some() {
                            return Err(de::Error::duplicate_field("reflect"));
                        }
                        reflect =
                            Some(map.next_value_seed(ReflectDeserializer::new(self.registry))?);
                    }
                }
            }
            let regular = regular.ok_or_else(|| de::Error::missing_field("regular"))?;
            let reflect = reflect.ok_or_else(|| de::Error::missing_field("reflect"))?;
            Ok(WithReflectComponent { regular, reflect })
        }
    }
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
    /// # use bevy::{prelude::*, state::app::StatesPlugin};
    /// # use bevy_replicon::prelude::*;
    /// # use serde::{Deserialize, Serialize};
    /// # let mut app = App::new();
    /// # app.add_plugins((StatesPlugin, RepliconPlugins));
    /// app.replicate_with_filtered::<_, With<StaticBox>>((
    ///     RuleFns::<Health>::default(),
    ///     (RuleFns::<Transform>::default(), ReplicationMode::Once),
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
    default ser/de functions and [`ReplicationMode::OnChange`] for all components.
    To customize this, use [`Self::replicate_with`].

    Can also be implemented manually for user-defined bundles.

    # Examples

    ```
    # use bevy::state::app::StatesPlugin;
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
    # app.add_plugins((StatesPlugin, RepliconPlugins));
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
            .replicate_as::<A, C>()
            .replicate_once_as::<B, D>();

        let rules = app
            .world_mut()
            .remove_resource::<ReplicationRules>()
            .unwrap();
        let [rule_a, rule_b, rule_a_as_b, rule_b_as_d] = rules.0.try_into().unwrap();
        assert_eq!(rule_a.priority, 1);
        assert_eq!(rule_b.priority, 1);

        let a = app.world_mut().spawn(A).archetype().id();
        let b = app.world_mut().spawn(B).archetype().id();

        let a = app.world().archetypes().get(a).unwrap();
        let b = app.world().archetypes().get(b).unwrap();

        assert!(rule_a.matches(a));
        assert!(!rule_a.matches(b));

        assert!(!rule_b.matches(a));
        assert!(rule_b.matches(b));

        assert!(rule_a_as_b.matches(a));
        assert!(!rule_a_as_b.matches(b));

        assert!(!rule_b_as_d.matches(a));
        assert!(rule_b_as_d.matches(b));
    }

    #[test]
    fn single_filtered() {
        let mut app = App::new();
        app.init_resource::<ProtocolHasher>()
            .init_resource::<ReplicationRules>()
            .init_resource::<ReplicationRegistry>()
            .replicate_filtered::<A, With<B>>()
            .replicate_filtered::<B, Or<(With<A>, With<C>)>>()
            .replicate_once_filtered::<C, (Without<A>, With<B>)>();

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
            .replicate_with((RuleFns::<A>::default(), ReplicationMode::Once))
            .replicate_with((RuleFns::<B>::default(), RuleFns::<C>::default()))
            .replicate_with_priority(
                4,
                (
                    RuleFns::<C>::default(),
                    (RuleFns::<D>::default(), ReplicationMode::Once),
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
            .replicate_with_filtered::<_, With<B>>((RuleFns::<A>::default(), ReplicationMode::Once))
            .replicate_with_filtered::<_, Or<(With<A>, With<D>)>>((
                RuleFns::<B>::default(),
                RuleFns::<C>::default(),
            ))
            .replicate_with_priority_filtered::<_, (Without<A>, With<B>)>(
                5,
                (
                    RuleFns::<C>::default(),
                    (RuleFns::<D>::default(), ReplicationMode::Once),
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

    #[derive(Component, Serialize, Deserialize, Clone, Copy)]
    struct A;

    #[derive(Component, Serialize, Deserialize, Clone, Copy)]
    struct B;

    #[derive(Component, Serialize, Deserialize)]
    struct C;

    impl From<C> for A {
        fn from(_value: C) -> Self {
            A
        }
    }

    impl From<A> for C {
        fn from(_value: A) -> Self {
            C
        }
    }

    #[derive(Component, Serialize, Deserialize)]
    struct D;

    impl From<D> for B {
        fn from(_value: D) -> Self {
            B
        }
    }

    impl From<B> for D {
        fn from(_value: B) -> Self {
            D
        }
    }
}
