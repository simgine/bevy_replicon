use core::marker::PhantomData;

use bevy::{ecs::component::Immutable, prelude::*};

use crate::shared::replication::registry::{
    ReplicationRegistry, component_mask::ComponentMask, receive_fns::MutWrite,
};

/// Component that controls remote entity visibility.
///
/// Should be registered via [`crate::server::visibility::AppVisibilityExt`].
pub trait VisibilityFilter: Component<Mutability = Immutable> {
    /**
    Component on the client entity that will be passed to [`Self::is_visible`].

    # Examples

    Different component for the client and replicated entities:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component)]
    #[component(immutable)]
    struct Moderator;

    #[derive(Component)]
    #[component(immutable)]
    struct SensitiveInfo;

    impl VisibilityFilter for SensitiveInfo {
        type ClientComponent = Moderator;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            // Only moderators can see entities with sensitive information.
            component.is_some()
        }
    }
    ```

    You can use `Self` to check for same component on both the client and replicated entities:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component, PartialEq)]
    #[component(immutable)]
    struct SpectatorOnly;

    impl VisibilityFilter for SpectatorOnly {
        type ClientComponent = Self;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            // Visible only if the client also has `SpectatorOnly`.
            component.is_some()
        }
    }
    ```
     */
    type ClientComponent: Component<Mutability = Immutable>;

    /**
    Defines what data is affected when the filter denies visibility.

    - To hide the entire entity, this type must be [`Entity`].
    - To hide a single component on the entity, this type must be [`SingleComponent`].
    - To hide more than one component on the entity, this type must be a tuple of those [`Component`]s.

    # Examples

    Hide the entire entity:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component, PartialEq)]
    #[component(immutable)]
    struct Team(u8);

    impl VisibilityFilter for Team {
        type ClientComponent = Self;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some_and(|c| self == c)
        }
    }
    ```

    Hide only a single component:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component, PartialEq)]
    #[component(immutable)]
    struct Team(u8);

    impl VisibilityFilter for Team {
        type ClientComponent = Self;
        type Scope = SingleComponent<Health>;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some_and(|c| self == c)
        }
    }

    #[derive(Component)]
    struct Health(u8);
    ```

    Hide multiple components:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component, PartialEq)]
    #[component(immutable)]
    struct Team(u8);

    impl VisibilityFilter for Team {
        type ClientComponent = Self;
        type Scope = (Health, Stats);

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some_and(|c| self == c)
        }
    }

    #[derive(Component)]
    struct Health(u8);

    #[derive(Component)]
    struct Stats {
    // ...
    }
    ```
    */
    type Scope: FilterScope;

    /**
    Controls when a [`Self::Scope`] is present on a client.

    - For an entity scope, this describes when the entity is spawned or despawned.
    - For a component scope, it describes when the components are inserted or removed.

    Changes are replicated only while the scope is visible.

    # Examples

    Keep a previously discovered entity on the client when it leaves
    the client's field of view.

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component)]
    #[component(immutable)]
    struct VisibilityPosition(Vec2);

    impl VisibilityFilter for VisibilityPosition {
        type ClientComponent = ClientView;
        type Scope = Entity;
        const LIFETIME: ScopeLifetime = ScopeLifetime::AfterFirstVisibility;

        fn is_visible(
            &self,
            _client: Entity,
            component: Option<&Self::ClientComponent>,
        ) -> bool {
            component.is_some_and(|view| {
                self.0.distance_squared(view.position) <= view.radius.powi(2)
            })
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct ClientView {
        position: Vec2,
        radius: f32,
    }
    ```

    Replicate an entity to all clients except its owner.
    The owner receives only the initial state, but simulates on its own.

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component)]
    #[component(immutable)]
    struct OwnedBy(Entity);

    impl VisibilityFilter for OwnedBy {
        type ClientComponent = AuthorizedClient;
        type Scope = Entity;
        const LIFETIME: ScopeLifetime = ScopeLifetime::AlwaysPresent;

        fn is_visible(&self, client: Entity, _: Option<&Self::ClientComponent>) -> bool {
            self.0 != client
        }
    }
    ```
     */
    const LIFETIME: ScopeLifetime = ScopeLifetime::WhileVisible;

