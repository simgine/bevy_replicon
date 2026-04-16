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
}

/// Associates the type with a visibility scope.
pub trait FilterScope {
    /// Returns data that should be hidden when [`VisibilityFilter::is_visible`] returns `false`.
    fn visibility_scope(world: &mut World, registry: &mut ReplicationRegistry) -> VisibilityScope;
}

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

impl FilterScope for Entity {
    fn visibility_scope(
        _world: &mut World,
        _registry: &mut ReplicationRegistry,
    ) -> VisibilityScope {
        VisibilityScope::Entity
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
    };
}

variadics_please::all_tuples!(impl_filter_scope, 2, 10, C);
