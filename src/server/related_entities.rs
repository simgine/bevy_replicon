use crate::{
    prelude::*,
    send::{
        add_relation, read_relations, remove_relation, start_replication, stop_replication,
    },
};
use bevy::{
    ecs::{component::Immutable, relationship::Relationship},
    prelude::*,
};

pub(super) use crate::send::RelatedEntities;

pub trait SyncRelatedAppExt {
    /// Ensures that entities related by `C` are replicated in sync.
    ///
    /// By default, we split mutations across multiple messages to apply them independently.
    /// We guarantee that all mutations for a single entity won't be split across messages,
    /// but mutations for separate entities may be received independently if they arrive in
    /// different messages.
    ///
    /// Calling this method guarantees that all mutations related by `C` are included in
    /// a single message.
    ///
    /// Internally we maintain a graph of all relationship types marked for replication in sync.
    /// It's updated via observers, so frequent changes may impact the performance.
    ///
    /// # Examples
    /// ```
    /// # use bevy::state::app::StatesPlugin;
    /// use bevy::prelude::*;
    /// use bevy_replicon::prelude::*;
    ///
    /// # let mut app = App::new();
    /// # app.add_plugins((StatesPlugin, RepliconPlugins));
    /// app.sync_related_entities::<ChildOf>();
    ///
    /// // Changes to any replicated components on these
    /// // entities will be replicated in sync.
    /// app.world_mut().spawn((
    ///     Replicated,
    ///     Transform::default(),
    ///     children![(Replicated, Transform::default())],
    /// ));
    /// ```
    fn sync_related_entities<C>(&mut self) -> &mut Self
    where
        C: Relationship + Component<Mutability = Immutable>;
}

