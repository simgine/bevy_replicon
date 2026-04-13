pub mod client_visibility;
pub mod filters_mask;
pub mod registry;

use bevy::{ecs::entity_disabling::Disabled, prelude::*};
use log::debug;

use crate::shared::replication::{
    registry::ReplicationRegistry,
    visibility::{FilterScope, VisibilityFilter},
};
use client_visibility::ClientVisibility;
use registry::FilterRegistry;

/// Remote visibility functions for [`App`].
pub trait AppVisibilityExt {
    /**
    Registers a component as a remote visibility filter.

    This component must be inserted on replicated entities and will be evaluated
    against [`VisibilityFilter::ClientComponent`] on client entities.

    If [`VisibilityFilter::is_visible`] returns `false` for this component on a
    client entity, the associated [`VisibilityFilter::Scope`] (entity or components)
    will be hidden for that client.

    If the component is missing on a replicated entity, it is treated as if
    [`VisibilityFilter::is_visible`] would return `false`.

    If multiple filters that affect components overlap on an entity, this will work as logical AND:
    [`VisibilityFilter::is_visible`] should return `true` for all of them, otherwise the component will be hidden.
    If any of the filters hide the entity itself, no components will be replicated.

    If the [`VisibilityFilter::Scope`] was previously visible, it will be despawned (for entities) or
    removed (for components).

    To keep the representation compact, the total number of registered filters cannot exceed [`u32::MAX`].
    But a filter can itself represent multiple flags using a bitmask. See the example in [`VisibilityFilter`].

    See also [`ClientVisibility::set`] for manual visibility control.

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
        type ClientComponent = Self;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some_and(|c| self == c)
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
                    filter_registry.register_filter::<F>(world, &mut registry);
                })
            });

        self.add_observer(update_for_new_clients::<F>)
            .add_observer(on_insert::<F>)
            .add_observer(on_client_insert::<F>)
            .add_observer(on_remove::<F>)
            .add_observer(on_client_remove::<F>)
    }
}

fn update_for_new_clients<F: VisibilityFilter>(
    insert: On<Insert, ClientVisibility>,
    registry: Res<FilterRegistry>,
    mut clients: Query<&mut ClientVisibility, Without<F::ClientComponent>>,
    entities: Query<(Entity, &F)>,
) {
    if let Ok(mut visibility) = clients.get_mut(insert.entity) {
        let bit = registry.bit::<F>();
        for (entity, component) in &entities {
            let visible = component.is_visible(insert.entity, None);
            debug!(
                "evaluating missing `{}` for new client `{}` for entity `{entity}` to `{visible}`",
                ShortName::of::<F>(),
                insert.entity,
            );
            visibility.set(entity, bit, visible);
        }
    }
}

fn on_insert<F: VisibilityFilter>(
    insert: On<Insert, F>,
    registry: Res<FilterRegistry>,
    entities: Query<(Entity, &F), (Without<ClientVisibility>, Allow<Disabled>)>,
    mut clients: Query<(Entity, Option<&F::ClientComponent>, &mut ClientVisibility)>,
) {
    // `F` and `F::ClientComponent` could be the same,
    // so we need to ensure that it was not inserted into a client
    if clients.contains(insert.entity) {
        return;
    }

    let bit = registry.bit::<F>();
    let (entity, component) = entities.get(insert.entity).unwrap();
    for (client, client_component, mut visibility) in &mut clients {
        let visible = component.is_visible(client, client_component);
        debug!(
            "evaluating inserted `{}` to entity `{entity}` for client `{client}` to `{visible}`",
            ShortName::of::<F>(),
        );
        visibility.set(insert.entity, bit, visible);
    }
}

fn on_client_insert<F: VisibilityFilter>(
    insert: On<Insert, F::ClientComponent>,
    registry: Res<FilterRegistry>,
    mut clients: Query<(&F::ClientComponent, &mut ClientVisibility)>,
    entities: Query<(Entity, &F), Without<ClientVisibility>>,
) {
    let Ok((client_component, mut visibility)) = clients.get_mut(insert.entity) else {
        return;
    };

    let bit = registry.bit::<F>();
    for (entity, component) in &entities {
        let visible = component.is_visible(insert.entity, Some(client_component));
        debug!(
            "evaluating inserted `{}` to client `{}` for entity `{entity}` to `{visible}`",
            ShortName::of::<F>(),
            insert.entity
        );
        visibility.set(entity, bit, visible);
    }
}

fn on_remove<F: VisibilityFilter>(
    remove: On<Remove, F>,
    registry: Res<FilterRegistry>,
    mut clients: Query<&mut ClientVisibility>,
) {
    // `F` and `F::ClientComponent` could be the same,
    // so we need to ensure that it wasn't removed from a client.
    if clients.contains(remove.entity) {
        return;
    }

    let bit = registry.bit::<F>();
    debug!(
        "removing `{}` filter from entity `{}`",
        ShortName::of::<F>(),
        remove.entity
    );
    for mut visibility in &mut clients {
        visibility.set(remove.entity, bit, true);
    }
}

fn on_client_remove<F: VisibilityFilter>(
    remove: On<Remove, F::ClientComponent>,
    registry: Res<FilterRegistry>,
    mut clients: Query<&mut ClientVisibility>,
    entities: Query<(Entity, &F), Without<ClientVisibility>>,
) {
    let Ok(mut visibility) = clients.get_mut(remove.entity) else {
        return;
    };

    let bit = registry.bit::<F>();
    for (entity, component) in &entities {
        let visible = component.is_visible(remove.entity, None);
        debug!(
            "evaluating removed `{}` from client `{}` for entity `{entity}` to `{visible}`",
            ShortName::of::<F>(),
            remove.entity
        );
        visibility.set(entity, bit, visible);
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn after_clients() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<SelfFilter>()
            .add_visibility_filter::<EntityFilter>();

        let client1 = app
            .world_mut()
            .spawn((ClientVisibility::default(), SelfFilter, ClientFilter))
            .id();
        let client2 = app.world_mut().spawn(ClientVisibility::default()).id();
        let entity1 = app.world_mut().spawn(SelfFilter).id();
        let entity2 = app.world_mut().spawn(EntityFilter).id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.get(entity1).is_hidden(registry));
        assert!(!visibility1.get(entity2).is_hidden(registry));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(visibility2.get(entity1).is_hidden(registry));
        assert!(visibility2.get(entity2).is_hidden(registry));
    }

    #[test]
    fn before_clients() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<SelfFilter>()
            .add_visibility_filter::<EntityFilter>();

        let entity1 = app.world_mut().spawn(SelfFilter).id();
        let entity2 = app.world_mut().spawn(EntityFilter).id();
        let client1 = app
            .world_mut()
            .spawn((ClientVisibility::default(), SelfFilter, ClientFilter))
            .id();
        let client2 = app.world_mut().spawn(ClientVisibility::default()).id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.get(entity1).is_hidden(registry));
        assert!(!visibility1.get(entity2).is_hidden(registry));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(visibility2.get(entity1).is_hidden(registry));
        assert!(visibility2.get(entity2).is_hidden(registry));
    }

    #[test]
    fn insert_filter_on_client() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<SelfFilter>()
            .add_visibility_filter::<EntityFilter>();

        let entity1 = app.world_mut().spawn(SelfFilter).id();
        let entity2 = app.world_mut().spawn(EntityFilter).id();

        let client = app.world_mut().spawn(ClientVisibility::default()).id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility = app.world().get::<ClientVisibility>(client).unwrap();

        assert!(visibility.get(entity1).is_hidden(registry));
        assert!(visibility.get(entity2).is_hidden(registry));

        app.world_mut()
            .entity_mut(client)
            .insert((SelfFilter, ClientFilter));

        let registry = app.world().resource::<FilterRegistry>();
        let visibility = app.world().get::<ClientVisibility>(client).unwrap();
        assert!(!visibility.get(entity1).is_hidden(registry));
        assert!(!visibility.get(entity2).is_hidden(registry));
    }

    #[test]
    fn remove_filter_from_entity() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<SelfFilter>()
            .add_visibility_filter::<EntityFilter>();

        let client = app.world_mut().spawn(ClientVisibility::default()).id();
        let entity1 = app
            .world_mut()
            .spawn(SelfFilter)
            .remove::<SelfFilter>()
            .id();
        let entity2 = app
            .world_mut()
            .spawn(EntityFilter)
            .remove::<EntityFilter>()
            .id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility = app.world().get::<ClientVisibility>(client).unwrap();
        assert!(!visibility.get(entity1).is_hidden(registry));
        assert!(!visibility.get(entity2).is_hidden(registry));
    }

    #[test]
    fn remove_filter_from_client() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<SelfFilter>()
            .add_visibility_filter::<EntityFilter>();

        let entity1 = app.world_mut().spawn(SelfFilter).id();
        let entity2 = app.world_mut().spawn(EntityFilter).id();
        let client = app
            .world_mut()
            .spawn((ClientVisibility::default(), SelfFilter, ClientFilter))
            .remove::<(SelfFilter, ClientFilter)>()
            .id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility = app.world().get::<ClientVisibility>(client).unwrap();
        assert!(visibility.get(entity1).is_hidden(registry));
        assert!(visibility.get(entity2).is_hidden(registry));
    }

    #[test]
    fn multiple_filters() {
        let mut app = App::new();
        app.init_resource::<FilterRegistry>()
            .init_resource::<ReplicationRegistry>()
            .add_visibility_filter::<SelfFilter>()
            .add_visibility_filter::<EntityFilter>();

        let client1 = app
            .world_mut()
            .spawn((ClientVisibility::default(), SelfFilter, ClientFilter))
            .id();
        let client2 = app
            .world_mut()
            .spawn((ClientVisibility::default(), SelfFilter))
            .id();
        let entity = app.world_mut().spawn((SelfFilter, EntityFilter)).id();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.get(entity).is_hidden(registry));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(visibility2.get(entity).is_hidden(registry));

        // Hide entity from the first client too.
        app.world_mut().entity_mut(client1).remove::<ClientFilter>();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(visibility1.get(entity).is_hidden(registry));

        // Relax visibility constraints to make it visible to both.
        app.world_mut().entity_mut(entity).remove::<EntityFilter>();

        let registry = app.world().resource::<FilterRegistry>();
        let visibility1 = app.world().get::<ClientVisibility>(client1).unwrap();
        assert!(!visibility1.get(entity).is_hidden(registry));

        let visibility2 = app.world().get::<ClientVisibility>(client2).unwrap();
        assert!(!visibility2.get(entity).is_hidden(registry));
    }

    #[derive(Component)]
    #[component(immutable)]
    struct SelfFilter;

    impl VisibilityFilter for SelfFilter {
        type ClientComponent = Self;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some()
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct EntityFilter;

    impl VisibilityFilter for EntityFilter {
        type ClientComponent = ClientFilter;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some()
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct ClientFilter;
}
