pub mod client_visibility;
pub mod filters_mask;
pub mod registry;

use core::marker::PhantomData;

use bevy::{
    ecs::{component::Immutable, entity_disabling::Disabled},
    prelude::*,
};
use log::debug;

use crate::shared::replication::registry::{
    ReplicationRegistry, command_fns::MutWrite, component_mask::ComponentMask,
};
use client_visibility::ClientVisibility;
use registry::{FilterRegistry, VisibilityScope};

/// Remote visibility functions for [`App`].
pub trait AppVisibilityExt {
    /**
    Registers a component as a remote visibility filter.

    This component needs to be inserted on both the client entity and replicated entities.
    If [`VisibilityFilter::is_visible`] on the client;s component returns `false` for the
    corresponding component on a replicated entity, the associated [`VisibilityFilter::Scope`]
    (entity or components) becomes hidden for the client.

    If the data was previously visible, it will be despawned or removed. If the component is
    missing on either the client or a replicated entity, it is treated as evaluating to `false`.

    To keep the representation compact, the total number of registered filters cannot exceed [`u32::MAX`].
    But a filter can itself represent multiple flags using a bitmask. See the example in [`VisibilityFilter`].

    # Examples

    ```
    # use bevy::state::app::StatesPlugin;
    use bevy::prelude::*;
    use bevy_replicon::prelude::*;
    use serde::{Deserialize, Serialize};

    # let mut app = App::new();
    # app.add_plugins((StatesPlugin, RepliconPlugins));
    app.add_client_event::<JoinGuild>(Channel::Ordered)
        .add_visibility_filter::<Guild>();

    /// Processes a client request to join a guild.
    fn add_to_guild(join: On<FromClient<JoinGuild>>, mut commands: Commands) {
        // The server can see all entities, so in listen-server mode,
        // we check if the sender is a client.
        if let ClientId::Client(client) = join.client_id {
            // Now the sender can see all entities that have a `Guild`
            // component with the same ID.
            commands.entity(client).insert(Guild { id: join.id });
        }
    }

    #[derive(Event, Deref, Serialize, Deserialize)]
    struct JoinGuild {
        id: u32
    };

    #[derive(Component, PartialEq)]
    #[component(immutable)]
    struct Guild {
        id: u32
    }

    impl VisibilityFilter for Guild {
        type Scope = Entity;

        fn is_visible(&self, entity_filter: &Self) -> bool {
            self == entity_filter
        }
    }
    ```
    */
    fn add_visibility_filter<F: VisibilityFilter>(&mut self) -> &mut Self;
}

impl AppVisibilityExt for App {
    fn add_visibility_filter<F: VisibilityFilter>(&mut self) -> &mut Self {
        debug!("adding visibility filter `{}`", ShortName::of::<F>());

        self.world_mut()
            .resource_scope(|world, mut filter_registry: Mut<FilterRegistry>| {
                world.resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                    filter_registry.register::<F>(world, &mut registry);
                })
            });

        self.add_observer(hide_for_new_clients::<F>)
            .add_observer(on_insert::<F>)
            .add_observer(on_remove::<F>)
    }
}

fn hide_for_new_clients<F: VisibilityFilter>(
    insert: On<Insert, ClientVisibility>,
    registry: Res<FilterRegistry>,
    mut clients: Query<&mut ClientVisibility, Without<F>>,
    entities: Query<Entity, With<F>>,
) {
    if let Ok(mut visibility) = clients.get_mut(insert.entity) {
        let bit = registry.bit::<F>();
        for entity in &entities {
            debug!(
                "hiding `{entity}` from client `{}` without `{}` filter",
                insert.entity,
                ShortName::of::<F>(),
            );
            visibility.set(entity, bit, false);
        }
    }
}