    /**
    Returns `true` if a client should see [`Self::Scope`] for an entity with this component
    based on [`Self::ClientComponent`] .

    # Examples

    Visible if the component is present on both the entity and the client:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    /// Only astral players can see other astral entities.
    #[derive(Component)]
    #[component(immutable)] // Component should be immutable.
    struct Astral;

    impl VisibilityFilter for Astral {
        type ClientComponent = Self;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some()
        }
    }
    ```

    Visible if the component is present on the entity, but missing on the client:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component)]
    #[component(immutable)]
    struct Unit;

    #[derive(Component)]
    #[component(immutable)]
    struct Blind;

    impl VisibilityFilter for Blind {
        type ClientComponent = Unit;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            // Blind clients cannot see units.
            component.is_none()
        }
    }
    ```

    Visible if the entity and the client have equal component values:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component, PartialEq)]
    #[component(immutable)]
    struct Team(u8);

    impl VisibilityFilter for Team {
        type ClientComponent = Self;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            // Visible if the client belongs to the same team.
            component.is_some_and(|c| self == c)
        }
    }
    ```

    Visible if client has all bits the entity has:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    use bitflags::bitflags;

    bitflags! {
        #[derive(Component, Clone, Copy)]
        #[component(immutable)]
        pub(crate) struct RemoteVisibility: u8 {
            const SPIRIT = 0b0001;
            const STEALTH = 0b0010;
            const SHADOW = 0b0100;
            const QUEST_ONLY = 0b1000;
        }
    }

    impl VisibilityFilter for RemoteVisibility {
        type ClientComponent = Self;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some_and(|&c| self.contains(c))
        }
    }
    ```

    Visible if the component references the client entity:

    ```
    # use bevy::prelude::*;
    # use bevy_replicon::prelude::*;
    #[derive(Component, PartialEq)]
    #[component(immutable)]
    struct Owner(Entity);

    impl VisibilityFilter for Owner {
        type ClientComponent = AuthorizedClient; // All clients authorized for replication have this component.
        type Scope = Entity;

        fn is_visible(&self, client: Entity, _component: Option<&Self::ClientComponent>) -> bool {
            self.0 == client
        }
    }
    ```
    */
    fn is_visible(&self, client: Entity, component: Option<&Self::ClientComponent>) -> bool;
}

/// Data affected by [`VisibilityFilter`].
#[derive(Clone)]
pub enum VisibilityScope {
    /// Whole entity.
    Entity,
    /// Specific components on the entity.
    Components(ComponentMask),
    /// All components on the entity, except these.
    AllExcept(ComponentMask),
}

/// Controls when a [`VisibilityScope`] is present on a client.
///
/// See also [`VisibilityFilter::Scope`] and
/// [`FilterRegistry::register_scope`](crate::server::visibility::registry::FilterRegistry::register_scope).
#[derive(PartialEq, Eq, Ord, PartialOrd, Clone, Copy)]
pub enum ScopeLifetime {
    /// Inserted/spawned when it becomes visible and despawns when loses
    /// visibility.
    ///
    /// The scope is present only while it is visible.
    ///
    /// It's spawned/inserted when it becomes visible, and despawned/removed
    /// when it becomes hidden.
    WhileVisible,

    /// The scope remains present after becoming visible for the first time.
    ///
    /// It's not spawned/inserted until it first becomes visible. After
    /// that, it remains present when hidden, but receives changes only while
    /// visible.
    ///
    /// When visibility is regained, existing components receive their latest
    /// state. However, component removals and entity despawns that happen while
    /// hidden won't be reapplied, so the client may retain stale components
    /// or entities.
    ///
    /// If you want to send a despawn or removal even to clients that don't see
    /// the scope, you can make it temporarily visible by removing the filter
    /// component (even in the same frame):
    ///
    /// ```
    /// # use bevy::prelude::*;
    /// # let mut world = World::new();
    /// # let mut entity = world.spawn_empty();
    /// entity.remove::<ComponentThatAffectsVisibility>();
    /// entity.despawn();
    /// # #[derive(Component)]
    /// # struct ComponentThatAffectsVisibility;
    /// ```
    ///
    /// Clients for which the scope was hidden will receive only
    /// the despawn or removal, without the latest state.
    AfterFirstVisibility,