impl SyncRelatedAppExt for App {
    fn sync_related_entities<C>(&mut self) -> &mut Self
    where
        C: Relationship + Component<Mutability = Immutable>,
    {
        self.add_systems(
            OnEnter(ServerState::Running),
            read_relations::<C>.in_set(ServerSystems::ReadRelations),
        )
        .add_observer(add_relation::<C>)
        .add_observer(remove_relation::<C>)
        .add_observer(start_replication::<C>)
        .add_observer(stop_replication::<C>)
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use test_log::test;

    use super::*;

    #[test]
    fn orphan() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let entity1 = app
            .world_mut()
            .spawn((Replicated, Children::default()))
            .id();
        let entity2 = app
            .world_mut()
            .spawn((Replicated, Children::default()))
            .id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(entity1), None);
        assert_eq!(related.graph_index(entity2), None);
    }

    #[test]
    fn single() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let root = app.world_mut().spawn(Replicated).id();
        let child1 = app.world_mut().spawn((Replicated, ChildOf(root))).id();
        let child2 = app.world_mut().spawn((Replicated, ChildOf(root))).id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(child2), Some(0));
    }

    #[test]
    fn disjoint() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let root1 = app.world_mut().spawn(Replicated).id();
        let child1 = app.world_mut().spawn((Replicated, ChildOf(root1))).id();
        let root2 = app.world_mut().spawn(Replicated).id();
        let child2 = app.world_mut().spawn((Replicated, ChildOf(root2))).id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 2);
        assert_eq!(related.graph_index(root1), Some(0));
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(root2), Some(1));
        assert_eq!(related.graph_index(child2), Some(1));
    }

    #[test]
    fn nested() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let root = app.world_mut().spawn(Replicated).id();
        let child = app.world_mut().spawn((Replicated, ChildOf(root))).id();
        let grandchild = app.world_mut().spawn((Replicated, ChildOf(child))).id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
        assert_eq!(related.graph_index(grandchild), Some(0));
    }

    #[test]
    fn split() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let root = app.world_mut().spawn(Replicated).id();
        let child = app.world_mut().spawn((Replicated, ChildOf(root))).id();
        let grandchild = app.world_mut().spawn((Replicated, ChildOf(child))).id();
        let grandgrandchild = app
            .world_mut()
            .spawn((Replicated, ChildOf(grandchild)))
            .id();

        app.world_mut().entity_mut(grandchild).remove::<ChildOf>();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 2);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
        assert_eq!(related.graph_index(grandchild), Some(1));
        assert_eq!(related.graph_index(grandgrandchild), Some(1));
    }

    #[test]
    fn join() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let root1 = app.world_mut().spawn(Replicated).id();
        let child1 = app.world_mut().spawn((Replicated, ChildOf(root1))).id();
        let root2 = app.world_mut().spawn(Replicated).id();
        let child2 = app.world_mut().spawn((Replicated, ChildOf(root2))).id();

        app.world_mut().entity_mut(child1).add_child(root2);

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root1), Some(0));
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(root2), Some(0));
        assert_eq!(related.graph_index(child2), Some(0));
    }

    #[test]
    fn reparent() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let root1 = app.world_mut().spawn(Replicated).id();
        let child1 = app.world_mut().spawn((Replicated, ChildOf(root1))).id();
        let root2 = app.world_mut().spawn(Replicated).id();
        let child2 = app.world_mut().spawn((Replicated, ChildOf(root2))).id();

        app.world_mut().entity_mut(child1).insert(ChildOf(root2));

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root1), None);
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(root2), Some(0));
        assert_eq!(related.graph_index(child2), Some(0));
    }

    #[test]
    fn orphan_after_split() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let root = app.world_mut().spawn(Replicated).id();
        let child = app.world_mut().spawn((Replicated, ChildOf(root))).id();

        app.world_mut().entity_mut(child).remove::<ChildOf>();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(root), None);
        assert_eq!(related.graph_index(child), None);
    }

    #[test]
    fn despawn() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let root = app.world_mut().spawn(Replicated).id();
        let child1 = app.world_mut().spawn((Replicated, ChildOf(root))).id();
        let child2 = app.world_mut().spawn((Replicated, ChildOf(root))).id();

        app.world_mut().despawn(root);

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(root), None);
        assert_eq!(related.graph_index(child1), None);
        assert_eq!(related.graph_index(child2), None);
    }

    #[test]
    fn intersection() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let root1 = app.world_mut().spawn(Replicated).id();
        let root2 = app.world_mut().spawn(Replicated).id();
        let child = app
            .world_mut()
            .spawn((Replicated, ChildOf(root1), OwnedBy(root2)))
            .id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root1), Some(0));
        assert_eq!(related.graph_index(root2), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
    }

    #[test]
    fn overlap() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let root = app.world_mut().spawn(Replicated).id();
        let child = app
            .world_mut()
            .spawn((Replicated, ChildOf(root), OwnedBy(root)))
            .id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
    }

    #[test]
    fn overlap_removal() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let root = app.world_mut().spawn(Replicated).id();
        let child = app
            .world_mut()
            .spawn((Replicated, ChildOf(root), OwnedBy(root)))
            .id();

        app.world_mut().entity_mut(child).remove::<ChildOf>();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
    }

    #[test]
    fn connected() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let root = app.world_mut().spawn(Replicated).id();
        let child = app.world_mut().spawn((Replicated, ChildOf(root))).id();
        let grandchild = app.world_mut().spawn((Replicated, OwnedBy(child))).id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
        assert_eq!(related.graph_index(grandchild), Some(0));
    }

    #[test]
    fn replication_start() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let root = app.world_mut().spawn_empty().id();
        let child = app.world_mut().spawn(ChildOf(root)).id();

        app.world_mut().entity_mut(child).insert(Replicated);
        app.world_mut().entity_mut(root).insert(Replicated);

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
    }

    #[test]
    fn replication_stop() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let root = app.world_mut().spawn(Replicated).id();
        let child = app
            .world_mut()
            .spawn((Replicated, ChildOf(root), OwnedBy(root)))
            .id();

        app.world_mut().entity_mut(child).remove::<Replicated>();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(root), None);
        assert_eq!(related.graph_index(child), None);
    }

    #[test]
    fn runs_only_with_server() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .init_state::<ServerState>()
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let root = app.world_mut().spawn(Replicated).id();
        let child1 = app.world_mut().spawn((Replicated, ChildOf(root))).id();
        let child2 = app.world_mut().spawn((Replicated, ChildOf(root))).id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(root), None);
        assert_eq!(related.graph_index(child1), None);
        assert_eq!(related.graph_index(child2), None);

        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);

        app.update();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(child2), Some(0));
    }

    #[derive(Component)]
    #[relationship(relationship_target = Owning)]
    struct OwnedBy(Entity);

    #[derive(Component)]
    #[relationship_target(relationship = OwnedBy)]
    struct Owning(Vec<Entity>);
}