fn on_insert<F: VisibilityFilter>(
    insert: On<Insert, F>,
    registry: Res<FilterRegistry>,
    entities: Query<(Entity, &F), (Without<ClientVisibility>, Allow<Disabled>)>,
    mut clients: Query<(Entity, Option<&F>, &mut ClientVisibility)>,
) {
    let bit = registry.bit::<F>();
    if let Ok((client_entity, client_component, mut visibility)) = clients.get_mut(insert.entity) {
        let client_component = client_component.unwrap();
        for (entity, component) in &entities {
            let visible = client_component.is_visible(component);
            debug!(
                "updating `{}` filter on client `{client_entity}` to `{visible}` for `{entity}`",
                ShortName::of::<F>(),
            );
            visibility.set(entity, bit, visible);
        }
    } else {
        let (entity, component) = entities.get(insert.entity).unwrap();
        for (client_entity, client_component, mut visibility) in &mut clients {
            let visible = client_component.is_some_and(|c| c.is_visible(component));
            debug!(
                "updating `{}` filter on `{entity}` to `{visible}` for client `{client_entity}`",
                ShortName::of::<F>(),
            );
            visibility.set(insert.entity, bit, visible);
        }
    }
}

fn on_remove<F: VisibilityFilter>(
    remove: On<Remove, F>,
    registry: Res<FilterRegistry>,
    mut clients: Query<&mut ClientVisibility>,
    entities: Query<Entity, (With<F>, Without<ClientVisibility>)>,
) {
    let bit = registry.bit::<F>();
    if let Ok(mut visibility) = clients.get_mut(remove.entity) {
        for entity in &entities {
            debug!(
                "hiding `{entity}` from client `{}` after `{}` filter removal",
                remove.entity,
                ShortName::of::<F>(),
            );
            visibility.set(entity, bit, false);
        }
    } else {
        debug!(
            "removing `{}` filter from `{}`",
            ShortName::of::<F>(),
            remove.entity
        );
        for mut visibility in &mut clients {
            visibility.set(remove.entity, bit, true);
        }
    }
}

/**
Component that controls remote entity visibility.

Should be registered via [`AppVisibilityExt`].

# Examples

Visible if the filter is present on both the entity and the client:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
/// Only ghost players can see ghosts.
#[derive(Component)]
#[component(immutable)] // Component should be immutable.
struct Ghost;

impl VisibilityFilter for Ghost {
    type Scope = Entity;

    fn is_visible(&self, _entity_filter: &Self) -> bool {
        true
    }
}
```

Visible if the entity and the client belong to the same team:

```
# use bevy::prelude::*;
# use bevy_replicon::prelude::*;
#[derive(Component, PartialEq)]
#[component(immutable)]
struct Team(u8);

impl VisibilityFilter for Team {
    type Scope = Entity;

    fn is_visible(&self, entity_filter: &Self) -> bool {
        self == entity_filter
    }
}
```

Visible if entity has all bits the client has:

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
    type Scope = Entity;

    fn is_visible(&self, entity_filter: &Self) -> bool {
        entity_filter.contains(*self)
    }
}
```
*/
pub trait VisibilityFilter: Component<Mutability = Immutable> {
    /// Defines what data is affected when the filter denies visibility.
    ///
    /// - To hide the entire entity, this type must be [`Entity`].
    /// - To hide a single component on the entity, this type must be [`ComponentScope`].
    /// - To hide more than one component on the entity, this type must be a tuple of those [`Component`]s.
    ///
    /// # Examples
    ///
    /// Hide the entire entity:
    ///
    /// ```
    /// # use bevy::prelude::*;
    /// # use bevy_replicon::prelude::*;
    /// #[derive(Component, PartialEq)]
    /// #[component(immutable)]
    /// struct Team(u8);
    ///
    /// impl VisibilityFilter for Team {
    ///     type Scope = Entity;
    ///
    ///     fn is_visible(&self, entity_filter: &Self) -> bool {
    ///         self == entity_filter
    ///     }
    /// }
    /// ```
    ///
    /// Hide only a single component:
    ///
    /// ```
    /// # use bevy::prelude::*;
    /// # use bevy_replicon::prelude::*;
    /// #[derive(Component, PartialEq)]
    /// #[component(immutable)]
    /// struct Team(u8);
    ///
    /// impl VisibilityFilter for Team {
    ///     type Scope = ComponentScope<Health>;
    ///
    ///     fn is_visible(&self, entity_filter: &Self) -> bool {
    ///         self == entity_filter
    ///     }
    /// }
    ///
    /// #[derive(Component)]
    /// struct Health(u8);
    /// ```
    ///
    /// Hide multiple components:
    ///
    /// ```
    /// # use bevy::prelude::*;
    /// # use bevy_replicon::prelude::*;
    /// #[derive(Component, PartialEq)]
    /// #[component(immutable)]
    /// struct Team(u8);
    ///
    /// impl VisibilityFilter for Team {
    ///     type Scope = (Health, Stats);
    ///
    ///     fn is_visible(&self, entity_filter: &Self) -> bool {
    ///         self == entity_filter
    ///     }
    /// }
    ///
    /// #[derive(Component)]
    /// struct Health(u8);
    ///
    /// #[derive(Component)]
    /// struct Stats {
    /// // ...
    /// }
    /// ```
    type Scope: FilterScope;