    /// The scope is always present, regardless of visibility.
    ///
    /// It's spawned/inserted regardless of the visibility, but receives changes
    /// only while visible.
    ///
    /// As with [`Self::AfterFirstVisibility`], removals and despawns that happen
    /// while hidden are not reapplied.
    AlwaysPresent,
}

/// Associates the type with a visibility scope.
pub trait FilterScope {
    /// Returns data that should be hidden when [`VisibilityFilter::is_visible`] returns `false`.
    fn visibility_scope(world: &mut World, registry: &mut ReplicationRegistry) -> VisibilityScope;
}

/// A [`FilterScope`] with components.
///
/// Implemented for [`SingleComponent`] and tuples of [`Component`]s.
///
/// Used for [`AllExcept`] in order to limit it to components.
pub trait ComponentsScope: FilterScope {}

#[deprecated(since = "0.39.0", note = "Renamed into `SingleComponent`")]
pub type ComponentScope<A> = SingleComponent<A>;

/// A scope for a single component `A`.
///
/// We can't implement [`FilterScope`] for both tuples and all types that implement [`Component`].
/// This is why this wrapper is needed to set the scope for only a single component.
///
/// If you need a [`FilterScope`] for multiple components, use a tuple directly, e.g. `(C1, C2)`.
pub struct SingleComponent<A: Component>(PhantomData<A>);

impl<C: Component<Mutability: MutWrite<C>>> FilterScope for SingleComponent<C> {
    fn visibility_scope(world: &mut World, registry: &mut ReplicationRegistry) -> VisibilityScope {
        let mut mask = ComponentMask::default();
        let (index, _) = registry.init_component_fns::<C>(world);
        mask.insert(index);
        VisibilityScope::Components(mask)
    }
}

impl<C: Component<Mutability: MutWrite<C>>> ComponentsScope for SingleComponent<C> {}

impl FilterScope for Entity {
    fn visibility_scope(
        _world: &mut World,
        _registry: &mut ReplicationRegistry,
    ) -> VisibilityScope {
        VisibilityScope::Entity
    }
}

/// Hides every component on the entity except those in `S` when the filter denies visibility.
///
/// `S` must be a [`ComponentsScope`]: [`SingleComponent`] or a tuple of [`Component`]s.
/// Useful for keeping a stripped-down view of an entity replicated past its full
/// visibility range, e.g. only its transform and light.
///
/// # Examples
///
/// ```
/// # use bevy::prelude::*;
/// # use bevy_replicon::prelude::*;
/// #[derive(Component, PartialEq)]
/// #[component(immutable)]
/// struct InRange(bool);
///
/// impl VisibilityFilter for InRange {
///     type ClientComponent = Self;
///     // Out-of-range clients keep only `Transform` and `PointLight`.
///     type Scope = AllExcept<(Transform, PointLight)>;
///
///     fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
///         component.is_some_and(|c| c.0)
///     }
/// }
/// # #[derive(Component)] struct PointLight;
/// ```
pub struct AllExcept<S>(PhantomData<S>);

impl<S: ComponentsScope> FilterScope for AllExcept<S> {
    fn visibility_scope(world: &mut World, registry: &mut ReplicationRegistry) -> VisibilityScope {
        let VisibilityScope::Components(mask) = S::visibility_scope(world, registry) else {
            unreachable!("`ComponentsScope` always yields `VisibilityScope::Components`");
        };
        VisibilityScope::AllExcept(mask)
    }
}

macro_rules! impl_filter_scope {
    ($($C:ident),*) => {
        impl<$($C: Component<Mutability: MutWrite<$C>>),*> FilterScope for ($($C,)*) {
            fn visibility_scope(world: &mut World, registry: &mut ReplicationRegistry) -> VisibilityScope {
                let mut mask = ComponentMask::default();
                $(
                    let (index, _) = registry.init_component_fns::<$C>(world);
                    mask.insert(index);
                )*
                VisibilityScope::Components(mask)
            }
        }

        impl<$($C: Component<Mutability: MutWrite<$C>>),*> ComponentsScope for ($($C,)*) {}
    };
}

variadics_please::all_tuples!(impl_filter_scope, 2, 10, C);
