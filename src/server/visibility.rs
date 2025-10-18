pub mod client_visibility;

use bevy::{
    ecs::{component::Immutable, entity_disabling::Disabled},
    prelude::*,
};
use log::debug;

use client_visibility::ClientVisibility;

/// Remote visibility functions for [`App`].
pub trait AppVisibilityExt {
    /**
    Registers a component as a remote visibility filter for entities.

    An entity will be visible to a client if it has all the filter components
    present on the entity, and [`VisibilityFilter::is_visible`] returns `true` for each of them.

    To check whether an entity is visible to a client based on all filters, use [`ClientVisibility::is_visible`].

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
        self.add_observer(hide_for_new_clients::<F>)
            .add_observer(on_insert::<F>)
            .add_observer(on_remove::<F>)
    }
}

fn hide_for_new_clients<F: VisibilityFilter>(
    insert: On<Insert, ClientVisibility>,
    mut clients: Query<&mut ClientVisibility, Without<F>>,
    entities: Query<Entity, With<F>>,
) {
    if let Ok(mut visibility) = clients.get_mut(insert.entity) {
        debug!(
            "updating visibility for client `{}` that doesn't have `{}`",
            insert.entity,
            ShortName::of::<F>(),
        );
        for entity in &entities {
            visibility.set_visibility::<F>(entity, false);
        }
    }
}

fn on_insert<F: VisibilityFilter>(
    insert: On<Insert, F>,
    entities: Query<(Entity, &F), (Without<ClientVisibility>, Allow<Disabled>)>,
    mut clients: Query<(Option<&F>, &mut ClientVisibility)>,
) {
    if let Ok((client_component, mut visibility)) = clients.get_mut(insert.entity) {
        debug!(
            "updating visibility for client `{}` after `{}` insertion",
            insert.entity,
            ShortName::of::<F>(),
        );
        let client_component = client_component.unwrap();
        for (entity, component) in &entities {
            let visible = client_component.is_visible(component);
            visibility.set_visibility::<F>(entity, visible);
        }
    } else {
        debug!(
            "updating visibility for `{}` after `{}` insertion",
            insert.entity,
            ShortName::of::<F>(),
        );
        let (_, component) = entities.get(insert.entity).unwrap();
        for (client_component, mut visibility) in &mut clients {
            let visible = client_component.is_some_and(|c| c.is_visible(component));
            visibility.set_visibility::<F>(insert.entity, visible);
        }
    }
}

fn on_remove<F: VisibilityFilter>(
    remove: On<Remove, F>,
    mut clients: Query<&mut ClientVisibility>,
    entities: Query<Entity, (With<F>, Without<ClientVisibility>)>,
) {
    if let Ok(mut visibility) = clients.get_mut(remove.entity) {
        debug!(
            "updating visibility for client `{}` after `{}` removal",
            remove.entity,
            ShortName::of::<F>(),
        );
        for entity in &entities {
            visibility.set_visibility::<F>(entity, false);
        }
    } else {
        debug!(
            "updating visibility for `{}` after `{}` removal",
            remove.entity,
            ShortName::of::<F>(),
        );
        for mut visibility in &mut clients {
            visibility.set_visibility::<F>(remove.entity, true);
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
#[derive(Component)]
#[component(immutable)] // Component should be immutable.
struct RemoteVisible;

impl VisibilityFilter for RemoteVisible {
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
    fn is_visible(&self, entity_filter: &Self) -> bool {
    entity_filter.contains(*self)
    }
}
```
*/
pub trait VisibilityFilter: Component<Mutability = Immutable> {
    /// Returns `true` if a client with this component should see an entity with this component.
    fn is_visible(&self, entity_filter: &Self) -> bool;
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn before_clients() {
        let mut app = App::new();
        app.add_visibility_filter::<A>();

        let client1 = app.world_mut().spawn((ClientVisibility::default(), A)).id();
        let client2 = app.world_mut().spawn(ClientVisibility::default()).id();
        let entity = app.world_mut().spawn(A).id();

        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.is_hidden(entity));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(visibility2.is_hidden(entity));
    }

    #[test]
    fn after_clients() {
        let mut app = App::new();
        app.add_visibility_filter::<A>();

        let entity = app.world_mut().spawn(A).id();
        let client1 = app.world_mut().spawn((ClientVisibility::default(), A)).id();
        let client2 = app.world_mut().spawn(ClientVisibility::default()).id();

        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.is_hidden(entity));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(visibility2.is_hidden(entity));
    }

    #[test]
    fn remove_filter_from_entity() {
        let mut app = App::new();
        app.add_visibility_filter::<A>();

        let client = app.world_mut().spawn((ClientVisibility::default(), A)).id();
        let entity = app.world_mut().spawn(A).remove::<A>().id();

        let visibility = app.world().get::<ClientVisibility>(client).unwrap();
        assert!(!visibility.is_hidden(entity));
    }

    #[test]
    fn remove_filter_from_client() {
        let mut app = App::new();
        app.add_visibility_filter::<A>();

        let entity = app.world_mut().spawn(A).id();
        let client = app
            .world_mut()
            .spawn((ClientVisibility::default(), A))
            .remove::<A>()
            .id();

        let visibility = app.world().get::<ClientVisibility>(client).unwrap();
        assert!(visibility.is_hidden(entity));
    }

    #[test]
    fn multiple_filters() {
        let mut app = App::new();
        app.add_visibility_filter::<A>()
            .add_visibility_filter::<B>();

        let client1 = app
            .world_mut()
            .spawn((ClientVisibility::default(), A, B))
            .id();
        let client2 = app.world_mut().spawn((ClientVisibility::default(), A)).id();
        let entity = app.world_mut().spawn((A, B)).id();

        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.is_hidden(entity));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(visibility2.is_hidden(entity));

        // Hide entity from the first client too.
        app.world_mut().entity_mut(client1).remove::<B>();

        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(visibility1.is_hidden(entity));

        // Relax visibility constraints to make it visible to both.
        app.world_mut().entity_mut(entity).remove::<B>();

        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.is_hidden(entity));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(!visibility2.is_hidden(entity));
    }

    #[derive(Component)]
    #[component(immutable)]
    struct A;

    impl VisibilityFilter for A {
        fn is_visible(&self, _entity_filter: &Self) -> bool {
            true
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct B;

    impl VisibilityFilter for B {
        fn is_visible(&self, _entity_filter: &Self) -> bool {
            true
        }
    }
}