    /// Returns `true` if a client with this component should see [`Self::Scope`] for an entity with this component.
    fn is_visible(&self, entity_filter: &Self) -> bool;
}

/// Associates the type with a visibility scope.
pub trait FilterScope {
    /// Returns data that should be hidden when [`VisibilityFilter::is_visible`] returns `false`.
    fn visibility_scope(world: &mut World, registry: &mut ReplicationRegistry) -> VisibilityScope;
}

/// A scope for a single component `A`.
///
/// We can't implement [`FilterScope`] for both tuples and all types that implement [`Component`].
/// This is why this wrapper is needed to set the scope for only a single component.
pub struct ComponentScope<A: Component>(PhantomData<A>);

impl<C: Component<Mutability: MutWrite<C>>> FilterScope for ComponentScope<C> {
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

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn after_clients() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<A>();

        let client1 = app.world_mut().spawn((ClientVisibility::default(), A)).id();
        let client2 = app.world_mut().spawn(ClientVisibility::default()).id();
        let entity = app.world_mut().spawn(A).id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.get(entity).is_hidden(registry));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(visibility2.get(entity).is_hidden(registry));
    }

    #[test]
    fn before_clients() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<A>();

        let entity = app.world_mut().spawn(A).id();
        let client1 = app.world_mut().spawn((ClientVisibility::default(), A)).id();
        let client2 = app.world_mut().spawn(ClientVisibility::default()).id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.get(entity).is_hidden(registry));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(visibility2.get(entity).is_hidden(registry));
    }

    #[test]
    fn remove_filter_from_entity() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<A>();

        let client = app.world_mut().spawn(ClientVisibility::default()).id();
        let entity = app.world_mut().spawn(A).remove::<A>().id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility = app.world().get::<ClientVisibility>(client).unwrap();
        assert!(!visibility.get(entity).is_hidden(registry));
    }

    #[test]
    fn remove_filter_from_client() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<A>();

        let entity = app.world_mut().spawn(A).id();
        let client = app
            .world_mut()
            .spawn((ClientVisibility::default(), A))
            .remove::<A>()
            .id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility = app.world().get::<ClientVisibility>(client).unwrap();
        assert!(visibility.get(entity).is_hidden(registry));
    }

    #[test]
    fn multiple_filters() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<A>()
            .add_visibility_filter::<B>();

        let client1 = app
            .world_mut()
            .spawn((ClientVisibility::default(), A, B))
            .id();
        let client2 = app.world_mut().spawn((ClientVisibility::default(), A)).id();
        let entity = app.world_mut().spawn((A, B)).id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.get(entity).is_hidden(registry));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(visibility2.get(entity).is_hidden(registry));

        // Hide entity from the first client too.
        app.world_mut().entity_mut(client1).remove::<B>();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(visibility1.get(entity).is_hidden(registry));

        // Relax visibility constraints to make it visible to both.
        app.world_mut().entity_mut(entity).remove::<B>();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.get(entity).is_hidden(registry));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(!visibility2.get(entity).is_hidden(registry));
    }

    #[derive(Component)]
    #[component(immutable)]
    struct A;

    impl VisibilityFilter for A {
        type Scope = Entity;

        fn is_visible(&self, _entity_filter: &Self) -> bool {
            true
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct B;

    impl VisibilityFilter for B {
        type Scope = Entity;

        fn is_visible(&self, _entity_filter: &Self) -> bool {
            true
        }
    }
}
